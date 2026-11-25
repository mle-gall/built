// SPDX-License-Identifier: GPL-3.0-or-later
//! Edit-context selection, preview lifetimes and clipboard commands.
use crate::document_host::*;
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet, PyTuple},
};
fn truth(v: &Obj, k: &str) -> PyResult<bool> {
    opt(v, k)?.is_truthy()
}
fn setting(v: &Obj, key: &str, value: &Obj) -> PyResult<()> {
    v.py()
        .import("ingetrazo_core.settings")?
        .call_method0("settings")?
        .call_method1("setValue", (key, value))?;
    Ok(())
}
mod clipboard;
mod selection;
pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    selection::register(m)?;
    clipboard::register(m)
}
