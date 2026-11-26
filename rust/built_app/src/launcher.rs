use crate::{absolute, python};
use pyo3::prelude::*;
use std::path::{Path, PathBuf};

struct PdfExport {
    input: PathBuf,
    output: PathBuf,
}

fn parse_pdf(args: &[String]) -> Result<Option<PdfExport>, String> {
    let flags = [
        "-i",
        "--input-file",
        "-o",
        "--output-file",
        "-p",
        "--create-pdf",
    ];
    if !args
        .iter()
        .any(|arg| flags.contains(&arg.split('=').next().unwrap()))
    {
        return Ok(None);
    }
    let mut input = None;
    let mut output = None;
    let mut pdf = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let (flag, value) = arg
            .split_once('=')
            .map_or((arg.as_str(), None), |(k, v)| (k, Some(v)));
        match flag {
            "-p" | "--create-pdf" if value.is_none() => pdf = true,
            "-i" | "--input-file" | "-o" | "--output-file" => {
                let path = value
                    .or_else(|| args.next().map(String::as_str))
                    .filter(|p| !p.is_empty() && !p.starts_with('-'))
                    .ok_or_else(|| format!("{flag} needs a file"))?;
                let dest = if flag == "-i" || flag == "--input-file" {
                    &mut input
                } else {
                    &mut output
                };
                *dest = Some(absolute(Path::new(path)));
            }
            _ => return Err(format!("unexpected export option {arg}")),
        }
    }
    let input = input.ok_or("--input-file is required")?;
    if !pdf {
        return Err("--create-pdf is required".into());
    }
    Ok(Some(PdfExport {
        output: output.unwrap_or_else(|| input.with_extension("pdf")),
        input,
    }))
}

pub fn surface_handles(
    window: &winit::window::Window,
) -> Result<(usize, usize, &'static str), String> {
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
    let window_handle = window.window_handle().map_err(|e| e.to_string())?.as_raw();
    let display_handle = window.display_handle().map_err(|e| e.to_string())?.as_raw();
    Ok(match (window_handle, display_handle) {
        #[cfg(target_os = "macos")]
        (RawWindowHandle::AppKit(w), RawDisplayHandle::AppKit(_)) => {
            (w.ns_view.as_ptr() as usize, 0, "cocoa")
        }
        (RawWindowHandle::Win32(w), RawDisplayHandle::Windows(_)) => {
            (w.hwnd.get() as usize, 0, "windows")
        }
        (RawWindowHandle::Wayland(w), RawDisplayHandle::Wayland(d)) => (
            w.surface.as_ptr() as usize,
            d.display.as_ptr() as usize,
            "wayland",
        ),
        (RawWindowHandle::Xlib(w), RawDisplayHandle::Xlib(d)) => (
            w.window as usize,
            d.display.ok_or("missing X11 display")?.as_ptr() as usize,
            "xlib",
        ),
        (RawWindowHandle::Xcb(w), RawDisplayHandle::Xcb(d)) => (
            w.window.get() as usize,
            d.connection.ok_or("missing XCB connection")?.as_ptr() as usize,
            "xcb",
        ),
        _ => return Err("no supported GPU surface for this window".into()),
    })
}

enum Batch<'a> { Probe, Pdf(&'a PdfExport), Design(&'a str, &'a str) }

struct BatchWindow<'a> {
    job: Batch<'a>,
    status: i32,
}

impl winit::application::ApplicationHandler for BatchWindow<'_> {
    fn resumed(&mut self, events: &winit::event_loop::ActiveEventLoop) {
        let result = (|| {
            let attrs = winit::window::Window::default_attributes()
                .with_visible(false)
                .with_active(false)
                .with_inner_size(winit::dpi::LogicalSize::new(1280., 800.));
            let window = events.create_window(attrs).map_err(|e| e.to_string())?;
            let surface = surface_handles(&window)?;
            if let Batch::Design(prefix, directory) = self.job {
                return Python::attach(|py| -> PyResult<i32> {
                    let renderer = py.import("built_render")?.getattr("Renderer")?;
                    let kwargs = pyo3::types::PyDict::new(py);
                    kwargs.set_item("display", surface.1)?;
                    kwargs.set_item("platform", surface.2)?;
                    let renderer = renderer.call((surface.0, 1280, 800, 1.0), Some(&kwargs))?;
                    py.import("scripts.design_system_capture")?.call_method1("capture_surface", (renderer, prefix, directory))?;
                    Ok(0)
                }).map_err(|err| { Python::attach(|py| err.print(py)); "design capture failed".to_string() });
            }
            let host = python::Host::new(surface, 1280., 800., window.scale_factor(), None, true)
                .map_err(|e| e.to_string())?;
            if let Batch::Pdf(job) = self.job {
                write_pdf(job, &host).map_err(|e| e.to_string())
            } else {
                let renderer = host.call0("attach_surface");
                if Python::attach(|py| renderer.is_none_or(|r| r.bind(py).is_none())) {
                    Err("no supported GPU backend".to_string())
                } else {
                    Ok(0)
                }
            }
        })();
        self.status = result.unwrap_or_else(|e| {
            eprintln!("built: {e}");
            1
        });
        events.exit();
    }

    fn window_event(
        &mut self,
        _: &winit::event_loop::ActiveEventLoop,
        _: winit::window::WindowId,
        _: winit::event::WindowEvent,
    ) {
    }
}

fn run_batch(job: Batch<'_>) -> Result<i32, String> {
    let layout = python::Layout::find(None, None);
    python::start(&layout).map_err(|e| e.to_string())?;
    let mut builder = winit::event_loop::EventLoop::builder();
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        builder
            .with_activation_policy(ActivationPolicy::Accessory)
            .with_activate_ignoring_other_apps(false);
    }
    let events = builder.build().map_err(|e| e.to_string())?;
    let mut app = BatchWindow { job, status: 1 };
    events.run_app(&mut app).map_err(|e| e.to_string())?;
    python::shutdown_workers();
    Ok(app.status)
}

fn write_pdf(job: &PdfExport, host: &python::Host) -> PyResult<i32> {
    Python::attach(|py| {
        let window = host
            .call("__getattribute__", ("window",))
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("missing document window"))?;
        let window = window.bind(py);
        let scene = window.getattr("viewport")?.getattr("scene")?;
        // Batch export reads the supplied archive directly, without prompting
        // for recovery or deleting a newer autosave of the same document.
        py.import("built_core.formats.igz")?
            .call_method1("load_into", (&scene, &job.input))?;
        window.setattr("_current_path", &job.input)?;
        let sheets = scene.getattr("compositions")?;
        if sheets.len()? == 0 {
            eprintln!("built: {} has no sheets", job.input.display());
            return Ok(1);
        }
        if let Some(dir) = job.output.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let renderer = host.call0("attach_surface");
        if renderer.is_none_or(|r| r.bind(py).is_none()) {
            eprintln!("built: no supported GPU backend");
            return Ok(1);
        }
        let composer = py
            .import("built_render.sheet_workspace")?
            .call_method1("composer_for", (window,))?;
        let troubled = composer.call_method1("export_all_pdf", (&job.output, sheets))?;
        if !job.output.is_file() {
            return Ok(1);
        }
        println!("Exported sheets to {}", job.output.display());
        Ok(if troubled.is_truthy()? { 3 } else { 0 })
    })
}

// Apply before startup allocates worker threads and their glibc arenas.
fn limit_allocator_arenas() -> bool {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        extern "C" {
            fn mallopt(param: i32, value: i32) -> i32;
        }
        const M_ARENA_MAX: i32 = -8;
        mallopt(M_ARENA_MAX, 2) != 0
    }
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    { false }
}

#[test]
fn arena_limit_applies_only_to_glibc() {
    assert_eq!(limit_allocator_arenas(), cfg!(all(target_os = "linux", target_env = "gnu")));
}

pub fn register_launcher() {
    limit_allocator_arenas();
    let args: Vec<_> = std::env::args().skip(1).collect();
    #[cfg(windows)]
    if args.iter().any(|a| a.starts_with('-')) {
        // A GUI executable launched from Explorer has no console. A CLI
        // client may already have supplied pipes, which must stay intact.
        use windows::Win32::System::Console::{
            AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        };
        unsafe {
            let missing = |kind| GetStdHandle(kind).map_or(true, |h| h.is_invalid());
            if missing(STD_INPUT_HANDLE) && missing(STD_OUTPUT_HANDLE) {
                let _ = AttachConsole(ATTACH_PARENT_PROCESS);
            }
        }
    }
    if let [flag, prefix, dest] = args.as_slice() {
        if flag == "--capture-design-system" && matches!(prefix.as_str(), "before" | "after") {
            std::process::exit(run_batch(Batch::Design(prefix, dest)).unwrap_or_else(|err| {
                eprintln!("built: {err}"); 1
            }));
        }
    }
    if args == ["--probe-gpu"] {
        let available = matches!(run_batch(Batch::Probe), Ok(0));
        println!("{{\"available\":{available}}}");
        std::process::exit(0);
    }
    if args == ["--mcp"] {
        let layout = python::Layout::find(None, None);
        let result = python::start(&layout).and_then(|_| {
            Python::attach(|py| {
                let options = pyo3::types::PyDict::new(py);
                options.set_item("run_name", "__main__")?;
                py.import("runpy")?.call_method(
                    "run_path",
                    (layout.root.join("scripts/built_mcp.py"),),
                    Some(&options),
                )?;
                Ok(())
            })
        });
        if let Err(error) = result {
            eprintln!("built: MCP server failed: {error}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }
    match parse_pdf(&args) {
        Ok(None) => (),
        Ok(Some(job)) if !job.input.is_file() => {
            eprintln!("built: no such file: {}", job.input.display());
            std::process::exit(2);
        }
        Ok(Some(job)) => std::process::exit(run_batch(Batch::Pdf(&job)).unwrap_or_else(|error| {
            eprintln!("built: PDF export failed: {error}");
            1
        })),
        Err(error) => {
            eprintln!("built: {error}");
            std::process::exit(2);
        }
    }
}

pub fn initialize(py: Python<'_>) -> PyResult<()> {
    #[cfg(target_os = "macos")]
    py.import("built_core")?.call_method0("NativeSettings")?;
    py.import("built_core.python_tls")?.call_method0("ensure_once")?;
    let core = py.import("built_core")?;
    core.call_method0("install_main_thread_collector")?;
    if std::env::var_os("BUILT_SCRIPTED").is_some()
        && std::env::var_os("BUILT_LOG_DIR").is_none()
        && std::env::var_os("BUILT_SETTINGS_DIR").is_none()
    {
        let folder = py.import("tempfile")?.call_method0("mkdtemp")?;
        py.import("os")?.getattr("environ")?.set_item("BUILT_LOG_DIR", folder)?;
    }
    let log = py.import("built_core.paths")?.call_method0("open_crash_log")?;
    if !log.is_none() {
        let faults = py.import("faulthandler")?;
        faults.call_method1("enable", (&log,))?;
        let path = log.getattr("name")?;
        for module in [core, py.import("built_render")?] {
            module.call_method1("install_panic_log", (&path,))?;
        }
        // Keep the descriptor valid as long as faulthandler may write to it.
        static CRASH_LOG: std::sync::OnceLock<Py<PyAny>> = std::sync::OnceLock::new();
        let _ = CRASH_LOG.set(log.clone().unbind());
        #[cfg(unix)]
        faults.call_method1("register", (py.import("signal")?.getattr("SIGUSR1")?, &log))?;
    }
    #[cfg(windows)]
    if std::env::var_os("BUILT_SETTINGS_DIR").is_none() {
        py.import("built_core")?
            .call_method0("ensure_high_performance_gpu")?;
    }
    Ok(())
}

pub fn self_check(layout: &python::Layout) -> i32 {
    let result = Python::attach(|py| -> PyResult<()> {
        for rel in [
            "i18n/es.json",
            "resources/textures/library.json",
            "resources/components/components.json",
            "resources/icons/built_256.png",
            "scripts/built_mcp.py",
        ] {
            if !layout.root.join(rel).is_file() {
                return Err(pyo3::exceptions::PyFileNotFoundError::new_err(rel));
            }
        }
        let reference: String = py
            .import("core.ai_recipes")?
            .getattr("reference")?
            .call0()?
            .extract()?;
        if !reference.contains("revolve(") || !reference.contains("house(") {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "incomplete AI recipe book",
            ));
        }
        for module in ["built_core", "mapbox_earcut", "manifold3d"] {
            py.import(module)?;
        }
        let rfa = py
            .import("importlib.util")?
            .call_method1("find_spec", ("built_rfa",))?;
        if rfa.is_none() {
            println!(".rfa import: unavailable (reader not installed)");
        } else {
            py.import("built_rfa")?;
            println!(".rfa import: available");
        }
        let converter = py
            .import("built_core.formats.dwg_bridge")?
            .call_method0("find_dwg2dxf")?;
        if converter.is_none() {
            println!("DWG converter: MISSING");
            let frozen = py
                .import("built_core.paths")?
                .call_method0("is_frozen")?
                .is_truthy()?;
            if frozen && !cfg!(target_os = "macos") {
                return Err(pyo3::exceptions::PyFileNotFoundError::new_err("dwg2dxf"));
            }
        } else {
            println!("DWG converter: {}", converter.str()?);
        }
        Ok(())
    });
    if let Err(error) = result {
        eprintln!("built: {error}");
        return 1;
    }
    0
}

pub fn check_extensions(host: &python::Host) -> bool {
    let Some(window) = host.call("__getattribute__", ("window",)) else {
        return false;
    };
    Python::attach(|py| {
        let result = window
            .bind(py)
            .getattr("_extension_load_errors")
            .and_then(|errors| Ok((errors.is_truthy()?, errors.str()?.to_string())));
        match result {
            Ok((false, _)) => {
                let running = window.bind(py).getattr("_running_extensions")
                    .and_then(|plugins| plugins.extract::<std::collections::BTreeSet<String>>());
                match running {
                    Ok(names) => {
                        println!("Loaded extensions: {}", names.into_iter().collect::<Vec<_>>().join(", "));
                        true
                    }
                    Err(error) => {
                        eprintln!("built: reading loaded extensions: {error}");
                        false
                    }
                }
            },
            Ok((true, errors)) => {
                eprintln!("built: extension loading failed: {errors}");
                false
            }
            Err(error) => {
                eprintln!("built: checking extensions: {error}");
                false
            }
        }
    })
}

pub fn exit_now(status: i32) -> ! {
    Python::attach(|py| {
        let flush = || -> PyResult<()> {
            py.import("built_core.settings")?
                .call_method0("settings")?
                .call_method0("sync")?;
            py.import("logging")?.call_method0("shutdown")?;
            let sys = py.import("sys")?;
            for name in ["stdout", "stderr"] {
                let stream = sys.getattr(name)?;
                if !stream.is_none() {
                    stream.call_method0("flush")?;
                }
            }
            Ok(())
        };
        if let Err(error) = flush() {
            eprintln!("built: flushing settings and logs: {error}");
        }
    });
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
        let _ = TerminateProcess(GetCurrentProcess(), status as u32);
    }
    std::process::exit(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Option<PdfExport>, String> {
        parse_pdf(&args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn ordinary_launches_pass_through() {
        for args in [&[][..], &["Casa.igz"], &["--new-instance"]] {
            assert!(parse(args).unwrap().is_none());
        }
    }

    #[test]
    fn output_defaults_to_the_input_stem() {
        let job = parse(&["-i", "Casa.igz", "-p"]).unwrap().unwrap();
        assert_eq!(job.input, absolute(Path::new("Casa.igz")));
        assert_eq!(job.output, absolute(Path::new("Casa.pdf")));
        let job = parse(&[
            "--input-file=Casa.igz",
            "--create-pdf",
            "--output-file",
            "out/plan.pdf",
        ])
        .unwrap()
        .unwrap();
        assert_eq!(job.output, absolute(Path::new("out/plan.pdf")));
    }

    #[test]
    fn missing_input_or_task_is_an_error() {
        for args in [&["-p"][..], &["-i", "Casa.igz"]] {
            assert!(parse(args).is_err());
        }
    }
}
