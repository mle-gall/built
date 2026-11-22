//! The embedded interpreter and the Python host object (`app_shell.host`).
//!
//! `ingetrazo_render` is linked into this binary and registered as a
//! built-in module before the interpreter starts, so Python's
//! `import ingetrazo_render` gets this binary's renderer, not a wheel from
//! the venv: one `Renderer` type that Rust and Python both hold.
//!
//! Every call takes the GIL for its duration only (`Python::attach`); in
//! between, Python threads (the snap-index worker) run while winit waits.

use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::path::PathBuf;

/// Where the repository (core/, tools/, views/, app_shell/) and its venv are.
#[derive(Clone, Debug)]
pub struct Layout {
    pub root: PathBuf,
    pub venv: Option<PathBuf>,
}

impl Layout {
    #[cfg(built_bundle)]
    pub fn find(_: Option<PathBuf>, _: Option<PathBuf>) -> Layout {
        let exe = std::env::current_exe().expect("locate Built.app");
        let root = exe.parent().unwrap().join("../Frameworks").canonicalize()
            .expect("locate bundled Python");
        Layout { root, venv: None }
    }

    #[cfg(not(built_bundle))]
    /// `--root`, else `BUILT_ROOT`, else the checkout this binary was built
    /// from; the venv is `--venv`, `VIRTUAL_ENV`, or `<root>/.venv`.
    pub fn find(root: Option<PathBuf>, venv: Option<PathBuf>) -> Layout {
        let root = root
            .or_else(|| std::env::var_os("BUILT_ROOT").map(PathBuf::from))
            .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
        let root = root.canonicalize().unwrap_or(root);
        let venv = venv
            .or_else(|| std::env::var_os("VIRTUAL_ENV").map(PathBuf::from))
            .or_else(|| Some(root.join(".venv")).filter(|p| p.is_dir()));
        Layout { root, venv }
    }
}

/// Register the renderer module and start the interpreter (once).
pub fn start(layout: &Layout) -> PyResult<()> {
    use ingetrazo_render::ingetrazo_render;
    pyo3::append_to_inittab!(ingetrazo_render);
    #[cfg(built_bundle)]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        extern "C" { fn built_initialize(root: *const i8, exe: *const i8); }
        let exe = std::env::current_exe().expect("locate Built.app");
        let root = CString::new(layout.root.as_os_str().as_bytes()).unwrap();
        let exe = CString::new(exe.as_os_str().as_bytes()).unwrap();
        // Called once on the main thread before PyO3 attaches.
        unsafe { built_initialize(root.as_ptr(), exe.as_ptr()) };
        std::env::set_var("QT_PLUGIN_PATH", layout.root.join("PySide6/Qt/plugins"));
    }
    #[cfg(not(built_bundle))]
    Python::initialize();
    Python::attach(|py| {
        let sys = py.import("sys")?;
        #[cfg(built_bundle)]
        sys.setattr("_MEIPASS", &layout.root)?;
        let path = sys.getattr("path")?;
        path.call_method1("insert", (0, layout.root.to_string_lossy().to_string()))?;
        sys.setattr("argv", vec!["built".to_string()])?;
        let host = py.import("app_shell.host")?;
        host.call_method1(
            "bootstrap",
            (
                layout.root.to_string_lossy().to_string(),
                layout.venv.as_ref().map(|v| v.to_string_lossy().to_string()),
            ),
        )?;
        Ok(())
    })
}

/// The `app_shell.host.Host` for one window.
pub struct Host {
    obj: Py<PyAny>,
}

pub fn report(py: Python<'_>, err: PyErr, what: &str) {
    eprintln!("[built] python error in {what}:");
    err.print(py);
}

impl Host {
    pub fn new(ns_view: usize, width: f64, height: f64, scale: f64, settings: Option<&str>) -> PyResult<Host> {
        Python::attach(|py| {
            let cls = py.import("app_shell.host")?.getattr("Host")?;
            let obj = cls.call1((ns_view, width, height, scale, settings))?;
            Ok(Host { obj: obj.unbind() })
        })
    }

    /// Call a method, printing (never raising) a Python error.
    pub fn call<A>(&self, name: &str, args: A) -> Option<Py<PyAny>>
    where
        A: for<'py> pyo3::call::PyCallArgs<'py>,
    {
        Python::attach(|py| match self.obj.bind(py).call_method1(name, args) {
            Ok(v) => Some(v.unbind()),
            Err(e) => {
                report(py, e, name);
                None
            }
        })
    }

    pub fn call0(&self, name: &str) -> Option<Py<PyAny>> {
        Python::attach(|py| match self.obj.bind(py).call_method0(name) {
            Ok(v) => Some(v.unbind()),
            Err(e) => {
                report(py, e, name);
                None
            }
        })
    }

    /// `Host.idle()`: `(frame_wanted, wake_ms, cursor, title)`.
    pub fn idle(&self) -> (bool, Option<f64>, Option<String>, Option<String>, bool) {
        Python::attach(|py| {
            let host = self.obj.bind(py);
            let out = host.call_method0("idle").and_then(|v| {
                v.extract::<(bool, Option<f64>, Option<String>, Option<String>)>()
            });
            let drag = host
                .getattr("window_drag_requested")
                .and_then(|v| v.extract::<bool>())
                .unwrap_or(false);
            if drag {
                let _ = host.setattr("window_drag_requested", false);
            }
            match out {
                Ok((a, b, c, d)) => (a, b, c, d, drag),
                Err(e) => {
                    report(py, e, "idle");
                    (false, None, None, None, false)
                }
            }
        })
    }

    /// `Host.frame()` → `(cpu_ms, gpu_ms, wall_ms)`.
    pub fn frame(&self) -> Option<(f64, Option<f64>, f64)> {
        Python::attach(|py| match self.obj.bind(py).call_method0("frame") {
            Ok(v) => v.extract().ok(),
            Err(e) => {
                report(py, e, "frame");
                None
            }
        })
    }

    /// The frame as RGBA bytes: request a capture, draw, take it.
    pub fn capture(&self) -> Option<Vec<u8>> {
        Python::attach(|py| -> PyResult<Option<Vec<u8>>> {
            let host = self.obj.bind(py);
            let renderer = host.getattr("renderer")?;
            if renderer.is_none() {
                return Ok(None);
            }
            renderer.call_method0("request_capture")?;
            host.call_method0("frame")?;
            let data = renderer.call_method0("take_capture")?;
            if data.is_none() {
                return Ok(None);
            }
            Ok(Some(data.extract::<Vec<u8>>()?))
        })
        .unwrap_or_else(|e| {
            Python::attach(|py| report(py, e, "capture"));
            None
        })
    }

    /// `Host.state()` as JSON text.
    pub fn state_json(&self) -> String {
        Python::attach(|py| -> PyResult<String> {
            let state = self.obj.bind(py).call_method0("state")?;
            let json = py.import("json")?;
            let kwargs = PyDict::new(py);
            kwargs.set_item("default", py.import("builtins")?.getattr("str")?)?;
            json.call_method("dumps", (state,), Some(&kwargs))?.extract()
        })
        .unwrap_or_else(|e| {
            Python::attach(|py| report(py, e, "state"));
            "{}".into()
        })
    }

    pub fn get_f64(&self, name: &str) -> Option<f64> {
        Python::attach(|py| self.obj.bind(py).getattr(name).ok()?.extract().ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The embedded interpreter runs the shell's document headless: the
    /// built-in renderer module, the repository's core/tools/formats, an
    /// open, a tool switch by key (ShortcutOverride, then the shortcut).
    #[test]
    fn embedded_python_opens_a_document_headless() {
        let layout = Layout::find(None, None);
        start(&layout).expect("python starts");
        let builtin = Python::attach(|py| -> PyResult<bool> {
            let names = py.import("sys")?.getattr("builtin_module_names")?;
            names.contains("ingetrazo_render")
        });
        assert!(builtin.unwrap(), "ingetrazo_render is registered as a built-in module");
        let settings = std::env::temp_dir().join("built-shell-cargo-test");
        let host = Host::new(0, 1280.0, 800.0, 2.0, Some(&settings.to_string_lossy())).expect("host");
        let fixture = layout.root.join("tests/fixtures/shell/slab-and-cube.igz");
        let opened = host.call("open", (fixture.to_string_lossy().to_string(),)).unwrap();
        assert!(Python::attach(|py| opened.extract::<bool>(py).unwrap()));
        host.call0("attach_surface"); // headless: no renderer, select tool on
        host.call("key", (true, 'L' as u32, 0u32, "l", false));
        let state = host.state_json();
        assert!(state.contains("\"faces\": 6"), "{state}");
        assert!(state.contains("\"tool\": \"LineTool\""), "{state}");
        assert!(state.contains("\"errors\": 0"), "{state}");
    }
}
