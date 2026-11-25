// SPDX-License-Identifier: GPL-3.0-or-later
//! Document host services installed on the Python-facing classes and modules.
use super::*;
use built_python_bindings::Adapter;
use pyo3::{
    types::{PyList, PySet},
    IntoPyObjectExt,
};
type Args<'py> = Bound<'py, PyTuple>;
type Reply<'py> = PyResult<Obj<'py>>;
type Method = (
    &'static str,
    Mode,
    Adapter,
    &'static [&'static str],
    &'static str,
    &'static str,
);
fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}
fn optional<'py>(o: &Obj<'py>, name: &str) -> Reply<'py> {
    Ok(o.getattr_opt(name)?.unwrap_or_else(|| none(o.py())))
}
fn attr_flag(obj: &Obj, field: &str, default: bool) -> PyResult<bool> {
    obj.getattr_opt(field)?
        .map_or(Ok(default), |value| value.is_truthy())
}
fn positional<'py>(o: &Obj<'py>) -> PyResult<Args<'py>> {
    Ok(o.py().get_type::<PyTuple>().call1((o,))?.cast_into()?)
}
fn unpack<'py, const N: usize>(value: &Obj<'py>) -> PyResult<[Obj<'py>; N]> {
    if value.cast_exact::<PyTuple>().is_ok() {
        return value.extract();
    }
    let mut items = value.try_iter()?;
    let row: Vec<_> = items.by_ref().take(N).collect::<PyResult<_>>()?;
    if row.len() != N || items.next().transpose()?.is_some() {
        return Err(pyo3::exceptions::PyValueError::new_err(format!(
            "expected {N} fields"
        )));
    }
    row.try_into()
        .map_err(|_| pyo3::exceptions::PyValueError::new_err("invalid row"))
}
macro_rules! fields {
    ($py:expr; $($key:expr => $value:expr),* $(,)?) => {{
        let fields=PyDict::new($py); $(fields.set_item($key,$value)?;)* fields
    }};
}
mod device;
mod document_controller;
mod document_interaction;
mod document_tools;
mod modules;
mod ndof_input;
mod view_commands;
mod view_host;
mod view_navigation;
fn install(owner: &Obj, namespace: &Bound<PyDict>, entries: &[Method]) -> PyResult<()> {
    let py = owner.py();
    let registrar = Bindings::new(py)?
        .with_plugin_method(py.import("tools")?.getattr("_plugin_method")?)
        .with_module_annotations(namespace)?;
    for &(name, mode, action, args, result, doc) in entries {
        registrar.define(
            owner,
            mode,
            name,
            Endpoint::Adapter(action),
            args,
            result,
            doc,
        )?;
    }
    Ok(())
}
#[pyfunction]
fn register_view_class(cls: &Obj) -> PyResult<()> {
    let module = cls.getattr("__module__")?.extract::<String>()?;
    let name = cls.getattr("__name__")?.extract::<String>()?;
    let methods = match (module.as_str(), name.as_str()) {
        ("views.document_tools", "DocumentTools") => document_tools::DOCUMENT_TOOLS,
        ("views.document_controller", "DocumentController") => {
            document_controller::DOCUMENT_CONTROLLER
        }
        ("views.document_controller", "QtViewHost") => document_controller::QT_VIEW_HOST,
        ("views.view_navigation", "ViewNavigation") => view_navigation::VIEW_NAVIGATION,
        ("views.view_host", "WidgetShim") => view_host::WIDGET_SHIM,
        ("views.document_interaction", "DocumentInteraction") => {
            document_interaction::DOCUMENT_INTERACTION
        }
        ("views.ndof_input", "NdofInput") => ndof_input::NDOF_INPUT,
        _ => return Err(pyo3::exceptions::PyValueError::new_err((module, name))),
    };
    install(cls, &cls.py().import(module)?.dict(), methods)
}
#[pyfunction]
fn register_view_functions(namespace: &Bound<PyDict>) -> PyResult<()> {
    let name = namespace
        .get_item("__name__")?
        .unwrap()
        .extract::<String>()?;
    let methods = match name.as_str() {
        "views.document_tools" => document_tools::FUNCTIONS,
        "views.document_interaction" => document_interaction::FUNCTIONS,
        "views.view_commands" => view_commands::FUNCTIONS,
        "views.ndof_input" => ndof_input::FUNCTIONS,
        _ => return Err(pyo3::exceptions::PyValueError::new_err(name)),
    };
    install(namespace.as_any(), namespace, methods)
}
pub(super) fn register(module: &Bound<PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(
        modules::install_interaction_module,
        module
    )?)?;

    module.add_function(wrap_pyfunction!(register_view_class, module)?)?;
    module.add_function(wrap_pyfunction!(register_view_functions, module)?)?;
    Ok(())
}
