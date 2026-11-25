//! Temporarily give a plugin the native window's document controller.
//! The parked model retains its undo history, file identity and pane layout.
use pyo3::{prelude::*, types::PyDict};

#[pyfunction]
fn workspace_tools(win: &Bound<'_, PyAny>, allowed: &Bound<'_, PyAny>) -> PyResult<()> {
    for (key, action) in win.getattr("_tool_actions")?.cast::<PyDict>()?.iter() {
        action.call_method1("setEnabled", (allowed.is_none() || allowed.contains(key)?,))?;
    }
    Ok(())
}

#[pyfunction]
fn enter_workspace(win: &Bound<'_, PyAny>, ws: &Bound<'_, PyAny>) -> PyResult<bool> {
    if !win.getattr("_workspace")?.is_none()
        || win.getattr("_open_in_progress")?.is_truthy()?
        || win.getattr("_opening_document")?.is_truthy()?
    {
        return Ok(false);
    }
    let py = win.py();
    let incoming = ws.getattr("scene")?;
    let history = ws.getattr("history")?;
    let allowed = ws
        .getattr("allowed_tools")
        .unwrap_or_else(|_| py.None().into_bound(py));
    let camera = ws.getattr("camera").ok();
    let doc = win.getattr("_doc")?;
    let vp = doc.getattr("viewport")?;
    let panes = doc.getattr("model_views")?;
    let parked = PyDict::new(py);
    for key in ["_current_path", "_import_name", "_saved_content"] {
        parked.set_item(key, win.getattr(key)?)?;
    }
    parked.set_item("camera", win.call_method1("_camera_dict", (&vp,))?)?;
    parked.set_item("panes", &panes)?;
    parked.set_item("composer", win.getattr("_composer")?)?;
    parked.set_item("last_sheet", win.getattr("_last_sheet")?)?;
    let sheet = win.getattr("_composer")?;
    let showing = !sheet.is_none() && sheet.call_method0("isVisible")?.is_truthy()?;
    parked.set_item("sheets_shown", showing)?;
    if showing {
        sheet.call_method0("hide")?;
    }
    let single = py
        .import("built_render")?
        .call_method0("DocumentViews")?;
    win.call_method0("_dismiss_home")?;
    win.call_method1("_activate_tool", ("select",))?;
    panes.call_method1("suspend", (&doc,))?;
    doc.setattr("model_views", single)?;
    vp.call_method0("end_group_edit")?;
    parked.set_item("clean", !win.call_method0("_is_dirty")?.is_truthy()?)?;
    parked.set_item("scene", vp.getattr("scene")?)?;
    parked.set_item("history", vp.getattr("history")?)?;
    win.setattr("_parked", &parked)?;
    win.setattr("_workspace", ws)?;
    win.setattr("_composer", py.None())?;
    win.setattr("_last_sheet", 0)?;
    win.setattr("_current_path", py.None())?;
    win.setattr("_import_name", py.None())?;
    let entered = || -> PyResult<()> {
        vp.call_method1("set_document", (incoming, history))?;
        workspace_tools(win, &allowed)?;
        if let Some(camera) = camera {
            win.call_method1("_apply_camera_dict", (camera,))?;
        }
        win.call_method0("_refresh_scene_tabs")?;
        win.call_method0("_update_title")?;
        Ok(())
    };
    if let Err(error) = entered() {
        restore(win, &parked)?;
        return Err(error);
    }
    Ok(true)
}

fn restore(win: &Bound<'_, PyAny>, parked: &Bound<'_, PyDict>) -> PyResult<()> {
    let py = win.py();
    let doc = win.getattr("_doc")?;
    let vp = doc.getattr("viewport")?;
    let get = |k: &str| parked.as_any().get_item(k);
    doc.getattr("model_views")?
        .call_method1("suspend", (&doc,))?;
    let model = get("scene")?;
    let clean = get("clean")?.is_truthy()?
        && model
            .getattr("content_version")?
            .eq(get("_saved_content")?)?;
    vp.call_method1("set_document", (&model, get("history")?))?;
    win.setattr("_workspace", py.None())?;
    win.setattr("_parked", py.None())?;
    for key in ["_current_path", "_import_name", "_saved_content"] {
        win.setattr(key, get(key)?)?;
    }
    if clean {
        win.setattr("_saved_content", model.getattr("content_version")?)?;
    }
    win.setattr("_composer", get("composer")?)?;
    win.setattr("_last_sheet", get("last_sheet")?)?;
    win.call_method1("_apply_camera_dict", (get("camera")?,))?;
    doc.setattr("model_views", get("panes")?)?;
    workspace_tools(win, &py.None().into_bound(py))?;
    if get("sheets_shown")?.is_truthy()? {
        get("composer")?.call_method0("show")?;
    }
    win.call_method0("_refresh_scene_tabs")?;
    win.call_method0("_update_title")?;
    Ok(())
}

#[pyfunction]
fn leave_workspace(win: &Bound<'_, PyAny>) -> PyResult<bool> {
    let ws = win.getattr("_workspace")?;
    if ws.is_none() {
        return Ok(true);
    }
    if !ws.call_method0("confirm_leave")?.is_truthy()? {
        return Ok(false);
    }
    let parked = win.getattr("_parked")?.cast_into::<PyDict>()?;
    let vp = win.getattr("_doc")?.getattr("viewport")?;
    if ws.hasattr("camera")? {
        ws.setattr("camera", win.call_method1("_camera_dict", (&vp,))?)?;
    }
    win.call_method1("_activate_tool", ("select",))?;
    restore(win, &parked)?;
    if ws.hasattr("left")? {
        ws.call_method0("left")?;
    }
    Ok(true)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(enter_workspace, m)?)?;
    m.add_function(wrap_pyfunction!(leave_workspace, m)?)?;
    Ok(())
}
