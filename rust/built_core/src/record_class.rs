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
    let model = py.import("built_core.doc_model")?;
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

/// Use an existing native entry point as a method of a journalled handle.
pub fn bind_methods(cls: &Bound<PyAny>, methods: &[(&str, &str, bool)]) -> PyResult<()> {
    let py = cls.py();
    let core = py.import("built_core")?;
    let classmethod = py.import("builtins")?.getattr("classmethod")?;
    let partial = py.import("functools")?.getattr("partialmethod")?;
    for &(name, function, is_class) in methods {
        let descriptor = if is_class { &classmethod } else { &partial };
        cls.setattr(name, descriptor.call1((core.getattr(function)?,))?)?;
    }
    Ok(())
}
