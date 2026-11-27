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
    /// The top bar's navigation cluster (M04): camera tools, then the
    /// eye-level tools after a divider.
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
    command("sidebar_project", "Project", "", &["Ctrl+Alt+1"], "Sidebar", Chrome).on(RegistryOnly),
    command("sidebar_tasks", "Tasks", "", &["Ctrl+Alt+2"], "Sidebar", Chrome).on(RegistryOnly),
    command("sidebar_deliveries", "Deliveries", "", &["Ctrl+Alt+3"], "Sidebar", Chrome).on(RegistryOnly),
    command("ask_agent", "Ask an Agent…", "", &["Ctrl+Shift+A"], "Sidebar", Chrome).on(RegistryOnly),
    command("orbit", "Orbit", "orbit", &["O"], "nav", Nav).on(NavigationBar),
    command("pan", "Pan", "pan", &["H"], "nav", Nav).on(NavigationBar),
    command("zoom", "Zoom", "zoom", &["Z"], "nav", Nav).on(NavigationBar),
    command("zoom_window", "Zoom Window", "zoom_window", &[], "nav", Nav).on(NavigationBar),
    command("zoom_extents", "Zoom Extents", "zoom_extents", &["Shift+Z", "F2"], "nav", Method("_on_zoom_extents")).on(NavigationBar),
    command("zoom_selection", "Zoom Selection", "zoom_selection", &["Ctrl+Alt+Z"], "nav", Method("_on_zoom_selection")).on(NavigationBar),
    command("look_around", "Look Around", "look_around", &[], "walkthrough", Tool).on(NavigationBar),
    command("walk", "Walk", "walk", &[], "walkthrough", Tool).on(NavigationBar),
    command("position_camera", "Position Camera", "position_camera", &[], "walkthrough", Tool).on(NavigationBar),
    command("xray", "X-ray", "xray", &["Alt+X"], "toggle", Xray).on(DisplayBar),
    command("axes", "Axes", "", &["Alt+A"], "toggle", Flag("_set_scene_visibility", "show_axes")),
    command("back_edges", "Back Edges", "back_edges", &["K"], "toggle", Flag("_set_style_field", "back_edges")),
    command("hidden_geometry", "Hidden Geometry", "hidden_geometry", &[], "toggle", Flag("_set_hidden_view", "show_hidden_geometry")).on(DisplayBar),
    command("shadows", "Shadows", "shadows", &[], "toggle", Shadows),
    command("section_cuts", "Section Cuts", "section_cut", &[], "toggle", Flag("_set_scene_visibility", "show_section_cuts")).on(DisplayBar),
    // Views in shortcut order (UX pass 1, M05).
    command("view_top", "Top", "view_top", &["Ctrl+1"], "Views", View("top")),
    command("view_front", "Front", "view_front", &["Ctrl+2"], "Views", View("front")),
    command("view_right", "Right", "view_right", &["Ctrl+3"], "Views", View("right")),
    command("view_iso", "Isometric", "view_iso", &["Ctrl+4"], "Views", View("iso")),
    command("view_back", "Back", "view_back", &["Ctrl+5"], "Views", View("back")),
    command("view_left", "Left", "view_left", &["Ctrl+6"], "Views", View("left")),
    command("view_bottom", "Bottom", "view_bottom", &["Ctrl+7"], "Views", View("bottom")),
    command("camera", "Camera Inspector…", "", &[], "Views", Chrome),
    command("projection_perspective", "Perspective", "proj_perspective", &[], "Projection", Projection("perspective")),
    command("projection_parallel", "Parallel Projection", "proj_parallel", &[], "Projection", Projection("parallel")),
    command("projection_two_point", "Two-Point Perspective", "proj_two_point", &[], "Projection", Projection("two_point")),
    command("style_default", "Default", "", &[], "Style", Style),
    command("style_architectural", "Architectural", "", &[], "Style", Style),
    command("style_shaded", "Shaded", "", &[], "Style", Style),
    command("style_hidden_line", "Hidden line", "", &[], "Style", Style),
    command("style_monochrome", "Monochrome", "", &[], "Style", Style),
    command("style_wireframe", "Wireframe", "", &[], "Style", Style),
    command("style_xray", "X-ray Style", "", &[], "Style", Style).on(RegistryOnly),
    command("section_planes", "Section Planes", "", &[], "Sections", Flag("_set_scene_visibility", "show_section_planes")),
    command("section_fill", "Section Fill", "", &[], "Sections", Flag("_set_style_field", "section_fill")),
    command("organise-scenes", "Organise Scenes", "", &[], "Scenes", Chrome).on(RegistryOnly),
    command("form-style", "Edit Style…", "", &[], "Settings", Chrome),
    command("recent-activity", "Recent Activity", "", &[], "Activity", Chrome),
    command("form-shadows", "Shadows and Sun…", "", &[], "Settings", Chrome),
];

impl Command {
    /// The display style a Style command picks, by its name in
    /// `built_core.style`. The X-ray style's command is called "X-ray
    /// Style" so that it does not share a name with the X-ray toggle.
    fn style_name(&self) -> &'static str {
        if self.id == "style_xray" { "X-ray" } else { self.label }
    }

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
            Style => self.style_name() == style,
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
    // Navigation, eye-level tools, display toggles.
    let mut cluster = vec![Vec::new(); 3];
    let mut menu: Menu = Vec::new();
    for c in COMMANDS {
        let (checked, enabled) = c.state(&active, &projection, &style, &flags, selection);
        let (shortcut, allowed) = bindings
            .get(c.id)
            .map(|(s, on)| (s.as_str(), *on))
            .unwrap_or((c.shortcuts.first().copied().unwrap_or(""), true));
        let label = translate(c.label);
        let cluster_group = match c.surface {
            NavigationBar if c.group == "walkthrough" => Some(1),
            NavigationBar => Some(0),
            DisplayBar => Some(2),
            RegistryOnly => continue,
            Surface::ViewMenu => None,
        };
        let tip = if cluster_group.is_some() && !shortcut.is_empty() {
            format!("{label}    {shortcut}")
        } else if cluster_group.is_some() {
            label.clone()
        } else {
            shortcut.to_owned()
        };
        // The View menu names its projection choices: labelled check rows,
        // not three bare icons (M05). The bar and registry keep the art.
        let icon = if cluster_group.is_none() && matches!(c.target, Projection(_)) { "" } else { c.icon };
        let item = (
            c.id.to_owned(),
            label,
            tip,
            icon.to_owned(),
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

/// The native menu uses short IDs for directions and registry IDs for other commands.
pub(super) fn native_view_command(key: &str) -> &str {
    COMMANDS.iter().find(|command| matches!(command.target, View(name) if name == key))
        .map_or(key, |command| command.id)
}

#[pyfunction]
pub fn activate_view_command(
    win: &Bound<'_, PyAny>,
    id: &str,
    snapshot: Snapshot,
) -> PyResult<bool> {
    if let Some(drag) = id.strip_prefix(GIZMO_ORBIT) {
        return gizmo_orbit(win, drag);
    }
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
                .call_method1("style_by_name", (c.style_name(),))?;
            win.call_method1("_apply_display_style", (preset,))?
        }
        Xray => win.call_method0("_toggle_xray")?,
        Shadows => win.call_method1("_on_toggle_shadows", (on,))?,
        Flag(method, field) => win.call_method1(method, (field, on))?,
        // Zoom extents and fit selection ease into place (M03 item 5).
        Method(method) if c.group == "nav" => {
            crate::native_view_actions::glide_after(win, |_| win.call_method0(method).map(drop))?;
            win.py().None().into_bound(win.py())
        }
        Method(method) => win.call_method0(method)?,
        Chrome if id == "ask_agent" => {
            let view = crate::dialogs_host::view_of(win.py(), win);
            view.getattr("renderer")?.call_method0("ui_compose_task")?;
            view.call_method0("update")?
        }
        Chrome if id.starts_with("sidebar_") => {
            let view = crate::dialogs_host::view_of(win.py(), win);
            view.getattr("renderer")?.call_method1("ui_show_destination", (&id[8..],))?;
            view.call_method0("update")?
        }
        Chrome if id == "recent-activity" => {
            let view = win.getattr("_doc")?.getattr("stand_in")?;
            view.getattr("renderer")?.call_method0("ui_open_activity")?;
            view.call_method0("update")?
        }
        Chrome => {
            let view = win.getattr("_doc")?.getattr("stand_in")?;
            calls::run_action(win, id, &view)?
        }
    };
    Ok(true)
}

/// The axis gizmo's drag: `gizmo_orbit:dx,dy` in logical pixels orbits the
/// active view as a drag in the model does (UX pass 2, M03).
pub const GIZMO_ORBIT: &str = "gizmo_orbit:";

fn parse_drag(drag: &str) -> Option<(f64, f64)> {
    let (dx, dy) = drag.split_once(',')?;
    let (dx, dy) = (dx.trim().parse::<f64>().ok()?, dy.trim().parse::<f64>().ok()?);
    (dx.is_finite() && dy.is_finite()).then_some((dx, dy))
}

fn gizmo_orbit(win: &Bound<'_, PyAny>, drag: &str) -> PyResult<bool> {
    let Some((dx, mut dy)) = parse_drag(drag) else { return Ok(false) };
    let vp = win.call_method0("_active_view")?;
    if vp
        .getattr_opt("_invert_orbit_y")?
        .is_some_and(|v| v.is_truthy().unwrap_or(false))
    {
        dy = -dy;
    }
    if let Some(player) = win.getattr_opt("scene_player")?.filter(|p| !p.is_none()) {
        player.call_method0("cancel")?;
    }
    let height = vp.call_method0("height")?;
    vp.getattr("camera")?.call_method1("orbit", (dx, dy, height))?;
    vp.call_method0("update")?;
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
    fn view_menu_orders_views_by_shortcut_and_labels_projections() {
        let (_, menu) = command_chrome(Default::default(), Default::default(), Default::default());
        let group = |name: &str| menu.iter().find(|(t, _)| t == name).map(|(_, rows)| rows.clone()).unwrap();
        let keys: Vec<String> = group("Views").iter().filter(|r| !r.2.is_empty()).map(|r| r.2.clone()).collect();
        assert_eq!(keys, (1..=7).map(|n| format!("Ctrl+{n}")).collect::<Vec<_>>());
        let projection = group("Projection");
        assert_eq!(projection.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
                   ["projection_perspective", "projection_parallel", "projection_two_point"]);
        // Rows with a label and a check, not icon-only buttons.
        assert!(projection.iter().all(|r| r.3.is_empty() && !r.1.is_empty() && r.4 >= 0));
    }
    #[test]
    fn chrome_shows_each_command_once_and_keeps_the_xray_shortcut() {
        let (bar, menu) =
            command_chrome(Default::default(), Default::default(), Default::default());
        let ids =
            |items: &[crate::ui::Item]| items.iter().map(|row| row.0.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&bar[0]), ["orbit", "pan", "zoom", "zoom_window", "zoom_extents", "zoom_selection"]);
        assert_eq!(ids(&bar[1]), ["look_around", "walk", "position_camera"]);
        assert_eq!(ids(&bar[2]), ["xray", "hidden_geometry", "section_cuts"]);
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
        // The View menu no longer repeats the navigation row (M04).
        assert!(!menu
            .iter()
            .any(|(title, _)| title == "Navigation" || title == "Walkthrough"));
        assert_eq!(view_shortcut("Alt+X"), Some("xray"));
    }

    #[test]
    fn every_navigation_tool_is_on_the_bar_with_a_named_tooltip() {
        let (bar, _) = command_chrome(Default::default(), Default::default(), Default::default());
        let nav: Vec<_> = bar[0].iter().chain(&bar[1]).collect();
        assert_eq!(nav.len(), 9);
        for item in nav {
            assert!(!item.3.is_empty(), "{} has an icon", item.0);
            assert!(item.2.starts_with(&item.1), "{} tooltip {:?} names it", item.0, item.2);
        }
        let zoom = bar[0].iter().find(|i| i.0 == "zoom").unwrap();
        assert_eq!(zoom.2, "Zoom    Z");
        assert_eq!(bar[1][1].2, "Walk");
    }

    #[test]
    fn style_commands_pick_their_style_by_its_name() {
        // The style is looked up by name: renaming a command must not lose it.
        let names: Vec<_> = COMMANDS.iter().filter(|c| matches!(c.target, Style)).map(Command::style_name).collect();
        assert_eq!(names, ["Default", "Architectural", "Shaded", "Hidden line", "Monochrome", "Wireframe", "X-ray"]);
        let xray = COMMANDS.iter().find(|c| c.id == "style_xray").unwrap();
        assert_eq!(xray.state("", "", "X-ray", &HashSet::new(), false).0, 1);
    }

    #[test]
    fn gizmo_drags_parse_only_finite_pairs() {
        assert_eq!(parse_drag("4,-2.5"), Some((4., -2.5)));
        assert_eq!(parse_drag("4"), None);
        assert_eq!(parse_drag("nan,1"), None);
        assert_eq!(parse_drag("a,b"), None);
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
        vec![("line", ""), ("line_on_surface", ""), ("sweep", ""), ("freehand", ""), ("rectangle", ""), ("rotated_rect", ""), ("circle", ""), ("polygon", ""), ("arc", ""), ("arc3", ""), ("center_arc", ""), ("pie", "")],
        vec![("pushpull", ""), ("move", ""), ("rotate", ""), ("scale", ""), ("flip", ""), ("followme", ""), ("offset", ""), ("fillet", ""), ("chamfer", ""), ("bridge", "_act_bridge"), ("make_faces", "_act_make_faces"), ("cut", "")],
        vec![("tape", ""), ("protractor", ""), ("change_axes", ""), ("north", ""), ("dimension", ""), ("text", ""), ("text3d", "_act_3dtext"), ("geopath", ""), ("section", "")],
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

pub(crate) const UNITS_SHORTCUT: &str = "Ctrl+U";

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
        for (id, path, label, key) in [
            ("about", "Built", "About Built", ""),
            ("prefs", "Built", "Settings…", "Ctrl+,"),
            ("cutlist", "Model", "Cut List…", ""),
            ("units", "Model", "Units…", UNITS_SHORTCUT),
            ("extensions", crate::desktop_menu_model::EXTENSIONS_MENU, "Extension Manager…", ""),
        ] {
            menu.push((id.into(), path.into(), label.into(), key.into()));
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
        "Cut list" | "Cut List" => "Review part dimensions, quantities and cutting requirements.",
        "Export" => "Choose a format and scope for a one-off export.",
        "Hide Rest of Model" => "Hide geometry outside the component being edited.",
        "Hide Similar Components" => "Hide other instances of the component being edited.",
        "Home" | "Start Page" => "Open the start page to create a project or reopen a recent one.",
        "List" => "Show saved scenes in a compact list.",
        "Thumbnails" => "Show saved scenes with preview images.",
        "Save a Copy as .igz (IngeTrazo)" => "Write a copy of this model in the legacy .igz format, for IngeTrazo.",
        "Terrain profile" | "Terrain Profile of Selection" => "Inspect ground elevations along a path through the model.",
        _ => match key {
            "view:sidebar_project" => "Browse the Project tree and inspect the selection.",
            "view:sidebar_tasks" => "Open agent reviews and questions.",
            "view:ask_agent" => "Send a request to the built-in assistant or a connected agent; it becomes a task to review.",
            "view:sidebar_deliveries" | "menu:delivery_export" => "Check and run the model’s saved deliveries.",
            "menu:about" => "The version, the authors and the licence of Built.",
            "menu:animation_settings" => "Set the travel and pause times for saved scenes.",
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
            "menu:import:step" => "Import STEP parts and their placements.",
            "menu:export:step" => "Save the model as validated AP242 STEP geometry.",
            "menu:recover_exact" => "Preview analytic supports fitted to selected faces.",
            "menu:make_faceted" => "Convert the selected exact geometry to facets.",
            "menu:export:dae" => "Save the model as COLLADA with its materials and textures.",
            "menu:export:dxf" => "Save the view on screen as a 2D line drawing for CAD, with the hidden lines removed.",
            "menu:export:glb" => "Export the model as glTF or binary GLB.",
            "menu:export:ifc" => "Save the model as IFC for BIM software, with what the BIM panel tagged.",
            "menu:export:obj" => "Save the model as Wavefront OBJ with its materials.",
            "menu:export:stl" => "Save the model as an STL mesh for 3D printing.",
            "menu:export:video" => "Save the scenes as a video: the camera glides from one to the next, with the times of View ▸ Animation.",
            "menu:new_project" => "Create a .builtproject that links models and keeps their project structure together.",
            "menu:export_image" => "Save the view on screen as a picture.",
            "menu:export_view_svg" => "Save the view on screen as an SVG line drawing.",
            "menu:export_igz" => "Write a copy of this model in the legacy .igz format, for IngeTrazo.",
            "menu:hide_rest" => "Hide the rest of the model while a group is edited, or show it again.",
            "menu:hide_similar" => "Hide the other copies of the component being edited.",
            "menu:extensions" => "Install, enable and update extensions.",
            "menu:get_models" => "Free websites with models and textures that open in Built.",
            "menu:axes_place" => "Enter the origin coordinates and rotations of the drawing axes.",
            "menu:axes_move" => "Pick the origin, then the red and green directions.",
            "menu:axes_reset" => "Restore the drawing axes to the model origin and orientation.",
            "menu:axes_align_view" => "Look along the blue axis with the green axis pointing up.",
            "menu:axes_face" => "Set red along the longest face edge and blue perpendicular to the face.",
            "menu:look_at_face" => "Look straight at the face under the cursor, from the side you see it, in parallel projection until the next orbit.",
            "menu:show_keystrokes" => "Show the keys you press, the values you type and the tool a key picks at the bottom of the model, for screen recordings.",
            "menu:weld_edges" => "Join connected edges into one curve, or into one straight edge when they line up; with nothing selected, join the model's collinear pieces.",
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
            "menu:review" => "Hide the panels to review the model; the same command shows them again.",
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
            "view:recent-activity" => "Show recent notices and job results.",
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
            "view:style_xray" => "The X-ray display style: see-through faces, so the edges behind them show.",
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
