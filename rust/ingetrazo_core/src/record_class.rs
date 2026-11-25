// SPDX-License-Identifier: GPL-3.0-or-later
//! Install native methods on journalled Python document handles. Defer the
//! Record import until the document model and the extension are initialized.
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};
pub fn build<'p>(
    module: &Bound<'p, PyModule>,
    name: &str,
    fields: &[&str],
    slots: bool,
    methods: Vec<Bound<'p, pyo3::types::PyCFunction>>,
    class_methods: &[&str],
) -> PyResult<Bound<'p, PyAny>> {
    let py = module.py();
    let model = py.import("core.doc_model")?;
    let attrs = PyDict::new(py);
    let names = PyTuple::new(py, fields)?;
    attrs.set_item("__module__", module.name()?)?;
    if slots {
        attrs.set_item("__slots__", &names)?;
    }
    let partial = py.import("functools")?.getattr("partialmethod")?;
    let builtin = py.import("builtins")?;
    for method in methods {
        let key = method.getattr("__name__")?.extract::<String>()?;
        let descriptor = if class_methods.contains(&key.as_str()) {
            builtin.call_method1("classmethod", (method,))?
        } else {
            partial.call1((method,))?
        };
        attrs.set_item(key, descriptor)?;
    }
    let cls = builtin.call_method1("type", (name, (model.getattr("Record")?,), attrs))?;
    let cls = model
        .getattr("record_fields")?
        .call1(names)?
        .call1((cls,))?;
    module.add(name, &cls)?;
    Ok(cls)
}
