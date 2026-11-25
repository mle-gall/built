// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared construction and installation of meshes prepared by a format worker.
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
type Obj<'p> = Bound<'p, PyAny>;
pub(crate) fn verts<'p>(py: Python<'p>, pts: &[DVec3]) -> PyResult<Bound<'p, PyList>> {
    let l = PyList::empty(py);
    for &p in pts {
        l.append(Py::new(py, crate::math::Vec3::from(p))?)?
    }
    Ok(l)
}
pub(crate) fn new_mesh(py: Python<'_>) -> PyResult<Obj<'_>> {
    py.import("built_core.mesh")?.call_method0("Mesh")
}
pub(crate) fn group<'p>(py: Python<'p>, mesh: &Obj<'p>, name: &str) -> PyResult<Obj<'p>> {
    let kw = PyDict::new(py);
    kw.set_item("name", name)?;
    py.import("built_core.group")?
        .getattr("Group")?
        .call((mesh,), Some(&kw))
}
pub(crate) fn assign(face: &Obj<'_>, attrs: &Obj<'_>) -> PyResult<()> {
    if attrs.is_truthy()? {
        face.getattr("attrs")?.call_method1("update", (attrs,))?;
    }
    Ok(())
}
#[pyfunction]
pub(crate) fn dae_add_fused(mesh: &Obj<'_>, fused: &Obj<'_>) -> PyResult<()> {
    let py = mesh.py();
    for item in fused.try_iter()? {
        let item = item?;
        let (outer, holes, attrs, originals): (Obj<'_>, Obj<'_>, Obj<'_>, Obj<'_>) =
            item.extract()?;
        let holes = holes.is_truthy()?.then_some(holes);
        if let Ok(f) = mesh.call_method1("add_face", (outer, holes)) {
            assign(&f, &attrs)?
        } else {
            for p in originals.try_iter()? {
                if let Ok(f) = mesh.call_method1("add_face", (p?,)) {
                    assign(&f, &attrs)?
                }
            }
        }
        crate::file_open::checkpoint(py)?;
    }
    Ok(())
}
pub(crate) fn place_mesh(py: Python<'_>, dst: &Obj<'_>, src: &Obj<'_>) -> PyResult<()> {
    let store = py.import("built_core.store_mesh")?.getattr("StoreMesh")?;
    if dst.get_type().is(&store)
        && src.get_type().is(&store)
        && dst
            .getattr("_store")?
            .getattr("revision")?
            .extract::<u64>()?
            == 0
        && dst
            .getattr("__dict__")?
            .call_method1("get", ("_instance_definition",))?
            .is_none()
        && dst.getattr("vertices")?.len()? == 0
    {
        dst.call_method1("restore_state", (src.call_method0("capture_state")?,))?;
        return Ok(());
    }
    let modal = py.import("built_render.shell_modal")?;
    for (i, f) in src.getattr("faces")?.try_iter()?.enumerate() {
        if i > 0 && i % 512 == 0 {
            modal.call_method0("tick")?;
        }
        let f = f?;
        let holes = f.getattr("holes")?;
        if let Ok(new) = dst.call_method1(
            "add_face",
            (f.getattr("vertices")?, holes.is_truthy()?.then_some(holes)),
        ) {
            assign(&new, &f.getattr("attrs")?)?
        }
    }
    for (i, e) in src.getattr("edges")?.try_iter()?.enumerate() {
        if i > 0 && i % 1024 == 0 {
            modal.call_method0("tick")?;
        }
        let e = e?;
        if let Ok(new) = dst.call_method1("add_edge", (e.getattr("a")?, e.getattr("b")?)) {
            new.setattr("soft", e.getattr("soft")?)?;
        }
    }
    Ok(())
}
