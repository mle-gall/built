//! The macOS menu bar (muda).
//!
//! Why muda and not only the egui top bar: the Mac expects the application
//! menu (About, Hide, Quit with ⌘Q), and standard key equivalents (⌘O,
//! ⌘S, ⌘Z, ⌘⇧Z, ⌘W, ⌘C/⌘V) are dispatched by AppKit before a key reaches
//! the view, so a value typed into the measurements box never collides
//! with them. The egui chrome stays the place for tools and panels; the
//! tools' single-letter shortcuts are not menu accelerators (Qt's
//! ShortcutOverride order, see app_shell.host.Host.key).
//!
//! Each item's id is the command string the host understands
//! (`app_shell.host.Host.menu`), or one the shell handles itself (`close`,
//! `quit`: both ask about unsaved work first). Quit is our own item, not
//! muda's predefined one: that one calls `[NSApp terminate:]`, which ends
//! the process without asking.

use muda::{
    accelerator::Accelerator, Menu, MenuItem, PredefinedMenuItem, Submenu,
};

pub struct Menus {
    // Kept alive for the life of the app: dropping a muda menu removes it.
    _bar: Menu,
    recent: Submenu,
}

fn item(id: &str, text: &str, accel: Option<&str>) -> MenuItem {
    let accel = accel.and_then(|a| a.parse::<Accelerator>().ok());
    MenuItem::with_id(id, text, true, accel)
}

/// The File menu's imports: (command, label). The command is
/// `import:<suffix>` (app_shell.imports).
pub const IMPORTS: &[(&str, &str)] = &[
    ("import:skp", ".skp…"),
    ("import:dae", "COLLADA (.dae)…"),
    ("import:dxf", "DXF (.dxf)…"),
    ("import:dwg", "DWG (.dwg)…"),
    ("import:rfa", "Revit family (.rfa)…"),
    ("import:glb", "glTF (.glb, .gltf)…"),
    ("import:igz", "Built or IngeTrazo model, as a group…"),
];

pub fn build() -> muda::Result<Menus> {
    let bar = Menu::new();
    let app = Submenu::with_items(
        "Built",
        true,
        &[
            &item("about", "About Built", None),
            &PredefinedMenuItem::separator(),
            &item("prefs", "Settings…", Some("super+Comma")),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &item("quit", "Quit Built", Some("super+KeyQ")),
        ],
    )?;
    let recent = Submenu::with_items("Open Recent", true, &[&MenuItem::new("(no recent documents)", false, None)])?;
    let imports: Vec<MenuItem> = IMPORTS.iter().map(|(id, label)| item(id, label, None)).collect();
    let import_refs: Vec<&dyn muda::IsMenuItem> = imports.iter().map(|i| i as &dyn muda::IsMenuItem).collect();
    let import = Submenu::with_items("Import", true, &import_refs)?;
    let file = Submenu::with_items(
        "File",
        true,
        &[&item("new", "New", Some("super+KeyN")),
          &item("new_window", "New Window", Some("super+shift+KeyN")),
          &item("open", "Open…", Some("super+KeyO")),
          &recent,
          &item("home", "Start Page", None),
          &PredefinedMenuItem::separator(),
          &item("close_tab", "Close Project", Some("super+KeyW")),
          &item("close", "Close Window", Some("super+shift+KeyW")),
          &PredefinedMenuItem::separator(),
          &item("save", "Save", Some("super+KeyS")),
          &item("save_as", "Save As…", Some("super+shift+KeyS")),
          &item("save_copy", "Save a Copy…", Some("super+alt+KeyS")),
          &item("export_image", "Export Image…", None),
          &item("delivery_export", "Export…", None),
          &item("export_igz", "Save a Copy as IngeTrazo (.igz)…", None),
          &PredefinedMenuItem::separator(),
          &import,
          &item("link", "Link a Document…", None),
          &item("reload_links", "Reload Linked Files", None),
          &PredefinedMenuItem::separator(),
          &item("classic", "Open in Classic App", None)],
    )?;
    let edit = Submenu::with_items(
        "Edit",
        true,
        &[&item("undo", "Undo", Some("super+KeyZ")),
          &item("redo", "Redo", Some("super+shift+KeyZ")),
          &PredefinedMenuItem::separator(),
          &item("cut", "Cut", Some("super+KeyX")),
          &item("copy", "Copy", Some("super+KeyC")),
          &item("paste", "Paste", Some("super+KeyV")),
          &item("paste_in_place", "Paste in Place", Some("super+alt+KeyV")),
          &PredefinedMenuItem::separator(),
          &item("soften", "Soften / Smooth Edges…", None),
          &item("select_all", "Select All", Some("super+KeyA")),
          &item("select_none", "Select None", Some("super+KeyT"))],
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
        &[&PredefinedMenuItem::minimize(None), &PredefinedMenuItem::maximize(None),
          &PredefinedMenuItem::separator(),
          &item("next_doc", "Next Project", Some("ctrl+Tab")),
          &item("prev_doc", "Previous Project", Some("ctrl+shift+Tab")),
          &PredefinedMenuItem::separator(),
          &item("move_to_window", "Move Project to New Window", None),
          &PredefinedMenuItem::separator(),
          &item("extensions", "Extension Manager…", None),
          &item("cutlist", "Cut list…", None),
          &item("library", "Library", Some("super+shift+KeyL"))],
    )?;
    let plugins = Submenu::with_items("Plugins", true, &[
        &item("panel:python_console", "Python Console", Some("CmdOrCtrl+Shift+P")),
        &item("panel:render_blender", "Blender Render", None),
    ])?;
    let help = Submenu::with_items("Help", true, &[&item("check_updates", "Check for Updates…", None)])?;
    bar.append_items(&[&app, &file, &edit, &view, &tools, &plugins, &window, &help])?;
    #[cfg(target_os = "macos")]
    {
        bar.init_for_nsapp();
        window.set_as_windows_menu_for_nsapp();
    }
    Ok(Menus { _bar: bar, recent })
}

impl Menus {
    /// File ▸ Open Recent from the host's list (paths, newest first):
    /// `recent:<index>` items, then Clear.
    pub fn set_recent(&self, paths: &[String]) {
        for _ in 0..self.recent.items().len() {
            let _ = self.recent.remove_at(0);
        }
        if paths.is_empty() {
            let _ = self.recent.append(&MenuItem::new("(no recent documents)", false, None));
            return;
        }
        for (i, p) in paths.iter().enumerate() {
            let name = std::path::Path::new(p).file_name().map_or(p.clone(), |n| n.to_string_lossy().to_string());
            let parent = std::path::Path::new(p).parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
            let _ = self.recent.append(&item(&format!("recent:{i}"), &format!("{name}  —  {parent}"), None));
        }
        let _ = self.recent.append(&PredefinedMenuItem::separator());
        let _ = self.recent.append(&item("recent:clear", "Clear Menu", None));
    }
}
