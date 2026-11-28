// SPDX-License-Identifier: GPL-3.0-or-later
//! The viewport chrome's per-frame mirror (`views.wgpu_ui`): the window's
//! state sent to the egui chrome the renderer draws in the model's frame —
//! tools, tabs, the app sheet and jobs, the value field, the selection bar
//! and the active tool's options (render-glue lane; from
//! `views/wgpu_ui.py`).
//!
//! The module's functions call each other by name on `views.wgpu_ui`: a
//! host or a test replaces one of them there.
use super::*;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::types::PyCFunction;

pub(super) fn ui<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyModule>> {
    py.import("views.wgpu_ui")
}
pub(super) fn render<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyModule>> {
    py.import("built_render")
}
pub(super) fn attr<'py>(o: &Obj<'py>, name: &str) -> Reply<'py> {
    optional(o, name)
}
pub(super) fn truthy(o: &Obj<'_>, name: &str) -> PyResult<bool> {
    attr_flag(o, name, false)
}
pub(super) fn id(o: &Obj<'_>) -> usize {
    o.as_ptr() as usize
}
/// `o.__dict__.get(name, default)`.
pub(super) fn own<'py>(o: &Obj<'py>, name: &str) -> PyResult<Option<Obj<'py>>> {
    o.getattr("__dict__")?.call_method1("get", (name,)).map(|value| (!value.is_none()).then_some(value))
}
/// `float(value)`, `None` where Python raises one of `caught`.
fn number(value: &Obj<'_>, type_error: bool) -> PyResult<Option<f64>> {
    let py = value.py();
    match py.get_type::<pyo3::types::PyFloat>().call1((value,)) {
        Ok(number) => Ok(Some(number.extract()?)),
        Err(error) if error.is_instance_of::<PyValueError>(py) || (type_error && error.is_instance_of::<PyTypeError>(py)) => Ok(None),
        Err(error) => Err(error),
    }
}
fn settings<'py>(given: Obj<'py>) -> Reply<'py> {
    if given.is_none() { given.py().import("built_core.settings")?.call_method0("settings") } else { Ok(given) }
}
/// A callable of no arguments for a strip toggle: it lives as long as the
/// frame's option list.
fn action<'py, F>(py: Python<'py>, run: F) -> Reply<'py>
where
    F: Fn(Python<'_>) -> PyResult<()> + Send + Sync + 'static,
{
    Ok(PyCFunction::new_closure(py, None, None, move |args: &Bound<'_, PyTuple>, _: Option<&Bound<'_, PyDict>>| run(args.py()))?.into_any())
}

fn window<'py>(a: &Args<'py>) -> Reply<'py> {
    let win = a.get_item(0)?.call_method0("window")?;
    Ok(if win.hasattr("_tool_actions")? { win } else { none(a.py()) })
}

/// Called once the view has its renderer.
fn attach<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let renderer = view.getattr("renderer")?;
    if truthy(&view, "_ui_attached")? {
        return Ok(none(py));
    }
    let win = ui(py)?.call_method1("_window", (&view,))?;
    if win.is_none() {
        return Ok(none(py));
    }
    view.setattr("_ui_timer", view.call_method1("_view_timer", (view.getattr("update")?,))?)?;
    let font = ui(py)?.getattr("FONT")?;
    let found = py.import("os")?.getattr("path")?.call_method1("exists", (&font,))?.is_truthy()?;
    renderer.call_method1("ui_enable", (if found { font } else { none(py) },))?;
    render(py)?.getattr("wgpu_library")?.call_method1("install", (&win,))?;
    if renderer.hasattr("ui_set_dock_width")? {
        let stored = settings(none(py))?.call_method1("value", ("ui/dock_width", 300.0))?;
        if let Some(width) = number(&stored, true)? {
            renderer.call_method1("ui_set_dock_width", (width,))?;
        }
    }
    // The columns the user last chose come back with the dock.
    if own(&view, "_ui_lens")?.is_none() {
        let (lens, mask) = stored_columns(py)?;
        view.setattr("_ui_lens", lens)?;
        view.setattr("_ui_columns", mask)?;
    }
    view.setattr("_ui_tools_sent", false)?;
    view.setattr("_ui_attached", true)?;
    let source = view.getattr("source_viewport")?;
    if !source.is_none() {
        source.setattr("_chrome_owns_labels", true)?;
    }
    Ok(none(py))
}

fn lens_of<'py>(view: &Obj<'py>) -> Reply<'py> {
    Ok(match own(view, "_ui_lens")? {
        Some(lens) => lens,
        None => 0.into_bound_py_any(view.py())?,
    })
}
/// The dock's width: the one the user set by dragging its inner edge. The
/// tree's column presets (`lens`) never change it.
pub(super) fn dock_width<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let store = settings(a.get_item(1)?)?;
    let own = number(&store.call_method1("value", ("ui/dock_width", 300.0))?, true)?.unwrap_or(300.0);
    own.into_bound_py_any(py)
}

fn apply_dock_width<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let renderer = attr(&view, "renderer")?;
    if renderer.is_none() || !renderer.hasattr("ui_set_dock_width")? {
        return Ok(none(py));
    }
    renderer.call_method1("ui_set_dock_width", (ui(py)?.call_method1("dock_width", (lens_of(&view)?, a.get_item(1)?))?,))?;
    Ok(none(py))
}

/// The grip let go (or double-clicked): keep the width, per user.
fn keep_dock_width<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let arg = a.get_item(1)?;
    let store = settings(a.get_item(2)?)?;
    let given = if arg.is_truthy()? { arg } else { 300.into_bound_py_any(py)? };
    let Some(width) = number(&given, false)? else {
        return Ok(none(py));
    };
    store.call_method1("setValue", ("ui/dock_width", width))?;
    Ok(none(py))
}

/// The tree's columns as last chosen ("preset" or "preset:mask"), per user.
fn stored_columns<'py>(py: Python<'py>) -> PyResult<(i64, Option<i64>)> {
    let stored = settings(none(py))?.call_method1("value", ("ui/dock_columns", ""))?;
    let text = if stored.is_none() { String::new() } else { stored.str()?.extract::<String>()? };
    let (lens, mask) = text.split_once(':').map_or((text.as_str(), None), |(lens, mask)| (lens, Some(mask)));
    let lens = lens.trim().parse::<i64>().ok().filter(|lens| (0..=2).contains(lens)).unwrap_or(0);
    Ok((lens, mask.and_then(|mask| mask.trim().parse::<i64>().ok()).filter(|mask| (0..=0xffff).contains(mask))))
}

/// Keep the columns the dock reported: the preset and, when the user
/// changed its columns, their mask.
pub(super) fn keep_columns(view: &Obj<'_>) -> PyResult<()> {
    let py = view.py();
    // Only a dock on screen speaks for the user.
    if !truthy(view, "_ui_attached")? {
        return Ok(());
    }
    let lens: i64 = lens_of(view)?.extract().unwrap_or(0);
    let mask = own(view, "_ui_columns")?.filter(|mask| !mask.is_none());
    let text = match mask {
        Some(mask) => format!("{lens}:{}", mask.str()?.extract::<String>()?),
        None => lens.to_string(),
    };
    settings(none(py))?.call_method1("setValue", ("ui/dock_columns", text))?;
    Ok(())
}

/// Once the window has built its tool actions (after the first view).
pub(super) fn send_tools<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, win) = (a.get_item(0)?, a.get_item(1)?);
    let module = ui(py)?;
    let icon = |key: &Obj<'py>| -> PyResult<[Obj<'py>; 3]> { unpack::<3>(&module.call_method1("_icon_rgba", (key,))?) };
    let tools = win.getattr_opt("_tools")?.unwrap_or_else(|| PyDict::new(py).into_any());
    let (groups, disabled, runs) = (PyList::empty(py), PyList::empty(py), PyDict::new(py));
    view.setattr("_ui_palette_runs", &runs)?;
    for keys in render(py)?.call_method0("rail_layout")?.try_iter()? {
        let row = PyList::empty(py);
        for entry in keys?.try_iter()? {
            let [key, binding] = unpack::<2>(&entry?)?;
            let action = if binding.is_truthy()? {
                let action = win.getattr_opt(binding.cast::<PyString>()?)?.unwrap_or_else(|| none(py));
                if !action.is_none() {
                    runs.set_item(&key, action.getattr("trigger")?)?;
                }
                action
            } else if tools.contains(&key)? {
                win.getattr("_tool_actions")?.call_method1("get", (&key,))?
            } else {
                none(py)
            };
            if action.is_none() {
                continue;
            }
            if !action.call_method0("isEnabled")?.is_truthy()? {
                disabled.append(&key)?;
            }
            let primary = match module.call_method1("action_keys", (&action,))?.try_iter()?.next() {
                Some(primary) => primary?,
                None => "".into_bound_py_any(py)?,
            };
            let shortcut = if primary.is_truthy()? {
                module.call_method1("shortcut_text", (&primary,))?.get_item(1)?
            } else {
                "".into_bound_py_any(py)?
            };
            let [w, h, rgba] = icon(&key)?;
            row.append((&key, action.call_method0("text")?.call_method1("replace", ("&", ""))?, shortcut, w, h, rgba))?;
        }
        if !row.is_empty() {
            groups.append(row)?;
        }
    }
    let renderer = view.getattr("renderer")?;
    if disabled.is_empty() {
        renderer.call_method1("ui_set_tools", (groups,))?;
    } else {
        renderer.call_method1("ui_set_tools", (groups, disabled))?;
    }
    let icons = PyList::empty(py);
    let add = |key: Obj<'py>| -> PyResult<()> {
        let [w, h, rgba] = icon(&key)?;
        icons.append((key, w, h, rgba))
    };
    for entry in py.import("views.view_commands")?.call_method0("entries")?.try_iter()? {
        let entry = entry?;
        let (group, key) = (entry.get_item(0)?, entry.get_item(4)?);
        let mut wanted = false;
        for name in ["nav", "walkthrough", "Views", "Projection"] {
            wanted |= group.eq(name)?;
        }
        if wanted && key.is_truthy()? {
            add(key)?;
        }
    }
    for key in ["four_views", "xray", "back_edges", "hidden_geometry", "section_cut", "shadows"] {
        add(key.into_bound_py_any(py)?)?;
    }
    renderer.call_method1("ui_set_icons", (icons,))?;
    view.setattr("_ui_tools_sent", true)?;
    Ok(none(py))
}

fn project_action<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (win, arg) = (a.get_item(0)?, a.get_item(1)?);
    if arg.eq("views")? {
        win.call_method0("_pick_view_layout")?;
    } else if arg.eq("new")? {
        win.call_method0("_on_new_tab")?;
    } else {
        let close = arg.call_method1("startswith", ("close:",))?.is_truthy()?;
        let index = py.get_type::<pyo3::types::PyInt>().call1((arg.call_method1("removeprefix", ("close:",))?,))?;
        let docs = win.call_method0("documents")?;
        let at: i64 = index.extract()?;
        if 0 <= at && (at as usize) < docs.len()? {
            let document = docs.get_item(at)?;
            if close {
                win.call_method1("run_command", ("close_tab", document))?;
            } else {
                win.call_method1("activate_document", (document,))?;
            }
        }
    }
    Ok(none(py))
}

fn listed<'py>(py: Python<'py>, value: Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    if value.is_truthy()? {
        Ok(py.get_type::<PyList>().call1((value,))?.cast_into()?)
    } else {
        Ok(PyList::empty(py))
    }
}

/// Model / sheet and the scenes for the bottom capsule bar.
pub(super) fn tabs<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, win) = (a.get_item(0)?, a.get_item(1)?);
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let comps = listed(py, attr(&scene, "compositions")?)?;
    let mut sheet = if comps.is_empty() {
        "New sheet".into_bound_py_any(py)?
    } else {
        let last: i64 = match win.getattr_opt("_last_sheet")? {
            Some(last) => last.extract()?,
            None => 0,
        };
        comps.get_item(last.max(0).min(comps.len() as i64 - 1) as usize)?.getattr("name")?
    };
    let views = listed(py, attr(&scene, "saved_views")?)?;
    let row = attr(&attr(&win, "_scene_tabs")?, "tabs")?;
    let marked = if !row.is_none() && row.hasattr("marked_view")? { row.call_method0("marked_view")? } else { none(py) };
    let sheets = py.import("built_render.sheet_workspace")?;
    let on_sheets = sheets.call_method1("shown", (&view,))?.is_truthy()?;
    if sheets.call_method1("embeds", (&win,))?.is_truthy()? {
        // Sheets is a workspace of this window (sheet_workspace).
        sheet = "Sheets".into_bound_py_any(py)?;
    }
    let work = PyList::new(
        py,
        [
            ("model", "Model".into_bound_py_any(py)?, "The model", "", if on_sheets { 0 } else { 1 }, true),
            ("sheet", sheet, "Sheets (⌃Tab)", "", if on_sheets { 1 } else { 0 }, true),
        ],
    )?;
    let core = py.import("built_core")?;
    let version = scene.getattr_opt("content_version")?.unwrap_or(0.into_bound_py_any(py)?);
    let cached = attr(&view, "_ui_visibility_cache")?;
    if cached.is_none() || !cached.get_item(0)?.is(&scene) || cached.get_item(1)?.ne(&version)? {
        view.setattr("_ui_visibility_cache", (&scene, &version, core.call_method1("visibility_set_rows", (&scene,))?))?;
    }
    let catalog_key = (id(&scene), scene.getattr_opt("version")?.unwrap_or(0.into_bound_py_any(py)?), id(&marked)).into_bound_py_any(py)?;
    if !on_sheets && catalog_key.ne(attr(&view, "_ui_scene_catalog_key")?)? {
        view.setattr("_ui_scene_menu", core.getattr("scene_tree")?.call_method1("menu_rows", (&scene, &marked))?)?;
        view.setattr("_ui_scene_catalog_key", catalog_key)?;
    }
    let (scenes, folders, shown) = if on_sheets {
        (PyList::empty(py).into_any(), PyList::empty(py).into_any(), PyList::empty(py).into_any())
    } else {
        let [scenes, folders] = unpack::<2>(&view.getattr("_ui_scene_menu")?)?;
        (scenes, folders, view.getattr("_ui_visibility_cache")?.get_item(2)?)
    };
    let tabs = PyList::new(py, [work.into_any(), scenes, shown, folders])?;
    if tabs.as_any().ne(attr(&view, "_ui_tabs")?)? {
        view.setattr("_ui_tabs", &tabs)?;
        view.getattr("renderer")?.call_method1("ui_set_tabs", (&tabs,))?;
    }
    view.setattr("_ui_scene_views", views)?;
    Ok(none(py))
}

/// The app sheet and the work in progress
/// (rust/built_render/src/dialogs_host.rs).
pub(super) fn sheet_and_jobs<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, win) = (a.get_item(0)?, a.get_item(1)?);
    let dialogs = render(py)?.getattr("chrome_dialogs")?;
    let renderer = view.getattr("renderer")?;
    let state = view.getattr("__dict__")?;
    let sheet = dialogs.call_method1("current_sheet", (&win,))?;
    let buttons = || dialogs.call_method1("sheet_buttons", (&sheet,));
    let sig = if sheet.is_none() {
        none(py)
    } else {
        (id(&sheet), sheet.getattr("title")?, sheet.getattr("text")?, py.get_type::<PyTuple>().call1((buttons()?,))?).into_bound_py_any(py)?
    };
    if sig.ne(state.call_method1("get", ("_ui_sheet_sig", false))?)? {
        view.setattr("_ui_sheet_sig", &sig)?;
        if sheet.is_none() {
            renderer.call_method1("ui_set_sheet", ("", "", "", PyList::empty(py)))?;
        } else {
            renderer.call_method1("ui_set_sheet", (sheet.getattr("title")?, sheet.getattr("text")?, sheet.getattr("tone")?, buttons()?))?;
        }
    }
    if !sheet.is_none() {
        sheet.setattr("shown", true)?;
    }
    let rows = PyList::empty(py);
    for job in dialogs.call_method1("jobs", (&win,))?.try_iter()? {
        let job = job?;
        let fraction: f64 = py.get_type::<pyo3::types::PyFloat>().call1((job.getattr("fraction")?,))?.extract()?;
        rows.append((job.getattr("id")?, job.getattr("label")?, job.getattr("phase")?, fraction, job.getattr("cancellable")?.is_truthy()?))?;
    }
    if rows.as_any().ne(state.call_method1("get", ("_ui_jobs_sig", false))?)? {
        view.setattr("_ui_jobs_sig", &rows)?;
        renderer.call_method1("ui_set_jobs", (&rows,))?;
    }
    // Polled work (the background auto-save) says when it is done only by
    // being gone: look again soon while any shows.
    let now: f64 = py.import("time")?.call_method0("monotonic")?.extract()?;
    let recent = PyList::empty(py);
    let secondary = PyDict::new(py);
    let mut transient = false;
    for row in dialogs.call_method1("history", (&win,))?.try_iter()? {
        let row = row?;
        let opacity: f64 = row.call_method1("opacity", (now,))?.extract()?;
        transient |= opacity > 0. && !row.getattr("tone")?.eq("error")?;
        let second = row.getattr("second_label")?;
        if second.is_truthy()? { secondary.set_item(row.getattr("id")?, second)?; }
        recent.append((
            row.getattr("id")?,
            row.getattr("title")?,
            row.getattr("detail")?,
            row.getattr("tone")?,
            row.getattr("action_label")?,
            opacity,
        ))?;
    }
    let banner = win.getattr("_doc")?.getattr_opt("_missing_link_banner")?.unwrap_or("".into_bound_py_any(py)?);
    let activity = (&recent, &banner, &secondary).into_bound_py_any(py)?;
    if activity.ne(state.call_method1("get", ("_ui_activity_sig",))?)? && renderer.hasattr("ui_set_activity")? {
        if secondary.is_empty() {
            renderer.call_method1("ui_set_activity", (&recent, &banner))?;
        } else {
            renderer.call_method1("ui_set_activity", (&recent, &banner, &secondary))?;
        }
        view.setattr("_ui_activity_sig", activity)?;
    }
    view.setattr("_ui_poll_jobs", !rows.is_empty() || transient)?;
    Ok(none(py))
}

/// The form on top, if any (rust/built_render/src/forms_host.rs).
pub(super) fn chrome_form<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    if view.getattr("renderer")?.hasattr("ui_set_dialog")? {
        render(py)?.getattr("chrome_forms")?.call_method1("before_frame", (&view, a.get_item(1)?))?;
    }
    Ok(none(py))
}

/// Exclusive work holds the window (an open, an import): the model is half
/// there, so a frame draws only the chrome over what is resident.
pub(super) fn chrome_only<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    if !truthy(&view, "_ui_attached")? {
        return false.into_bound_py_any(py);
    }
    let win = ui(py)?.call_method1("_window", (&view,))?;
    if win.is_none() {
        return false.into_bound_py_any(py);
    }
    render(py)?.getattr("chrome_dialogs")?.call_method1("exclusive", (win,))
}

/// The sheet, the jobs, the form, the start page and the chrome sheet: what
/// both a chrome-only frame and a full one mirror.
fn modal_layers<'py>(view: &Obj<'py>, win: &Obj<'py>) -> PyResult<()> {
    let py = view.py();
    let module = ui(py)?;
    module.call_method1("_sheet_and_jobs", (view, win))?;
    direct(py, chrome_form, (view, win))?;
    render(py)?.getattr("wgpu_start")?.call_method1("before_frame", (view, win))?;
    render(py)?.getattr("chrome_sheet")?.call_method1("before_frame", (view, win))?;
    Ok(())
}
fn document_tabs<'py>(view: &Obj<'py>, win: &Obj<'py>) -> PyResult<Bound<'py, PyModule>> {
    let py = view.py();
    let tabs = py.import("views.wgpu_tabs")?;
    tabs.call_method1("document_tabs", (view,))?.call_method1("sync", (view.getattr("renderer")?, win, ui(py)?.getattr("LIGHTS_ROOM")?))?;
    Ok(tabs)
}

/// A frame of chrome alone: the projects, the sheet and the jobs, over the
/// resident (blank) model. Nothing here reads the scene.
fn chrome_frame<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let renderer = view.getattr("renderer")?;
    if renderer.is_none() {
        return Ok(none(py));
    }
    let win = ui(py)?.call_method1("_window", (&view,))?;
    if !win.is_none() {
        document_tabs(&view, &win)?;
        modal_layers(&view, &win)?;
    }
    let [cpu, gpu] = unpack::<2>(&renderer.call_method0("render")?)?;
    ui(py)?.call_method1("after_frame", (&view,))?;
    (cpu, gpu, 0.0).into_bound_py_any(py)
}

fn sync_units<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, history) = (a.get_item(0)?, a.get_item(1)?);
    let revision = (&history, history.getattr("_native")?.getattr("units_revision")?).into_bound_py_any(py)?;
    if attr(&view, "_ui_units_sent")?.ne(&revision)? {
        let units = py.import("built_core.units")?;
        let unit = units.call_method1("model_units_of", (history.getattr("scene")?,))?.get_item("length")?;
        let suffix = units.call_method1("length_suffix", (&unit,))?.call_method0("strip")?;
        view.getattr("renderer")?.call_method1("ui_set_units", (if suffix.is_truthy()? { suffix } else { unit },))?;
        view.setattr("_ui_units_sent", revision)?;
    }
    Ok(none(py))
}

/// Only the active gesture supplies the tool label and hint.
fn sync_tool_status(view: &Obj, win: &Obj) -> PyResult<()> {
    let py = view.py();
    crate::native_view_actions::tool_hint(win)?;
    let bar = win.call_method0("statusBar")?;
    let base = bar.getattr("_tool_hint")?;
    let [name, _, hint] = unpack::<3>(&base.call_method1("partition", (" — ",))?)?;
    render(py)?.call_method1("sync_status_readout", (view, name, hint))?;
    Ok(())
}

/// Mirror the window's state into the chrome (cheap: a few dict reads).
/// Selections at least this large reach the panels after the view, one
/// panel a frame.
const DEFERRED_PANELS: usize = 50_000;

/// How far the panels have caught up with a huge selection change: 0 on
/// the frame of the change (the viewport only), then 1 (the selection
/// bar), 2 (and the tree's rows), 3 (and the inspector: everything, as for
/// any other selection). Each step asks for another frame, so the highlight
/// shows first and no frame reads 600k entries for three panels at once.
fn panels_stage(view: &Obj<'_>) -> PyResult<u8> {
    let py = view.py();
    let selection = attr(&attr(view, "scene")?, "selection")?;
    if selection.is_none() || selection.len()? < DEFERRED_PANELS {
        return Ok(3);
    }
    let key = (id(&selection), direct(py, sel_key, (&selection,))?).into_bound_py_any(py)?;
    let old = attr(view, "_panels_selection_key")?;
    let stage = if !old.is_none() && old.get_item(0)?.eq(&key)? {
        old.get_item(1)?.extract::<u8>()?.saturating_add(1).min(3)
    } else {
        0
    };
    view.setattr("_panels_selection_key", (key, stage))?;
    if stage < 3 {
        view.call_method0("update")?;
    }
    Ok(stage)
}

fn before_frame<'py>(a: &Args<'py>) -> Reply<'py> {
    crate::ui::sync_language(a.py())?;
    let py = a.py();
    let view = a.get_item(0)?;
    if !truthy(&view, "_ui_attached")? {
        return Ok(none(py));
    }
    let module = ui(py)?;
    let win = module.call_method1("_window", (&view,))?;
    if win.is_none() {
        return Ok(none(py));
    }
    let renderer = view.getattr("renderer")?;
    crate::plugin_panels::sync(&view, &win)?;
    if let Some(menus) = own(&win, "_extension_menus")? {
        if menus.is_truthy()? {
            menus.call_method1("present", (&view,))?;
        }
    }
    modal_layers(&view, &win)?;
    let actions = win.getattr("_tool_actions")?;
    if !view.getattr("_ui_tools_sent")?.is_truthy()? && actions.is_truthy()? {
        module.call_method1("_send_tools", (&view, &win))?;
    }
    let eyedropper = attr(&win, "_act_eyedropper")?;
    let mut active = attr(&view.getattr("source_viewport")?, "nav_mode")?;
    if !active.is_truthy()? {
        active = "".into_bound_py_any(py)?;
        if !eyedropper.is_none() && eyedropper.call_method0("isChecked")?.is_truthy()? {
            active = "eyedropper".into_bound_py_any(py)?;
        } else {
            for entry in win.getattr("_tool_actions")?.call_method0("items")?.try_iter()? {
                let [key, action] = unpack::<2>(&entry?)?;
                if action.call_method0("isChecked")?.is_truthy()? {
                    active = key;
                    break;
                }
            }
        }
    }
    renderer.call_method1("ui_set_active", (&active,))?;
    sync_tool_status(&view, &win)?;
    module.call_method1("_sync_units", (&view, win.getattr("viewport")?.getattr("history")?))?;
    module.call_method1("_cursor_value", (&view, &win))?;
    // A huge selection change shows in the viewport first; the panels that
    // read the whole selection (the selection bar's verbs, the tree, the
    // inspector) follow on the next frame.
    let stage = panels_stage(&view)?;
    view.setattr("_panels_stage", stage)?;
    if stage >= 1 {
        module.call_method1("_selection", (&view, &win, &active))?;
    }
    direct(py, cluster, (&view, &win))?;
    direct(py, super::chrome_actions::form_fields, (&view, &win))?;
    render(py)?.call_method1("bind_deliveries", (&win,))?;
    crate::tasks_host::tasks_before_frame(&view, &win)?;
    render(py)?.getattr("wgpu_library")?.call_method1("before_frame", (&view, &win))?;
    if stage >= 2 {
        module.call_method1("_sync_project_rows", (&view, &win))?;
    }
    if stage >= 3 {
        render(py)?.call_method1("inspector_sync", (&view, &win))?;
    }
    module.call_method1("_tabs", (&view, &win))?;
    document_tabs(&view, &win)?.call_method1("sync_host_context", (&view, &win))?;
    let options = PyList::empty(py);
    for option in module.call_method1("_options", (&view,))?.try_iter()? {
        let [key, label, shortcut, state, _] = unpack::<5>(&option?)?;
        options.append((key, label, shortcut, state))?;
    }
    if options.as_any().ne(attr(&view, "_ui_opts")?)? {
        view.setattr("_ui_opts", &options)?;
        renderer.call_method1("ui_set_options", (&options,))?;
    }
    Ok(none(py))
}

/// The native name of the key the tools call Ctrl (⌘ on the Mac).
pub(super) fn ctrl<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    crate::shortcut_text::modifier_text("Ctrl").into_bound_py_any(py)
}

/// (id, label, key, state, toggle) for the active tool's strip. Each toggle
/// does exactly what tapping the key does; locks are read-only.
fn options<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?;
    let module = ui(py)?;
    let source = view.getattr("source_viewport")?;
    let tool = attr(&source, "active_tool")?;
    let host = view.getattr("tools")?; // what the tools are handed as their viewport here
    let name: String = if tool.is_none() { String::new() } else { tool.get_type().name()?.extract()? };
    let out = PyList::empty(py);
    let ctrl = || direct(py, ctrl, ());
    let flag = |name: &str| -> PyResult<i32> { Ok(tool.getattr(name)?.is_truthy()? as i32) };
    let (held_tool, held_host, held_view) = (tool.clone().unbind(), host.clone().unbind(), view.clone().unbind());
    match name.as_str() {
        "CutTool" => {
            let on = truthy(&source.getattr("scene")?, "show_all_cutters")?;
            let run = action(py, move |py| {
                let win = ui(py)?.call_method1("_window", (held_view.bind(py),))?;
                py.import("views.view_commands")?.call_method1("run", (win, "show_cutters")).map(drop)
            })?;
            out.append(("show_cutters", "Show All Cutters", "", i32::from(on), run))?;
        }
        "MoveTool" if tool.hasattr("_toggle_copy")? => {
            let run = action(py, move |py| held_tool.bind(py).call_method1("_toggle_copy", (held_host.bind(py),)).map(drop))?;
            out.append(("copy", "Copy", ctrl()?, flag("_copy")?, run))?;
        }
        "PushPullTool" if tool.hasattr("_keep_mode")? => {
            let run = action(py, move |py| {
                let (tool, host) = (held_tool.bind(py), held_host.bind(py));
                let keep = !tool.getattr("_keep_mode")?.is_truthy()?;
                tool.setattr("_keep_mode", keep)?;
                tool.setattr("_keep_base", keep)?;
                tool.call_method1("refresh_hint", (host,))?;
                host.call_method0("update").map(drop)
            })?;
            out.append(("keep", "Keep base", ctrl()?, flag("_keep_mode")?, run))?;
        }
        "RectangleTool" if tool.hasattr("_set_from_center")? => {
            let run = action(py, move |py| {
                let tool = held_tool.bind(py);
                tool.call_method1("_set_from_center", (held_host.bind(py), !tool.getattr("_from_center")?.is_truthy()?)).map(drop)
            })?;
            out.append(("centre", "From centre", ctrl()?, flag("_from_center")?, run))?;
        }
        "SelectTool" => {
            let selected = attr(&attr(&source, "scene")?, "selection")?;
            if !selected.is_none() && selected.len()? == 1 {
                let group = selected.try_iter()?.next().transpose()?.unwrap_or_else(|| none(py));
                if attr(&group, "text3d")?.is_truthy()? && !truthy(&group, "locked")? {
                    py.import("built_core.links")?;
                    if module.getattr("built_core")?.call_method1("link_linked_ancestor", (&group,))?.is_none() {
                        let group = group.unbind();
                        let run = action(py, move |py| {
                            let win = ui(py)?.call_method1("_window", (held_view.bind(py),))?;
                            win.call_method1("_on_edit_3d_text", (group.bind(py),)).map(drop)
                        })?;
                        out.append(("edittext3d", "Edit 3D Text…", "", 0, run))?;
                    }
                }
            }
        }
        "PlaceGroupTool" if attr(&tool.getattr("_group")?, "text3d")?.is_truthy()? => {
            let win = module.call_method1("_window", (&view,))?.unbind();
            let run = action(py, move |py| {
                let win = win.bind(py);
                let text = held_tool.bind(py).getattr("_group")?.getattr("text3d")?;
                render(py)?.getattr("font_picker")?.call_method1("text3d", (win, text, win.getattr("_place_text3d")?)).map(drop)
            })?;
            out.append(("textfont", "Text and font…", "", 0, run))?;
        }
        "DimensionTool" | "TextTool" => {
            let held_view = held_view.clone_ref(py);
            let run = action(py, move |py| {
                let win = ui(py)?.call_method1("_window", (held_view.bind(py),))?;
                render(py)?.getattr("font_picker")?.call_method1("annotation", (win,)).map(drop)
            })?;
            out.append(("font", "Font…", "", 0, run))?;
        }
        _ => {}
    }
    if name == "DimensionTool" {
        let held_view = view.clone().unbind();
        let run = action(py, move |py| direct(py, super::chrome_actions::open_form, (held_view.bind(py), "dimstyle")).map(drop))?;
        out.append(("dimstyle", "Dimension style…", "", 0, run))?;
    }
    let lock = attr(&source, "axis_lock")?;
    if lock.is_truthy()? {
        out.append(("axis", format!("{} axis locked", lock.call_method0("upper")?.extract::<String>()?), "", 2, py.None()))?;
    }
    if !source.is_none() && (attr(&source, "_previous_views")?.is_truthy()? || own(&source, "_isolate_base")?.is_some_and(|base| base.is_truthy().unwrap_or(false))) {
        let source = source.clone().unbind();
        let run = action(py, move |py| ui(py)?.call_method1("_show_all", (source.bind(py),)).map(drop))?;
        out.append(("showall", "Isolated · Show all", "", 1, run))?;
    }
    let mode = source.getattr_opt("linear_inference_mode")?;
    if let Some(mode) = mode.filter(|_| !source.is_none()) {
        if mode.eq("off")? {
            out.append(("inf", "Inferences off", "", 2, py.None()))?;
        } else if mode.eq("parallel_perp")? {
            out.append(("inf", "Parallel / perpendicular only", "", 2, py.None()))?;
        }
    }
    Ok(out.into_any())
}

pub(super) fn cluster<'py>(a: &Args<'py>) -> Reply<'py> {
    a.py().import("views.view_commands")?.call_method1("sync", (a.get_item(0)?, a.get_item(1)?))
}

/// The chrome's offset in the display, `(0, 0)` without one.
fn chrome_offset(display: &Obj<'_>) -> PyResult<(f64, f64)> {
    match display.getattr_opt("chrome_offset")? {
        Some(offset) => offset.extract(),
        None => Ok((0., 0.)),
    }
}

/// Label the parsed entry before translation, so its on-canvas readback
/// keeps the same caption and units after the tool clears its buffer.
fn typed_pill(py: Python, caption: &str, buffer: &str, value: String) -> PyResult<(String, String)> {
    let units = py.import("built_core.units")?;
    let parsed = units.call_method1("parse_value_box", (buffer,))?;
    let tag = parsed.cast::<PyTuple>().ok()
        .and_then(|tuple| tuple.get_item(0).ok())
        .and_then(|item| item.extract::<String>().ok());
    Ok(match tag.as_deref() {
        Some("array") if parsed.len()? == 3 && parsed.get_item(2)?.eq("x")? =>
            ("Copies".into(), format!("{}×", parsed.get_item(1)?)),
        Some("array") => ("Copies".into(), value),
        Some("segments") => ("Segments".into(), value),
        Some("radius") => ("Radius".into(), value),
        Some("profile") => ("Profile".into(), value),
        Some("ratio") if caption == "Angle" => {
            let angle = units.call_method1("fmt_angle_deg", (parsed.get_item(1)?,))?;
            (caption.into(), format!("{buffer} ({angle})"))
        }
        None if caption == "Angle" && parsed.extract::<f64>().is_ok() =>
            (caption.into(), format!("{}°", buffer.trim())),
        _ => (caption.into(), value),
    })
}

/// The value field on the geometry (Maxence 2026-11-22), drawn and kept by
/// the chrome (ui/value_field.rs): the tool's caption, the value where the
/// tool shows it (`_length_label_at`), the tool's start on screen and the
/// document revision, from which Rust reads a commit and its readback. The
/// same in both hosts: it reads the tool, not a status bar.
fn cursor_value<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, win) = (a.get_item(0)?, a.get_item(1)?);
    let tr = py.import("built_core.i18n")?.getattr("tr")?;
    let tools = attr(&view, "tools")?;
    let tool = attr(&tools, "active_tool")?;
    let caption = match attr(&tool, "vcb_caption")? {
        caption if caption.is_callable() => caption.call0()?,
        _ => attr(&tool, "vcb_label")?,
    };
    let label = if caption.is_none() {
        none(py)
    } else {
        py.import("views.document_interaction")?.getattr("DocumentInteraction")?.call_method1("_length_label_at", (&tools,))?
    };
    let display = tools.getattr_opt("view")?.unwrap_or_else(|| view.clone());
    let sweep=attr(&tool,"kind")?.eq("sweep")?;
    let cut=attr(&tool,"kind")?.eq("cut")?;
    let at = if (sweep || cut) && !attr(&tool,"hover_point")?.is_none(){
        display.call_method1("_world_to_pixel",(tool.getattr("hover_point")?,))?
    }else if label.is_none() { none(py) } else { display.call_method1("_world_to_pixel", (label.get_item(1)?,))? };
    let history = attr(&attr(&win, "viewport")?, "history")?;
    let revision = match attr(&history, "revision")? {
        revision if revision.is_truthy()? => revision.call_method0("__int__")?,
        _ => 0.into_bound_py_any(py)?,
    };
    let renderer = view.getattr("renderer")?;
    if renderer.hasattr("ui_set_section_inset")?{
        let inset=if sweep && !at.is_none(){tool.call_method1("_state",("section_inset",))?}else{none(py)};
        renderer.call_method1("ui_set_section_inset",(inset,))?;
    }
    if tool.is_none() && attr(&display.getattr("source_viewport")?, "nav_mode")?.eq("zoom")? {
        direct(py, lens_value, (&view, &tools, &revision))?;
        return Ok(none(py));
    }
    let buffer: String = attr(&tools, "value_buffer")?.extract::<Option<String>>()?.unwrap_or_default();
    let typing = !buffer.is_empty();
    let caption = caption.extract::<Option<String>>()?.unwrap_or_default();
    let value = if label.is_none() {
        render(py)?.call_method1("pending_tool_value", (&tool, &buffer))?.extract::<String>()?
    } else {
        label.get_item(0)?.extract::<String>()?
    };
    let (caption, value) = if typing && !caption.is_empty() {
        typed_pill(py, &caption, &buffer, value)?
    } else {
        (caption, value)
    };
    let (shown,value) = if !typing && value == "Repeat run" && tool.hasattr("_repeat_multipliers")? {
        (tr.call1(("Repeat run",))?, tool.getattr("_repeat_multipliers")?.extract::<String>()?)
    } else if sweep && !typing {
        let chip=tool.call_method1("_state",("chip",))?;
        let dist:Option<f64>=tool.call_method1("_state",("length",))?.extract()?;
        match dist {
            Some(n)=>(chip,py.import("built_core.units")?.call_method1("fmt_len",(n,))?.extract::<String>()?),
            None=>(tr.call1(("Sweep",))?,chip.extract::<String>()?),
        }
    } else if cut && !typing && tool.call_method1("_state",("editing_grip",))?.is_truthy()? {
        let value=tool.call_method1("_state",("grip_value",))?.extract::<f64>()?;
        let unit=py.import("built_core.units")?;
        let value=if caption=="Angle"{unit.call_method1("fmt_angle_deg",(value,))?}else{unit.call_method1("fmt_len",(value/1000.,))?};
        (tr.call1((&caption,))?,value.extract()?)
    } else if cut && !typing {
        let prompt=tool.call_method1("_state",("prompt",))?;
        let chip=tr.call1((prompt,))?;
        let offset:f64=tool.call_method1("_state",("offset",))?.extract()?;
        (chip,py.import("built_core.units")?.call_method1("fmt_len",(offset/1000.,))?.extract::<String>()?)
    } else {(tr.call1((&caption,))?,value)};
    // The tool's refusal reads in the error colour (Fillet: a size that
    // does not fit the selection).
    let error = match tool.getattr_opt("value_error")? {
        Some(flag) => flag.is_truthy()?,
        None => false,
    };
    if at.is_none() {
        let shown = if typing { shown } else { "".into_bound_py_any(py)? };
        renderer.call_method1("ui_set_cursor_value", (shown, &value, typing, py.None(), py.None(), py.None(), &revision, error))?;
        return Ok(none(py));
    }
    let (dx, dy) = chrome_offset(&display)?;
    let xy = |point: &Obj<'py>| -> PyResult<(f64, f64)> {
        let (x, y): (f64, f64) = if point.hasattr("x")? {
            (point.call_method0("x")?.extract()?, point.call_method0("y")?.extract()?)
        } else {
            point.extract()?
        };
        Ok((x + dx, y + dy))
    };
    let start = match attr(&tool, "start_point")? {
        start if start.is_none() => None,
        start => Some(display.call_method1("_world_to_pixel", (start,))?).filter(|start| !start.is_none()),
    };
    let start = match start {
        Some(start) => Some(xy(&start)?),
        None => None,
    };
    let (x, y) = xy(&at)?;
    let mut top = y;
    let mut bottom = y;
    if let Some(bands) = tool.getattr_opt("rubber_band_lines")? {
        let segments = bands.call0()?;
        if segments.len()? <= 64 {
            for segment in segments.try_iter()? {
                let segment = segment?;
                for endpoint in segment.try_iter()? {
                    let pixel = display.call_method1("_world_to_pixel", (endpoint?,))?;
                    if !pixel.is_none() {
                        let (_, py) = xy(&pixel)?;
                        top = top.min(py);
                        bottom = bottom.max(py);
                    }
                }
            }
        }
    }
    let beside = if top >= 68. { top - 24. } else { bottom + 24. };
    renderer.call_method1("ui_set_cursor_value", (shown, &value, typing, x, beside, start, &revision, error))?;
    Ok(none(py))
}

/// Zoom takes the lens typed in the value field (35mm, 60deg; viewport
/// `_handle_zoom_value_key`): the same field, by the cursor, reading the
/// focal length the view has now. The lens is what the value acts on, so it
/// is part of the revision Rust reads a commit from.
pub(super) fn lens_value<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, tools, revision) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?);
    let display = tools.getattr_opt("view")?.unwrap_or_else(|| view.clone());
    let position = attr(&display, "_cursor_pos")?;
    let camera = display.getattr("source_viewport")?.getattr("camera")?;
    let (dx, dy) = chrome_offset(&display)?;
    let renderer = view.getattr("renderer")?;
    if position.is_none() {
        renderer.call_method1("ui_set_cursor_value", ("", "", false, 0.0, 0.0, py.None(), &revision))?;
        return Ok(none(py));
    }
    let buffer = match attr(&tools, "value_buffer")? {
        buffer if buffer.is_truthy()? => buffer.str()?.extract::<String>()?,
        _ => String::new(),
    };
    let typing = !buffer.is_empty();
    let text = if typing {
        buffer
    } else if camera.getattr("perspective")?.is_truthy()? {
        py.import("builtins")?.call_method1("format", (camera.getattr("focal_mm")?, ".0f"))?.extract::<String>()? + " mm"
    } else {
        "—".to_owned()
    };
    let field = py.import("builtins")?.call_method1("round", (camera.getattr("fov_deg")?.mul(1000)?,))?.call_method0("__int__")?;
    let lens = revision.lshift(20)?.bitxor(field)?;
    let (x, y): (f64, f64) = (position.call_method0("x")?.extract()?, position.call_method0("y")?.extract()?);
    renderer.call_method1(
        "ui_set_cursor_value",
        (py.import("built_core.i18n")?.call_method1("tr", ("Field of view",))?, text, typing, x + dx, y + dy - 40.0, py.None(), lens),
    )?;
    Ok(none(py))
}

/// The selection as the providers' signatures see it: its native revision
/// (core/element_set.py), taken on every change of content and for every
/// new set. Every empty selection is one and the same.
pub(super) fn sel_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let selection = a.get_item(0)?;
    if selection.len()? != 0 { selection.getattr("revision") } else { 0.into_bound_py_any(a.py()) }
}

/// "2 groups · 3 items" from `selection.snapshot()`: placements are the
/// groups, the rest are items.
fn summary<'py>(a: &Args<'py>) -> Reply<'py> {
    let snapshot = a.get_item(0)?;
    let (count, groups): (i64, i64) = (snapshot.get_item(0)?.extract()?, snapshot.get_item(1)?.extract()?);
    let rest = count - groups;
    let counted = |count: i64, noun: &str| format!("{count} {noun}{}", if count != 1 { "s" } else { "" });
    let mut parts = Vec::new();
    if groups != 0 {
        parts.push(counted(groups, "group"));
    }
    if rest != 0 {
        parts.push(counted(rest, "item"));
    }
    parts.join(" · ").into_bound_py_any(a.py())
}

/// The selection bar: with Select active, something selected, no drag.
fn selection<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (view, win, active) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?);
    let module = ui(py)?;
    let scene = attr(&view, "scene")?;
    let selected = attr(&scene, "selection")?;
    let dragging = !attr(&view, "last")?.is_none();
    let renderer = view.getattr("renderer")?;
    if active.ne("select")? || selected.is_none() || selected.len()? == 0 || dragging {
        if !attr(&view, "_ui_sel_sig")?.is_none() {
            renderer.call_method1("ui_set_selection", ("", PyList::empty(py)))?;
            view.setattr("_ui_sel_sig", py.None())?;
        }
        return Ok(none(py));
    }
    let sig = (id(&scene), direct(py, sel_key, (&selected,))?).into_bound_py_any(py)?;
    if sig.eq(attr(&view, "_ui_sel_sig")?)? {
        return Ok(none(py));
    }
    view.setattr("_ui_sel_sig", sig)?;
    let verbs = PyList::empty(py);
    for verb in direct(py, commands, (&win, &selected))?.try_iter()? {
        let [key, label, shortcut, _run, enabled, more] = unpack::<6>(&verb?)?;
        verbs.append((key, label, shortcut, enabled, more))?;
    }
    renderer.call_method1("ui_set_selection", (module.call_method1("_summary", (selected.call_method0("snapshot")?,))?, verbs))?;
    Ok(none(py))
}

/// (id, label, shortcut, run, enabled, in More) for the inspector's verbs:
/// the verb source's (verbs.rs), the very entries the right-click menu
/// offers this selection, run the same way (viewport_menu.rs).
pub(super) fn commands<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    // The selection itself when it keeps its kinds apart: a huge one is
    // then read by kind (viewport_menu::Kinds::bulk), not handle by handle.
    let selection = a.get_item(1)?;
    let sel = if selection.hasattr("values_of")? { selection } else { py.get_type::<PyList>().call1((selection,))? };
    render(py)?.call_method1("inspector_verbs", (a.get_item(0)?, sel))
}

/// «Isolated · Show all»: back to exactly what was shown before the first
/// Isolate (a saved view's filter included), and the selection Isolate
/// dropped, when nothing changed in between.
fn show_all<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let viewport = a.get_item(0)?;
    let facets = py.import("core.agent_facets")?;
    let base = viewport.getattr("__dict__")?.call_method1("pop", ("_isolate_base", py.None()))?;
    if !base.is_none() {
        let [state, previous, selection, content] = unpack::<4>(&base)?;
        let [expression, colour, isolate, key, exact] = unpack::<5>(&state)?;
        let expression = py.import("copy")?.call_method1("deepcopy", (expression,))?;
        facets.call_method("set_display", (&viewport, expression, colour), Some(&fields!(py; "isolate" => isolate, "key" => key, "remember" => false, "exact" => exact)))?;
        viewport.setattr("_previous_views", previous)?;
        let scene = viewport.getattr("scene")?;
        if content.eq(scene.getattr("content_version")?)? {
            let kept = PyList::empty(py);
            for entity in selection.try_iter()? {
                let entity = entity?;
                if scene.call_method1("entity_selectable", (&entity,))?.is_truthy()? {
                    kept.append(entity)?;
                }
            }
            scene.call_method1("select", (kept.try_iter()?,))?;
        }
        let update = viewport.getattr("update")?;
        viewport.getattr_opt("notify_scene_changed")?.unwrap_or(update).call0()?;
    } else if viewport.getattr_opt("_previous_views")?.is_some_and(|previous| previous.is_truthy().unwrap_or(false)) {
        facets.call_method1("previous_display", (&viewport,))?;
    } else {
        facets.call_method("set_display", (&viewport,), Some(&fields!(py; "remember" => false)))?;
    }
    Ok(none(py))
}

/// Show only the placements `uids` (at any depth: a part of a linked file
/// shows through the file's placement) until «Show all».
pub(super) fn isolate<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let win = a.get_item(0)?;
    let viewport = win.getattr("viewport")?;
    let scene = viewport.getattr("scene")?;
    let mut uids: Vec<Obj<'py>> = a.get_item(1)?.try_iter()?.collect::<PyResult<_>>()?;
    if uids.iter().any(|uid| !uid.is_instance_of::<PyString>()) {
        // A placement's key is its UID, or its address when the UID repeats
        // (built_core.facets): from the native tree, not the facet index.
        let row_id = py.import("built_core.link_structure")?.getattr("row_id")?;
        let repeated = scene.call_method0("repeated_uids")?;
        for entry in uids.iter_mut() {
            if entry.is_instance_of::<PyString>() {
                continue;
            }
            let uid = entry.getattr("uid")?;
            *entry = if repeated.contains(&uid)? || uid.call_method1("startswith", ("@[",))?.is_truthy()? {
                let path = scene.call_method1("occurrence_path", (&*entry,))?;
                row_id.call1((if path.is_truthy()? { path } else { PyList::new(py, [entry.getattr("uid")?])?.into_any() },))?
            } else {
                uid
            };
        }
    }
    let unique = PyDict::new(py);
    for uid in uids {
        if uid.is_truthy()? {
            unique.set_item(uid, py.None())?;
        }
    }
    if unique.is_empty() {
        viewport.call_method1("flash_status", ("Nothing to isolate: select objects first",))?;
        return Ok(none(py));
    }
    let facets = py.import("core.agent_facets")?;
    if own(&viewport, "_isolate_base")?.is_none() {
        let state = (
            py.import("copy")?.call_method1("deepcopy", (attr(&viewport, "_view_expression")?,))?,
            attr(&viewport, "_view_color")?,
            viewport.getattr_opt("_view_isolate")?.unwrap_or(true.into_bound_py_any(py)?),
            attr(&viewport, "active_view_key")?,
            viewport.getattr_opt("_view_exact")?.unwrap_or(false.into_bound_py_any(py)?),
        );
        let previous = listed(py, attr(&viewport, "_previous_views")?)?;
        let selection = py.get_type::<PyList>().call1((scene.getattr("selection")?,))?;
        viewport.setattr("_isolate_base", (state, previous, selection, scene.getattr("content_version")?))?;
    }
    let shown = fields!(py; "facet" => "occurrence", "in" => unique.keys());
    facets.call_method("set_display", (&viewport, shown), Some(&fields!(py; "isolate" => true, "exact" => false)))?;
    Ok(none(py))
}

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Method] = &[
    ("_window", Mode::Function, window, &["view"], "", ""),
    ("attach", Mode::Function, attach, &["view"], "", ""),
    ("dock_width", Mode::Function, dock_width, &["lens", "settings=None"], "", ""),
    ("apply_dock_width", Mode::Function, apply_dock_width, &["view", "settings=None"], "", ""),
    ("keep_dock_width", Mode::Function, keep_dock_width, &["view", "arg", "settings=None"], "", ""),
    ("_send_tools", Mode::Function, send_tools, &["view", "win"], "", ""),
    ("_project_action", Mode::Function, project_action, &["win", "arg"], "", ""),
    ("_tabs", Mode::Function, tabs, &["view", "win"], "", ""),
    ("_sheet_and_jobs", Mode::Function, sheet_and_jobs, &["view", "win"], "", ""),
    ("chrome_frame", Mode::Function, chrome_frame, &["view"], "", ""),
    ("_sync_units", Mode::Function, sync_units, &["view", "history"], "", ""),
    ("before_frame", Mode::Function, before_frame, &["view"], "", ""),
    ("_options", Mode::Function, options, &["view"], "", ""),
    ("_cursor_value", Mode::Function, cursor_value, &["view", "win"], "", ""),
    ("_commands", Mode::Function, commands, &["win", "sel"], "", ""),
    ("_summary", Mode::Function, summary, &["snapshot"], "", ""),
    ("_selection", Mode::Function, selection, &["view", "win", "active"], "", ""),
    ("_show_all", Mode::Function, show_all, &["vp"], "", ""),
    ("_isolate", Mode::Function, isolate, &["win", "uids"], "", ""),
];
