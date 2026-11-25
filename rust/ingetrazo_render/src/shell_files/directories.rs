//! The initial directory belongs to all four pickers. A cancelled picker
//! keeps the previous choice, including when an old network share is offline.
use super::*;
use std::path::{Path, PathBuf};

const KEY: &str = "files/last_dir";

fn settings(py: Python<'_>) -> PyResult<Any<'_>> {
    py.import("core.settings")?.call_method0("settings")
}

#[pyfunction]
#[pyo3(signature=(fallback=None))]
pub(super) fn last_dir(py: Python, fallback: Option<PathBuf>) -> PyResult<String> {
    let value = settings(py)?.call_method1("value", (KEY, ""))?;
    let saved = value.extract::<PathBuf>().ok();
    let directory = saved
        .into_iter()
        .chain(fallback)
        .find(|p| p.is_dir())
        .or_else(|| crate::platform_paths::documents_directory().filter(|p| p.is_dir()))
        .or_else(std::env::home_dir)
        .ok_or_else(|| pyo3::exceptions::PyOSError::new_err("No home directory"))?;
    Ok(directory.to_string_lossy().into_owned())
}

#[pyfunction]
#[pyo3(signature=(path=None))]
pub(super) fn remember(py: Python, path: Option<PathBuf>) -> PyResult<()> {
    if let Some(path) = path.filter(|p| !p.as_os_str().is_empty()) {
        let folder = if path.is_dir() {
            Some(path.as_path())
        } else {
            path.parent()
        };
        if let Some(folder) = folder.filter(|p| p.is_dir()) {
            settings(py)?.call_method1("setValue", (KEY, folder.to_string_lossy().as_ref()))?;
        }
    }
    Ok(())
}

#[pyfunction]
#[pyo3(signature=(directory=None,fallback=None))]
pub(super) fn start_in(
    py: Python,
    directory: Option<PathBuf>,
    fallback: Option<PathBuf>,
) -> PyResult<String> {
    if let Some(path) = directory.filter(|p| !p.as_os_str().is_empty()) {
        if path.is_dir() || path.parent().is_some_and(Path::is_dir) {
            return Ok(path.to_string_lossy().into_owned());
        }
        if let Some(name) = path.file_name() {
            return Ok(Path::new(&last_dir(py, fallback)?)
                .join(name)
                .to_string_lossy()
                .into_owned());
        }
    }
    last_dir(py, fallback)
}

fn document_folder(parent: &Any) -> PyResult<Option<PathBuf>> {
    let path = |w: &Any| -> PyResult<Option<PathBuf>> {
        let Some(p) = w.getattr_opt("_current_path")?.filter(|p| !p.is_none()) else {
            return Ok(None);
        };
        Ok(p.extract::<PathBuf>()?
            .parent()
            .filter(|p| p.is_dir())
            .map(Path::to_owned))
    };
    if let Some(dir) = path(parent)? {
        return Ok(Some(dir));
    }
    if let Some(w) = parent.getattr_opt("_window")? {
        if let Some(dir) = path(&w)? {
            return Ok(Some(dir));
        }
    }
    if let Some(window) = parent.getattr_opt("window")?.filter(|w| w.is_callable()) {
        return path(&window.call0()?);
    }
    Ok(None)
}

pub(super) fn initial(
    py: Python,
    directory: Option<&Any>,
    parent: Option<&Any>,
) -> PyResult<String> {
    let dir = directory
        .filter(|v| !v.is_none())
        .map(|v| v.extract::<PathBuf>())
        .transpose()?;
    let fallback = parent.map(document_folder).transpose()?.flatten();
    start_in(py, dir, fallback)
}
pub(super) fn selected(value: &Any) -> PyResult<()> {
    if value.is_truthy()? {
        remember(value.py(), Some(value.extract()?))?;
    }
    Ok(())
}
