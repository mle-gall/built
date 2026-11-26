//! Commands shared by native chrome and the classic host. View flags and the
//! camera still live on the document controller, supplied as a frame snapshot.
use crate::document_registration::view::calls;
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
    command("axes", "Axes", "", &["Alt+A"], "toggle", Flag("_set_scene_visibility", "show_axes")),
    command("back_edges", "Back edges", "back_edges", &["K"], "toggle", Flag("_set_style_field", "back_edges")),
    command("hidden_geometry", "Hidden geometry", "hidden_geometry", &[], "toggle", Flag("_set_hidden_view", "show_hidden_geometry")).on(DisplayBar),
    command("shadows", "Shadows", "shadows", &[], "toggle", Shadows),
    command("section_cuts", "Section cuts", "section_cut", &[], "toggle", Flag("_set_scene_visibility", "show_section_cuts")).on(DisplayBar),
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
    command("section_planes", "Section Planes", "", &[], "Sections", Flag("_set_scene_visibility", "show_section_planes")),
    command("section_fill", "Section Fill", "", &[], "Sections", Flag("_set_style_field", "section_fill")),
    command("organise-scenes", "Organise scenes", "", &[], "Scenes", Chrome).on(RegistryOnly),
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
                .import("built_core.style")?
                .call_method1("style_by_name", (c.label,))?;
            win.call_method1("_apply_display_style", (preset,))?
        }
        Xray => win.call_method0("_toggle_xray")?,
        Shadows => win.call_method1("_on_toggle_shadows", (on,))?,
        Flag(method, field) => win.call_method1(method, (field, on))?,
        Method(method) => win.call_method0(method)?,
        Chrome => {
            let view = win.getattr("_doc")?.getattr("stand_in")?;
            calls::run_action(win, id, &view)?
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

/// Each document sends its rail to its own renderer when it next draws.
pub(crate) fn invalidate_command_rails(win: &Bound<'_, PyAny>) -> PyResult<()> {
    for doc in win.call_method0("documents")?.try_iter()? {
        let view = doc?.getattr("stand_in")?;
        if !view.is_none() {
            view.setattr("_ui_tools_sent", false)?;
        }
    }
    win.getattr("_host")?.call_method0("request_frame")?;
    Ok(())
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
    // Keep standard commands available to hosts without a system menu bar.
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
        for (id, label, key) in [
            ("save", "Save", "Ctrl+S"),
            ("save_as", "Save As…", "Ctrl+Shift+S"),
        ] {
            menu.push((id.into(), "File".into(), label.into(), key.into()));
        }
        for (id, label, key) in [
            ("undo", "Undo", "Ctrl+Z"),
            ("redo", "Redo", "Ctrl+Shift+Z"),
            ("select_all", "Select All", "Ctrl+A"),
            ("select_none", "Select None", "Ctrl+T"),
        ] {
            menu.push((id.into(), "Edit".into(), label.into(), key.into()));
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
        if row.5 == "redo" {
            for alternative in ["Ctrl+Y", "Ctrl+Shift+Z"] {
                if !row.3.iter().any(|key| key == alternative) {
                    row.3.push(alternative.into());
                }
            }
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
/// Bit 0 is the model, bit 1 the sheet. Window-level and OS gestures
/// can be received from either workspace.
#[pyfunction]
pub fn shortcut_domains(id: &str) -> u8 {
    if id.starts_with("sheet:") { return 2; }
    if id.starts_with("menu:platform:") || matches!(id, "command_search" | "menu:new" | "menu:open"
        | "menu:close_tab" | "menu:save" | "menu:save_as" | "menu:quit" | "menu:prefs") { 3 } else { 1 }
}

#[pyfunction]
pub fn shortcut_conflicts(
    target: &str,
    wanted: HashSet<String>,
    current: Vec<(String, Vec<String>)>,
) -> Vec<(String, Vec<String>)> {
    current
        .into_iter()
        .filter_map(|(id, mut keys)| {
            if shortcut_domains(&id) & shortcut_domains(target) == 0 { return None; }
            let before = keys.len();
            keys.retain(|k| !wanted.contains(k));
            (id != target && keys.len() != before).then_some((id, keys))
        })
        .collect()
}


/// Descriptions for shared menu commands whose host action has no status tip.
#[pyfunction(signature=(label, key=""))]
pub fn command_description(label: &str, key: &str) -> Option<&'static str> {
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
        "Save a Copy as Built (.igz)" => "Write a copy of this project in the Built file format.",
        "Terrain profile" => "Inspect ground elevations along a path through the model.",
        _ => match key {
            "menu:about" => "The version, the authors and the licence of Built.",
            "menu:animation_settings" => "Set the travel and pause times for saved scenes.",
            "menu:animation_video" => "Export a movie of the saved scenes.",
            "menu:bridge" => "Join two faces, a face and a line, or two lines with the faces between them.",
            "menu:clear_texture_cache" => "Delete the images extracted from imported .skp files.",
            "menu:convert_path" => "Turn the selected paths into edges of the model; a closed path becomes a face, ready to push up.",
            "menu:delete_guides" => "Remove every guide line and guide point from the model.",
            "menu:plugins_folder" => "Open the folder of your extensions; one put there loads at the next start.",
            "menu:save" => "Save the document — the model and every sheet — to its file.",
            "menu:save_as" => "Save the document under another name or in another folder.",
            "menu:develop_plugin" => "Open the developer guide for writing a Built extension.",
            "menu:documentation" => "Open the Built documentation in the web browser.",
            "menu:explode_group" => "Break the selected groups apart, back into the geometry around them.",
            "menu:export:dae" => "Bring in a COLLADA model with its materials and textures.",
            "menu:export:dxf" => "Save the view on screen as a 2D line drawing for CAD, with the hidden lines removed.",
            "menu:export:glb" => "Export the model as glTF or binary GLB.",
            "menu:export:ifc" => "Save the model as IFC for BIM software, with what the BIM panel tagged.",
            "menu:export:obj" => "Bring in a Wavefront OBJ model with its materials.",
            "menu:export:stl" => "Save the model as an STL mesh for 3D printing.",
            "menu:export:video" => "Save the scenes as a video: the camera glides from one to the next, with the times of Camera ▸ Animation.",
            "menu:extensions" => "Install, enable and update extensions.",
            "menu:get_models" => "Free websites with models and textures that open in Built.",
            "menu:heal_overlaps" => "Remove the faces left lying over their own subdivisions after drawing or erasing.",
            "menu:hide" => "Stop showing the selected objects, faces and edges; they stay in the document until unhidden.",
            "menu:import:alignment" => "Bring in a KML, KMZ or GeoJSON alignment as paths in their true place, ready to profile or measure.",
            "menu:import:image" => "Place a picture in the model to trace over.",
            "menu:import:obj" => "Bring in a Wavefront OBJ model with its materials.",
            "menu:import:orthomosaic" => "Lay a GeoTIFF orthomosaic under the model at its true place and size, to trace over.",
            "menu:import:photomesh" => "Bring in a WebODM drone survey in its true place, as a mesh to trace over.",
            "menu:import:stl" => "Import triangles from an STL mesh.",
            "menu:import:survey_points" => "Bring in surveyed points from a CSV file with UTM coordinates.",
            "menu:intersect_context" => "Add edges where the selected faces cross the rest of the group being edited, or of the model.",
            "menu:intersect_model" => "Add edges wherever the selected faces cross the rest of the model, groups included.",
            "menu:intersect_selection" => "Add edges where the selected faces cross one another.",
            "menu:invert_selection" => "Select what is not selected, and drop what is.",
            "menu:join_segments" => "Join collinear segments into single lines: the selected ones, or all of them when nothing is selected.",
            "menu:layout:four" => "Arrange four views of this model.",
            "menu:layout:single" => "Show a single view of this model.",
            "menu:layout:three" => "Arrange one large and two smaller model views.",
            "menu:layout:three_mirror" => "Place the large model view at the right of two smaller views.",
            "menu:layout:two" => "Place two model views side by side.",
            "menu:layout:two_stacked" => "Stack two model views vertically.",
            "menu:layout:two_wide" => "Show two model views with a wider primary view.",
            "menu:make_faces" => "Make the faces that the selected edges close, on those same edges.",
            "menu:make_group" => "Wrap the selection in a group that moves as one and keeps apart from the geometry around it.",
            "menu:maximize_model_view" => "Expand the active view or restore the previous layout.",
            "menu:orient_faces" => "Turn every face connected to the selected one so its front side matches.",
            "menu:prefs" => "Edit application and model settings.",
            "menu:rebuild_planar" => "Work the faces out again from the edges: on the plane of the selected faces, or over the whole drawing when it is flat.",
            "menu:recover_discarded" => "Auto-saved copies retired when a session was closed without saving — the last ones are kept here for a second chance.",
            "menu:redo" => "Bring back the change that was just undone.",
            "menu:repeat_last_command" => "Run the last command again, on what is selected now.",
            "menu:rest_fade" => "While a group is edited, the rest of the model shows pale, so the group stands out.",
            "menu:rest_hide" => "While a group is edited, the rest of the model is not drawn — the quickest on a heavy model.",
            "menu:rest_intensity" => "Set how faint the rest of the model appears while editing a component.",
            "menu:rest_normal" => "While a group is edited, the rest of the model shows as usual.",
            "menu:reverse_faces" => "Swap the front and back sides of the selected faces.",
            "menu:review" => "Open the model for review.",
            "menu:scene_add" => "Save the current view and layer visibility",
            "menu:scene_delete" => "Delete the selected scenes",
            "menu:scene_duplicate" => "Make a copy of the selected scene",
            "menu:scene_next" => "Recall the next saved scene.",
            "menu:scene_play" => "Start or stop playback through the saved scenes.",
            "menu:scene_previous" => "Recall the previous saved scene.",
            "menu:scene_rename" => "Rename the selected scene",
            "menu:scene_show" => "Show the selected scene",
            "menu:scene_transitions" => "Toggle camera glides between saved scenes.",
            "menu:scene_update" => "Update the selected scene from the current view",
            "menu:select_all" => "Select everything: edges, faces, groups and dimensions.",
            "menu:select_none" => "Deselect everything: edges, faces, groups and dimensions.",
            "menu:split_pieces" => "Regroup the selected group by the pieces that do not touch; explode it afterwards for each piece on its own.",
            "menu:terrain_profile" => "Draw the terrain profile along the selected path in the profile panel.",
            "menu:text3d" => "Create extruded lettering in the model.",
            "menu:toggle_projection" => "Switch between perspective and a parallel projection, where sizes do not shrink with distance.",
            "menu:undo" => "Take back the last change to the model.",
            "menu:unhide_all" => "Show again everything that is hidden.",
            "menu:unhide_last" => "Show again what the last hiding put away.",
            "menu:unhide_selected" => "Show again the hidden things that are selected — Hidden Objects or Hidden Geometry lets you select them.",
            "menu:units" => "Choose the units lengths, areas, volumes and angles are shown and typed in.",
            "menu:verb:field_of_view" => "Change the camera's angle of view, like a lens: wider takes more in, narrower keeps the lines calm.",
            "menu:verb:make_component" => "Turn the selection into a component: every copy shares it, so changing one changes them all.",
            "menu:verb:simplify_mesh" => "Merge coplanar and near-coplanar faces",
            "text:Sample material" => "Click a face to pick up its paint or texture (or hold Alt with the Paint tool).",
            "tool:change_axes" => "Set a new origin and direction for the modelling axes.",
            "tool:scale_manager" => "Manage drawing scales for this model.",
            "tool:texture_position" => "Place and size the texture on a face.",
            "view:axes" => "Show or hide the model’s drawing axes. Inference along the axes stays active.",
            "view:back_edges" => "Draw the edges hidden behind faces as dashed lines.",
            "view:organise-scenes" => "Manage folders and file saved scenes.",
            "view:camera" => "Edit the projection, lens and position of the camera.",
            "view:form-shadows" => "Set the sun position and shadow appearance.",
            "view:form-style" => "Edit the current model display style.",
            "view:hidden_geometry" => "Show or hide geometry marked as hidden.",
            "view:look_around" => "Drag to turn your head; the eye stays where it is.",
            "view:orbit" => "Left-drag to rotate the view.",
            "view:pan" => "Left-drag to slide the view.",
            "view:position_camera" => "Click where to stand and the eye goes there at a person's height; drag to also say where to look.",
            "view:projection_parallel" => "No vanishing points: measured elevations, plans and isometrics.",
            "view:projection_perspective" => "The view as the eye sees it.",
            "view:projection_two_point" => "Vertical lines stay vertical, the usual view for architectural presentations.",
            "view:section_cuts" => "Show or hide cuts made by active section planes.",
            "view:section_fill" => "Fill the cut faces of a section with a solid colour.",
            "view:section_planes" => "Show the section planes themselves; the cut they make stays either way.",
            "view:shadows" => "Cast the sun's shadows; the Shadows panel sets the date, the time and how dark they are.",
            "view:style_architectural" => "Faces with their materials on a plain white background, without the sky.",
            "view:style_default" => "Faces with their materials and textures, under the sky.",
            "view:style_hidden_line" => "White faces that hide what lies behind them — a clean line drawing.",
            "view:style_monochrome" => "Every face in the front or back colour, without materials.",
            "view:style_shaded" => "Faces in their colours, without textures.",
            "view:style_wireframe" => "Only the edges: the faces are not drawn.",
            "view:style_xray" => "See-through faces, so the edges behind them show.",
            "view:view_back" => "Look at the model from the back.",
            "view:view_bottom" => "Look straight up at the model from below.",
            "view:view_front" => "Look at the model from the front.",
            "view:view_iso" => "Look at the model from a corner above, at the same angle to the three axes.",
            "view:view_left" => "Look at the model from the left.",
            "view:view_right" => "Look at the model from the right.",
            "view:view_top" => "Look straight down on the model, in plan.",
            "view:walk" => "Drag to walk through the model at eye height: up and down go forward and back, left and right turn.",
            "view:xray" => "See-through faces, so the edges behind them show.",
            "view:zoom" => "Drag up or down to zoom in or out.",
            "view:zoom_extents" => "Frame the whole model in the view.",
            "view:zoom_selection" => "Frame the selection in the view.",
            "view:zoom_window" => "Drag a box to zoom to that region.",
            _ => return None,
        },
    })
}
