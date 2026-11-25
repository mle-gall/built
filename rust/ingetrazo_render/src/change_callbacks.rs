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

/// A non-data descriptor. Its first lookup installs a per-instance subscriber list.
#[pyclass(module = "ingetrazo_render")]
pub struct CallbackSlot {
    name: Option<String>,
}

#[pymethods]
impl CallbackSlot {
    #[new]
    fn new() -> Self {
        Self { name: None }
    }

    fn __set_name__(&mut self, _owner: &Bound<'_, PyAny>, name: String) {
        self.name = Some(name);
    }

    fn __get__<'py>(
        slf: &Bound<'py, Self>,
        instance: Option<&Bound<'py, PyAny>>,
        _owner: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let Some(instance) = instance else {
            return Ok(slf.clone().into_any());
        };
        let callbacks = Bound::new(slf.py(), ChangeCallbacks::default())?;
        let slot = slf.borrow();
        let name = slot
            .name
            .as_ref()
            .ok_or_else(|| pyo3::exceptions::PyAttributeError::new_err("unnamed callback slot"))?;
        instance.getattr("__dict__")?.set_item(name, &callbacks)?;
        Ok(callbacks.into_any())
    }
}
