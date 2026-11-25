// SPDX-License-Identifier: GPL-3.0-or-later
//! Questions, messages and progress drawn by the Built chrome (egui, in the
//! Metal frame) instead of Qt windows. The submodule
//! `ingetrazo_render.chrome_dialogs`.
//!
//! Two surfaces, both drawn by `ui.rs`:
//!
//! * **The sheet**: one question at a time, hung under the top bar over the
//!   model, with a scrim that takes every click. Unsaved changes, errors,
//!   confirmations. A synchronous question pumps the window until the user answers.
//!   A watchdog cancels a sheet that never draws.
//!
//! * **Jobs**: opening, importing, saving, auto-saving, reloading links. A
//!   capsule in the top bar with the file, the phase, the progress and
//!   Cancel when the work can stop. Notices fade below it; the capsule
//!   keeps recent results and their actions.
//!
//! While a sheet is up (or an exclusive job runs) the window's guard makes
//! it modal as Qt would: keys and shortcuts answer the sheet (Enter, Esc,
//! ⌘D), clicks reach only the Metal view, the menu bar greys out. Forms
//! (`forms_host.rs`) and `ask_later` never block: they take a continuation
//! the chrome's click calls on a later turn.
//!
use crate::settings_rows::{func, none, obj, opt, s, tr, trf};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
use std::sync::atomic::{AtomicU64, Ordering};

/// Cancel a sheet still not drawn after this interval (ms).
const WATCHDOG_MS: i64 = 1500;

static IDS: AtomicU64 = AtomicU64::new(1);

fn next_id() -> String {
    IDS.fetch_add(1, Ordering::Relaxed).to_string()
}

/// This submodule, through its attributes.
fn cd(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("ingetrazo_render")?.getattr("chrome_dialogs")
}

fn dict_get<'py>(o: &Bound<'py, PyAny>, key: &str) -> PyResult<Bound<'py, PyAny>> {
    o.getattr("__dict__")?.call_method1("get", (key,))
}

fn attr_or<'py>(o: &Bound<'py, PyAny>, name: &str, fallback: Bound<'py, PyAny>) -> Bound<'py, PyAny> {
    o.getattr(name).unwrap_or(fallback)
}

fn flag(o: &Bound<'_, PyAny>, name: &str) -> bool {
    o.getattr(name).and_then(|v| v.is_truthy()).unwrap_or(false)
}

fn int0(py: Python<'_>) -> Bound<'_, PyAny> {
    0i64.into_pyobject(py).expect("int").into_any()
}

/// A line on Python's `sys.stderr` (where the app's log and tests read it).
pub fn stderr(py: Python<'_>, line: &str) -> PyResult<()> {
    py.import("sys")?.getattr("stderr")?.call_method1("write", (format!("{line}\n"),))?;
    Ok(())
}

// ---- Is the chrome there to answer? ---------------------------------------------------

/// True inside the `built` binary (its built-in `built_shell` module is
/// there), where no Qt window can show.
#[pyfunction]
fn in_shell(py: Python<'_>) -> PyResult<bool> {
    py.import("sys")?.getattr("builtin_module_names")?.contains("built_shell")
}

fn is_main_window(o: &Bound<'_, PyAny>) -> PyResult<bool> {
    Ok(o.hasattr("_tool_actions")? && o.hasattr("_documents")?)
}

/// The main window `parent` belongs to, if it is one.
#[pyfunction]
fn window_of<'py>(py: Python<'py>, parent: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let nothing = py.None().into_bound(py);
    if parent.is_none() {
        return Ok(nothing);
    }
    if is_main_window(parent)? && !parent.hasattr("isWindow")? {
        return Ok(parent.clone()); // the shell's window (no QWidget)
    }
    let win = match parent.call_method0("window") {
        Ok(w) => w,
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyAttributeError>(py) || e.is_instance_of::<pyo3::exceptions::PyRuntimeError>(py) => return Ok(nothing),
        Err(e) => return Err(e),
    };
    Ok(if is_main_window(&win)? { win } else { nothing })
}

#[pyfunction]
pub(crate) fn view_of<'py>(py: Python<'py>, win: &Bound<'py, PyAny>) -> Bound<'py, PyAny> {
    opt(win, "_doc").and_then(|doc| opt(&doc, "stand_in")).unwrap_or_else(|| py.None().into_bound(py))
}

/// Only the front window's attached, idle renderer can draw a blocking question.
#[pyfunction]
fn live<'py>(py: Python<'py>, parent: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    crate::shell_runtime::modal::live(py, parent)
}

/// The window to post an unanswered model on: in the shell, one whose
/// chrome can draw now; headless (plain CPython, tests), the parent's window.
#[pyfunction]
fn async_host<'py>(py: Python<'py>, parent: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let m = cd(py)?;
    let win = m.call_method1("window_of", (parent,))?;
    if !m.call_method0("in_shell")?.is_truthy()? || !m.call_method1("live", (parent,))?.is_none() {
        return Ok(win);
    }
    Ok(py.None().into_bound(py))
}

// ---- The sheet ------------------------------------------------------------------------

/// One question on the sheet. `buttons`: [(id, label, key hint, role)].
#[pyclass(name = "_Sheet", module = "ingetrazo_render.chrome_dialogs", get_all, set_all)]
pub struct Sheet {
    title: Py<PyAny>,
    text: Py<PyAny>,
    tone: Py<PyAny>,
    buttons: Py<PyAny>,
    default: Py<PyAny>,
    cancel: Py<PyAny>,
    discard: Py<PyAny>,
    answer: Py<PyAny>,
    event_loop: Py<PyAny>,
    shown: bool,
    blocking: bool,
    /// Non-blocking: called with the answer.
    done: Py<PyAny>,
    checklist: Py<PyAny>,
}

#[pymethods]
impl Sheet {
    #[new]
    #[allow(clippy::too_many_arguments)]
    fn new(py: Python<'_>, title: Py<PyAny>, text: Py<PyAny>, tone: Py<PyAny>, buttons: Py<PyAny>, default: Py<PyAny>, cancel: Py<PyAny>, discard: Py<PyAny>, blocking: bool) -> Self {
        Sheet { title, text, tone, buttons, default, cancel, discard, answer: py.None(), event_loop: py.None(), shown: false, blocking, done: py.None(), checklist: py.None() }
    }
}

fn sheets<'py>(win: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    win.getattr("__dict__")?.call_method1("setdefault", ("_chrome_sheets", PyList::empty(win.py())))
}

#[pyfunction]
#[pyo3(name = "_sheets")]
fn sheets_py<'py>(win: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    sheets(win)
}

#[pyfunction]
fn current_sheet<'py>(py: Python<'py>, win: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    if win.is_none() {
        return Ok(py.None().into_bound(py));
    }
    let stack = dict_get(win, "_chrome_sheets")?;
    if stack.is_truthy()? { stack.get_item(-1) } else { Ok(py.None().into_bound(py)) }
}

#[pyfunction]
#[pyo3(name = "_roles")]
fn roles<'py>(py: Python<'py>, buttons: &Bound<'py, PyAny>, default: &Bound<'py, PyAny>, cancel: &Bound<'py, PyAny>, discard: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    let out = PyList::empty(py);
    for b in buttons.try_iter()? {
        let b = b?;
        let (bid, label) = (b.get_item(0)?, b.get_item(1)?);
        let (role, hint) = if bid.eq(default)? {
            (1, "↩")
        } else if bid.eq(discard)? {
            (2, "⌘D")
        } else if bid.eq(cancel)? {
            (3, "esc")
        } else {
            (0, "")
        };
        out.append((bid, label, hint, role))?;
    }
    Ok(out)
}

fn new_sheet<'py>(py: Python<'py>, title: &Bound<'py, PyAny>, text: &Bound<'py, PyAny>, tone: &str, buttons: &Bound<'py, PyAny>, default: &Bound<'py, PyAny>, cancel: &Bound<'py, PyAny>, discard: &Bound<'py, PyAny>, blocking: bool) -> PyResult<Bound<'py, Sheet>> {
    let rows = roles(py, buttons, default, cancel, discard)?;
    Bound::new(py, Sheet::new(py, title.clone().unbind(), text.clone().unbind(), obj(py, tone)?, rows.into_any().unbind(), default.clone().unbind(), cancel.clone().unbind(), discard.clone().unbind(), blocking))
}

/// Show a sheet and wait for its answer: the id of the button chosen, or
/// `cancel`'s id on Esc or a missing frame.
///
/// `buttons`: [(id, label)] in reading order. `default` answers Enter,
/// `cancel` Esc, `discard` ⌘D and sits apart on the left (macOS's «Don't
/// Save»). `tone`: question, info, warning, error.
#[pyfunction]
#[pyo3(signature = (parent, title, text, buttons, *, default=None, cancel=None, discard=None, tone="question", checklist=None))]
#[allow(clippy::too_many_arguments)]
fn ask<'py>(py: Python<'py>, parent: &Bound<'py, PyAny>, title: &Bound<'py, PyAny>, text: &Bound<'py, PyAny>, buttons: &Bound<'py, PyAny>, default: Option<Bound<'py, PyAny>>, cancel: Option<Bound<'py, PyAny>>, discard: Option<Bound<'py, PyAny>>, tone: &str, checklist: Option<Bound<'py, PyAny>>) -> PyResult<Bound<'py, PyAny>> {
    let n = || py.None().into_bound(py);
    let (default, cancel, discard) = (default.unwrap_or_else(n), cancel.unwrap_or_else(n), discard.unwrap_or_else(n));
    let m = cd(py)?;
    let view = m.call_method1("live", (parent,))?;
    if view.is_none() {
        stderr(py, &format!("Built could not show the question: {}", title.str()?))?;
        return Ok(cancel);
    }
    let win = m.call_method1("window_of", (parent,))?;
    let sheet = new_sheet(py, title, text, tone, buttons, &default, &cancel, &discard, true)?;
    sheet.borrow_mut().checklist = checklist.map_or_else(|| py.None(), Bound::unbind);
    let event_loop = Bound::new(py, crate::shell_runtime::modal::ModalWait::new())?.into_any();
    sheet.borrow_mut().event_loop = event_loop.clone().unbind();
    push(&win, sheet.as_any())?;
    let frames = attr_or(&view, "_ui_frames", int0(py)).unbind();
    let undrawn = m.getattr("_UNDRAWN")?.unbind();
    let watchdog = {
        let (sheet, view, event_loop, undrawn) = (sheet.clone().unbind(), view.clone().unbind(), event_loop.clone().unbind(), undrawn.clone_ref(py));
        func(py, move |py, _a| {
            // Nothing drew the sheet (no frame came: the window was covered,
            // the surface lost): release the caller.
            let sheet = sheet.bind(py);
            let now = attr_or(view.bind(py), "_ui_frames", int0(py));
            if !sheet.borrow().shown && now.eq(frames.bind(py))? && sheet.borrow().answer.is_none(py) {
                sheet.borrow_mut().answer = undrawn.clone_ref(py);
                event_loop.bind(py).call_method0("quit")?;
            }
            Ok(none(py))
        })?
    };
    let dog = py.import("ingetrazo_core.timers")?.call_method1("timer", (&win, watchdog))?;
    dog.call_method1("start", (m.getattr("WATCHDOG_MS")?,))?;
    let ran = if sheet.borrow().answer.is_none(py) { event_loop.call_method0("exec").map(|_| ()) } else { Ok(()) };
    let stopped = dog.call_method0("stop").map(|_| ());
    let popped = pop(&win, sheet.as_any());
    ran?;
    stopped?;
    popped?;
    let answer = sheet.borrow().answer.clone_ref(py).into_bound(py);
    if answer.is(m.getattr("_UNDRAWN")?) {
        win.getattr("viewport")?.call_method1("flash_status", (format!("Built could not draw the question: {}", title.str()?),))?;
        return Ok(cancel);
    }
    if answer.is_none() {
        return Ok(sheet.borrow().cancel.clone_ref(py).into_bound(py)); // the loop was ended from outside (the app quit)
    }
    Ok(answer)
}

/// Post a question and deliver its answer to the continuation.
#[pyfunction]
#[pyo3(signature = (parent, title, text, buttons, done, *, default=None, cancel=None, discard=None, tone="question"))]
#[allow(clippy::too_many_arguments)]
fn ask_later(py: Python<'_>, parent: &Bound<'_, PyAny>, title: &Bound<'_, PyAny>, text: &Bound<'_, PyAny>, buttons: &Bound<'_, PyAny>, done: &Bound<'_, PyAny>, default: Option<Bound<'_, PyAny>>, cancel: Option<Bound<'_, PyAny>>, discard: Option<Bound<'_, PyAny>>, tone: &str) -> PyResult<()> {
    let n = || py.None().into_bound(py);
    let (default, cancel, discard) = (default.unwrap_or_else(n), cancel.unwrap_or_else(n), discard.unwrap_or_else(n));
    let win = cd(py)?.call_method1("async_host", (parent,))?;
    if win.is_none() {
        stderr(py, &format!("Built could not show the question: {}", title.str()?))?;
        done.call1((cancel,))?;
        return Ok(());
    }
    let sheet = new_sheet(py, title, text, tone, buttons, &default, &cancel, &discard, false)?;
    sheet.borrow_mut().done = done.clone().unbind();
    push(&win, sheet.as_any())
}

// ---- Notices and results ------------------------------------------------------------

/// A notice or a job's result, kept 45 s (30 at most).
#[pyclass(module = "ingetrazo_render.chrome_dialogs", get_all, set_all)]
pub struct Activity {
    id: String,
    title: Py<PyAny>,
    detail: Py<PyAny>,
    tone: Py<PyAny>,
    created: f64,
    action_label: Py<PyAny>,
    action: Py<PyAny>,
    toast: bool,
}

#[pymethods]
impl Activity {
    #[new]
    #[allow(clippy::too_many_arguments)]
    fn new(id: String, title: Py<PyAny>, detail: Py<PyAny>, tone: Py<PyAny>, created: f64, action_label: Py<PyAny>, action: Py<PyAny>, toast: bool) -> Self {
        Activity { id, title, detail, tone, created, action_label, action, toast }
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!("Activity(id={:?}, title={}, tone={})", self.id, self.title.bind(py).repr()?, self.tone.bind(py).repr()?))
    }
}

fn monotonic(py: Python<'_>) -> PyResult<f64> {
    py.import("time")?.call_method0("monotonic")?.extract()
}

/// The window's recent notices and results, newest first.
#[pyfunction]
fn history<'py>(py: Python<'py>, win: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let rows = win.getattr("__dict__")?.call_method1("setdefault", ("_chrome_history", PyList::empty(py)))?;
    let cutoff = monotonic(py)? - 45.0;
    let mut kept = Vec::new();
    for r in rows.try_iter()? {
        let r = r?;
        if r.getattr("created")?.extract::<f64>()? > cutoff && kept.len() < 30 {
            kept.push(r);
        }
    }
    rows.set_item(pyo3::types::PySlice::full(py), PyList::new(py, kept)?)?;
    Ok(rows)
}

#[pyfunction]
#[pyo3(name = "_record", signature = (win, title, detail, tone, action_label=None, action=None, toast=true))]
#[allow(clippy::too_many_arguments)]
fn record(py: Python<'_>, win: &Bound<'_, PyAny>, title: Py<PyAny>, detail: Py<PyAny>, tone: Py<PyAny>, action_label: Option<Py<PyAny>>, action: Option<Py<PyAny>>, toast: bool) -> PyResult<()> {
    let m = cd(py)?;
    let row = Activity::new(next_id(), title, detail, tone, monotonic(py)?, action_label.unwrap_or_else(|| s(py, "").unbind()), action.unwrap_or_else(|| py.None()), toast);
    m.call_method1("history", (win,))?.call_method1("insert", (0, Bound::new(py, row)?))?;
    m.call_method1("_repaint", (win,))?;
    Ok(())
}

fn record_with(py: Python<'_>, win: &Bound<'_, PyAny>, title: Bound<'_, PyAny>, detail: Bound<'_, PyAny>, tone: &str, action_label: Bound<'_, PyAny>, action: Bound<'_, PyAny>, toast: bool) -> PyResult<()> {
    let kw = PyDict::new(py);
    kw.set_item("toast", toast)?;
    cd(py)?.getattr("_record")?.call((win, title, detail, tone, action_label, action), Some(&kw))?;
    Ok(())
}

/// A notice under the top bar; kept even if the first frame has yet to draw.
#[pyfunction]
#[pyo3(signature = (parent, title, text, *, tone="info", button="OK", action_label=None, action=None))]
#[allow(clippy::too_many_arguments)]
fn notify(py: Python<'_>, parent: &Bound<'_, PyAny>, title: &Bound<'_, PyAny>, text: &Bound<'_, PyAny>, tone: &str, button: &str, action_label: Option<Bound<'_, PyAny>>, action: Option<Bound<'_, PyAny>>) -> PyResult<bool> {
    let _ = button;
    let win = cd(py)?.call_method1("window_of", (parent,))?;
    if win.is_none() || flag(&win, "_closed") {
        stderr(py, &format!("{}: {}", title.str()?, text.str()?))?;
        return Ok(false);
    }
    let mut text = text.clone();
    if text.str()?.to_string().contains("<br") {
        let plain = py.import("ingetrazo_render")?.call_method1("notice_text", (&text,))?;
        text = py.import("html")?.call_method1("unescape", (plain,))?;
    }
    record_with(py, &win, title.clone(), text, tone, action_label.unwrap_or_else(|| s(py, "")), action.unwrap_or_else(|| py.None().into_bound(py)), true)?;
    Ok(true)
}

/// After opening the requested file, keep the other version available.
#[pyfunction]
fn offer_recovery(py: Python<'_>, win: &Bound<'_, PyAny>, path: &Bound<'_, PyAny>, slot: &Bound<'_, PyAny>) -> PyResult<()> {
    // Reserve the snapshot before auto-save can reuse this document's slot.
    let hex: String = py.import("uuid")?.call_method0("uuid4")?.getattr("hex")?.extract()?;
    let name = format!("{}-{}{}", path.getattr("stem")?.str()?, &hex[..12], slot.getattr("suffix")?.str()?);
    let archived = py.import("ingetrazo_core.autosave")?.call_method0("discarded_dir")?.div(name)?;
    let archived = match slot.call_method1("rename", (&archived,)) {
        Ok(_) => archived,
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyOSError>(py) => slot.clone(),
        Err(e) => return Err(e),
    };
    let open = py.import("ingetrazo_render")?.getattr("open_recovery_copy")?;
    let action = py.import("functools")?.getattr("partial")?.call1((open, win, archived))?;
    record_with(py, win, s(py, &tr(py, "Auto-saved copy available")?), path.getattr("name")?, "info", s(py, &tr(py, "Open copy")?), action, true)
}

#[pyfunction]
fn save_failed(py: Python<'_>, win: &Bound<'_, PyAny>, exc: &Bound<'_, PyAny>, retry: &Bound<'_, PyAny>) -> PyResult<()> {
    let m = cd(py)?;
    let message = exc.str()?;
    if m.call_method1("live", (win,))?.is_none() {
        m.call_method1("critical", (win, tr(py, "Save failed")?, &message))?;
        return Ok(());
    }
    let kw = PyDict::new(py);
    kw.set_item("default", "retry")?;
    kw.set_item("cancel", "cancel")?;
    kw.set_item("tone", "error")?;
    let buttons = PyList::new(py, [("cancel", tr(py, "Cancel")?), ("retry", tr(py, "Save As…")?)])?;
    let answer = m.getattr("ask")?.call((win, tr(py, "Save to another location?")?, &message, buttons), Some(&kw))?;
    if answer.eq("retry")? {
        retry.call0()?;
    } else if answer.is_none() {
        m.call_method1("critical", (win, tr(py, "Save failed")?, &message))?;
    }
    Ok(())
}

/// Retain a way back to the project that finished opening.
#[pyfunction]
fn opened(py: Python<'_>, bar: &Bound<'_, PyAny>, win: &Bound<'_, PyAny>, path: &Bound<'_, PyAny>) -> PyResult<()> {
    if !bar.is_instance(&cd(py)?.getattr("ChromeBar")?)? {
        return Ok(());
    }
    let (doc, w, p) = (win.getattr("_doc")?.unbind(), win.clone().unbind(), path.clone().unbind());
    let show = func(py, move |py, _a| {
        let (doc, win) = (doc.bind(py), w.bind(py));
        if win.call_method0("documents")?.contains(doc)? {
            win.call_method1("activate_document", (doc,))?;
        } else {
            win.call_method1("open_path", (p.bind(py),))?;
        }
        Ok(none(py))
    })?;
    let kw = PyDict::new(py);
    kw.set_item("action_label", "Show")?;
    kw.set_item("action", show)?;
    bar.getattr("job")?.call_method("end", (), Some(&kw))?;
    Ok(())
}

#[pyfunction]
fn close_failed(py: Python<'_>, bar: &Bound<'_, PyAny>, error: &Bound<'_, PyAny>) -> PyResult<()> {
    if bar.is_instance(&cd(py)?.getattr("ChromeBar")?)? {
        let kw = PyDict::new(py);
        kw.set_item("failed", true)?;
        kw.set_item("detail", error.str()?)?;
        bar.getattr("job")?.call_method("end", (), Some(&kw))?;
    }
    bar.call_method0("close")?;
    Ok(())
}

#[pyfunction]
fn autosave_result(py: Python<'_>, win: &Bound<'_, PyAny>, name: &Bound<'_, PyAny>, error: &Bound<'_, PyAny>) -> PyResult<()> {
    if cd(py)?.call_method1("live", (win,))?.is_none() {
        return Ok(());
    }
    let failed = error.is_truthy()?;
    let title = if failed { format!("Auto-save failed · {}", name.str()?) } else { format!("Auto-saved · {}", name.str()?) };
    record_with(py, win, s(py, &title), error.clone(), if failed { "error" } else { "info" }, s(py, ""), py.None().into_bound(py), failed)
}

/// The "linked files missing" banner of the window's document; the count.
#[pyfunction]
fn update_link_banner(py: Python<'_>, win: &Bound<'_, PyAny>) -> PyResult<usize> {
    let links = py.import("core.links")?;
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let mut count = 0usize;
    for p in links.call_method1("sources", (&scene,))?.try_iter()? {
        if links.call_method1("link_state", (&scene, p?))?.eq("missing")? {
            count += 1;
        }
    }
    let text = match count {
        0 => String::new(),
        1 => tr(py, "1 linked file missing")?,
        n => trf(py, "{n} linked files missing", &[("n", obj(py, n)?.into_bound(py))])?,
    };
    win.getattr("_doc")?.setattr("_missing_link_banner", text)?;
    Ok(count)
}

/// The sheet's buttons, then its checklist as toggles (role 5 on, 4 off).
#[pyfunction]
fn sheet_buttons<'py>(py: Python<'py>, sheet: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    let rows = PyList::new(py, sheet.getattr("buttons")?.try_iter()?.collect::<PyResult<Vec<_>>>()?)?;
    let checklist = sheet.getattr("checklist")?;
    if checklist.is_truthy()? {
        for (i, row) in checklist.try_iter()?.enumerate() {
            let row = row?;
            let checked = row.get_item(1)?.is_truthy()?;
            rows.append((format!("check:{i}"), row.get_item(0)?, "", if checked { 5 } else { 4 }))?;
        }
    }
    Ok(rows)
}

fn push(win: &Bound<'_, PyAny>, sheet: &Bound<'_, PyAny>) -> PyResult<()> {
    sheets(win)?.call_method1("append", (sheet,))?;
    guard_engage(win)?;
    cd(win.py())?.call_method1("_repaint", (win,))?;
    Ok(())
}

fn pop(win: &Bound<'_, PyAny>, sheet: &Bound<'_, PyAny>) -> PyResult<()> {
    let stack = sheets(win)?;
    if stack.contains(sheet)? {
        stack.call_method1("remove", (sheet,))?;
    }
    guard_release(win)?;
    cd(win.py())?.call_method1("_repaint", (win,))?;
    Ok(())
}

#[pyfunction]
#[pyo3(name = "_push")]
fn push_py(win: &Bound<'_, PyAny>, sheet: &Bound<'_, PyAny>) -> PyResult<()> {
    push(win, sheet)
}

#[pyfunction]
#[pyo3(name = "_pop")]
fn pop_py(win: &Bound<'_, PyAny>, sheet: &Bound<'_, PyAny>) -> PyResult<()> {
    pop(win, sheet)
}

/// The chrome (or a key) answered the sheet on top.
#[pyfunction]
fn answer(py: Python<'_>, win: &Bound<'_, PyAny>, bid: &str) -> PyResult<bool> {
    let m = cd(py)?;
    let sheet = m.call_method1("current_sheet", (win,))?;
    if !sheet.is_none() {
        let checklist = sheet.getattr("checklist")?;
        if checklist.is_truthy()? {
            if let Some(index) = bid.strip_prefix("check:") {
                if let Ok(i) = index.parse::<usize>() {
                    if !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit()) && i < checklist.len()? {
                        let row = checklist.get_item(i)?;
                        checklist.set_item(i, (row.get_item(0)?, !row.get_item(1)?.is_truthy()?))?;
                        m.call_method1("_repaint", (win,))?;
                        return Ok(true);
                    }
                }
            }
        }
    }
    if sheet.is_none() {
        return Ok(false);
    }
    let mut known = false;
    for b in sheet.getattr("buttons")?.try_iter()? {
        if b?.get_item(0)?.eq(bid)? {
            known = true;
            break;
        }
    }
    if !known {
        return Ok(false);
    }
    sheet.setattr("answer", bid)?;
    if sheet.getattr("blocking")?.is_truthy()? {
        let event_loop = sheet.getattr("event_loop")?;
        if !event_loop.is_none() {
            event_loop.call_method0("quit")?;
        }
    } else {
        pop(win, &sheet)?;
        let done = sheet.getattr("done")?;
        if !done.is_none() {
            done.call1((bid,))?;
        }
    }
    Ok(true)
}

/// Enter = default, Esc = cancel, ⌘D = discard. True: it was the sheet's.
#[pyfunction]
fn sheet_key(py: Python<'_>, win: &Bound<'_, PyAny>, event: &Bound<'_, PyAny>) -> PyResult<bool> {
    let m = cd(py)?;
    let sheet = m.call_method1("current_sheet", (win,))?;
    if sheet.is_none() {
        return Ok(false);
    }
    let qt = py.import("ingetrazo_core")?.getattr("Input")?;
    let (key, mods) = (event.call_method0("key")?, event.call_method0("modifiers")?);
    let (default, cancel, discard) = (sheet.getattr("default")?, sheet.getattr("cancel")?, sheet.getattr("discard")?);
    if (key.eq(qt.getattr("Key_Return")?)? || key.eq(qt.getattr("Key_Enter")?)?) && default.is_truthy()? {
        m.call_method1("answer", (win, default))?;
    } else if key.eq(qt.getattr("Key_Escape")?)? && cancel.is_truthy()? {
        m.call_method1("answer", (win, cancel))?;
    } else if key.eq(qt.getattr("Key_D")?)? && mods.bitand(qt.getattr("ControlModifier")?)?.is_truthy()? && discard.is_truthy()? {
        m.call_method1("answer", (win, discard))?;
    }
    Ok(true)
}

#[pyfunction]
#[pyo3(name = "_repaint")]
fn repaint(py: Python<'_>, win: &Bound<'_, PyAny>) -> PyResult<()> {
    let view = cd(py)?.call_method1("view_of", (win,))?;
    if !view.is_none() {
        view.setattr("_ui_sheet_sig", false)?; // sent again at the next frame
        view.setattr("_ui_jobs_sig", false)?;
        view.call_method0("update")?;
    }
    Ok(())
}

// ---- Message boxes: QMessageBox's static calls, drawn on the sheet ----------------------

/// (button, label, accepts?) in the order they read, left to right.
const STANDARD: [(&str, &str, bool); 17] = [
    ("Discard", "Don't Save", false),
    ("No", "No", false),
    ("NoToAll", "No to All", false),
    ("Abort", "Abort", false),
    ("Ignore", "Ignore", false),
    ("Retry", "Retry", true),
    ("Cancel", "Cancel", false),
    ("Close", "Close", false),
    ("Reset", "Reset", false),
    ("RestoreDefaults", "Restore Defaults", false),
    ("Apply", "Apply", true),
    ("Open", "Open", true),
    ("YesToAll", "Yes to All", true),
    ("SaveAll", "Save All", true),
    ("Save", "Save", true),
    ("Yes", "Yes", true),
    ("Ok", "OK", true),
];

/// The standard buttons `critical`/`warning`/`information`/`question`
/// answer with (`QMessageBox`'s values).
fn message_box(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("ingetrazo_core")?.getattr("StandardButton")
}

fn button_key(py: Python<'_>, b: &Bound<'_, PyAny>) -> PyResult<String> {
    py.import("builtins")?.getattr("int")?.call1((b.getattr("value")?,))?.str().map(|s| s.to_string())
}

fn standard<'py>(py: Python<'py>, parent: &Bound<'py, PyAny>, tone: &str, title: &Bound<'py, PyAny>, text: &Bound<'py, PyAny>, buttons: Option<Bound<'py, PyAny>>, default: Option<Bound<'py, PyAny>>) -> PyResult<Bound<'py, PyAny>> {
    let qm = message_box(py)?;
    let buttons = match buttons.filter(|b| !b.is_none()) {
        Some(b) => b,
        None if tone == "question" => qm.getattr("Yes")?.bitor(qm.getattr("No")?)?,
        None => qm.getattr("Ok")?,
    };
    let mut chosen: Vec<(Bound<'py, PyAny>, &str, bool)> = Vec::new();
    for (name, label, accept) in STANDARD {
        let b = qm.getattr(name)?;
        if buttons.bitand(&b)?.is_truthy()? {
            chosen.push((b, label, accept));
        }
    }
    let ok = qm.getattr("Ok")?;
    if chosen.is_empty() {
        chosen.push((ok.clone(), "OK", true));
    }
    if chosen.len() == 1 && chosen[0].0.eq(&ok)? {
        let kw = PyDict::new(py);
        kw.set_item("tone", tone)?;
        cd(py)?.getattr("notify")?.call((parent, title, text), Some(&kw))?;
        return Ok(ok);
    }
    let ids = PyDict::new(py);
    for (b, _, _) in &chosen {
        ids.set_item(button_key(py, b)?, b)?;
    }
    let no_button = qm.getattr("NoButton")?;
    let default = match default.filter(|d| !d.is_none() && !d.eq(&no_button).unwrap_or(false)) {
        Some(d) => d,
        None => chosen.iter().rev().find(|c| c.2).map(|c| c.0.clone()).unwrap_or_else(|| chosen[chosen.len() - 1].0.clone()),
    };
    let present = |name: &str| -> PyResult<Option<Bound<'py, PyAny>>> {
        let b = qm.getattr(name)?;
        for (c, _, _) in &chosen {
            if c.eq(&b)? {
                return Ok(Some(b));
            }
        }
        Ok(None)
    };
    let mut escape = None;
    for name in ["Cancel", "No", "Close"] {
        if let Some(b) = present(name)? {
            escape = Some(b);
            break;
        }
    }
    if escape.is_none() && chosen.len() == 1 {
        escape = Some(chosen[0].0.clone());
    }
    let discard = present("Discard")?;
    let key = |b: Option<&Bound<'py, PyAny>>| -> PyResult<Bound<'py, PyAny>> {
        Ok(match b {
            Some(b) => s(py, &button_key(py, b)?),
            None => py.None().into_bound(py),
        })
    };
    let rows = PyList::empty(py);
    for (b, label, _) in &chosen {
        rows.append((button_key(py, b)?, tr(py, label)?))?;
    }
    let kw = PyDict::new(py);
    kw.set_item("default", key(Some(&default))?)?;
    kw.set_item("cancel", key(escape.as_ref())?)?;
    kw.set_item("discard", key(discard.as_ref())?)?;
    kw.set_item("tone", tone)?;
    let got = cd(py)?.getattr("ask")?.call((parent, title, text, rows), Some(&kw))?;
    ids.call_method1("get", (got, no_button))
}

#[pyfunction]
#[pyo3(name = "_standard")]
fn standard_py<'py>(py: Python<'py>, parent: &Bound<'py, PyAny>, tone: &str, title: &Bound<'py, PyAny>, text: &Bound<'py, PyAny>, buttons: Option<Bound<'py, PyAny>>, default: Option<Bound<'py, PyAny>>) -> PyResult<Bound<'py, PyAny>> {
    standard(py, parent, tone, title, text, buttons, default)
}

#[pyfunction]
#[pyo3(signature = (parent, title, text, buttons=None, defaultButton=None))]
#[allow(non_snake_case)]
fn critical<'py>(py: Python<'py>, parent: &Bound<'py, PyAny>, title: &Bound<'py, PyAny>, text: &Bound<'py, PyAny>, buttons: Option<Bound<'py, PyAny>>, defaultButton: Option<Bound<'py, PyAny>>) -> PyResult<Bound<'py, PyAny>> {
    standard(py, parent, "error", title, text, buttons, defaultButton)
}

#[pyfunction]
#[pyo3(signature = (parent, title, text, buttons=None, defaultButton=None))]
#[allow(non_snake_case)]
fn warning<'py>(py: Python<'py>, parent: &Bound<'py, PyAny>, title: &Bound<'py, PyAny>, text: &Bound<'py, PyAny>, buttons: Option<Bound<'py, PyAny>>, defaultButton: Option<Bound<'py, PyAny>>) -> PyResult<Bound<'py, PyAny>> {
    standard(py, parent, "warning", title, text, buttons, defaultButton)
}

#[pyfunction]
#[pyo3(signature = (parent, title, text, buttons=None, defaultButton=None))]
#[allow(non_snake_case)]
fn information<'py>(py: Python<'py>, parent: &Bound<'py, PyAny>, title: &Bound<'py, PyAny>, text: &Bound<'py, PyAny>, buttons: Option<Bound<'py, PyAny>>, defaultButton: Option<Bound<'py, PyAny>>) -> PyResult<Bound<'py, PyAny>> {
    standard(py, parent, "info", title, text, buttons, defaultButton)
}

#[pyfunction]
#[pyo3(signature = (parent, title, text, buttons=None, defaultButton=None))]
#[allow(non_snake_case)]
fn question<'py>(py: Python<'py>, parent: &Bound<'py, PyAny>, title: &Bound<'py, PyAny>, text: &Bound<'py, PyAny>, buttons: Option<Bound<'py, PyAny>>, defaultButton: Option<Bound<'py, PyAny>>) -> PyResult<Bound<'py, PyAny>> {
    standard(py, parent, "question", title, text, buttons, defaultButton)
}

// ---- Modality while a sheet is up ---------------------------------------------------

/// What a modal QMessageBox does for its window, for the chrome's sheet and
/// exclusive jobs: the shell's host asks `modal(win)` and keeps keys,
/// clicks and gestures from the rest of the window; a menu bar the window
/// has greys out.
#[pyclass(name = "_Guard", module = "ingetrazo_render.chrome_dialogs", get_all)]
pub struct Guard {
    win: Py<PyAny>,
    menus: Py<PyAny>,
    count: i64,
}

#[pymethods]
impl Guard {
    #[staticmethod]
    fn engage(win: &Bound<'_, PyAny>) -> PyResult<()> {
        guard_engage(win)
    }

    #[staticmethod]
    fn release(win: &Bound<'_, PyAny>) -> PyResult<()> {
        guard_release(win)
    }
}

fn guard_engage(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let mut guard = dict_get(win, "_chrome_guard")?;
    if guard.is_none() {
        guard = Bound::new(py, Guard { win: win.clone().unbind(), menus: PyList::empty(py).into_any().unbind(), count: 0 })?.into_any();
        win.setattr("_chrome_guard", &guard)?;
    }
    let g = guard.cast::<Guard>()?;
    g.borrow_mut().count += 1;
    if g.borrow().count != 1 {
        return Ok(());
    }
    let menus = PyList::empty(py);
    if win.hasattr("menuBar")? {
        let bar = win.call_method0("menuBar")?;
        if !bar.is_none() {
            for action in bar.call_method0("actions")?.try_iter()? {
                let action = action?;
                if action.call_method0("isEnabled")?.is_truthy()? {
                    action.call_method1("setEnabled", (false,))?;
                    menus.append(action)?;
                }
            }
        }
    }
    g.borrow_mut().menus = menus.into_any().unbind();
    Ok(())
}

fn guard_release(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let guard = dict_get(win, "_chrome_guard")?;
    if guard.is_none() {
        return Ok(());
    }
    let g = guard.cast::<Guard>()?;
    if g.borrow().count == 0 {
        return Ok(());
    }
    g.borrow_mut().count -= 1;
    if g.borrow().count != 0 {
        return Ok(());
    }
    let menus = g.borrow().menus.clone_ref(py);
    for action in menus.bind(py).try_iter()? {
        match action?.call_method1("setEnabled", (true,)) {
            Ok(_) => {}
            Err(e) if e.is_instance_of::<pyo3::exceptions::PyRuntimeError>(py) => {}
            Err(e) => return Err(e),
        }
    }
    g.borrow_mut().menus = PyList::empty(py).into_any().unbind();
    Ok(())
}

fn form_up(win: &Bound<'_, PyAny>) -> PyResult<bool> {
    dict_get(win, "_chrome_forms")?.is_truthy()
}

#[pyfunction]
#[pyo3(name = "_form_up")]
fn form_up_py(win: &Bound<'_, PyAny>) -> PyResult<bool> {
    form_up(win)
}

/// Hold the window as a sheet does (a form, `forms_host.rs`).
#[pyfunction]
fn engage(win: &Bound<'_, PyAny>) -> PyResult<()> {
    guard_engage(win)
}

#[pyfunction]
fn release(win: &Bound<'_, PyAny>) -> PyResult<()> {
    guard_release(win)
}

/// A sheet or an exclusive job holds the window.
#[pyfunction]
fn modal(win: &Bound<'_, PyAny>) -> PyResult<bool> {
    if win.is_none() {
        return Ok(false);
    }
    let guard = dict_get(win, "_chrome_guard")?;
    Ok(!guard.is_none() && guard.getattr("count")?.extract::<i64>()? > 0)
}

// ---- Jobs: open, import, save, auto-save, link reloads -----------------------------------

/// One piece of work shown in the top bar. `exclusive` work (an open, an
/// import) holds the window the way Qt's window-modal bar did: the model is
/// half there, so nothing else may touch it.
#[pyclass(module = "ingetrazo_render.chrome_dialogs", dict, weakref)]
pub struct Job {
    #[pyo3(get, set)]
    win: Py<PyAny>,
    #[pyo3(get, set)]
    id: String,
    #[pyo3(get, set)]
    label: Py<PyAny>,
    #[pyo3(get, set)]
    phase: Py<PyAny>,
    #[pyo3(get, set)]
    fraction: f64,
    #[pyo3(get, set)]
    on_cancel: Py<PyAny>,
    #[pyo3(get, set)]
    exclusive: bool,
    #[pyo3(get, set)]
    cancelled: bool,
    ended: bool,
    last_paint: f64,
}

fn perf_counter(py: Python<'_>) -> PyResult<f64> {
    py.import("time")?.call_method0("perf_counter")?.extract()
}

#[pymethods]
impl Job {
    #[new]
    #[pyo3(signature = (win, label, *, exclusive=false, on_cancel=None))]
    fn new(py: Python<'_>, win: &Bound<'_, PyAny>, label: Py<PyAny>, exclusive: bool, on_cancel: Option<Py<PyAny>>) -> PyResult<Py<Self>> {
        let job = Bound::new(py, Job {
            win: win.clone().unbind(),
            id: next_id(),
            label,
            phase: s(py, "").unbind(),
            fraction: -1.0,
            on_cancel: on_cancel.unwrap_or_else(|| py.None()),
            exclusive,
            cancelled: false,
            ended: false,
            last_paint: 0.0,
        })?;
        win.getattr("__dict__")?.call_method1("setdefault", ("_chrome_jobs", PyList::empty(py)))?.call_method1("append", (&job,))?;
        if exclusive {
            guard_engage(win)?;
        }
        job.call_method1("paint", (true,))?;
        Ok(job.unbind())
    }

    #[getter]
    fn cancellable(&self, py: Python<'_>) -> bool {
        !self.on_cancel.is_none(py) && !self.cancelled
    }

    #[pyo3(signature = (fraction=None, phase=None))]
    fn update(slf: &Bound<'_, Self>, fraction: Option<f64>, phase: Option<Py<PyAny>>) -> PyResult<()> {
        {
            let mut me = slf.borrow_mut();
            if let Some(f) = fraction {
                me.fraction = f;
            }
            if let Some(p) = phase {
                me.phase = p;
            }
        }
        slf.call_method0("paint")?;
        Ok(())
    }

    fn cancel(slf: &Bound<'_, Self>) -> PyResult<()> {
        let py = slf.py();
        if !slf.borrow().cancellable(py) {
            return Ok(());
        }
        let callback = {
            let mut me = slf.borrow_mut();
            me.cancelled = true;
            me.phase = s(py, "Cancelling…").unbind();
            std::mem::replace(&mut me.on_cancel, py.None())
        };
        callback.bind(py).call0()?;
        slf.call_method1("paint", (true,))?;
        Ok(())
    }

    /// Ask for a frame (at most 30 a second while it reports).
    #[pyo3(signature = (force=false))]
    fn paint(slf: &Bound<'_, Self>, force: bool) -> PyResult<()> {
        let py = slf.py();
        let now = perf_counter(py)?;
        if !force && now - slf.borrow().last_paint < 1.0 / 30.0 {
            return Ok(());
        }
        slf.borrow_mut().last_paint = now;
        let win = slf.borrow().win.clone_ref(py);
        cd(py)?.call_method1("_repaint", (win,))?;
        Ok(())
    }

    /// A frame now, before blocking work that pumps no events (a save).
    fn draw_now(&self, py: Python<'_>) -> PyResult<()> {
        let view = cd(py)?.call_method1("view_of", (self.win.bind(py),))?;
        if !view.is_none() {
            view.setattr("_ui_jobs_sig", false)?;
            if let Err(e) = view.call_method0("draw_frame") {
                // A missed frame only loses the chip.
                if !e.is_instance_of::<pyo3::exceptions::PyException>(py) {
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    /// Finish once and optionally keep a result action in the capsule.
    #[pyo3(signature = (*, failed=false, detail=None, action_label=None, action=None))]
    fn end(slf: &Bound<'_, Self>, failed: bool, detail: Option<Bound<'_, PyAny>>, action_label: Option<Bound<'_, PyAny>>, action: Option<Bound<'_, PyAny>>) -> PyResult<()> {
        let py = slf.py();
        if slf.borrow().ended {
            return Ok(());
        }
        slf.borrow_mut().ended = true;
        let (win, exclusive, cancelled, label) = {
            let me = slf.borrow();
            (me.win.clone_ref(py).into_bound(py), me.exclusive, me.cancelled, me.label.bind(py).str()?.to_string())
        };
        let jobs = dict_get(&win, "_chrome_jobs")?;
        if !jobs.is_none() && jobs.contains(slf)? {
            jobs.call_method1("remove", (slf,))?;
        }
        if exclusive {
            guard_release(&win)?;
        }
        let result = if cancelled { "Cancelled" } else if failed { "Failed" } else { "Done" };
        let title = format!("{result} · {}", label.trim_end_matches('…'));
        record_with(py, &win, s(py, &title), detail.unwrap_or_else(|| s(py, "")), if failed { "error" } else { "info" }, action_label.unwrap_or_else(|| s(py, "")), action.unwrap_or_else(|| py.None().into_bound(py)), false)
    }
}

/// The jobs to show: those running, then work polled from elsewhere (the
/// background auto-save).
#[pyfunction]
fn jobs<'py>(py: Python<'py>, win: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    let out = PyList::empty(py);
    let running = dict_get(win, "_chrome_jobs")?;
    if !running.is_none() {
        for j in running.try_iter()? {
            out.append(j?)?;
        }
    }
    let auto = dict_get(win, "_background_autosave")?;
    if !auto.is_none() && flag(&auto, "busy") {
        out.append(Bound::new(py, AutosaveJob::of(py, &auto)?)?)?;
    }
    Ok(out)
}

fn running_jobs<'py>(win: &Bound<'py, PyAny>) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let running = dict_get(win, "_chrome_jobs")?;
    if running.is_none() {
        return Ok(Vec::new());
    }
    running.try_iter()?.collect()
}

#[pyfunction]
fn exclusive(win: &Bound<'_, PyAny>) -> PyResult<bool> {
    for j in running_jobs(win)? {
        if j.getattr("exclusive")?.is_truthy()? {
            return Ok(true);
        }
    }
    Ok(false)
}

#[pyfunction]
fn find_job<'py>(py: Python<'py>, win: &Bound<'py, PyAny>, jid: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    for j in running_jobs(win)? {
        if j.getattr("id")?.eq(jid)? {
            return Ok(j);
        }
    }
    Ok(py.None().into_bound(py))
}

/// Esc cancels the exclusive job, when it can stop.
#[pyfunction]
fn job_key(py: Python<'_>, win: &Bound<'_, PyAny>, event: &Bound<'_, PyAny>) -> PyResult<bool> {
    let escape = py.import("ingetrazo_core")?.getattr("Input")?.getattr("Key_Escape")?;
    if event.call_method0("key")?.eq(escape)? {
        for j in running_jobs(win)?.into_iter().rev() {
            if j.getattr("exclusive")?.is_truthy()? && j.getattr("cancellable")?.is_truthy()? {
                j.call_method0("cancel")?;
                break;
            }
        }
    }
    Ok(true)
}

/// The background auto-save as a quiet job (polled, never cancelled here).
#[pyclass(name = "_AutosaveJob", module = "ingetrazo_render.chrome_dialogs", get_all)]
pub struct AutosaveJob {
    id: String,
    label: String,
    exclusive: bool,
    cancellable: bool,
    fraction: f64,
    phase: String,
}

impl AutosaveJob {
    fn of(py: Python<'_>, auto: &Bound<'_, PyAny>) -> PyResult<Self> {
        let job = opt(auto, "job").filter(|j| j.is_truthy().unwrap_or(false));
        let doc = match &job {
            Some(j) => j.call_method1("get", ("doc",))?,
            None => py.None().into_bound(py),
        };
        let name = if !doc.is_none() && doc.hasattr("display_name")? { doc.call_method0("display_name")?.str()?.to_string() } else { String::new() };
        let label = if name.is_empty() { tr(py, "Auto-saving")? } else { trf(py, "Auto-saving {name}", &[("name", s(py, &name))])?.trim().to_string() };
        Ok(AutosaveJob { id: "autosave".into(), label, exclusive: false, cancellable: false, fraction: -1.0, phase: String::new() })
    }
}

#[pymethods]
impl AutosaveJob {
    #[new]
    fn new(py: Python<'_>, auto: &Bound<'_, PyAny>) -> PyResult<Self> {
        AutosaveJob::of(py, auto)
    }
}

/// `with busy(win, "Saving x…"):` around blocking work that reports no
/// progress: the chip is drawn before it starts and goes when it ends. A
/// no-op when the chrome is not on screen.
#[pyclass(name = "busy", module = "ingetrazo_render.chrome_dialogs", get_all, set_all)]
pub struct Busy {
    parent: Py<PyAny>,
    label: Py<PyAny>,
    exclusive: bool,
    output_path: Py<PyAny>,
    job: Py<PyAny>,
}

#[pymethods]
impl Busy {
    #[new]
    #[pyo3(signature = (parent, label, *, output_path=None, exclusive=false))]
    fn new(py: Python<'_>, parent: Py<PyAny>, label: Py<PyAny>, output_path: Option<Py<PyAny>>, exclusive: bool) -> Self {
        Busy { parent, label, exclusive, output_path: output_path.unwrap_or_else(|| py.None()), job: py.None() }
    }

    fn __enter__<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        let py = slf.py();
        let m = cd(py)?;
        let (parent, label, exclusive) = {
            let me = slf.borrow();
            (me.parent.clone_ref(py).into_bound(py), me.label.clone_ref(py), me.exclusive)
        };
        if !m.call_method1("live", (&parent,))?.is_none() {
            let kw = PyDict::new(py);
            kw.set_item("exclusive", exclusive)?;
            let job = m.getattr("Job")?.call((m.call_method1("window_of", (&parent,))?, label), Some(&kw))?;
            slf.borrow_mut().job = job.clone().unbind();
            job.call_method0("draw_now")?;
        }
        Ok(slf.borrow().job.clone_ref(py).into_bound(py))
    }

    #[pyo3(signature = (*exc))]
    fn __exit__(&self, py: Python<'_>, exc: &Bound<'_, PyTuple>) -> PyResult<bool> {
        let job = self.job.bind(py);
        if job.is_none() {
            return Ok(false);
        }
        let failed = exc.len() > 0 && !exc.get_item(0)?.is_none();
        let error = if exc.len() > 1 { exc.get_item(1)? } else { py.None().into_bound(py) };
        let kw = PyDict::new(py);
        kw.set_item("failed", failed)?;
        kw.set_item("detail", if error.is_truthy()? { error.str()?.to_string() } else { String::new() })?;
        let path = self.output_path.bind(py);
        if path.is_truthy()? && !failed {
            let folder = py.import("pathlib")?.getattr("Path")?.call1((path,))?.call_method0("resolve")?.getattr("parent")?.unbind();
            let action = func(py, move |py, _a| {
                py.import("ingetrazo_core.desktop_open")?.call_method1("open_url", (folder.bind(py),))?;
                Ok(none(py))
            })?;
            kw.set_item("action_label", "Open folder")?;
            kw.set_item("action", action)?;
        }
        job.call_method("end", (), Some(&kw))?;
        Ok(false)
    }
}

/// The part of `QProgressDialog` the loaders use, drawn as a job in the top
/// bar instead of a window-modal dialog.
#[pyclass(module = "ingetrazo_render.chrome_dialogs", dict, weakref)]
pub struct ChromeBar {
    value: i64,
    visible: bool,
    #[pyo3(get)]
    job: Py<PyAny>,
    /// Emitted on Cancel: `canceled.connect(callback)`.
    #[pyo3(get)]
    canceled: Py<PyAny>,
}

impl ChromeBar {
    fn job<'py>(&self, py: Python<'py>) -> Bound<'py, PyAny> {
        self.job.bind(py).clone()
    }
}

#[pymethods]
impl ChromeBar {
    #[new]
    #[pyo3(signature = (win, title, *, exclusive=true))]
    fn new(py: Python<'_>, win: &Bound<'_, PyAny>, title: &Bound<'_, PyAny>, exclusive: bool) -> PyResult<Self> {
        let kw = PyDict::new(py);
        kw.set_item("exclusive", exclusive)?;
        let job = cd(py)?.getattr("Job")?.call((win, title), Some(&kw))?;
        job.call_method1("update", (0.0, ""))?;
        let canceled = py.import("ingetrazo_render")?.getattr("ChangeCallbacks")?.call0()?;
        Ok(ChromeBar { value: 0, visible: true, job: job.unbind(), canceled: canceled.unbind() })
    }

    #[allow(non_snake_case)]
    fn setValue(&mut self, py: Python<'_>, value: i64) -> PyResult<()> {
        self.value = value;
        let job = self.job(py);
        if job.getattr("fraction")?.extract::<f64>()? >= 0.0 {
            let kw = PyDict::new(py);
            kw.set_item("fraction", (value as f64 / 100.0).clamp(0.0, 1.0))?;
            job.call_method("update", (), Some(&kw))?;
        }
        Ok(())
    }

    fn value(&self) -> i64 {
        self.value
    }

    #[allow(non_snake_case)]
    fn setLabelText(&self, py: Python<'_>, text: &Bound<'_, PyAny>) -> PyResult<()> {
        let kw = PyDict::new(py);
        kw.set_item("phase", text)?;
        self.job(py).call_method("update", (), Some(&kw))?;
        Ok(())
    }

    #[allow(non_snake_case)]
    fn setRange(&self, py: Python<'_>, low: i64, high: i64) -> PyResult<()> {
        let job = self.job(py);
        job.setattr("fraction", if high <= low { -1.0 } else { 0.0 })?;
        job.call_method1("paint", (true,))?;
        Ok(())
    }

    #[allow(non_snake_case)]
    fn setCancelButtonText(&self, py: Python<'_>, _text: &Bound<'_, PyAny>) -> PyResult<()> {
        self.set_cancellable(py, true)
    }

    fn set_cancellable(&self, py: Python<'_>, on: bool) -> PyResult<()> {
        let job = self.job(py);
        job.setattr("on_cancel", if on { self.canceled.bind(py).getattr("emit")? } else { py.None().into_bound(py) })?;
        job.call_method1("paint", (true,))?;
        Ok(())
    }

    #[allow(non_snake_case)]
    fn wasCanceled(&self, py: Python<'_>) -> PyResult<bool> {
        self.job(py).getattr("cancelled")?.is_truthy()
    }

    #[allow(non_snake_case)]
    fn isVisible(&self) -> bool {
        self.visible
    }

    fn reset(&self) {}

    fn hide(slf: &Bound<'_, Self>) -> PyResult<()> {
        slf.call_method0("close")?;
        Ok(())
    }

    fn close(&mut self, py: Python<'_>) -> PyResult<bool> {
        self.visible = false;
        self.job(py).call_method0("end")?;
        Ok(true)
    }
}

// ---- The module -------------------------------------------------------------------------

/// Resolve the plugin-facing message box button constants on demand.
#[pyfunction]
#[pyo3(name = "__getattr__")]
fn module_getattr<'py>(py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyAny>> {
    if name == "QMessageBox" {
        return message_box(py);
    }
    Err(pyo3::exceptions::PyAttributeError::new_err(format!("module 'ingetrazo_render.chrome_dialogs' has no attribute '{name}'")))
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    let sub = PyModule::new(py, "chrome_dialogs")?;
    sub.add("WATCHDOG_MS", WATCHDOG_MS)?;
    sub.add("_UNDRAWN", py.import("builtins")?.getattr("object")?.call0()?)?;
    sub.add("time", py.import("time")?)?;
    sub.add_class::<Sheet>()?;
    sub.add_class::<Activity>()?;
    sub.add_class::<Guard>()?;
    sub.add_class::<Job>()?;
    sub.add_class::<AutosaveJob>()?;
    sub.add_class::<ChromeBar>()?;
    sub.add_class::<Busy>()?;
    for f in [
        wrap_pyfunction!(in_shell, &sub)?,
        wrap_pyfunction!(window_of, &sub)?,
        wrap_pyfunction!(view_of, &sub)?,
        wrap_pyfunction!(live, &sub)?,
        wrap_pyfunction!(async_host, &sub)?,
        wrap_pyfunction!(sheets_py, &sub)?,
        wrap_pyfunction!(current_sheet, &sub)?,
        wrap_pyfunction!(roles, &sub)?,
        wrap_pyfunction!(ask, &sub)?,
        wrap_pyfunction!(ask_later, &sub)?,
        wrap_pyfunction!(history, &sub)?,
        wrap_pyfunction!(record, &sub)?,
        wrap_pyfunction!(notify, &sub)?,
        wrap_pyfunction!(offer_recovery, &sub)?,
        wrap_pyfunction!(save_failed, &sub)?,
        wrap_pyfunction!(opened, &sub)?,
        wrap_pyfunction!(close_failed, &sub)?,
        wrap_pyfunction!(autosave_result, &sub)?,
        wrap_pyfunction!(update_link_banner, &sub)?,
        wrap_pyfunction!(sheet_buttons, &sub)?,
        wrap_pyfunction!(push_py, &sub)?,
        wrap_pyfunction!(pop_py, &sub)?,
        wrap_pyfunction!(answer, &sub)?,
        wrap_pyfunction!(sheet_key, &sub)?,
        wrap_pyfunction!(repaint, &sub)?,
        wrap_pyfunction!(standard_py, &sub)?,
        wrap_pyfunction!(critical, &sub)?,
        wrap_pyfunction!(warning, &sub)?,
        wrap_pyfunction!(information, &sub)?,
        wrap_pyfunction!(question, &sub)?,
        wrap_pyfunction!(form_up_py, &sub)?,
        wrap_pyfunction!(engage, &sub)?,
        wrap_pyfunction!(release, &sub)?,
        wrap_pyfunction!(modal, &sub)?,
        wrap_pyfunction!(jobs, &sub)?,
        wrap_pyfunction!(exclusive, &sub)?,
        wrap_pyfunction!(find_job, &sub)?,
        wrap_pyfunction!(job_key, &sub)?,
        wrap_pyfunction!(module_getattr, &sub)?,
    ] {
        sub.add_function(f)?;
    }
    m.add_submodule(&sub)?;
    py.import("sys")?.getattr("modules")?.set_item("ingetrazo_render.chrome_dialogs", &sub)?;
    Ok(())
}
