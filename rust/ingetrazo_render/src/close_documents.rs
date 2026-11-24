//! Closing a tab or quitting keeps unsaved documents until the requested saves succeed.
use super::model_edit::translated;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};

fn ask<'py>(
    win: &Bound<'py, PyAny>,
    title: &Bound<'py, PyAny>,
    text: &Bound<'py, PyAny>,
    checklist: Option<&Bound<'py, PyList>>,
) -> PyResult<Bound<'py, PyAny>> {
    let py = win.py();
    let buttons = PyList::empty(py);
    for (id, caption) in [
        ("discard", "Don't Save"),
        ("cancel", "Cancel"),
        (
            "save",
            if checklist.is_some() {
                "Save Selected"
            } else {
                "Save"
            },
        ),
    ] {
        buttons.append((id, translated(py, caption, None)?))?;
    }
    let kw = PyDict::new(py);
    for role in ["default", "cancel", "discard"] {
        kw.set_item(role, if role == "default" { "save" } else { role })?;
    }
    if let Some(list) = checklist {
        kw.set_item("checklist", list)?;
    }
    py.import("ingetrazo_render.chrome_dialogs")?
        .getattr("ask")?
        .call((win, title, text, buttons), Some(&kw))
}

#[pyfunction]
fn confirm_discard(win: &Bound<'_, PyAny>, prompt: &Bound<'_, PyAny>) -> PyResult<bool> {
    if !win.call_method0("_is_dirty")?.is_truthy()? {
        return Ok(true);
    }
    let doc = win.getattr("_doc")?;
    let answer = ask(
        win,
        prompt,
        &translated(win.py(), "Unsaved changes will be lost.", None)?,
        None,
    )?;
    if !win.getattr("_doc")?.is(&doc) {
        return Ok(false);
    }
    if answer.eq("save")? {
        win.call_method0("_on_save")?;
        return Ok(!win.call_method1("_doc_is_dirty", (doc,))?.is_truthy()?);
    }
    answer.eq("discard")
}
#[pyfunction]
fn confirm_close_tab(win: &Bound<'_, PyAny>, doc: &Bound<'_, PyAny>, ask: bool) -> PyResult<bool> {
    if !win.getattr("_workspace")?.is_none()
        && win.getattr("_doc")?.is(doc)
        && !win.call_method0("leave_workspace")?.is_truthy()?
    {
        return Ok(false);
    }
    if !ask || !win.call_method1("_doc_is_dirty", (doc,))?.is_truthy()? {
        return Ok(true);
    }
    if !win.getattr("_doc")?.is(doc) {
        win.call_method1("activate_document", (doc,))?;
        if !win.getattr("_doc")?.is(doc) {
            return Ok(false);
        }
    }
    win.call_method0("_end_group_edits")?;
    let vars = PyDict::new(win.py());
    vars.set_item("name", doc.call_method0("display_name")?)?;
    let prompt = translated(win.py(), "Close “{name}”?", Some(&vars))?;
    win.call_method1("_confirm_discard", (prompt,))?.is_truthy()
}

#[pyfunction]
fn save_selected(
    win: &Bound<'_, PyAny>,
    docs: &Bound<'_, PyAny>,
    answer: &Bound<'_, PyAny>,
) -> PyResult<bool> {
    if answer.eq("discard")? {
        return Ok(true);
    }
    if !answer.eq("save")? {
        return Ok(false);
    }
    for doc in docs.try_iter()? {
        let doc = doc?;
        win.call_method1("activate_document", (&doc,))?;
        win.call_method0("_on_save")?;
        if win.call_method1("_doc_is_dirty", (doc,))?.is_truthy()? {
            return Ok(false);
        }
    }
    Ok(true)
}
#[pyfunction]
fn confirm_quit(win: &Bound<'_, PyAny>) -> PyResult<bool> {
    let py = win.py();
    let mut dirty = Vec::new();
    for doc in win.call_method0("documents")?.try_iter()? {
        let doc = doc?;
        if win.call_method1("_doc_is_dirty", (&doc,))?.is_truthy()? {
            dirty.push(doc);
        }
    }
    if dirty.is_empty() {
        return Ok(true);
    }
    if let [doc] = dirty.as_slice() {
        if !win.getattr("_doc")?.is(doc) {
            win.call_method1("activate_document", (doc,))?;
        }
        return win
            .call_method1("_confirm_discard", (translated(py, "Quit Built?", None)?,))?
            .is_truthy();
    }
    let checklist = PyList::empty(py);
    for doc in &dirty {
        checklist.append((doc.call_method0("display_name")?, true))?;
    }
    let vars = PyDict::new(py);
    vars.set_item("n", dirty.len())?;
    let answer = ask(
        win,
        &translated(py, "Which projects should be saved?", None)?,
        &translated(
            py,
            "{n} projects have unsaved changes. Unchecked changes will be discarded.",
            Some(&vars),
        )?,
        Some(&checklist),
    )?;
    let selected = PyList::empty(py);
    for (doc, row) in dirty.iter().zip(checklist.iter()) {
        if row.get_item(1)?.is_truthy()? {
            selected.append(doc)?;
        }
    }
    win.call_method1("_save_all_or_stay", (selected, answer))?
        .is_truthy()
}
/// Confirm each window before Quit retires any recovery slots. A later
/// window can cancel, leaving every earlier document open and recoverable.
#[pyfunction]
fn confirm_window_close(win: &Bound<'_, PyAny>) -> PyResult<bool> {
    let dialogs = win.py().import("ingetrazo_render.chrome_dialogs")?;
    if !dialogs.call_method1("current_sheet", (win,))?.is_none() {
        return Ok(false);
    }
    if win.getattr("_open_in_progress")?.is_truthy()? {
        win.setattr("_open_aborted", true)?;
        return Ok(false);
    }
    if dialogs.call_method1("modal", (win,))?.is_truthy()?
        || (!win.getattr("_workspace")?.is_none() && !win.call_method0("leave_workspace")?.is_truthy()?)
        || !win.call_method0("_confirm_quit")?.is_truthy()?
    {
        return Ok(false);
    }
    win.py().import("app_shell.project")?.call_method1("persist", (win,))?;
    Ok(true)
}

#[pyfunction]
fn finish_window_close(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let auto = win.getattr("__dict__")?.call_method1("get", ("_background_autosave",))?;
    if !auto.is_none() {
        auto.call_method0("cancel")?;
    }
    let clear = py.import("core.autosave")?.getattr("clear")?;
    for doc in win.getattr("_documents")?.try_iter()? {
        let doc = doc?;
        let args = PyDict::new(py);
        args.set_item("untitled", doc.getattr("untitled_no")?)?;
        // A stale recovery slot is preferable to blocking an accepted close.
        let _ = clear.call((doc.getattr("path")?,), Some(&args));
    }
    win.setattr("_closed", true)
}

#[pyfunction]
fn request_close(win: &Bound<'_, PyAny>) -> PyResult<bool> {
    if !confirm_window_close(win)? {
        return Ok(false);
    }
    finish_window_close(win)?;
    Ok(true)
}

/// The target can be refused by a workspace. Resolve that before any command
/// that writes to or closes the foreground document.
#[pyfunction]
fn project_tab_menu(win: &Bound<'_, PyAny>, verb: &str, index: usize) -> PyResult<()> {
    let docs: Vec<Bound<'_, PyAny>> = win.getattr("_project_tabs")?.call_method0("documents")?.extract()?;
    let Some(doc) = docs.get(index) else { return Ok(()); };
    if !win.getattr("_doc")?.is(doc) {
        win.call_method1("activate_document", (doc,))?;
        if !win.getattr("_doc")?.is(doc) { return Ok(()); }
    }
    match verb {
        "new" => { win.getattr("_project_tabs")?.getattr("newRequested")?.call_method0("emit")?; }
        "open" => { win.call_method0("_on_open")?; }
        "save" => { win.call_method0("_on_save")?; }
        "save_as" => { win.call_method0("_on_save_as")?; }
        "close" => { win.call_method1("close_document", (doc,))?; }
        "close_left" | "close_right" | "close_all" => {
            let targets = match verb {
                "close_left" => PyList::new(win.py(), &docs[..index])?,
                "close_right" => PyList::new(win.py(), docs[index + 1..].iter().rev())?,
                _ => PyList::new(win.py(), &docs)?,
            };
            win.call_method1("close_documents", (targets,))?;
        }
        "move" => { win.py().import("app_shell.windows")?.call_method1("move_to_new_window", (win, doc))?; }
        "pictures" => {
            let key = "tabs/hover_picture";
            let previous = super::navigation::setting(win.py(), key, "1")?;
            let settings = win.py().import("core.settings")?.call_method0("settings")?;
            settings.call_method1("setValue", (key, if previous == "0" { "1" } else { "0" }))?;
        }
        _ => {}
    }
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(project_tab_menu, m)?)?;
    m.add_function(wrap_pyfunction!(request_close, m)?)?;
    m.add_function(wrap_pyfunction!(confirm_window_close, m)?)?;
    m.add_function(wrap_pyfunction!(finish_window_close, m)?)?;
    m.add_function(wrap_pyfunction!(confirm_close_tab, m)?)?;
    m.add_function(wrap_pyfunction!(confirm_quit, m)?)?;
    m.add_function(wrap_pyfunction!(confirm_discard, m)?)?;
    m.add_function(wrap_pyfunction!(save_selected, m)?)?;
    Ok(())
}
