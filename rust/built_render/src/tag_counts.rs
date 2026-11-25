// SPDX-License-Identifier: GPL-3.0-or-later
//! Incremental entity counts for the Drawing inspector, using the document's
//! lazy-mesh tally. A partial count never reaches a row.
use pyo3::{prelude::*, types::PyDict};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[pyclass(module = "built_render")]
#[derive(Default)]
pub struct TagCounts {
    version: Option<(u64, i64)>,
    pending: Option<Py<PyAny>>,
    counts: Option<HashMap<String, u64>>,
}

impl TagCounts {
    pub fn cancel_pending(&mut self) {
        if self.pending.is_some() {
            *self = Self::default();
        }
    }
}

#[pymethods]
impl TagCounts {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    /// True while the next frame has work to do.
    pub fn tick(&mut self, history: &Bound<'_, PyAny>) -> PyResult<bool> {
        let scene = history.getattr("scene")?;
        let revision = || scene.getattr("content_version")?.extract::<i64>();
        let document = scene
            .getattr("_root")?
            .getattr("_doc_key")?
            .extract::<u64>()?;
        let version = (document, revision()?);
        if self.version != Some(version) {
            *self = Self::default();
            let py = history.py();
            let options = PyDict::new(py);
            options.set_item("history", history)?;
            let iter = py.import("built_core.purge")?.call_method(
                "layer_count_steps",
                (&scene,),
                Some(&options),
            )?;
            self.pending = Some(iter.unbind());
            self.version = Some(version);
        }
        let Some(iter) = self.pending.as_ref() else {
            return Ok(false);
        };
        let deadline = Instant::now() + Duration::from_millis(4);
        loop {
            let result = iter.call_method0(history.py(), "__next__")?;
            if !result.is_none(history.py()) {
                if revision()? != version.1 {
                    *self = Self::default();
                    return Ok(true);
                }
                self.counts = Some(result.extract(history.py())?);
                self.pending = None;
                return Ok(false);
            }
            if Instant::now() >= deadline {
                return Ok(true);
            }
        }
    }

    pub fn count(&self, key: &str) -> Option<u64> {
        self.counts
            .as_ref()
            .map(|counts| counts.get(key).copied().unwrap_or(0))
    }
}
