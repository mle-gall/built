// SPDX-License-Identifier: GPL-3.0-or-later
use crate::settings_rows::tr;
use pyo3::{
    prelude::*,
    types::{IntoPyDict, PyDict, PyList},
};
type Obj<'p> = Bound<'p, PyAny>;
#[pyclass]
struct ScaleEditor {
    window: Py<PyAny>,
    group: Py<PyAny>,
    closed: Option<Py<PyAny>>,
}
impl ScaleEditor {
    fn factors(&self, py: Python) -> PyResult<[f64; 3]> {
        py.import("core.group_scale")?
            .call_method1("group_scale", (self.group.bind(py),))?
            .extract()
    }
    fn refresh(&self, form: &Obj) -> PyResult<()> {
        let py = form.py();
        let format = py.import("builtins")?.getattr("format")?;
        for (axis, value) in ["x", "y", "z"].into_iter().zip(self.factors(py)?) {
            form.call_method1("set", (axis, format.call1((value, ".6g"))?))?;
        }
        Ok(())
    }
    fn apply(&self, form: &Obj, factors: [f64; 3]) -> PyResult<()> {
        let py = form.py();
        let vp = self.window.bind(py).getattr("viewport")?;
        let args = PyDict::new(py);
        args.set_item("placement", self.group.bind(py).getattr("uid")?)?;
        args.set_item("factors", factors)?;
        match py
            .import("core.protocol.commands")?
            .getattr("run")?
            .call((vp.getattr("history")?, "set_group_scale"), Some(&args))
        {
            Ok(_) => {
                vp.call_method0("update")?;
            }
            Err(e) if e.is_instance_of::<pyo3::exceptions::PyKeyError>(py) => {}
            Err(e) => return Err(e),
        }
        self.refresh(form)
    }
}
#[pymethods]
impl ScaleEditor {
    fn change(&self, form: &Obj, field: &str, value: &str) -> PyResult<()> {
        let mut scale = self.factors(form.py())?;
        if let (Some(i), Ok(x)) = (
            ["x", "y", "z"].iter().position(|s| *s == field),
            value.trim().replace(',', ".").parse::<f64>(),
        ) {
            scale[i] = x;
            if scale.iter().all(|x| x.is_finite()) {
                return self.apply(form, scale);
            }
        }
        self.refresh(form)
    }
    fn action(&self, form: &Obj, key: &str) -> PyResult<()> {
        if key == "reset" {
            return self.apply(form, [1.; 3]);
        }
        if matches!(key, "undo" | "redo") {
            let vp = self.window.bind(form.py()).getattr("viewport")?;
            vp.getattr("history")?.call_method0(key)?;
            self.refresh(form)?;
            vp.call_method0("update")?;
        }
        Ok(())
    }
    fn finished(&self, py: Python, _answer: &Obj, _values: &Obj) -> PyResult<()> {
        if let Some(cb) = &self.closed {
            cb.call0(py)?;
        }
        Ok(())
    }
}
#[pyfunction]
#[pyo3(signature=(window,group,done=None))]
pub(crate) fn ask_scale(window: &Obj, group: Py<PyAny>, done: Option<Py<PyAny>>) -> PyResult<()> {
    let py = window.py();
    let editor = Bound::new(
        py,
        ScaleEditor {
            window: window.clone().unbind(),
            group,
            closed: done,
        },
    )?;
    let forms = py.import("ingetrazo_render.chrome_forms")?;
    let fields = PyList::empty(py);
    let field = forms.getattr("Field")?;
    for (axis, v) in ["x", "y", "z"]
        .into_iter()
        .zip(editor.borrow().factors(py)?)
    {
        let value = py.import("builtins")?.call_method1("format", (v, ".6g"))?;
        fields.append(field.call(
            (axis, "edit", axis.to_uppercase(), value),
            Some(&[("commit_on_blur", true)].into_py_dict(py)?),
        )?)?;
    }
    for (key, label) in [("reset", "Reset scale"), ("undo", "Undo"), ("redo", "Redo")] {
        fields.append(field.call1((key, "button", tr(py, label)?))?)?;
    }
    let kw = PyDict::new(py);
    kw.set_item("buttons", [("ok", tr(py, "Close")?)])?;
    kw.set_item("default", py.None())?;
    kw.set_item("cancel", "ok")?;
    for (key, name) in [
        ("on_change", "change"),
        ("on_button", "action"),
        ("on_done", "finished"),
    ] {
        kw.set_item(key, editor.getattr(name)?)?;
    }
    let form = forms
        .getattr("Form")?
        .call((tr(py, "Scale")?, fields), Some(&kw))?;
    forms.call_method1("show", (window, form))?;
    Ok(())
}
