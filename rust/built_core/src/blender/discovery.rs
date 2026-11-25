// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use pyo3::exceptions::PyOSError;
use std::path::{Path, PathBuf};

#[pyclass(module = "built_core.render_blender", get_all, set_all)]
pub(super) struct BlenderFound {
    command: Py<PyList>,
    r#where: String,
}
#[pymethods]
impl BlenderFound {
    #[new]
    fn new(command: Py<PyList>, r#where: String) -> Self {
        Self { command, r#where }
    }
}
#[pyfunction]
fn in_flatpak() -> bool {
    Path::new("/.flatpak-info").exists()
}
#[pyfunction]
fn in_snap() -> bool {
    std::env::var_os("SNAP").is_some_and(|v| !v.is_empty())
}
fn which(py: Python<'_>, name: &str) -> PyResult<Option<String>> {
    py.import("shutil")?
        .call_method1("which", (name,))?
        .extract()
}
fn candidates(py: Python<'_>) -> PyResult<Vec<PathBuf>> {
    let mut out = Vec::new();
    let home = py
        .import("pathlib")?
        .getattr("Path")?
        .call_method0("home")?
        .extract::<PathBuf>()?;
    fn matching(out: &mut Vec<PathBuf>, root: &Path, prefix: &str, suffix: &str, bin: &str) {
        let mut paths = Vec::new();
        if let Ok(entries) = root.read_dir() {
            for entry in entries.flatten() {
                let n = entry.file_name();
                let n = n.to_string_lossy();
                if n.starts_with(prefix) && n.ends_with(suffix) {
                    paths.push(entry.path().join(bin));
                }
            }
        }
        paths.sort_unstable_by(|a, b| b.cmp(a));
        out.extend(paths);
    }
    if cfg!(windows) {
        for key in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Some(root) = std::env::var_os(key) {
                let root = PathBuf::from(root);
                matching(
                    &mut out,
                    &root.join("Blender Foundation"),
                    "Blender",
                    "",
                    "blender.exe",
                );
                out.push(root.join("Steam/steamapps/common/Blender/blender.exe"));
            }
        }
    } else if cfg!(target_os = "macos") {
        for base in [PathBuf::from("/Applications"), home.join("Applications")] {
            matching(&mut out, &base, "Blender", ".app", "Contents/MacOS/Blender");
        }
    } else {
        out.extend(
            [
                "/usr/bin/blender",
                "/usr/local/bin/blender",
                "/snap/bin/blender",
            ]
            .map(PathBuf::from),
        );
        out.push(home.join(".local/bin/blender"));
        matching(&mut out, Path::new("/opt"), "blender", "", "blender");
    }
    Ok(out)
}
fn run(py: Python<'_>, args: Vec<String>, timeout: f64) -> PyResult<Option<String>> {
    let sub = py.import("subprocess")?;
    let kw = PyDict::new(py);
    kw.set_item("capture_output", true)?;
    kw.set_item("text", true)?;
    kw.set_item("timeout", timeout)?;
    match sub.getattr("run")?.call((args,), Some(&kw)) {
        Ok(r) if r.getattr("returncode")?.eq(0)? => Ok(Some(r.getattr("stdout")?.extract()?)),
        Ok(_) => Ok(None),
        Err(e)
            if e.is_instance_of::<PyOSError>(py)
                || e.value(py).is_instance(&sub.getattr("TimeoutExpired")?)? =>
        {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}
#[pyfunction(signature=(args,timeout=10.0))]
fn _host(py: Python<'_>, args: Vec<String>, timeout: f64) -> PyResult<Option<String>> {
    let mut argv = vec!["flatpak-spawn".to_owned(), "--host".to_owned()];
    argv.extend(args);
    run(py, argv, timeout)
}
#[pyfunction]
fn app_bundle_executable(path: String) -> String {
    let bundle = Path::new(&path);
    if bundle
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("app"))
        && bundle.is_dir()
    {
        for name in ["Blender", "blender"] {
            let exe = bundle.join("Contents/MacOS").join(name);
            if exe.is_file() {
                return exe.to_string_lossy().into_owned();
            }
        }
    }
    path
}
#[pyfunction(pass_module,signature=(saved=None))]
fn find_blender(m: &Bound<PyModule>, saved: Option<String>) -> PyResult<Option<BlenderFound>> {
    let py = m.py();
    let saved = saved.filter(|v| !v.is_empty());
    let found = |cmd: Vec<&str>, label: &str| -> PyResult<_> {
        Ok(Some(BlenderFound::new(
            PyList::new(py, cmd)?.unbind(),
            label.into(),
        )))
    };
    let hosted = format!("Flatpak {FLAT_ID}");
    if m.call_method0("in_flatpak")?.is_truthy()? {
        if let Some(path) = saved {
            return found(vec!["flatpak-spawn", "--host", &path], &path);
        }
        let path = m
            .call_method1("_host", (vec!["sh", "-c", "command -v blender"],))?
            .extract::<Option<String>>()?
            .unwrap_or_default();
        let path = path.trim();
        if !path.is_empty() {
            return found(vec!["flatpak-spawn", "--host", path], path);
        }
        if !m
            .call_method1("_host", (vec!["flatpak", "info", FLAT_ID],))?
            .is_none()
        {
            return found(
                vec![
                    "flatpak-spawn",
                    "--host",
                    "flatpak",
                    "run",
                    "--filesystem=home",
                    FLAT_ID,
                ],
                &hosted,
            );
        }
        return Ok(None);
    }
    if let Some(path) = saved
        .map(app_bundle_executable)
        .filter(|s| Path::new(s).is_file())
    {
        return found(vec![&path], &path);
    }
    if let Some(p) = which(py, "blender")? {
        return found(vec![&p], &p);
    }
    for path in candidates(py)? {
        if path.is_file() {
            let s = path.to_string_lossy();
            return found(vec![&s], &s);
        }
    }
    if cfg!(target_os = "linux") {
        if let Some(flat) = which(py, "flatpak")? {
            if run(py, vec![flat.clone(), "info".into(), FLAT_ID.into()], 10.)?.is_some() {
                return found(vec![&flat, "run", "--filesystem=home", FLAT_ID], &hosted);
            }
        }
    }
    Ok(None)
}
#[pyfunction(pass_module)]
fn package_kind(m: &Bound<PyModule>) -> PyResult<&'static str> {
    Ok(if m.call_method0("in_snap")?.is_truthy()? {
        "snap"
    } else if m.call_method0("in_flatpak")?.is_truthy()? {
        "flatpak"
    } else if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    })
}
#[pyfunction(pass_module)]
fn flatpak_host_allowed(m: &Bound<PyModule>) -> PyResult<bool> {
    Ok(!m.call_method1("_host", (vec!["true"], 5.))?.is_none())
}
#[pyfunction(pass_module,signature=(kind=None,host_allowed=None))]
fn install_steps(
    m: &Bound<PyModule>,
    kind: Option<String>,
    host_allowed: Option<bool>,
) -> PyResult<Vec<(&'static str, Option<&'static str>)>> {
    let k = match kind.filter(|k| !k.is_empty()) {
        Some(k) => k,
        None => package_kind(m)?.into(),
    };
    Ok(match k.as_str(){
        "windows"=>vec![("Download Blender from blender.org and install it, or run in a terminal:",Some("winget install --id BlenderFoundation.Blender -e"))],
        "macos"=>vec![("Download Blender from blender.org, open the .dmg and drag Blender to Applications.",None)],
        "snap"=>vec![("The Snap package cannot start other programs. To render with Blender, use Built's AppImage or Flatpak.",None)],
        "flatpak"=>{let mut steps=vec![("Install Blender from Flathub (skip it if you already have Blender):",Some("flatpak install flathub org.blender.Blender"))];if !match host_allowed{Some(v)=>v,None=>flatpak_host_allowed(m)?}{steps.push(("Let Built start it (the Flatpak cannot see other programs on its own), once:",Some(PERMISSION)));}steps},
        _=>vec![("Install Blender, for example with Snap (current version):",Some("sudo snap install blender --classic")),("or with Flatpak:",Some("flatpak install flathub org.blender.Blender"))]
    })
}
#[pyfunction(signature=(environ=None))]
fn clean_env<'p>(py: Python<'p>, environ: Option<&Obj<'p>>) -> PyResult<Bound<'p, PyDict>> {
    let env = PyDict::new(py);
    let base = match environ {
        Some(e) => e.clone(),
        None => py.import("os")?.getattr("environ")?,
    };
    env.call_method1("update", (base,))?;
    let frozen = py
        .import("sys")?
        .getattr_opt("frozen")?
        .map_or(Ok(false), |v| v.is_truthy())?;
    for k in ["LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH"] {
        let orig = env.call_method1("pop", (format!("{k}_ORIG"), py.None()))?;
        if !orig.is_none() {
            env.set_item(k, orig)?
        } else if frozen {
            env.call_method1("pop", (k, py.None()))?;
        }
    }
    for k in [
        "PYTHONHOME",
        "PYTHONPATH",
        "QT_PLUGIN_PATH",
        "QT_QPA_PLATFORM_PLUGIN_PATH",
        "QML2_IMPORT_PATH",
        "QT_QPA_PLATFORM",
    ] {
        env.call_method1("pop", (k, py.None()))?;
    }
    Ok(env)
}
pub(super) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_class::<BlenderFound>()?;
    for f in [
        wrap_pyfunction!(in_flatpak, m)?,
        wrap_pyfunction!(in_snap, m)?,
        wrap_pyfunction!(find_blender, m)?,
        wrap_pyfunction!(_host, m)?,
        wrap_pyfunction!(app_bundle_executable, m)?,
        wrap_pyfunction!(package_kind, m)?,
        wrap_pyfunction!(install_steps, m)?,
        wrap_pyfunction!(flatpak_host_allowed, m)?,
        wrap_pyfunction!(clean_env, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
