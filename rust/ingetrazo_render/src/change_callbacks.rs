//! View notifications delivered synchronously on the shell's event thread.
use pyo3::{prelude::*, types::PyTuple, PyTraverseError, PyVisit};
use std::cell::RefCell;

#[pyclass(unsendable)]
#[derive(Default)]
pub struct ChangeCallbacks(RefCell<Vec<Py<PyAny>>>);
#[pymethods]
impl ChangeCallbacks {
    #[new]
    fn new() -> Self {
        Self::default()
    }
    fn connect(&self, callback: Py<PyAny>) {
        self.0.borrow_mut().push(callback);
    }
    fn disconnect(&self, py: Python<'_>, callback: &Bound<'_, PyAny>) -> PyResult<()> {
        let callbacks = self.snapshot(py);
        for handler in callbacks {
            if handler.bind(py).eq(callback)? {
                self.0.borrow_mut().retain(|f| !f.is(&handler));
                break;
            }
        }
        Ok(())
    }
    #[pyo3(signature=(*args))]
    fn emit(&self, args: &Bound<'_, PyTuple>) -> PyResult<()> {
        let py = args.py();
        for cb in self.snapshot(py) {
            cb.call1(py, args)?;
        }
        Ok(())
    }
    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        if let Ok(callbacks) = self.0.try_borrow() {
            for cb in callbacks.iter() {
                visit.call(cb)?;
            }
        }
        Ok(())
    }
    fn __clear__(&mut self) {
        self.0.get_mut().clear();
    }
}
impl ChangeCallbacks {
    fn snapshot(&self, py: Python<'_>) -> Vec<Py<PyAny>> {
        self.0.borrow().iter().map(|f| f.clone_ref(py)).collect()
    }
}
