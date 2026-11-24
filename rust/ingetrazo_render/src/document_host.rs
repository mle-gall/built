// SPDX-License-Identifier: GPL-3.0-or-later
//! Borrowed document handles and binary64 vectors at the viewport boundary.
use glam::DVec3;
use pyo3::prelude::*;
pub(super) type Obj<'py> = Bound<'py, PyAny>;
pub(super) fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}
pub(super) fn opt<'py>(o: &Obj<'py>, name: &str) -> PyResult<Obj<'py>> {
    Ok(o.getattr_opt(name)?.unwrap_or_else(|| none(o.py())))
}
pub(super) fn host(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    // The controller imports this extension before its first native call. Consult
    // the interpreter's module table directly, including when a host reloads it.
    let modules = unsafe { Bound::from_borrowed_ptr(py, pyo3::ffi::PyImport_GetModuleDict()) };
    Ok(modules
        .get_item("views.document_interaction")?
        .cast_into()?)
}
pub(super) fn xyz(p: &Obj) -> PyResult<DVec3> {
    Ok(DVec3::from_array(p.call_method0("toTuple")?.extract()?))
}
pub(super) fn vec3(py: Python<'_>, p: DVec3) -> PyResult<Obj<'_>> {
    py.import("ingetrazo_core")?
        .call_method1("Vec3", (p.x, p.y, p.z))
}
pub(super) fn id(p: &Obj) -> usize {
    p.as_ptr() as usize
}
pub(super) fn call_opt<'py>(p: &Obj<'py>, name: &str) -> PyResult<Obj<'py>> {
    match p.getattr_opt(name)? {
        Some(f) if f.is_callable() => f.call0(),
        _ => Ok(none(p.py())),
    }
}
