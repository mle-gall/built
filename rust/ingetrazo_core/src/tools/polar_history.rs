// SPDX-License-Identifier: GPL-3.0-or-later
//! Polar array command planning and the lazy overlap checkpoint. History still
//! publishes ordinary document commands, including placements and IFC records.
mod planes;
use std::collections::HashSet;
use crate::math::{mat, xyz, Vec3};
use pyo3::{
    prelude::*,
    types::{PyDict, PyFloat, PyList, PySet},
};

struct Checkpoint {
    mesh: Py<PyAny>,
    selection: Py<PySet>,
}
impl Checkpoint {
    fn capture(scene: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            mesh: scene
                .getattr("mesh")?
                .call_method0("capture_state")?
                .unbind(),
            selection: scene
                .py()
                .get_type::<PySet>()
                .call1((scene.getattr("selection")?,))?
                .cast_into()?
                .unbind(),
        })
    }

    fn restore(&self, scene: &Bound<'_, PyAny>, context: &Py<PyAny>) -> PyResult<()> {
        scene
            .getattr("mesh")?
            .call_method1("restore_state", (&self.mesh,))?;
        if scene.getattr("edit_group")?.is(context.bind(scene.py())) {
            scene.setattr(
                "selection",
                self.selection.bind(scene.py()).call_method0("copy")?,
            )?;
        }
        let version: u64 = scene.getattr("version")?.extract()?;
        scene.setattr("version", version + 1)
    }
}

#[pyclass(module = "ingetrazo_core", dict)]
struct PolarMerge {
    batches: Vec<Vec<Py<PyAny>>>,
    before: Option<(Checkpoint, Py<PyAny>)>,
    after: Option<Checkpoint>,
}

#[pymethods]
impl PolarMerge {
    #[classattr]
    #[pyo3(name = "keeps_groups")]
    const KEEPS_GROUPS: bool = true;

    fn checkpoint(&mut self, scene: &Bound<'_, PyAny>) -> PyResult<()> {
        if self.before.is_none() {
            self.before = Some((
                Checkpoint::capture(scene)?,
                scene.getattr("edit_group")?.unbind(),
            ));
        }
        Ok(())
    }

    #[pyo3(name = "do")]
    fn apply(slf: &Bound<'_, Self>, scene: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = scene.py();
        let batches = {
            let state = slf.borrow();
            if let (Some(after), Some((_, context))) = (&state.after, &state.before) {
                return after.restore(scene, context);
            }
            state
                .batches
                .iter()
                .map(|batch| {
                    batch
                        .iter()
                        .map(|cmd| cmd.bind(py).getattr("face"))
                        .collect::<PyResult<Vec<_>>>()
                })
                .collect::<PyResult<Vec<_>>>()?
        };
        let mut pending = HashSet::new();
        for face in batches.iter().flatten().filter(|f| !f.is_none()) {
            pending.insert(face.getattr("key")?.extract::<u128>()?);
        }
        let mesh = scene.getattr("mesh")?;
        for batch in batches {
            for f in batch.iter().filter(|f| !f.is_none()) {
                pending.remove(&f.getattr("key")?.extract::<u128>()?);
            }
            if let Err(error) = planes::merge(scene, &mesh, &batch, &pending, || slf.borrow_mut().checkpoint(scene)) {
                slf.borrow().undo(scene)?;
                return Err(error);
            }
        }
        if slf.borrow().before.is_some() {
            slf.borrow_mut().after = Some(Checkpoint::capture(scene)?);
        }
        Ok(())
    }

    fn undo(&self, scene: &Bound<'_, PyAny>) -> PyResult<()> {
        if let Some((before, context)) = &self.before {
            before.restore(scene, context)?;
        }
        Ok(())
    }
}

#[pyfunction]
pub fn rotation_command<'py>(
    py: Python<'py>,
    start: &Bound<'py, PyAny>,
    axis: &Bound<'py, PyAny>,
    copy: bool,
    groups: &Bound<'py, PyAny>,
    positions: &Bound<'py, PyAny>,
    faces: &Bound<'py, PyAny>,
    edges: &Bound<'py, PyAny>,
    guides: &Bound<'py, PyAny>,
    splanes: &Bound<'py, PyAny>,
    images: &Bound<'py, PyAny>,
    degs: &Bound<'py, PyAny>,
) -> PyResult<Option<Bound<'py, PyAny>>> {
    let elements = |items: &Bound<'py, PyAny>| items.try_iter()?.collect::<PyResult<Vec<_>>>();
    let (groups, guides) = (elements(groups)?, elements(guides)?);
    let number = py.get_type::<PyFloat>();
    let degs = degs
        .try_iter()?
        .map(|d| number.call1((d?,))?.extract::<f64>())
        .collect::<PyResult<Vec<_>>>()?;
    let history = py.import("core.history")?;
    let mut commands = Vec::new();
    if copy {
        let (faces, edges) = (elements(faces)?, elements(edges)?);
        let placements = py.import("core.group")?;
        let new_attrs = py
            .import("core.ifc_identity")?
            .getattr("copied_face_attrs")?;
        let copy_guide = py.import("core.move_ops")?.getattr("copied_guide")?;
        let next_curve = py
            .import("core.mesh")?
            .getattr("Mesh")?
            .getattr("next_curve_id")?;
        let curves = PyDict::new(py);
        let mut batches = Vec::new();
        for degrees in degs {
            let matrix = history.call_method1("rotation_matrix", (start, axis, degrees))?;
            let m = mat(&matrix)?;
            let point =
                |p: &Bound<'_, PyAny>| -> PyResult<Vec3> { Ok(m.transform_point3(xyz(p)?).into()) };
            let ring = |r: &Bound<'_, PyAny>| -> PyResult<Vec<Vec3>> {
                r.try_iter()?.map(|v| point(&v?)).collect()
            };
            let attrs = new_attrs.call0()?;
            for group in &groups {
                let clone = placements.call_method1("copy_group", (group,))?;
                commands.push(history.call_method1("InsertGroupCommand", (&clone,))?);
                commands.push(
                    history.call_method1("RotateGroupCommand", (clone, start, axis, degrees))?,
                );
            }
            for guide in &guides {
                let clone = copy_guide.call1((guide, &matrix))?;
                commands.push(history.call_method1("AddGuideCommand", (clone,))?);
            }
            let mut batch = Vec::new();
            for face in &faces {
                let holes = face
                    .getattr("holes")?
                    .try_iter()?
                    .map(|h| ring(&h?))
                    .collect::<PyResult<Vec<_>>>()?;
                let cmd = super::copy_history::copied_face(
                    &history,
                    &placements,
                    &attrs,
                    &matrix,
                    face,
                    PyList::new(py, ring(&face.getattr("vertices")?)?)?.as_any(),
                    PyList::new(py, holes)?.as_any(),
                    false,
                )?;
                batch.push(cmd.clone().unbind());
                commands.push(cmd);
            }
            if !batch.is_empty() {
                batches.push(batch);
            }
            for edge in &edges {
                let kw = PyDict::new(py);
                if edge
                    .getattr_opt("soft")?
                    .map(|flag| flag.is_truthy())
                    .transpose()?
                    .unwrap_or(false)
                {
                    kw.set_item("soft", true)?;
                }
                if let Some(id) = edge.getattr_opt("curve")?.filter(|c| !c.is_none()) {
                    let key = (id, degrees);
                    let fresh = match curves.get_item(&key)? {
                        Some(c) => c,
                        None => {
                            let id = next_curve.call0()?;
                            curves.set_item(&key, &id)?;
                            id
                        }
                    };
                    kw.set_item("curve", fresh)?;
                }
                commands.push(history.getattr("AddEdgeCommand")?.call(
                    (point(&edge.getattr("a")?)?, point(&edge.getattr("b")?)?),
                    Some(&kw),
                )?);
            }
        }
        if !batches.is_empty() {
            commands.push(
                Py::new(
                    py,
                    PolarMerge {
                        batches,
                        before: None,
                        after: None,
                    },
                )?
                .into_bound(py)
                .into_any(),
            );
        }
    } else {
        let degrees = *degs
            .last()
            .ok_or_else(|| pyo3::exceptions::PyIndexError::new_err("rotation needs an angle"))?;
        for group in groups {
            commands
                .push(history.call_method1("RotateGroupCommand", (group, start, axis, degrees))?);
        }
        if positions.is_truthy()? {
            let kwargs = PyDict::new(py);
            kwargs.set_item("carry_textures", true)?;
            commands.push(
                history
                    .getattr("RotateVerticesCommand")?
                    .call((positions, start, axis, degrees), Some(&kwargs))?,
            );
        }
        for (class, records) in [
            ("RotateSectionPlanesCommand", splanes),
            ("RotateImagePlanesCommand", images),
        ] {
            if records.is_truthy()? {
                commands.push(history.call_method1(class, (records, start, axis, degrees))?);
            }
        }
        if !guides.is_empty() {
            commands
                .push(history.call_method1("RotateGuidesCommand", (guides, start, axis, degrees))?);
        }
    }
    match commands.len() {
        0 => Ok(None),
        1 => Ok(commands.pop()),
        _ => Ok(Some(history.call_method1("CompoundCommand", (commands,))?)),
    }
}
