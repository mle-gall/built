// SPDX-License-Identifier: GPL-3.0-or-later
//! The chrome's actions (`views.wgpu_ui._run`): what egui queued during a
//! frame ("kind:arg"), run after it; the settings forms that mirror a
//! panel's controls; and the command search.
use super::chrome_frame::{attr, own, render, ui};
use super::*;
use pyo3::exceptions::{PyException, PyKeyError, PyOSError, PyRuntimeError, PyValueError};

fn partition(text: &str) -> (&str, &str) {
    text.split_once(':').unwrap_or((text, ""))
}
fn empty<'py>(py: Python<'py>) -> Bound<'py, PyList> {
    PyList::empty(py)
}
/// `getattr(getattr(win, "_doc", None), "stand_in", None)`.
fn stand_in<'py>(win: &Obj<'py>) -> Reply<'py> {
    attr(&attr(win, "_doc")?, "stand_in")
}
fn flash(viewport: &Obj<'_>, message: &Obj<'_>) -> PyResult<()> {
    viewport.call_method1("flash_status", (message.str()?,)).map(drop)
}
pub(super) fn prompt<'py>(view: &Obj<'py>, title: impl IntoPyObject<'py>, body: impl IntoPyObject<'py>, yes: impl IntoPyObject<'py>) -> PyResult<()> {
    let py = view.py();
    let buttons = PyList::new(py, [("yes", yes.into_bound_py_any(py)?), ("no", "Cancel".into_bound_py_any(py)?)])?;
    view.getattr("renderer")?.call_method1("ui_set_prompt", (title, body, buttons)).map(drop)
}

/// Where the model is seen, in device coordinates (ui.rs `free_ndc`): the
/// window minus the rail, dock and bars showing. None without the chrome.
pub(super) fn free_area<'py>(a: &Args<'py>) -> Reply<'py> {
    let view = stand_in(&a.get_item(0)?)?;
    if view.is_none() || !attr_flag(&view, "_ui_attached", false)? {
        return Ok(none(a.py()));
    }
    view.getattr("renderer")?.call_method0("ui_free_ndc")
}

/// Review frames the model into the window the panels left (B8); leaving
/// it, the camera goes back to where it was if Review's framing is
/// untouched.
pub(super) fn review_camera<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let win = a.get_item(0)?;
    let state = win.getattr("__dict__")?;
    if a.get_item(1)?.is_truthy()? {
        let before = win.call_method0("_camera_pose")?;
        win.call_method0("_on_zoom_extents")?;
        state.set_item("_review_back", (before, win.call_method0("_camera_pose")?))?;
        return Ok(none(py));
    }
    let back = state.call_method1("pop", ("_review_back", py.None()))?;
    if !back.is_none() && back.get_item(1)?.eq(win.call_method0("_camera_pose")?)? {
        let viewport = win.getattr("viewport")?;
        let camera = viewport.getattr("camera")?;
        let [x, y, z, distance, yaw, pitch] = unpack::<6>(&back.get_item(0)?)?;
        camera.setattr("distance", distance)?;
        camera.setattr("yaw", yaw)?;
        camera.setattr("pitch", pitch)?;
        camera.setattr("target", (x, y, z))?;
        viewport.call_method0("update")?;
    }
    Ok(none(py))
}

pub(super) fn run<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (win, source_view) = (a.get_item(0)?, a.get_item(2)?);
    if win.is_none() {
        return Ok(none(py));
    }
    for action in a.get_item(1)?.try_iter()? {
        run_one(&win, &action?, &source_view)?;
    }
    Ok(none(py))
}

fn run_one<'py>(win: &Obj<'py>, action: &Obj<'py>, source_view: &Obj<'py>) -> PyResult<()> {
    let py = win.py();
    let module = ui(py)?;
    let glue = render(py)?;
    let dialogs = glue.getattr("chrome_dialogs")?;
    let library = || glue.getattr("wgpu_library");
    let text: String = action.extract()?;
    let (kind, arg) = partition(&text);
    let viewport = || win.getattr("viewport");
    match kind {
        "extension" => return win.call_method1("run_command", (action,)).map(drop),
        "scenephoto" => {
            let view = dialogs.call_method1("view_of", (win,))?;
            if !view.is_none() {
                let (pixels, failure) = match glue.call_method1("render_scene_photo", (win, arg)) {
                    Ok(pixels) => (pixels, None),
                    // The visible row retries a missed frame.
                    Err(error) if error.is_instance_of::<PyRuntimeError>(py) || error.is_instance_of::<PyValueError>(py) || error.is_instance_of::<PyOSError>(py) => (none(py), None),
                    Err(error) => (none(py), Some(error)),
                };
                view.getattr("renderer")?.call_method1("ui_finish_scene_photo", (arg, pixels))?;
                if let Some(error) = failure {
                    return Err(error);
                }
            }
            return Ok(());
        }
        "scenephotomode" => return glue.call_method1("save_thumbnail_mode", (arg == "1",)).map(drop),
        "font" => return glue.getattr("font_picker")?.call_method1("answer", (win, arg, source_view)).map(drop),
        "review" => return direct(py, review_camera, (win, arg == "1")).map(drop),
        "sheet" => return dialogs.call_method1("answer", (win, arg)).map(drop),
        "job" => {
            let (verb, job) = partition(arg);
            let job = dialogs.call_method1("find_job", (win, job))?;
            if verb == "cancel" && !job.is_none() {
                job.call_method0("cancel")?;
            }
            return Ok(());
        }
        "dlg" => {
            if dialogs.call_method1("current_sheet", (win,))?.is_none() {
                glue.getattr("chrome_forms")?.call_method1("action", (win, arg))?;
            }
            return Ok(());
        }
        _ => {}
    }
    if dialogs.call_method1("modal", (win,))?.is_truthy()? {
        return Ok(()); // a click queued before the sheet came up
    }
    match kind {
        "plugin" => return crate::plugin_panels::dispatch(win, pyo3::types::PyString::new(py, arg).as_any()),
        "task" => return crate::tasks_host::task_action(win, arg),
        "activity" | "activity2" | "dismiss" | "pause-toast" | "resume-toast" => {
            for row in dialogs.call_method1("history", (win,))?.try_iter()? {
                let row = row?;
                if row.getattr("id")?.eq(arg)? {
                    match kind {
                        "pause-toast" | "resume-toast" => { row.call_method1("hover", (kind == "pause-toast",))?; }
                        _ => {
                            row.setattr("toast", false)?;
                            let cb = row.getattr(if kind == "activity2" { "second_action" } else { "action" })?;
                            if kind != "dismiss" && cb.is_callable() { cb.call0()?; }
                        }
                    }
                    break;
                }
            }
            return Ok(());
        }
        "linkbanner" => {
            if arg == "locate" {
                let links = py.import("built_core.links")?;
                let scene = viewport()?.getattr("scene")?;
                for group in links.call_method1("linked_groups", (&scene,))?.try_iter()? {
                    let group = group?;
                    let path = links.call_method1("link_of", (&group,))?.get_item("path")?;
                    if links.call_method1("link_state", (&scene, path))?.eq("missing")? {
                        if group.is_truthy()? {
                            module.call_method1("_link_action", (win, dialogs.call_method1("view_of", (win,))?, group, "linkrelink"))?;
                        }
                        break;
                    }
                }
                dialogs.call_method1("update_link_banner", (win,))?;
            } else {
                win.getattr("_doc")?.setattr("_missing_link_banner", "")?;
            }
            return dialogs.call_method1("_repaint", (win,)).map(drop);
        }
        "lsheet" => return glue.getattr("chrome_sheet")?.call_method1("action", (win, arg)).map(drop),
        "start" => return glue.getattr("wgpu_start")?.call_method1("action", (win, arg)).map(drop),
        "sheets" => return py.import("built_render.sheet_workspace")?.call_method1("action", (win, arg)).map(drop),
        "tab" if arg == "scene:editcamera" => {
            return win.call_method1("edit_scene_camera", (glue.call_method1("marked_view", (win,))?,)).map(drop);
        }
        _ => {}
    }
    if kind == "panel" || (kind == "act" && arg == "camera") || (kind == "tab" && arg == "scene:camera") {
        let payload = if kind == "panel" { arg } else { "[\"camera.open\",null]" };
        return glue.call_method1("inspector_action", (win, win.getattr("_doc")?.getattr("stand_in")?, payload)).map(drop);
    }
    let view = stand_in(win)?;
    match kind {
        "drawer" => { crate::plugin_panels::open_panel(win, &arg.into_bound_py_any(py)?)?; }
        "tool" => {
            let run = match view.getattr_opt("_ui_palette_runs")? {
                Some(runs) => runs.call_method1("get", (arg,))?,
                None => none(py),
            };
            if run.is_none() {
                win.call_method1("_activate_tool", (arg,))?;
            } else {
                run.call0()?;
            }
        }
        "opt" => {
            if view.is_truthy()? {
                for option in module.call_method1("_options", (&view,))?.try_iter()? {
                    let [key, _, _, _, toggle] = unpack::<5>(&option?)?;
                    if key.eq(arg)? && toggle.is_truthy()? {
                        toggle.call0()?;
                        view.call_method0("update")?;
                        break;
                    }
                }
            }
        }
        "tree" => {
            module.call_method1("_tree_action", (win, &view, arg))?;
        }
        "pal" => {
            library()?.call_method1("paint_from_strip", (win, arg))?;
        }
        "libsheet" => {
            library()?.call_method1("open_library", (win, if arg.is_empty() { None } else { Some(arg) }))?;
        }
        "lib" => {
            library()?.call_method1("action", (win, &view, arg))?;
        }
        "dockw" => {
            module.call_method1("keep_dock_width", (&view, arg))?;
        }
        "cmdscope" | "cmdq" | "cmdrun" | "cmdshortcut" => {
            if !view.is_none() {
                module.call_method1("command_action", (&view, win, kind, arg))?;
                view.call_method0("update")?;
            }
        }
        "proj" => {
            module.call_method1("_project_action", (win, arg))?;
        }
        "ptab" => {
            py.import("views.wgpu_tabs")?.call_method1("action", (win, &view, arg))?;
        }
        "ctx" => {
            py.import("plugins._context_menu")?.call_method1("context_menu_action", (win, &view, arg))?;
        }
        "tab" => tab(win, &view, arg)?,
        "deliver" => {
            let data = match attr(&view, "_ui_tree")? {
                data if data.is_truthy()? => data,
                _ => PyDict::new(py).into_any(),
            };
            if arg == "unfiled" || arg == "problems" {
                library()?.call_method1("set_mode", (&view, win, "problems"))?;
            } else if arg == "active" && data.call_method1("get", ("active",))?.is_truthy()? {
                module.call_method1("_select", (win, direct(py, super::chrome_tree::tree_groups, (&view, data.get_item("active")?))?))?;
            } else {
                glue.call_method1("delivery_action", (win, &view, arg))?;
            }
        }
        "act" if arg == "organise-scenes" => {
            view.getattr("renderer")?.call_method0("ui_organise_scenes")?;
            view.call_method0("update")?;
        }
        "act" if matches!(arg, "form-style" | "form-shadows") => {
            win.call_method("open_preferences", (), Some(&fields!(py; "page" => "appearance")))?;
        }
        "act" if arg.starts_with("form-") => {
            if !view.is_none() {
                direct(py, open_form, (&view, &arg[5..]))?;
            }
        }
        "form" => {
            if !view.is_none() {
                direct(py, form_apply, (win, &view, arg))?;
            }
        }
        "act" if arg.into_bound_py_any(py)?.call_method0("isdecimal")?.is_truthy()? => {
            // The Sheets workspace retains its composer actions.
            let shown = if source_view.is_truthy()? { source_view.clone() } else { attr(&win.getattr("_doc")?, "stand_in")? };
            let workspace = py.import("built_render.sheet_workspace")?.call_method1("workspace_of", (shown,))?;
            if !workspace.is_none() {
                let index = py.get_type::<pyo3::types::PyInt>().call1((arg,))?;
                let actions = workspace.getattr("_actions")?;
                if index.lt(actions.len()?)? {
                    actions.get_item(index)?.call_method0("trigger")?;
                }
            }
        }
        "act" => {
            py.import("views.view_commands")?.call_method1("run", (win, arg))?;
        }
        "cmd" => {
            let selection = py.get_type::<PyList>().call1((viewport()?.getattr("scene")?.getattr("selection")?,))?;
            for verb in direct(py, super::chrome_frame::commands, (win, selection))?.try_iter()? {
                let [key, _, _, run, enabled, _] = unpack::<6>(&verb?)?;
                if key.eq(arg)? && enabled.is_truthy()? {
                    if !run.is_none() {
                        run.call0()?;
                    }
                    break;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// The bottom bar: the workspaces, the visibility sets and the scenes.
fn tab<'py>(win: &Obj<'py>, view: &Obj<'py>, arg: &str) -> PyResult<()> {
    let py = win.py();
    let module = ui(py)?;
    let viewport = win.getattr("viewport")?;
    let scene_tree = || py.import("built_core")?.getattr("scene_tree");
    let mark: String = module.getattr("ED")?.extract()?;
    let field = |name: &str, value: &str| format!("{mark}edit{mark}{name}{mark}{value}");
    if arg == "model" {
        win.call_method0("_show_model")?;
    } else if arg == "sheet" {
        win.call_method0("_to_sheets")?;
    } else if let Some(rest) = arg.strip_prefix("visibility:") {
        let (operation, key) = partition(rest);
        if ["save", "rename", "delete"].contains(&operation) {
            let mut label = String::new();
            if let Some(cache) = view.getattr_opt("_ui_visibility_cache")? {
                let wanted = format!("visibility:apply:{key}");
                for row in cache.get_item(2)?.try_iter()? {
                    let row = row?;
                    if row.get_item(0)?.eq(&wanted)? {
                        label = row.get_item(1)?.str()?.extract()?;
                        break;
                    }
                }
            }
            let (title, body) = match operation {
                "save" => ("Save visibility set".to_owned(), field("name", &label)),
                "rename" => ("Rename visibility set".to_owned(), field("name", &label)),
                _ => (format!("Delete “{label}”?"), String::new()),
            };
            view.setattr("_ui_pending", ("visibility_set", (viewport.getattr("scene")?, operation, key)))?;
            prompt(view, title, body, operation.into_bound_py_any(py)?.call_method0("capitalize")?)?;
        } else if super::chrome_tree_action::core_command(&viewport, "edit_visibility_set".into_bound_py_any(py)?.as_any(), Some(&fields!(py; "verb" => operation, "key" => key)))?.get_item("ok")?.is_truthy()? {
            viewport.call_method0("notify_scene_changed")?;
            view.call_method0("update")?;
        }
    } else if arg.starts_with("scenedelete:") {
        let targets = match scene_tree()?.call_method1("menu_selection", (viewport.getattr("scene")?, partition(arg).1)) {
            Ok(targets) => targets,
            Err(error) if error.is_instance_of::<PyValueError>(py) => return flash(&viewport, error.value(py)),
            Err(error) => return Err(error),
        };
        let count = targets.len()?;
        view.setattr("_ui_pending", ("scene_delete", targets))?;
        prompt(view, format!("Delete {count} scenes?"), "", "Delete")?;
    } else if arg.starts_with("scenefolders:") {
        let edited = (|| -> PyResult<()> {
            let add = attr(&win.getattr("tray")?.getattr("scenes")?, "add_scene_in_folder")?;
            scene_tree()?.call_method1("edit_menu_folder", (viewport.getattr("history")?, partition(arg).1, add))?;
            viewport.call_method0("notify_scene_changed")?;
            view.setattr("_ui_tabs", py.None())?;
            view.call_method0("update").map(drop)
        })();
        match edited {
            Err(error) if error.is_instance_of::<PyValueError>(py) => flash(&viewport, error.value(py))?,
            other => other?,
        }
    } else if ["scene:add", "scene:update", "scene:up", "scene:down", "scene:delete", "scene:rename", "scene:duplicate"].contains(&arg) {
        let panel = attr(&win.getattr("tray")?, "scenes")?;
        let row = attr(&attr(win, "_scene_tabs")?, "tabs")?;
        let marked = if row.is_none() { none(py) } else { row.call_method0("marked_view")? };
        if !panel.is_none() {
            let name = || -> PyResult<String> { marked.getattr("name")?.str()?.extract() };
            if arg == "scene:add" {
                panel.call_method0("add_scene")?;
            } else if marked.is_none() {
            } else if arg == "scene:duplicate" {
                panel.call_method1("duplicate_view", (&marked,))?;
            } else if arg == "scene:update" {
                panel.call_method1("update_view", (&marked,))?;
                viewport.call_method1("flash_status", (format!("Updated “{}” from the view", name()?),))?;
            } else if arg == "scene:up" || arg == "scene:down" {
                if scene_tree()?.call_method1("move_among_siblings", (viewport.getattr("history")?, &marked, arg == "scene:up"))?.is_truthy()? {
                    viewport.call_method0("notify_scene_changed")?;
                }
            } else if arg == "scene:rename" {
                view.setattr("_ui_pending", ("scene_rename", &marked))?;
                prompt(view, "Rename scene", field("scene", &name()?), "Rename")?;
            } else if arg == "scene:delete" {
                view.setattr("_ui_pending", ("scene_delete", PyList::new(py, [&marked])?))?;
                prompt(view, format!("Delete “{}”?", name()?), "The scene goes; the model does not change.", "Delete")?;
            }
            view.setattr("_ui_tabs", py.None())?;
        }
    } else if arg == "scene:prev" || arg == "scene:next" {
        win.call_method1("step_scene", (if arg == "scene:next" { 1 } else { -1 },))?;
    } else if let Some(index) = arg.strip_prefix("scene:") {
        let index = index.into_bound_py_any(py)?;
        if index.call_method0("isdecimal")?.is_truthy()? {
            let views = view.getattr_opt("_ui_scene_views")?.unwrap_or_else(|| empty(py).into_any());
            render(py)?.call_method1("show_listed_scene", (win, views, py.get_type::<pyo3::types::PyInt>().call1((index,))?))?;
        }
    }
    Ok(())
}

// ---- Settings forms ---------------------------------------------------------
// A form mirrors an existing panel's controls, so its logic (live apply,
// daylight range, the style library) stays the panel's.

/// A field's words: fixed, or read off the panel.
#[derive(Clone, Copy)]
enum Words {
    Plain(&'static str),
    /// The text of a label of the panel.
    Label(&'static str),
    /// A control's value and a suffix.
    Value(&'static str, &'static str),
    /// A control's value in `:g` and a suffix.
    Short(&'static str, &'static str),
    Date,
    SunTimes,
}
#[derive(Clone, Copy)]
enum Colour {
    /// A colour of the style as shown (`panel._shown_color`).
    Style(&'static str),
    Dimension,
}
#[derive(Clone, Copy)]
struct Field {
    kind: &'static str,
    /// The panel's control (or, for a button, its handler).
    control: Option<&'static str>,
    label: Words,
    /// A slider's value in words.
    shown: Option<Words>,
    colour: Option<Colour>,
    /// A button shows only while this control is enabled.
    gate: Option<&'static str>,
    scale: f64,
}
const fn field(kind: &'static str, control: Option<&'static str>, label: Words) -> Field {
    Field { kind, control, label, shown: None, colour: None, gate: None, scale: 1.0 }
}
const fn named(kind: &'static str, control: &'static str, label: &'static str) -> Field {
    field(kind, Some(control), Words::Plain(label))
}
const fn caption(label: &'static str) -> Field {
    field("caption", None, Words::Plain(label))
}
const fn colour(control: &'static str, label: &'static str, colour: Colour) -> Field {
    Field { colour: Some(colour), ..named("color", control, label) }
}
const fn slider(control: &'static str, label: &'static str, shown: Words) -> Field {
    Field { shown: Some(shown), ..named("slider", control, label) }
}

/// (title, the window's panel, its fields) of a form.
fn form(name: &str) -> Option<(&'static str, &'static str, &'static [Field])> {
    const STYLE: &[Field] = &[
        named("choice", "_combo", "Style"),
        named("choice", "_mode", "Faces"),
        caption("Edges"),
        named("check", "_edges", "Edges"),
        colour("_edge_c", "Edge colour", Colour::Style("edge_color")),
        named("check", "_profiles", "Profiles"),
        named("check", "_back_edges", "Back edges, dashed"),
        caption("Faces and background"),
        colour("_front_c", "Front colour", Colour::Style("front_color")),
        colour("_back_c", "Back colour", Colour::Style("back_color")),
        named("check", "_sky", "Sky"),
        colour("_sky_c", "Sky colour", Colour::Style("sky_color")),
        colour("_ground_c", "Ground", Colour::Style("ground_color")),
        colour("_bg_c", "Background", Colour::Style("background")),
        named("check", "_fill", "Section fill"),
        colour("_fill_c", "Section fill colour", Colour::Style("section_fill_color")),
        named("button", "_on_save", "Save style…"),
        Field { gate: Some("_del_btn"), ..named("button", "_on_delete", "Delete style") },
    ];
    const DIMENSIONS: &[Field] = &[
        named("choice", "_units", "Unit"),
        slider("_decimals", "Decimals", Words::Value("_decimals", "")),
        named("check", "_show_unit", "Show the unit"),
        slider("_font", "Text size", Words::Value("_font", " pt")),
        field("button", None, Words::Plain("Font family…")),
        colour("_color_btn", "Colour", Colour::Dimension),
        named("choice", "_ends", "Ends"),
        Field { scale: 2.0, ..slider("_base_step", "Baseline step", Words::Short("_base_step", " mm")) },
        field("text", None, Words::Plain("Applies to every dimension in the document.")),
    ];
    const SHADOWS: &[Field] = &[
        named("check", "_enable", "Show shadows"),
        slider("_doy", "Date", Words::Date),
        slider("_time", "Time", Words::Label("_time_lbl")),
        field("text", None, Words::SunTimes),
        slider("_dark", "Darkness", Words::Value("_dark", " %")),
        named("choice", "_tz", "Time zone"),
        caption("Site"),
        field("text", None, Words::Label("_site")),
        named("button", "_on_north_tool", "Set north on screen"),
        named("button", "_on_add_location", "Add location…"),
    ];
    match name {
        "style" => Some(("Style", "styles_panel", STYLE)),
        "dimstyle" => Some(("Dimension style", "dimstyle_panel", DIMENSIONS)),
        "shadows" => Some(("Shadows and sun", "shadows_panel", SHADOWS)),
        _ => None,
    }
}
fn form_named(name: &Obj<'_>) -> PyResult<Option<(&'static str, &'static str, &'static [Field])>> {
    Ok(match name.extract::<String>() {
        Ok(name) => form(&name),
        Err(_) => None,
    })
}

fn words(panel: &Obj<'_>, words: Words) -> PyResult<String> {
    let py = panel.py();
    let said = |name: &str| -> PyResult<String> { panel.getattr(name)?.call_method0("text")?.str()?.extract() };
    Ok(match words {
        Words::Plain(text) => text.to_owned(),
        Words::Label(name) => said(name)?,
        Words::Value(name, suffix) => format!("{}{suffix}", panel.getattr(name)?.call_method0("value")?.str()?),
        Words::Short(name, suffix) => {
            let value = panel.getattr(name)?.call_method0("value")?;
            format!("{}{suffix}", py.import("builtins")?.call_method1("format", (value, "g"))?.str()?)
        }
        Words::Date => panel.getattr("_date")?.call_method0("date")?.call_method1("toString", ("d MMMM",))?.str()?.extract()?,
        Words::SunTimes => {
            let sunrise = said("_sunrise_lbl")?;
            if sunrise.is_empty() { String::new() } else { format!("Sunrise {sunrise} · sunset {}", said("_sunset_lbl")?) }
        }
    })
}

pub(super) fn open_form<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, name) = (a.get_item(0)?, a.get_item(1)?);
    let module = ui(py)?;
    let win = module.call_method1("_window", (&view,))?;
    view.setattr("_ui_form", &name)?;
    view.setattr("_ui_form_sig", py.None())?;
    let panel = if win.is_none() { none(py) } else { direct(py, form_panel, (win, name))? };
    if !panel.is_none() && panel.hasattr("refresh")? {
        panel.call_method0("refresh")?;
    }
    view.call_method0("update")?;
    Ok(none(py))
}

pub(super) fn form_panel<'py>(a: &Args<'py>) -> Reply<'py> {
    match form_named(&a.get_item(1)?)? {
        Some((_, panel, _)) => attr(&a.get_item(0)?, panel),
        None => Ok(none(a.py())),
    }
}

/// Send the open form's fields (only when they changed).
pub(super) fn form_fields<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, win) = (a.get_item(0)?, a.get_item(1)?);
    let Some(name) = own(&view, "_ui_form")?.filter(|name| name.is_truthy().unwrap_or(false)) else {
        return Ok(none(py));
    };
    let panel = direct(py, form_panel, (&win, &name))?;
    let renderer = view.getattr("renderer")?;
    let Some((title, _, spec)) = form_named(&name)?.filter(|_| !panel.is_none()) else {
        view.setattr("_ui_form", py.None())?;
        renderer.call_method1("ui_set_form", ("", empty(py)))?;
        return Ok(none(py));
    };
    let fields = empty(py);
    let number = |value: Obj<'py>| -> PyResult<f64> { py.get_type::<pyo3::types::PyFloat>().call1((value,))?.extract() };
    for (at, entry) in spec.iter().enumerate() {
        let id = at.to_string();
        let label = words(&panel, entry.label)?;
        let none_yet = empty(py);
        match entry.kind {
            "caption" => {
                fields.append((id, entry.kind, label, "", 0.0, 0.0, 0.0, none_yet))?;
                continue;
            }
            "text" => {
                if !label.is_empty() {
                    fields.append((id, entry.kind, "", label, 0.0, 0.0, 0.0, none_yet))?;
                }
                continue;
            }
            "button" => {
                let shown = match entry.gate {
                    Some(gate) => panel.getattr(gate)?.call_method0("isEnabled")?.is_truthy()?,
                    None => true,
                };
                if shown {
                    fields.append((id, entry.kind, label, "", 0.0, 0.0, 0.0, none_yet))?;
                }
                continue;
            }
            _ => {}
        }
        let control = match entry.control {
            Some(control) => attr(&panel, control)?,
            None => none(py),
        };
        if control.is_none() || control.call_method0("isHidden")?.is_truthy()? || !control.call_method0("isEnabled")?.is_truthy()? {
            continue;
        }
        match entry.kind {
            "check" => {
                let on = control.call_method0("isChecked")?.is_truthy()?;
                fields.append((id, entry.kind, label, "", if on { 1.0 } else { 0.0 }, 0.0, 1.0, none_yet))?;
            }
            "slider" => {
                let shown = match entry.shown {
                    Some(shown) => words(&panel, shown)?,
                    None => control.call_method0("value")?.str()?.extract()?,
                };
                let scaled = |name: &str| -> PyResult<f64> { Ok(number(control.call_method0(name)?)? * entry.scale) };
                fields.append((id, entry.kind, label, shown, scaled("value")?, scaled("minimum")?, scaled("maximum")?, none_yet))?;
            }
            "choice" => {
                let count: usize = control.call_method0("count")?.extract()?;
                for item in 0..count {
                    none_yet.append(control.call_method1("itemText", (item,))?)?;
                }
                let current = number(control.call_method0("currentIndex")?)?;
                fields.append((id, entry.kind, label, control.call_method0("currentText")?, current, 0.0, count as f64, none_yet))?;
            }
            "color" => {
                let read = match entry.colour {
                    Some(Colour::Style(name)) => panel.call_method1("_shown_color", (name,)),
                    Some(Colour::Dimension) => (|| {
                        let levels = empty(py);
                        for channel in panel.call_method0("_style")?.call_method1("get", ("color", [45, 55, 75]))?.try_iter()? {
                            levels.append(channel?.div(255)?)?;
                        }
                        Ok(levels.into_any())
                    })(),
                    None => Ok(none(py)),
                };
                let shown = match read {
                    Ok(colour) if colour.is_truthy()? => direct(py, super::chrome_tree::hex, (colour,))?,
                    Ok(_) => "".into_bound_py_any(py)?,
                    Err(error) if error.is_instance_of::<PyException>(py) => "".into_bound_py_any(py)?,
                    Err(error) => return Err(error),
                };
                fields.append((id, entry.kind, label, shown, 0.0, 0.0, 0.0, none_yet))?;
            }
            _ => {}
        }
    }
    let sig = (&name, fields.to_tuple()).into_bound_py_any(py)?;
    if !own(&view, "_ui_form_sig")?.is_some_and(|sent| sent.eq(&sig).unwrap_or(false)) {
        view.setattr("_ui_form_sig", sig)?;
        renderer.call_method1("ui_set_form", (title, fields))?;
    }
    Ok(none(py))
}

pub(super) fn form_apply<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (win, view) = (a.get_item(0)?, a.get_item(1)?);
    let arg: String = a.get_item(2)?.extract()?;
    if arg == "close" {
        view.setattr("_ui_form", py.None())?;
        view.setattr("_ui_form_sig", py.None())?;
        return Ok(none(py));
    }
    let name = match own(&view, "_ui_form")? {
        Some(name) => name,
        None => none(py),
    };
    let panel = direct(py, form_panel, (&win, &name))?;
    if panel.is_none() {
        return Ok(none(py));
    }
    let Some((_, _, spec)) = form_named(&name)? else {
        return Err(PyKeyError::new_err(name.unbind()));
    };
    let (id, value) = partition(&arg);
    let int = py.get_type::<pyo3::types::PyInt>();
    let index: isize = int.call1((id,))?.extract()?;
    let at = if index < 0 { index + spec.len() as isize } else { index };
    let Some(entry) = usize::try_from(at).ok().and_then(|at| spec.get(at)) else {
        return Err(pyo3::exceptions::PyIndexError::new_err("list index out of range"));
    };
    match (entry.kind, entry.control) {
        ("button", None) => {
            render(py)?.getattr("font_picker")?.call_method1("annotation", (&win,))?;
        }
        ("button", Some(handler)) => {
            panel.call_method0(handler)?;
        }
        (kind, control) => {
            let control = panel.getattr(control.unwrap_or(""))?;
            match kind {
                "check" => {
                    control.call_method1("setChecked", (value == "1",))?;
                }
                "slider" => {
                    let moved: f64 = py.get_type::<pyo3::types::PyFloat>().call1((value,))?.extract::<f64>()? / entry.scale;
                    if entry.scale != 1.0 {
                        control.call_method1("setValue", (moved,))?;
                    } else {
                        control.call_method1("setValue", (int.call1((moved,))?,))?;
                    }
                }
                "choice" => {
                    let chosen = int.call1((value,))?;
                    control.call_method1("setCurrentIndex", (&chosen,))?;
                    control.getattr("activated")?.call_method1("emit", (chosen,))?;
                }
                "color" => {
                    control.call_method0("click")?;
                }
                _ => {}
            }
        }
    }
    if panel.hasattr("refresh")? && entry.kind != "slider" {
        panel.call_method0("refresh")?;
    }
    view.setattr("_ui_form_sig", py.None())?;
    view.setattr("_ui_cluster_sig", py.None())?;
    view.call_method0("update")?;
    Ok(none(py))
}

fn all_commands<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let rows = empty(py);
    for command in render(py)?.getattr("command_metadata")?.call_method1("commands", (a.get_item(0)?,))?.try_iter()? {
        let command = command?;
        rows.append((command.getattr("name")?, command.getattr("path")?, command.getattr("keys")?, command.getattr("action")?))?;
    }
    Ok(rows.into_any())
}

fn command_action<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, win, arg) = (a.get_item(0)?, a.get_item(1)?, a.get_item(3)?);
    let verb: String = a.get_item(2)?.extract()?;
    match verb.as_str() {
        "cmdscope" => {
            view.setattr("_ui_command_scope", arg)?;
            view.setattr("_ui_command_owner", py.None())?;
        }
        "cmdq" => {
            direct(py, command_query, (&view, &win, arg))?;
        }
        "cmdrun" | "cmdshortcut" => {
            let hits = view.getattr_opt("_ui_cmd_hits")?.unwrap_or_else(|| PyTuple::empty(py).into_any());
            let index: i64 = py.get_type::<pyo3::types::PyInt>().call1((arg,))?.extract()?;
            if 0 <= index && (index as usize) < hits.len()? {
                let action = hits.get_item(index)?;
                if attr(&view, "_ui_command_scope")?.eq("Sweep presets")? {
                    if verb == "cmdrun" {
                        let tool=win.getattr("_tools")?.get_item("sweep")?;
                        let machine=tool.getattr("_machine")?;
                        if let Ok(preset)=action.get_item("preset"){
                            machine.call_method1("set",("preset",preset))?;
                            py.import("built_render.wgpu_library")?.call_method1("_remember",(&view,action.get_item("key")?))?;
                        }else if let Ok(sketch)=action.get_item("sketch"){
                            machine.call_method1("set",("sketch",sketch))?;
                        }else{
                            machine.call_method1("set",("profile",action.get_item("key")?))?;
                        }
                        view.call_method0("update")?;
                    }
                } else if verb == "cmdrun" {
                    render(py)?.getattr("command_metadata")?.call_method1("run", (action,))?;
                } else if action.call_method0("isVisible")?.is_truthy()? {
                    let owner = if win.hasattr("open_preferences")? { win } else { win.call_method0("parentWidget")? };
                    if !owner.is_none() {
                        owner.call_method("open_preferences", (), Some(&fields!(py; "shortcut_of" => action)))?;
                    }
                }
            }
        }
        _ => {}
    }
    Ok(none(py))
}

pub(super) fn command_query<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, win, query) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?);
    if attr(&view,"_ui_command_scope")?.eq("Sweep presets")? {
        let provider=py.import("built_render.wgpu_library")?.getattr("ProfilesProvider")?.call0()?;
        let ctx=py.import("types")?.getattr("SimpleNamespace")?.call((),Some(&fields!(py;"scene"=>win.getattr("viewport")?.getattr("scene")?)))?;
        let needle=query.extract::<String>()?.to_lowercase();
        let (actions, results)=(empty(py), empty(py));
        let presets=py.import("built_render.wgpu_library")?.getattr("PresetsProvider")?.call0()?;
        let recent:Vec<String>=py.import("built_render.wgpu_library")?.call_method1("_state",(&view,))?.get_item("recent")?.extract()?;
        let mut matches=Vec::new();
        for entry in presets.call_method1("items",(&ctx,))?.try_iter()?{
            let entry=entry?;
            let p=entry.get_item("preset")?;
            if !p.get_item("primitive")?.eq("sweep")?{continue;}
            let name=entry.get_item("name")?.str()?.to_string();
            let pack=entry.get_item("category")?.str()?.to_string();
            let record=p.get_item("record")?;
            let profile=p.get_item("values")?.get_item("profile")?.call_method1("get",("library","Drawn profile"))?.str()?.to_string();
            let class=record.call_method1("get",("class",""))?.str()?.to_string();
            let subtype=record.call_method1("get",("predefined_type",""))?.str()?.to_string();
            let detail=format!("{} · {} · {} {}",profile,record.call_method1("get",("material",""))?.str()?,class,subtype);
            if format!("{name} {pack} {detail}").to_lowercase().contains(&needle){
                let id:String=p.get_item("uid")?.extract()?;
                let source:String=entry.get_item("source")?.extract()?;
                let at=recent.iter().position(|r|r==&id);
                let group=if at.is_some(){"Recent".to_string()}else{match source.as_str(){"model"=>"This project".into(),"mine"=>"My presets".into(),_=>pack}};
                matches.push((at.unwrap_or(usize::MAX),group,name,detail,entry));
            }
        }
        matches.sort_by(|a,b|(&a.0,&a.1,&a.2).cmp(&(&b.0,&b.1,&b.2)));
        for (_,group,name,detail,entry) in matches{
            results.append((name,group,detail,true))?;
            actions.append(entry)?;
        }
        for profile in provider.call_method1("items",(ctx,))?.try_iter()?{
            let profile=profile?;
            let name:String=profile.get_item("name")?.extract()?;
            let family:String=profile.get_item("category")?.extract()?;
            let class=if profile.get_item("key")?.eq("basic/wall")? {"IfcWall"} else {"IfcBeam"};
            if format!("{name} {family} {class}").to_lowercase().contains(&needle){
                results.append((name,family,class,true))?;
                actions.append(profile)?;
            }
        }
        view.setattr("_ui_cmd_hits",actions)?;
        view.getattr("renderer")?.call_method1("ui_set_command_results",(results,))?;
        return Ok(none(py));
    }
    let metadata = render(py)?.getattr("command_metadata")?;
    let workspace = py.import("built_render.sheet_workspace")?.call_method1("workspace_of", (&view,))?;
    let owner = if workspace.is_none() { win } else { render(py)?.call_method1("sheet_command_scope", (&win,))? };
    if !query.is_truthy()? || !attr(&view, "_ui_command_owner")?.is(&owner) {
        let scope = view.getattr_opt("_ui_command_scope")?.unwrap_or("".into_bound_py_any(py)?);
        view.setattr("_ui_command_pool", metadata.call_method1("commands", (&owner, scope))?)?;
        view.setattr("_ui_command_owner", &owner)?;
    }
    let live = empty(py);
    for command in view.getattr("_ui_command_pool")?.try_iter()? {
        let command = command?;
        if command.getattr("action")?.call_method0("isVisible")?.is_truthy()? {
            live.append(command)?;
        }
    }
    let hits = metadata.call_method("search", (&owner, &query), Some(&fields!(py; "pool" => live)))?;
    let (actions, results, details) = (empty(py), empty(py), empty(py));
    for command in hits.try_iter()? {
        let command = command?;
        let action = command.getattr("action")?;
        results.append((command.getattr("name")?, command.getattr("path")?, command.getattr("keys")?, action.call_method0("isEnabled")?))?;
        actions.append(action)?;
    }
    view.setattr("_ui_cmd_hits", actions)?;
    let renderer = view.getattr("renderer")?;
    renderer.call_method1("ui_set_command_results", (results,))?;
    let detail = attr(&renderer, "ui_set_command_details")?;
    if detail.is_truthy()? {
        for command in hits.try_iter()? {
            let command = command?;
            let action = command.getattr("action")?;
            let checkable = action.hasattr("isChecked")?;
            let checked = checkable && action.call_method0("isChecked")?.is_truthy()?;
            details.append((command.getattr("tip")?, checked, checkable))?;
        }
        detail.call1((details,))?;
    }
    Ok(none(py))
}

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Method] = &[
    ("_run", Mode::Function, run, &["win", "actions", "source_view=None"], "", ""),
    ("_all_commands", Mode::Function, all_commands, &["win"], "", ""),
    ("command_action", Mode::Function, command_action, &["view", "win", "verb", "arg"], "", ""),
];
