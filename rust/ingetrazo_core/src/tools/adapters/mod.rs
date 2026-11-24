// SPDX-License-Identifier: GPL-3.0-or-later
//! Host-side tool actions. The class and model handles belong to the plugin API;
//! events and commands still enter the same ToolMachine and history.
use built_python_bindings::{Adapter, Endpoint, Mode, Obj, Registrar};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet, PyTuple},
    IntoPyObjectExt,
};

mod pushpull;
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

fn optional<'py>(obj: &Obj<'py>, name: &str) -> Reply<'py> {
    Ok(obj.getattr_opt(name)?.unwrap_or_else(|| none(obj.py())))
}

fn screen<'py>(ctx: &Obj<'py>) -> PyResult<(Obj<'py>, Obj<'py>)> {
    let point = ctx.getattr("screen")?;
    Ok((point.call_method0("x")?, point.call_method0("y")?))
}

#[pyfunction]
pub fn register_tool_adapter<'py>(cls: &Obj<'py>) -> Reply<'py> {
    let py = cls.py();
    let factory = Registrar::new(py)?;
    let module: String = cls.getattr("__module__")?.extract()?;
    let name: String = cls.getattr("__name__")?.extract()?;
    let methods = match (module.as_str(), name.as_str()) {
        ("tools.pushpull", "PushPullTool") => pushpull::METHODS,
        _ => {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "Unknown tool adapter {module}.{name}"
            )))
        }
    };
    for &(name, mode, action, params, result, doc) in methods {
        factory.define(
            cls,
            mode,
            name,
            Endpoint::Adapter(action),
            params,
            result,
            doc,
        )?;
    }
    py.import("abc")?
        .call_method1("update_abstractmethods", (cls,))?;
    Ok(cls.clone())
}
