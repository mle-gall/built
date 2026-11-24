//! Commands shared by native chrome and the classic host. View flags and the
//! camera still live on the document controller, supplied as a frame snapshot.
use pyo3::prelude::*;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy)]
enum Target {
    Nav,
    Tool,
    Method(&'static str),
    View(&'static str),
    Projection(&'static str),
    Style,
    Xray,
    Shadows,
    Flag(&'static str, &'static str),
    Chrome,
}
use Target::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Surface {
    ViewMenu,
    NavigationBar,
    DisplayBar,
    RegistryOnly,
}
use Surface::{DisplayBar, NavigationBar, RegistryOnly};

struct Command {
    id: &'static str,
    label: &'static str,
    icon: &'static str,
    shortcuts: &'static [&'static str],
    group: &'static str,
    target: Target,
    surface: Surface,
}
const fn command(
    id: &'static str,
    label: &'static str,
    icon: &'static str,
    shortcuts: &'static [&'static str],
    group: &'static str,
    target: Target,
) -> Command {
    Command {
        id,
        label,
        icon,
        shortcuts,
        group,
        target,
        surface: Surface::ViewMenu,
    }
}

impl Command {
    const fn on(mut self, surface: Surface) -> Self {
        self.surface = surface;
        self
    }
}
#[rustfmt::skip]
const COMMANDS: &[Command] = &[
    command("orbit", "Orbit", "orbit", &["O"], "nav", Nav).on(NavigationBar),
    command("pan", "Pan", "pan", &["H"], "nav", Nav).on(NavigationBar),
    command("zoom", "Zoom", "zoom", &["Z"], "nav", Nav),
    command("zoom_window", "Zoom Window", "zoom_window", &[], "nav", Nav),
    command("zoom_extents", "Zoom Extents", "zoom_extents", &["Shift+Z", "F2"], "nav", Method("_on_zoom_extents")).on(NavigationBar),
    command("zoom_selection", "Zoom Selection", "zoom_selection", &["Ctrl+Alt+Z"], "nav", Method("_on_zoom_selection")),
    command("look_around", "Look Around", "look_around", &[], "walkthrough", Tool),
    command("walk", "Walk", "walk", &[], "walkthrough", Tool),
    command("position_camera", "Position Camera", "position_camera", &[], "walkthrough", Tool),
    command("xray", "X-ray", "xray", &["Alt+X"], "toggle", Xray).on(DisplayBar),
    command("back_edges", "Back edges", "back_edges", &["K"], "toggle", Flag("_set_style_field", "back_edges")),
    command("hidden_geometry", "Hidden geometry", "hidden_geometry", &[], "toggle", Flag("_set_hidden_view", "show_hidden_geometry")).on(DisplayBar),
    command("shadows", "Shadows", "shadows", &[], "toggle", Shadows),
    command("section_cuts", "Section cuts", "section_cut", &[], "toggle", Flag("_set_section_visibility", "show_section_cuts")).on(DisplayBar),
    command("view_iso", "Isometric", "view_iso", &["Ctrl+4"], "Views", View("iso")),
    command("view_top", "Top", "view_top", &["Ctrl+1"], "Views", View("top")),
    command("view_front", "Front", "view_front", &["Ctrl+2"], "Views", View("front")),
    command("view_right", "Right", "view_right", &["Ctrl+3"], "Views", View("right")),
    command("view_left", "Left", "view_left", &["Ctrl+6"], "Views", View("left")),
    command("view_back", "Back", "view_back", &["Ctrl+5"], "Views", View("back")),
    command("view_bottom", "Bottom", "view_bottom", &["Ctrl+7"], "Views", View("bottom")),
    command("camera", "Camera inspector…", "", &[], "Views", Chrome),
    command("projection_parallel", "Parallel Projection", "proj_parallel", &[], "Projection", Projection("parallel")),
    command("projection_perspective", "Perspective", "proj_perspective", &[], "Projection", Projection("perspective")),
    command("projection_two_point", "Two-Point Perspective", "proj_two_point", &[], "Projection", Projection("two_point")),
    command("style_default", "Default", "", &[], "Style", Style),
    command("style_architectural", "Architectural", "", &[], "Style", Style),
    command("style_shaded", "Shaded", "", &[], "Style", Style),
    command("style_hidden_line", "Hidden line", "", &[], "Style", Style),
    command("style_monochrome", "Monochrome", "", &[], "Style", Style),
    command("style_wireframe", "Wireframe", "", &[], "Style", Style),
    command("style_xray", "X-ray", "", &[], "Style", Style).on(RegistryOnly),
    command("section_planes", "Section Planes", "", &[], "Sections", Flag("_set_section_visibility", "show_section_planes")),
    command("section_fill", "Section Fill", "", &[], "Sections", Flag("_set_style_field", "section_fill")),
    command("form-style", "Edit style…", "", &[], "Settings", Chrome),
    command("form-shadows", "Shadows and sun…", "", &[], "Settings", Chrome),
];

impl Command {
    fn state(
        &self,
        active: &str,
        projection: &str,
        style: &str,
        flags: &HashSet<String>,
        selection: bool,
    ) -> (i8, bool) {
        let checked = match self.target {
            Nav | Tool => active == self.id,
            Projection(mode) => mode == projection,
            Style => self.label == style,
            Flag(_, field) => flags.contains(field),
            Xray | Shadows => flags.contains(self.id),
            _ => return (-1, self.id != "zoom_selection" || selection),
        };
        (i8::from(checked), true)
    }
}

// Controller fields that have not yet moved into the native document.
type Snapshot = (String, String, String, HashSet<String>, bool);
type Bindings = HashMap<String, (String, bool)>;
type LabelMap = HashMap<String, String>;
type Menu = Vec<(String, Vec<crate::ui::Item>)>;

#[pyfunction]
pub fn view_commands() -> Vec<(
    &'static str,
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static str,
)> {
    COMMANDS
        .iter()
        .map(|c| (c.group, c.id, c.label, c.shortcuts, c.icon))
        .collect()
}

#[pyfunction]
pub fn command_chrome(
    snapshot: Snapshot,
    labels: LabelMap,
    bindings: Bindings,
) -> (Vec<Vec<crate::ui::Item>>, Menu) {
    let (active, projection, style, flags, selection) = snapshot;
    let translate = |s: &str| labels.get(s).cloned().unwrap_or_else(|| s.to_owned());
    let mut cluster = vec![Vec::new(); 2];
    let mut menu: Menu = Vec::new();
    for c in COMMANDS {
        let (checked, enabled) = c.state(&active, &projection, &style, &flags, selection);
        let (shortcut, allowed) = bindings
            .get(c.id)
            .map(|(s, on)| (s.as_str(), *on))
            .unwrap_or((c.shortcuts.first().copied().unwrap_or(""), true));
        let label = translate(c.label);
        let cluster_group = match c.surface {
            NavigationBar => Some(0),
            DisplayBar => Some(1),
            RegistryOnly => continue,
            Surface::ViewMenu => None,
        };
        let tip = if cluster_group.is_some() && !shortcut.is_empty() {
            format!("{label}    {shortcut}")
        } else {
            shortcut.to_owned()
        };
        let item = (
            c.id.to_owned(),
            label,
            tip,
            c.icon.to_owned(),
            checked,
            enabled && allowed,
        );
        if let Some(group) = cluster_group {
            cluster[group].push(item);
        } else {
            let title = translate(match c.group {
                "nav" => "Navigation",
                "walkthrough" => "Walkthrough",
                "toggle" => "Display",
                title => title,
            });
            if menu.last().is_none_or(|(group, _)| *group != title) {
                menu.push((title, Vec::new()));
            }
            menu.last_mut().unwrap().1.push(item);
        }
    }
    (cluster, menu)
}

#[pyfunction]
pub fn activate_view_command(
    win: &Bound<'_, PyAny>,
    id: &str,
    snapshot: Snapshot,
) -> PyResult<bool> {
    let Some(c) = COMMANDS.iter().find(|c| c.id == id) else {
        return Ok(false);
    };
    let (active, projection, style, flags, selection) = snapshot;
    let (checked, enabled) = c.state(&active, &projection, &style, &flags, selection);
    if !enabled {
        return Ok(false);
    }
    run_view_command(win, id, checked != 1)
}

#[pyfunction]
pub fn run_view_command(win: &Bound<'_, PyAny>, id: &str, on: bool) -> PyResult<bool> {
    let Some(c) = COMMANDS.iter().find(|c| c.id == id) else {
        return Ok(false);
    };
    if matches!(c.target, Tool | Nav) {
        let vp = win.getattr("viewport")?;
        let active = win
            .call_method1("_tool_key", (vp.getattr("active_tool")?,))?
            .extract::<Option<String>>()?;
        if let Some(key) = active {
            if !COMMANDS
                .iter()
                .any(|entry| entry.id == key && matches!(entry.target, Tool))
            {
                vp.setattr("_return_tool", key)?;
            }
        }
    }
    match c.target {
        Nav => win.call_method1("_activate_nav", (id,))?,
        Tool => win.call_method1("_activate_tool", (id,))?,
        View(view) => win.call_method1("_on_standard_view", (view,))?,
        Projection(mode) => win.call_method1("_set_projection", (mode,))?,
        Style => {
            let preset = win
                .py()
                .import("core.style")?
                .call_method1("style_by_name", (c.label,))?;
            win.call_method1("_apply_display_style", (preset,))?
        }
        Xray => win.call_method0("_toggle_xray")?,
        Shadows => win.call_method1("_on_toggle_shadows", (on,))?,
        Flag(method, field) => win.call_method1(method, (field, on))?,
        Method(method) => win.call_method0(method)?,
        Chrome => {
            let view = win.getattr("_doc")?.getattr("stand_in")?;
            win.py().import("views.wgpu_ui")?.call_method1("_run", (win, [format!("act:{id}")], view))?
        }
    };
    Ok(true)
}

#[pyfunction]
pub fn view_shortcut(key: &str) -> Option<&'static str> {
    COMMANDS
        .iter()
        .find(|c| c.shortcuts.contains(&key))
        .map(|c| c.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids_are_unique_and_shortcuts_resolve() {
        let mut ids = HashSet::new();
        for c in COMMANDS {
            assert!(ids.insert(c.id));
            for key in c.shortcuts {
                assert_eq!(view_shortcut(key), Some(c.id));
            }
        }
    }
    #[test]
    fn chrome_shows_each_command_once_and_keeps_the_xray_shortcut() {
        let (bar, menu) =
            command_chrome(Default::default(), Default::default(), Default::default());
        let ids =
            |items: &[crate::ui::Item]| items.iter().map(|row| row.0.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&bar[0]), ["orbit", "pan", "zoom_extents"]);
        assert_eq!(ids(&bar[1]), ["xray", "hidden_geometry", "section_cuts"]);
        let all: Vec<_> = bar
            .iter()
            .flatten()
            .chain(menu.iter().flat_map(|(_, rows)| rows))
            .collect();
        assert_eq!(all.iter().filter(|item| item.1 == "X-ray").count(), 1);
        assert_eq!(
            all.iter().map(|item| &item.0).collect::<HashSet<_>>().len(),
            all.len()
        );
        assert!(menu
            .iter()
            .any(|(title, rows)| title == "Walkthrough" && ids(rows).contains(&"walk".into())));
        assert_eq!(view_shortcut("Alt+X"), Some("xray"));
    }

    #[test]
    fn selection_and_active_camera_modes_drive_state() {
        let state = |id| {
            COMMANDS.iter().find(|c| c.id == id).unwrap().state(
                "orbit",
                "parallel",
                "Default",
                &HashSet::from(["show_hidden_geometry".into()]),
                false,
            )
        };
        assert_eq!(state("zoom_selection"), (-1, false));
        for id in [
            "orbit",
            "projection_parallel",
            "style_default",
            "hidden_geometry",
        ] {
            assert_eq!(state(id), (1, true));
        }
        assert_eq!(state("pan"), (0, true));
    }
}

/// Escape from the camera controller, including its eye-level tools.
#[pyfunction]
pub fn leave_navigation(win: &Bound<'_, PyAny>) -> PyResult<bool> {
    let vp = win.getattr("viewport")?;
    if vp.getattr("nav_mode")?.is_none() {
        let tool = win
            .call_method1("_tool_key", (vp.getattr("active_tool")?,))?
            .extract::<Option<String>>()?;
        if !COMMANDS
            .iter()
            .any(|c| Some(c.id) == tool.as_deref() && matches!(c.target, Tool))
        {
            return Ok(false);
        }
    }
    let key = vp
        .getattr("_return_tool")
        .and_then(|v| v.extract::<String>())
        .unwrap_or_else(|_| "select".into());
    win.call_method1("_activate_tool", (key,))?;
    Ok(true)
}

#[pyfunction]
#[rustfmt::skip]
pub fn rail_layout() -> Vec<Vec<(&'static str, &'static str)>> {
    vec![
        vec![("select", ""), ("eraser", ""), ("paint", ""), ("eyedropper", "_act_eyedropper"), ("components", "_act_library_components")],
        vec![("line", ""), ("freehand", ""), ("rectangle", ""), ("rotated_rect", ""), ("circle", ""), ("polygon", ""), ("arc", ""), ("arc3", ""), ("center_arc", ""), ("pie", "")],
        vec![("pushpull", ""), ("move", ""), ("rotate", ""), ("scale", ""), ("flip", ""), ("followme", ""), ("offset", ""), ("fillet", ""), ("bridge", "_act_bridge"), ("make_faces", "_act_make_faces")],
        vec![("tape", ""), ("protractor", ""), ("north", ""), ("dimension", ""), ("text", ""), ("text3d", "_act_3dtext"), ("geopath", ""), ("section", "")],
        vec![("outer_shell", ""), ("solid_union", ""), ("solid_subtract", ""), ("solid_trim", ""), ("solid_intersect", ""), ("solid_split", "")],
    ]
}

type RegistryRow = (String, String, String, Vec<String>, String, String);

/// One row per dispatch target. A Cocoa menu entry and egui View command
/// for the same operation share both their shortcut list and their action.
#[pyfunction]
pub fn command_registry(
    tools: Vec<(String, String, Vec<String>)>,
    mut menu: Vec<(String, String, String, String)>,
) -> Vec<RegistryRow> {
    use std::collections::BTreeMap;
    let search = "command_search".to_owned();
    let mut rows = BTreeMap::from([(
        search.clone(),
        (
            search,
            "Help".into(),
            "Search commands…".into(),
            vec!["F3".into(), "Ctrl+K".into()],
            String::new(),
            String::new(),
        ),
    )]);
    for (key, label, shortcuts) in tools {
        let id = format!("tool:{key}");
        rows.insert(
            id.clone(),
            (id, "Tools".into(), label, shortcuts, key, String::new()),
        );
    }
    for c in COMMANDS {
        let tool = rows
            .remove(&format!("tool:{}", c.id))
            .map(|r| r.4)
            .unwrap_or_default();
        let id = format!("view:{}", c.id);
        rows.insert(
            id.clone(),
            (
                id,
                c.group.to_owned(),
                c.label.to_owned(),
                c.shortcuts.iter().map(|s| (*s).to_owned()).collect(),
                tool,
                String::new(),
            ),
        );
    }
    // These sheets also open from the registry on hosts without a system menu bar.
    if menu.iter().all(|r| r.0.starts_with("extension:")) {
        for (id, label, key) in [
            ("about", "About Built", ""),
            ("cutlist", "Cut list", ""),
            ("extensions", "Extension Manager", ""),
            ("prefs", "Settings…", "Ctrl+,"),
            ("units", "Units…", "Ctrl+U"),
        ] {
            menu.push((id.into(), "Window".into(), label.into(), key.into()));
        }
    }
    // Platform actions (for example Hide Built) keep their factory bindings.
    let platform_keys: HashSet<_> = menu.iter().filter(|row| row.0.starts_with("platform:"))
        .map(|row| row.3.clone()).collect();
    for entry in crate::model_menu::entries() {
        let default = if platform_keys.contains(entry.shortcut) { "" } else { entry.shortcut };
        if let Some(installed) = menu.iter_mut().find(|row| row.0 == entry.id) {
            if installed.3.is_empty() {
                installed.3 = default.into();
            }
        } else {
            menu.push((entry.id.into(), entry.path.into(), entry.label.into(), default.into()));
        }
    }
    menu.sort_by_key(|row| row.0.starts_with("extension:"));
    for (menu_id, path, name, mut shortcut) in menu {
        if menu_id.starts_with("extension:") && !shortcut.is_empty() {
            let normalized = |key: &str| crate::shortcut_text::shortcut_text(key).map(|seq|seq.0);
            let key=normalized(&shortcut);
            if rows.values().flat_map(|r| &r.3).any(|other| normalized(other)==key) {
                shortcut.clear();
            }
        }
        let id = if let Some(key) = menu_id.strip_prefix("view:") {
            format!("view:view_{key}")
        } else if COMMANDS.iter().any(|c| c.id == menu_id) {
            format!("view:{menu_id}")
        } else if menu_id.starts_with("tool:") {
            menu_id.clone()
        } else {
            format!("menu:{menu_id}")
        };
        let row = rows.entry(id.clone()).or_insert_with(|| {
            (
                id,
                path.clone(),
                name.clone(),
                vec![],
                String::new(),
                String::new(),
            )
        });
        row.1 = path;
        row.5 = menu_id;
        if !shortcut.is_empty() && !row.3.contains(&shortcut) {
            row.3.insert(0, shortcut);
        }
    }
    rows.into_values().collect()
}

/// Qt supplies the logical key name after interpreting the native keyboard layout.
#[pyfunction]
pub fn shortcut_reserved(key: &str, plain: bool) -> Option<&'static str> {
    match key {
        "Key_Escape" | "Key_Return" | "Key_Enter" | "Key_Backspace" | "Key_Tab" | "Key_Backtab"
        | "Key_Shift" | "Key_Control" | "Key_Alt" | "Key_Meta" | "Key_AltGr" => {
            Some("the drawing tools use it (cancel, confirm, edit a value)")
        }
        "Key_Left" | "Key_Right" | "Key_Up" | "Key_Down" if plain => {
            Some("the arrow keys lock an axis while drawing")
        }
        "Key_Period" | "Key_Comma" | "Key_Semicolon" | "Key_Minus" | "Key_Slash"
        | "Key_Asterisk" | "Key_Apostrophe" | "Key_QuoteDbl"
            if plain =>
        {
            Some("it types a measure in the value box")
        }
        name if plain
            && name
                .strip_prefix("Key_")
                .is_some_and(|s| s.len() == 1 && s.as_bytes()[0].is_ascii_digit()) =>
        {
            Some("it types a measure in the value box")
        }
        _ => None,
    }
}

/// Plan the other edits in an assignment, retaining non-conflicting alternatives.
#[pyfunction]
pub fn shortcut_conflicts(
    target: &str,
    wanted: HashSet<String>,
    current: Vec<(String, Vec<String>)>,
) -> Vec<(String, Vec<String>)> {
    current
        .into_iter()
        .filter_map(|(id, mut keys)| {
            let before = keys.len();
            keys.retain(|k| !wanted.contains(k));
            (id != target && keys.len() != before).then_some((id, keys))
        })
        .collect()
}


/// Descriptions for shared menu commands whose host action has no status tip.
#[pyfunction]
pub fn command_description(label: &str) -> Option<&'static str> {
    Some(match label {
        "Components" => "Browse component definitions and place a copy in the model.",
        "Materials" => "Browse the materials available to apply to faces.",
        "Cut list" => "Review part dimensions, quantities and cutting requirements.",
        "Export" => "Choose a format and scope for a one-off export.",
        "Hide Rest of Model" => "Hide geometry outside the component being edited.",
        "Hide Similar Components" => "Hide other instances of the component being edited.",
        "Home" | "Start Page" => "Open the start page to create a project or reopen a recent one.",
        "List" => "Show saved scenes in a compact list.",
        "Thumbnails" => "Show saved scenes with preview images.",
        "Save a Copy as IngeTrazo (.igz)" => "Write a copy of this project in the IngeTrazo file format.",
        "Terrain profile" => "Inspect ground elevations along a path through the model.",
        _ => return None,
    })
}
