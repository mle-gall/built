//! `built_shell`: the platform services the embedded Python asks for.
//!
//! Registered as a built-in module next to `built_render` (python.rs),
//! so `import built_shell` works only inside the `built` binary; the
//! Host service (`shell_runtime/services.rs`) falls back to doing nothing when it
//! is missing (pytest, the Qt app).
//!
//! * **File dialogs** (rfd): NSOpenPanel / NSSavePanel, run modally inside
//!   the winit handler, which winit supports (rust-windowing/winit#1779).
//! * **The modal pump**: one turn of AppKit's queue in
//!   `NSModalPanelRunLoopMode`, for a question asked synchronously from
//!   Python (`chrome_dialogs.ask`, the Save/Don't Save sheet) or a long
//!   job that reports progress. winit's handler is busy for the whole
//!   call, so its queued closures (default mode only) and its run-loop
//!   observers (which check that the handler is free) stay out. Input
//!   events for the shell's window are *not* sent to the NSView: winit
//!   would queue them and replay every click after the question was
//!   answered. They come back to Python instead, already in Qt's terms,
//!   for the chrome (the sheet's buttons, Cancel on a job). Everything
//!   else (activation, other windows, the menu bar) is sent as usual.
//! * **The pasteboard** (NSPasteboard): typed data, so geometry copied in
//!   one Built goes to another, and plain text for the rest of the Mac.
//! * **Open With / Dock quit**: methods added to winit's own application
//!   delegate (see [`install_app_hooks`]).

use pyo3::prelude::*;

// ---- file dialogs -----------------------------------------------------------

type Filters = Vec<(String, Vec<String>)>;

fn dialog(title: &str, filters: &Filters, directory: Option<String>) -> rfd::FileDialog {
    let mut d = rfd::FileDialog::new().set_title(title);
    for (name, exts) in filters {
        let exts: Vec<&str> = exts.iter().map(|s| s.trim_start_matches('.')).collect();
        if !exts.is_empty() {
            d = d.add_filter(name, &exts);
        }
    }
    if let Some(dir) = directory.filter(|d| !d.is_empty()) {
        d = d.set_directory(dir);
    }
    d
}

/// Files picked in an open panel ([] when cancelled).
#[pyfunction]
#[pyo3(signature = (title, filters=Vec::new(), directory=None, multiple=false))]
fn open_files(py: Python<'_>, title: &str, filters: Filters, directory: Option<String>, multiple: bool) -> Vec<String> {
    let d = dialog(title, &filters, directory);
    // The panel is modal on the main thread; Python's own threads (the
    // snap-index worker) keep going meanwhile.
    let picked = py.detach(move || if multiple { d.pick_files().unwrap_or_default() } else { d.pick_file().into_iter().collect() });
    picked.into_iter().map(|p| p.to_string_lossy().to_string()).collect()
}

/// The path chosen in a save panel, or None.
#[pyfunction]
#[pyo3(signature = (title, name="", filters=Vec::new(), directory=None))]
fn save_file(py: Python<'_>, title: &str, name: &str, filters: Filters, directory: Option<String>) -> Option<String> {
    let mut d = dialog(title, &filters, directory);
    if !name.is_empty() {
        d = d.set_file_name(name);
    }
    py.detach(move || d.save_file()).map(|p| p.to_string_lossy().to_string())
}

/// A folder chosen in an open panel, or None.
#[pyfunction]
#[pyo3(signature = (title, directory=None))]
fn pick_folder(py: Python<'_>, title: &str, directory: Option<String>) -> Option<String> {
    let d = dialog(title, &Vec::new(), directory);
    py.detach(move || d.pick_folder()).map(|p| p.to_string_lossy().to_string())
}

/// A native alert, when the chrome cannot ask: the index of the button.
#[pyfunction]
#[pyo3(signature = (title, text, buttons, tone="info"))]
fn alert(py: Python<'_>, title: &str, text: &str, buttons: Vec<String>, tone: &str) -> usize {
    use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
    let level = match tone {
        "warning" => MessageLevel::Warning,
        "error" => MessageLevel::Error,
        _ => MessageLevel::Info,
    };
    let labels = buttons.clone();
    let kind = match labels.len() {
        0 | 1 => MessageButtons::OkCustom(labels.first().cloned().unwrap_or_else(|| "OK".into())),
        2 => MessageButtons::OkCancelCustom(labels[0].clone(), labels[1].clone()),
        _ => MessageButtons::YesNoCancelCustom(labels[0].clone(), labels[1].clone(), labels[2].clone()),
    };
    let d = MessageDialog::new().set_title(title).set_description(text).set_level(level).set_buttons(kind);
    let got = py.detach(move || d.show());
    match got {
        MessageDialogResult::Custom(s) => buttons.iter().position(|b| *b == s).unwrap_or(0),
        MessageDialogResult::Ok | MessageDialogResult::Yes => 0,
        MessageDialogResult::No => 1,
        MessageDialogResult::Cancel => buttons.len().saturating_sub(1),
    }
}

// ---- the modal pump -----------------------------------------------------------

#[cfg(windows)]
#[path = "modal_windows.rs"]
mod modal_windows;

#[cfg(all(unix, not(target_os = "macos")))]
#[path = "modal_linux.rs"]
mod modal_linux;

/// (kind, x, y, button bit, Qt modifiers, Qt key, text).
pub type Input = (String, f64, f64, u32, u32, u32, String);

/// Qt's modifier bits from AppKit's flags (⌘ is Qt's Control on the Mac).
#[cfg(target_os = "macos")]
fn qt_mods(flags: objc2_app_kit::NSEventModifierFlags) -> u32 {
    use objc2_app_kit::NSEventModifierFlags as F;
    let mut m = 0;
    if flags.contains(F::Shift) { m |= crate::keys::SHIFT; }
    if flags.contains(F::Command) { m |= crate::keys::CONTROL; }
    if flags.contains(F::Option) { m |= crate::keys::ALT; }
    if flags.contains(F::Control) { m |= crate::keys::META; }
    m
}

/// Qt's key code for an AppKit key event (the keys a sheet or a job hears;
/// letters and digits by their character).
pub fn qt_key_for(key_code: u16, chars: &str) -> u32 {
    match key_code {
        36 => 0x0100_0004,  // Return
        76 => 0x0100_0005,  // Enter
        53 => 0x0100_0000,  // Escape
        48 => 0x0100_0001,  // Tab
        51 => 0x0100_0003,  // Backspace
        117 => 0x0100_0007, // Delete
        123 => 0x0100_0012, // Left
        124 => 0x0100_0014, // Right
        125 => 0x0100_0015, // Down
        126 => 0x0100_0013, // Up
        49 => 0x20,         // Space
        _ => chars.chars().next().map_or(0, |c| c.to_ascii_uppercase() as u32),
    }
}

/// Wait up to `timeout_ms` for AppKit events, handle the ones that are not
/// the shell window's input, and return those that are.
#[pyfunction]
#[pyo3(signature = (view, timeout_ms=16.0))]
fn pump(py: Python<'_>, view: usize, timeout_ms: f64) -> PyResult<Vec<Input>> {
    #[cfg(target_os = "macos")]
    { let _ = py; Ok(pump_macos(view, timeout_ms)) }
    #[cfg(windows)]
    { let _ = py; Ok(modal_windows::pump(view, timeout_ms)) }
    #[cfg(all(unix, not(target_os = "macos")))]
    { modal_linux::pump(py, view, timeout_ms) }
}

#[cfg(target_os = "macos")]
fn pump_macos(view: usize, timeout_ms: f64) -> Vec<Input> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSEventMask, NSEventType, NSModalPanelRunLoopMode, NSView};
    use objc2_foundation::NSDate;
    let mut out = Vec::new();
    let Some(mtm) = MainThreadMarker::new() else { return out };
    let app = NSApplication::sharedApplication(mtm);
    let ns_view: Option<&NSView> = if view == 0 { None } else { Some(unsafe { &*(view as *const NSView) }) };
    let ours = ns_view.and_then(|v| v.window());
    let mut until = NSDate::dateWithTimeIntervalSinceNow(timeout_ms.max(0.0) / 1000.0);
    // Never more than a frame's worth of events per call.
    for _ in 0..64 {
        let event = unsafe {
            app.nextEventMatchingMask_untilDate_inMode_dequeue(NSEventMask::Any, Some(&until), NSModalPanelRunLoopMode, true)
        };
        let Some(event) = event else { break };
        until = NSDate::distantPast();
        let kind = event.r#type();
        let in_ours = match (&ours, event.window(mtm)) {
            (Some(a), Some(b)) => std::ptr::eq(&**a, &*b),
            (Some(_), None) => matches!(kind, NSEventType::KeyDown | NSEventType::KeyUp),
            _ => false,
        };
        let mouse = match kind {
            NSEventType::LeftMouseDown => Some(("press", 1)),
            NSEventType::LeftMouseUp => Some(("release", 1)),
            NSEventType::RightMouseDown => Some(("press", 2)),
            NSEventType::RightMouseUp => Some(("release", 2)),
            NSEventType::OtherMouseDown => Some(("press", 4)),
            NSEventType::OtherMouseUp => Some(("release", 4)),
            NSEventType::MouseMoved | NSEventType::LeftMouseDragged | NSEventType::RightMouseDragged
            | NSEventType::OtherMouseDragged => Some(("move", 0)),
            _ => None,
        };
        if in_ours {
            if let (Some((name, bit)), Some(v)) = (mouse, ns_view) {
                let p = v.convertPoint_fromView(event.locationInWindow(), None);
                let y = if v.isFlipped() { p.y } else { v.frame().size.height - p.y };
                out.push((name.to_owned(), p.x, y, bit, qt_mods(event.modifierFlags()), 0, String::new()));
                continue;
            }
            match kind {
                NSEventType::KeyDown | NSEventType::KeyUp => {
                    let bare = event.charactersIgnoringModifiers().map(|s| s.to_string()).unwrap_or_default();
                    let text = event.characters().map(|s| s.to_string()).unwrap_or_default();
                    let name = if kind == NSEventType::KeyDown { "key" } else { "keyup" };
                    out.push((name.to_owned(), 0.0, 0.0, 0, qt_mods(event.modifierFlags()),
                              qt_key_for(event.keyCode(), &bare), text));
                    continue;
                }
                // The model is the sheet's while it waits: no scrolling,
                // pinching or rotating behind it.
                NSEventType::ScrollWheel | NSEventType::Magnify | NSEventType::Rotate
                | NSEventType::Swipe | NSEventType::SmartMagnify | NSEventType::Gesture
                | NSEventType::BeginGesture | NSEventType::EndGesture | NSEventType::Pressure => continue,
                _ => {}
            }
        }
        app.sendEvent(&event);
    }
    out
}

/// Put a key press and release for the shell's window at the front of
/// this app's own event queue (never the system's: nothing reaches other
/// apps or moves the pointer). Scripted runs answer a sheet with it, so
/// the answer travels the same way a real Return does: the modal pump.
#[pyfunction]
#[pyo3(signature = (view, key_code, chars, command=false))]
fn post_key(view: usize, key_code: u16, chars: &str, command: bool) -> bool {
    #[cfg(target_os = "macos")]
    {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSView};
        use objc2_foundation::{NSPoint, NSString};
        let Some(mtm) = MainThreadMarker::new() else { return false };
        if view == 0 {
            return false;
        }
        let v: &NSView = unsafe { &*(view as *const NSView) };
        let Some(window) = v.window() else { return false };
        let app = NSApplication::sharedApplication(mtm);
        let flags = if command { NSEventModifierFlags::Command } else { NSEventModifierFlags::empty() };
        let text = NSString::from_str(chars);
        for kind in [NSEventType::KeyUp, NSEventType::KeyDown] {
            let ev = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                    kind, NSPoint::new(0.0, 0.0), flags, 0.0, window.windowNumber(), None, &text, &text, false, key_code);
            let Some(ev) = ev else { return false };
            // At the start, in reverse: the press comes out first.
            app.postEvent_atStart(&ev, true);
        }
        true
    }
    #[cfg(windows)]
    { modal_windows::post_key(view, key_code, chars, command) }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = chars;
        Python::attach(|py| modal_linux::post_key(py, view, key_code, command)).unwrap_or(false)
    }
}

// ---- the pasteboard -----------------------------------------------------------

/// The general pasteboard, or the private one `BUILT_PASTEBOARD` names
/// (scripted runs and tests: the user's clipboard stays untouched).
#[cfg(target_os = "macos")]
fn board() -> objc2::rc::Retained<objc2_app_kit::NSPasteboard> {
    use objc2_app_kit::NSPasteboard;
    match std::env::var("BUILT_PASTEBOARD") {
        Ok(name) if !name.is_empty() => NSPasteboard::pasteboardWithName(&objc2_foundation::NSString::from_str(&name)),
        _ => NSPasteboard::generalPasteboard(),
    }
}

/// Replace the general pasteboard's contents: (type, bytes) pairs; a
/// `public.utf8-plain-text` entry is written as a string.
#[pyfunction]
fn pasteboard_write(items: Vec<(String, Vec<u8>)>) -> bool {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{NSArray, NSData, NSString};
        let pb = board();
        let types: Vec<objc2::rc::Retained<NSString>> = items.iter().map(|(t, _)| NSString::from_str(t)).collect();
        let refs: Vec<&NSString> = types.iter().map(|t| &**t).collect();
        let array = NSArray::from_slice(&refs);
        unsafe { pb.declareTypes_owner(&array, None) };
        let mut ok = true;
        for ((_, data), t) in items.iter().zip(types.iter()) {
            ok &= if t.to_string() == "public.utf8-plain-text" {
                pb.setString_forType(&NSString::from_str(&String::from_utf8_lossy(data)), t)
            } else {
                pb.setData_forType(Some(&NSData::with_bytes(data)), t)
            };
        }
        ok
    }
    #[cfg(not(target_os = "macos"))]
    {
        crate::clipboard::write(items)
    }
}

/// The bytes of `kind` on the general pasteboard, or None.
#[pyfunction]
fn pasteboard_read(kind: &str) -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::NSString;
        let pb = board();
        let t = NSString::from_str(kind);
        if kind == "public.utf8-plain-text" {
            return pb.stringForType(&t).map(|s| s.to_string().into_bytes());
        }
        pb.dataForType(&t).map(|d| d.to_vec())
    }
    #[cfg(not(target_os = "macos"))]
    {
        crate::clipboard::read(kind)
    }
}

/// The types on the general pasteboard and its change count.
#[pyfunction]
fn pasteboard_types() -> (Vec<String>, isize) {
    #[cfg(target_os = "macos")]
    {
        let pb = board();
        let types = pb.types().map(|a| a.iter().map(|s| s.to_string()).collect()).unwrap_or_default();
        (types, pb.changeCount())
    }
    #[cfg(not(target_os = "macos"))]
    {
        crate::clipboard::types()
    }
}

/// Use the sheet graphite while AppKit waits for a new drawable.
/// Disable macOS window tabs for this window (shell design: one strip, the
/// app's own project tabs; AppKit's tab bar would also collide with the
/// custom 40 pt top bar). The app-wide switch is winit's
/// `set_allows_automatic_window_tabbing(false)`; this makes sure no
/// window joins a tab group through "Prefer tabs" in System Settings.
pub fn configure_window(view: usize) {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{NSView, NSWindowTabbingMode};
        if view == 0 || objc2::MainThreadMarker::new().is_none() {
            return;
        }
        let v: &NSView = unsafe { &*(view as *const NSView) };
        if let Some(window) = v.window() {
            let g = f64::from(built_render::ui::tokens::SHEET.r()) / 255.0;
            let graphite = objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(g, g, g, 1.0);
            window.setBackgroundColor(Some(&graphite));
            window.setTabbingMode(NSWindowTabbingMode::Disallowed);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = view;
}

/// Background (test) runs: an invisible, click-through window behind every
/// other app's windows, so a scripted run never covers what the user is doing.
pub fn send_window_back(view: usize) {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSView;
        if view == 0 || objc2::MainThreadMarker::new().is_none() {
            return;
        }
        let v: &NSView = unsafe { &*(view as *const NSView) };
        if let Some(window) = v.window() {
            // Fully transparent and click-through: an accessory app's window
            // still shows on whatever Space is active, full-screen games
            // included, so being behind is not enough. It stays on screen
            // (ordered back) so drawing and frame pacing are unchanged.
            window.setAlphaValue(0.0);
            window.setIgnoresMouseEvents(true);
            window.setExcludedFromWindowsMenu(true);
            window.setHasShadow(false);
            window.orderBack(None);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = view;
}

/// Whether this window may join a macOS window tab group (`--script`'s
/// state, tests).
pub fn window_tabs_allowed(view: usize) -> bool {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{NSView, NSWindowTabbingMode};
        if view == 0 {
            return false;
        }
        let v: &NSView = unsafe { &*(view as *const NSView) };
        v.window().is_some_and(|w| w.tabbingMode() != NSWindowTabbingMode::Disallowed)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = view;
        false
    }
}

/// Start another `built` process: kept for `--new-instance` users. File ▸
/// New Window makes a window in this process now (shell_runtime/windows.rs).
#[pyfunction]
#[pyo3(signature = (args=Vec::new()))]
fn spawn_self(args: Vec<String>) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    std::process::Command::new(exe).arg("--new-instance").args(args).spawn().is_ok()
}

#[cfg(windows)]
static RAW_INPUT: std::sync::Mutex<Option<Py<PyAny>>> = std::sync::Mutex::new(None);

#[cfg(windows)]
#[pyfunction]
fn set_raw_input_handler(callback: Option<Py<PyAny>>) {
    *RAW_INPUT.lock().unwrap() = callback;
}

#[cfg(windows)]
pub fn raw_input_message(msg: *const std::ffi::c_void) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{MSG, WM_INPUT};
    // The winit hook supplies a valid MSG until this callback returns.
    if !msg.is_null() && unsafe { (*msg.cast::<MSG>()).message } == WM_INPUT {
        Python::attach(|py| {
            let callback = RAW_INPUT.lock().unwrap().as_ref().map(|f| f.clone_ref(py));
            if let Some(callback) = callback {
                if let Err(err) = callback.call1(py, (msg as usize,)) {
                    crate::python::report(py, err, "3D mouse report");
                }
            }
        });
    }
    false
}

#[pymodule]
pub fn built_shell(m: &Bound<'_, PyModule>) -> PyResult<()> {
    #[cfg(target_os = "macos")]
    m.add_function(wrap_pyfunction!(crate::menu_commands::menu_commands, m)?)?;
    #[cfg(any(windows,target_os="linux"))]
    m.add_function(wrap_pyfunction!(built_render::desktop_menu::menu_commands, m)?)?;
    #[cfg(windows)]
    m.add_function(wrap_pyfunction!(set_raw_input_handler,m)?)?;
    m.add_function(wrap_pyfunction!(built_render::platform_paths::cache_directory, m)?)?;
    m.add_function(wrap_pyfunction!(built_render::platform_paths::local_data_directory, m)?)?;
    m.add_function(wrap_pyfunction!(built_render::platform_paths::preferred_languages, m)?)?;
    m.add_function(wrap_pyfunction!(built_render::platform_paths::documents_directory, m)?)?;
    m.add_function(wrap_pyfunction!(built_render::platform_paths::recovery_directory, m)?)?;
    m.add_function(wrap_pyfunction!(open_files, m)?)?;
    m.add_function(wrap_pyfunction!(save_file, m)?)?;
    m.add_function(wrap_pyfunction!(pick_folder, m)?)?;
    m.add_function(wrap_pyfunction!(alert, m)?)?;
    m.add_function(wrap_pyfunction!(pump, m)?)?;
    m.add_function(wrap_pyfunction!(pasteboard_write, m)?)?;
    m.add_function(wrap_pyfunction!(pasteboard_read, m)?)?;
    m.add_function(wrap_pyfunction!(pasteboard_types, m)?)?;
    m.add_function(wrap_pyfunction!(spawn_self, m)?)?;
    m.add_function(wrap_pyfunction!(post_key, m)?)?;
    Ok(())
}

// ---- Open With, Dock quit ---------------------------------------------------------

/// What the application delegate hears that winit 0.30 does not pass on.
#[derive(Debug, Clone, PartialEq)]
pub enum AppRequest {
    Open(Vec<String>),
    Quit,
}

static SINK: std::sync::OnceLock<Box<dyn Fn(AppRequest) + Send + Sync>> = std::sync::OnceLock::new();

/// Finder's "Open With", a double-click on a document, a drop on the Dock
/// icon, and Quit from the Dock or at logout.
///
/// Why this way: winit 0.30 owns the NSApplication delegate and implements
/// neither `application:openURLs:` nor `applicationShouldTerminate:`, so
/// AppKit's own Apple-event handlers (odoc, quit) find nothing to call and
/// documents are dropped, or the app terminates without asking about
/// unsaved work. Replacing the delegate would cut winit off from launch
/// and activation; installing our own `odoc` Apple-event handler would be
/// overwritten by AppKit during `finishLaunching`, which is exactly when a
/// cold launch's document arrives. Adding the two methods to winit's
/// delegate class (`class_addMethod`, which never replaces an existing
/// method) lets AppKit's handlers find them, before and after launch, and
/// leaves everything winit does untouched. Call after the event loop is
/// built (winit sets its delegate there) and before `run_app`.
pub fn install_app_hooks(sink: impl Fn(AppRequest) + Send + Sync + 'static) -> bool {
    let _ = SINK.set(Box::new(sink));
    #[cfg(target_os = "macos")]
    {
        install_macos()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

fn send(req: AppRequest) {
    if let Some(sink) = SINK.get() {
        sink(req);
    }
}

/// Paths of `file://` URLs (others are not documents).
pub fn paths_of_urls(urls: &[String]) -> Vec<String> {
    urls.iter()
        .filter_map(|u| u.strip_prefix("file://"))
        .map(|p| percent_decode(p.strip_prefix("localhost").unwrap_or(p)))
        .collect()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(target_os = "macos")]
fn install_macos() -> bool {
    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2::{sel, MainThreadMarker};
    use objc2_app_kit::NSApplication;
    use objc2_foundation::{NSArray, NSString, NSURL};

    extern "C-unwind" fn open_urls(_this: *mut AnyObject, _cmd: Sel, _app: *mut AnyObject, urls: *mut NSArray<NSURL>) {
        let Some(urls) = (unsafe { urls.as_ref() }) else { return };
        let urls: Vec<String> = urls.iter().filter_map(|u| u.absoluteString().map(|s| s.to_string())).collect();
        let paths = paths_of_urls(&urls);
        if !paths.is_empty() {
            send(AppRequest::Open(paths));
        }
    }
    extern "C-unwind" fn open_files(_this: *mut AnyObject, _cmd: Sel, _app: *mut AnyObject, files: *mut NSArray<NSString>) {
        let Some(files) = (unsafe { files.as_ref() }) else { return };
        let paths: Vec<String> = files.iter().map(|s| s.to_string()).collect();
        if !paths.is_empty() {
            send(AppRequest::Open(paths));
        }
    }
    extern "C-unwind" fn should_terminate(_this: *mut AnyObject, _cmd: Sel, _app: *mut AnyObject) -> usize {
        // NSTerminateCancel: the shell asks about unsaved work, then ends
        // its own event loop (Built ▸ Quit takes the same path).
        send(AppRequest::Quit);
        0
    }

    let Some(mtm) = MainThreadMarker::new() else { return false };
    let app = NSApplication::sharedApplication(mtm);
    let Some(delegate) = app.delegate() else { return false };
    let obj: &AnyObject = unsafe { &*(objc2::rc::Retained::as_ptr(&delegate) as *const AnyObject) };
    let cls: &AnyClass = obj.class();
    let cls_ptr = cls as *const AnyClass as *mut AnyClass;
    unsafe {
        let a = objc2::ffi::class_addMethod(
            cls_ptr, sel!(application:openURLs:),
            std::mem::transmute::<extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut NSArray<NSURL>), Imp>(open_urls),
            c"v@:@@".as_ptr());
        let b = objc2::ffi::class_addMethod(
            cls_ptr, sel!(application:openFiles:),
            std::mem::transmute::<extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut NSArray<NSString>), Imp>(open_files),
            c"v@:@@".as_ptr());
        let c = objc2::ffi::class_addMethod(
            cls_ptr, sel!(applicationShouldTerminate:),
            std::mem::transmute::<extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> usize, Imp>(should_terminate),
            c"Q@:@".as_ptr());
        a.as_bool() && b.as_bool() && c.as_bool()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_urls_become_paths() {
        let urls = vec![
            "file:///Users/me/Models/Ride%20as-built.built".to_string(),
            "file://localhost/tmp/a.igz".to_string(),
            "https://example.com/x.built".to_string(),
        ];
        assert_eq!(paths_of_urls(&urls), vec!["/Users/me/Models/Ride as-built.built", "/tmp/a.igz"]);
        assert_eq!(percent_decode("caf%C3%A9%2"), "café%2");
    }

    #[test]
    fn sheet_keys_map_to_qt() {
        assert_eq!(qt_key_for(36, "\r"), 0x0100_0004);
        assert_eq!(qt_key_for(53, "\u{1b}"), 0x0100_0000);
        assert_eq!(qt_key_for(2, "d"), 'D' as u32);
    }

    #[test]
    fn app_requests_reach_the_sink() {
        // The sink is process-wide: the first install wins (as in main).
        let got = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let g = got.clone();
        let _ = SINK.set(Box::new(move |r| g.lock().unwrap().push(r)));
        send(AppRequest::Open(vec!["/tmp/a.built".into()]));
        send(AppRequest::Quit);
        let seen = got.lock().unwrap().clone();
        if !seen.is_empty() {
            assert_eq!(seen, vec![AppRequest::Open(vec!["/tmp/a.built".into()]), AppRequest::Quit]);
        }
    }
}


#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy)]
struct ScrollEvidence {
    device_id: u64,
    subtype: i16,
    source_pid: i64,
    source_state: i64,
}

#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy)]
struct ScrollObservation {
    precise: bool,
    signature: Option<ScrollEvidence>,
}

#[cfg(any(target_os = "macos", test))]
fn screen_emulated_scroll(event: Option<ScrollObservation>, app_pid: u32) -> bool {
    matches!(event, Some(ScrollObservation {
        precise: true,
        signature: Some(ScrollEvidence { device_id: 0, subtype: 1, source_pid, source_state: 0 }),
    }) if source_pid == i64::from(app_pid))
}

/// Inspect only scroll-event fields. CGEvent is borrowed from the live NSEvent.
#[cfg(target_os = "macos")]
fn scroll_evidence(event: &objc2_app_kit::NSEvent) -> Option<ScrollEvidence> {
    use std::ffi::c_void;
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventGetIntegerValueField(event: *const c_void, field: u32) -> i64;
    }
    // CGEventField constants from CGEventTypes.h.
    const SOURCE_UNIX_PROCESS_ID: u32 = 41;
    const SOURCE_STATE_ID: u32 = 45;
    let subtype = event.subtype().0;
    // Only the probed tablet-point subtype needs a deviceID query.
    if subtype != 1 {
        return None;
    }
    let cg: *const c_void = unsafe { objc2::msg_send![event, CGEvent] };
    if cg.is_null() {
        return None;
    }
    Some(ScrollEvidence {
        device_id: event.deviceID() as u64,
        subtype,
        source_pid: unsafe { CGEventGetIntegerValueField(cg, SOURCE_UNIX_PROCESS_ID) },
        source_state: unsafe { CGEventGetIntegerValueField(cg, SOURCE_STATE_ID) },
    })
}

/// winit folds Cocoa momentum into ordinary scroll phases. Read the active
/// scroll event while dispatching it, without installing a global input tap.
pub fn scroll_source() -> (bool, bool, bool) {
    #[cfg(target_os = "macos")]
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        use objc2_app_kit::{NSApplication, NSEventType};
        if let Some(e) = NSApplication::sharedApplication(mtm).currentEvent() {
            if e.r#type() == NSEventType::ScrollWheel {
                return (
                    !e.momentumPhase().is_empty(),
                    objc2::available!(macos = 27.0)
                        && screen_emulated_scroll(
                            Some(ScrollObservation {
                                precise: e.hasPreciseScrollingDeltas(),
                                signature: scroll_evidence(&e),
                            }),
                            std::process::id(),
                        ),
                    e.isDirectionInvertedFromDevice(),
                );
            }
        }
    }
    (false, false, true)
}

#[cfg(test)]
mod scroll_tests {
    use super::*;

    #[test]
    fn only_the_recorded_emulation_signature_selects_touchscreen_navigation() {
        let pid = std::process::id();
        let screen = ScrollEvidence {
            device_id: 0,
            subtype: 1,
            source_pid: pid.into(),
            source_state: 0,
        };
        let precise_scroll = |signature| {
            Some(ScrollObservation {
                precise: true,
                signature,
            })
        };
        assert!(screen_emulated_scroll(precise_scroll(Some(screen)), pid));
        // Keep every other field screen-like to isolate the measured trackpad ID.
        let trackpad = ScrollEvidence {
            device_id: 504403158265495724,
            ..screen
        };
        assert!(!screen_emulated_scroll(precise_scroll(Some(trackpad)), pid));
        // Magic Mouse: the only observed markers were phased, precise deltas.
        // Neither supplies the emulation signature.
        assert!(!screen_emulated_scroll(precise_scroll(None), pid));
        // No NSApp.currentEvent at dispatch.
        assert!(!screen_emulated_scroll(None, pid));
        for other in [
            ScrollEvidence {
                device_id: 1,
                ..screen
            },
            ScrollEvidence {
                subtype: 0,
                ..screen
            },
            ScrollEvidence {
                source_pid: 0,
                ..screen
            },
            ScrollEvidence {
                source_state: 1,
                ..screen
            },
        ] {
            assert!(!screen_emulated_scroll(precise_scroll(Some(other)), pid));
        }
    }
}
