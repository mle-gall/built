// SPDX-License-Identifier: GPL-3.0-or-later
//! Native History steps (docs/commands-port.md). A `NativeCommand` is a
//! typed delta over the document: executing it writes the fields in Rust
//! inside the step's transaction, and the recorded inverse is explicit, so
//! no Python `do` or `undo` takes part. History replays the transaction;
//! the inverse serves direct calls outside History and mixed compounds.
//!
//! The selection and the scene's version counter remain host state: a
//! step that hides or locks placements drops them from the selection, and
//! every step bumps `scene.version`, as the Python commands did.
use super::py::{fval_for, fval_to_py, keys_of, PyPlacement};
use super::{drain_pending, lock, Field, FVal, Key, Trash};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::gc::PyVisit;
use pyo3::prelude::*;
use pyo3::types::PyList;
use pyo3::PyTraverseError;
use std::collections::HashSet;
use std::sync::Mutex;

/// The values a field delta writes: one value for every target (written
/// as a single journal op) or one value per target (an inverse).
#[derive(Clone)]
enum Values {
    One(FVal),
    Each(Vec<FVal>),
}

struct State {
    /// What `do` writes, and after `do` what `undo` writes back.
    forward: Values,
    inverse: Option<Values>,
}

#[pyclass(module = "ingetrazo_core", name = "NativeCommand", frozen, dict, weakref)]
pub struct NativeCommand {
    /// The step's name in the feed and the History journal. Kept from the
    /// retired Python class so feed consumers and MCP see the same record.
    #[pyo3(get)]
    history_name: String,
    field: Field,
    targets: Vec<Py<PyAny>>,
    keys: Vec<Key>,
    /// Drop the targets from the selection on execute and redo.
    unselect: bool,
    /// The feed may call the step complete from its journal alone.
    covers: bool,
    state: Mutex<State>,
}

fn placements(groups: &Bound<'_, PyAny>) -> PyResult<(Vec<Py<PyAny>>, Vec<Key>)> {
    let mut seen = HashSet::new();
    let mut targets = Vec::new();
    let mut keys = Vec::new();
    for (k, _, item) in keys_of(groups)? {
        if !item.is_instance_of::<PyPlacement>() {
            return Err(PyTypeError::new_err("native placement commands take placements"));
        }
        // A repeated placement is written once, as `doc_set_many` did.
        if seen.insert(k) {
            targets.push(item.unbind());
            keys.push(k);
        }
    }
    Ok((targets, keys))
}

impl NativeCommand {
    fn new(name: &str, field: Field, groups: &Bound<'_, PyAny>, forward: Values, unselect: bool) -> PyResult<Self> {
        let (targets, keys) = placements(groups)?;
        if let Values::Each(values) = &forward {
            if values.len() != keys.len() {
                return Err(PyValueError::new_err("one value per distinct placement"));
            }
        }
        Ok(Self {
            history_name: name.to_owned(),
            field,
            targets,
            keys,
            unselect,
            covers: field != Field::Material,
            state: Mutex::new(State { forward, inverse: None }),
        })
    }

    /// Write `values` and return what the fields held before.
    fn apply(&self, py: Python<'_>, values: &Values) -> PyResult<Values> {
        let f = self.field;
        let mut trash = Trash::default();
        let result = {
            let mut d = lock();
            for (k, h) in self.keys.iter().zip(&self.targets) {
                d.set_handle(*k, h.as_ptr());
            }
            (|| -> Result<Values, String> {
                let mut old = Vec::with_capacity(self.keys.len());
                for &k in &self.keys {
                    old.push(d.need(k)?.get(f)?);
                }
                match values {
                    Values::One(v) => {
                        // Writes that change nothing stay out of the journal.
                        let keys: Vec<Key> = self
                            .keys
                            .iter()
                            .zip(&old)
                            .filter(|(_, o)| *o != v)
                            .map(|(k, _)| *k)
                            .collect();
                        if !keys.is_empty() {
                            d.write_many(py, &keys, f, v, &mut trash)?;
                        }
                    }
                    Values::Each(vs) => {
                        for ((&k, v), o) in self.keys.iter().zip(vs).zip(&old) {
                            if o != v {
                                d.write(py, k, f, v.clone(), &mut trash)?;
                            }
                        }
                    }
                }
                Ok(Values::Each(old))
            })()
        };
        drop(trash);
        drain_pending();
        result.map_err(|e| PyValueError::new_err(e))
    }

    fn unselect_targets(&self, py: Python<'_>, scene: &Bound<'_, PyAny>) -> PyResult<()> {
        let selection = scene.getattr("selection")?;
        let size = selection.len()?;
        if size == 0 {
            return Ok(());
        }
        // From the selection's side when it is the smaller (Lock of 20,000
        // placements with a handful selected need not look each one up).
        if size * 4 < self.targets.len() {
            let wanted: HashSet<usize> = self.targets.iter().map(|t| t.as_ptr() as usize).collect();
            let gone: Vec<Bound<'_, PyAny>> = selection
                .try_iter()?
                .filter_map(|item| item.ok())
                .filter(|item| wanted.contains(&(item.as_ptr() as usize)))
                .collect();
            for item in gone {
                selection.call_method1("discard", (item,))?;
            }
        } else {
            for target in &self.targets {
                selection.call_method1("discard", (target.bind(py),))?;
            }
        }
        Ok(())
    }
}

fn bump(scene: &Bound<'_, PyAny>) -> PyResult<()> {
    let version: i64 = scene.getattr("version")?.extract()?;
    scene.setattr("version", version + 1)
}

#[pymethods]
#[allow(non_upper_case_globals)]
impl NativeCommand {
    /// Undone and redone by the step's document transaction.
    #[classattr]
    const doc_undo: bool = true;
    #[classattr]
    const keeps_rows: bool = false;
    #[classattr]
    const keeps_paint: bool = false;
    #[classattr]
    const keeps_groups: bool = false;

    /// Rename one placement (`RenameGroupCommand`).
    #[staticmethod]
    pub fn rename(group: &Bound<'_, PyAny>, name: &Bound<'_, PyAny>) -> PyResult<Self> {
        let groups = PyList::new(group.py(), [group])?;
        let value = Values::One(fval_for(Field::Name, name)?);
        Self::new("RenameGroupCommand", Field::Name, groups.as_any(), value, false)
    }

    /// Give each placement its own name, as one step (`names` pairs with
    /// `groups`; a repeated placement is not allowed).
    #[staticmethod]
    fn rename_each(groups: &Bound<'_, PyAny>, names: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut values = Vec::new();
        for name in names.try_iter()? {
            values.push(fval_for(Field::Name, &name?)?);
        }
        Self::new("RenameGroupCommand", Field::Name, groups, Values::Each(values), false)
    }

    /// Lock or unlock placements (`LockCommand`). Locking drops them from the selection.
    #[staticmethod]
    #[pyo3(signature = (groups, locked = true))]
    fn lock(groups: &Bound<'_, PyAny>, locked: bool) -> PyResult<Self> {
        Self::new("LockCommand", Field::Locked, groups, Values::One(FVal::Bool(locked)), locked)
    }

    /// Paint placements as a whole with one paint dict, or unpaint them
    /// with None (`SetGroupMaterialCommand`). Faces painted themselves keep
    /// their own paint; nothing in the mesh changes.
    #[staticmethod]
    fn paint(groups: &Bound<'_, PyAny>, material: &Bound<'_, PyAny>) -> PyResult<Self> {
        let py = groups.py();
        // An empty paint dict unpaints, as the Python command treated it.
        let material = if material.is_truthy()? { material.clone() } else { py.None().into_bound(py) };
        let value = Values::One(fval_for(Field::Material, &material)?);
        Self::new("SetGroupMaterialCommand", Field::Material, groups, value, false)
    }

    /// Whether the transaction names everything a feed consumer must
    /// re-read (the feed need not list the placements again). A whole-
    /// placement paint changes what the faces inside show, so its step
    /// stays incomplete, as the Python command's was.
    #[getter]
    fn journal_covers(&self) -> bool {
        self.covers
    }

    /// The placements this step writes.
    #[getter]
    fn targets<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        PyList::new(py, self.targets.iter().map(|t| t.bind(py)))
    }

    /// The single placement of a one-target step (feed and caller compatibility).
    #[getter]
    fn group(&self, py: Python<'_>) -> Option<Py<PyAny>> {
        (self.targets.len() == 1).then(|| self.targets[0].clone_ref(py))
    }

    /// The document field the step writes (`"name"`, `"locked"`, `"material"`).
    #[getter]
    fn field(&self) -> &'static str {
        self.field.name()
    }

    /// The value written to every target.
    #[getter]
    fn value<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        match &self.state.lock().unwrap().forward {
            Values::One(v) => fval_to_py(py, v.clone()),
            Values::Each(_) => Ok(py.None().into_bound(py)),
        }
    }

    #[pyo3(name = "do")]
    fn execute(&self, py: Python<'_>, scene: &Bound<'_, PyAny>) -> PyResult<()> {
        let forward = self.state.lock().unwrap().forward.clone();
        let inverse = self.apply(py, &forward)?;
        self.state.lock().unwrap().inverse = Some(inverse);
        if self.unselect {
            self.unselect_targets(py, scene)?;
        }
        bump(scene)
    }

    fn undo(&self, py: Python<'_>, scene: &Bound<'_, PyAny>) -> PyResult<()> {
        let inverse = self.state.lock().unwrap().inverse.clone();
        if let Some(values) = inverse {
            self.apply(py, &values)?;
        }
        bump(scene)
    }

    fn bind_transaction(slf: &Bound<'_, Self>, txn: &Bound<'_, PyAny>) -> PyResult<()> {
        slf.as_any().setattr("_doc_txn", txn)
    }

    /// What a transaction undo or redo still does beside the document.
    fn doc_after(&self, py: Python<'_>, scene: &Bound<'_, PyAny>, phase: &str) -> PyResult<()> {
        if phase == "redo" && self.unselect {
            self.unselect_targets(py, scene)?;
        }
        bump(scene)
    }

    fn __repr__(&self) -> String {
        format!("<NativeCommand {} ({} placements)>", self.history_name, self.targets.len())
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        for target in &self.targets {
            visit.call(target)?;
        }
        Ok(())
    }
}

/// Every class and function of the commands lane (docs/commands-port.md):
/// the one registration the shared `lib.rs` calls.
pub fn register_commands(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<NativeCommand>()?;
    super::steps::register(m)?;
    super::group_steps::register(m)?;
    super::paint_steps::register(m)?;
    super::tag_steps::register(m)?;
    super::purge::register(m)?;
    super::layer_steps::register(m)?;
    super::transform_steps::register(m)?;
    super::host_steps::register(m)?;
    super::draw_steps::register(m)?;
    super::erase_steps::register(m)?;
    super::vertex_steps::register(m)?;
    super::plane_steps::register(m)?;
    super::explode_steps::register(m)?;
    super::mesh_steps::register(m)?;
    crate::store::tally::register(m)
}
