//! Deliver an outstanding worker future on its originating window's event loop.
use pyo3::{prelude::*, types::PyWeakrefReference};

#[pyclass]
pub struct WindowJob {
    owner: Py<PyWeakrefReference>,
    future: Py<PyAny>,
    ready: Option<Py<PyAny>>,
    error: Option<Py<PyAny>>,
}

#[pymethods]
impl WindowJob {
    #[staticmethod]
    fn submit(
        py: Python<'_>,
        owner: &Bound<'_, PyAny>,
        future: Py<PyAny>,
        ready: Py<PyAny>,
        error: Py<PyAny>,
    ) -> PyResult<Py<Self>> {
        let job = Py::new(
            py,
            Self {
                owner: PyWeakrefReference::new(owner)?.unbind(),
                future,
                ready: Some(ready),
                error: Some(error),
            },
        )?;
        owner
            .getattr("_host")?
            .getattr("timers")?
            .call_method1("later", (owner, 0, job.bind(py)))?;
        Ok(job)
    }

    fn abort(&mut self, py: Python<'_>) -> PyResult<()> {
        self.ready.take();
        self.error.take();
        self.future.bind(py).call_method0("cancel")?;
        Ok(())
    }

    fn __call__(this: &Bound<'_, Self>) -> PyResult<()> {
        let py = this.py();
        let mut state = this.borrow_mut();
        let Some(owner) = state.owner.bind(py).upgrade() else {
            return Ok(());
        };
        if state.ready.is_none() {
            return Ok(());
        }
        if !state.future.bind(py).call_method0("done")?.is_truthy()? {
            drop(state);
            let host = owner.getattr("_host")?;
            host.getattr("timers")?
                .call_method1("later", (&owner, 40, this))?;
            return Ok(());
        }
        let result = state.future.bind(py).call_method0("result");
        let ready = state.ready.take().unwrap();
        let error = state.error.take().unwrap();
        drop(state);
        match result {
            Ok(data) => ready.call1(py, (data,))?,
            Err(exc) => error.call1(py, (exc.value(py).str()?,))?,
        };
        Ok(())
    }
}
