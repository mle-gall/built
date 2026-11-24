// SPDX-License-Identifier: GPL-3.0-or-later
//! Native record steps (docs/commands-port.md): annotations, section planes,
//! guides, reference images, survey paths, saved scenes and sheets.
//!
//! Each command is a `RecordStep` subclass with the retired Python class's
//! name and constructor. Its operation runs in Rust over the document
//! records (the `presentation` and `reference_layers` kernels). Undo has no
//! hand-written inverse: History replays the step's transaction, and a direct
//! `undo` outside History replays the transaction the step opened for itself.
//! What a step does beside the document (the selection, the scene's version
//! counters) is one of a few shared host effects.
use crate::math::{xyz, Matrix4, Vec3};
use crate::presentation as present;
use crate::reference_layers as refs;
use glam::{DMat4, DVec3};
use super::py::{run_refills, PyTransaction};
use super::{drain_pending, lock, Key, Trash};
use pyo3::exceptions::{PyAttributeError, PyRuntimeError};
use pyo3::gc::PyVisit;
use pyo3::prelude::*;
use pyo3::PyClassInitializer;
use pyo3::types::{PyDict, PyList, PyTuple};
use pyo3::PyTraverseError;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

pub(super) type Obj<'py> = Bound<'py, PyAny>;
pub(super) type Run = for<'py> fn(&RecordStep, Python<'py>, &Obj<'py>) -> PyResult<()>;
/// A step's own after-effect, in place of the shared pick and bump.
#[derive(Clone, Copy)]
pub(super) struct Meshes(pub for<'py> fn(&RecordStep, Python<'py>, &Obj<'py>) -> PyResult<Vec<Obj<'py>>>);
#[derive(Clone, Copy)]
pub(super) struct After(pub for<'py> fn(&RecordStep, Python<'py>, &Obj<'py>, Option<&str>) -> PyResult<()>);

/// The counter a step bumps after it runs, and after a replay.
#[derive(Clone, Copy)]
enum Bump {
    Version,
    Sections,
    Catalog,
    /// The scene catalog, and the changed view's photo (its first argument).
    CatalogView,
}

/// The selection effect of a replay, on the step's first argument.
#[derive(Clone, Copy, PartialEq)]
enum Pick {
    Nothing,
    /// Redo drops every item from the selection (a delete).
    RedoDropAll,
    /// Undo drops the one item (an add).
    UndoDiscard,
    /// Redo selects only the item, undo drops it (a placement).
    RedoSelect,
    /// Redo drops the item when it ends hidden.
    RedoDropHidden,
    /// Every replay clears the paths' built surface triangles.
    ClearSurfaces,
}

#[pyclass(module = "ingetrazo_core", name = "RecordStep", subclass, frozen, dict, weakref)]
pub struct RecordStep {
    name: &'static str,
    run: Run,
    args: Vec<(&'static str, Py<PyAny>)>,
    #[pyo3(get)]
    keeps_paint: bool,
    #[pyo3(get)]
    keeps_rows: bool,
    #[pyo3(get)]
    keeps_groups: bool,
    hook: Option<After>,
    bump: Bump,
    pick: Pick,
    /// What a repeated execute reuses (a scene update's captured state).
    memory: Mutex<Vec<Py<PyAny>>>,
    /// The transaction a direct execute (outside History) opened.
    own: Mutex<Option<Py<PyAny>>>,
    /// Results an execute publishes under the retired class's attribute
    /// names (`_owner`, `index`): None until it runs.
    named: Mutex<Vec<(&'static str, Option<Py<PyAny>>)>>,
    /// The meshes whose store a History step enlists (a mesh step).
    meshes: Option<Meshes>,
    doc_undo: bool,
    /// History journals the enlisted stores (`_journal_geometry`).
    journal: AtomicBool,
    /// Set while a direct `undo` (outside History's replay) runs.
    direct: AtomicBool,
}

impl RecordStep {
    pub(super) fn new(name: &'static str, run: Run, args: Vec<(&'static str, Obj<'_>)>) -> Self {
        Self {
            name,
            run,
            args: args.into_iter().map(|(k, v)| (k, v.unbind())).collect(),
            keeps_paint: false,
            keeps_rows: false,
            keeps_groups: false,
            hook: None,
            bump: Bump::Version,
            pick: Pick::Nothing,
            memory: Mutex::new(Vec::new()),
            own: Mutex::new(None),
            named: Mutex::new(Vec::new()),
            meshes: None,
            doc_undo: true,
            journal: AtomicBool::new(false),
            direct: AtomicBool::new(false),
        }
    }
    fn paint(mut self) -> Self {
        self.keeps_paint = true;
        self
    }
    fn rows(mut self) -> Self {
        self.keeps_rows = true;
        self
    }
    pub(super) fn groups(mut self) -> Self {
        self.keeps_groups = true;
        self
    }
    /// A mesh step: History enlists these meshes' stores for the step.
    pub(super) fn on_meshes(mut self, meshes: Meshes) -> Self {
        self.meshes = Some(meshes);
        self
    }
    /// A step whose own inverse undoes what History does not journal
    /// (`doc_undo` false): History replays it by its transaction only when
    /// it journals the step's stores.
    pub(super) fn own_inverse(mut self) -> Self {
        self.doc_undo = false;
        self
    }
    /// As `own_inverse`, only when `own` holds (a step whose targets may
    /// or may not include mesh elements).
    pub(super) fn own_inverse_if(mut self, own: bool) -> Self {
        self.doc_undo = !own;
        self
    }
    /// Whether History journals the step's stores (else its snapshots undo them).
    pub(super) fn journalled(&self) -> bool {
        self.journal.load(Ordering::Relaxed)
    }
    /// Whether the replay running is a direct undo, outside History.
    pub(super) fn is_direct(&self) -> bool {
        self.direct.load(Ordering::Relaxed)
    }
    fn read_journal(slf: &Bound<'_, Self>) -> PyResult<()> {
        let flag = match slf.as_any().getattr_opt("_journal_geometry")? {
            Some(v) => v.is_truthy()?,
            None => false,
        };
        slf.get().journal.store(flag, Ordering::Relaxed);
        Ok(())
    }
    pub(super) fn hook(mut self, after: After) -> Self {
        self.hook = Some(after);
        self
    }
    /// What an earlier execute of this step kept for a repeated one.
    pub(super) fn remembered<'py>(&self, py: Python<'py>, i: usize) -> Option<Obj<'py>> {
        self.memory.lock().unwrap().get(i).map(|v| v.clone_ref(py).into_bound(py))
    }
    /// Attribute names an execute publishes results under.
    pub(super) fn publishes(self, names: &[&'static str]) -> Self {
        *self.named.lock().unwrap() = names.iter().map(|n| (*n, None)).collect();
        self
    }
    pub(super) fn publish(&self, name: &'static str, value: Obj<'_>) {
        if let Some(slot) = self.named.lock().unwrap().iter_mut().find(|(n, _)| *n == name) {
            slot.1 = Some(value.unbind());
        }
    }
    /// A published result (None until the step ran).
    pub(super) fn as_named<'py>(&self, py: Python<'py>, name: &str) -> PyResult<Obj<'py>> {
        match self.named.lock().unwrap().iter().find(|(k, _)| *k == name) {
            Some((_, v)) => Ok(v.as_ref().map_or_else(|| py.None().into_bound(py), |v| v.clone_ref(py).into_bound(py))),
            None => Err(PyAttributeError::new_err(name.to_owned())),
        }
    }
    pub(super) fn remember(&self, values: Vec<Obj<'_>>) {
        *self.memory.lock().unwrap() = values.into_iter().map(|v| v.unbind()).collect();
    }
    fn bumps(mut self, bump: Bump) -> Self {
        self.bump = bump;
        self
    }
    fn picks(mut self, pick: Pick) -> Self {
        self.pick = pick;
        self
    }
    pub(super) fn arg<'py>(&self, py: Python<'py>, i: usize) -> Obj<'py> {
        self.args[i].1.bind(py).clone()
    }
    pub(super) fn items<'py>(&self, py: Python<'py>, i: usize) -> PyResult<Vec<Obj<'py>>> {
        self.arg(py, i).try_iter()?.collect()
    }
    fn after(&self, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
        if let Some(After(hook)) = self.hook {
            return hook(self, py, scene, phase);
        }
        let target = self.arg(py, 0);
        let selection = || scene.getattr("selection");
        match (self.pick, phase) {
            (Pick::RedoDropAll, Some("redo")) => {
                selection()?.call_method1("difference_update", (&target,))?;
            }
            (Pick::UndoDiscard, Some("undo")) => {
                selection()?.call_method1("discard", (&target,))?;
            }
            (Pick::RedoSelect, Some("redo")) => {
                let sel = selection()?;
                sel.call_method0("clear")?;
                sel.call_method1("add", (&target,))?;
            }
            (Pick::RedoSelect, _) => {
                selection()?.call_method1("discard", (&target,))?;
            }
            (Pick::RedoDropHidden, Some("redo")) => {
                if target.getattr("hidden")?.is_truthy()? {
                    selection()?.call_method1("discard", (&target,))?;
                }
            }
            (Pick::ClearSurfaces, _) => {
                for path in target.try_iter()? {
                    path?.setattr("_surface_tris", py.None())?;
                }
            }
            _ => (),
        }
        bump(scene, self.bump, &target)
    }
}

fn bump(scene: &Obj<'_>, kind: Bump, target: &Obj<'_>) -> PyResult<()> {
    match kind {
        Bump::Version => version(scene),
        Bump::Sections => scene.call_method0("bump_sections").map(|_| ()),
        Bump::Catalog => present::bump_scene_catalog(scene, None),
        Bump::CatalogView => present::bump_scene_catalog(scene, Some(target)),
    }
}

pub(super) fn version(scene: &Obj<'_>) -> PyResult<()> {
    let v: i64 = scene.getattr("version")?.extract()?;
    scene.setattr("version", v + 1)
}

fn sections(scene: &Obj<'_>) -> PyResult<()> {
    scene.call_method0("bump_sections").map(|_| ())
}

fn rejected(py: Python<'_>, why: &Obj<'_>) -> PyErr {
    match py
        .import("core.history_commands")
        .and_then(|m| m.getattr("CommandRejected"))
        .and_then(|c| c.call1((why,)))
    {
        Ok(e) => PyErr::from_value(e),
        Err(e) => e,
    }
}

fn discard<'py>(scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    scene.getattr("selection")?.getattr("discard")
}

fn vector<'py>(py: Python<'py>, v: &Obj<'py>) -> PyResult<Obj<'py>> {
    Ok(Bound::new(py, Vec3::from(xyz(v)?))?.into_any())
}

fn optional_vector<'py>(py: Python<'py>, v: Option<Obj<'py>>) -> PyResult<Obj<'py>> {
    match v {
        Some(v) if !v.is_none() => vector(py, &v),
        _ => Ok(py.None().into_bound(py)),
    }
}

fn list<'py>(items: &Obj<'py>) -> PyResult<Obj<'py>> {
    Ok(PyList::new(items.py(), items.try_iter()?.collect::<PyResult<Vec<_>>>()?)?.into_any())
}

/// The rigid rotation about `centre`/`axis` that the Python helper
/// `rotation_matrix` built, operation for operation.
fn rotation<'py>(py: Python<'py>, centre: &Obj<'py>, axis: &Obj<'py>, deg: f64) -> PyResult<Obj<'py>> {
    let c = xyz(centre)?;
    let a = xyz(axis)?.normalize_or_zero().normalize_or_zero();
    let mut m = DMat4::IDENTITY;
    m *= DMat4::from_translation(c);
    if a != DVec3::ZERO {
        m *= DMat4::from_axis_angle(a, deg.to_radians());
    }
    m *= DMat4::from_translation(-c);
    Ok(Bound::new(py, Matrix4::from(m))?.into_any())
}

#[pymethods]
#[allow(non_upper_case_globals)]
impl RecordStep {
    /// Undone and redone by the step's document transaction, unless it
    /// keeps a real inverse for meshes History does not journal.
    #[getter]
    fn doc_undo(&self) -> bool {
        self.doc_undo
    }
    #[getter]
    fn history_name(&self) -> &'static str {
        self.name
    }

    /// The constructor's arguments, under the retired class's names.
    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        if let Some((_, v)) = self.args.iter().find(|(k, _)| *k == name) {
            return Ok(v.clone_ref(py));
        }
        match self.named.lock().unwrap().iter().find(|(k, _)| *k == name) {
            Some((_, v)) => Ok(v.as_ref().map_or_else(|| py.None(), |v| v.clone_ref(py))),
            None => Err(PyAttributeError::new_err(name.to_owned())),
        }
    }

    #[pyo3(name = "do")]
    fn execute(slf: &Bound<'_, Self>, scene: &Obj<'_>) -> PyResult<()> {
        let py = slf.py();
        let me = slf.get();
        let doc = py.import("core.doc_model")?;
        // A stand-in scene without a document (a tool's preview, a test
        // fixture) runs the operation alone: there is nothing to journal.
        let key: Option<Key> = match scene.getattr_opt("_root")? {
            Some(root) => Some(root.getattr("_doc_key")?.extract()?),
            None => None,
        };
        let outer = lock().depth();
        if key.is_none() && outer == 0 {
            Self::read_journal(slf)?;
            return (me.run)(me, py, scene);
        }
        if outer == 0 {
            // A direct call: the step's own transaction.
            doc.call_method1("begin", (key,))?;
        } else {
            // Inside another step (History, a compound): a savepoint whose
            // ops this step keeps a copy of, for its own direct undo.
            doc.call_method0("flush")?;
            lock().begin(key).ok_or_else(|| PyRuntimeError::new_err("Document writer reservation lost"))?;
        }
        let pins = lock().open.as_ref().map_or(0, |o| o.pins.len());
        Self::read_journal(slf)?;
        let result = (me.run)(me, py, scene);
        let own = match (&result, outer) {
            (Ok(()), 0) => {
                let txn = doc.call_method0("commit")?;
                (!txn.is_none()).then(|| txn.unbind())
            }
            (Ok(()), _) => {
                doc.call_method0("flush")?;
                let (ops, held) = {
                    let mut d = lock();
                    let level = d.open.as_ref().map(|o| {
                        let mark = o.marks.last().copied().unwrap_or(0);
                        (o.ops[mark..].to_vec(), o.pins[pins.min(o.pins.len())..].iter().map(|p| p.clone_ref(py)).collect::<Vec<_>>())
                    });
                    d.commit();
                    level.unwrap_or_default()
                };
                Some(Py::new(py, PyTransaction::from_parts(ops, held))?.into_any())
            }
            (Err(_), 0) => {
                doc.call_method0("rollback")?;
                None
            }
            (Err(_), _) => {
                let mut trash = Trash::default();
                let r = { lock().rollback(py, &mut trash) };
                drop(trash);
                drain_pending();
                run_refills(py)?;
                r.map_err(PyRuntimeError::new_err)?;
                None
            }
        };
        *me.own.lock().unwrap() = own;
        result
    }

    /// A direct undo: the transaction this step wrote, backwards.
    fn undo(slf: &Bound<'_, Self>, scene: &Obj<'_>) -> PyResult<()> {
        let py = slf.py();
        let me = slf.get();
        let own = me.own.lock().unwrap().as_ref().map(|t| t.clone_ref(py));
        let txn = match own {
            Some(t) => Some(t.into_bound(py)),
            None => slf.as_any().getattr_opt("_doc_txn")?.filter(|t| !t.is_none()),
        };
        if let Some(txn) = txn {
            py.import("core.doc_model")?.call_method0("flush")?;
            txn.call_method0("undo")?;
        }
        Self::read_journal(slf)?;
        me.direct.store(true, Ordering::Relaxed);
        let r = me.after(py, scene, Some("undo"));
        me.direct.store(false, Ordering::Relaxed);
        r
    }

    fn bind_transaction(slf: &Bound<'_, Self>, txn: &Obj<'_>) -> PyResult<()> {
        slf.as_any().setattr("_doc_txn", txn)
    }

    /// What a transaction undo or redo still does beside the document.
    #[pyo3(signature = (scene, phase = None))]
    fn doc_after(slf: &Bound<'_, Self>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
        Self::read_journal(slf)?;
        slf.get().after(slf.py(), scene, phase)
    }

    /// The meshes a mesh step's History transaction enlists.
    fn geometry_meshes<'py>(&self, py: Python<'py>, scene: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
        match self.meshes {
            Some(Meshes(f)) => PyTuple::new(py, f(self, py, scene)?),
            None => PyTuple::new(py, [scene.getattr("mesh")?]),
        }
    }

    fn __repr__(&self) -> String {
        format!("<{} (native)>", self.name)
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        for (_, v) in &self.args {
            visit.call(v)?;
        }
        if let Ok(memory) = self.memory.try_lock() {
            for v in memory.iter() {
                visit.call(v)?;
            }
        }
        if let Ok(named) = self.named.try_lock() {
            for v in named.iter().filter_map(|(_, v)| v.as_ref()) {
                visit.call(v)?;
            }
        }
        if let Ok(own) = self.own.try_lock() {
            if let Some(t) = own.as_ref() {
                visit.call(t)?;
            }
        }
        Ok(())
    }
}

// ------------------------------------------------------------ operations

fn add_to(attr: &'static str) -> Run {
    // One function per list keeps `Run` a plain function pointer.
    match attr {
        "text_labels" => |s, py, scene| { present::add_item(&scene.getattr("text_labels")?, &s.arg(py, 0))?; version(scene) },
        "guides" => |s, py, scene| { present::add_item(&scene.getattr("guides")?, &s.arg(py, 0))?; version(scene) },
        "compositions" => |s, py, scene| { present::add_item(&scene.getattr("compositions")?, &s.arg(py, 0))?; version(scene) },
        "geo_paths" => |s, py, scene| { present::add_item(&scene.getattr("geo_paths")?, &s.arg(py, 0))?; version(scene) },
        _ => unreachable!("no such list"),
    }
}

fn delete_from(attr: &'static str) -> Run {
    match attr {
        "dimensions" => |s, py, scene| { present::delete_items(&scene.getattr("dimensions")?, s.items(py, 0)?, Some(&discard(scene)?), true)?; version(scene) },
        "text_labels" => |s, py, scene| { present::delete_items(&scene.getattr("text_labels")?, s.items(py, 0)?, Some(&discard(scene)?), true)?; version(scene) },
        "guides" => |s, py, scene| { present::delete_items(&scene.getattr("guides")?, s.items(py, 0)?, Some(&discard(scene)?), true)?; version(scene) },
        "image_planes" => |s, py, scene| { present::delete_items(&scene.getattr("image_planes")?, s.items(py, 0)?, Some(&discard(scene)?), true)?; version(scene) },
        "geo_paths" => |s, py, scene| { present::delete_items(&scene.getattr("geo_paths")?, s.items(py, 0)?, Some(&discard(scene)?), true)?; version(scene) },
        "section_planes" => |s, py, scene| { present::delete_items(&scene.getattr("section_planes")?, s.items(py, 0)?, Some(&discard(scene)?), true)?; sections(scene) },
        "geo_points" => |s, py, scene| { present::delete_items(&scene.getattr("geo_points")?, s.items(py, 0)?, None, false)?; version(scene) },
        "saved_views" => |s, py, scene| { present::delete_items(&scene.getattr("saved_views")?, s.items(py, 0)?, None, true)?; present::bump_scene_catalog(scene, None) },
        _ => unreachable!("no such list"),
    }
}

fn run_add_dimension(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let dim = s.arg(py, 0);
    if dim.hasattr("bind")? {
        dim.call_method1("bind", (scene,))?;
    }
    present::add_item(&scene.getattr("dimensions")?, &dim)?;
    version(scene)
}

fn run_translate_offset(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    present::translate_items(s.items(py, 0)?, "offset", &s.arg(py, 1))?;
    version(scene)
}

fn run_translate_point(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    present::translate_items(s.items(py, 0)?, "point", &s.arg(py, 1))?;
    sections(scene)
}

fn run_translate_origin(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    present::translate_items(s.items(py, 0)?, "origin", &s.arg(py, 1))?;
    version(scene)
}

fn run_shift_guides(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let delta = s.arg(py, 1);
    for g in s.items(py, 0)? {
        present::shift_guide(&g, &delta)?;
    }
    version(scene)
}

fn run_rotate_guides(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let states = s
        .items(py, 0)?
        .iter()
        .map(|g| present::guide_state(g)?.into_pyobject(py).map(|t| t.into_any()))
        .collect::<PyResult<Vec<_>>>()?;
    let m = rotation(py, &s.arg(py, 1), &s.arg(py, 2), s.arg(py, 3).extract()?)?;
    for state in states {
        present::place_guide(&state, Some(&m))?;
    }
    version(scene)
}

fn shift_dimension(dim: &Obj<'_>, mode: &Obj<'_>, step: &Obj<'_>) -> PyResult<()> {
    let line = mode.eq("line")?;
    let free = |a: &str, b: &str| -> PyResult<bool> {
        Ok(!line && dim.getattr(a)?.is_none() && dim.getattr(b)?.is_none())
    };
    present::shift_dimension(dim, line, step, free("anchor_a", "attachment_a")?, free("anchor_b", "attachment_b")?)
}

fn run_shift_dimensions(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let delta = s.arg(py, 1);
    for pair in s.arg(py, 0).cast_into::<PyDict>()?.iter() {
        shift_dimension(&pair.0, &pair.1, &delta)?;
    }
    version(scene)
}

fn run_set_text(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    present::set_text(&s.arg(py, 0), &s.arg(py, 1))?;
    version(scene)
}

fn run_place_section(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let plane = s.arg(py, 0);
    present::activate_section(scene, &plane, true)?;
    let sel = scene.getattr("selection")?;
    sel.call_method0("clear")?;
    sel.call_method1("add", (&plane,))?;
    sections(scene)
}

fn run_flip_section(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    s.arg(py, 0).call_method0("flip")?;
    sections(scene)
}

fn run_activate_section(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    present::activate_section(scene, &s.arg(py, 0), false)?;
    sections(scene)
}

fn run_section_mode(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let plane = s.arg(py, 0);
    present::section_mode(scene, &plane, s.arg(py, 1).extract()?, s.arg(py, 2).extract()?)?;
    if plane.getattr("hidden")?.is_truthy()? {
        discard(scene)?.call1((&plane,))?;
    }
    sections(scene)
}

fn run_section_fill(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    present::set_section_fill(&s.arg(py, 0), &s.arg(py, 1))?;
    sections(scene)
}

fn run_section_label(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    present::set_section_label(&s.arg(py, 0), &s.arg(py, 1), &s.arg(py, 2))?;
    sections(scene)
}

fn run_rotate_sections(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let m = rotation(py, &s.arg(py, 1), &s.arg(py, 2), s.arg(py, 3).extract()?)?;
    present::rotate_sections(s.items(py, 0)?, &m)?;
    sections(scene)
}

fn run_place_view(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    present::add_item(&s.arg(py, 0).getattr("frames")?, &s.arg(py, 1))?;
    version(scene)
}

fn run_sheet_inner(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    s.arg(py, 0).call_method0("do")?;
    version(scene)
}

fn run_insert_sheet(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let sheets = scene.getattr("compositions")?;
    let (sheet, index) = (s.arg(py, 0), s.arg(py, 1));
    present::add_item(&sheets, &sheet)?;
    if !index.is_none() {
        present::move_item(&sheets, &sheet, &index, true, false)?;
    }
    version(scene)
}

fn run_remove_sheet(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    present::remove_item(&scene.getattr("compositions")?, &s.arg(py, 0))?;
    version(scene)
}

fn run_move_sheet(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    present::move_item(&scene.getattr("compositions")?, &s.arg(py, 0), &s.arg(py, 1), true, false)?;
    version(scene)
}

fn run_create_plan(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    present::make_plan(scene, &s.arg(py, 0), &s.arg(py, 1), &s.arg(py, 2), s.arg(py, 3).extract()?)?;
    version(scene)
}

fn run_insert_scene(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    present::insert_scene(scene, &s.arg(py, 0), &s.arg(py, 1))?;
    present::bump_scene_catalog(scene, None)
}

fn run_update_scene(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let view = s.arg(py, 0);
    // A repeated execute writes the values the first one captured.
    let after = s.memory.lock().unwrap().first().map(|v| v.clone_ref(py).into_bound(py));
    let after = after.unwrap_or_else(|| py.None().into_bound(py));
    let (_, after) = present::update_scene(scene, &view, &s.arg(py, 1), &s.arg(py, 2), &after)?;
    *s.memory.lock().unwrap() = vec![after.unbind()];
    present::bump_scene_catalog(scene, Some(&view))
}

fn run_scene_tree(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let memory: Vec<Obj<'_>> = s.memory.lock().unwrap().iter().map(|v| v.clone_ref(py).into_bound(py)).collect();
    let none = py.None().into_bound(py);
    let (before, after) = (memory.first().unwrap_or(&none), memory.get(1).unwrap_or(&none));
    match present::edit_view_tree(scene, &s.arg(py, 0), before, after)? {
        None => Err(rejected(py, pyo3::types::PyString::new(py, "Nothing to change").as_any())),
        Some((before, after)) => {
            *s.memory.lock().unwrap() = vec![before.unbind(), after.unbind()];
            present::bump_scene_catalog(scene, None)
        }
    }
}

fn run_rename_scene(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let name = s.arg(py, 1);
    if present::rename_view(scene, &s.arg(py, 0), &name)?.is_none() {
        return Err(rejected(py, &name));
    }
    present::bump_scene_catalog(scene, None)
}

fn run_move_scene(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let view = s.arg(py, 0);
    if present::move_item(&scene.getattr("saved_views")?, &view, &s.arg(py, 1), false, true)?.is_none() {
        return Err(rejected(py, &view.getattr("name")?));
    }
    present::bump_scene_catalog(scene, None)
}

fn run_set_direction(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    present::set_direction(&s.arg(py, 0), &s.arg(py, 1))?;
    version(scene)
}

fn run_add_image(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let image = s.arg(py, 0);
    refs::add_image(scene, &image)?;
    let sel = scene.getattr("selection")?;
    sel.call_method0("clear")?;
    sel.call_method1("add", (&image,))?;
    version(scene)
}

fn run_image_frame(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    refs::set_image_frame(&s.arg(py, 0), &s.arg(py, 1))?;
    version(scene)
}

fn run_image_opacity(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    s.arg(py, 0).setattr("opacity", s.arg(py, 1))?;
    version(scene)
}

fn run_rotate_images(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let m = rotation(py, &s.arg(py, 1), &s.arg(py, 2), s.arg(py, 3).extract()?)?;
    refs::transform_images(s.items(py, 0)?, &m)?;
    version(scene)
}

fn run_scale_images(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    // The grip box's frame comes from `core.axes`; the matrix is the
    // Python helper's, as the tools' previews build it.
    let m = py
        .import("core.history_commands")?
        .call_method1("scale_matrix", (s.arg(py, 1), s.arg(py, 2), s.arg(py, 3)))?;
    refs::transform_images(s.items(py, 0)?, &m)?;
    version(scene)
}

fn run_toggle_paths(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    refs::toggle_paths(s.items(py, 0)?)?;
    version(scene)
}

fn run_path_surfaces(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    refs::path_surfaces(s.items(py, 0)?, &s.arg(py, 1))?;
    version(scene)
}

fn run_move_node(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    refs::move_node(&s.arg(py, 0), &s.arg(py, 1), &s.arg(py, 2))?;
    version(scene)
}

// ------------------------------------------------------------ the classes

macro_rules! step {
    ($ty:ident, $name:literal) => {
        #[pyclass(module = "ingetrazo_core", name = $name, extends = RecordStep, frozen)]
        pub struct $ty;
    };
}

macro_rules! steps {
    ($($ty:ident = $name:literal;)*) => {
        $(step!($ty, $name);)*
        pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
            m.add_class::<RecordStep>()?;
            $(m.add_class::<$ty>()?;)*
            Ok(())
        }
    };
}

steps! {
    AddDimension = "AddDimensionCommand";
    DeleteDimensions = "DeleteDimensionsCommand";
    AddTextLabel = "AddTextLabelCommand";
    MoveTextLabels = "MoveTextLabelsCommand";
    MoveGuides = "MoveGuidesCommand";
    RotateGuides = "RotateGuidesCommand";
    MoveDimensions = "MoveDimensionsCommand";
    EditTextLabel = "EditTextLabelCommand";
    EditDimensionText = "EditDimensionTextCommand";
    DeleteTextLabels = "DeleteTextLabelsCommand";
    AddGuide = "AddGuideCommand";
    PlaceSectionPlane = "PlaceSectionPlaneCommand";
    DeleteSectionPlanes = "DeleteSectionPlanesCommand";
    ReverseSectionPlane = "ReverseSectionPlaneCommand";
    SetActiveSection = "SetActiveSectionCommand";
    SetSectionMode = "SetSectionModeCommand";
    SetSectionFill = "SetSectionFillCommand";
    RenameSectionPlane = "RenameSectionPlaneCommand";
    AddSheet = "AddSheetCommand";
    PlaceView = "PlaceViewCommand";
    Sheet = "SheetCommand";
    InsertSheet = "InsertSheetCommand";
    RemoveSheet = "RemoveSheetCommand";
    MoveSheet = "MoveSheetCommand";
    CreatePlan = "CreatePlanCommand";
    AddScene = "AddSceneCommand";
    DeleteScenes = "DeleteScenesCommand";
    UpdateScene = "UpdateSceneCommand";
    SceneTree = "SceneTreeCommand";
    RenameScene = "RenameSceneCommand";
    MoveScene = "MoveSceneCommand";
    MoveSectionPlanes = "MoveSectionPlanesCommand";
    RotateSectionPlanes = "RotateSectionPlanesCommand";
    ChangeGuide = "ChangeGuideCommand";
    DeleteGuides = "DeleteGuidesCommand";
    AddImagePlane = "AddImagePlaneCommand";
    DeleteImagePlanes = "DeleteImagePlanesCommand";
    TransformImagePlane = "TransformImagePlaneCommand";
    SetImagePlaneOpacity = "SetImagePlaneOpacityCommand";
    MoveImagePlanes = "MoveImagePlanesCommand";
    RotateImagePlanes = "RotateImagePlanesCommand";
    ScaleImagePlanes = "ScaleImagePlanesCommand";
    AddGeoPath = "AddGeoPathCommand";
    DeleteGeoPoints = "DeleteGeoPointsCommand";
    DeleteGeoPaths = "DeleteGeoPathsCommand";
    ToggleGeoPathClosed = "ToggleGeoPathClosedCommand";
    SetGeoPathSurface = "SetGeoPathSurfaceCommand";
    MoveGeoPathNode = "MoveGeoPathNodeCommand";
}

/// A delete of `items` (copied) from one scene list.
fn deleting<'py>(name: &'static str, attr: &'static str, aliases: [&'static str; 2], items: &Obj<'py>) -> PyResult<RecordStep> {
    let items = list(items)?;
    Ok(RecordStep::new(name, delete_from(attr), vec![(aliases[0], items.clone()), (aliases[1], items)]).picks(Pick::RedoDropAll))
}

#[pymethods]
impl AddDimension {
    #[new]
    fn new(dimension: Obj<'_>) -> PyClassInitializer<Self> {
        PyClassInitializer::from(RecordStep::new("AddDimensionCommand", run_add_dimension, vec![("dimension", dimension)]).rows()).add_subclass(Self)
    }
}

#[pymethods]
impl DeleteDimensions {
    #[new]
    fn new(dimensions: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        Ok(PyClassInitializer::from(deleting("DeleteDimensionsCommand", "dimensions", ["dimensions", "_dims"], &dimensions)?).add_subclass(Self))
    }
}

#[pymethods]
impl AddTextLabel {
    #[new]
    fn new(label: Obj<'_>) -> PyClassInitializer<Self> {
        PyClassInitializer::from(RecordStep::new("AddTextLabelCommand", add_to("text_labels"), vec![("label", label)]).rows().picks(Pick::UndoDiscard)).add_subclass(Self)
    }
}

#[pymethods]
impl MoveTextLabels {
    #[new]
    fn new(py: Python<'_>, labels: Obj<'_>, delta: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let (labels, delta) = (list(&labels)?, vector(py, &delta)?);
        Ok(PyClassInitializer::from(RecordStep::new("MoveTextLabelsCommand", run_translate_offset, vec![("_labels", labels), ("_delta", delta)]).paint()).add_subclass(Self))
    }
}

#[pymethods]
impl MoveGuides {
    #[new]
    fn new(py: Python<'_>, guides: Obj<'_>, delta: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let (guides, delta) = (list(&guides)?, vector(py, &delta)?);
        Ok(PyClassInitializer::from(RecordStep::new("MoveGuidesCommand", run_shift_guides, vec![("_guides", guides), ("_delta", delta)]).paint().rows()).add_subclass(Self))
    }
    /// One step of the move: the tool's live preview uses it too.
    #[staticmethod]
    fn shift(guide: Obj<'_>, step: Obj<'_>) -> PyResult<()> {
        present::shift_guide(&guide, &step)
    }
}

#[pymethods]
impl RotateGuides {
    #[new]
    fn new(py: Python<'_>, guides: Obj<'_>, centre: Obj<'_>, axis: Obj<'_>, deg: f64) -> PyResult<PyClassInitializer<Self>> {
        let args = vec![
            ("_guides", list(&guides)?),
            ("centre", vector(py, &centre)?),
            ("axis", vector(py, &axis)?),
            ("deg", deg.into_pyobject(py)?.into_any()),
        ];
        Ok(PyClassInitializer::from(RecordStep::new("RotateGuidesCommand", run_rotate_guides, args).paint().rows()).add_subclass(Self))
    }
    /// What a rotation changes, copied: the tool's preview keeps it.
    #[staticmethod]
    fn state<'py>(py: Python<'py>, guide: Obj<'py>) -> PyResult<Obj<'py>> {
        Ok(present::guide_state(&guide)?.into_pyobject(py)?.into_any())
    }
    /// Put a guide at `m` applied to its `state` (None: as it was).
    #[staticmethod]
    #[pyo3(signature = (state, m = None))]
    fn place(state: Obj<'_>, m: Option<Obj<'_>>) -> PyResult<()> {
        present::place_guide(&state, m.as_ref().filter(|m| !m.is_none()))
    }
}

#[pymethods]
impl MoveDimensions {
    #[new]
    fn new(py: Python<'_>, modes: Obj<'_>, delta: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let modes = py.import("builtins")?.getattr("dict")?.call1((modes,))?;
        Ok(PyClassInitializer::from(RecordStep::new("MoveDimensionsCommand", run_shift_dimensions, vec![("_modes", modes), ("_delta", vector(py, &delta)?)]).paint()).add_subclass(Self))
    }
    /// One step of the move: the tool's live preview uses it too.
    #[staticmethod]
    fn shift(dim: Obj<'_>, mode: Obj<'_>, step: Obj<'_>) -> PyResult<()> {
        shift_dimension(&dim, &mode, &step)
    }
}

#[pymethods]
impl EditTextLabel {
    #[new]
    fn new(label: Obj<'_>, text: Obj<'_>) -> PyClassInitializer<Self> {
        PyClassInitializer::from(RecordStep::new("EditTextLabelCommand", run_set_text, vec![("label", label), ("_new", text)])).add_subclass(Self)
    }
}

#[pymethods]
impl EditDimensionText {
    #[new]
    fn new(py: Python<'_>, dim: Obj<'_>, text: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let text = if text.is_truthy()? { text } else { py.None().into_bound(py) };
        Ok(PyClassInitializer::from(RecordStep::new("EditDimensionTextCommand", run_set_text, vec![("dim", dim), ("_new", text)])).add_subclass(Self))
    }
}

#[pymethods]
impl DeleteTextLabels {
    #[new]
    fn new(labels: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        Ok(PyClassInitializer::from(deleting("DeleteTextLabelsCommand", "text_labels", ["labels", "_labels"], &labels)?).add_subclass(Self))
    }
}

#[pymethods]
impl AddGuide {
    #[new]
    fn new(guide: Obj<'_>) -> PyClassInitializer<Self> {
        PyClassInitializer::from(RecordStep::new("AddGuideCommand", add_to("guides"), vec![("guide", guide)]).paint().rows().picks(Pick::UndoDiscard)).add_subclass(Self)
    }
}

#[pymethods]
impl PlaceSectionPlane {
    #[new]
    fn new(plane: Obj<'_>) -> PyClassInitializer<Self> {
        PyClassInitializer::from(RecordStep::new("PlaceSectionPlaneCommand", run_place_section, vec![("plane", plane)]).bumps(Bump::Sections).picks(Pick::RedoSelect)).add_subclass(Self)
    }
}

#[pymethods]
impl DeleteSectionPlanes {
    #[new]
    fn new(planes: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        Ok(PyClassInitializer::from(deleting("DeleteSectionPlanesCommand", "section_planes", ["planes", "_planes"], &planes)?.bumps(Bump::Sections)).add_subclass(Self))
    }
}

#[pymethods]
impl ReverseSectionPlane {
    #[new]
    fn new(plane: Obj<'_>) -> PyClassInitializer<Self> {
        PyClassInitializer::from(RecordStep::new("ReverseSectionPlaneCommand", run_flip_section, vec![("plane", plane)]).bumps(Bump::Sections)).add_subclass(Self)
    }
}

#[pymethods]
impl SetActiveSection {
    #[new]
    fn new(plane: Obj<'_>) -> PyClassInitializer<Self> {
        PyClassInitializer::from(RecordStep::new("SetActiveSectionCommand", run_activate_section, vec![("plane", plane)]).bumps(Bump::Sections)).add_subclass(Self)
    }
}

#[pymethods]
impl SetSectionMode {
    #[new]
    fn new(py: Python<'_>, plane: Obj<'_>, cut: Obj<'_>, frame: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let flag = |v: &Obj<'_>| -> PyResult<Obj<'_>> { Ok(pyo3::types::PyBool::new(py, v.is_truthy()?).to_owned().into_any()) };
        let args = vec![("plane", plane), ("cut", flag(&cut)?), ("frame", flag(&frame)?)];
        Ok(PyClassInitializer::from(RecordStep::new("SetSectionModeCommand", run_section_mode, args).bumps(Bump::Sections).picks(Pick::RedoDropHidden)).add_subclass(Self))
    }
}

#[pymethods]
impl SetSectionFill {
    #[new]
    fn new(py: Python<'_>, plane: Obj<'_>, fill: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let fill = if fill.is_truthy()? { py.import("builtins")?.getattr("dict")?.call1((fill,))? } else { py.None().into_bound(py) };
        Ok(PyClassInitializer::from(RecordStep::new("SetSectionFillCommand", run_section_fill, vec![("plane", plane), ("fill", fill)]).bumps(Bump::Sections)).add_subclass(Self))
    }
}

#[pymethods]
impl RenameSectionPlane {
    #[new]
    fn new(plane: Obj<'_>, name: Obj<'_>, symbol: Obj<'_>) -> PyClassInitializer<Self> {
        PyClassInitializer::from(RecordStep::new("RenameSectionPlaneCommand", run_section_label, vec![("plane", plane), ("name", name), ("symbol", symbol)]).bumps(Bump::Sections)).add_subclass(Self)
    }
}

#[pymethods]
impl AddSheet {
    #[new]
    fn new(sheet: Obj<'_>) -> PyClassInitializer<Self> {
        PyClassInitializer::from(RecordStep::new("AddSheetCommand", add_to("compositions"), vec![("sheet", sheet)])).add_subclass(Self)
    }
}

#[pymethods]
impl PlaceView {
    #[new]
    fn new(sheet: Obj<'_>, frame: Obj<'_>) -> PyClassInitializer<Self> {
        PyClassInitializer::from(RecordStep::new("PlaceViewCommand", run_place_view, vec![("sheet", sheet), ("frame", frame)])).add_subclass(Self)
    }
}

#[pymethods]
impl Sheet {
    #[new]
    fn new(inner: Obj<'_>) -> PyClassInitializer<Self> {
        PyClassInitializer::from(RecordStep::new("SheetCommand", run_sheet_inner, vec![("inner", inner)])).add_subclass(Self)
    }
}

#[pymethods]
impl InsertSheet {
    #[new]
    #[pyo3(signature = (sheet, index = None))]
    fn new(py: Python<'_>, sheet: Obj<'_>, index: Option<Obj<'_>>) -> PyClassInitializer<Self> {
        let index = index.unwrap_or_else(|| py.None().into_bound(py));
        PyClassInitializer::from(RecordStep::new("InsertSheetCommand", run_insert_sheet, vec![("sheet", sheet), ("index", index)])).add_subclass(Self)
    }
}

#[pymethods]
impl RemoveSheet {
    #[new]
    fn new(sheet: Obj<'_>) -> PyClassInitializer<Self> {
        PyClassInitializer::from(RecordStep::new("RemoveSheetCommand", run_remove_sheet, vec![("sheet", sheet)])).add_subclass(Self)
    }
}

#[pymethods]
impl MoveSheet {
    #[new]
    fn new(sheet: Obj<'_>, index: Obj<'_>) -> PyClassInitializer<Self> {
        PyClassInitializer::from(RecordStep::new("MoveSheetCommand", run_move_sheet, vec![("sheet", sheet), ("index", index)])).add_subclass(Self)
    }
}

#[pymethods]
impl CreatePlan {
    #[new]
    #[pyo3(signature = (plane, view, sheet = None, add_plane = true))]
    fn new(py: Python<'_>, plane: Obj<'_>, view: Obj<'_>, sheet: Option<Obj<'_>>, add_plane: bool) -> PyResult<PyClassInitializer<Self>> {
        let sheet = sheet.unwrap_or_else(|| py.None().into_bound(py));
        let add = pyo3::types::PyBool::new(py, add_plane).to_owned().into_any();
        let args = vec![("plane", plane), ("view", view), ("sheet", sheet), ("add_plane", add)];
        Ok(PyClassInitializer::from(RecordStep::new("CreatePlanCommand", run_create_plan, args).picks(Pick::UndoDiscard)).add_subclass(Self))
    }
}

#[pymethods]
impl AddScene {
    #[new]
    #[pyo3(signature = (view, index = None))]
    fn new(py: Python<'_>, view: Obj<'_>, index: Option<Obj<'_>>) -> PyClassInitializer<Self> {
        let index = index.unwrap_or_else(|| py.None().into_bound(py));
        PyClassInitializer::from(RecordStep::new("AddSceneCommand", run_insert_scene, vec![("view", view), ("index", index)]).bumps(Bump::Catalog)).add_subclass(Self)
    }
}

#[pymethods]
impl DeleteScenes {
    #[new]
    fn new(views: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let views = list(&views)?;
        Ok(PyClassInitializer::from(RecordStep::new("DeleteScenesCommand", delete_from("saved_views"), vec![("views", views)]).bumps(Bump::Catalog)).add_subclass(Self))
    }
}

#[pymethods]
impl UpdateScene {
    #[new]
    #[pyo3(signature = (view, camera, style = None))]
    fn new(py: Python<'_>, view: Obj<'_>, camera: Obj<'_>, style: Option<Obj<'_>>) -> PyClassInitializer<Self> {
        let style = style.unwrap_or_else(|| py.None().into_bound(py));
        let args = vec![("view", view), ("camera", camera), ("style", style)];
        PyClassInitializer::from(RecordStep::new("UpdateSceneCommand", run_update_scene, args).bumps(Bump::CatalogView)).add_subclass(Self)
    }
}

#[pymethods]
impl SceneTree {
    #[new]
    fn new(change: Obj<'_>) -> PyClassInitializer<Self> {
        PyClassInitializer::from(RecordStep::new("SceneTreeCommand", run_scene_tree, vec![("change", change)]).bumps(Bump::Catalog)).add_subclass(Self)
    }
}

#[pymethods]
impl RenameScene {
    #[new]
    fn new(view: Obj<'_>, name: Obj<'_>) -> PyClassInitializer<Self> {
        PyClassInitializer::from(RecordStep::new("RenameSceneCommand", run_rename_scene, vec![("view", view), ("name", name)]).paint().bumps(Bump::Catalog)).add_subclass(Self)
    }
}

#[pymethods]
impl MoveScene {
    #[new]
    fn new(py: Python<'_>, view: Obj<'_>, index: i64) -> PyResult<PyClassInitializer<Self>> {
        let index = index.into_pyobject(py)?.into_any();
        Ok(PyClassInitializer::from(RecordStep::new("MoveSceneCommand", run_move_scene, vec![("view", view), ("index", index)]).paint().bumps(Bump::Catalog)).add_subclass(Self))
    }
}

#[pymethods]
impl MoveSectionPlanes {
    #[new]
    fn new(py: Python<'_>, planes: Obj<'_>, delta: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let args = vec![("_planes", list(&planes)?), ("delta", vector(py, &delta)?)];
        Ok(PyClassInitializer::from(RecordStep::new("MoveSectionPlanesCommand", run_translate_point, args).paint().bumps(Bump::Sections)).add_subclass(Self))
    }
}

#[pymethods]
impl RotateSectionPlanes {
    #[new]
    fn new(py: Python<'_>, planes: Obj<'_>, centre: Obj<'_>, axis: Obj<'_>, deg: f64) -> PyResult<PyClassInitializer<Self>> {
        let args = vec![
            ("_planes", list(&planes)?),
            ("centre", vector(py, &centre)?),
            ("axis", vector(py, &axis)?),
            ("deg", deg.into_pyobject(py)?.into_any()),
        ];
        Ok(PyClassInitializer::from(RecordStep::new("RotateSectionPlanesCommand", run_rotate_sections, args).bumps(Bump::Sections)).add_subclass(Self))
    }
}

#[pymethods]
impl ChangeGuide {
    #[new]
    fn new(py: Python<'_>, guide: Obj<'_>, direction: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let direction = Bound::new(py, Vec3::from(xyz(&direction)?.normalize_or_zero()))?.into_any();
        Ok(PyClassInitializer::from(RecordStep::new("ChangeGuideCommand", run_set_direction, vec![("guide", guide), ("new", direction)]).paint().rows()).add_subclass(Self))
    }
}

#[pymethods]
impl DeleteGuides {
    #[new]
    fn new(guides: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        Ok(PyClassInitializer::from(deleting("DeleteGuidesCommand", "guides", ["guides", "_guides"], &guides)?.paint().rows()).add_subclass(Self))
    }
}

#[pymethods]
impl AddImagePlane {
    #[new]
    fn new(image: Obj<'_>) -> PyClassInitializer<Self> {
        PyClassInitializer::from(RecordStep::new("AddImagePlaneCommand", run_add_image, vec![("image", image)]).picks(Pick::RedoSelect)).add_subclass(Self)
    }
}

#[pymethods]
impl DeleteImagePlanes {
    #[new]
    fn new(images: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        Ok(PyClassInitializer::from(deleting("DeleteImagePlanesCommand", "image_planes", ["images", "_images"], &images)?).add_subclass(Self))
    }
}

#[pymethods]
impl TransformImagePlane {
    #[new]
    #[pyo3(signature = (image, origin = None, u = None, v = None))]
    fn new(py: Python<'_>, image: Obj<'_>, origin: Option<Obj<'_>>, u: Option<Obj<'_>>, v: Option<Obj<'_>>) -> PyResult<PyClassInitializer<Self>> {
        let frame = PyTuple::new(py, [optional_vector(py, origin)?, optional_vector(py, u)?, optional_vector(py, v)?])?.into_any();
        Ok(PyClassInitializer::from(RecordStep::new("TransformImagePlaneCommand", run_image_frame, vec![("image", image), ("_new", frame)])).add_subclass(Self))
    }
}

#[pymethods]
impl SetImagePlaneOpacity {
    #[new]
    fn new(py: Python<'_>, image: Obj<'_>, opacity: f64) -> PyResult<PyClassInitializer<Self>> {
        let opacity = opacity.clamp(0.0, 1.0).into_pyobject(py)?.into_any();
        Ok(PyClassInitializer::from(RecordStep::new("SetImagePlaneOpacityCommand", run_image_opacity, vec![("image", image), ("opacity", opacity)])).add_subclass(Self))
    }
}

#[pymethods]
impl MoveImagePlanes {
    #[new]
    fn new(py: Python<'_>, images: Obj<'_>, delta: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let args = vec![("_images", list(&images)?), ("delta", vector(py, &delta)?)];
        Ok(PyClassInitializer::from(RecordStep::new("MoveImagePlanesCommand", run_translate_origin, args).paint()).add_subclass(Self))
    }
}

#[pymethods]
impl RotateImagePlanes {
    #[new]
    fn new(py: Python<'_>, images: Obj<'_>, centre: Obj<'_>, axis: Obj<'_>, deg: f64) -> PyResult<PyClassInitializer<Self>> {
        let args = vec![
            ("_images", list(&images)?),
            ("centre", vector(py, &centre)?),
            ("axis", vector(py, &axis)?),
            ("deg", deg.into_pyobject(py)?.into_any()),
        ];
        Ok(PyClassInitializer::from(RecordStep::new("RotateImagePlanesCommand", run_rotate_images, args)).add_subclass(Self))
    }
}

#[pymethods]
impl ScaleImagePlanes {
    #[new]
    #[pyo3(signature = (images, anchor, factor, axes = None))]
    fn new(py: Python<'_>, images: Obj<'_>, anchor: Obj<'_>, factor: Obj<'_>, axes: Option<Obj<'_>>) -> PyResult<PyClassInitializer<Self>> {
        // A float scales uniformly; a sequence per axis (the grip box).
        let factor = if factor.is_instance_of::<PyTuple>() || factor.is_instance_of::<PyList>() {
            let fs = factor.try_iter()?.map(|f| f?.extract::<f64>()).collect::<PyResult<Vec<_>>>()?;
            PyTuple::new(py, fs)?.into_any()
        } else {
            factor.extract::<f64>()?.into_pyobject(py)?.into_any()
        };
        let axes = axes.unwrap_or_else(|| py.None().into_bound(py));
        let args = vec![("_images", list(&images)?), ("anchor", vector(py, &anchor)?), ("factor", factor), ("axes", axes)];
        Ok(PyClassInitializer::from(RecordStep::new("ScaleImagePlanesCommand", run_scale_images, args)).add_subclass(Self))
    }
}

#[pymethods]
impl AddGeoPath {
    #[new]
    fn new(path: Obj<'_>) -> PyClassInitializer<Self> {
        PyClassInitializer::from(RecordStep::new("AddGeoPathCommand", add_to("geo_paths"), vec![("path", path)]).picks(Pick::UndoDiscard)).add_subclass(Self)
    }
}

#[pymethods]
impl DeleteGeoPoints {
    #[new]
    fn new(points: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        Ok(PyClassInitializer::from(RecordStep::new("DeleteGeoPointsCommand", delete_from("geo_points"), vec![("_points", list(&points)?)])).add_subclass(Self))
    }
}

#[pymethods]
impl DeleteGeoPaths {
    #[new]
    fn new(paths: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        Ok(PyClassInitializer::from(deleting("DeleteGeoPathsCommand", "geo_paths", ["paths", "_paths"], &paths)?).add_subclass(Self))
    }
}

#[pymethods]
impl ToggleGeoPathClosed {
    #[new]
    fn new(paths: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        Ok(PyClassInitializer::from(RecordStep::new("ToggleGeoPathClosedCommand", run_toggle_paths, vec![("_paths", list(&paths)?)])).add_subclass(Self))
    }
}

#[pymethods]
impl SetGeoPathSurface {
    #[new]
    fn new(paths: Obj<'_>, mode: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let args = vec![("_paths", list(&paths)?), ("mode", mode)];
        Ok(PyClassInitializer::from(RecordStep::new("SetGeoPathSurfaceCommand", run_path_surfaces, args).picks(Pick::ClearSurfaces)).add_subclass(Self))
    }
}

#[pymethods]
impl MoveGeoPathNode {
    #[new]
    fn new(py: Python<'_>, path: Obj<'_>, index: Obj<'_>, new_point: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let args = vec![("path", path), ("index", index), ("_new", vector(py, &new_point)?)];
        Ok(PyClassInitializer::from(RecordStep::new("MoveGeoPathNodeCommand", run_move_node, args)).add_subclass(Self))
    }
}
