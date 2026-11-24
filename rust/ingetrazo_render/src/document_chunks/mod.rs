// SPDX-License-Identifier: GPL-3.0-or-later
//! Cached mesh and placement streams shared by document drawing and picking.
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict, PyList, PyTuple},
};
use std::time::Instant;
type Obj<'py> = Bound<'py, PyAny>;
fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}
fn opt<'py>(o: &Obj<'py>, k: &str) -> PyResult<Obj<'py>> {
    Ok(o.getattr_opt(k)?.unwrap_or_else(|| none(o.py())))
}
fn get<'py>(o: &Obj<'py>, k: &str) -> PyResult<Obj<'py>> {
    o.call_method1("get", (k,))
}
fn truth(o: &Obj, k: &str) -> PyResult<bool> {
    opt(o, k)?.is_truthy()
}
fn id(o: &Obj) -> usize {
    o.as_ptr() as usize
}
use crate::document_host::host;
fn tuple<'py>(o: &Obj<'py>) -> PyResult<Obj<'py>> {
    o.py().import("builtins")?.call_method1("tuple", (o,))
}
fn rounded(py: Python<'_>, x: f64, places: u8) -> PyResult<Obj<'_>> {
    py.import("builtins")?.call_method1("round", (x, places))
}
fn xyz(p: &Obj) -> PyResult<DVec3> {
    let a = p.call_method0("toTuple")?.extract::<[f64; 3]>()?;
    Ok(DVec3::from_array(a))
}
fn coords(p: DVec3) -> (f64, f64, f64) {
    (p.x, p.y, p.z)
}
fn vector(py: Python<'_>, v: DVec3) -> PyResult<Obj<'_>> {
    py.import("ingetrazo_core")?.call_method1("Vec3", coords(v))
}
fn memo<'py>(v: &Obj<'py>, name: &str) -> PyResult<Obj<'py>> {
    let current = opt(v, name)?;
    if !current.is_none() {
        return Ok(current);
    }
    let d = PyDict::new(v.py());
    v.setattr(name, &d)?;
    Ok(d.into_any())
}
fn uid(py: Python<'_>) -> PyResult<Obj<'_>> {
    host(py)?.getattr("_chunk_uid")?.call_method0("__next__")
}
fn perf<'py>(h: &Bound<'py, PyModule>) -> PyResult<Option<Instant>> {
    Ok(h.getattr("_PERF")?.is_truthy()?.then(Instant::now))
}
fn log(
    h: &Bound<PyModule>,
    start: Option<Instant>,
    name: &str,
    extra: String,
    floor: f64,
) -> PyResult<()> {
    if let Some(t) = start {
        h.call_method1(
            "_plog",
            (name, t.elapsed().as_secs_f64() * 1000., extra, floor),
        )?;
    }
    Ok(())
}
mod bake;
mod billboards;
mod disk;
mod faces;
mod fingerprint;
mod materials;
mod placement;
pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    fingerprint::register(m)?;
    materials::register(m)?;
    faces::register(m)?;
    billboards::register(m)?;
    bake::register(m)?;
    placement::register(m)?;
    disk::register(m)
}
