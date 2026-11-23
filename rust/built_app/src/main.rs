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
#[cfg(target_os = "macos")]
mod menu;
mod native;
mod python;
mod script;

use std::collections::{HashMap, VecDeque};
#[cfg(unix)]
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
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
    gallery: bool,
}

impl Args {
    fn temporary_profile(&self) -> bool {
        self.check || self.measure || self.gallery || self.script.is_some()
    }
}

fn parse_args(mut it: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut a = Args { size: (1280., 800.), ..Default::default() };
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
            "--gallery" => { a.gallery = true; a.new_instance = true; },
            "--new-instance" => a.new_instance = true,
            "--measure" => a.measure = true,
            "--check" => a.check = true,
            "-h" | "--help" => {
                println!("built [file] [--script f] [--shots dir] [--settings dir] [--root dir] \
                          [--venv dir] [--size WxH] [--new-instance] [--gallery] [--measure] [--check]");
                std::process::exit(0);
            }
            // Launch Services' process serial number (old macOS, Finder launches).
            #[cfg(target_os = "macos")]
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

#[cfg(unix)]
fn instance_socket() -> PathBuf {
    let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
    std::env::temp_dir().join(format!("built-shell-{user}.sock"))
}

/// Hand `file` to a running shell; true when one took it.
#[cfg(unix)]
fn forward_to_running(file: Option<&Path>) -> bool {
    let Ok(mut stream) = UnixStream::connect(instance_socket()) else {
        return false;
    };
    let line = file.map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    stream.write_all(format!("{line}\n").as_bytes()).is_ok()
}

#[cfg(unix)]
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

/// One window: its winit window, its Python host (app_shell.host.Host:
/// its projects, its renderer) and its own pointer state. Every window
/// lives in this one process (app_shell/windows.py): ⇧⌘N and the tab
/// menu's "Move to new window" make another one here.
struct Win {
    window: Window,
    host: python::Host,
    ns_view: usize,
    cursor: (f64, f64),
    held: u32,
    last_press: Option<(Instant, u32, (f64, f64))>,
    last_key_text: Option<String>,
}

struct App {
    args: Args,
    /// The open windows, oldest first.
    wins: Vec<(WindowId, Win)>,
    /// The window in front (menu commands, opened files, script steps).
    focused: Option<WindowId>,
    #[cfg(target_os = "macos")]
    _menus: Option<menu::Menus>,
    modifiers: ModifiersState,
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
    #[cfg(target_os = "macos")]
    recent_sig: Option<Vec<String>>,
    /// Files given on the command line: AppKit hands them to the Open
    /// With hook too (as odoc), which must not open them a second time.
    cli_files: Vec<PathBuf>,
    started: bool,
}

const DOUBLE_CLICK: Duration = Duration::from_millis(500);

impl App {
    fn win(&self, id: WindowId) -> Option<&Win> {
        self.wins.iter().find(|(w, _)| *w == id).map(|(_, w)| w)
    }

    fn win_mut(&mut self, id: WindowId) -> Option<&mut Win> {
        self.wins.iter_mut().find(|(w, _)| *w == id).map(|(_, w)| w)
    }

    /// The window in front, else the newest.
    fn front(&self) -> Option<WindowId> {
        self.focused.filter(|id| self.win(*id).is_some()).or_else(|| self.wins.last().map(|(id, _)| *id))
    }

    fn host(&self, id: WindowId) -> Option<&python::Host> {
        self.win(id).map(|w| &w.host)
    }

    fn front_host(&self) -> Option<&python::Host> {
        self.front().and_then(|id| self.win(id)).map(|w| &w.host)
    }

    fn scale(&self, id: WindowId) -> f64 {
        self.win(id).map_or(2.0, |w| w.window.scale_factor())
    }

    fn logical(&self, id: WindowId, p: PhysicalPosition<f64>) -> (f64, f64) {
        let s = self.scale(id);
        (p.x / s, p.y / s)
    }

    fn mods(&self) -> u32 {
        keys::qt_modifiers(self.modifiers)
    }

    fn redraw(&self, id: WindowId) {
        if let Some(w) = self.win(id) {
            w.window.request_redraw();
        }
    }

    // ---- dispatch: real and scripted input both come through here -------
    fn pointer(&mut self, id: WindowId, kind: &str, x: f64, y: f64, button: u32, mods: u32) {
        let Some(w) = self.win_mut(id) else { return };
        w.cursor = (x, y);
        let mut kind = kind;
        if kind == "press" {
            w.held |= button;
            let now = Instant::now();
            if let Some((t, b, (px, py))) = w.last_press {
                if b == button && now - t < DOUBLE_CLICK && (px - x).hypot(py - y) < 5.0 {
                    kind = "double";
                }
            }
            w.last_press = if kind == "double" { None } else { Some((now, button, (x, y))) };
        } else if kind == "release" {
            w.held &= !button;
        }
        let held = w.held;
        w.host.call("pointer", (kind, x, y, button, held, mods));
        self.redraw(id);
    }

    fn key(&mut self, id: WindowId, pressed: bool, code: u32, text: &str, repeat: bool, mods: u32) {
        if let Some(w) = self.win(id) {
            w.host.call("key", (pressed, code, mods, text, repeat));
        }
        self.redraw(id);
    }

    fn command(&mut self, event_loop: &ActiveEventLoop, cmd: &str) {
        let front = self.front();
        match cmd {
            // Close Window (⇧⌘W): the window in front, after asking about
            // its unsaved work (app_shell.files: the sheet, answered
            // through the modal pump). The last window closed ends the app.
            "close" => {
                if let Some(id) = front {
                    self.close_window(event_loop, id);
                }
            }
            // Quit asks each window in turn; a Cancel stops there.
            "quit" => {
                let ids: Vec<WindowId> = self.wins.iter().map(|(id, _)| *id).collect();
                for id in ids {
                    let ok = self.win(id).map_or(true, |w| ask_close(&w.host));
                    if !ok {
                        self.focus(id);
                        return;
                    }
                }
                event_loop.exit();
            }
            "activate" => {
                if let Some(id) = front {
                    self.focus(id);
                }
            }
            other => {
                if let Some(h) = self.front_host() {
                    h.call("menu", (other,));
                }
            }
        }
        if let Some(id) = self.front() {
            self.redraw(id);
        }
    }

    fn focus(&mut self, id: WindowId) {
        if let Some(w) = self.win(id) {
            w.window.focus_window();
        }
        self.focused = Some(id);
    }

    /// Ask the window's host, then drop the window; the last one ends the app.
    fn close_window(&mut self, event_loop: &ActiveEventLoop, id: WindowId) {
        let ok = self.win(id).map_or(true, |w| ask_close(&w.host));
        if !ok {
            return;
        }
        if let Some(i) = self.wins.iter().position(|(w, _)| *w == id) {
            let (_, w) = self.wins.remove(i);
            // The renderer (and its CAMetalLayer) before the window goes.
            w.host.call0("close");
            drop(w);
        }
        if self.focused == Some(id) {
            self.focused = self.wins.last().map(|(id, _)| *id);
        }
        if self.wins.is_empty() {
            event_loop.exit();
        }
    }

    fn open(&mut self, path: &Path) {
        let t = Instant::now();
        let Some(id) = self.front() else {
            self.pending_opens.push(path.to_path_buf());
            return;
        };
        if let Some(w) = self.win(id) {
            w.host.call("open_path", (path.to_string_lossy().to_string(),));
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!("[built] opened {} in {ms:.0} ms", path.display());
        self.timings.opened.get_or_insert(self.timings.process.elapsed());
        if let Some(w) = self.win(id) {
            w.window.focus_window();
        }
        self.redraw(id);
    }

    fn capture(&mut self, name: &str) {
        let Some(id) = self.front() else { return };
        let Some(win) = self.win(id) else { return };
        let Some(rgba) = win.host.capture() else {
            eprintln!("[built] capture {name}: no frame");
            return;
        };
        let size = win.window.inner_size();
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

    fn print_state(&self) {
        if let Some(id) = self.front() {
            if let Some(w) = self.win(id) {
                println!("[built] state {}", w.host.state_json());
                println!("[built] windows {{\"count\": {}, \"front\": {}, \"macos_tabs_allowed\": {}}}",
                         self.wins.len(),
                         self.wins.iter().position(|(i, _)| *i == id).map_or(-1, |i| i as i64),
                         self.wins.iter().any(|(_, w)| native::window_tabs_allowed(w.ns_view)));
            }
        }
    }

    /// One scripted step per turn; `wait` lets the loop run normally.
    /// Steps go to the window in front (a new window takes them over).
    fn run_script(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(at) = self.resume_at {
            if Instant::now() < at {
                event_loop.set_control_flow(ControlFlow::WaitUntil(at));
                return;
            }
            self.resume_at = None;
        }
        let Some(step) = self.steps.pop_front() else { return };
        let Some(id) = self.front() else { return };
        match step {
            Step::Open(p) => self.open(&absolute(Path::new(&p))),
            Step::Wait(ms) => self.resume_at = Some(Instant::now() + Duration::from_millis(ms)),
            Step::Move(x, y) => self.pointer(id, "move", x, y, 0, 0),
            Step::Click(x, y, b, m) => {
                self.pointer(id, "move", x, y, 0, m);
                self.pointer(id, "press", x, y, b, m);
                self.pointer(id, "release", x, y, b, m);
            }
            Step::Double(x, y, b, m) => {
                if let Some(w) = self.win_mut(id) {
                    w.last_press = None;
                }
                for _ in 0..2 {
                    self.pointer(id, "press", x, y, b, m);
                    self.pointer(id, "release", x, y, b, m);
                }
            }
            Step::Drag { from, to, button, steps, mods } => {
                self.pointer(id, "move", from.0, from.1, 0, mods);
                self.pointer(id, "press", from.0, from.1, button, mods);
                for i in 1..=steps {
                    let f = i as f64 / steps as f64;
                    let (x, y) = (from.0 + (to.0 - from.0) * f, from.1 + (to.1 - from.1) * f);
                    self.pointer(id, "move", x, y, 0, mods);
                    if let Some(h) = self.host(id) {
                        h.frame();
                    }
                }
                self.pointer(id, "release", to.0, to.1, button, mods);
            }
            Step::Key(name, mods) => match keys::qt_key_by_name(&name) {
                Some((code, text)) => {
                    let text = if mods & (keys::CONTROL | keys::META) != 0 { String::new() } else { text };
                    self.key(id, true, code, &text, false, mods);
                    self.key(id, false, code, &text, false, mods);
                }
                None => eprintln!("[built] script: unknown key {name:?}"),
            },
            Step::Type(text) => {
                for c in text.chars() {
                    let s = c.to_string();
                    self.key(id, true, keys::qt_char(c), &s, false, 0);
                    self.key(id, false, keys::qt_char(c), &s, false, 0);
                }
            }
            Step::Menu(cmd) => self.command(event_loop, &cmd),
            Step::Preedit(text) => {
                if let Some(h) = self.host(id) {
                    let n = text.chars().count();
                    h.call("preedit", (text, n));
                }
            }
            Step::Commit(text) => {
                if let Some(h) = self.host(id) {
                    h.call("text", (text,));
                }
            }
            Step::Wheel(x, y, dy, phase) => {
                if let Some(h) = self.host(id) {
                    h.call("wheel", (x, y, 0.0, dy, 0.0, dy * 2.0, phase.as_str(), 0u32, 0u32, true));
                }
            }
            Step::Notch(x, y, n) => {
                if let Some(h) = self.host(id) {
                    h.call("wheel", (x, y, 0.0, 0.0, 0.0, n * 120.0, "none", 0u32, 0u32, false));
                }
            }
            Step::Pinch(x, y, v) => {
                if let Some(h) = self.host(id) {
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
            Step::State => self.print_state(),
            Step::Quit => event_loop.exit(),
        }
        if self.steps.is_empty() && self.resume_at.is_none() {
            self.print_state();
            event_loop.exit();
        }
        if let Some(id) = self.front() {
            self.redraw(id);
        }
    }

    /// Let deferred work land before a capture: Qt timers, the snap index.
    fn settle(&mut self) {
        let Some(h) = self.front_host() else { return };
        for _ in 0..4 {
            let _ = h.idle();
            h.frame();
        }
    }

    /// A window and its Python host (its own renderer on its NSView).
    fn make_window(&mut self, event_loop: &ActiveEventLoop) -> Result<WindowId, String> {
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
        if self.timings.window.is_none() {
            self.timings.window = Some(self.timings.process.elapsed());
        }
        let window_handle = window.window_handle().map_err(|e| e.to_string())?.as_raw();
        let display_handle = window.display_handle().map_err(|e| e.to_string())?.as_raw();
        let (window_id, display, platform) = match (window_handle, display_handle) {
            #[cfg(target_os = "macos")]
            (RawWindowHandle::AppKit(w), RawDisplayHandle::AppKit(_)) => (w.ns_view.as_ptr() as usize, 0, "cocoa"),
            (RawWindowHandle::Win32(w), RawDisplayHandle::Windows(_)) => (w.hwnd.get() as usize, 0, "windows"),
            (RawWindowHandle::Wayland(w), RawDisplayHandle::Wayland(d)) => (w.surface.as_ptr() as usize, d.display.as_ptr() as usize, "wayland"),
            (RawWindowHandle::Xlib(w), RawDisplayHandle::Xlib(d)) => (w.window as usize, d.display.ok_or("missing X11 display")?.as_ptr() as usize, "xlib"),
            (RawWindowHandle::Xcb(w), RawDisplayHandle::Xcb(d)) => (w.window.get() as usize, d.connection.ok_or("missing XCB connection")?.as_ptr() as usize, "xcb"),
            _ => return Err("no supported GPU surface for this window".into()),
        };
        let ns_view = if platform == "cocoa" { window_id } else { 0 };
        native::no_window_tabs(ns_view);
        let size = window.inner_size();
        let scale = window.scale_factor();
        let host = python::Host::new(
            (window_id, display, platform),
            size.width as f64 / scale,
            size.height as f64 / scale,
            scale,
            self.args.settings.as_deref(), self.args.temporary_profile(),
        )
        .map_err(|e| {
            pyo3::Python::attach(|py| python::report(py, e, "Host"));
            "the Python host did not start".to_string()
        })?;
        host.call0("attach_surface").ok_or("the renderer did not start")?;
        if self.timings.host.is_none() {
            self.timings.host = Some(self.timings.process.elapsed());
        }
        let id = window.id();
        self.wins.push((id, Win {
            window, host, ns_view, cursor: (0.0, 0.0), held: 0, last_press: None, last_key_text: None,
        }));
        self.focused = Some(id);
        Ok(id)
    }

    fn start(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            // One strip of tabs: the app's own. AppKit's window tabs stay off.
            use winit::platform::macos::ActiveEventLoopExtMacOS;
            event_loop.set_allows_automatic_window_tabbing(false);
        }
        let id = self.make_window(event_loop)?;
        #[cfg(target_os = "macos")]
        match menu::build() {
            Ok(m) => self._menus = Some(m),
            Err(e) => eprintln!("[built] menu bar: {e}"),
        }
        #[cfg(built_bundle)]
        if !self.args.temporary_profile() {
            if let Some(w) = self.win(id) {
                w.host.call("menu", ("check_updates_quiet",));
            }
        }
        let mut files: Vec<PathBuf> = self.args.file.clone().into_iter().collect();
        files.append(&mut self.pending_opens);
        if files.is_empty() {
            // A cold start without a file: the start page (recents, new,
            // templates), and the untitled drawings an earlier session
            // left in its auto-save slots.
            if let Some(w) = self.win(id) {
                w.host.call0("cold_start");
            }
        }
        for file in files {
            self.open(&file);
        }
        self.redraw(id);
        Ok(())
    }

    /// A window Python asked for (app_shell.windows): made here, then its
    /// host adopts the request (the start page, or a project moved in).
    fn new_window(&mut self, event_loop: &ActiveEventLoop, token: &str) {
        match self.make_window(event_loop) {
            Ok(id) => {
                if let Some(w) = self.win(id) {
                    w.host.call("adopt", (token,));
                    w.window.focus_window();
                }
                self.redraw(id);
            }
            Err(e) => eprintln!("[built] new window: {e}"),
        }
    }

    fn report_measure(&self) {
        let t = &self.timings;
        let ms = |d: Option<Duration>| d.map_or(-1.0, |d| d.as_secs_f64() * 1000.0);
        let open_ms = self.front_host().map(|h| h.state_json()).unwrap_or_default();
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

    /// One window's turn: Python's idle, cursor, title, drag, and what else
    /// it asks for (a new window, to come to the front).
    fn turn(&mut self, event_loop: &ActiveEventLoop, id: WindowId) -> Option<f64> {
        let Some(w) = self.win(id) else { return None };
        let (wanted, wake_ms, cursor, title, drag) = w.host.idle();
        let extra = w.host.extras();
        if extra.exit {
            event_loop.exit();
            return None;
        }
        if let Some(c) = cursor {
            if let Some(key) = c.strip_prefix("custom:") {
                if !self.cursors.contains_key(key) {
                    if let Some((cw, ch, hx, hy, rgba)) = w.host.custom_cursor(key) {
                        match CustomCursor::from_rgba(rgba, cw, ch, hx, hy) {
                            Ok(src) => {
                                let made = event_loop.create_custom_cursor(src);
                                self.cursors.insert(key.to_owned(), made);
                            }
                            Err(e) => eprintln!("[built] cursor {key}: {e}"),
                        }
                    }
                }
                if let (Some(w), Some(cur)) = (self.win(id), self.cursors.get(key)) {
                    w.window.set_cursor(Cursor::Custom(cur.clone()));
                }
            } else if let Some(w) = self.win(id) {
                w.window.set_cursor(cursor_icon(&c));
            }
        }
        if let Some(w) = self.win(id) {
            if let Some((x, y, cw, ch)) = extra.ime_area {
                w.window.set_ime_cursor_area(winit::dpi::LogicalPosition::new(x, y), LogicalSize::new(cw, ch));
            }
            if wanted {
                w.window.request_redraw();
            }
            if let Some(t) = title {
                w.window.set_title(&t);
            }
            if drag {
                let _ = w.window.drag_window();
            }
        }
        #[cfg(target_os = "macos")]
        if let Some(recent) = extra.recent {
            if self.recent_sig.as_ref() != Some(&recent) {
                if let Some(m) = &self._menus {
                    m.set_recent(&recent);
                }
                self.recent_sig = Some(recent);
            }
        }
        if extra.focus {
            self.focus(id);
        }
        if let Some(token) = extra.new_window {
            self.new_window(event_loop, &token);
        }
        wake_ms
    }
}

/// `Host.request_close()`: True once nothing unsaved stands in the way.
fn ask_close(host: &python::Host) -> bool {
    host.call0("request_close")
        .map(|v| pyo3::Python::attach(|py| v.extract::<bool>(py).unwrap_or(false)))
        .unwrap_or(true)
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
        if self.started {
            return;
        }
        self.started = true;
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

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.win(id).is_none() {
            return;
        }
        match event {
            WindowEvent::CloseRequested => self.close_window(event_loop, id),
            WindowEvent::Resized(size) => {
                let s = self.scale(id);
                if let Some(w) = self.win(id) {
                    w.host.call("resize", (size.width as f64 / s, size.height as f64 / s, s));
                }
                self.redraw(id);
            }
            WindowEvent::RedrawRequested => {
                let drew = self.win(id).is_some_and(|w| w.host.frame().is_some());
                if drew {
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
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::CursorMoved { position, .. } => {
                let (x, y) = self.logical(id, position);
                let m = self.mods();
                self.pointer(id, "move", x, y, 0, m);
            }
            WindowEvent::CursorLeft { .. } => {
                if let Some(w) = self.win(id) {
                    w.host.call("pointer", ("leave", -1.0, -1.0, 0u32, 0u32, 0u32));
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let bit = mouse_bit(button);
                let (x, y) = self.win(id).map_or((0.0, 0.0), |w| w.cursor);
                let m = self.mods();
                let kind = if state == ElementState::Pressed { "press" } else { "release" };
                self.pointer(id, kind, x, y, bit, m);
            }
            WindowEvent::MouseWheel { delta, phase, .. } => {
                let m = self.mods();
                let s = self.scale(id);
                if let Some(w) = self.win(id) {
                    let (x, y) = w.cursor;
                    let held = w.held;
                    match delta {
                        MouseScrollDelta::LineDelta(dx, dy) => {
                            w.host.call("wheel", (x, y, 0.0, 0.0, dx as f64 * 120.0, dy as f64 * 120.0,
                                                  "none", m, held, false));
                        }
                        MouseScrollDelta::PixelDelta(p) => {
                            let (dx, dy) = (p.x / s, p.y / s);
                            // Qt's Cocoa angleDelta for precise deltas: 2× pixels.
                            w.host.call("wheel", (x, y, dx, dy, dx * 2.0, dy * 2.0, phase_name(phase),
                                                  m, held, true));
                        }
                    }
                }
                self.redraw(id);
            }
            WindowEvent::PinchGesture { delta, phase, .. } => {
                if let Some(w) = self.win(id) {
                    let (x, y) = w.cursor;
                    if phase == TouchPhase::Started {
                        w.host.call("gesture", ("begin", 0.0, x, y));
                    }
                    if delta.is_finite() && delta != 0.0 {
                        w.host.call("gesture", ("zoom", delta, x, y));
                    }
                    if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                        w.host.call("gesture", ("end", 0.0, x, y));
                    }
                }
                self.redraw(id);
            }
            WindowEvent::RotationGesture { delta, .. } => {
                if let Some(w) = self.win(id) {
                    let (x, y) = w.cursor;
                    w.host.call("gesture", ("rotate", delta as f64, x, y));
                }
            }
            WindowEvent::DoubleTapGesture { .. } => {
                if let Some(w) = self.win(id) {
                    let (x, y) = w.cursor;
                    w.host.call("gesture", ("smart_zoom", 0.0, x, y));
                }
                self.redraw(id);
            }
            WindowEvent::KeyboardInput { event, is_synthetic, .. } => {
                if is_synthetic {
                    return;
                }
                let m = self.mods();
                let (code, text) = keys::qt_key(&event.logical_key, event.text.as_deref(), m);
                let pressed = event.state == ElementState::Pressed;
                if pressed {
                    if let Some(w) = self.win_mut(id) {
                        w.last_key_text = Some(text.clone()).filter(|t| !t.is_empty());
                    }
                }
                if code != 0 || !text.is_empty() {
                    self.key(id, pressed, code, &text, event.repeat, m);
                }
            }
            WindowEvent::Ime(Ime::Preedit(text, cursor)) => {
                // The composition being typed (dead keys, input methods):
                // shown in the chrome's focused field, or as the value
                // being typed, until it commits.
                if let Some(w) = self.win(id) {
                    let at = cursor.map_or(text.chars().count(), |(a, _)| text[..a.min(text.len())].chars().count());
                    w.host.call("preedit", (text, at));
                }
                self.redraw(id);
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                // A plain key already delivered its own text; a composed
                // one (dead keys, input methods) arrives only here.
                let typed = self.win_mut(id).and_then(|w| w.last_key_text.take());
                if typed.as_deref() != Some(text.as_str()) {
                    if let Some(w) = self.win(id) {
                        w.host.call("text", (text,));
                    }
                    self.redraw(id);
                }
            }
            WindowEvent::Focused(on) => {
                if on {
                    self.focused = Some(id);
                }
                if let Some(w) = self.win(id) {
                    w.host.call("focus", (on,));
                }
            }
            WindowEvent::DroppedFile(path) => {
                self.focused = Some(id);
                self.open(&path)
            }
            WindowEvent::HoveredFile(path) => {
                if let Some(w) = self.win(id) {
                    w.host.call("drag_hover", (path.to_string_lossy().to_string(),));
                }
            }
            WindowEvent::HoveredFileCancelled => {
                if let Some(w) = self.win(id) {
                    w.host.call("drag_hover", ("",));
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if !self.steps.is_empty() || self.resume_at.is_some() {
            self.run_script(event_loop);
        }
        let mut wake: Option<f64> = None;
        let ids: Vec<WindowId> = self.wins.iter().map(|(id, _)| *id).collect();
        for id in ids {
            if let Some(ms) = self.turn(event_loop, id) {
                wake = Some(wake.map_or(ms, |w: f64| w.min(ms)));
            }
        }
        let scripted = !self.steps.is_empty() && self.resume_at.is_none();
        let flow = if scripted {
            ControlFlow::Poll
        } else {
            match (wake, self.resume_at) {
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
    let host = match python::Host::new((0, 0, ""), args.size.0, args.size.1, 2.0, args.settings.as_deref(), args.temporary_profile()) {
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
    #[cfg(built_bundle)]
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--qt")) {
        use std::os::unix::process::CommandExt;
        let classic = std::env::current_exe().unwrap().with_file_name("ingetrazo");
        let error = std::process::Command::new(classic).args(std::env::args_os().skip(2)).exec();
        eprintln!("built: launching classic app: {error}");
        std::process::exit(1);
    }
    let process = Instant::now();
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("built: {e}");
            std::process::exit(2);
        }
    };
    if args.gallery { std::env::set_var("BUILT_GALLERY", "1"); }
    #[cfg(unix)]
    let shared_instance = !args.temporary_profile() && args.settings.is_none()
        && std::env::var_os("BUILT_SETTINGS_DIR").is_none() && !args.new_instance;
    #[cfg(unix)]
    if shared_instance && forward_to_running(args.file.as_deref()) {
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
        #[cfg(built_bundle)]
        let args = {
            if let Err(error) = pyo3::Python::attach(|py| {
                use pyo3::prelude::*;
                py.import("native_check")?.call_method0("check")?;
                PyResult::Ok(())
            }) {
                pyo3::Python::attach(|py| python::report(py, error, "bundle check"));
                std::process::exit(1);
            }
            let mut args = args;
            args.file.get_or_insert_with(|| layout.root.join("examples/slab-and-cube.igz"));
            args
        };
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
    #[cfg(target_os = "macos")]
    {
        let proxy = proxy.clone();
        muda::MenuEvent::set_event_handler(Some(move |e: muda::MenuEvent| {
            let _ = proxy.send_event(UserEvent::Menu(e.id().0.clone()));
        }));
    }
    #[cfg(target_os = "macos")]
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
    #[cfg(unix)]
    if shared_instance {
        listen_for_instances(proxy);
    }
    let mut app = App {
        args,
        wins: Vec::new(),
        focused: None,
        #[cfg(target_os = "macos")]
        _menus: None,
        modifiers: ModifiersState::empty(),
        steps: steps.into(),
        resume_at: None,
        frames: 0,
        timings: Timings { process, python_ready, window: None, host: None, opened: None, first_frame: None },
        exit_code: 0,
        cursors: HashMap::new(),
        pending_opens: Vec::new(),
        #[cfg(target_os = "macos")]
        recent_sig: None,
        cli_files: Vec::new(),
        started: false,
    };
    app.cli_files = app.args.file.iter().chain(&app.args.script).cloned().collect();
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("built: {e}");
        app.exit_code = 1;
    }
    #[cfg(unix)]
    if shared_instance {
        let _ = std::fs::remove_file(instance_socket());
    }
    // Drop each renderer (and its CAMetalLayer) before its window goes.
    for (_, w) in app.wins.drain(..) {
        w.host.call0("close");
    }
    std::process::exit(app.exit_code);
}


#[cfg(test)]
mod profile_tests {
    use super::*;

    #[test]
    fn only_automation_requests_a_temporary_profile() {
        let temporary = |args: &[&str]| {
            parse_args(args.iter().map(|s| s.to_string())).unwrap().temporary_profile()
        };
        assert!(!temporary(&[]));
        assert!(!temporary(&["--new-instance", "drawing.built"]));
        for args in [&["--script", "steps"][..], &["--check"], &["--measure"], &["--gallery"]] {
            assert!(temporary(args));
        }
    }
}
