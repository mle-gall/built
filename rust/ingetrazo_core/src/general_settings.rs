// SPDX-License-Identifier: GPL-3.0-or-later
//! General preferences use the canonical capitalized group; older plists
//! used the lower-case spelling.
use pyo3::prelude::*;
type Obj<'p> = Bound<'p, PyAny>;
fn source<'p>(py: Python<'p>, settings: Option<&Obj<'p>>) -> PyResult<Obj<'p>> {
    match settings {
        Some(store) => Ok(store.clone()),
        None => py.import("core.settings")?.call_method0("settings"),
    }
}
#[pyfunction]
#[pyo3(signature=(name,default=None,settings=None))]
fn value<'p>(
    py: Python<'p>,
    name: &str,
    default: Option<&Obj<'p>>,
    settings: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let st = source(py, settings)?;
    for group in ["General", "general"] {
        let value = st.call_method1("value", (format!("{group}/{name}"),))?;
        if !value.is_none() {
            return Ok(value);
        }
    }
    Ok(default.cloned().unwrap_or_else(|| py.None().into_bound(py)))
}
#[pyfunction]
#[pyo3(signature=(name,val,settings=None))]
fn set_value(name: &str, val: &Obj, settings: Option<&Obj>) -> PyResult<()> {
    source(val.py(), settings)?.call_method1("setValue", (format!("General/{name}"), val))?;
    Ok(())
}
pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    let name = "ingetrazo_core.general_settings";
    let sub = PyModule::new(m.py(), name)?;
    sub.add_function(wrap_pyfunction!(value, &sub)?)?;
    sub.add_function(wrap_pyfunction!(set_value, &sub)?)?;
    m.py()
        .import("sys")?
        .getattr("modules")?
        .set_item(name, &sub)?;
    m.add("general_settings", sub)
}
