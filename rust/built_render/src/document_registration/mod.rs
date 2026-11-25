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
pub(crate) mod view;
mod plugin;

struct Registrar<'py> {
    core: Bound<'py, PyModule>,
    render: Bound<'py, PyModule>,
    bindings: Bindings<'py>,
}

impl<'py> Registrar<'py> {
    fn new(py: Python<'py>) -> PyResult<Self> {
        Ok(Self {
            core: py.import("built_core")?,
            render: py.import("built_render")?,
            bindings: Bindings::new(py)?,
        })
    }
}
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

fn register_document_module(ns: &Bound<PyDict>) -> PyResult<()> {
    let r = Registrar::new(ns.py())?;
    defaults::module(&r, ns)?;
    helpers::install(&r, ns)
}

/// Document and view bindings owned by the interaction lane.
pub(super) fn register_interaction(m: &Bound<PyModule>) -> PyResult<()> {
    crate::document_bindings::register(m)?;
    view::register(m)?;
    crate::document_camera::register(m)?;
    crate::document_input::register(m)?;
    crate::document_chunks::register(m)?;
    crate::document_records::register(m)?;
    crate::document_state::register(m)?;
    crate::document_environment::register(m)?;
    crate::document_cache::register(m)?;
    crate::document_grid::register(m)?;
    crate::document_edits::register(m)?;
    crate::document_targets::register(m)?;
    crate::document_hlr::register(m)?;
    crate::document_placements::register(m)?;
    crate::document_annotations::register(m)?;
    crate::document_planes::register(m)?;
    Ok(())
}
