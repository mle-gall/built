//! Point references survive successive cursor queries and reset with the document.
use crate::snap::{norm, sub, P};
use pyo3::prelude::*;

#[pyclass]
#[derive(Default)]
pub struct InferenceMemory {
    #[pyo3(get, set)]
    encouraged: Vec<P>,
    #[pyo3(get, set)]
    dwell: Option<P>,
    #[pyo3(get, set)]
    acquired: Option<P>,
}

#[pymethods]
impl InferenceMemory {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    fn rest(&mut self, p: P) -> bool {
        if self.dwell.is_some_and(|q| norm(sub(q, p)) < 1e-6) {
            return false;
        }
        self.dwell = Some(p);
        true
    }
    fn encourage(&mut self, p: P) {
        self.encouraged.retain(|&q| norm(sub(q, p)) > 1e-6);
        self.encouraged.push(p);
        if self.encouraged.len() > 2 {
            self.encouraged.drain(..self.encouraged.len() - 2);
        }
        self.acquired = Some(p);
    }
}
