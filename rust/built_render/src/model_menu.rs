//! Model commands shared by the system menu and the portable command registry.
use pyo3::prelude::*;

#[derive(Clone, Copy)]
pub struct MenuEntry {
    pub id: &'static str,
    pub path: &'static str,
    pub label: &'static str,
    pub shortcut: &'static str,
    handler: Handler,
}

impl MenuEntry {
    /// The key the native menu bar shows and takes as its key equivalent:
    /// only ⌘ keys ("Ctrl" in portable notation). ⌥-only and bare keys type
    /// text in a field, the system owns ⌘H ⌘M ⌘Q ⌘Space, and a sheet's own
    /// keys (⌘G…) must still reach the sheet (docs/menus.md, rule 7).
    pub fn menu_key(&self) -> Option<&str> {
        let key = self.shortcut;
        let system = ["Ctrl+H", "Ctrl+M", "Ctrl+Q", "Ctrl+W", "Ctrl+Space", "Ctrl+Tab"];
        (key.split('+').any(|m| m == "Ctrl")
            && !system.contains(&key)
            && !crate::sheet_search::sheet_keys().any(|k| k == key))
        .then_some(key)
    }

    /// A separator goes before this row in its submenu.
    pub fn starts_group(&self) -> bool {
        BREAKS.contains(&self.id)
    }

    /// Shown as a check row (the state comes from the window).
    pub fn checkable(&self) -> bool {
        CHECKS.contains(&self.id)
    }
}

/// The path of a command with a key and a palette row but no menu row:
/// a shortcut for something a menu already offers as choices.
pub const PALETTE_ONLY: &str = "";

/// Rows a submenu separates from the rows before them.
const BREAKS: &[&str] = &[
    "export:ifc", "export_image", "export_igz",
    "scene_previous", "organise-scenes",
    "rest_intensity", "hide_similar",
    "form-style", "maximize_model_view",
];

/// Check rows: their state is mirrored from the window.
pub const CHECKS: &[&str] = &[
    "scene_play", "scene_transitions", "hide_similar", "rest_normal", "rest_fade", "rest_hide",
];

#[derive(Clone, Copy)]
enum Handler {
    Window(&'static str),
    Web(&'static str),
    PluginsFolder,
    Argument(&'static str, &'static str),
    Tool(&'static str),
    View(&'static str),
    Scene(&'static str),
    Playback,
    Transitions,
    Step(i32),
    RestMode(&'static str),
    DisplaySettings,
    Review,
    Export(&'static str),
    /// `shell_files::command` runs it before the model menu is asked.
    Shell,
}

pub fn entries() -> &'static [MenuEntry] {
    static ENTRIES: std::sync::OnceLock<Vec<MenuEntry>> = std::sync::OnceLock::new();
    ENTRIES.get_or_init(build_entries)
}

/// The commands of the menu bar that are not plain rows of
/// `desktop_menu_model::GROUPS`, with their home. `GROUPS` places the
/// top-level ones (`Row::Entry`) and the submenus (`Row::Sub`); a
/// submenu lists its rows in this order.
#[rustfmt::skip]
fn build_entries() -> Vec<MenuEntry> {
    use Handler::*;
    let mut entries = Vec::new();
    let mut add = |id, path, label, shortcut, handler| {
        entries.push(MenuEntry { id, path, label, shortcut, handler });
    };
    // File
    add("recover_discarded", "File", "Recover a Discarded Auto-Save…", "", Window("_on_recover_discarded"));
    add("import:obj", "File ▸ Import", "Wavefront OBJ (.obj)…", "", Window("_on_import_obj"));
    add("import:step", "File ▸ Import", "STEP (.step, .stp)…", "", Window("_on_import_step"));
    add("import:stl", "File ▸ Import", "STL (.stl)…", "", Window("_on_import_stl"));
    for (id, label, method) in [
        ("import:photomesh", "Photogrammetric Mesh (WebODM)…", "_on_import_photomesh"),
        ("import:alignment", "Georeference (KML / GeoJSON)…", "_on_import_georef"),
        ("import:survey_points", "Survey Points (UTM CSV)…", "_on_import_survey_points"),
        ("import:orthomosaic", "Orthomosaic (GeoTIFF)…", "_on_import_orthophoto"),
        ("import:image", "Image (PNG / JPG)…", "_on_import_image"),
    ] {
        add(id, "File ▸ Import", label, "", Window(method));
    }
    // Export ▸: the handover, the single formats, the views, the legacy copy.
    add("delivery_export", "File ▸ Export", "Deliveries…", "", Shell);
    for (id, label, fmt) in [
        ("export:ifc", "IFC (.ifc)…", "ifc"),
        ("export:glb", "glTF (.glb)…", "glb"),
        ("export:dae", "COLLADA (.dae)…", "dae"),
        ("export:step", "STEP (.step)…", "step"),
        ("export:stl", "STL (.stl)…", "stl"),
        ("export:obj", "Wavefront OBJ (.obj)…", "obj"),
    ] {
        add(id, "File ▸ Export", label, "", Export(fmt));
    }
    add("export_image", "File ▸ Export", "Current View as Image…", "", Shell);
    add("export_view_svg", "File ▸ Export", "Current View as SVG…", "", Shell);
    add("export:dxf", "File ▸ Export", "Current View as DXF…", "", Export("dxf"));
    add("export:video", "File ▸ Export", "Scenes as Video…", "", Window("export_video"));
    add("export_igz", "File ▸ Export", "Save a Copy as .igz (IngeTrazo)…", "", Shell);
    add("recover_exact", "Tools", "Recover Exact Geometry…", "", Window("_on_recover_exact"));
    add("make_faceted", "Tools", "Make Faceted", "", Window("_on_make_faceted"));
    // Edit
    for (id, label, keys, method) in [
        ("repeat_last_command", "Repeat Last Command", "Shift+R", "repeat_last_command"),
        ("invert_selection", "Invert Selection", "Ctrl+Shift+I", "_on_invert_selection"),
        ("make_group", "Make Group", "Ctrl+G", "_on_make_group"),
        ("verb:make_component", "Make Component…", "G", "_on_make_component"),
        ("explode_group", "Explode Group", "Ctrl+Shift+G", "_on_explode_group"),
        ("split_pieces", "Split into Pieces", "", "_on_split_into_pieces"),
        ("hide", "Hide", "Ctrl+H", "_on_hide"),
        ("delete_guides", "Delete Guides", "", "_on_delete_guides"),
        ("reverse_faces", "Reverse Faces", "", "_on_reverse_faces"),
        ("orient_faces", "Orient Faces", "", "_on_orient_faces"),
        ("heal_overlaps", "Heal Overlapping Faces", "", "_on_heal_overlaps"),
        ("rebuild_planar", "Rebuild Faces (Planar)", "", "_on_rebuild_planar"),
        ("join_segments", "Join Segments", "", "_on_join_segments"),
        ("verb:simplify_mesh", "Simplify Mesh…", "", "_on_simplify_mesh"),
        ("convert_path", "Convert Path to Geometry", "", "_on_convert_geopath"),
    ] {
        add(id, "Edit", label, keys, Window(method));
    }
    for (id, label, keys, method) in [
        ("unhide_selected", "Selected", "", "_on_unhide_selected"),
        ("unhide_last", "Last", "", "_on_unhide_last"),
        ("unhide_all", "All", "Ctrl+Shift+H", "_on_unhide_all"),
    ] {
        add(id, "Edit ▸ Unhide", label, keys, Window(method));
    }
    for (id, label, mode) in [
        ("intersect_model", "With Model", "model"),
        ("intersect_selection", "With Selection", "selection"),
        ("intersect_context", "With Context", "context"),
    ] {
        add(id, "Edit ▸ Intersect Faces", label, "", Argument("_on_intersect_faces", mode));
    }
    // View: the registry's camera and display commands, then scenes.
    for (group, id, label, keys, _) in crate::commands::view_commands() {
        let path = match (group, id) {
            (_, "zoom_extents") => continue,
            (_, "style_xray") => PALETTE_ONLY,
            (_, "form-style") => "View ▸ Style",
            (_, "section_cuts") => "View ▸ Sections",
            ("Style", _) => "View ▸ Style",
            ("Projection", _) => "View ▸ Projection",
            ("Views", _) if id.starts_with("view_") => "View ▸ Standard Views",
            ("walkthrough", _) => "View ▸ Walkthrough",
            ("Sections", _) => "View ▸ Sections",
            ("Scenes", _) => continue, // after the scene commands, below
            ("Sidebar" | "Activity", _) => "Window",
            _ => "View",
        };
        add(id, path, label, keys.first().copied().unwrap_or(""), View(id));
    }
    add("tool:first_person", "View ▸ Walkthrough", "First Person", "", Tool("first_person"));
    add("toggle_projection", PALETTE_ONLY, "Toggle Perspective / Parallel", "Shift+P", Window("_toggle_projection"));
    add("verb:field_of_view", "View", "Field of View…", "", Window("_ask_field_of_view"));
    for (id, label, action) in [
        ("scene_add", "Add Scene", "add"), ("scene_show", "Show Scene", "show"),
        ("scene_update", "Update Scene", "update"), ("scene_rename", "Rename Scene…", "rename"),
        ("scene_duplicate", "Duplicate Scene", "duplicate"), ("scene_delete", "Delete Scene", "delete"),
    ] {
        add(id, "View ▸ Scenes", label, "", Scene(action));
    }
    add("scene_previous", "View ▸ Scenes", "Previous Scene", "PgUp", Step(-1));
    add("scene_next", "View ▸ Scenes", "Next Scene", "PgDown", Step(1));
    add("organise-scenes", "View ▸ Scenes", "Organise Scenes", "", View("organise-scenes"));
    add("scene_transitions", "View ▸ Animation", "Scene Transitions", "", Transitions);
    add("scene_play", "View ▸ Animation", "Play / Stop", "", Playback);
    add("animation_settings", "View ▸ Animation", "Animation Settings…", "", Window("ask_animation"));
    for (id, label, mode) in [("rest_normal", "Show Normally", "normal"), ("rest_fade", "Fade", "fade"), ("rest_hide", "Hide (Fastest)", "hide")] {
        add(id, "View ▸ Rest of Model While Editing", label, "", RestMode(mode));
    }
    add("rest_intensity", "View ▸ Rest of Model While Editing", "Fade Intensity…", "", DisplaySettings);
    add("hide_similar", "View ▸ Rest of Model While Editing", "Hide Similar Components", "Alt+W", Window("_on_toggle_hide_similar"));
    add("hide_rest", PALETTE_ONLY, "Toggle Hide Rest of Model", "Alt+Q", Window("_on_toggle_hide_rest"));
    for (id, key, label) in crate::model_views::presets() {
        add(id, "View ▸ Model Views", label, "", Argument("set_view_layout", key));
    }
    add("maximize_model_view", "View ▸ Model Views", "Maximize / Restore View", "Ctrl+Space", Window("_maximize_model_view"));
    // Draw and Tools: the tools you arm, then what runs at once.
    for (id, path, label, tool) in [
        ("tool:line", "Draw", "Line", "line"),
        ("tool:freehand", "Draw", "Freehand", "freehand"),
        ("tool:geopath", "Draw", "Path", "geopath"),
        ("tool:rectangle", "Draw", "Rectangle", "rectangle"),
        ("tool:rotated_rect", "Draw", "Rotated Rectangle", "rotated_rect"),
        ("tool:circle", "Draw", "Circle", "circle"),
        ("tool:polygon", "Draw", "Polygon", "polygon"),
        ("tool:arc", "Draw ▸ Arcs", "Arc", "arc"),
        ("tool:arc3", "Draw ▸ Arcs", "3-Point Arc", "arc3"),
        ("tool:center_arc", "Draw ▸ Arcs", "Center Arc", "center_arc"),
        ("tool:pie", "Draw ▸ Arcs", "Pie", "pie"),
        ("tool:select", "Tools", "Select", "select"),
        ("tool:eraser", "Tools", "Eraser", "eraser"),
        ("tool:paint", "Tools", "Paint", "paint"),
        ("tool:pushpull", "Tools", "Push/Pull", "pushpull"),
        ("tool:move", "Tools", "Move", "move"),
        ("tool:rotate", "Tools", "Rotate", "rotate"),
        ("tool:scale", "Tools", "Scale", "scale"),
        ("tool:flip", "Tools", "Flip", "flip"),
        ("tool:followme", "Tools", "Follow Me", "followme"),
        ("tool:offset", "Tools", "Offset", "offset"),
        ("tool:fillet", "Tools", "Fillet", "fillet"),
        ("tool:chamfer", "Tools", "Chamfer", "chamfer"),
        ("tool:outer_shell", "Tools", "Outer Shell", "outer_shell"),
        ("tool:solid_intersect", "Tools ▸ Solid Tools", "Intersect", "solid_intersect"),
        ("tool:solid_union", "Tools ▸ Solid Tools", "Union", "solid_union"),
        ("tool:solid_subtract", "Tools ▸ Solid Tools", "Subtract", "solid_subtract"),
        ("tool:solid_trim", "Tools ▸ Solid Tools", "Trim", "solid_trim"),
        ("tool:solid_split", "Tools ▸ Solid Tools", "Split", "solid_split"),
        ("tool:tape", "Tools", "Tape Measure", "tape"),
        ("tool:protractor", "Tools", "Protractor", "protractor"),
        ("tool:dimension", "Tools", "Dimension", "dimension"),
        ("tool:text", "Tools", "Text", "text"),
        ("tool:section", "Tools", "Section Plane", "section"),
        ("tool:north", "Tools", "North", "north"),
    ] {
        add(id, path, label, "", Tool(tool));
    }
    for (id, path, label, method) in [
        ("bridge", "Tools", "Bridge", "_on_bridge"),
        ("make_faces", "Tools", "Make Faces", "_on_make_faces"),
        ("text3d", "Tools", "3D Text…", "_on_insert_3d_text"),
        ("terrain_profile", "Tools", "Terrain Profile of Selection", "_on_terrain_profile"),
        ("clear_texture_cache", "Tools", "Clear Imported Texture Cache…", "_on_clear_texture_cache"),
        ("documentation", "Help", "Documentation", "_on_documentation"),
        ("get_models", "Help", "Get More Models and Textures…", "_on_get_models"),
    ] {
        add(id, path, label, "", Window(method));
    }
    add("plugins_folder", "Extensions", "Open Extensions Folder", "", Handler::PluginsFolder);
    add("develop_plugin", "Extensions", "Develop an Extension…", "",
        Handler::Web("https://github.com/mle-gall/built/blob/main/docs/plugins.md"));
    add("review", "Window", "Review (Hide Panels)", "Ctrl+0", Review);
    entries
}

#[pyfunction]
pub fn run_model_menu(win: &Bound<'_, PyAny>, id: &str) -> PyResult<bool> {
    let Some(entry) = entries().iter().find(|row| row.id == id) else {
        return Ok(false);
    };
    match entry.handler {
        Handler::PluginsFolder => {
            let py = win.py();
            let folder = py.import("built_core.extensions")?.call_method0("user_plugins_dir")?;
            std::fs::create_dir_all(folder.extract::<std::path::PathBuf>()?)?;
            py.import("built_core.desktop_open")?.call_method1("open_url", (folder,))?
        }
        Handler::Web(url) => win.py().import("built_core.desktop_open")?.call_method1("open_url", (url,))?,
        Handler::Shell => return Ok(false),
        Handler::Window(method) => win.call_method0(method)?,
        Handler::Argument(method, arg) => win.call_method1(method, (arg,))?,
        Handler::Tool(tool) => win.call_method1("_activate_tool", (tool,))?,
        Handler::Export(fmt) => win
            .py()
            .import("built_render")?
            .call_method1("delivery_export_form", (win, win.py().None(), true, fmt))?,
        Handler::RestMode(mode) => win
            .getattr("viewport")?
            .call_method1("set_edit_rest_mode", (mode,))?,
        Handler::DisplaySettings => win
            .py()
            .import("built_render.settings_sheet")?
            .call_method1("show", (win, "display"))?,
        Handler::Review => {
            let view = win.getattr("view")?;
            let renderer = view.getattr("renderer")?;
            if renderer.is_none() { return Ok(false); }
            renderer.call_method0("ui_toggle_review")?;
            view.call_method0("update")?
        }
        Handler::Playback => {
            let playing = win
                .getattr("scene_player")?
                .call_method0("is_playing")?
                .is_truthy()?;
            win.call_method1("play_scenes", (!playing,))?
        }
        Handler::Transitions => {
            let anim = win
                .getattr("viewport")?
                .getattr("scene")?
                .getattr("animation")?;
            win.call_method1(
                "set_scene_transitions",
                (!anim.getattr("enabled")?.is_truthy()?,),
            )?
        }
        Handler::Step(delta) => win.call_method1("step_scene", (delta,))?,
        Handler::Scene(action) => {
            let marked = win.getattr("_scene_tabs")?.call_method0("marked_view")?;
            let vp = win.getattr("viewport")?;
            if action != "add"
                && !vp
                    .getattr("scene")?
                    .getattr("saved_views")?
                    .contains(&marked)?
            {
                win.call_method0("_no_scene_selected")?
            } else if matches!(action, "show" | "add" | "duplicate") {
                let scenes = win.getattr("tray")?.getattr("scenes")?;
                let view = match action {
                    "add" => scenes.call_method0("add_scene")?,
                    "duplicate" => scenes.call_method1("duplicate_view", (marked,))?,
                    _ => { scenes.call_method1("show_view", (&marked,))?; marked }
                };
                if action == "show" && !view.is_none() {
                    let values = pyo3::types::PyDict::new(win.py());
                    values.set_item("name", view.getattr("name")?)?;
                    let caption = "Scene '{name}'";
                    let text = crate::model_edit::translated(win.py(), caption, Some(&values))?;
                    win.call_method0("statusBar")?.call_method1("showMessage", (text, 2000))?;
                }
                win.py().None().into_bound(win.py())
            } else {
                win.py()
                    .import("views.wgpu_ui")?
                    .call_method1("_run", (win, [format!("tab:scene:{action}")]))?
            }
        }
        Handler::View(key) => win
            .py()
            .import("views.view_commands")?
            .call_method1("run", (win, key))?,
    };
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_leaf_has_one_id_and_one_menu_location() {
        let mut ids = HashSet::new();
        let mut locations = HashSet::new();
        for row in entries() {
            assert!(ids.insert(row.id), "duplicate command {}", row.id);
            assert!(
                locations.insert((row.path, row.label)),
                "duplicate menu label {}",
                row.label
            );
        }
    }
}
