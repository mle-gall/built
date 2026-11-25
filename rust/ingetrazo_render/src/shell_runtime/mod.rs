//! Services shared by embedded windows and the hostless document fixtures.
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

pub(crate) mod modal;
mod project;
mod resident;
pub(crate) mod services;
mod views;
pub(crate) mod windows;

type Obj<'p> = Bound<'p, PyAny>;
fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}
fn attr<'p>(o: &Obj<'p>, name: &str) -> PyResult<Obj<'p>> {
    Ok(o.getattr_opt(name)?.unwrap_or_else(|| none(o.py())))
}
fn flag(o: &Obj<'_>, name: &str) -> PyResult<bool> {
    attr(o, name)?.is_truthy()
}
fn core(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("ingetrazo_render")
}
fn module<'p>(py: Python<'p>, name: &str) -> PyResult<Bound<'p, PyModule>> {
    py.import(format!("ingetrazo_render.{name}"))
}
fn create<'p>(parent: &Bound<'p, PyModule>, name: &str) -> PyResult<Bound<'p, PyModule>> {
    let py = parent.py();
    let path = format!("ingetrazo_render.{name}");
    let sub = PyModule::new(py, &path)?;
    py.import("sys")?.getattr("modules")?.set_item(path, &sub)?;
    parent.add(name, &sub)?;
    Ok(sub)
}
fn bind<'p>(f: Obj<'p>, args: &[Obj<'p>]) -> PyResult<Obj<'p>> {
    let py = f.py();
    let mut bound = vec![f];
    bound.extend_from_slice(args);
    py.import("functools")?
        .getattr("partial")?
        .call1(PyTuple::new(py, bound)?)
}
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    services::register(m)?;
    windows::register(m)?;
    modal::register(m)?;
    project::register(m)?;
    views::register(m)
}
