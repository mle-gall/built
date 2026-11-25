//! Recovery slots belong to one document. Only an explicit decline retires a
//! slot. An unavailable question leaves it available for the next launch.
use super::model_edit::translated;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use std::path::PathBuf;

#[pyfunction]
fn offer_untitled_recovery(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let slots = win
        .py()
        .import("ingetrazo_core.autosave")?
        .call_method0("pending_untitled")?;
    for entry in slots.try_iter()? {
        let (n, slot): (usize, Bound<'_, PyAny>) = entry?.extract()?;
        let mut in_use = false;
        for doc in win.getattr("_documents")?.try_iter()? {
            let doc = doc?;
            if doc.getattr("untitled_no")?.eq(n)?
                && !win.call_method1("_doc_is_blank", (doc,))?.is_truthy()?
            {
                in_use = true;
                break;
            }
        }
        if !in_use {
            recover_untitled(win, n, &slot)?;
        }
    }
    Ok(())
}
fn recovery_answer<'py>(
    win: &Bound<'py, PyAny>,
    text: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let py = win.py();
    let buttons = PyList::empty(py);
    for (id, caption) in [("later", "Cancel"), ("no", "No"), ("yes", "Yes")] {
        buttons.append((id, translated(py, caption, None)?))?;
    }
    let kw = PyDict::new(py);
    kw.set_item("default", "yes")?;
    kw.set_item("cancel", "later")?;
    py.import("ingetrazo_render.chrome_dialogs")?.getattr("ask")?.call(
        (
            win,
            translated(py, "Recover the auto-saved drawing?", None)?,
            text,
            buttons,
        ),
        Some(&kw),
    )
}
#[pyfunction]
fn named_recovery(win: &Bound<'_, PyAny>, path: &Bound<'_, PyAny>) -> PyResult<Option<bool>> {
    let py = win.py();
    let args = PyDict::new(py);
    args.set_item("name", path.getattr("name")?)?;
    let text=translated(py,"A session with '{name}' ended without saving (auto-saved copy found). Recover it instead of the file on disk?",Some(&args))?;
    let answer = recovery_answer(win, &text)?;
    if answer.eq("yes")? {
        return Ok(Some(true));
    }
    if answer.eq("no")? {
        py.import("ingetrazo_core.autosave")?.call_method1("clear", (path,))?;
        return Ok(Some(false));
    }
    Ok(None)
}
#[pyfunction]
fn recover_untitled(win: &Bound<'_, PyAny>, n: usize, slot: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let dialog = py.import("ingetrazo_render.chrome_dialogs")?;
    let text = translated(py,"The last session ended without saving an untitled drawing (auto-saved copy found). Recover it?", None)?;
    let answer = recovery_answer(win, &text)?;
    if answer.eq("no")? {
        let kw = PyDict::new(py);
        kw.set_item("untitled", n)?;
        py.import("ingetrazo_core.autosave")?
            .getattr("clear")?
            .call((py.None(),), Some(&kw))?;
        return Ok(());
    }
    if !answer.eq("yes")? {
        return Ok(());
    }
    let staged = py.import("core.scene")?.call_method0("Scene")?;
    if let Err(err) = py
        .import("formats.igz")?
        .call_method1("load_into", (&staged, slot))
    {
        dialog.call_method1(
            "critical",
            (
                win,
                translated(py, "Open failed", None)?,
                err.value(py).str()?,
            ),
        )?;
        return Ok(());
    }
    if !win.call_method0("_doc_is_blank")?.is_truthy()? {
        win.call_method0("new_document")?;
    }
    let target = win.getattr("_doc")?;
    for doc in win.getattr("_documents")?.try_iter()? {
        let doc = doc?;
        if !doc.is(&target) && doc.getattr("untitled_no")?.eq(n)? {
            doc.setattr("untitled_no", win.call_method0("_free_untitled_no")?)?;
        }
    }
    target.setattr("untitled_no", n)?;
    let vp = win.getattr("viewport")?;
    let history = vp.getattr("history")?;
    history.setattr("scene", &staged)?;
    vp.call_method1("set_document", (&staged, &history))?;
    vp.getattr("history")?.call_method0("clear")?;
    vp.call_method0("reset_texture_cache")?;
    win.setattr("_current_path", py.None())?;
    win.setattr("_import_name", py.None())?;
    win.setattr("_saved_version", -1)?;
    vp.call_method0("notify_scene_changed")?;
    win.call_method0("_update_title")?;
    Ok(())
}
#[pyfunction]
fn recover_discarded(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let archive = py.import("formats.built")?;
    let folder: PathBuf = py
        .import("ingetrazo_core.autosave")?
        .call_method0("discarded_dir")?
        .extract()?;
    let suffixes: Vec<String> = archive.getattr("DOCUMENT_SUFFIXES")?.extract()?;
    let mut found = false;
    for entry in std::fs::read_dir(&folder)? {
        let path = entry?.path();
        if path.is_file()
            && path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                suffixes
                    .iter()
                    .any(|s| s.trim_start_matches('.').eq_ignore_ascii_case(e))
            })
        {
            found = true;
            break;
        }
    }
    let title = translated(py, "Recover a discarded auto-save…", None)?;
    if !found {
        py.import("ingetrazo_render.chrome_dialogs")?.call_method1(
            "information",
            (
                win,
                title,
                translated(py, "No discarded auto-saved copies yet.", None)?,
            ),
        )?;
        return Ok(());
    }
    let chosen: String = py
        .import("ingetrazo_render.shell_files")?
        .getattr("ShellFileDialog")?
        .call_method1(
            "getOpenFileName",
            (
                win,
                title,
                folder.to_string_lossy(),
                archive.getattr("DOCUMENT_FILTER")?,
            ),
        )?
        .get_item(0)?
        .extract()?;
    if chosen.is_empty() {
        return Ok(());
    }
    let path = py.import("pathlib")?.call_method1("Path", (chosen,))?;
    open_recovery_copy(win, &path)
}
#[pyfunction]
fn open_recovery_copy(win: &Bound<'_, PyAny>, path: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    if win.call_method1("open_path", (path,))?.is_truthy()? {
        let done = py.import("functools")?.call_method1(
            "partial",
            (wrap_pyfunction!(discarded_opened, py)?, win, path),
        )?;
        py.import("ingetrazo_render.document_open")?
            .call_method1("after_open", (win, path, done))?;
    }
    Ok(())
}
#[pyfunction]
fn discarded_opened(
    win: &Bound<'_, PyAny>,
    path: &Bound<'_, PyAny>,
    doc: &Bound<'_, PyAny>,
) -> PyResult<()> {
    if !doc.call_method1("same_file", (path,))?.is_truthy()? {
        return Ok(());
    }
    doc.setattr("path", win.py().None())?;
    doc.setattr("saved_content", -1)?;
    win.call_method0("_update_title")?;
    win.call_method0("statusBar")?.call_method1(
        "showMessage",
        (
            translated(
                win.py(),
                "Recovered copy loaded — use Save As to keep it.",
                None,
            )?,
            6000,
        ),
    )?;
    Ok(())
}
#[pyfunction]
fn reset_cancelled_open(win: &Bound<'_, PyAny>) -> PyResult<()> {
    win.setattr("_open_cancelled", false)?;
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    scene.call_method0("clear")?;
    scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)?;
    vp.getattr("history")?.call_method0("clear")?;
    vp.call_method0("reset_texture_cache")?;
    win.setattr("_saved_version", scene.getattr("version")?)?;
    vp.call_method0("notify_scene_changed")?;
    win.call_method0("statusBar")?.call_method1(
        "showMessage",
        (translated(win.py(), "Opening cancelled.", None)?, 4000),
    )?;
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(offer_untitled_recovery, m)?)?;
    m.add_function(wrap_pyfunction!(recover_untitled, m)?)?;
    m.add_function(wrap_pyfunction!(recover_discarded, m)?)?;
    m.add_function(wrap_pyfunction!(named_recovery, m)?)?;
    m.add_function(wrap_pyfunction!(open_recovery_copy, m)?)?;
    m.add_function(wrap_pyfunction!(reset_cancelled_open, m)?)?;
    Ok(())
}
