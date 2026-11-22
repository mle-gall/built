//! `built`: the Rust app shell (docs/rust-port-plan.md, R1.1).
//!
//! The event loop is winit's, on the main thread. The window's NSView goes
//! to the wgpu renderer (linked in, see `python.rs`); the document, the
//! tools and the egui chrome's state run in the embedded Python
//! (`app_shell/`), which gets every input event already in Qt's terms.
//!
//! ```text
//! built [file.built|file.igz] [--script steps.txt] [--shots dir]
//!       [--settings dir] [--root repo] [--venv dir] [--size 1280x800]
//!       [--new-instance] [--measure] [--check]
//! ```

mod keys;
mod menu;
mod native;
mod python;
mod script;

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{Cursor, CursorIcon, CustomCursor, Window, WindowId};

use script::Step;

#[derive(Default, Debug)]
struct Args {
    file: Option<PathBuf>,
    script: Option<PathBuf>,
    shots: Option<PathBuf>,
    settings: Option<String>,
    root: Option<PathBuf>,
    venv: Option<PathBuf>,
    size: (f64, f64),
    new_instance: bool,
    measure: bool,
    check: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args { size: (1280., 800.), ..Default::default() };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--script" => a.script = Some(value("--script")?.into()),
            "--shots" => a.shots = Some(value("--shots")?.into()),
            "--settings" => a.settings = Some(value("--settings")?),
            "--root" => a.root = Some(value("--root")?.into()),
            "--venv" => a.venv = Some(value("--venv")?.into()),
            "--size" => {
                let v = value("--size")?;
                let (w, h) = v.split_once('x').ok_or("--size WxH")?;
                a.size = (w.parse().map_err(|_| "--size WxH")?, h.parse().map_err(|_| "--size WxH")?);
            }
            "--new-instance" => a.new_instance = true,
            "--measure" => a.measure = true,
            "--check" => a.check = true,
            "-h" | "--help" => {
                println!("built [file] [--script f] [--shots dir] [--settings dir] [--root dir] \
                          [--venv dir] [--size WxH] [--new-instance] [--measure] [--check]");
                std::process::exit(0);
            }
            // Launch Services' process serial number (old macOS, Finder launches).
            s if s.starts_with("-psn_") => {}
            s if s.starts_with('-') => return Err(format!("unknown option {s}")),
            s => a.file = Some(absolute(Path::new(s))),
        }
    }
    a.script = a.script.map(|p| absolute(&p));
    a.shots = a.shots.map(|p| absolute(&p));
    Ok(a)
}

fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().map(|d| d.join(p)).unwrap_or_else(|_| p.to_path_buf())
    }
}

#[derive(Debug)]
enum UserEvent {
    Menu(String),
    Open(PathBuf),
}

// ---- single instance ------------------------------------------------------

fn instance_socket() -> PathBuf {
    let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
    std::env::temp_dir().join(format!("built-shell-{user}.sock"))
}

/// Hand `file` to a running shell; true when one took it.
fn forward_to_running(file: Option<&Path>) -> bool {
    let Ok(mut stream) = UnixStream::connect(instance_socket()) else {
        return false;
    };
    let line = file.map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    stream.write_all(format!("{line}\n").as_bytes()).is_ok()
}

fn listen_for_instances(proxy: EventLoopProxy<UserEvent>) {
    let path = instance_socket();
    let _ = std::fs::remove_file(&path);
    let Ok(listener) = UnixListener::bind(&path) else {
        eprintln!("[built] single instance: cannot listen on {}", path.display());
        return;
    };
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            if BufReader::new(stream).read_line(&mut line).is_ok() {
                let line = line.trim();
                let event = if line.is_empty() {
                    UserEvent::Menu("activate".into())
                } else {
                    UserEvent::Open(PathBuf::from(line))
                };
                if proxy.send_event(event).is_err() {
                    break;
                }
            }
        }
    });
}

// ---- the app ----------------------------------------------------------------

struct Timings {
    process: Instant,
    python_ready: Duration,
    window: Option<Duration>,
    host: Option<Duration>,
    opened: Option<Duration>,
    first_frame: Option<Duration>,
}

struct App {
    args: Args,
    window: Option<Window>,
    host: Option<python::Host>,
    _menus: Option<menu::Menus>,
    modifiers: ModifiersState,
    cursor: (f64, f64),
    held: u32,
    last_press: Option<(Instant, u32, (f64, f64))>,
    last_key_text: Option<String>,
    steps: VecDeque<Step>,
    resume_at: Option<Instant>,
    frames: u64,
    timings: Timings,
    exit_code: i32,
    /// Tool cursors made from the pixmaps Python draws, by key.
    cursors: HashMap<String, CustomCursor>,
    /// Files handed over before the window and its host exist (a cold
    /// launch from Finder: the odoc event comes before `resumed`).
    pending_opens: Vec<PathBuf>,
    /// The recent files the File menu shows, as last sent.
    recent_sig: Option<Vec<String>>,
    /// Files given on the command line: AppKit hands them to the Open
    /// With hook too (as odoc), which must not open them a second time.
    cli_files: Vec<PathBuf>,
}

const DOUBLE_CLICK: Duration = Duration::from_millis(500);

impl App {
    fn scale(&self) -> f64 {
        self.window.as_ref().map_or(2.0, |w| w.scale_factor())
    }

    fn logical(&self, p: PhysicalPosition<f64>) -> (f64, f64) {
        let s = self.scale();
        (p.x / s, p.y / s)
    }

    fn mods(&self) -> u32 {
        keys::qt_modifiers(self.modifiers)
    }

    fn redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    // ---- dispatch: real and scripted input both come through here -------
    fn pointer(&mut self, kind: &str, x: f64, y: f64, button: u32, mods: u32) {
        self.cursor = (x, y);
        let mut kind = kind;
        if kind == "press" {
            self.held |= button;
            let now = Instant::now();
            if let Some((t, b, (px, py))) = self.last_press {
                if b == button && now - t < DOUBLE_CLICK && (px - x).hypot(py - y) < 5.0 {
                    kind = "double";
                }
            }
            self.last_press = if kind == "double" { None } else { Some((now, button, (x, y))) };
        } else if kind == "release" {
            self.held &= !button;
        }
        if let Some(h) = &self.host {
            h.call("pointer", (kind, x, y, button, self.held, mods));
        }
        self.redraw();
    }

    fn key(&mut self, pressed: bool, code: u32, text: &str, repeat: bool, mods: u32) {
        if let Some(h) = &self.host {
            h.call("key", (pressed, code, mods, text, repeat));
        }
        self.redraw();
    }

    fn command(&mut self, event_loop: &ActiveEventLoop, cmd: &str) {
        match cmd {
            // The window's close and Quit ask about unsaved work first
            // (app_shell.files: the sheet, answered through the modal pump).
            "close" | "quit" => {
                let ok = self.host.as_ref().map_or(true, |h| {
                    h.call0("request_close")
                        .map(|v| pyo3::Python::attach(|py| v.extract::<bool>(py).unwrap_or(false)))
                        .unwrap_or(true)
                });
                if ok {
                    event_loop.exit();
                }
            }
            "activate" => {
                if let Some(w) = &self.window {
                    w.focus_window();
                }
            }
            other => {
                if let Some(h) = &self.host {
                    h.call("menu", (other,));
                }
            }
        }
        self.redraw();
    }

    fn open(&mut self, path: &Path) {
        let t = Instant::now();
        let Some(h) = &self.host else {
            self.pending_opens.push(path.to_path_buf());
            return;
        };
        h.call("open_path", (path.to_string_lossy().to_string(),));
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!("[built] opened {} in {ms:.0} ms", path.display());
        self.timings.opened.get_or_insert(self.timings.process.elapsed());
        if let Some(w) = &self.window {
            w.focus_window();
        }
        self.redraw();
    }

    fn capture(&mut self, name: &str) {
        let Some(h) = &self.host else { return };
        let Some(rgba) = h.capture() else {
            eprintln!("[built] capture {name}: no frame");
            return;
        };
        let size = self.window.as_ref().map(|w| w.inner_size()).unwrap_or_default();
        let (mut w, mut hgt) = (size.width, size.height);
        if (w as usize) * (hgt as usize) * 4 != rgba.len() && hgt > 0 {
            w = (rgba.len() / 4 / hgt as usize) as u32;
        }
        if w == 0 || hgt == 0 {
            hgt = 0;
        }
        let dir = self.args.shots.clone().unwrap_or_else(|| PathBuf::from("build/shell-shots"));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("{name}.png"));
        match write_png(&path, w, hgt, &rgba) {
            Ok(()) => println!("[built] capture {}", path.display()),
            Err(e) => eprintln!("[built] capture {name}: {e}"),
        }
    }

    /// One scripted step per turn; `wait` lets the loop run normally.
    fn run_script(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(at) = self.resume_at {
            if Instant::now() < at {
                event_loop.set_control_flow(ControlFlow::WaitUntil(at));
                return;
            }
            self.resume_at = None;
        }
        let Some(step) = self.steps.pop_front() else { return };
        match step {
            Step::Open(p) => self.open(&absolute(Path::new(&p))),
            Step::Wait(ms) => self.resume_at = Some(Instant::now() + Duration::from_millis(ms)),
            Step::Move(x, y) => self.pointer("move", x, y, 0, 0),
            Step::Click(x, y, b, m) => {
                self.pointer("move", x, y, 0, m);
                self.pointer("press", x, y, b, m);
                self.pointer("release", x, y, b, m);
            }
            Step::Double(x, y, b, m) => {
                self.last_press = None;
                for _ in 0..2 {
                    self.pointer("press", x, y, b, m);
                    self.pointer("release", x, y, b, m);
                }
            }
            Step::Drag { from, to, button, steps, mods } => {
                self.pointer("move", from.0, from.1, 0, mods);
                self.pointer("press", from.0, from.1, button, mods);
                for i in 1..=steps {
                    let f = i as f64 / steps as f64;
                    let (x, y) = (from.0 + (to.0 - from.0) * f, from.1 + (to.1 - from.1) * f);
                    self.pointer("move", x, y, 0, mods);
                    if let Some(h) = &self.host {
                        h.frame();
                    }
                }
                self.pointer("release", to.0, to.1, button, mods);
            }
            Step::Key(name, mods) => match keys::qt_key_by_name(&name) {
                Some((code, text)) => {
                    let text = if mods & (keys::CONTROL | keys::META) != 0 { String::new() } else { text };
                    self.key(true, code, &text, false, mods);
                    self.key(false, code, &text, false, mods);
                }
                None => eprintln!("[built] script: unknown key {name:?}"),
            },
            Step::Type(text) => {
                for c in text.chars() {
                    let s = c.to_string();
                    self.key(true, keys::qt_char(c), &s, false, 0);
                    self.key(false, keys::qt_char(c), &s, false, 0);
                }
            }
            Step::Menu(cmd) => self.command(event_loop, &cmd),
            Step::Preedit(text) => {
                if let Some(h) = &self.host {
                    let n = text.chars().count();
                    h.call("preedit", (text, n));
                }
            }
            Step::Commit(text) => {
                if let Some(h) = &self.host {
                    h.call("text", (text,));
                }
            }
            Step::Wheel(x, y, dy, phase) => {
                if let Some(h) = &self.host {
                    h.call("wheel", (x, y, 0.0, dy, 0.0, dy * 2.0, phase.as_str(), 0u32, 0u32, true));
                }
            }
            Step::Notch(x, y, n) => {
                if let Some(h) = &self.host {
                    h.call("wheel", (x, y, 0.0, 0.0, 0.0, n * 120.0, "none", 0u32, 0u32, false));
                }
            }
            Step::Pinch(x, y, v) => {
                if let Some(h) = &self.host {
                    h.call("gesture", ("begin", 0.0, x, y));
                    for _ in 0..5 {
                        h.call("gesture", ("zoom", v / 5.0, x, y));
                    }
                    h.call("gesture", ("end", 0.0, x, y));
                }
            }
            Step::Capture(name) => {
                self.settle();
                self.capture(&name);
            }
            Step::State => {
                if let Some(h) = &self.host {
                    println!("[built] state {}", h.state_json());
                }
            }
            Step::Quit => event_loop.exit(),
        }
        if self.steps.is_empty() && self.resume_at.is_none() {
            if let Some(h) = &self.host {
                println!("[built] state {}", h.state_json());
            }
            event_loop.exit();
        }
        self.redraw();
    }

    /// Let deferred work land before a capture: Qt timers, the snap index.
    fn settle(&mut self) {
        let Some(h) = &self.host else { return };
        for _ in 0..4 {
            let _ = h.idle();
            h.frame();
        }
    }

    fn start(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let (w, h) = self.args.size;
        #[allow(unused_mut)]
        let mut attrs = Window::default_attributes()
            .with_title("Built (Rust shell)")
            .with_inner_size(LogicalSize::new(w, h))
            .with_min_inner_size(LogicalSize::new(640.0, 480.0));
        #[cfg(target_os = "macos")]
        {
            use winit::platform::macos::WindowAttributesExtMacOS;
            // Full bleed, as the Qt app's egui chrome expects: the model
            // reaches the top edge, the traffic lights sit on the top bar.
            attrs = attrs
                .with_titlebar_transparent(true)
                .with_fullsize_content_view(true)
                .with_title_hidden(true);
        }
        let window = event_loop.create_window(attrs).map_err(|e| e.to_string())?;
        window.set_ime_allowed(true);
        self.timings.window = Some(self.timings.process.elapsed());
        let ns_view = match window.window_handle().map_err(|e| e.to_string())?.as_raw() {
            RawWindowHandle::AppKit(h) => h.ns_view.as_ptr() as usize,
            _ => return Err("not an AppKit window".into()),
        };
        let size = window.inner_size();
        let scale = window.scale_factor();
        let host = python::Host::new(
            ns_view,
            size.width as f64 / scale,
            size.height as f64 / scale,
            scale,
            self.args.settings.as_deref(),
        )
        .map_err(|e| {
            pyo3::Python::attach(|py| python::report(py, e, "Host"));
            "the Python host did not start".to_string()
        })?;
        host.call0("attach_surface").ok_or("the renderer did not start")?;
        self.timings.host = Some(self.timings.process.elapsed());
        self.window = Some(window);
        self.host = Some(host);
        match menu::build() {
            Ok(m) => self._menus = Some(m),
            Err(e) => eprintln!("[built] menu bar: {e}"),
        }
        let mut files: Vec<PathBuf> = self.args.file.clone().into_iter().collect();
        files.append(&mut self.pending_opens);
        if files.is_empty() {
            // A cold start without a file: the start page (recents, new,
            // templates), and the untitled drawings an earlier session
            // left in its auto-save slots.
            if let Some(h) = &self.host {
                h.call0("cold_start");
            }
        }
        for file in files {
            self.open(&file);
        }
        self.redraw();
        Ok(())
    }

    fn report_measure(&self) {
        let t = &self.timings;
        let ms = |d: Option<Duration>| d.map_or(-1.0, |d| d.as_secs_f64() * 1000.0);
        let open_ms = self.host.as_ref().map(|h| h.state_json()).unwrap_or_default();
        println!(
            "[built] measure {{\"python_ready_ms\": {:.0}, \"window_ms\": {:.0}, \"renderer_ms\": {:.0}, \
             \"opened_ms\": {:.0}, \"first_frame_ms\": {:.0}, \"state\": {open_ms}}}",
            t.python_ready.as_secs_f64() * 1000.0,
            ms(t.window),
            ms(t.host),
            ms(t.opened),
            ms(t.first_frame)
        );
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    if w == 0 || h == 0 {
        return Err("empty frame".into());
    }
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&rgba[..(w * h * 4) as usize]).map_err(|e| e.to_string())
}

fn cursor_icon(name: &str) -> CursorIcon {
    match name {
        "crosshair" => CursorIcon::Crosshair,
        "wait" => CursorIcon::Wait,
        "text" => CursorIcon::Text,
        "ns-resize" => CursorIcon::NsResize,
        "ew-resize" => CursorIcon::EwResize,
        "nesw-resize" => CursorIcon::NeswResize,
        "nwse-resize" => CursorIcon::NwseResize,
        "move" => CursorIcon::Move,
        "row-resize" => CursorIcon::RowResize,
        "col-resize" => CursorIcon::ColResize,
        "pointer" => CursorIcon::Pointer,
        "not-allowed" => CursorIcon::NotAllowed,
        "help" => CursorIcon::Help,
        "progress" => CursorIcon::Progress,
        "grab" => CursorIcon::Grab,
        "grabbing" => CursorIcon::Grabbing,
        "copy" => CursorIcon::Copy,
        "alias" => CursorIcon::Alias,
        _ => CursorIcon::Default,
    }
}

fn mouse_bit(b: MouseButton) -> u32 {
    match b {
        MouseButton::Left => 1,
        MouseButton::Right => 2,
        MouseButton::Middle => 4,
        MouseButton::Back => 8,
        MouseButton::Forward => 16,
        MouseButton::Other(_) => 0,
    }
}

fn phase_name(p: TouchPhase) -> &'static str {
    match p {
        TouchPhase::Started => "start",
        TouchPhase::Moved => "move",
        TouchPhase::Ended | TouchPhase::Cancelled => "end",
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        if let Err(e) = self.start(event_loop) {
            eprintln!("[built] {e}");
            self.exit_code = 1;
            event_loop.exit();
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Menu(cmd) => self.command(event_loop, &cmd),
            UserEvent::Open(path) => {
                if let Some(i) = self.cli_files.iter().position(|p| same_path(p, &path)) {
                    self.cli_files.remove(i);
                    return;
                }
                self.open(&path)
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.command(event_loop, "close"),
            WindowEvent::Resized(size) => {
                let s = self.scale();
                if let Some(h) = &self.host {
                    h.call("resize", (size.width as f64 / s, size.height as f64 / s, s));
                }
                self.redraw();
            }
            WindowEvent::RedrawRequested => {
                if let Some(h) = &self.host {
                    if h.frame().is_some() {
                        self.frames += 1;
                        if self.timings.first_frame.is_none()
                            && (self.args.file.is_none() || self.timings.opened.is_some())
                        {
                            self.timings.first_frame = Some(self.timings.process.elapsed());
                            if self.args.measure {
                                self.report_measure();
                                event_loop.exit();
                            }
                        }
                    }
                }
            }
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::CursorMoved { position, .. } => {
                let (x, y) = self.logical(position);
                let m = self.mods();
                self.pointer("move", x, y, 0, m);
            }
            WindowEvent::CursorLeft { .. } => {
                if let Some(h) = &self.host {
                    h.call("pointer", ("leave", -1.0, -1.0, 0u32, 0u32, 0u32));
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let bit = mouse_bit(button);
                let (x, y) = self.cursor;
                let m = self.mods();
                let kind = if state == ElementState::Pressed { "press" } else { "release" };
                self.pointer(kind, x, y, bit, m);
            }
            WindowEvent::MouseWheel { delta, phase, .. } => {
                let (x, y) = self.cursor;
                let m = self.mods();
                let held = self.held;
                if let Some(h) = &self.host {
                    match delta {
                        MouseScrollDelta::LineDelta(dx, dy) => {
                            h.call("wheel", (x, y, 0.0, 0.0, dx as f64 * 120.0, dy as f64 * 120.0,
                                             "none", m, held, false));
                        }
                        MouseScrollDelta::PixelDelta(p) => {
                            let s = self.scale();
                            let (dx, dy) = (p.x / s, p.y / s);
                            // Qt's Cocoa angleDelta for precise deltas: 2× pixels.
                            h.call("wheel", (x, y, dx, dy, dx * 2.0, dy * 2.0, phase_name(phase),
                                             m, held, true));
                        }
                    }
                }
                self.redraw();
            }
            WindowEvent::PinchGesture { delta, phase, .. } => {
                let (x, y) = self.cursor;
                if let Some(h) = &self.host {
                    if phase == TouchPhase::Started {
                        h.call("gesture", ("begin", 0.0, x, y));
                    }
                    if delta.is_finite() && delta != 0.0 {
                        h.call("gesture", ("zoom", delta, x, y));
                    }
                    if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                        h.call("gesture", ("end", 0.0, x, y));
                    }
                }
                self.redraw();
            }
            WindowEvent::RotationGesture { delta, .. } => {
                let (x, y) = self.cursor;
                if let Some(h) = &self.host {
                    h.call("gesture", ("rotate", delta as f64, x, y));
                }
            }
            WindowEvent::DoubleTapGesture { .. } => {
                let (x, y) = self.cursor;
                if let Some(h) = &self.host {
                    h.call("gesture", ("smart_zoom", 0.0, x, y));
                }
                self.redraw();
            }
            WindowEvent::KeyboardInput { event, is_synthetic, .. } => {
                if is_synthetic {
                    return;
                }
                let m = self.mods();
                let (code, text) = keys::qt_key(&event.logical_key, event.text.as_deref(), m);
                let pressed = event.state == ElementState::Pressed;
                if pressed {
                    self.last_key_text = Some(text.clone()).filter(|t| !t.is_empty());
                }
                if code != 0 || !text.is_empty() {
                    self.key(pressed, code, &text, event.repeat, m);
                }
            }
            WindowEvent::Ime(Ime::Preedit(text, cursor)) => {
                // The composition being typed (dead keys, input methods):
                // shown in the chrome's focused field, or as the value
                // being typed, until it commits.
                if let Some(h) = &self.host {
                    let at = cursor.map_or(text.chars().count(), |(a, _)| text[..a.min(text.len())].chars().count());
                    h.call("preedit", (text, at));
                }
                self.redraw();
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                // A plain key already delivered its own text; a composed
                // one (dead keys, input methods) arrives only here.
                if self.last_key_text.take().as_deref() != Some(text.as_str()) {
                    if let Some(h) = &self.host {
                        h.call("text", (text,));
                    }
                    self.redraw();
                }
            }
            WindowEvent::Focused(on) => {
                if let Some(h) = &self.host {
                    h.call("focus", (on,));
                }
            }
            WindowEvent::DroppedFile(path) => self.open(&path),
            WindowEvent::HoveredFile(path) => {
                if let Some(h) = &self.host {
                    h.call("drag_hover", (path.to_string_lossy().to_string(),));
                }
            }
            WindowEvent::HoveredFileCancelled => {
                if let Some(h) = &self.host {
                    h.call("drag_hover", ("",));
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if !self.steps.is_empty() || self.resume_at.is_some() {
            self.run_script(event_loop);
        }
        let Some(h) = &self.host else { return };
        let (wanted, wake_ms, cursor, title, drag) = h.idle();
        let extra = h.extras();
        if extra.exit {
            event_loop.exit();
            return;
        }
        if let Some(c) = cursor {
            if let Some(key) = c.strip_prefix("custom:") {
                if !self.cursors.contains_key(key) {
                    if let Some((w, hgt, hx, hy, rgba)) = h.custom_cursor(key) {
                        match CustomCursor::from_rgba(rgba, w, hgt, hx, hy) {
                            Ok(src) => {
                                let made = event_loop.create_custom_cursor(src);
                                self.cursors.insert(key.to_owned(), made);
                            }
                            Err(e) => eprintln!("[built] cursor {key}: {e}"),
                        }
                    }
                }
                if let (Some(w), Some(cur)) = (&self.window, self.cursors.get(key)) {
                    w.set_cursor(Cursor::Custom(cur.clone()));
                }
            } else if let Some(w) = &self.window {
                w.set_cursor(cursor_icon(&c));
            }
        }
        if let Some(recent) = extra.recent {
            if self.recent_sig.as_ref() != Some(&recent) {
                if let Some(m) = &self._menus {
                    m.set_recent(&recent);
                }
                self.recent_sig = Some(recent);
            }
        }
        if let Some(w) = &self.window {
            if let Some((x, y, cw, ch)) = extra.ime_area {
                w.set_ime_cursor_area(winit::dpi::LogicalPosition::new(x, y), LogicalSize::new(cw, ch));
            }
            if wanted {
                w.request_redraw();
            }
            if let Some(t) = title {
                w.set_title(&t);
            }
            if drag {
                let _ = w.drag_window();
            }
        }
        let scripted = !self.steps.is_empty() && self.resume_at.is_none();
        let flow = if scripted {
            ControlFlow::Poll
        } else {
            match (wake_ms, self.resume_at) {
                (_, Some(at)) => ControlFlow::WaitUntil(at),
                (Some(ms), None) => ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(ms.max(1.0) as u64)),
                (None, None) => ControlFlow::Wait,
            }
        };
        event_loop.set_control_flow(flow);
    }
}

/// `--check`: start Python and the shell's document headless, open the
/// file if one is given, print its state, exit 0 when all of it worked.
fn check(args: &Args) -> i32 {
    let host = match python::Host::new(0, args.size.0, args.size.1, 2.0, args.settings.as_deref()) {
        Ok(h) => h,
        Err(e) => {
            pyo3::Python::attach(|py| python::report(py, e, "Host"));
            return 1;
        }
    };
    if let Some(file) = &args.file {
        let ok = host
            .call("open", (file.to_string_lossy().to_string(),))
            .map(|v| pyo3::Python::attach(|py| v.extract::<bool>(py).unwrap_or(false)))
            .unwrap_or(false);
        if !ok {
            println!("[built] check: open failed");
            return 1;
        }
    }
    println!("[built] check ok {}", host.state_json());
    0
}

fn main() {
    let process = Instant::now();
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("built: {e}");
            std::process::exit(2);
        }
    };
    let interactive = !args.check && args.script.is_none() && !args.measure;
    if interactive && !args.new_instance && forward_to_running(args.file.as_deref()) {
        eprintln!("[built] handed over to the running shell");
        return;
    }
    let steps = match &args.script {
        Some(p) => match std::fs::read_to_string(p).map_err(|e| e.to_string()).and_then(|t| script::parse(&t)) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("built: script {}: {e}", p.display());
                std::process::exit(2);
            }
        },
        None => Vec::new(),
    };
    let layout = python::Layout::find(args.root.clone(), args.venv.clone());
    if let Err(e) = python::start(&layout) {
        pyo3::Python::attach(|py| python::report(py, e, "startup"));
        std::process::exit(1);
    }
    let python_ready = process.elapsed();
    if args.check {
        std::process::exit(check(&args));
    }
    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("built: {e}");
            std::process::exit(1);
        }
    };
    let proxy = event_loop.create_proxy();
    {
        let proxy = proxy.clone();
        muda::MenuEvent::set_event_handler(Some(move |e: muda::MenuEvent| {
            let _ = proxy.send_event(UserEvent::Menu(e.id().0.clone()));
        }));
    }
    {
        // Finder's Open With / double-click / Dock drop, and Quit from the
        // Dock: AppKit asks winit's delegate, which hands them on here.
        let proxy = std::sync::Mutex::new(proxy.clone());
        native::install_app_hooks(move |req| {
            let Ok(p) = proxy.lock() else { return };
            match req {
                native::AppRequest::Open(paths) => {
                    for path in paths {
                        let _ = p.send_event(UserEvent::Open(PathBuf::from(path)));
                    }
                }
                native::AppRequest::Quit => {
                    let _ = p.send_event(UserEvent::Menu("quit".into()));
                }
            }
        });
    }
    if interactive {
        listen_for_instances(proxy);
    }
    let mut app = App {
        args,
        window: None,
        host: None,
        _menus: None,
        modifiers: ModifiersState::empty(),
        cursor: (0.0, 0.0),
        held: 0,
        last_press: None,
        last_key_text: None,
        steps: steps.into(),
        resume_at: None,
        frames: 0,
        timings: Timings { process, python_ready, window: None, host: None, opened: None, first_frame: None },
        exit_code: 0,
        cursors: HashMap::new(),
        pending_opens: Vec::new(),
        recent_sig: None,
        cli_files: Vec::new(),
    };
    app.cli_files = app.args.file.clone().into_iter().collect();
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("built: {e}");
        app.exit_code = 1;
    }
    if interactive {
        let _ = std::fs::remove_file(instance_socket());
    }
    // Drop the renderer (and its CAMetalLayer) before the window goes.
    if let Some(h) = app.host.take() {
        h.call0("close");
    }
    std::process::exit(app.exit_code);
}
