// SPDX-License-Identifier: GPL-3.0-or-later
//! Manager-owned extension packages, review metadata and discovery settings.
use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict, PyList, PyString, PyTuple},
};
use std::{
    collections::{BTreeSet, HashSet},
    fs,
    path::{Path, PathBuf},
};
type Obj<'p> = Bound<'p, PyAny>;
mod catalog;
pub(crate) mod discovery;
mod examples;
mod files;
mod install;
pyo3::create_exception!(
    extension_manager,
    ExtensionManagerError,
    pyo3::exceptions::PyException
);
fn err(msg: impl Into<String>) -> PyErr {
    ExtensionManagerError::new_err(msg.into())
}
fn module(py: Python) -> PyResult<Obj> {
    py.import("ingetrazo_core")?.getattr("extension_manager")
}
fn root(py: Python) -> PyResult<PathBuf> {
    py.import("core.extensions")?
        .call_method0("user_plugins_dir")?
        .extract()
}
fn path<'p>(py: Python<'p>, p: &Path) -> PyResult<Obj<'p>> {
    py.import("pathlib")?.call_method1("Path", (p,))
}
fn paths<'p>(py: Python<'p>, ps: Vec<PathBuf>) -> PyResult<Vec<Obj<'p>>> {
    ps.iter().map(|p| path(py, p)).collect()
}
fn valid(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
}
fn require_id(id: &str) -> PyResult<()> {
    if valid(id) {
        Ok(())
    } else {
        Err(err("Invalid extension ID."))
    }
}
fn name(p: &Path) -> &str {
    p.file_name().and_then(|s| s.to_str()).unwrap_or("")
}
fn version(py: Python) -> PyResult<String> {
    py.import("ingetrazo_core.version")?.getattr("__version__")?.extract()
}
fn digest(py: Python, bytes: &[u8]) -> PyResult<String> {
    py.import("hashlib")?
        .call_method1("sha256", (PyBytes::new(py, bytes),))?
        .call_method0("hexdigest")?
        .extract()
}
fn entries(folder: &Path) -> Vec<PathBuf> {
    let mut entries = fs::read_dir(folder)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect::<Vec<_>>();
    entries.sort();
    entries
}
fn key<'p>(d: &Obj<'p>, name: &str) -> PyResult<Obj<'p>> {
    d.call_method1("get", (name,))
}
fn s(obj: &Obj, key: &str) -> PyResult<String> {
    obj.getattr(key)?.extract()
}
fn visible(p: &Path) -> bool {
    !name(p).starts_with(['.', '_'])
}
fn clear_file(p: &Path) -> PyResult<()> {
    match fs::remove_file(p) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        v => Ok(v?),
    }
}

/// Multiple windows and processes share the same plugin root. Keep the lock
/// outside it, so plugin discovery and installation ownership stay independent.
fn edit_lock(py: Python) -> PyResult<fs::File> {
    let plugins = root(py)?;
    let parent = plugins.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let lock = parent.join(format!(".{}.manager.lock", name(&plugins)));
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock)?;
    file.try_lock().map_err(|e| {
        err(format!(
            "Cannot change this plugin folder while another extension operation is running: {e}"
        ))
    })?;
    Ok(file)
}

fn data_class(
    m: &Bound<PyModule>,
    name: &str,
    required: &[&str],
    optional: &[&str],
) -> PyResult<()> {
    let py = m.py();
    let dc = py.import("dataclasses")?;
    let fields = PyList::new(py, required)?;
    let obj = py.get_type::<PyAny>();
    for k in optional {
        let kw = PyDict::new(py);
        kw.set_item("default", py.None())?;
        fields.append((k, &obj, dc.call_method("field", (), Some(&kw))?))?
    }
    let kw = PyDict::new(py);
    kw.set_item("frozen", true)?;
    let class = dc.call_method("make_dataclass", (name, fields), Some(&kw))?;
    class.setattr("__module__", "extension_manager")?;
    m.add(name, class)
}
/// Catalogue records are constructed on first access. The modelling engine
/// can start and import its bindings before any plugin catalogue is opened.
#[pyfunction]
fn __getattr__<'py>(py: Python<'py>, name: &str) -> PyResult<Obj<'py>> {
    let m = module(py)?.cast_into::<PyModule>()?;
    match name {
        "Extension" => data_class(
            &m,
            name,
            &[
                "id",
                "version",
                "author",
                "license",
                "repository",
                "download",
                "sha256",
                "minimum_version",
                "name",
                "summary",
                "tags",
            ],
            &[
                "screenshot",
                "updated_at",
                "thumb",
                "screenshot_sha",
                "bundled",
            ],
        )?,
        "CatalogSnapshot" => data_class(
            &m,
            name,
            &["extensions", "reviewed", "fetched_at", "from_cache"],
            &[],
        )?,
        _ => return Err(pyo3::exceptions::PyAttributeError::new_err(name.to_owned())),
    }
    m.getattr(name)
}
pub(crate) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    let py = m.py();
    let sub = PyModule::new(py, "ingetrazo_core.extension_manager")?;
    sub.add_function(wrap_pyfunction!(__getattr__, &sub)?)?;
    sub.add(
        "ExtensionManagerError",
        py.get_type::<ExtensionManagerError>(),
    )?;
    catalog::register(&sub)?;
    files::register(&sub)?;
    examples::register(&sub)?;
    install::register(&sub)?;
    py.import("sys")?
        .getattr("modules")?
        .set_item("ingetrazo_core.extension_manager", &sub)?;
    m.add("extension_manager", sub)
}
