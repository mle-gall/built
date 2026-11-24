// SPDX-License-Identifier: GPL-3.0-or-later
//! Install native viewport callbacks on the Python host class.
use built_python_bindings::{AdapterEntry, Endpoint, Entry, Mode, Obj, Registrar as Bindings};
use pyo3::{
    prelude::*,
    types::{PyDict, PyString, PyTuple},
};
mod actions;
mod defaults;
mod helpers;
mod host;
mod plugin;

struct Registrar<'py> {
    core: Bound<'py, PyModule>,
    render: Bound<'py, PyModule>,
    bindings: Bindings<'py>,
}

impl<'py> Registrar<'py> {
    fn new(py: Python<'py>) -> PyResult<Self> {
        Ok(Self {
            core: py.import("ingetrazo_core")?,
            render: py.import("ingetrazo_render")?,
            bindings: Bindings::new(py)?,
        })
    }
}
#[pyfunction]
fn register_document_host(cls: &Obj) -> PyResult<()> {
    let r = Registrar::new(cls.py())?;
    defaults::controller(&r, cls)?;
    host::install(&r, cls)?;
    plugin::install(&r, cls)?;
    for &(name, action, params, result, doc) in actions::METHODS {
        r.bindings.define(
            cls,
            Mode::Method,
            name,
            Endpoint::Adapter(action),
            params,
            result,
            doc,
        )?;
    }
    Ok(())
}

#[pyfunction]
fn register_document_module(ns: &Bound<PyDict>) -> PyResult<()> {
    let r = Registrar::new(ns.py())?;
    defaults::module(&r, ns)?;
    helpers::install(&r, ns)
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(register_document_module, m)?)?;
    m.add_function(wrap_pyfunction!(register_document_host, m)?)?;
    Ok(())
}
