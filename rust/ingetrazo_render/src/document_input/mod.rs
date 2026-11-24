// SPDX-License-Identifier: GPL-3.0-or-later
//! Document-side input dispatch. The shell owns focus and pane routing;
//! tools and extensions receive events only after the document accepts them.
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};
type Obj<'py> = Bound<'py, PyAny>;
fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}
fn opt<'py>(o: &Obj<'py>, name: &str) -> PyResult<Obj<'py>> {
    Ok(o.getattr_opt(name)?.unwrap_or_else(|| none(o.py())))
}
fn host(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("views.document_interaction")
}
fn doc<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    host(v.py())?.getattr("DocumentInteraction")
}
fn reset(v: &Obj, attrs: &[&str]) -> PyResult<()> {
    for name in attrs {
        v.setattr(*name, v.py().None())?
    }
    Ok(())
}
fn tool<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    v.getattr("active_tool")
}
fn busy(v: &Obj, t: &Obj) -> PyResult<bool> {
    Ok(!t.is_none() && v.call_method1("_tool_busy", (t,))?.is_truthy()?)
}
fn emit(v: &Obj, signal: &str, value: &Obj) -> PyResult<()> {
    v.getattr(signal)?.call_method1("emit", (value,))?;
    Ok(())
}
fn refresh(v: &Obj) -> PyResult<()> {
    v.call_method0("_refresh_snap")?;
    Ok(())
}
fn update(v: &Obj) -> PyResult<()> {
    v.call_method0("update")?;
    Ok(())
}
fn read_only<T>(py: Python<'_>, read: impl FnOnce() -> PyResult<T>) -> PyResult<T> {
    let guard = py
        .import("views.array_bake")?
        .call_method0("reading_only")?;
    guard.call_method0("__enter__")?;
    let result = read();
    guard.call_method1("__exit__", (py.None(), py.None(), py.None()))?;
    result
}
#[pyclass(frozen, module = "ingetrazo_render")]
struct InferenceField {
    attribute: String,
}
#[pymethods]
impl InferenceField {
    #[new]
    fn new(attribute: String) -> Self {
        Self { attribute }
    }
    fn __get__<'py>(
        slf: &Bound<'py, Self>,
        instance: Option<&Obj<'py>>,
        _owner: Option<&Obj<'py>>,
    ) -> PyResult<Obj<'py>> {
        match instance {
            Some(v) => v.getattr("inference")?.getattr(&slf.borrow().attribute),
            None => Ok(slf.clone().into_any()),
        }
    }
    fn __set__(&self, v: &Obj, value: &Obj) -> PyResult<()> {
        v.getattr("inference")?.setattr(&self.attribute, value)
    }
}
mod extension;
mod inference;
mod keys;
mod navigation;
mod pointer;
mod value;
pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_class::<InferenceField>()?;
    value::register(m)?;
    extension::register(m)?;
    keys::register(m)?;
    inference::register(m)?;
    pointer::register(m)?;
    navigation::register(m)
}
