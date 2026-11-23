// SPDX-License-Identifier: GPL-3.0-or-later
//! Each live geometry facade (including a deferred mesh) names one record.
//! A weak index accepts legacy callers which construct a placement from
//! its mesh. Placements and undo records hold the definition itself.
use pyo3::gc::PyVisit;
use pyo3::prelude::*;
use pyo3::types::{PyWeakrefMethods, PyWeakrefReference};
use super::{FastMap, FastSet, Key};
use pyo3::PyTraverseError;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

type Index = FastMap<usize, (u64, Py<PyWeakrefReference>)>;
static INDEX: LazyLock<Mutex<Index>> = LazyLock::new(|| Mutex::new(Index::default()));
static NEXT: AtomicU64 = AtomicU64::new(1);

#[pyclass(frozen, weakref, module = "ingetrazo_core")]
pub struct Definition {
    #[pyo3(get)]
    pub key: u64,
    #[pyo3(get)]
    pub geometry: Py<PyAny>,
    pub placements: Mutex<FastSet<Key>>,
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

impl Drop for Definition {
    fn drop(&mut self) {
        let address = self.geometry.as_ptr() as usize;
        let weak = {
            let mut index = INDEX.lock().unwrap();
            if index.get(&address).is_some_and(|entry| entry.0 == self.key) {
                index.remove(&address)
            } else {
                None
            }
        };
        drop(weak);
    }
}

impl DefinitionRef {
    pub fn for_geometry(mesh: &Bound<'_, PyAny>) -> PyResult<Arc<Self>> {
        let py = mesh.py();
        let address = mesh.as_ptr() as usize;
        let lookup = || {
            let index = INDEX.lock().unwrap();
            index.get(&address).and_then(|(_, weak)| weak.bind(py).upgrade())
        };
        if let Some(existing) = lookup() {
            return Ok(Arc::new(Self(existing.cast_into::<Definition>()?.unbind())));
        }
        let key = NEXT.fetch_add(1, Ordering::Relaxed);
        let new = Bound::new(py, Definition { key, geometry: mesh.clone().unbind(), placements: Mutex::default() })?;
        let weak = PyWeakrefReference::new(new.as_any())?;
        // Allocations above can run a Python finalizer and reenter this lookup.
        if let Some(existing) = lookup() {
            return Ok(Arc::new(Self(existing.cast_into::<Definition>()?.unbind())));
        }
        let previous = INDEX.lock().unwrap().insert(address, (key, weak.unbind()));
        drop(previous);
        Ok(Arc::new(Self(new.unbind())))
    }
}
