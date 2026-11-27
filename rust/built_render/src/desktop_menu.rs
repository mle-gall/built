//! In-window application menus for the X11, Wayland and Win32 shells.
use crate::{
    desktop_menu_model::{self, Row, EXTENSIONS_MENU, GROUPS, IMPORTS},
    extension_menu, model_menu,
};
use egui::{Rect, Ui};
use std::cell::RefCell;

type Checks = (String, bool, bool, bool, bool);
#[derive(Default)]
struct Desktop {
    dispatch: Option<Box<dyn Fn(String)>>,
    recent: Vec<String>,
    checks: Option<Checks>,
    shortcuts: std::collections::HashMap<String, String>,
}
type Buttons = Vec<(String, (f32, f32))>;
thread_local! { static BUTTONS: RefCell<Buttons> = RefCell::default(); }
pub fn take_buttons() -> Buttons {
    BUTTONS.take()
}
fn remember(id: String, rect: Rect) {
    BUTTONS.with_borrow_mut(|b| b.push((id, rect.center().into())));
}
thread_local! { static DESKTOP: RefCell<Desktop> = RefCell::default(); }

pub fn install(dispatch: impl Fn(String) + 'static) {
    DESKTOP.with_borrow_mut(|state| state.dispatch = Some(Box::new(dispatch)));
}
pub fn update(recent: Option<Vec<String>>, checks: Option<Checks>) {
    DESKTOP.with_borrow_mut(|state| {
        if let Some(paths) = recent {
            state.recent = paths;
        }
        state.checks = checks;
    });
}

fn item(
    ui: &mut Ui,
    id: &str,
    label: &str,
    key: &str,
    enabled: bool,
    check: Option<bool>,
    selected: &mut Option<String>,
) {
    let custom = DESKTOP.with_borrow(|s| s.shortcuts.get(id).cloned());
    let key = custom
        .as_deref()
        .unwrap_or(key)
        .replace("super+", "Ctrl+")
        .replace("CmdOrCtrl+", "Ctrl+")
        .replace("Key", "")
        .replace("shift+", "Shift+")
        .replace("alt+", "Alt+")
        .replace("ctrl+", "Ctrl+");
    let label = if check == Some(true) {
        format!("✓  {label}")
    } else {
        label.into()
    };
    let button = ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(key));
    remember(format!("app-item:{id}"), button.rect);
    if button.clicked() {
        *selected = Some(id.to_owned());
        ui.close_menu();
    }
}
fn entry_check(state: &Desktop, id: &str) -> Option<bool> {
    if !model_menu::CHECKS.contains(&id) {
        return None;
    }
    state.checks.as_ref().and_then(|(rest, transitions, similar, playing, _)| {
        Some(match id {
            "scene_transitions" => *transitions,
            "scene_play" => *playing,
            "hide_similar" => *similar,
            id => id.strip_prefix("rest_")? == rest,
        })
    })
}
/// The model-menu rows of `path` in order; at the top level of a menu only
/// what `GROUPS` does not place itself.
fn model_rows(
    ui: &mut Ui,
    path: &str,
    state: &Desktop,
    selected: &mut Option<String>,
    hits: &mut Vec<Rect>,
) {
    let prefix = format!("{path} ▸ ");
    let mut groups = Vec::new();
    let mut first = true;
    for entry in model_menu::entries() {
        if entry.path == path {
            if desktop_menu_model::placed(entry.id) && !path.contains(" ▸ ") {
                continue;
            }
            if entry.starts_group() && !first {
                ui.separator();
            }
            first = false;
            item(ui, entry.id, entry.label, entry.shortcut, true, entry_check(state, entry.id), selected);
        } else if let Some(sub) = entry.path.strip_prefix(&prefix) {
            let sub = sub.split(" ▸ ").next().unwrap();
            let full = format!("{prefix}{sub}");
            if (path == "File" && sub == "Import") || groups.contains(&sub) || desktop_menu_model::placed(&full) {
                continue;
            }
            groups.push(sub);
            first = false;
            ui.menu_button(sub, |ui| {
                model_rows(ui, &full, state, selected, hits);
                hits.push(ui.min_rect());
            });
        }
    }
}
fn plugins(
    ui: &mut Ui,
    rows: &[extension_menu::Row],
    path: &mut Vec<String>,
    selected: &mut Option<String>,
    hits: &mut Vec<Rect>,
) {
    let mut shown = Vec::new();
    for (id, parent, label, key, mark, enabled) in rows {
        if !parent.starts_with(path) {
            continue;
        }
        if let Some(name) = parent.get(path.len()) {
            if shown.contains(&name) {
                continue;
            }
            shown.push(name);
            ui.menu_button(name, |ui| {
                path.push(name.clone());
                plugins(ui, rows, path, selected, hits);
                path.pop();
                hits.push(ui.min_rect());
            });
        } else if label.is_empty() {
            ui.separator();
        } else {
            item(
                ui,
                id,
                label,
                key,
                *enabled,
                (*mark >= 0).then_some(*mark == 1),
                selected,
            );
        }
    }
}

pub(crate) fn show(ui: &mut Ui, extensions: &[extension_menu::Row], enabled: bool) -> Vec<Rect> {
    BUTTONS.with_borrow_mut(Vec::clear);
    let mut hits = Vec::new();
    let mut selected = None;
    DESKTOP.with_borrow(|state| {
        ui.add_enabled_ui(enabled, |ui| {
            egui::menu::bar(ui, |ui| {
                for &(group, rows) in GROUPS {
                    let group_button = ui.menu_button(group, |ui| {
                        let mut separator = false;
                        for &row in rows {
                            if matches!(row, Row::Separator) {
                                separator = true;
                                continue;
                            }
                            let system = match row {
                                Row::System("maximize") => {
                                    Some(("window:maximize", "Maximize / Restore"))
                                }
                                Row::System("minimize") => Some(("window:minimize", "Minimize")),
                                Row::System("fullscreen") => {
                                    Some(("window:fullscreen", "Full Screen"))
                                }
                                Row::System(_) => continue,
                                _ => None,
                            };
                            if separator {
                                ui.separator();
                                separator = false;
                            }
                            match row {
                                Row::Command(id, label, key) => item(
                                    ui,
                                    id,
                                    label,
                                    key.unwrap_or(""),
                                    true,
                                    None,
                                    &mut selected,
                                ),
                                Row::Entry(id) => {
                                    if let Some(e) = model_menu::entries().iter().find(|e| e.id == id) {
                                        item(ui, e.id, e.label, e.shortcut, id != "weld_edges" || state.checks.as_ref().is_some_and(|c| c.4), entry_check(state, id), &mut selected);
                                    }
                                }
                                Row::Sub(title) => {
                                    ui.menu_button(title, |ui| {
                                        model_rows(ui, &format!("{group} ▸ {title}"), state, &mut selected, &mut hits);
                                        hits.push(ui.min_rect());
                                    });
                                }
                                Row::System(_) => {
                                    if let Some((id, label)) = system {
                                        item(ui, id, label, "", true, None, &mut selected);
                                    }
                                }
                                Row::Recent => {
                                    ui.menu_button("Open Recent", |ui| {
                                        if state.recent.is_empty() {
                                            ui.weak("(no recent documents)");
                                        }
                                        for (i, path) in state.recent.iter().enumerate() {
                                            item(
                                                ui,
                                                &format!("recent:{i}"),
                                                path,
                                                "",
                                                true,
                                                None,
                                                &mut selected,
                                            );
                                        }
                                        if !state.recent.is_empty() {
                                            ui.separator();
                                            item(
                                                ui,
                                                "recent:clear",
                                                "Clear Menu",
                                                "",
                                                true,
                                                None,
                                                &mut selected,
                                            );
                                        }
                                        hits.push(ui.min_rect());
                                    });
                                }
                                Row::Imports => {
                                    ui.menu_button("Import", |ui| {
                                        for &(id, label) in IMPORTS {
                                            item(ui, id, label, "", true, None, &mut selected);
                                        }
                                        model_rows(
                                            ui,
                                            "File ▸ Import",
                                            state,
                                            &mut selected,
                                            &mut hits,
                                        );
                                        hits.push(ui.min_rect());
                                    });
                                }
                                Row::Separator => {}
                            }
                        }
                        model_rows(ui, group, state, &mut selected, &mut hits);
                        if group == EXTENSIONS_MENU {
                            plugins(ui, extensions, &mut Vec::new(), &mut selected, &mut hits);
                        }
                        hits.push(ui.min_rect());
                    });
                    remember(format!("app-menu:{group}"), group_button.response.rect);
                }
            })
        });
        if let (Some(id), Some(dispatch)) = (selected, &state.dispatch) {
            dispatch(id);
        }
    });
    hits
}

#[pyo3::pyclass]
pub struct Command {
    #[pyo3(get)]
    id: String,
    #[pyo3(get)]
    path: String,
    #[pyo3(get)]
    name: String,
    #[pyo3(get)]
    keys: String,
}
#[pyo3::pymethods]
impl Command {
    fn trigger(&self) {
        DESKTOP.with_borrow(|s| {
            if let Some(dispatch) = &s.dispatch {
                dispatch(self.id.clone());
            }
        });
    }
    fn visible(&self) -> bool {
        true
    }
    #[pyo3(name = "isEnabled")]
    fn enabled(&self) -> bool {
        true
    }
    fn set_shortcut(&self, key: &str, factory: bool) {
        DESKTOP.with_borrow_mut(|s| {
            if factory {
                s.shortcuts.remove(&self.id);
            } else {
                s.shortcuts.insert(self.id.clone(), key.into());
            }
        });
    }
}

#[pyo3::pyfunction]
pub fn menu_commands() -> Vec<Command> {
    let mut result = Vec::new();
    let mut push = |id: &str, path: &str, name: &str, keys: &str| {
        let keys = keys
            .replace("super+", "Ctrl+")
            .replace("CmdOrCtrl+", "Ctrl+")
            .replace("Key", "")
            .replace("shift+", "Shift+")
            .replace("alt+", "Alt+")
            .replace("ctrl+", "Ctrl+")
            .replace("Comma", ",");
        result.push(Command {
            id: id.into(),
            path: path.into(),
            name: name.into(),
            keys,
        });
    };
    for &(path, rows) in GROUPS {
        for row in rows {
            match *row {
                Row::Command(id, name, key) => push(id, path, name, key.unwrap_or("")),
                Row::Imports => {
                    for &(id, name) in IMPORTS {
                        push(id, "File ▸ Import", name, "");
                    }
                }
                _ => {}
            }
        }
    }
    for e in model_menu::entries() {
        push(e.id, e.path, e.label, e.shortcut);
    }
    result
}
