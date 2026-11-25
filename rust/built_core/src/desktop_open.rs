// SPDX-License-Identifier: GPL-3.0-or-later
//! Launch the system's registered handler outside a frozen runtime.
use pyo3::{exceptions::PyOSError, prelude::*, types::PyDict};
use std::path::PathBuf;
type Obj<'p> = Bound<'p, PyAny>;
#[pyfunction]
#[pyo3(signature=(environ=None))]
fn host_env<'p>(py: Python<'p>, environ: Option<&Obj<'p>>) -> PyResult<Bound<'p, PyDict>> {
    let env = py
        .import("built_core.render_blender")?
        .call_method1("clean_env", (environ,))?
        .cast_into::<PyDict>()?;
    let read = |k| env.call_method1("get", (k, ""))?.extract::<String>();
    let dir = read("APPDIR")?;
    if !dir.is_empty() && read("GIO_MODULE_DIR")?.starts_with(&dir) {
        env.del_item("GIO_MODULE_DIR")?;
    }
    Ok(env)
}
fn launch(py: Python, cmd: Vec<String>, kw: &Bound<PyDict>, quiet: bool) -> PyResult<bool> {
    let sp = py.import("subprocess")?;
    if quiet {
        for stream in ["stdout", "stdin", "stderr"] {
            kw.set_item(stream, sp.getattr("DEVNULL")?)?;
        }
    }
    match sp.call_method("Popen", (cmd,), Some(kw)) {
        Ok(_) => Ok(true),
        Err(e) if e.is_instance_of::<PyOSError>(py) => Ok(false),
        Err(e) => Err(e),
    }
}
#[pyfunction]
fn open_url(target: &Obj) -> PyResult<bool> {
    let py = target.py();
    let sys = py.import("sys")?;
    let os = py.import("os")?;
    let arg = target.str()?.to_string();
    let platform = sys.getattr("platform")?.extract::<String>()?;
    if platform == "win32" {
        return match os.call_method1("startfile", (arg,)) {
            Ok(_) => Ok(true),
            Err(e) if e.is_instance_of::<PyOSError>(py) => Ok(false),
            Err(e) => Err(e),
        };
    }
    let env = os.getattr("environ")?;
    let truth = |k| env.call_method1("get", (k,))?.is_truthy();
    let frozen = sys
        .getattr_opt("frozen")?
        .map(|v| v.is_truthy())
        .transpose()?
        .unwrap_or(false);
    let outside = platform == "darwin"
        || (platform.starts_with("linux")
            && (frozen || truth("APPIMAGE")?)
            && !truth("FLATPAK_ID")?
            && !truth("SNAP")?);
    let opener = if platform == "darwin" {
        Some("/usr/bin/open".to_owned())
    } else {
        py.import("shutil")?
            .call_method1("which", ("xdg-open",))?
            .extract()?
    };
    let Some(opener) = opener else {
        return Ok(false);
    };
    let kw = PyDict::new(py);
    kw.set_item("start_new_session", true)?;
    kw.set_item(
        "env",
        if outside {
            Some(host_env(py, Some(&env))?)
        } else {
            None
        },
    )?;
    launch(py, vec![opener, arg], &kw, true)
}
#[pyfunction]
fn reveal(py: Python, path: PathBuf) -> PyResult<bool> {
    let platform = py.import("sys")?.getattr("platform")?.extract::<String>()?;
    if path.exists() {
        let args = if platform == "darwin" {
            Some(("open", "-R", true))
        } else if platform.starts_with("win") {
            Some(("explorer", "/select,", false))
        } else {
            None
        };
        if let Some((cmd, flag, quiet)) = args {
            if launch(
                py,
                vec![cmd.into(), flag.into(), path.to_string_lossy().into_owned()],
                &PyDict::new(py),
                quiet,
            )? {
                return Ok(true);
            }
        }
    }
    let folder = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    if folder.exists() {
        open_url(&folder.into_pyobject(py)?)
    } else {
        Ok(false)
    }
}
pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    let name = "built_core.desktop_open";
    let s = PyModule::new(m.py(), name)?;
    for f in [
        wrap_pyfunction!(host_env, &s)?,
        wrap_pyfunction!(open_url, &s)?,
        wrap_pyfunction!(reveal, &s)?,
    ] {
        s.add_function(f)?;
    }
    m.py()
        .import("sys")?
        .getattr("modules")?
        .set_item(name, &s)?;
    m.add("desktop_open", s)
}
