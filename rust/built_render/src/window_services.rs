//! Native-window services over the active document, timer and plugin backends.
use pyo3::{prelude::*, types::PyDict};
use std::time::{Duration, Instant};

fn attrs<'py>(win: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyDict>> {
    Ok(win.getattr("__dict__")?.cast_into::<PyDict>()?)
}

#[pyfunction]
fn configure_recovery(win: &Bound<'_, PyAny>, timer: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let read = py.import("built_core.general_settings")?.getattr("value")?;
    let store = py.import("built_core.settings")?.call_method0("settings")?;
    let enabled = read.call1(("autosave", "1", &store))?.str()?.to_str()? != "0";
    let minutes = read.call1(("autosave_min", 5, store))?;
    let minutes: i64 = py
        .import("builtins")?
        .call_method1("int", (minutes,))?
        .extract()?;
    timer.call_method0("stop")?;
    if enabled {
        timer.call_method1("start", (minutes.clamp(1, 60) * 60_000,))?;
    } else if let Some(worker) = attrs(win)?
        .get_item("_background_autosave")?
        .filter(|w| !w.is_none())
    {
        worker.call_method0("cancel")?;
    }
    Ok(())
}

/// Avoid collecting during a drag or when a previous full collection took
/// too long. The background writer itself owns the recovery slots and jobs.
#[pyfunction]
fn request_recovery(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let d = attrs(win)?;
    if d.get_item("_opening_document")?
        .is_some_and(|v| v.is_truthy().unwrap_or(false))
    {
        return Ok(());
    }
    let py = win.py();
    let blocks: u64 = py
        .import("sys")?
        .call_method0("getallocatedblocks")?
        .extract()?;
    if blocks > 40_000_000 {
        d.set_item("_gc_too_slow", true)?;
    }
    let dragging = win
        .getattr("viewport")?
        .getattr("_last_pos")
        .is_ok_and(|v| !v.is_none());
    let slow = d
        .get_item("_gc_too_slow")?
        .is_some_and(|v| v.is_truthy().unwrap_or(false));
    if !dragging && !slow {
        let started = Instant::now();
        py.import("gc")?.call_method0("collect")?;
        if started.elapsed() > Duration::from_millis(500) {
            d.set_item("_gc_too_slow", true)?;
        }
    }
    let manager = match d.get_item("_background_autosave")?.filter(|v| !v.is_none()) {
        Some(manager) => manager,
        None => {
            let manager = crate::background_autosave::background_autosave(win)?;
            let result = py
                .import("built_render.chrome_dialogs")?
                .getattr("autosave_result")?;
            let callback = py
                .import("functools")?
                .call_method1("partial", (result, win))?;
            manager
                .getattr("finished")?
                .call_method1("connect", (callback,))?;
            d.set_item("_background_autosave", &manager)?;
            manager
        }
    };
    let docs = py
        .import("builtins")?
        .call_method1("list", (win.getattr("_documents")?,))?;
    manager.call_method1("request", (docs,))?;
    Ok(())
}

#[pyfunction]
fn remember_document(
    win: &Bound<'_, PyAny>,
    path: &Bound<'_, PyAny>,
    doc: &Bound<'_, PyAny>,
) -> PyResult<()> {
    if path.is_none() {
        return Ok(());
    }
    let py = win.py();
    let suffix = py
        .import("pathlib")?
        .getattr("Path")?
        .call1((path,))?
        .getattr("suffix")?
        .call_method0("lower")?;
    if py
        .import("built_core.formats.built")?
        .getattr("DOCUMENT_SUFFIXES")?
        .contains(suffix)?
    {
        py.import("built_core.recent")?
            .call_method1("remember", (path,))?;
        let kw = PyDict::new(py);
        kw.set_item("doc", doc)?;
        py.import("built_render.wgpu_start")?
            .call_method("note", (win, path), Some(&kw))?;
    }
    Ok(())
}

#[pyfunction]
fn link_watcher<'py>(win: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let d = attrs(win)?;
    if let Some(watcher) = d.get_item("_links_watch")?.filter(|v| !v.is_none()) {
        return Ok(watcher);
    }
    let watcher = Bound::new(win.py(), super::link_watch::LinkWatch::new(win)?)?.into_any();
    d.set_item("_links_watch", &watcher)?;
    Ok(watcher)
}

#[pyfunction]
fn links_loaded(win: &Bound<'_, PyAny>) -> PyResult<()> {
    win.call_method0("_link_watcher")?.call_method0("refresh")?;
    let py = win.py();
    let dialogs = py.import("built_render.chrome_dialogs")?;
    if !dialogs.call_method1("live", (win,))?.is_none()
        && dialogs
            .call_method1("update_link_banner", (win,))?
            .is_truthy()?
    {
        dialogs.call_method1("_repaint", (win,))?;
    } else if let Ok(errors) = win
        .getattr("viewport")?
        .getattr("scene")?
        .getattr("link_errors")
    {
        if errors.is_truthy()? {
            let tr = py.import("built_core.i18n")?.getattr("tr")?;
            let mut body=tr.call1(("Some linked documents could not be loaded; their copies stay in place, empty, until the files are found:",))?.extract::<String>()?;
            body.push('\n');
            for error in errors.call_method0("values")?.try_iter()? {
                body.push_str("\n• ");
                body.push_str(error?.str()?.to_str()?);
            }
            dialogs.call_method1("warning", (win, tr.call1(("Linked files",))?, body))?;
        }
    }
    Ok(())
}
#[pyfunction]
fn tool_key(tools: &Bound<'_, PyDict>, tool: &Bound<'_, PyAny>) -> Option<Py<PyAny>> {
    tools
        .iter()
        .find(|(_, t)| t.is(tool))
        .map(|(k, _)| k.unbind())
}
#[pyfunction]
fn toggle_eyedropper(win: &Bound<'_, PyAny>, armed: bool) -> PyResult<()> {
    let py = win.py();
    py.import("tools.paint")?
        .getattr("PaintTool")?
        .setattr("sample_armed", armed)?;
    if armed {
        win.call_method1("_activate_tool", ("paint",))?;
        let hint = py
            .import("built_core.i18n")?
            .call_method1("tr", ("Click a face to sample its material",))?;
        win.getattr("viewport")?
            .call_method1("flash_status", (hint,))?;
    }
    win.getattr("viewport")?
        .call_method0("_apply_tool_cursor")?;
    Ok(())
}
/// Registration is per window; a transferred tab must use the destination's callbacks.
#[pyfunction]
fn attach_viewport_extensions(win: &Bound<'_, PyAny>, vp: &Bound<'_, PyAny>) -> PyResult<()> {
    let attrs = attrs(win)?;
    let shared = match attrs.get_item("_ext_shared")? {
        Some(shared) => shared.cast_into::<PyDict>()?,
        None => {
            let shared = PyDict::new(win.py());
            for name in ["_ext_overlays", "_ext_snap_providers", "_ext_pickables"] {
                let rows = vp.getattr(name).or_else(|e| {
                    if e.is_instance_of::<pyo3::exceptions::PyAttributeError>(win.py()) {
                        Ok(pyo3::types::PyList::empty(win.py()).into_any())
                    } else { Err(e) }
                })?;
                shared.set_item(name, rows)?;
            }
            attrs.set_item("_ext_shared", &shared)?;
            shared
        }
    };
    for (key, value) in shared.iter() { vp.setattr(key.cast::<pyo3::types::PyString>()?, value)?; }
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(attach_viewport_extensions, m)?)?;
    m.add_function(wrap_pyfunction!(configure_recovery, m)?)?;
    m.add_function(wrap_pyfunction!(request_recovery, m)?)?;
    m.add_function(wrap_pyfunction!(remember_document, m)?)?;
    m.add_function(wrap_pyfunction!(link_watcher, m)?)?;
    m.add_function(wrap_pyfunction!(links_loaded, m)?)?;
    m.add_function(wrap_pyfunction!(tool_key, m)?)?;
    m.add_function(wrap_pyfunction!(toggle_eyedropper, m)?)?;
    Ok(())
}
