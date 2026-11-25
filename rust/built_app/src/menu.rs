//! The macOS menu bar (muda).
//!
//! Why muda and not only the egui top bar: the Mac expects the application
//! menu (About, Hide, Quit with ⌘Q), and standard key equivalents (⌘O,
//! ⌘S, ⌘Z, ⌘⇧Z, ⌘W, ⌘C/⌘V) are dispatched by AppKit before a key reaches
//! the view, so a value typed into the measurements box never collides
//! with them. The egui chrome stays the place for tools and panels; the
//! tools' single-letter shortcuts are not menu accelerators (Qt's
//! ShortcutOverride order, see ingetrazo_render.Host.key).
//!
//! Each item's id is the command string the host understands
//! (`ingetrazo_render.Host.menu`), or one the shell handles itself (`close`,
//! `quit`: both ask about unsaved work first). Quit is our own item, not
//! muda's predefined one: that one calls `[NSApp terminate:]`, which ends
//! the process without asking.

use muda::{
    accelerator::Accelerator, CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu,
};

pub const EMPTY_RECENT: &str = "(no recent documents)";

pub struct Menus {
    // Kept alive for the life of the app: dropping a muda menu removes it.
    _bar: Menu,
    recent: Submenu,
    plugins: Submenu,
    fixed_plugin_items: usize,
    model_checks: Vec<CheckMenuItem>,
}

type InstalledIds = std::collections::HashMap<(String, String), String>;

thread_local! { static COMMAND_IDS: std::cell::RefCell<InstalledIds> = Default::default(); }

pub fn command_id(path: &str, label: &str) -> Option<String> {
    COMMAND_IDS.with(|ids| ids.borrow().get(&(path.into(), label.into())).cloned())
}

fn index_commands(rows: Vec<muda::MenuItemKind>, trail: &mut Vec<String>, ids: &mut InstalledIds) {
    for row in rows {
        match row {
            muda::MenuItemKind::Submenu(sub) => {
                trail.push(sub.text());
                index_commands(sub.items(), trail, ids);
                trail.pop();
            }
            muda::MenuItemKind::MenuItem(item) => {
                let id = &item.id().0;
                if !id.starts_with("recent:") || id == "recent:clear" {
                    ids.insert((trail.join(" ▸ "), item.text()), id.clone());
                }
            }
            muda::MenuItemKind::Check(item) => {
                if !item.id().0.starts_with("extension:") {
                    ids.insert((trail.join(" ▸ "), item.text()), item.id().0.clone());
                }
            }
            _ => {}
        }
    }
}

fn index_bar(bar: &Menu) {
    let mut ids = InstalledIds::new();
    index_commands(bar.items(), &mut Vec::new(), &mut ids);
    COMMAND_IDS.with(|slot| slot.replace(ids));
}

fn item(id: &str, text: &str, accel: Option<&str>) -> MenuItem {
    let accel = accel.and_then(|a| a.parse::<Accelerator>().ok());
    MenuItem::with_id(id, text, true, accel)
}

/// The File menu's imports: (command, label). The command is
/// `import:<suffix>` (app_shell.imports).
pub use ingetrazo_render::desktop_menu_model::IMPORTS;

pub fn build() -> muda::Result<Menus> {
    use ingetrazo_render::desktop_menu_model::{Row, GROUPS};
    let bar = Menu::new();
    let recent = Submenu::with_items("Open Recent", true, &[&MenuItem::new(EMPTY_RECENT, false, None)])?;
    let import = Submenu::new("Import", true);
    for &(id, label) in IMPORTS {
        import.append(&item(id, label, None))?;
    }
    let mut groups = std::collections::HashMap::from([("File ▸ Import", import.clone())]);
    for &(name, rows) in GROUPS {
        let sub = Submenu::new(name, true);
        for row in rows {
            match *row {
                Row::Command(id, label, accel) => sub.append(&item(id, label, accel))?,
                Row::Recent => sub.append(&recent)?,
                Row::Imports => sub.append(&import)?,
                Row::Separator => sub.append(&PredefinedMenuItem::separator())?,
                Row::System(role) => {
                    let item = match role {
                        "services" => PredefinedMenuItem::services(None),
                        "hide" => PredefinedMenuItem::hide(None),
                        "hide_others" => PredefinedMenuItem::hide_others(None),
                        "show_all" => PredefinedMenuItem::show_all(None),
                        "fullscreen" => PredefinedMenuItem::fullscreen(None),
                        "minimize" => PredefinedMenuItem::minimize(None),
                        "maximize" => PredefinedMenuItem::maximize(None),
                        _ => unreachable!("unknown system menu role {role}"),
                    };
                    sub.append(&item)?;
                }
            }
        }
        groups.insert(name, sub);
    }
    let plugins = groups["Plugins"].clone();
    let window = groups["Window"].clone();
    let mut model_checks = Vec::new();
    for entry in ingetrazo_render::model_menu::entries() {
        if let Some((parent, label)) = entry.path.split_once(" ▸ ") {
            if !groups.contains_key(entry.path) {
                let sub = Submenu::new(label, true);
                groups[parent].append(&sub)?;
                groups.insert(entry.path, sub);
            }
        }
        if matches!(entry.id,"scene_play"|"scene_transitions"|"hide_rest"|"hide_similar"|"rest_normal"|"rest_fade"|"rest_hide") {
            let check=CheckMenuItem::with_id(entry.id,entry.label,true,false,None);
            groups[entry.path].append(&check)?;
            model_checks.push(check);
        } else {
            groups[entry.path].append(&item(entry.id, entry.label, None))?;
        }
    }
    for &(name, _) in GROUPS { bar.append(&groups[name])?; }
    #[cfg(target_os = "macos")]
    {
        bar.init_for_nsapp();
        window.set_as_windows_menu_for_nsapp();
    }
    index_bar(&bar);
    Ok(Menus { _bar: bar, recent, model_checks, fixed_plugin_items: plugins.items().len(), plugins })
}

impl Menus {
    pub fn reflect_model_checks(&self, rest: &str, transitions: bool, similar: bool, playing: bool) {
        for item in &self.model_checks {
            let on=match item.id().0.as_str(){
                "scene_transitions"=>transitions,
                "scene_play"=>playing,
                "hide_similar"=>similar,
                "hide_rest"=>rest=="hide",
                id=>id.strip_prefix("rest_")==Some(rest),
            };
            if on!=item.is_checked(){ item.set_checked(on); }
        }
    }

    /// Rebuild only the contributions below the two built-in utilities.
    pub fn set_extensions(&self, rows: &[ingetrazo_render::extension_menu::Row]) -> muda::Result<()> {
        while self.plugins.items().len() > self.fixed_plugin_items {
            self.plugins.remove_at(self.fixed_plugin_items);
        }
        let mut groups = std::collections::HashMap::new();
        for (id, path, label, key, mark, enabled) in rows {
            let mut parent = self.plugins.clone();
            for depth in 1..=path.len() {
                let prefix=&path[..depth];
                if let Some(sub) = groups.get(prefix) {
                    parent = Submenu::clone(sub);
                } else {
                    let sub = Submenu::new(&path[depth-1], true);
                    parent.append(&sub)?;
                    groups.insert(prefix.to_vec(), sub.clone());
                    parent = sub;
                }
            }
            let accelerator = extension_accelerator(key);
            if label.is_empty() {
                parent.append(&PredefinedMenuItem::separator())?;
            } else if *mark >= 0 {
                parent.append(&CheckMenuItem::with_id(id.as_str(), label, *enabled, *mark == 1, accelerator))?;
            } else {
                parent.append(&MenuItem::with_id(id.as_str(), label, *enabled, accelerator))?;
            }
        }
        Ok(())
    }

    /// File ▸ Open Recent from the host's list (paths, newest first):
    /// `recent:<index>` items, then Clear.
    pub fn set_recent(&self, paths: &[String]) {
        for _ in 0..self.recent.items().len() {
            let _ = self.recent.remove_at(0);
        }
        if paths.is_empty() {
            let _ = self.recent.append(&MenuItem::new(EMPTY_RECENT, false, None));
            return;
        }
        for (i, p) in paths.iter().enumerate() {
            let name = std::path::Path::new(p).file_name().map_or(p.clone(), |n| n.to_string_lossy().to_string());
            let parent = std::path::Path::new(p).parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
            let _ = self.recent.append(&item(&format!("recent:{i}"), &format!("{name}  —  {parent}"), None));
        }
        let _ = self.recent.append(&PredefinedMenuItem::separator());
        let _ = self.recent.append(&item("recent:clear", "Clear Menu", None));
        index_bar(&self._bar);
    }
}

/// Bare keys belong to the focused text field or the active modelling tool.
fn extension_accelerator(portable: &str) -> Option<Accelerator> {
    let mut parts: Vec<_> = portable.split('+').collect();
    let key = parts.pop()?;
    if !parts.iter().any(|m| matches!(*m, "Ctrl" | "Meta" | "Alt")) { return None; }
    for modifier in &mut parts {
        *modifier = match *modifier { "Ctrl" => "Super", "Meta" => "Control", s => s };
    }
    parts.push(key);
    parts.join("+").parse().ok()
}
