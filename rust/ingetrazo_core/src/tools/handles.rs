// SPDX-License-Identifier: GPL-3.0-or-later
//! Stable names for host records. Store views of the same entity compare
//! equal. Unhashable extension objects are retained by identity.
use pyo3::{exceptions::PyTypeError, prelude::*, types::PyDict};

#[pyclass(module = "ingetrazo_core")]
pub struct ToolHandles {
    ids: Py<PyDict>,
    objects: Vec<Py<PyAny>>,
    first: u64,
    limit: Option<usize>,
}

#[pymethods]
impl ToolHandles {
    #[new]
    #[pyo3(signature=(limit=Some(4096)))]
    pub fn new(py: Python<'_>, limit: Option<usize>) -> Self {
        Self {
            ids: PyDict::new(py).unbind(),
            objects: Vec::new(),
            first: 1,
            limit,
        }
    }

    pub fn id(&mut self, obj: &Bound<'_, PyAny>) -> PyResult<u64> {
        let py = obj.py();
        let key = match obj.hash() {
            Ok(_) => (false, obj).into_pyobject(py)?,
            Err(e) if e.is_instance_of::<PyTypeError>(py) => {
                (true, obj.as_ptr() as usize).into_pyobject(py)?
            }
            Err(e) => return Err(e),
        };
        if let Some(id) = self.ids.bind(py).get_item(&key)? {
            return id.extract();
        }
        if self.limit.is_some_and(|n| self.objects.len() >= n) {
            self.clear(py);
        }
        let id = self.first + self.objects.len() as u64;
        self.ids.bind(py).set_item(key, id)?;
        self.objects.push(obj.clone().unbind());
        Ok(id)
    }

    #[pyo3(signature=(ident))]
    pub fn get<'py>(&self, py: Python<'py>, ident: Option<u64>) -> Option<Bound<'py, PyAny>> {
        let i = ident?.checked_sub(self.first)?;
        Some(self.objects.get(usize::try_from(i).ok()?)?.bind(py).clone())
    }

    fn clear(&mut self, py: Python<'_>) {
        self.first += self.objects.len() as u64;
        self.objects.clear();
        self.ids.bind(py).clear();
    }
}
