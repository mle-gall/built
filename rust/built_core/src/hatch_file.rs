//! A PAT file extends one model's pattern catalogue in one undoable edit.
use pyo3::{exceptions::PyValueError, prelude::*, types::PyDict};
use std::{fs::File, io::Read, path::PathBuf};

#[pyclass(dict)]
struct ImportPatterns {
    before: Py<PyDict>,
    after: Py<PyDict>,
}

fn replace(scene: &Bound<'_, PyAny>, patterns: &Py<PyDict>) -> PyResult<()> {
    let py = scene.py();
    scene.setattr(
        "hatch_patterns",
        py.import("copy")?.call_method1("deepcopy", (patterns,))?,
    )?;
    let version: u64 = scene.getattr("version")?.extract()?;
    scene.setattr("version", version + 1)
}

#[pymethods]
impl ImportPatterns {
    #[classattr]
    #[pyo3(name = "keeps_rows")]
    const KEEPS_ROWS: bool = true;
    #[classattr]
    #[pyo3(name = "history_name")]
    const HISTORY_NAME: &'static str = "Import hatch patterns";

    fn r#do(&self, scene: &Bound<'_, PyAny>) -> PyResult<()> {
        replace(scene, &self.after)
    }
    fn undo(&self, scene: &Bound<'_, PyAny>) -> PyResult<()> {
        replace(scene, &self.before)
    }
}

pub(super) fn read_pattern(path: PathBuf) -> PyResult<String> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(PyValueError::new_err("The pattern file exceeds 4 MiB."));
    }
    let text = String::from_utf8(bytes)
        .unwrap_or_else(|e| e.into_bytes().into_iter().map(char::from).collect());
    Ok(text.trim_start_matches('\u{feff}').to_owned())
}

#[pyfunction]
pub fn import_hatch_file(history: &Bound<'_, PyAny>, path: PathBuf) -> PyResult<usize> {
    let text = read_pattern(path)?;
    let py = history.py();
    let found = py
        .import("built_core.hatches")?
        .call_method1("parse_pat", (text,))?
        .cast_into::<PyDict>()?;
    let count = found.len();
    if count == 0 {
        return Ok(0);
    }
    let current = history.getattr("scene")?.getattr("hatch_patterns")?;
    let before = py
        .import("copy")?
        .call_method1("deepcopy", (&current,))?
        .cast_into::<PyDict>()?;
    let after = before.copy()?;
    after.call_method1("update", (&found,))?;
    if !after.eq(&before)? {
        let step = Py::new(
            py,
            ImportPatterns {
                before: before.unbind(),
                after: after.unbind(),
            },
        )?;
        history.call_method1("execute", (step,))?;
        for attr in ["last_rejected", "last_error"] {
            if let Some(error) = history.getattr(attr)?.extract::<Option<String>>()? {
                return Err(PyValueError::new_err(error));
            }
        }
    }
    Ok(count)
}
