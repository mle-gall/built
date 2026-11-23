// SPDX-License-Identifier: GPL-3.0-or-later
//! Per-document history. Stack entries are host commands while some tools
//! still implement Python inverses. Existing scripting clients edit these
//! lists directly when folding a group-edit session or cancelling a preview.
use std::collections::VecDeque;

use pyo3::class::gc::{PyTraverseError, PyVisit};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

#[pyclass(module = "ingetrazo_core")]
pub struct DocumentHistory {
    #[pyo3(get)]
    undo: Py<PyList>,
    #[pyo3(get)]
    redo: Py<PyList>,
    #[pyo3(get)]
    subscribers: Py<PyList>,
    #[pyo3(get)]
    revision: u64,
    journal: VecDeque<(u64, String, String)>,
}

#[pymethods]
impl DocumentHistory {
    #[new]
    fn new(py: Python<'_>) -> Self {
        Self {
            undo: PyList::empty(py).unbind(),
            redo: PyList::empty(py).unbind(),
            subscribers: PyList::empty(py).unbind(),
            revision: 0,
            journal: VecDeque::new(),
        }
    }

    fn push(&self, py: Python<'_>, command: &Bound<'_, PyAny>, limit: usize) -> PyResult<()> {
        let undo = self.undo.bind(py);
        undo.append(command)?;
        if limit > 0 && undo.len() > limit {
            undo.del_slice(0, undo.len() - limit)?;
        }
        let redo = self.redo.bind(py);
        redo.del_slice(0, redo.len())
    }

    fn advance(&mut self, phase: String, command: String) {
        self.revision += 1;
        if self.journal.len() == 256 {
            self.journal.pop_front();
        }
        self.journal.push_back((self.revision, phase, command));
    }

    fn changes_since<'py>(&self, py: Python<'py>, since: u64) -> PyResult<Bound<'py, PyDict>> {
        let entries = PyList::empty(py);
        for (rev, phase, command) in self.journal.iter().filter(|(r, _, _)| *r > since) {
            let entry = PyDict::new(py);
            entry.set_item("revision", rev)?;
            entry.set_item("phase", phase)?;
            entry.set_item("command", command)?;
            entries.append(entry)?;
        }
        let result = PyDict::new(py);
        let floor = self
            .journal
            .front()
            .map_or(self.revision, |entry| entry.0 - 1);
        result.set_item("revision", self.revision)?;
        result.set_item("complete", since >= floor)?;
        result.set_item("changes", entries)?;
        Ok(result)
    }

    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.undo)?;
        visit.call(&self.redo)?;
        visit.call(&self.subscribers)
    }
}
