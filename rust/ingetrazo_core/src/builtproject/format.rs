// SPDX-License-Identifier: GPL-3.0-or-later
//! Python-facing manifests retain the registry and link filter validators.
use super::{
    project_convert, project_manifest_load, project_manifest_new, project_manifest_peek,
    project_manifest_save,
};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList},
};
use std::path::PathBuf;
type Obj<'p> = Bound<'p, PyAny>;
#[pyfunction]
fn is_project(path: PathBuf) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
        s.eq_ignore_ascii_case("builtproject") || s.eq_ignore_ascii_case("igzproj")
    })
}
#[pyfunction]
fn project_check<'p>(value: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = value.py();
    let verify = py.import("core.links")?.getattr("validate_select")?;
    for e in value.get_item("links")?.try_iter()? {
        verify.call1((e?.call_method1("get", ("select",))?,))?;
    }
    let errors = py
        .import("core.bim_registry")?
        .call_method1("validate", (value.get_item("bim_registry")?,))?
        .extract::<Vec<String>>()?;
    if !errors.is_empty() {
        return Err(PyValueError::new_err(errors.join("; ")));
    }
    Ok(value.clone())
}
#[pyfunction]
fn project_open_path(py: Python<'_>, path: PathBuf) -> PyResult<Obj<'_>> {
    py.import("pathlib")?
        .call_method1("Path", (project_convert(path)?.0,))
}
#[pyfunction]
fn project_load(py: Python<'_>, path: PathBuf) -> PyResult<Obj<'_>> {
    project_check(
        &py.import("json")?
            .call_method1("loads", (project_manifest_load(project_convert(path)?.0)?,))?,
    )
}
#[pyfunction]
fn project_peek(py: Python<'_>, path: PathBuf) -> PyResult<Obj<'_>> {
    py.import("json")?
        .call_method1("loads", (project_manifest_peek(path)?,))
}
#[pyfunction]
fn project_save(path: PathBuf, value: &Obj<'_>) -> PyResult<()> {
    let py = value.py();
    project_check(value)?;
    let kw = PyDict::new(py);
    kw.set_item("ensure_ascii", false)?;
    kw.set_item("allow_nan", false)?;
    project_manifest_save(
        path,
        py.import("json")?
            .call_method("dumps", (value,), Some(&kw))?
            .str()?
            .to_str()?,
    )
}
#[pyfunction]
fn project_create<'p>(
    py: Python<'p>,
    path: PathBuf,
    registry: &Obj<'p>,
    identity: &Obj<'p>,
) -> PyResult<Obj<'p>> {
    let data = PyDict::new(py);
    data.set_item("links", PyList::empty(py))?;
    data.set_item("bim_registry", registry)?;
    project_check(&data)?;
    let json = py.import("json")?;
    let dump = |o: &Obj<'_>| json.call_method1("dumps", (o,))?.extract::<String>();
    json.call_method1(
        "loads",
        (project_manifest_new(
            path,
            &dump(registry)?,
            &dump(identity)?,
            None,
        )?,),
    )
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    macro_rules! add{($($f:ident),*)=>{$(m.add_function(wrap_pyfunction!($f,m)?)?;)*}}
    add!(
        is_project,
        project_check,
        project_open_path,
        project_load,
        project_peek,
        project_save,
        project_create
    );
    Ok(())
}
