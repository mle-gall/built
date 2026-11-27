//! Desktop application menus in display order. Both the native AppKit menu
//! and the egui menu use these command IDs and factory accelerators.
//!
//! `GROUPS` orders every top-level menu: plain rows (`Row::Command`), the
//! commands of [`crate::model_menu`] placed by id (`Row::Entry`) and their
//! submenus placed by title (`Row::Sub`). The rules (one home per command,
//! where each kind goes, label case) are in docs/menus.md.
pub const IMPORTS: &[(&str, &str)] = &[
    ("import:skp", "SKP Model (.skp)…"),
    ("import:dae", "COLLADA (.dae)…"),
    ("import:dxf", "DXF Drawing (.dxf)…"),
    ("import:dwg", "DWG Drawing (.dwg)…"),
    ("import:rfa", "RFA Family (.rfa)…"),
    ("import:glb", "glTF (.glb, .gltf)…"),
    ("import:igz", "Built Model as a Group (.built, .igz)…"),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Row {
    Command(&'static str, &'static str, Option<&'static str>),
    /// A [`crate::model_menu`] command, by id, at this place.
    Entry(&'static str),
    /// A [`crate::model_menu`] submenu, by title, at this place.
    Sub(&'static str),
    Separator,
    System(&'static str),
    Recent,
    Imports,
}

/// The menu bar's titles in order. The extensions' menu is the one whose
/// tail the installed extensions fill.
pub const EXTENSIONS_MENU: &str = "Extensions";

pub const GROUPS: &[(&str, &[Row])] = &[
    (
        "Built",
        &[
            Row::Command("about", "About Built", None),
            Row::Separator,
            Row::Command("prefs", "Settings…", Some("super+Comma")),
            Row::Separator,
            Row::System("services"),
            Row::Separator,
            Row::System("hide"),
            Row::System("hide_others"),
            Row::System("show_all"),
            Row::Separator,
            Row::Command("quit", "Quit Built", Some("super+KeyQ")),
        ],
    ),
    (
        "File",
        &[
            Row::Command("new", "New Model", Some("super+KeyN")),
            Row::Command("new_project", "New Project…", None),
            Row::Command("new_window", "New Window", Some("super+shift+KeyN")),
            Row::Command("open", "Open…", Some("super+KeyO")),
            Row::Recent,
            Row::Command("home", "Start Page", None),
            Row::Separator,
            Row::Command("close_tab", "Close Model", Some("super+KeyW")),
            Row::Command("close", "Close Window", Some("super+shift+KeyW")),
            Row::Separator,
            Row::Command("save", "Save", Some("super+KeyS")),
            Row::Command("save_as", "Save As…", Some("super+shift+KeyS")),
            Row::Command("save_copy", "Save a Copy…", Some("super+alt+KeyS")),
            Row::Entry("recover_discarded"),
            Row::Separator,
            Row::Imports,
            Row::Command("link", "Link a Document…", None),
            Row::Command("reload_links", "Reload Linked Files", None),
            Row::Separator,
            Row::Sub("Export"),
        ],
    ),
    (
        "Edit",
        &[
            Row::Command("undo", "Undo", Some("super+KeyZ")),
            Row::Command("redo", "Redo", Some("super+shift+KeyZ")),
            Row::Entry("repeat_last_command"),
            Row::Separator,
            Row::Command("cut", "Cut", Some("super+KeyX")),
            Row::Command("copy", "Copy", Some("super+KeyC")),
            Row::Command("paste", "Paste", Some("super+KeyV")),
            Row::Command("paste_in_place", "Paste in Place", Some("super+alt+KeyV")),
            Row::Separator,
            Row::Command("select_all", "Select All", Some("super+KeyA")),
            Row::Command("select_none", "Select None", Some("super+KeyT")),
            Row::Entry("invert_selection"),
            Row::Separator,
            Row::Entry("make_group"),
            Row::Entry("verb:make_component"),
            Row::Entry("explode_group"),
            Row::Entry("split_pieces"),
            Row::Separator,
            Row::Entry("hide"),
            Row::Sub("Unhide"),
            Row::Entry("delete_guides"),
            Row::Separator,
            Row::Sub("Intersect Faces"),
            Row::Command("soften", "Soften / Smooth Edges…", None),
            Row::Entry("reverse_faces"),
            Row::Entry("orient_faces"),
            Row::Entry("heal_overlaps"),
            Row::Entry("rebuild_planar"),
            Row::Entry("weld_edges"),
            Row::Entry("verb:simplify_mesh"),
            Row::Entry("convert_path"),
        ],
    ),
    (
        "View",
        &[
            Row::Entry("orbit"),
            Row::Entry("pan"),
            Row::Entry("zoom"),
            Row::Entry("zoom_window"),
            Row::Command("zoom_extents", "Zoom Extents", Some("super+shift+KeyE")),
            Row::Entry("zoom_selection"),
            Row::Entry("look_at_face"),
            Row::Separator,
            Row::Sub("Standard Views"),
            Row::Sub("Projection"),
            Row::Entry("verb:field_of_view"),
            Row::Entry("camera"),
            Row::Sub("Walkthrough"),
            Row::Separator,
            Row::Sub("Style"),
            Row::Entry("xray"),
            Row::Entry("axes"),
            Row::Entry("back_edges"),
            Row::Entry("hidden_geometry"),
            Row::Entry("shadows"),
            Row::Entry("form-shadows"),
            Row::Sub("Sections"),
            Row::Separator,
            Row::Sub("Scenes"),
            Row::Sub("Animation"),
            Row::Sub("Rest of Model While Editing"),
            Row::Separator,
            Row::Sub("Model Views"),
            Row::Entry("show_keystrokes"),
            Row::Separator,
            Row::System("fullscreen"),
        ],
    ),
    (
        "Model",
        &[
            Row::Command("site_placement", "Site Placement…", None),
            Row::Command("units", "Units…", Some("super+KeyU")),
            Row::Command("cutlist", "Cut List…", None),
        ],
    ),
    (
        "Draw",
        &[
            Row::Entry("tool:line"),
            Row::Entry("tool:line_on_surface"),
            Row::Entry("tool:sweep"),
            Row::Entry("tool:cut"),
            Row::Entry("tool:freehand"),
            Row::Entry("tool:geopath"),
            Row::Separator,
            Row::Entry("tool:rectangle"),
            Row::Entry("tool:rotated_rect"),
            Row::Entry("tool:circle"),
            Row::Entry("tool:polygon"),
            Row::Sub("Arcs"),
        ],
    ),
    (
        "Tools",
        &[
            Row::Entry("tool:select"),
            Row::Entry("tool:eraser"),
            Row::Entry("tool:paint"),
            Row::Separator,
            Row::Entry("tool:pushpull"),
            Row::Entry("tool:move"),
            Row::Entry("tool:rotate"),
            Row::Entry("tool:scale"),
            Row::Entry("tool:flip"),
            Row::Entry("tool:followme"),
            Row::Entry("tool:offset"),
            Row::Entry("tool:fillet"),
            Row::Entry("tool:chamfer"),
            Row::Entry("tool:outer_shell"),
            Row::Sub("Solid Tools"),
            Row::Entry("recover_exact"),
            Row::Entry("make_faceted"),
            Row::Entry("bridge"),
            Row::Entry("make_faces"),
            Row::Separator,
            Row::Entry("tool:tape"),
            Row::Entry("tool:protractor"),
            Row::Entry("tool:change_axes"),
            Row::Sub("Drawing Axes"),
            Row::Entry("tool:dimension"),
            Row::Entry("tool:text"),
            Row::Entry("text3d"),
            Row::Entry("tool:section"),
            Row::Entry("tool:north"),
            Row::Entry("terrain_profile"),
            Row::Separator,
            Row::Command("panel:solid_inspector", "Diagnostics", None),
            Row::Command("panel:python_console", "Python Console", Some("CmdOrCtrl+Shift+P")),
            Row::Entry("clear_texture_cache"),
        ],
    ),
    (
        EXTENSIONS_MENU,
        &[
            Row::Command("extensions", "Extension Manager…", None),
            Row::Entry("plugins_folder"),
            Row::Entry("develop_plugin"),
            Row::Separator,
            Row::Command("panel:render_blender", "Blender Render", None),
        ],
    ),
    (
        "Window",
        &[
            Row::System("minimize"),
            Row::System("maximize"),
            Row::Separator,
            Row::Command("next_doc", "Next Model", Some("ctrl+Tab")),
            Row::Command("prev_doc", "Previous Model", Some("ctrl+shift+Tab")),
            Row::Command("move_to_window", "Move Model to New Window", None),
            Row::Separator,
            Row::Entry("sidebar_project"),
            Row::Entry("sidebar_tasks"),
            Row::Entry("sidebar_deliveries"),
            Row::Command("library", "Library", Some("super+shift+KeyL")),
            Row::Separator,
            Row::Entry("recent-activity"),
            Row::Entry("ask_agent"),
            Row::Entry("review"),
        ],
    ),
    (
        "Help",
        &[
            Row::Entry("documentation"),
            Row::Entry("get_models"),
            Row::Separator,
            Row::Command("check_updates", "Check for Updates…", None),
        ],
    ),
];

/// Whether `GROUPS` places this model-menu command (`Row::Entry`) or
/// submenu path (`Row::Sub`, "View ▸ Scenes") itself.
pub fn placed(path_or_id: &str) -> bool {
    GROUPS.iter().any(|(group, rows)| {
        rows.iter().any(|row| match *row {
            Row::Entry(id) => id == path_or_id,
            Row::Sub(title) => path_or_id.strip_prefix(group).and_then(|p| p.strip_prefix(" ▸ ")) == Some(title),
            _ => false,
        })
    })
}

/// One row of the whole menu bar as the user reads it: (menu path, label,
/// command id). Submenus are listed with an empty id; separators and system
/// rows (Services, Hide, Full Screen…) are left out.
pub fn rows() -> Vec<(String, String, String)> {
    use crate::model_menu::{entries, PALETTE_ONLY};
    fn sub_rows(path: &str, out: &mut Vec<(String, String, String)>) {
        let prefix = format!("{path} ▸ ");
        let mut subs: Vec<&str> = Vec::new();
        for entry in entries() {
            if entry.path == path {
                out.push((path.into(), entry.label.into(), entry.id.into()));
            } else if let Some(sub) = entry.path.strip_prefix(&prefix) {
                let sub = sub.split(" ▸ ").next().unwrap();
                if !subs.contains(&sub) {
                    subs.push(sub);
                    out.push((path.into(), sub.into(), String::new()));
                    sub_rows(&format!("{prefix}{sub}"), out);
                }
            }
        }
    }
    let mut out = Vec::new();
    for &(group, rows) in GROUPS {
        for row in rows {
            match *row {
                Row::Command(id, label, _) => out.push((group.into(), label.into(), id.into())),
                Row::Entry(id) => {
                    let entry = entries().iter().find(|e| e.id == id).unwrap_or_else(|| panic!("no model-menu command {id}"));
                    out.push((group.into(), entry.label.into(), id.into()));
                }
                Row::Sub(title) => {
                    out.push((group.into(), title.into(), String::new()));
                    sub_rows(&format!("{group} ▸ {title}"), &mut out);
                }
                Row::Imports => {
                    out.push((group.into(), "Import".into(), String::new()));
                    for &(id, label) in IMPORTS {
                        out.push(("File ▸ Import".into(), label.into(), id.into()));
                    }
                    for entry in entries().iter().filter(|e| e.path == "File ▸ Import") {
                        out.push((entry.path.into(), entry.label.into(), entry.id.into()));
                    }
                }
                Row::Recent => out.push((group.into(), "Open Recent".into(), String::new())),
                Row::Separator | Row::System(_) => {}
            }
        }
    }
    debug_assert!(entries().iter().all(|e| e.path != PALETTE_ONLY || !placed(e.id)));
    out
}

/// Commands deliberately in two top-level menus (docs/menus.md, rule 1).
/// Empty: Deliveries is a destination in Window (`sidebar_deliveries`) and
/// the handover in File ▸ Export (`delivery_export`), two ids by design.
pub const SHARED_COMMANDS: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_menu::{entries, PALETTE_ONLY};
    use std::collections::{HashMap, HashSet};

    fn top(path: &str) -> &str {
        path.split(" ▸ ").next().unwrap()
    }

    #[test]
    fn every_command_has_one_home_in_the_menu_bar() {
        let mut homes: HashMap<String, String> = HashMap::new();
        for (path, label, id) in rows() {
            if id.is_empty() {
                continue;
            }
            if let Some(first) = homes.insert(id.clone(), path.clone()) {
                assert!(
                    SHARED_COMMANDS.contains(&id.as_str()) && top(&first) != top(&path),
                    "{id} ({label}) is in {first} and in {path}"
                );
            }
        }
    }

    #[test]
    fn no_menu_has_two_rows_with_the_same_label() {
        // Within one top-level menu and its submenus, a label names one row:
        // the AppKit index maps (path, label) to a command id.
        let mut seen: HashMap<(String, String), String> = HashMap::new();
        let mut labels: HashMap<(String, String), String> = HashMap::new();
        for (path, label, id) in rows() {
            let key = (path.clone(), label.clone());
            assert!(seen.insert(key, id.clone()).is_none(), "{path} ▸ {label} twice");
            if id.is_empty() {
                continue;
            }
            // Import ▸ and Export ▸ name the same formats on purpose.
            let menu = if path == "File ▸ Import" || path == "File ▸ Export" { path.clone() } else { top(&path).into() };
            if let Some(other) = labels.insert((menu.clone(), label.clone()), id.clone()) {
                panic!("{label}: {other} and {id} in the {menu} menu");
            }
        }
    }

    #[test]
    fn groups_place_every_top_level_command_and_submenu() {
        let groups: HashSet<&str> = GROUPS.iter().map(|g| g.0).collect();
        for entry in entries() {
            if entry.path == PALETTE_ONLY {
                assert!(!placed(entry.id), "{} is palette-only and placed", entry.id);
                continue;
            }
            assert!(groups.contains(top(entry.path)), "{} has no menu {}", entry.id, entry.path);
            match entry.path.split_once(" ▸ ") {
                None => assert!(placed(entry.id), "{} is not placed in {}", entry.id, entry.path),
                Some((group, rest)) if !rest.contains(" ▸ ") && entry.path != "File ▸ Import" => {
                    assert!(placed(entry.path), "submenu {} of {group} is not placed", entry.path);
                }
                _ => {}
            }
        }
        for &(group, rows) in GROUPS {
            for row in rows {
                if let Row::Entry(id) = *row {
                    let entry = entries().iter().find(|e| e.id == id).unwrap();
                    assert_eq!(entry.path, group, "{id} placed in {group}");
                }
            }
        }
    }

    #[test]
    fn file_has_one_export_structure() {
        let file: Vec<_> = rows().into_iter().filter(|r| top(&r.0) == "File").collect();
        let outside: Vec<_> = file
            .iter()
            .filter(|(path, label, _)| path != "File ▸ Export" && label.to_lowercase().contains("export") && label != "Export")
            .collect();
        assert!(outside.is_empty(), "export rows outside File ▸ Export: {outside:?}");
        let export: Vec<&str> = file.iter().filter(|r| r.0 == "File ▸ Export").map(|r| r.2.as_str()).collect();
        assert_eq!(export, [
            "delivery_export", "export:ifc", "export:glb", "export:dae", "export:step", "export:stl", "export:obj",
            "export_image", "export_view_svg", "export:dxf", "export:video", "export_igz",
        ]);
        let label = |id: &str| file.iter().find(|r| r.2 == id).unwrap().1.clone();
        assert_eq!(label("delivery_export"), "Deliveries…");
        assert!(label("export_igz").contains(".igz (IngeTrazo)"));
        // The legacy format is never called Built.
        for (_, label, _) in rows() {
            assert!(!(label.contains("igz") && label.contains("Built") && !label.contains(".built")), "{label}");
        }
    }

    #[test]
    fn menu_bar_labels_are_title_case() {
        let small = ["a", "an", "and", "as", "at", "by", "for", "in", "of", "on", "or", "the", "to", "with", "from", "into"];
        for (path, label, _) in rows() {
            for (i, word) in label.split([' ', '/']).enumerate() {
                let first = word.trim_start_matches(['(', '.', '“']).chars().next();
                let lower = first.is_some_and(|c| c.is_lowercase());
                // Style presets are names ("Hidden line"); file suffixes
                // and glTF keep their own case.
                if lower && (i == 0 || !small.contains(&word)) && !word.starts_with(['(', '.']) && !word.starts_with("glTF")
                    && path != "View ▸ Style" && path != "View ▸ Model Views"
                {
                    panic!("{path} ▸ {label}: “{word}” is not Title Case");
                }
            }
        }
    }
}
