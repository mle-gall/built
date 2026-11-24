// SPDX-License-Identifier: GPL-3.0-or-later
//! Placement revisions and document-context traversal.
use crate::document_host::*;
use pyo3::{
    prelude::*,
    types::{PyDict, PyFrozenSet, PyList, PySet, PyTuple},
};
fn cache<'py>(v: &Obj<'py>, name: &str) -> PyResult<Obj<'py>> {
    let hit = opt(v, name)?;
    if hit.is_none() {
        let d = PyDict::new(v.py());
        v.setattr(name, &d)?;
        Ok(d.into_any())
    } else {
        Ok(hit)
    }
}
fn truth(v: &Obj, k: &str) -> PyResult<bool> {
    opt(v, k)?.is_truthy()
}
fn tuple<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    v.py().import("builtins")?.call_method1("tuple", (v,))
}
fn get<'py>(map: &Obj<'py>, k: &str) -> PyResult<Obj<'py>> {
    map.call_method1("get", (k,))
}
fn class(py: Python) -> PyResult<Obj> {
    host(py)?.getattr("DocumentInteraction")
}
mod bake;
mod epoch;
mod walk;
pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    epoch::register(m)?;
    walk::register(m)?;
    bake::register(m)
}
