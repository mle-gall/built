//! Window notifications for a document. History steps arrive synchronously
//! through the feed. Explicit model changes and frame observations can report
//! the same version again, so subscribers only see one notice per stamp.
use pyo3::{prelude::*, types::PyWeakrefReference};
use std::sync::Mutex;

#[pyclass(module = "ingetrazo_core")]
pub struct DocumentWatch {
    document: Py<PyWeakrefReference>,
    last: Mutex<Option<(u64, u64)>>,
}

impl DocumentWatch {
    fn notify(&self, py: Python<'_>, force: bool) -> PyResult<()> {
        let Some(doc) = self.document.bind(py).upgrade() else {
            return Ok(());
        };
        let vp = doc.getattr("viewport")?;
        if vp.is_none() {
            return Ok(());
        }
        let version = vp.getattr("scene")?.getattr("version")?.extract()?;
        let revision = vp.getattr("history")?.getattr("revision")?.extract()?;
        {
            let mut last = self.last.lock().unwrap();
            let current = (version, revision);
            if !force && last.as_ref() == Some(&current) {
                return Ok(());
            }
            *last = Some(current);
        }
        // A detached tab's viewport follows the destination host. Looking up
        // the owner here avoids retaining either window or a stale callback.
        vp.getattr("_host")?
            .getattr("window")?
            .call_method1("_document_changed", (&doc, version))?;
        Ok(())
    }
}

#[pymethods]
impl DocumentWatch {
    #[new]
    fn new(document: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            document: PyWeakrefReference::new(document)?.unbind(),
            last: Mutex::new(None),
        })
    }

    fn __call__(&self, py: Python<'_>, _step: &Bound<'_, PyAny>) -> PyResult<()> {
        self.notify(py, false)
    }

    #[pyo3(signature = (force=false))]
    fn observe(&self, py: Python<'_>, force: bool) -> PyResult<()> {
        self.notify(py, force)
    }
}
