// SPDX-License-Identifier: GPL-3.0-or-later
//! Resolve drawing planes and face references against the active document.
use crate::document_host::*;
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
fn locking(tool: &Obj) -> PyResult<bool> {
    tool.is_instance(&tool.py().import("tools.base")?.getattr("PlaneLock")?)
}
fn plane_of<'py>(face: &Obj<'py>, place: &Obj<'py>) -> PyResult<Obj<'py>> {
    face.py()
        .import("core.snap")?
        .call_method1("face_plane_world", (face, opt(place, "xform")?))
}
fn copied<'py>(p: &Obj<'py>) -> PyResult<Obj<'py>> {
    vec3(p.py(), xyz(p)?)
}
fn pixel<'py>(v: &Obj<'py>, method: &str, cursor: &Obj<'py>) -> PyResult<Obj<'py>> {
    v.call_method1(method, (cursor.get_item(0)?, cursor.get_item(1)?))
}
fn ray_on<'py>(
    v: &Obj<'py>,
    o: &Obj<'py>,
    d: &Obj<'py>,
    plane: &Obj<'py>,
    grazing: bool,
) -> PyResult<Obj<'py>> {
    let kw = PyDict::new(v.py());
    kw.set_item("allow_grazing", grazing)?;
    v.call_method(
        "_ray_plane",
        (o, d, plane.get_item(0)?, plane.get_item(1)?),
        Some(&kw),
    )
}
fn plane_lock_free(tool: &Obj) -> PyResult<bool> {
    Ok(opt(tool, "plane_lock")?.is_none() && opt(tool, "plane_ref")?.is_none())
}
mod crossing;
mod faces;
mod work;
pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    work::register(m)?;
    faces::register(m)?;
    crossing::register(m)
}
