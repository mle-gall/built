//! The macOS menu bar (muda).
//!
//! Why muda and not only the egui top bar: the Mac expects the application
//! menu (About, Hide, Quit with ⌘Q), and standard key equivalents (⌘O,
//! ⌘Z, ⌘⇧Z, ⌘W) are dispatched by AppKit before a key reaches the view, so a
//! value typed into the measurements box never collides with them. The egui
//! chrome stays the place for tools and panels; the tools' single-letter
//! shortcuts are not menu accelerators (Qt's ShortcutOverride order, see
//! app_shell.host.Host.key).
//!
//! Each item's id is the command string the host understands
//! (`app_shell.host.Host.menu`), or one the shell handles itself (`open`,
//! `close`).

use muda::{
    accelerator::Accelerator, AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu,
};

pub struct Menus {
    // Kept alive for the life of the app: dropping a muda menu removes it.
    _bar: Menu,
}

fn item(id: &str, text: &str, accel: Option<&str>) -> MenuItem {
    let accel = accel.and_then(|a| a.parse::<Accelerator>().ok());
    MenuItem::with_id(id, text, true, accel)
}

pub fn build() -> muda::Result<Menus> {
    let bar = Menu::new();
    let app = Submenu::with_items(
        "Built",
        true,
        &[
            &PredefinedMenuItem::about(
                Some("About Built"),
                Some(AboutMetadata {
                    name: Some("Built".into()),
                    comments: Some("Rust app shell (R1.1)".into()),
                    ..Default::default()
                }),
            ),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ],
    )?;
    let file = Submenu::with_items(
        "File",
        true,
        &[&item("open", "Open…", Some("super+KeyO")), &PredefinedMenuItem::separator(),
          &item("close", "Close Window", Some("super+KeyW"))],
    )?;
    let edit = Submenu::with_items(
        "Edit",
        true,
        &[&item("undo", "Undo", Some("super+KeyZ")),
          &item("redo", "Redo", Some("super+shift+KeyZ"))],
    )?;
    let view = Submenu::with_items(
        "Camera",
        true,
        &[&item("zoom_extents", "Zoom Extents", Some("super+shift+KeyE")),
          &PredefinedMenuItem::separator(),
          &item("view:iso", "Iso", None), &item("view:top", "Top", None),
          &item("view:front", "Front", None), &item("view:right", "Right", None),
          &PredefinedMenuItem::separator(),
          &PredefinedMenuItem::fullscreen(None)],
    )?;
    let tools = Submenu::with_items(
        "Tools",
        true,
        &[&item("tool:select", "Select", None), &item("tool:line", "Line", None),
          &item("tool:rectangle", "Rectangle", None), &item("tool:circle", "Circle", None),
          &item("tool:pushpull", "Push/Pull", None), &item("tool:move", "Move", None),
          &item("tool:rotate", "Rotate", None), &item("tool:offset", "Offset", None),
          &item("tool:tape", "Tape Measure", None), &item("tool:eraser", "Eraser", None),
          &item("tool:paint", "Paint", None)],
    )?;
    let window = Submenu::with_items(
        "Window",
        true,
        &[&PredefinedMenuItem::minimize(None), &PredefinedMenuItem::maximize(None)],
    )?;
    bar.append_items(&[&app, &file, &edit, &view, &tools, &window])?;
    #[cfg(target_os = "macos")]
    {
        bar.init_for_nsapp();
        window.set_as_windows_menu_for_nsapp();
    }
    Ok(Menus { _bar: bar })
}
