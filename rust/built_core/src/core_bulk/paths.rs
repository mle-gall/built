// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
};
fn path(py: Python<'_>, v: PathBuf) -> PyResult<Obj<'_>> {
    py.import("pathlib")?.call_method1("Path", (v,))
}
#[pyfunction]
pub(super) fn app_root(py: Python<'_>) -> PyResult<Obj<'_>> {
    if let Some(v) = py
        .import("sys")?
        .getattr_opt("_MEIPASS")?
        .filter(|v| v.is_truthy().unwrap_or(false))
    {
        return py.import("pathlib")?.call_method1("Path", (v,));
    }
    let file = py
        .import("core")?
        .getattr("__file__")?
        .extract::<PathBuf>()?;
    let file = fs::canonicalize(file)?;
    path(py, file.parent().and_then(Path::parent).unwrap().to_owned())
}
#[pyfunction]
fn is_frozen(py: Python<'_>) -> PyResult<bool> {
    Ok(py
        .import("sys")?
        .getattr_opt("frozen")?
        .is_some_and(|v| v.is(pyo3::types::PyBool::new(py, true))))
}
#[pyfunction]
fn user_log_dir(py: Python<'_>) -> PyResult<Obj<'_>> {
    let home = py
        .import("pathlib")?
        .getattr("Path")?
        .call_method0("home")?
        .extract::<PathBuf>()?;
    let platform = py.import("sys")?.getattr("platform")?.extract::<String>()?;
    let configured = |key| env::var_os(key).filter(|s| !s.is_empty()).map(PathBuf::from);
    let folder = if let Some(logs) = configured("BUILT_LOG_DIR") {
        logs
    } else if let Some(profile) = configured("BUILT_SETTINGS_DIR") {
        profile.join("logs")
    } else if platform.starts_with("win") {
        env::var_os("LOCALAPPDATA")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Local"))
            .join("Built")
    } else if platform == "darwin" {
        home.join("Library/Logs/Built")
    } else {
        env::var_os("XDG_STATE_HOME")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/state"))
            .join("built")
    };
    let temp = py
        .import("tempfile")?
        .call_method0("gettempdir")?
        .extract::<PathBuf>()?;
    for d in [folder, temp.join("built")] {
        if fs::create_dir_all(&d).is_ok() {
            let probe = d.join(".write-test");
            if fs::write(&probe, [])
                .and_then(|_| fs::remove_file(probe))
                .is_ok()
            {
                return path(py, d);
            }
        }
    }
    path(py, temp)
}
#[pyfunction]
fn prune_handled_com_errors(path: PathBuf) {
    let Ok(bytes) = fs::read(&path) else { return };
    let text = String::from_utf8_lossy(&bytes);
    if !text.contains("code 0x8001") {
        return;
    }
    let mut kept = String::new();
    let mut omit = false;
    for line in text.split_inclusive('\n') {
        if line.starts_with("Windows fatal exception:") || line.starts_with("Fatal Python error:") {
            omit = line
                .to_ascii_lowercase()
                .starts_with("windows fatal exception: code 0x8001");
        }
        if !omit {
            kept.push_str(line);
        }
    }
    if let Ok(mut f) = fs::OpenOptions::new()
        .truncate(true)
        .write(true)
        .open(&path)
    {
        let _ = f.write_all(kept.as_bytes());
    }
}
#[pyfunction(pass_module,signature=(name="built-crash.log"))]
fn open_crash_log<'p>(m: &Module<'p>, name: &str) -> PyResult<Obj<'p>> {
    let py = m.py();
    let path = m
        .call_method0("user_log_dir")?
        .call_method1("joinpath", (name,))?;
    m.call_method1("prune_handled_com_errors", (&path,))?;
    let kw = pyo3::types::PyDict::new(py);
    kw.set_item("encoding", "utf-8")?;
    match py
        .import("builtins")?
        .getattr("open")?
        .call((path, "a"), Some(&kw))
    {
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyOSError>(py) => {
            Ok(py.None().into_bound(py))
        }
        res => res,
    }
}
#[pyfunction]
fn cache_directory(py: Python<'_>) -> PyResult<Obj<'_>> {
    let dir = env::var_os("BUILT_SETTINGS_DIR").filter(|s| !s.is_empty());
    py.import("pathlib")?.call_method1(
        "Path",
        (py.import("built_render")?
            .call_method1("cache_directory", (dir,))?,),
    )
}
pub(super) fn install(m: &Module<'_>) -> PyResult<()> {
    for f in [
        wrap_pyfunction!(app_root, m)?,
        wrap_pyfunction!(is_frozen, m)?,
        wrap_pyfunction!(user_log_dir, m)?,
        wrap_pyfunction!(prune_handled_com_errors, m)?,
        wrap_pyfunction!(open_crash_log, m)?,
        wrap_pyfunction!(cache_directory, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
