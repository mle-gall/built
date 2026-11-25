//! Window entry points shared by the renderer, the shell and input dispatch.
use crate::document_registration::view::calls;
use super::{Obj, SheetHost};
use pyo3::{prelude::*, types::PyDict};
use std::time::Instant;

fn workspace<'py>(view: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    if view.is_none() {
        return Ok(None);
    }
    let vars = view.getattr("__dict__")?;
    let ws = vars.cast::<PyDict>()?.get_item("_sheets_ws")?;
    match ws {
        Some(ws)
            if !ws.is_none()
                && ws.getattr("shown")?.is_truthy()?
                && ws.getattr("view")?.is(view) =>
        {
            Ok(Some(ws))
        }
        _ => Ok(None),
    }
}

#[pyfunction]
fn workspace_of<'py>(view: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    workspace(view)
}
#[pyfunction]
fn shown(view: &Obj<'_>) -> PyResult<bool> {
    Ok(workspace(view)?.is_some())
}

fn host<'py>(win: &Obj<'py>) -> Obj<'py> {
    win.getattr("_host_window").unwrap_or_else(|_| win.clone())
}

#[pyfunction]
fn composer_for<'py>(win: &Obj<'py>) -> PyResult<Obj<'py>> {
    let win = host(win);
    let composer = win.getattr("_composer")?;
    if !composer.is_none() {
        return Ok(composer);
    }
    let py = win.py();
    let doc = py
        .import("built_render.shell_views")?
        .call_method1("SheetDocumentHost", (&win, win.getattr("_doc")?))?;
    let composer = py.get_type::<SheetHost>().call1((doc,))?;
    win.setattr("_composer", &composer)?;
    win.call_method0("_update_title")?;
    Ok(composer)
}

#[pyfunction]
#[pyo3(signature=(win,index=None))]
fn to_sheets(win: &Obj<'_>, index: Option<isize>) -> PyResult<()> {
    let win = host(win);
    let comp = composer_for(&win)?;
    let sheets = win
        .getattr("viewport")?
        .getattr("scene")?
        .getattr("compositions")
        .ok();
    let count = sheets
        .filter(|s| !s.is_none())
        .map(|s| s.len())
        .transpose()?
        .unwrap_or(0);
    let index = index.or_else(|| {
        (count > 0).then(|| {
            win.getattr("_last_sheet")
                .and_then(|v| v.extract::<isize>())
                .unwrap_or(0)
                .clamp(0, count as isize - 1)
        })
    });
    comp.call_method0("show")?;
    if let Some(i) = index {
        comp.call_method1("show_sheet", (i,))?;
    }
    Ok(())
}

#[pyfunction]
pub(super) fn to_model(win: &Obj<'_>) -> PyResult<()> {
    if let Ok(comp) = host(win).getattr("_composer") {
        if comp
            .getattr("EMBEDDED")
            .is_ok_and(|v| v.is_truthy().unwrap_or(false))
        {
            comp.call_method0("hide")?;
        }
    }
    Ok(())
}

#[pyfunction]
fn frame<'py>(view: &Obj<'py>) -> PyResult<(Obj<'py>, Obj<'py>, f64)> {
    let py = view.py();
    let ui = py.import("views.wgpu_ui")?;
    let ws = workspace(view)?
        .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("sheet workspace is closed"))?;
    let renderer = view.getattr("renderer")?;
    let start = Instant::now();
    let win = ui.call_method1("_window", (view,))?;
    if !win.is_none()
        && view
            .getattr("_ui_attached")
            .is_ok_and(|v| v.is_truthy().unwrap_or(false))
    {
        calls::sheet_and_jobs(view, &win)?;
        for module in [
            "built_render.wgpu_start",
            "built_render.chrome_sheet",
        ] {
            py.import(module)?
                .call_method1("before_frame", (view, &win))?;
        }
        if !view.getattr("_ui_tools_sent")?.is_truthy()?
            && win.getattr("_tool_actions")?.is_truthy()?
        {
            calls::send_tools(view, &win)?;
        }
        calls::document_tabs(view)?
            .call_method1(
                "sync_projects",
                (&renderer, &win, ui.getattr("LIGHTS_ROOM")?),
            )?;
        calls::tabs(view, &win)?;
    }
    if let Err(e) = ws.call_method1("before_frame", (view,)) {
        e.print(py);
    }
    let (cpu, gpu) = renderer.call_method0("render_page")?.extract()?;
    ui.call_method1("after_frame", (view,))?;
    if ws.call_method0("timers_pending")?.is_truthy()? {
        if let Ok(timer) = view.getattr("_ui_timer") {
            if !timer.is_none()
                && (!timer.call_method0("isActive")?.is_truthy()?
                    || timer.call_method0("remainingTime")?.extract::<i64>()? > 100)
            {
                timer.call_method1("start", (100,))?;
            }
        }
    }
    Ok((cpu, gpu, start.elapsed().as_secs_f64() * 1000.))
}

#[pyfunction]
fn pointer(view: &Obj<'_>, kind: &str, event: &Obj<'_>) -> PyResult<bool> {
    let Some(ws) = workspace(view)? else {
        return Ok(false);
    };
    ws.call_method1("pointer", (kind, event))?;
    Ok(true)
}
#[pyfunction]
fn chrome_took(view: &Obj<'_>) -> PyResult<()> {
    if let Some(ws) = workspace(view)? {
        ws.call_method0("chrome_took")?;
    }
    Ok(())
}
#[pyfunction]
fn wheel(view: &Obj<'_>, event: &Obj<'_>) -> PyResult<bool> {
    let Some(ws) = workspace(view)? else {
        return Ok(false);
    };
    ws.call_method1("wheel", (event,))?;
    Ok(true)
}
#[pyfunction]
fn context_menu(view: &Obj<'_>, event: &Obj<'_>) -> PyResult<bool> {
    let Some(ws) = workspace(view)? else {
        return Ok(false);
    };
    ws.call_method1("context_menu", (event,))?;
    Ok(true)
}
#[pyfunction]
fn gesture(view: &Obj<'_>, event: &Obj<'_>) -> PyResult<bool> {
    let Some(ws) = workspace(view)? else {
        return Ok(false);
    };
    ws.call_method1("gesture", (event,))?.is_truthy()
}
#[pyfunction]
fn claims_key(view: &Obj<'_>, event: &Obj<'_>) -> PyResult<bool> {
    let Some(ws) = workspace(view)? else {
        return Ok(false);
    };
    Ok(!ws.call_method1("window_key", (event,))?.is_truthy()?)
}
#[pyfunction]
fn key(view: &Obj<'_>, event: &Obj<'_>, pressed: bool) -> PyResult<bool> {
    let Some(ws) = workspace(view)? else {
        return Ok(false);
    };
    if ws.call_method1("window_key", (event,))?.is_truthy()? {
        return Ok(false);
    }
    ws.call_method1("key", (event, pressed))?;
    Ok(true)
}
#[pyfunction]
fn action(win: &Obj<'_>, arg: &str) -> PyResult<()> {
    let Ok(view) = win.getattr("_doc").and_then(|d| d.getattr("stand_in")) else {
        return Ok(());
    };
    if let Some(ws) = workspace(&view)? {
        if let Err(e) = ws.call_method1("action", (arg,)) {
            e.print(win.py());
            ws.call_method1("flash", ("That did not work — see the log",))?;
        }
    }
    Ok(())
}
#[pyfunction]
fn timers_pending(view: &Obj<'_>) -> PyResult<bool> {
    let Some(ws) = workspace(view)? else {
        return Ok(false);
    };
    ws.call_method0("timers_pending")?.is_truthy()
}

pub(super) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    for f in [
        wrap_pyfunction!(workspace_of, module)?,
        wrap_pyfunction!(shown, module)?,
        wrap_pyfunction!(composer_for, module)?,
        wrap_pyfunction!(to_sheets, module)?,
        wrap_pyfunction!(to_model, module)?,
        wrap_pyfunction!(frame, module)?,
        wrap_pyfunction!(pointer, module)?,
        wrap_pyfunction!(chrome_took, module)?,
        wrap_pyfunction!(wheel, module)?,
        wrap_pyfunction!(context_menu, module)?,
        wrap_pyfunction!(gesture, module)?,
        wrap_pyfunction!(claims_key, module)?,
        wrap_pyfunction!(key, module)?,
        wrap_pyfunction!(action, module)?,
        wrap_pyfunction!(timers_pending, module)?,
    ] {
        module.add_function(f)?;
    }
    Ok(())
}
