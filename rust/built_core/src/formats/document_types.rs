// SPDX-License-Identifier: GPL-3.0-or-later
//! Script-visible exception and archive types inherit their standard-library
//! bases so existing handlers, mapping operations and context managers work.
use super::Obj;
use pyo3::prelude::*;
use pyo3::types::PyDict;

pub(super) fn register<'p>(core: &Bound<'p, PyModule>) -> PyResult<()> {
    let py = core.py();
    let builtins = py.import("builtins")?;
    let method = py.import("functools")?.getattr("partialmethod")?;
    let define = |name: &str, base: Obj<'p>, members: &[(&str, &str)]| -> PyResult<Obj<'p>> {
        let ns = PyDict::new(py);
        ns.set_item("__module__", "built_core")?;
        for (name, kernel) in members {
            ns.set_item(*name, method.call1((core.getattr(*kernel)?,))?)?;
        }
        let cls = builtins.call_method1("type", (name, (base,), ns))?;
        core.add(name, &cls)?;
        Ok(cls)
    };
    let exc = builtins.getattr("Exception")?;
    let value = builtins.getattr("ValueError")?;
    define(
        "NonFiniteDocumentError",
        value.clone(),
        &[("__init__", "igz_nonfinite_error")],
    )?;
    define("NothingToImport", value, &[("__init__", "dxf_empty_error")])?;
    define(
        "DwgBridgeError",
        exc.clone(),
        &[("__init__", "dwg_bridge_error")],
    )?;
    define(
        "NeedsConverter",
        exc,
        &[("__init__", "skp_converter_error")],
    )?;
    define(
        "DocumentArchive",
        py.import("zipfile")?.getattr("ZipFile")?,
        &[("read", "document_archive_read")],
    )?;
    define(
        "GeometryMembers",
        builtins.getattr("dict")?,
        &[("__init__", "igz_members_init")],
    )?;
    let stream = define(
        "SaveStream",
        builtins.getattr("dict")?,
        &[
            ("__init__", "igz_stream_start"),
            ("__exit__", "igz_stream_close"),
            ("store", "igz_stream_store"),
            ("encode", "igz_stream_encode"),
            ("json", "igz_stream_encode"),
            ("document", "igz_stream_encode"),
        ],
    )?;
    stream.setattr(
        "__enter__",
        py.import("contextlib")?
            .getattr("AbstractContextManager")?
            .getattr("__enter__")?,
    )?;
    Ok(())
}
