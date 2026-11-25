// SPDX-License-Identifier: GPL-3.0-or-later
//! Defer cyclic destruction on workers until the main thread can collect.
use pyo3::prelude::*;
use pyo3::types::PyList;

#[pyclass(frozen)]
struct MainThreadCollector {
    main: u64,
    thread_id: Py<PyAny>,
    gc: Py<PyModule>,
}

#[pymethods]
impl MainThreadCollector {
    fn __call__(&self, py: Python<'_>, phase: &str, _info: &Bound<PyAny>) -> PyResult<()> {
        if phase == "start" {
            let gc = self.gc.bind(py);
            if self.thread_id.call0(py)?.extract::<u64>(py)? != self.main {
                gc.call_method0("freeze")?;
            } else if gc.call_method0("get_freeze_count")?.is_truthy()? {
                gc.call_method0("unfreeze")?;
            }
        }
        Ok(())
    }
}

#[pyfunction]
pub(crate) fn install_main_thread_collector(py: Python<'_>) -> PyResult<Py<PyAny>> {
    let gc = py.import("gc")?;
    let callbacks = gc.getattr("callbacks")?.cast_into::<PyList>()?;
    for cb in callbacks.iter() {
        if cb.is_instance_of::<MainThreadCollector>() {
            return Ok(cb.unbind());
        }
    }
    let threading = py.import("threading")?;
    let guard = Py::new(
        py,
        MainThreadCollector {
            main: threading
                .call_method0("main_thread")?
                .getattr("ident")?
                .extract()?,
            thread_id: threading.getattr("get_ident")?.unbind(),
            gc: gc.unbind(),
        },
    )?;
    callbacks.append(&guard)?;
    Ok(guard.into_any())
}
