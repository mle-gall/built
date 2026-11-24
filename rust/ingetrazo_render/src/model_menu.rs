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
}

pub fn entries() -> &'static [MenuEntry] {
    static ENTRIES: std::sync::OnceLock<Vec<MenuEntry>> = std::sync::OnceLock::new();
    ENTRIES.get_or_init(build_entries)
}

#[rustfmt::skip]
fn build_entries() -> Vec<MenuEntry> {
    use Handler::*;
    let mut entries = Vec::new();
    let mut add = |id, path, label, shortcut, handler| {
        entries.push(MenuEntry { id, path, label, shortcut, handler });
    };
    add("import:obj", "File ▸ Import", "Wavefront OBJ (.obj)…", "", Window("_on_import_obj"));
    add("import:stl", "File ▸ Import", "STL mesh (.stl)…", "", Window("_on_import_stl"));
    for (id, label, method) in [
        ("import:photomesh", "Photogrammetric mesh (WebODM)…", "_on_import_photomesh"),
        ("import:alignment", "Georeference (KML / GeoJSON)…", "_on_import_georef"),
        ("import:survey_points", "Survey points CSV (UTM)…", "_on_import_survey_points"),
        ("import:orthomosaic", "Orthomosaic (GeoTIFF)…", "_on_import_orthophoto"),
        ("import:image", "Image (PNG / JPG)…", "_on_import_image"),
        ("clear_texture_cache", "Clear imported texture cache…", "_on_clear_texture_cache"),
    ] {
        add(id, "File ▸ Import", label, "", Window(method));
    }
    for (id, label, fmt) in [
        ("export:ifc", "IFC (BIM)…", "ifc"),
        ("export:glb", "glTF / GLB…", "glb"),
        ("export:dae", "COLLADA (.dae)…", "dae"),
        ("export:stl", "STL (3D printing)…", "stl"),
        ("export:obj", "Wavefront OBJ (.obj)…", "obj"),
        ("export:dxf", "Current view as DXF…", "dxf"),
    ] {
        add(id, "File ▸ Export", label, "", Export(fmt));
    }
    for (id, label, action) in [
        ("scene_add", "Add Scene", "add"), ("scene_show", "Show Scene", "show"),
        ("scene_update", "Update Scene", "update"), ("scene_rename", "Rename Scene…", "rename"),
        ("scene_duplicate", "Duplicate Scene", "duplicate"), ("scene_delete", "Delete Scene", "delete"),
    ] {
        add(id, "Camera ▸ Scenes", label, "", Scene(action));
    }
    for (id,label,shortcut,handler) in [
        ("scene_transitions","Scene Transitions","",Transitions), ("scene_play","Play / Stop","",Playback),
        ("scene_next","Next Scene","PgDown",Step(1)), ("scene_previous","Previous Scene","PgUp",Step(-1)),
        ("animation_video","Export Video…","",Window("export_video")),
        ("animation_settings","Animation Settings…","",Window("ask_animation")),
    ] { add(id,"Camera ▸ Animation",label,shortcut,handler); }
    for (id, key, label) in crate::model_views::presets() {
        add(id, "Window ▸ Model Views", label, "", Argument("set_view_layout", key));
    }
    add("review", "Window", "Review", "Ctrl+0", Review);
    add("maximize_model_view", "Window", "Maximize / Restore View", "Ctrl+Space", Window("_maximize_model_view"));
    add("export:video","File ▸ Export","Scenes as Video…","",Window("export_video"));
    for (id,label,mode) in [("rest_normal","Show normally","normal"),("rest_fade","Fade","fade"),("rest_hide","Hide (fastest)","hide")] {
        add(id,"Camera ▸ Rest of model while editing",label,"",RestMode(mode));
    }
    add("rest_intensity","Camera ▸ Rest of model while editing","Fade intensity…","",DisplaySettings);
    add("hide_rest","Camera ▸ Component Edit","Hide Rest of Model","Alt+Q",Window("_on_toggle_hide_rest"));
    add("hide_similar","Camera ▸ Component Edit","Hide Similar Components","Alt+W",Window("_on_toggle_hide_similar"));
    for (id, label, keys, method) in [
        ("repeat_last_command", "Repeat last command", "Shift+R", "repeat_last_command"),
        ("invert_selection", "Invert Selection", "Ctrl+Shift+I", "_on_invert_selection"),
        ("make_group", "Make Group", "Ctrl+G", "_on_make_group"),
        ("verb:make_component", "Make Component…", "G", "_on_make_component"),
        ("explode_group", "Explode Group", "Ctrl+Shift+G", "_on_explode_group"),
        ("split_pieces", "Split into Pieces", "", "_on_split_into_pieces"),
        ("verb:simplify_mesh", "Simplify Mesh…", "", "_on_simplify_mesh"),
        ("convert_path", "Convert Path to Geometry", "", "_on_convert_geopath"),
        ("delete_guides", "Delete Guides", "", "_on_delete_guides"),
        ("hide", "Hide", "Ctrl+H", "_on_hide"),
        ("reverse_faces", "Reverse Faces", "", "_on_reverse_faces"),
        ("orient_faces", "Orient Faces", "", "_on_orient_faces"),
        ("heal_overlaps", "Heal Overlapping Faces", "", "_on_heal_overlaps"),
        ("join_segments", "Join Segments", "", "_on_join_segments"),
        ("rebuild_planar", "Rebuild Faces (Planar)", "", "_on_rebuild_planar"),
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
    for (id, path, label, tool) in [
        ("tool:freehand", "Draw", "Freehand", "freehand"),
        ("tool:geopath", "Draw", "Path", "geopath"),
        ("tool:arc", "Draw ▸ Arcs", "Arc", "arc"),
        ("tool:arc3", "Draw ▸ Arcs", "3-Point Arc", "arc3"),
        ("tool:center_arc", "Draw ▸ Arcs", "Center Arc", "center_arc"),
        ("tool:pie", "Draw ▸ Arcs", "Pie", "pie"),
        ("tool:rotated_rect", "Draw ▸ Shapes", "Rotated Rectangle", "rotated_rect"),
        ("tool:polygon", "Draw ▸ Shapes", "Polygon", "polygon"),
        ("tool:first_person", "Camera ▸ Walkthrough", "First Person", "first_person"),
        ("tool:scale", "Tools", "Scale", "scale"),
        ("tool:flip", "Tools", "Flip", "flip"),
        ("tool:followme", "Tools", "Follow Me", "followme"),
        ("tool:fillet", "Tools", "Fillet 3D", "fillet"),
        ("tool:protractor", "Tools", "Protractor", "protractor"),
        ("tool:north", "Tools", "North", "north"),
        ("tool:dimension", "Tools", "Dimension", "dimension"),
        ("tool:text", "Tools", "Text", "text"),
        ("tool:section", "Tools", "Section Plane", "section"),
        ("tool:outer_shell", "Tools", "Outer Shell", "outer_shell"),
        ("tool:solid_intersect", "Tools ▸ Solid Tools", "Intersect", "solid_intersect"),
        ("tool:solid_union", "Tools ▸ Solid Tools", "Union", "solid_union"),
        ("tool:solid_subtract", "Tools ▸ Solid Tools", "Subtract", "solid_subtract"),
        ("tool:solid_trim", "Tools ▸ Solid Tools", "Trim", "solid_trim"),
        ("tool:solid_split", "Tools ▸ Solid Tools", "Split", "solid_split"),
    ] {
        add(id, path, label, "", Tool(tool));
    }
    for (id, path, label, method) in [
        ("toggle_projection", "Camera", "Toggle Perspective / Parallel", "_toggle_projection"),
        ("verb:field_of_view", "Camera", "Field of View…", "_ask_field_of_view"),
        ("bridge", "Tools", "Bridge", "_on_bridge"),
        ("make_faces", "Tools", "Make Faces", "_on_make_faces"),
        ("text3d", "Tools", "3D Text…", "_on_insert_3d_text"),
        ("terrain_profile", "Tools", "Terrain profile of selection", "_on_terrain_profile"),
        ("recover_discarded", "File", "Recover a discarded auto-save…", "_on_recover_discarded"),
        ("documentation", "Help", "Documentation", "_on_documentation"),
        ("get_models", "Help", "Get more models and textures…", "_on_get_models"),
    ] {
        add(id, path, label, if id == "toggle_projection" { "Shift+P" } else { "" }, Window(method));
    }
    add("plugins_folder", "Plugins", "Open extensions folder", "", Handler::PluginsFolder);
    add("develop_plugin", "Plugins", "Develop an extension…", "",
        Handler::Web("https://github.com/ingelibre/ingetrazo/blob/main/docs/plugins.md"));
    for (group, id, label, _, _) in crate::commands::view_commands() {
        let path = match group {
            "Style" => "Camera ▸ Style",
            "Projection" => "Camera ▸ Projection",
            "Views" if id.starts_with("view_") => "Camera ▸ Standard Views",
            "walkthrough" => "Camera ▸ Walkthrough",
            "Sections" => "Camera ▸ Sections",
            "Settings" => continue,
            _ => "Camera",
        };
        if !matches!(id, "zoom_extents" | "view_top" | "view_front" | "view_right" | "view_iso") {
            add(id, path, label, "", View(id));
        }
    }
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
            let folder = py.import("core.extensions")?.call_method0("user_plugins_dir")?;
            std::fs::create_dir_all(folder.extract::<std::path::PathBuf>()?)?;
            py.import("core.desktop_open")?.call_method1("open_url", (folder,))?
        }
        Handler::Web(url) => win.py().import("core.desktop_open")?.call_method1("open_url", (url,))?,
        Handler::Window(method) => win.call_method0(method)?,
        Handler::Argument(method, arg) => win.call_method1(method, (arg,))?,
        Handler::Tool(tool) => win.call_method1("_activate_tool", (tool,))?,
        Handler::Export(fmt) => win
            .py()
            .import("views.wgpu_deliveries")?
            .call_method1("export_form", (win, win.py().None(), true, fmt))?,
        Handler::RestMode(mode) => win
            .getattr("viewport")?
            .call_method1("set_edit_rest_mode", (mode,))?,
        Handler::DisplaySettings => win
            .py()
            .import("views.settings_sheet")?
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
                if !view.is_none() {
                    let values = pyo3::types::PyDict::new(win.py());
                    values.set_item("name", view.getattr("name")?)?;
                    let caption = if action == "show" { "Scene '{name}'" } else { "Scene '{name}' added" };
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
