// SPDX-License-Identifier: GPL-3.0-or-later
//! Each live geometry facade (including a deferred mesh) names one record.
//! A weak index accepts legacy callers which construct a placement from
//! its mesh. Placements and undo records hold the definition itself.
use pyo3::gc::PyVisit;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::PyTraverseError;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

static INDEX: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
static NEXT: AtomicU64 = AtomicU64::new(1);

#[pyclass(frozen, weakref, module = "ingetrazo_core")]
pub struct Definition {
    #[pyo3(get)]
    pub key: u64,
    #[pyo3(get)]
    pub geometry: Py<PyAny>,
}

#[pymethods]
impl Definition {
    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.geometry)
    }
}

#[derive(Debug)]
pub struct DefinitionRef(pub Py<Definition>);

impl PartialEq for DefinitionRef {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_ptr() == other.0.as_ptr()
    }
}

impl DefinitionRef {
    pub fn for_geometry(mesh: &Bound<'_, PyAny>) -> PyResult<Arc<Self>> {
        let py = mesh.py();
        let index = INDEX.get_or_try_init(py, || -> PyResult<_> {
            Ok(py.import("weakref")?.getattr("WeakValueDictionary")?.call0()?.unbind())
        })?.bind(py);
        let address = mesh.as_ptr() as usize;
        let existing = index.call_method1("get", (address,))?;
        let definition = if existing.is_none() {
            let new = Bound::new(py, Definition {
                key: NEXT.fetch_add(1, Ordering::Relaxed),
                geometry: mesh.clone().unbind(),
            })?;
            index.set_item(address, &new)?;
            new
        } else {
            existing.cast_into::<Definition>()?
        };
        Ok(Arc::new(Self(definition.unbind())))
    }
}
