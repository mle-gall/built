// SPDX-License-Identifier: GPL-3.0-or-later
//! Native face paint steps (docs/commands-port.md): colour, opacity,
//! texture, back paint and the material tag of a set of faces.
//!
//! History enlists the faces' meshes. When it journals their stores, the
//! transaction replays the paint. Otherwise (the object-mesh oracle, or a
//! direct call) the step keeps the fields it overwrote and puts them back:
//! its real inverse.
use super::paint::{apply_face_paint, restore_face_paint, PaintRows};
use super::steps::{version, After, Meshes, Obj, RecordStep};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use pyo3::PyClassInitializer;
use std::collections::HashSet;

fn faces(s: &RecordStep, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
    Ok(s.items(py, 0)?.into_iter().map(|f| f.unbind()).collect())
}

/// Every mesh owning a face, once; a face without one gives None, which
/// sends History to the step's own inverse.
fn face_meshes<'py>(s: &RecordStep, py: Python<'py>, _scene: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for face in s.items(py, 0)? {
        let mesh = face.getattr_opt("_m")?.unwrap_or_else(|| py.None().into_bound(py));
        if seen.insert(mesh.as_ptr() as usize) {
            out.push(mesh);
        }
    }
    Ok(out)
}



/// Write `fields` on the faces; keep what they held on the first direct run.
fn paint(s: &RecordStep, py: Python<'_>, fields: Vec<(String, Py<PyAny>)>) -> PyResult<()> {
    let capture = s.remembered(py, 0).is_none() && !s.journalled();
    if let Some(rows) = apply_face_paint(py, faces(s, py)?, fields, capture)? {
        if capture {
            let rows = PyList::new(py, rows.into_iter().map(|row| PyList::new(py, row).map(|l| l.unbind())).collect::<PyResult<Vec<_>>>()?)?;
            s.remember(vec![rows.into_any()]);
        }
    }
    Ok(())
}

fn restore(s: &RecordStep, py: Python<'_>, fields: Vec<String>) -> PyResult<()> {
    let rows: PaintRows = match s.remembered(py, 0) {
        Some(rows) => rows.extract()?,
        None => Vec::new(),
    };
    restore_face_paint(py, faces(s, py)?, fields, rows)
}

fn field(s: &RecordStep, py: Python<'_>) -> PyResult<String> {
    s.arg(py, 1).extract()
}

fn run_field(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    paint(s, py, vec![(field(s, py)?, s.arg(py, 2).unbind())])?;
    crate::doc::history_support::dirty_group_chunks(scene)?;
    version(scene)
}

fn after_field(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    if phase == Some("undo") && s.is_direct() {
        restore(s, py, vec![field(s, py)?])?;
        crate::doc::history_support::dirty_group_chunks(scene)?;
    }
    version(scene)
}

fn run_material_tag(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let (name, material) = (s.arg(py, 1), s.arg(py, 2));
    // A library swatch registers itself on first use.
    let registry = scene.getattr("materials")?;
    if !material.is_none() && name.is_truthy()? && !registry.contains(&name)? {
        registry.set_item(&name, &material)?;
        s.publish("_registered", pyo3::types::PyBool::new(py, true).to_owned().into_any());
    }
    paint(s, py, vec![("pbr".into(), material.unbind()), ("mat".into(), name.unbind())])?;
    version(scene)
}

fn after_material_tag(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    if phase == Some("undo") && s.is_direct() {
        restore(s, py, vec!["pbr".into(), "mat".into()])?;
    }
    version(scene)
}

fn step(name: &'static str, faces: Obj<'_>, field: &'static str, value: Obj<'_>) -> PyResult<RecordStep> {
    let py = faces.py();
    let faces = PyList::new(py, faces.try_iter()?.collect::<PyResult<Vec<_>>>()?)?.into_any();
    let field = pyo3::types::PyString::new(py, field).into_any();
    Ok(RecordStep::new(name, run_field, vec![("_faces", faces), ("field", field), ("_value", value)])
        .own_inverse()
        .on_meshes(Meshes(face_meshes))
        .hook(After(after_field)))
}

macro_rules! steps {
    ($($ty:ident = $name:literal;)*) => {
        $(
            #[pyclass(module = "ingetrazo_core", name = $name, extends = RecordStep, frozen)]
            pub struct $ty;
        )*
        pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
            $(m.add_class::<$ty>()?;)*
            Ok(())
        }
    };
}

steps! {
    SetFaceColor = "SetFaceColorCommand";
    SetFaceOpacity = "SetFaceOpacityCommand";
    SetFaceTexture = "SetFaceTextureCommand";
    SetFaceBack = "SetFaceBackCommand";
    SetFaceMaterialTag = "SetFaceMaterialTagCommand";
}

fn optional<'py>(py: Python<'py>, value: &Obj<'py>, f: impl FnOnce(&Obj<'py>) -> PyResult<Obj<'py>>) -> PyResult<Obj<'py>> {
    if value.is_none() { Ok(py.None().into_bound(py)) } else { f(value) }
}

#[pymethods]
impl SetFaceColor {
    /// Set or clear the front colour of each face.
    #[new]
    fn new(py: Python<'_>, faces: Obj<'_>, color: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let color = optional(py, &color, |c| Ok(PyList::new(py, c.try_iter()?.collect::<PyResult<Vec<_>>>()?)?.into_any()))?;
        Ok(PyClassInitializer::from(step("SetFaceColorCommand", faces, "color", color)?).add_subclass(Self))
    }
}

#[pymethods]
impl SetFaceOpacity {
    /// Set or clear the opacity of coloured or textured faces.
    #[new]
    fn new(py: Python<'_>, faces: Obj<'_>, opacity: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let opacity = optional(py, &opacity, |o| Ok(o.extract::<f64>()?.into_pyobject(py)?.into_any()))?;
        Ok(PyClassInitializer::from(step("SetFaceOpacityCommand", faces, "opacity", opacity)?).add_subclass(Self))
    }
}

#[pymethods]
impl SetFaceTexture {
    /// Set or clear a texture with an independent mapping per face.
    #[new]
    fn new(py: Python<'_>, faces: Obj<'_>, texture: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let texture = optional(py, &texture, |t| Ok(py.get_type::<PyDict>().call1((t,))?))?;
        Ok(PyClassInitializer::from(step("SetFaceTextureCommand", faces, "texture", texture)?).add_subclass(Self))
    }
}

#[pymethods]
impl SetFaceBack {
    /// Per-face back paint (one dict for all, or one per face; None clears).
    #[new]
    fn new(py: Python<'_>, faces: Obj<'_>, backs: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let faces = PyList::new(py, faces.try_iter()?.collect::<PyResult<Vec<_>>>()?)?.into_any();
        let count = faces.len()?;
        let each: Vec<Obj<'_>> = if backs.is_none() || backs.is_instance_of::<PyDict>() {
            vec![backs; count]
        } else {
            backs.try_iter()?.collect::<PyResult<Vec<_>>>()?
        };
        let backs = PyList::new(
            py,
            each.iter()
                .map(|b| if b.is_instance_of::<PyDict>() { py.get_type::<PyDict>().call1((b,)) } else { Ok(py.None().into_bound(py)) })
                .collect::<PyResult<Vec<_>>>()?,
        )?
        .into_any();
        Ok(PyClassInitializer::from(step("SetFaceBackCommand", faces, "back", backs)?).add_subclass(Self))
    }
}

#[pymethods]
impl SetFaceMaterialTag {
    /// The registry identity `attrs["mat"]` on the faces, the companion of
    /// the colour or texture stamp; a given `material` joins the registry
    /// when it is missing.
    #[new]
    #[pyo3(signature = (faces, name, material = None))]
    fn new(py: Python<'_>, faces: Obj<'_>, name: Obj<'_>, material: Option<Obj<'_>>) -> PyResult<PyClassInitializer<Self>> {
        let faces = PyList::new(py, faces.try_iter()?.collect::<PyResult<Vec<_>>>()?)?.into_any();
        let material = material.unwrap_or_else(|| py.None().into_bound(py));
        let step = RecordStep::new("SetFaceMaterialTagCommand", run_material_tag, vec![("_faces", faces), ("_name", name), ("_material", material)])
            .own_inverse()
            .on_meshes(Meshes(face_meshes))
            .publishes(&["_registered"])
            .hook(After(after_material_tag));
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}
