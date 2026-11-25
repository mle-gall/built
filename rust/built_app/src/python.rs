//! The embedded interpreter and the per-window controller (`ingetrazo_render.Host`).
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
        #[cfg(not(target_os = "macos"))]
        if let Some(bundle) = portable_root() {
            return Layout { root: bundle, venv: None };
        }
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

#[cfg(not(target_os = "macos"))]
fn portable_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let root = exe.parent()?.join("_internal");
    root.join("base_library.zip").is_file().then_some(root)
}

/// Register the renderer module and start the interpreter (once).
pub fn start(layout: &Layout) -> PyResult<()> {
    use ingetrazo_render::ingetrazo_render;
    use crate::native::built_shell;
    pyo3::append_to_inittab!(ingetrazo_render);
    pyo3::append_to_inittab!(built_shell);
    #[cfg(built_bundle)]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        extern "C" { fn built_initialize(root: *const i8, exe: *const i8); }
        let exe = std::env::current_exe().expect("locate Built.app");
        let root = CString::new(layout.root.as_os_str().as_bytes()).unwrap();
        let exe = CString::new(exe.as_os_str().as_bytes()).unwrap();
        // Called once on the main thread before PyO3 attaches.
        unsafe { built_initialize(root.as_ptr(), exe.as_ptr()) };
    }
    #[cfg(not(target_os = "macos"))]
    if let Some(root) = portable_root() {
        let mut paths = vec![root.clone(), root.join("base_library.zip")];
        for entry in root.read_dir()?.flatten() {
            let dynload = entry.path().join("lib-dynload");
            if dynload.is_dir() { paths.push(dynload); }
        }
        std::env::set_var("PYTHONPATH", std::env::join_paths(paths)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?);
        std::env::set_var("PYTHONHOME", root);
        std::env::set_var("PYTHONNOUSERSITE", "1");
    }
    #[cfg(not(built_bundle))]
    Python::initialize();
    Python::attach(|py| {
        let sys = py.import("sys")?;
        #[cfg(built_bundle)]
        sys.setattr("_MEIPASS", &layout.root)?;
        #[cfg(not(target_os = "macos"))]
        if let Some(root) = portable_root() {
            sys.setattr("_MEIPASS", &root)?;
            sys.setattr("frozen", true)?;
            sys.setattr("executable", std::env::current_exe()?)?;
            #[cfg(windows)]
            sys.setattr("_built_dll_directory", py.import("os")?.call_method1("add_dll_directory", (&root,))?)?;
        }
        let path = sys.getattr("path")?;
        path.call_method1("insert", (0, layout.root.to_string_lossy().to_string()))?;
        sys.setattr("argv", vec!["built".to_string()])?;
        // Process the venv (including its .pth files) before loading any
        // application module. A packaged runtime already has its search path.
        if let Some(venv) = &layout.venv {
            let path = py.import("pathlib")?.call_method1("Path", (venv,))?;
            let pattern = if cfg!(windows) { "Lib/site-packages" } else { "lib/python3*/site-packages" };
            let directories = py.import("builtins")?.call_method1("sorted", (path.call_method1("glob", (pattern,))?,))?;
            let site = py.import("site")?;
            for directory in directories.try_iter()? {
                site.call_method1("addsitedir", (directory?.str()?,))?;
            }
            let binaries: &[&str] = if cfg!(windows) { &["Scripts/python.exe"] } else { &["bin/python3", "bin/python"] };
            if let Some(executable) = binaries.iter().map(|p| venv.join(p)).find(|p| p.is_file()) {
                sys.setattr("executable", executable.to_string_lossy().as_ref())?;
            }
        }
        std::env::set_current_dir(&layout.root)?;
        crate::launcher::initialize(py)
    })
}

#[derive(Default, Debug)]
pub struct Extras {
    pub exit: bool,
    pub ime_area: Option<(f64, f64, f64, f64)>,
    pub recent: Option<Vec<String>>,
    /// A window Python asked for (app_shell.windows): its token.
    pub new_window: Option<String>,
    /// This window should come to the front.
    pub focus: bool,
}

/// The `ingetrazo_render.Host` for one window.
pub struct Host {
    obj: Py<PyAny>,
}

pub fn report(py: Python<'_>, err: PyErr, what: &str) {
    eprintln!("[built] python error in {what}:");
    err.print(py);
}

impl Host {
    pub fn new(surface: (usize, usize, &str), width: f64, height: f64, scale: f64, settings: Option<&str>, temporary: bool) -> PyResult<Host> {
        Python::attach(|py| {
            let cls = py.import("ingetrazo_render")?.getattr("Host")?;
            let (window, display, platform) = surface;
            let obj = cls.call1((window, width, height, scale, settings, temporary, display, platform))?;
            Ok(Host { obj: obj.unbind() })
        })
    }

    /// Call a method, printing (never raising) a Python error.
    pub fn call<A>(&self, name: &str, args: A) -> Option<Py<PyAny>>
    where
        A: for<'py> pyo3::call::PyCallArgs<'py>,
    {
        let t0 = std::time::Instant::now();
        let out = Python::attach(|py| match self.obj.bind(py).call_method1(name, args) {
            Ok(v) => Some(v.unbind()),
            Err(e) => {
                report(py, e, name);
                None
            }
        });
        // BUILT_PERF: each wheel (slide, notch) and gesture (pinch, rotate)
        // event's handler time, beside the pointer moves.
        match name {
            "wheel" => crate::perf::note("wheel_ms", t0),
            "gesture" => crate::perf::note("gesture_ms", t0),
            _ => {}
        }
        out
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

    pub fn open_fraction(&self) -> Option<f64> {
        Python::attach(|py| self.obj.bind(py).getattr("window").ok()?
            .getattr("_pending_open").ok()?.getattr("job").ok()?
            .getattr("fraction").ok()?.extract().ok())
    }

    pub fn open_phase(&self) -> Option<String> {
        Python::attach(|py| self.obj.bind(py).getattr("window").ok()?
            .getattr("_pending_open").ok()?.getattr("job").ok()?
            .getattr("phase").ok()?.extract().ok())
    }

    pub fn opening(&self) -> bool {
        Python::attach(|py| self.obj.bind(py).getattr("window")
            .and_then(|win| win.getattr("_open_in_progress"))
            .and_then(|v| v.extract()).unwrap_or(false))
    }

    pub fn aborting_open(&self) -> bool {
        self.opening() && Python::attach(|py| self.obj.bind(py).getattr("window")
            .and_then(|win| win.getattr("_open_aborted"))
            .and_then(|value| value.extract()).unwrap_or(false))
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

    /// Exercise the same store and live renderer binding as the Display row.
    pub fn apply_msaa(&self, count: u32) {
        Python::attach(|py| {
            let result = (|| -> PyResult<()> {
                let win = self.obj.bind(py).getattr("window")?;
                let env = py.import("ingetrazo_render.settings_sheet")?.getattr("Env")?.call1((win,))?;
                let render = py.import("ingetrazo_render")?;
                let levels: Vec<u32> = render.call_method1("msaa_levels", (&env,))?.extract()?;
                if levels.contains(&count) {
                    render.call_method1("apply_msaa", (env, count))?;
                    println!("[built] MSAA {count}");
                }
                Ok(())
            })();
            if let Err(e) = result { report(py, e, "Display anti-aliasing"); }
        });
    }

    /// Save the renderer's current target at its actual physical dimensions.
    pub fn capture(&self, path: &std::path::Path) -> bool {
        Python::attach(|py| -> PyResult<bool> {
            let host = self.obj.bind(py);
            let renderer = host.getattr("renderer")?;
            if renderer.is_none() {
                return Ok(false);
            }
            renderer.call_method0("request_capture")?;
            host.call_method0("frame")?;
            renderer.call_method1("save_capture", (path,))?.extract()
        })
        .unwrap_or_else(|e| {
            Python::attach(|py| report(py, e, "capture"));
            false
        })
    }

    /// Last painted button centers, in the same logical pixels as winit.
    pub fn command_center(&self, command: &str) -> Option<(f64, f64)> {
        Python::attach(|py| {
            let renderer = self.obj.bind(py).getattr("renderer").ok()?;
            let buttons = renderer.call_method0("ui_cluster_buttons").ok()?
                .extract::<Vec<(String, (f64, f64))>>().ok()?;
            buttons.into_iter().find(|(id, _)| id == command).map(|(_, center)| center)
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

    pub fn model_menu_state(&self) -> Option<(String,bool,bool,bool)> {
        Python::attach(|py| -> PyResult<_> {
            let viewport=self.obj.bind(py).getattr("viewport")?;
            let scene=viewport.getattr("scene")?;
            let transitions=scene.getattr("animation")?.getattr("enabled")?.is_truthy()?;
            let similar=scene.getattr("hide_similar_components")?.is_truthy()?;
            let playing = self.obj.bind(py).getattr("window")?.getattr("scene_player")?.call_method0("is_playing")?.is_truthy()?;
            Ok((viewport.getattr("edit_rest_mode")?.extract()?,transitions,similar,playing))
        }).ok()
    }

    #[cfg(target_os = "macos")]
    pub fn extension_updates(&self, previous: Option<(u64,u64)>) -> Option<((u64,u64), Vec<ingetrazo_render::extension_menu::Row>)> {
        Python::attach(|py| -> PyResult<_> {
            let menus=self.obj.bind(py).getattr("window")?.getattr("_extension_menus")?;
            let stamp=menus.call_method0("stamp")?.extract()?;
            if previous==Some(stamp) { return Ok(None); }
            Ok(Some((stamp,menus.call_method0("rows")?.extract()?)))
        }).unwrap_or_else(|e| {Python::attach(|py|report(py,e,"extension menus"));None})
    }

    /// What else the window should do this turn (`Host.extras()`): end the
    /// app, where the IME's candidate window goes, the recent files for
    /// File ▸ Open Recent.
    pub fn extras(&self) -> Extras {
        Python::attach(|py| -> PyResult<Extras> {
            let v = self.obj.bind(py).call_method0("extras")?;
            let exit: bool = v.get_item("exit")?.extract()?;
            let ime_area: Option<(f64, f64, f64, f64)> = v.get_item("ime_area")?.extract()?;
            let recent: Option<Vec<String>> = v.get_item("recent")?.extract()?;
            let new_window: Option<String> = v.get_item("new_window")?.extract()?;
            let focus: bool = v.get_item("focus")?.extract()?;
            Ok(Extras { exit, ime_area, recent, new_window, focus })
        })
        .unwrap_or_else(|e| {
            Python::attach(|py| report(py, e, "extras"));
            Extras::default()
        })
    }

    /// A tool cursor's picture: (width, height, hot x, hot y, RGBA).
    pub fn custom_cursor(&self, key: &str) -> Option<(u16, u16, u16, u16, Vec<u8>)> {
        Python::attach(|py| match self.obj.bind(py).call_method1("custom_cursor", (key,)) {
            Ok(v) => v.extract().ok(),
            Err(e) => {
                report(py, e, "custom_cursor");
                None
            }
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
        let host = Host::new((0, 0, ""), 1280.0, 800.0, 2.0, Some(&settings.to_string_lossy()), true).expect("host");
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

pub fn touch_select() -> bool {
    Python::attach(|py| -> PyResult<bool> {
        let store = py.import("core.settings")?.getattr("settings")?.call0()?;
        Ok(store
            .call_method1("value", ("nav/touch_slide", "orbit"))?
            .extract::<String>()?
            == "select")
    })
    .unwrap_or(false)
}

/// Drain staging and executor threads before Qt static destructors run.
pub fn shutdown_workers() {
    pyo3::Python::attach(|py| {
        let drain = || -> PyResult<()> {
            py.import("ingetrazo_core")?.getattr("DocumentOpen")?.call_method0("shutdown")?;
            py.import("threading")?.call_method0("_shutdown")?;
            Ok(())
        };
        if let Err(e) = drain() { report(py, e, "worker shutdown"); }
    });
}
