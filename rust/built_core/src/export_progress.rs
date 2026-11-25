// SPDX-License-Identifier: GPL-3.0-or-later
//! A scoped progress callback for exports, isolated across threads and nested jobs.
use pyo3::{prelude::*, types::PyDict};
use std::time::Instant;
type Obj<'p> = Bound<'p, PyAny>;

#[pyclass]
struct Reporting {
    context: Py<PyAny>,
    callback: Py<PyAny>,
    token: Option<Py<PyAny>>,
    told: Instant,
    fraction: f64,
}
#[pymethods]
impl Reporting {
    fn __enter__(slf: &Bound<'_, Self>) -> PyResult<()> {
        let token = slf
            .borrow()
            .context
            .bind(slf.py())
            .call_method1("set", (slf,))?;
        let mut scope = slf.borrow_mut();
        scope.token = Some(token.unbind());
        scope.told = Instant::now();
        Ok(())
    }
    fn __exit__(&mut self, py: Python<'_>, _kind: &Obj, _err: &Obj, _tb: &Obj) -> PyResult<()> {
        if let Some(token) = self.token.take() {
            self.context.bind(py).call_method1("reset", (token,))?;
        }
        Ok(())
    }
}

#[pyfunction(pass_module)]
fn reporting(module: &Bound<PyModule>, callback: Py<PyAny>) -> PyResult<Reporting> {
    Ok(Reporting {
        context: module.getattr("_current")?.unbind(),
        callback,
        token: None,
        told: Instant::now(),
        fraction: 0.0,
    })
}

#[pyfunction(pass_module)]
fn reporting_active(module: &Bound<PyModule>) -> PyResult<bool> {
    Ok(!module.getattr("_current")?.call_method0("get")?.is_none())
}

#[pyfunction(pass_module, signature = (done=None))]
fn tick(module: &Bound<PyModule>, done: Option<f64>) -> PyResult<()> {
    let current = module.getattr("_current")?.call_method0("get")?;
    if current.is_none() {
        return Ok(());
    }
    let scope = current.cast::<Reporting>()?;
    let (callback, fraction) = {
        let mut state = scope.borrow_mut();
        if let Some(done) = done {
            state.fraction = done;
        }
        if state.told.elapsed().as_secs_f64() < module.getattr("REPORT_EVERY")?.extract::<f64>()? {
            return Ok(());
        }
        state.told = Instant::now();
        (state.callback.clone_ref(module.py()), state.fraction)
    };
    callback.call1(module.py(), (fraction,))?;
    scope.borrow_mut().told = Instant::now();
    Ok(())
}

pub(crate) fn register(parent: &Bound<PyModule>) -> PyResult<()> {
    let py = parent.py();
    let m = PyModule::new(py, "built_core.progress")?;
    let kw = PyDict::new(py);
    kw.set_item("default", py.None())?;
    m.add(
        "_current",
        py.import("contextvars")?
            .getattr("ContextVar")?
            .call(("export progress",), Some(&kw))?,
    )?;
    m.add("REPORT_EVERY", 0.1)?;
    m.add_function(wrap_pyfunction!(reporting, &m)?)?;
    m.add_function(wrap_pyfunction!(reporting_active, &m)?)?;
    m.add_function(wrap_pyfunction!(tick, &m)?)?;
    py.import("sys")?
        .getattr("modules")?
        .set_item("built_core.progress", &m)?;
    parent.add("progress", m)
}
