// SPDX-License-Identifier: GPL-3.0-or-later
//! The start page on the window: the submodule `ingetrazo_render.wgpu_start`.
//!
//! The Start sheet opens on a cold start without a file and from File ▸
//! Home. `start_view.rs` decides its rows, their words, the preview and the
//! templates from what this module gathers: the recents, what each
//! document noted while open, and what a worker thread found in each
//! archive (whether it exists, its `thumbnail.png`, its links, a newer
//! auto-save, a project's files). Nothing here blocks the first frame: the
//! page shows what it has and fills in as the worker reports. Opening goes
//! through the window's `open_path`, so the progress capsule shows over the
//! page while it loads; the page goes when the document is in.
use crate::settings_rows::{func, none, obj, opt, s};
use pyo3::{
    prelude::*,
    types::{IntoPyDict, PyDict, PyList, PyTuple},
};

pub const SEP: &str = "\x1f";
/// Asked for and not drawn this long after: report the failed frame.
const WATCHDOG_MS: u32 = 2000;
/// Thumbnails wider than this are scaled down before upload (points × 2).
const THUMB_PX: u32 = 512;

/// This submodule, through its attributes (tests replace `_load_one`).
fn start(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("ingetrazo_render")?.getattr("wgpu_start")
}

fn cs(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("ingetrazo_render")?.getattr("chrome_sheet")
}

fn recent(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("core.recent")
}

fn store(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("core.settings")?.call_method0("settings")
}

/// The view in front of the window (the document's stand-in).
#[pyfunction]
#[pyo3(name = "_view")]
fn view_in_front<'py>(win: &Bound<'py, PyAny>) -> Option<Bound<'py, PyAny>> {
    opt(win, "_doc").and_then(|d| opt(&d, "stand_in"))
}

/// Through the module (tests replace `_view`).
fn view_of<'py>(win: &Bound<'py, PyAny>) -> Option<Bound<'py, PyAny>> {
    match start(win.py()).and_then(|m| m.getattr("_view")).and_then(|f| f.call1((win,))) {
        Ok(v) => Some(v).filter(|v| !v.is_none()),
        Err(_) => view_in_front(win),
    }
}

/// Give a view the keys; a view already gone needs none.
fn focus_view(py: Python<'_>, view: &Bound<'_, PyAny>) {
    if let Ok(reason) = py.import("ingetrazo_core").and_then(|m| m.getattr("Input")?.getattr("OtherFocusReason")) {
        let _ = view.call_method1("setFocus", (reason,));
    }
}

/// A critical section on the worker's cache (`_LOCK`).
fn locked<T>(py: Python<'_>, f: impl FnOnce() -> PyResult<T>) -> PyResult<T> {
    let lock = start(py)?.getattr("_LOCK")?;
    lock.call_method0("acquire")?;
    let out = f();
    lock.call_method0("release")?;
    out
}

fn cache(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
    Ok(start(py)?.getattr("_CACHE")?.cast_into::<PyDict>()?)
}

// ---- templates ---------------------------------------------------------------------------

/// `(id, name, description, structure nodes or None, active node, units or
/// None)`, from `start_view.rs`.
fn templates<'py>(m: &Bound<'py, PyModule>) -> PyResult<Bound<'py, PyList>> {
    let py = m.py();
    let json = py.import("json")?;
    let out = PyList::empty(py);
    for t in m.getattr("start_templates")?.call0()?.try_iter()? {
        let t = t?;
        let load = |v: Bound<'_, PyAny>| -> PyResult<Py<PyAny>> { Ok(if v.is_truthy()? { json.call_method1("loads", (v,))?.unbind() } else { py.None() }) };
        let active = t.get_item(4)?;
        out.append((t.get_item(0)?, t.get_item(1)?, t.get_item(2)?, load(t.get_item(3)?)?, if active.is_truthy()? { active.unbind() } else { py.None() }, load(t.get_item(5)?)?))?;
    }
    Ok(out)
}

// ---- the page on a window ---------------------------------------------------------------

#[pyfunction]
fn page<'py>(win: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = win.py();
    if let Some(p) = win.getattr("__dict__")?.cast::<PyDict>()?.get_item("_start_page")? {
        if !p.is_none() {
            return Ok(p);
        }
    }
    let p = Bound::new(py, Page::make(py, win)?)?.into_any();
    p.setattr("sheet", Bound::new(py, StartSheet::make(py, &p)?)?)?;
    win.setattr("_start_page", &p)?;
    Ok(p)
}

#[pyfunction]
fn shown(win: &Bound<'_, PyAny>) -> PyResult<bool> {
    if win.is_none() {
        return Ok(false);
    }
    let p = win.getattr("__dict__")?.call_method1("get", ("_start_page",))?;
    Ok(!p.is_none() && p.getattr("wanted")?.is_truthy()?)
}

/// Open the Start sheet in this window.
#[pyfunction]
fn show(win: &Bound<'_, PyAny>) -> PyResult<bool> {
    page(win)?.call_method0("open")?.is_truthy()
}

#[pyfunction]
fn hide(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let p = win.getattr("__dict__")?.call_method1("get", ("_start_page",))?;
    if !p.is_none() && p.getattr("wanted")?.is_truthy()? {
        p.call_method0("close")?;
    }
    Ok(())
}

/// A document is open behind the page (not just the blank untitled one).
#[pyfunction]
fn can_return(win: &Bound<'_, PyAny>) -> bool {
    let check = || -> PyResult<bool> {
        let docs: Vec<Bound<'_, PyAny>> = match opt(win, "_documents") {
            Some(d) => d.try_iter()?.collect::<PyResult<_>>()?,
            None => Vec::new(),
        };
        if docs.len() > 1 {
            return Ok(true);
        }
        Ok(match docs.first() {
            Some(d) => !win.call_method1("_doc_is_blank", (d,))?.is_truthy()?,
            None => false,
        })
    };
    check().unwrap_or(false)
}

fn docs_sig<'py>(win: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyTuple>> {
    let py = win.py();
    let mut parts: Vec<Bound<'py, PyAny>> = Vec::new();
    if let Some(docs) = opt(win, "_documents") {
        for d in docs.try_iter()? {
            let d = d?;
            let path = d.getattr("path").unwrap_or_else(|_| py.None().into_bound(py));
            let name = d.getattr("import_name").unwrap_or_else(|_| py.None().into_bound(py));
            parts.push(PyTuple::new(py, [obj(py, d.as_ptr() as usize)?.into_bound(py), path.str()?.into_any(), name])?.into_any());
        }
    }
    let doc = win.getattr("_doc").unwrap_or_else(|_| py.None().into_bound(py));
    parts.push(obj(py, doc.as_ptr() as usize)?.into_bound(py));
    parts.push(obj(py, can_return(win))?.into_bound(py));
    PyTuple::new(py, parts)
}

/// One window's start page: whether it is wanted, the loader, the sheet's
/// provider.
#[pyclass(module = "ingetrazo_render.wgpu_start", dict, get_all, set_all)]
pub struct Page {
    win: Py<PyAny>,
    wanted: bool,
    drawn: bool,
    docs_sig: Py<PyAny>,
    loader: Py<PyAny>,
    poll: Py<PyAny>,
    dog: Py<PyAny>,
    focused: bool,
    asked: f64,
    sheet: Py<PyAny>,
}

impl Page {
    fn make(py: Python<'_>, win: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Page { win: win.clone().unbind(), wanted: false, drawn: false, docs_sig: py.None(), loader: py.None(), poll: py.None(), dog: py.None(), focused: false, asked: 0.0, sheet: py.None() })
    }
}

fn monotonic(py: Python<'_>) -> PyResult<f64> {
    py.import("time")?.call_method0("monotonic")?.extract()
}

fn timer<'py>(py: Python<'py>, win: &Bound<'py, PyAny>, f: Bound<'py, PyAny>, kw: Option<(u32, bool)>) -> PyResult<Bound<'py, PyAny>> {
    let timers = py.import("core.timers")?;
    match kw {
        Some((ms, repeat)) => timers.getattr("timer")?.call((win, f, ms), Some(&[("repeat", repeat)].into_py_dict(py)?)),
        None => timers.call_method1("timer", (win, f)),
    }
}

#[pymethods]
impl Page {
    #[new]
    fn new(py: Python<'_>, win: &Bound<'_, PyAny>) -> PyResult<Self> {
        Page::make(py, win)
    }

    fn open(slf: &Bound<'_, Self>) -> PyResult<bool> {
        let py = slf.py();
        let win = slf.borrow().win.bind(py).clone();
        if let Some(docs) = opt(&win, "_documents") {
            for doc in docs.try_iter()? {
                let doc = doc?;
                if let Some(path) = opt(&doc, "path") {
                    note(&win, &path, Some(doc))?;
                }
            }
        }
        let cs = cs(py)?;
        let sheet = slf.borrow().sheet.clone_ref(py);
        if slf.borrow().wanted && cs.call_method1("current", (&win,))?.is(sheet.bind(py)) {
            // Already up (asked again at the end of a cold start): fresh
            // rows, nothing typed lost.
            slf.call_method0("load")?;
            cs.call_method1("refresh", (&win,))?;
            return Ok(true);
        }
        cs.call_method1("open", (&win, sheet.bind(py)))?;
        {
            let mut p = slf.borrow_mut();
            p.wanted = true;
            p.drawn = false;
            p.focused = false;
        }
        sheet.bind(py).setattr("focus", py.None())?;
        slf.borrow_mut().asked = monotonic(py)?;
        slf.borrow_mut().docs_sig = docs_sig(&win)?.into_any().unbind();
        slf.call_method0("load")?;
        if let Some(view) = view_of(&win) {
            focus_view(py, &view);
        }
        if !cs.call_method0("in_shell")?.is_truthy()? {
            return Ok(true); // headless: no frames come, nothing to watch for
        }
        if slf.borrow().dog.is_none(py) {
            let dog = timer(py, &win, slf.getattr("_watchdog")?, None)?;
            slf.borrow_mut().dog = dog.unbind();
        }
        slf.borrow().dog.bind(py).call_method1("start", (WATCHDOG_MS,))?;
        Ok(true)
    }

    fn close(slf: &Bound<'_, Self>) -> PyResult<()> {
        let py = slf.py();
        slf.borrow_mut().wanted = false;
        let (dog, win, sheet) = {
            let p = slf.borrow();
            (p.dog.clone_ref(py), p.win.clone_ref(py), p.sheet.clone_ref(py))
        };
        if !dog.is_none(py) {
            dog.bind(py).call_method0("stop")?;
        }
        let cs = cs(py)?;
        if cs.call_method1("current", (win.bind(py),))?.is(sheet.bind(py)) {
            cs.call_method1("close", (win.bind(py),))?;
        }
        slf.call_method0("repaint")?;
        Ok(())
    }

    fn _watchdog(slf: &Bound<'_, Self>) -> PyResult<()> {
        let py = slf.py();
        if !slf.borrow().wanted || slf.borrow().drawn {
            return Ok(());
        }
        let win = slf.borrow().win.bind(py).clone();
        let view = view_of(&win);
        let waited = monotonic(py)? - slf.borrow().asked;
        let coming = match &view {
            Some(v) => opt(v, "renderer").is_none() || !opt(v, "_ui_attached").is_some_and(|a| a.is_truthy().unwrap_or(false)) || !win.call_method0("isVisible")?.is_truthy()?,
            None => false,
        };
        if coming && waited < 10.0 {
            slf.borrow().dog.bind(py).call_method1("start", (500,))?; // a cold start: the Metal view is still on its way
            return Ok(());
        }
        slf.borrow_mut().wanted = false;
        let sheet = slf.borrow().sheet.clone_ref(py);
        if cs(py)?.call_method1("current", (&win,))?.is(sheet.bind(py)) {
            win.getattr("__dict__")?.call_method1("pop", ("_chrome_sheet", py.None()))?;
        }
        win.call_method0("statusBar")?.call_method1("showMessage", ("Built could not draw the Start sheet.",))?;
        Ok(())
    }

    fn repaint(&self, py: Python<'_>) {
        if let Some(view) = view_of(self.win.bind(py)) {
            let _ = view.call_method0("update"); // a view already gone
        }
    }

    fn changed(slf: &Bound<'_, Self>) -> PyResult<()> {
        let py = slf.py();
        cs(py)?.call_method1("refresh", (slf.borrow().win.bind(py),))?;
        slf.borrow().repaint(py);
        Ok(())
    }

    /// Start the worker on the recents it has not read yet.
    fn load(slf: &Bound<'_, Self>) -> PyResult<()> {
        let py = slf.py();
        let rec = recent(py)?;
        let mut paths: Vec<String> = Vec::new();
        for kind in ["files", "projects"] {
            for r in rec.call_method1("entries", (kind,))?.try_iter()? {
                paths.push(r?.get_item("path")?.extract()?);
            }
        }
        let last = last_file(py)?;
        if !last.is_empty() && !paths.contains(&last) {
            paths.push(last);
        }
        let mut todo = Vec::new();
        for p in paths {
            if _cached(py, &p)?.is_none() {
                todo.push(p);
            }
        }
        let loader = slf.borrow().loader.clone_ref(py);
        if todo.is_empty() || (!loader.is_none(py) && loader.bind(py).call_method0("is_alive")?.is_truthy()?) {
            return Ok(());
        }
        let kw = PyDict::new(py);
        kw.set_item("target", start(py)?.getattr("_load_all")?)?;
        kw.set_item("args", (todo,))?;
        kw.set_item("daemon", true)?;
        kw.set_item("name", "start-page-thumbnails")?;
        let thread = py.import("threading")?.getattr("Thread")?.call((), Some(&kw))?;
        thread.call_method0("start")?;
        slf.borrow_mut().loader = thread.unbind();
        if slf.borrow().poll.is_none(py) {
            let win = slf.borrow().win.bind(py).clone();
            let poll = timer(py, &win, slf.getattr("_poll")?, Some((120, true)))?;
            slf.borrow_mut().poll = poll.unbind();
        }
        slf.borrow().poll.bind(py).call_method0("start")?;
        Ok(())
    }

    fn _poll(slf: &Bound<'_, Self>) -> PyResult<()> {
        let py = slf.py();
        let loader = slf.borrow().loader.clone_ref(py);
        let busy = !loader.is_none(py) && loader.bind(py).call_method0("is_alive")?.is_truthy()?;
        if _take_news(py)? {
            slf.call_method0("changed")?;
        }
        if !busy {
            slf.borrow().poll.bind(py).call_method0("stop")?;
            if _take_news(py)? {
                slf.call_method0("changed")?;
            }
        }
        Ok(())
    }

    fn before_frame(slf: &Bound<'_, Self>, view: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = slf.py();
        let win = slf.borrow().win.bind(py).clone();
        if slf.borrow().wanted && docs_sig(&win)?.ne(slf.borrow().docs_sig.bind(py))? {
            // A document came in (⌘O, a drop, the OS): the page has done its job.
            slf.call_method0("close")?;
        }
        if !slf.borrow().wanted {
            return Ok(());
        }
        let cs = cs(py)?;
        let sheet = slf.borrow().sheet.clone_ref(py);
        let current = cs.call_method1("current", (&win,))?;
        if !current.is(sheet.bind(py)) {
            // Another sheet took the window (Settings from the footer): the
            // page waits under it and comes back when that one closes.
            if current.is_none() {
                cs.call_method1("open", (&win, sheet.bind(py)))?;
            }
            return Ok(());
        }
        if !slf.borrow().focused {
            // Keys come to the view; never focus from inside a frame (a
            // native view can expose, and paint, right there).
            slf.borrow_mut().focused = true;
            let v = view.clone().unbind();
            let focus = func(py, move |py, _a| {
                focus_view(py, v.bind(py));
                Ok(none(py))
            })?;
            py.import("core.timers")?.call_method1("later", (&win, 0, focus))?;
        }
        Ok(())
    }
}

// ---- the sheet -----------------------------------------------------------------------------

/// The start page on the large sheet: verbs and a filter in the header, a
/// dense list on the left (continue, pinned, recent, projects, templates),
/// a preview on the right of the focused file.
#[pyclass(module = "ingetrazo_render.wgpu_start", extends = crate::sheet_runtime::Provider, dict, get_all, set_all)]
pub struct StartSheet {
    page: Py<PyAny>,
    focus: Py<PyAny>,
    filter: String,
    cont_path: String,
}

impl StartSheet {
    fn make(py: Python<'_>, page: &Bound<'_, PyAny>) -> PyResult<PyClassInitializer<Self>> {
        Ok(PyClassInitializer::from(crate::sheet_runtime::Provider).add_subclass(StartSheet { page: page.clone().unbind(), focus: py.None(), filter: String::new(), cont_path: String::new() }))
    }
}

fn page_win<'py>(sheet: &Bound<'py, StartSheet>) -> PyResult<Bound<'py, PyAny>> {
    sheet.borrow().page.bind(sheet.py()).getattr("win")
}

#[pymethods]
impl StartSheet {
    #[new]
    fn new(py: Python<'_>, page: &Bound<'_, PyAny>) -> PyResult<PyClassInitializer<Self>> {
        StartSheet::make(py, page)
    }

    #[classattr]
    fn id() -> &'static str {
        "start"
    }

    fn signature<'py>(slf: &Bound<'py, Self>, _ctx: &Bound<'py, PyAny>) -> PyResult<(bool, Bound<'py, PyAny>)> {
        let win = page_win(slf)?;
        Ok((can_return(&win), win.getattr("__dict__")?.call_method1("get", ("_pending_open",))?))
    }

    /// The list's sections after the filter.
    fn items<'py>(slf: &Bound<'py, Self>, back: bool) -> PyResult<Bound<'py, PyDict>> {
        let py = slf.py();
        let out = PyDict::new(py);
        for pair in py.import("ingetrazo_render")?.call_method1("start_sections", (state(slf, back)?,))?.try_iter()? {
            let pair = pair?;
            out.set_item(pair.get_item(0)?, pair.get_item(1)?)?;
        }
        Ok(out)
    }

    fn build<'py>(slf: &Bound<'py, Self>, _ctx: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let py = slf.py();
        let page = slf.borrow().page.bind(py).clone();
        page.setattr("drawn", true)?;
        let win = page.getattr("win")?;
        let text = py.import("ingetrazo_render")?.call_method1("start_sheet", (state(slf, can_return(&win))?,))?;
        let spec = py.import("json")?.call_method1("loads", (text,))?;
        slf.setattr("focus", spec.get_item("focus")?)?;
        slf.setattr("cont_path", spec.call_method1("pop", ("cont_path",))?)?;
        let icons = spec.call_method1("pop", ("icons",))?;
        let sheet = cs(py)?.getattr("Spec")?.call1((&spec,))?;
        let pictures = locked(py, || {
            let p = PyDict::new(py);
            for c in cache(py)?.values().iter() {
                let thumb = c.call_method1("get", ("thumb",))?;
                if thumb.is_truthy()? {
                    p.set_item(thumb.get_item(0)?, thumb)?;
                }
            }
            Ok(p)
        })?;
        for key in icons.try_iter()? {
            if let Some(pic) = pictures.get_item(key?)? {
                sheet.getattr("icon")?.call1(pic.cast_into::<PyTuple>()?)?;
            }
        }
        Ok(sheet)
    }

    fn act(slf: &Bound<'_, Self>, _ctx: &Bound<'_, PyAny>, verb: &str, args: Vec<String>) -> PyResult<()> {
        let py = slf.py();
        let win = page_win(slf)?;
        let arg = args.first().cloned().unwrap_or_default();
        match verb {
            "search" => slf.borrow_mut().filter = arg,
            "focus" => slf.setattr("focus", &arg)?,
            "open" => slf.call_method1("activate", (&win, &arg)).map(|_| ())?,
            "menu" if args.len() > 1 => slf.call_method1("item_verb", (&win, &args[1], &arg)).map(|_| ())?,
            "headmenu" if arg == "template" && args.len() > 1 => from_template(&win, &args[1])?,
            "verb" => {
                if matches!(arg.as_str(), "open" | "finder" | "locate" | "forget" | "pin") {
                    let focus = slf.getattr("focus")?;
                    let focus = if focus.is_truthy()? { focus.str()?.to_string() } else { String::new() };
                    slf.call_method1("item_verb", (&win, &arg, focus))?;
                } else {
                    let routed = match arg.as_str() {
                        "libraries" => format!("nav{SEP}libraries"),
                        "prefs" => format!("cmd{SEP}prefs"),
                        "extensions" => format!("cmd{SEP}extensions"),
                        "help" => format!("cmd{SEP}help"),
                        other => other.to_owned(),
                    };
                    action(py, &win, &routed)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn activate(&self, py: Python<'_>, win: &Bound<'_, PyAny>, rid: &str) -> PyResult<()> {
        let (kind, rest) = rid.split_once(':').unwrap_or((rid, ""));
        if rid == "cont" {
            action(py, win, "continue")?;
        } else if kind == "tpl" {
            from_template(win, rest)?;
        } else if kind == "file" || kind == "proj" {
            start(py)?.call_method1("open_recent", (win, rest))?;
        }
        Ok(())
    }

    fn item_verb(slf: &Bound<'_, Self>, win: &Bound<'_, PyAny>, verb: &str, rid: &str) -> PyResult<()> {
        let py = slf.py();
        let (mut kind, mut path) = rid.split_once(':').map(|(a, b)| (a.to_owned(), b.to_owned())).unwrap_or((rid.to_owned(), String::new()));
        if rid == "cont" {
            kind = "file".into();
            path = slf.borrow().cont_path.clone();
        }
        if verb == "open" {
            return slf.call_method1("activate", (win, rid)).map(|_| ());
        }
        if path.is_empty() || !(kind == "file" || kind == "proj") {
            return Ok(());
        }
        match verb {
            "pin" => action(py, win, &format!("pin{SEP}{path}"))?,
            "forget" => action(py, win, &format!("forget{SEP}{path}"))?,
            "locate" => {
                start(py)?.call_method1("open_recent", (win, &path))?;
            }
            "finder" => reveal(py, &path)?,
            _ => {}
        }
        Ok(())
    }

    fn closed(slf: &Bound<'_, Self>, ctx: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = slf.py();
        if ctx.getattr("replaced")?.is_truthy()? {
            return Ok(()); // Settings over it: the page comes back after
        }
        let page = slf.borrow().page.bind(py).clone();
        if page.getattr("wanted")?.is_truthy()? {
            page.setattr("wanted", false)?;
            let dog = page.getattr("dog")?;
            if !dog.is_none() {
                dog.call_method0("stop")?;
            }
        }
        Ok(())
    }
}

/// What the Rust view model reads (start_view.rs `State`): the recents,
/// what each document noted, what the worker found.
fn state(sheet: &Bound<'_, StartSheet>, back: bool) -> PyResult<String> {
    let py = sheet.py();
    let win = page_win(sheet)?;
    let rec = recent(py)?;
    let json = py.import("json")?;
    let meta = store(py)?.call_method1("value", ("home/meta", "{}"))?;
    let meta = json.call_method1("loads", (meta,)).ok().filter(|m| m.is_instance_of::<PyDict>()).unwrap_or_else(|| PyDict::new(py).into_any());
    let open_doc = PyDict::new(py);
    if back {
        let doc = win.getattr("_doc").unwrap_or_else(|_| py.None().into_bound(py));
        let path = if doc.is_none() { None } else { opt(&doc, "path") };
        // As the recents hold it (resolved): one row, one picture.
        let resolved = match path {
            Some(p) => py.import("pathlib")?.getattr("Path")?.call1((p,))?.call_method0("resolve")?.str()?.to_string(),
            None => String::new(),
        };
        open_doc.set_item("path", resolved)?;
        let name = if !doc.is_none() && doc.hasattr("display_name")? { doc.call_method0("display_name")? } else { s(py, "Untitled") };
        open_doc.set_item("name", name)?;
    }
    let cache_view = locked(py, || {
        let out = PyDict::new(py);
        for (p, c) in cache(py)?.iter() {
            let get = |k: &str| c.call_method1("get", (k,));
            let thumb = get("thumb")?;
            let d = PyDict::new(py);
            d.set_item("exists", get("exists")?)?;
            d.set_item("mtime", get("mtime")?)?;
            d.set_item("thumb", if thumb.is_truthy()? { thumb.get_item(0)? } else { s(py, "") })?;
            d.set_item("files", get("files")?)?;
            d.set_item("links", get("links")?)?;
            d.set_item("autosave_newer", get("autosave_newer")?.is_truthy()?)?;
            out.set_item(p, d)?;
        }
        Ok(out)
    })?;
    let pending = win.getattr("__dict__")?.call_method1("get", ("_pending_open",))?;
    let d = PyDict::new(py);
    d.set_item("filter", &sheet.borrow().filter)?;
    d.set_item("focus", sheet.getattr("focus")?)?;
    d.set_item("back", back)?;
    d.set_item("open_doc", open_doc)?;
    d.set_item("last", last_file(py)?)?;
    d.set_item("files", rec.call_method1("entries", ("files",))?)?;
    d.set_item("projects", rec.call_method1("entries", ("projects",))?)?;
    d.set_item("cache", cache_view)?;
    d.set_item("meta", meta)?;
    d.set_item("pending", if pending.is_truthy()? { pending.getattr("job")?.getattr("label")? } else { s(py, "") })?;
    d.set_item("engine", footer(py))?;
    d.set_item("home", py.import("pathlib")?.getattr("Path")?.call_method0("home")?.str()?)?;
    let time = py.import("time")?;
    d.set_item("now", time.call_method0("time")?)?;
    d.set_item("utc_offset", time.call_method0("localtime")?.getattr("tm_gmtoff")?)?;
    json.call_method1("dumps", (d,))?.extract()
}

/// Show the file in the Finder (both apps).
fn reveal(py: Python<'_>, path: &str) -> PyResult<()> {
    py.import("core.desktop_open")?.call_method1("reveal", (path,))?;
    Ok(())
}

/// The document opened or saved last (pins reorder the recent list, so its
/// first row is not it).
fn last_file(py: Python<'_>) -> PyResult<String> {
    let value = store(py)?.call_method1("value", ("home/last_file", ""))?;
    if value.is_instance_of::<pyo3::types::PyString>() && value.is_truthy()? {
        return value.extract();
    }
    let rows: Vec<Bound<'_, PyAny>> = recent(py)?.call_method1("entries", ("files",))?.try_iter()?.collect::<PyResult<_>>()?;
    let mut plain = Vec::new();
    for r in &rows {
        if !r.call_method1("get", ("pinned",))?.is_truthy()? {
            plain.push(r.clone());
        }
    }
    Ok(match plain.first().or(rows.first()) {
        Some(r) => r.get_item("path")?.extract()?,
        None => String::new(),
    })
}

#[pyfunction]
#[pyo3(name = "last_file")]
fn last_file_py(py: Python<'_>) -> PyResult<String> {
    last_file(py)
}

fn footer(py: Python<'_>) -> String {
    let text = || -> PyResult<String> {
        let engine: String = py.import("core._native")?.call_method0("engine")?.extract()?;
        let renderer: String = py.import("core.renderer_settings")?.call_method0("status_text")?.extract()?;
        Ok(format!("Engine: {engine} · Renderer: {renderer}"))
    };
    text().unwrap_or_default()
}

// ---- what a document says about itself (noted on open, save, start) ------------------

/// Remember what the start page shows of a document while it is open: its
/// project, links, origin, unfiled items and problems, where it was left.
fn note(win: &Bound<'_, PyAny>, path: &Bound<'_, PyAny>, doc: Option<Bound<'_, PyAny>>) -> PyResult<()> {
    let py = win.py();
    let gather = || -> PyResult<()> {
        let doc = doc.or_else(|| opt(win, "_doc"));
        let scene = doc.as_ref().and_then(|d| opt(d, "viewport")).and_then(|v| opt(&v, "scene"));
        let (Some(doc), Some(scene)) = (doc, scene) else { return Ok(()) };
        if path.is_none() {
            return Ok(());
        }
        let data = PyDict::new(py);
        let pathlib = py.import("pathlib")?.getattr("Path")?;
        let plugin = scene.getattr("plugin_data")?;
        if plugin.is_instance_of::<PyDict>() {
            let proj = plugin.call_method1("get", ("project",))?;
            let proj_path = if proj.is_truthy()? { proj.call_method1("get", ("path",))? } else { py.None().into_bound(py) };
            if proj_path.is_truthy()? {
                data.set_item("project", pathlib.call1((proj_path,))?.getattr("stem")?)?;
            }
        }
        data.set_item("links", py.import("core.links")?.call_method1("sources", (&scene,))?.len()?)?;
        if let Some(name) = opt(&doc, "import_name").filter(|n| n.is_truthy().unwrap_or(false)) {
            let suffix: String = pathlib.call1((name.str()?,))?.getattr("suffix")?.extract()?;
            data.set_item("imported", suffix.to_lowercase().trim_start_matches('.'))?;
        }
        if let Some(tree) = opt(&doc, "stand_in").and_then(|v| opt(&v, "_ui_tree")).filter(|t| t.is_instance_of::<PyDict>()) {
            let count = |k: &str| -> PyResult<usize> { let v = tree.call_method1("get", (k,))?; Ok(if v.is_truthy()? { v.len()? } else { 0 }) };
            data.set_item("unfiled", count("unfiled")?)?;
            data.set_item("problems", count("to_fix")?)?;
        }
        let mut where_ = "Model".to_owned();
        let row = opt(win, "_scene_tabs").and_then(|t| opt(&t, "tabs"));
        if let Some(row) = row.filter(|r| r.hasattr("marked_view").unwrap_or(false)) {
            let marked = row.call_method0("marked_view")?;
            if !marked.is_none() {
                let name = marked.getattr("name").map(|n| n.str().map(|s| s.to_string()).unwrap_or_default()).unwrap_or_default();
                if !name.is_empty() {
                    where_.push_str(&format!(" › {name}"));
                }
            }
        }
        data.set_item("workspace", where_)?;
        recent(py)?.call_method1("note", (path, data))?;
        Ok(())
    };
    // The page only loses a line.
    let _ = gather();
    Ok(())
}

#[pyfunction]
#[pyo3(name = "note", signature = (win, path, doc=None))]
fn note_py(win: &Bound<'_, PyAny>, path: &Bound<'_, PyAny>, doc: Option<Bound<'_, PyAny>>) -> PyResult<()> {
    note(win, path, doc)
}

// ---- the worker: existence, pictures, project files ---------------------------------

fn _cached<'py>(py: Python<'py>, path: &str) -> PyResult<Bound<'py, PyAny>> {
    locked(py, || cache(py)?.call_method1("get", (path,)))
}

#[pyfunction]
#[pyo3(name = "_cached")]
fn cached_py<'py>(py: Python<'py>, path: &str) -> PyResult<Bound<'py, PyAny>> {
    _cached(py, path)
}

#[pyfunction]
fn _take_news(py: Python<'_>) -> PyResult<bool> {
    locked(py, || {
        let news = start(py)?.getattr("_NEWS")?;
        let was = news.get_item(0)?.is_truthy()?;
        news.set_item(0, false)?;
        Ok(was)
    })
}

/// The worker thread: read each path; one bad file never stops the rest.
#[pyfunction]
fn _load_all(py: Python<'_>, paths: Vec<String>) -> PyResult<()> {
    let load_one = start(py)?.getattr("_load_one")?;
    for path in paths {
        let entry = match load_one.call1((&path,)) {
            Ok(e) => e,
            Err(_) => [("exists", py.None())].into_py_dict(py)?.into_any(),
        };
        locked(py, || {
            cache(py)?.set_item(&path, &entry)?;
            start(py)?.getattr("_NEWS")?.set_item(0, true)
        })?;
    }
    Ok(())
}

/// What the page shows of one file without opening it.
#[pyfunction]
fn _load_one<'py>(py: Python<'py>, path: &str) -> PyResult<Bound<'py, PyDict>> {
    let p = py.import("pathlib")?.getattr("Path")?.call1((path,))?;
    let st = match p.call_method0("stat") {
        Ok(st) => st,
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyOSError>(py) => return [("exists", false)].into_py_dict(py),
        Err(e) => return Err(e),
    };
    let entry = PyDict::new(py);
    entry.set_item("exists", true)?;
    entry.set_item("mtime", st.getattr("st_mtime")?)?;
    entry.set_item("key", (st.getattr("st_mtime_ns")?, st.getattr("st_size")?))?;
    let suffix: String = p.getattr("suffix")?.extract::<String>()?.to_lowercase();
    if suffix == ".builtproject" || suffix == ".igzproj" {
        // Read-only: listing a project never converts it.
        let files = py.import("ingetrazo_core").and_then(|c| c.call_method1("project_members", (&p,))).and_then(|rows| {
            let out = PyList::empty(py);
            for r in rows.try_iter()? {
                out.append(py.import("builtins")?.getattr("tuple")?.call1((r?,))?)?;
            }
            Ok(out.into_any())
        });
        entry.set_item("files", files.unwrap_or_else(|_| py.None().into_bound(py)))?; // unreadable: said on its card
        return Ok(entry);
    }
    entry.set_item("thumb", _thumbnail(py, &p, &st)?)?;
    let links = py.import("core.link_scan").and_then(|m| m.call_method1("scan", (&p,)));
    entry.set_item("links", links.unwrap_or_else(|_| PyList::empty(py).into_any()))?; // the preview only loses its links
    let newer = (|| -> PyResult<bool> {
        let slot = py.import("core.autosave")?.call_method1("pending", (&p,))?;
        if slot.is_none() {
            return Ok(false);
        }
        let slot_m: f64 = slot.call_method0("stat")?.getattr("st_mtime")?.extract()?;
        let m: f64 = st.getattr("st_mtime")?.extract()?;
        Ok(slot_m > m + 1.0)
    })()
    .unwrap_or(false);
    entry.set_item("autosave_newer", newer)?;
    Ok(entry)
}

/// `(texture key, w, h, RGBA)` of the picture a save keeps in the archive.
fn _thumbnail<'py>(py: Python<'py>, p: &Bound<'py, PyAny>, st: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let nothing = || py.None().into_bound(py);
    let data = (|| -> PyResult<Option<Bound<'py, PyAny>>> {
        let archive = py.import("zipfile")?.getattr("ZipFile")?.call1((p,))?;
        let read = (|| -> PyResult<Option<Bound<'py, PyAny>>> {
            let info = archive.call_method1("getinfo", ("thumbnail.png",))?;
            if info.getattr("file_size")?.extract::<u64>()? > 4 * 1024 * 1024 {
                return Ok(None);
            }
            Ok(Some(archive.call_method1("read", (info,))?))
        })();
        archive.call_method0("close")?;
        read
    })();
    // A missing or broken archive (OSError, KeyError, BadZipFile, ValueError): no picture.
    let Ok(Some(data)) = data else { return Ok(nothing()) };
    let picture = py.import("ingetrazo_render")?.call_method1("archive_thumbnail", (data, THUMB_PX))?;
    if picture.is_none() {
        return Ok(nothing());
    }
    let ident = format!("{}|{}|{}", p.str()?, st.getattr("st_mtime_ns")?, st.getattr("st_size")?);
    let digest: String = py.import("hashlib")?.call_method1("sha1", (ident.as_bytes(),))?.call_method0("hexdigest")?.extract()?;
    let key = format!("thumb:{}", &digest[..16]);
    Ok(PyTuple::new(py, [s(py, &key), picture.get_item(0)?, picture.get_item(1)?, picture.get_item(2)?])?.into_any())
}

/// An image object a caller hands in (`isNull`, `convertToFormat`,
/// `constBits`); Rust samples its RGBA pixels. The format is read from the
/// image's own type, so nothing here imports an image toolkit.
#[pyfunction]
fn blank_picture(py: Python<'_>, image: &Bound<'_, PyAny>) -> PyResult<bool> {
    if image.is_none() || image.call_method0("isNull")?.is_truthy()? {
        return Ok(true);
    }
    let fmt = image.get_type().getattr("Format_RGBA8888")?;
    let rgba = image.call_method1("convertToFormat", (fmt,))?;
    let bytes = py.import("builtins")?.getattr("bytes")?.call1((rgba.call_method0("constBits")?,))?;
    py.import("ingetrazo_render")?.call_method1("blank_picture", (rgba.call_method0("width")?, rgba.call_method0("height")?, bytes))?.is_truthy()
}

/// Tests: start from nothing known.
#[pyfunction]
fn forget_cache(py: Python<'_>) -> PyResult<()> {
    locked(py, || {
        cache(py)?.clear();
        start(py)?.getattr("_NEWS")?.set_item(0, false)
    })
}

// ---- frames, actions ------------------------------------------------------------------

#[pyfunction]
fn before_frame(view: &Bound<'_, PyAny>, win: &Bound<'_, PyAny>) -> PyResult<()> {
    if win.is_none() {
        return Ok(());
    }
    let p = win.getattr("__dict__")?.call_method1("get", ("_start_page",))?;
    if !p.is_none() {
        p.call_method1("before_frame", (view,))?;
    }
    Ok(())
}

/// The library browser.
fn libraries(py: Python<'_>) -> Option<Bound<'_, PyAny>> {
    let lib = py.import("ingetrazo_render").ok()?.getattr("wgpu_library").ok()?;
    ["show", "open", "open_library", "toggle"].iter().find_map(|n| lib.getattr(*n).ok().filter(|f| f.is_callable()))
}

/// `<verb>[\x1f<arg>]`: the page's verbs (the sheet, the shell's menu and
/// the tests call them by name).
fn action(py: Python<'_>, win: &Bound<'_, PyAny>, arg: &str) -> PyResult<()> {
    let (verb, rest) = arg.split_once(SEP).unwrap_or((arg, ""));
    let p = page(win)?;
    let rec = recent(py)?;
    let module = start(py)?;
    match verb {
        "show" => {
            show(win)?;
        }
        "close" => {
            if can_return(win) {
                hide(win)?;
            }
        }
        "new" => {
            module.call_method1("new_model", (win,))?;
        }
        "project" => new_project(win)?,
        "openfile" => open_dialog(win)?,
        "open" if !rest.is_empty() => {
            module.call_method1("open_recent", (win, rest))?;
        }
        "continue" => {
            if can_return(win) {
                hide(win)?;
            } else {
                let path = last_file(py)?;
                if !path.is_empty() {
                    module.call_method1("open_recent", (win, path))?;
                }
            }
        }
        // Opening asks which to keep (the file or its auto-saved copy).
        "compare" if !rest.is_empty() => {
            module.call_method1("open_recent", (win, rest))?;
        }
        "pin" if !rest.is_empty() => {
            let mut pinned = false;
            'find: for kind in ["files", "projects"] {
                for r in rec.call_method1("entries", (kind,))?.try_iter()? {
                    let r = r?;
                    if r.get_item("path")?.eq(rest)? {
                        pinned = r.call_method1("get", ("pinned", false))?.is_truthy()?;
                        break 'find;
                    }
                }
            }
            rec.getattr("remember")?.call((rest,), Some(&[("pinned", !pinned)].into_py_dict(py)?))?;
            p.call_method0("changed")?;
        }
        // Off the list only; the file stays where it is.
        "forget" if !rest.is_empty() => {
            rec.call_method1("forget", (rest,))?;
            p.call_method0("changed")?;
        }
        "clearmissing" => {
            let sheet = p.getattr("sheet")?.cast_into::<StartSheet>()?;
            let gone = py.import("ingetrazo_render")?.call_method1("start_missing", (state(&sheet, false)?,))?;
            if gone.is_truthy()? {
                rec.call_method1("forget", (gone,))?;
            }
            p.call_method0("changed")?;
        }
        "template" if !rest.is_empty() => from_template(win, rest)?,
        "nav" if rest == "libraries" => {
            if let Some(f) = libraries(py) {
                hide(win)?;
                if let Err(e) = f.call1((win,)) {
                    if !e.is_instance_of::<pyo3::exceptions::PyTypeError>(py) {
                        return Err(e);
                    }
                    f.call0()?;
                }
            }
        }
        "cmd" => {
            let slot = match rest {
                "prefs" => Some("_on_preferences"),
                "extensions" => Some("_on_manage_extensions"),
                "help" => Some("_on_documentation"),
                _ => None,
            };
            if let Some(f) = slot.and_then(|n| opt(win, n)).filter(|f| f.is_callable()) {
                f.call0()?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[pyfunction]
#[pyo3(name = "action")]
fn action_py(py: Python<'_>, win: &Bound<'_, PyAny>, arg: &str) -> PyResult<()> {
    action(py, win, arg)
}

// ---- verbs -------------------------------------------------------------------

#[pyfunction]
#[pyo3(signature = (win, ask_units=true))]
fn new_model(py: Python<'_>, win: &Bound<'_, PyAny>, ask_units: bool) -> PyResult<()> {
    win.call_method0("_on_new")?;
    hide(win)?;
    if ask_units {
        py.import("ingetrazo_render.units_question")?.call_method1("offer_at_start", (win,))?;
    }
    Ok(())
}

fn new_project(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let dialogs = py.import("ingetrazo_render.shell_files")?.getattr("ShellFileDialog")?;
    let name = dialogs.call_method1("getSaveFileName", (win, "New project", "", "Built project (*.builtproject)"))?.get_item(0)?;
    if !name.is_truthy()? {
        return Ok(());
    }
    let path = py.import("pathlib")?.getattr("Path")?.call1((name,))?.call_method1("with_suffix", (".builtproject",))?;
    let made = (|| -> PyResult<()> {
        let scene = py.import("core.scene")?.getattr("Scene")?.call0()?;
        py.import("core.project")?.call_method1("create", (scene, &path))?;
        win.call_method1("open_project", (&path,))?;
        Ok(())
    })();
    if let Err(e) = made {
        if e.is_instance_of::<pyo3::exceptions::PyOSError>(py) || e.is_instance_of::<pyo3::exceptions::PyValueError>(py) {
            py.import("ingetrazo_render.chrome_dialogs")?.call_method1("warning", (win, "Project", e.value(py).str()?))?;
            return Ok(());
        }
        return Err(e);
    }
    hide(win)
}

fn open_dialog(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let suffix: String = py.import("formats.built")?.getattr("BUILT_SUFFIX")?.extract()?;
    let dialogs = py.import("ingetrazo_render.shell_files")?.getattr("ShellFileDialog")?;
    let name = dialogs.call_method1("getOpenFileName", (win, "Open", "", format!("Documents (*{suffix} *.igz *.builtproject *.igzproj)")))?.get_item(0)?;
    if name.is_truthy()? {
        start(py)?.call_method1("open_recent", (win, name))?;
    }
    Ok(())
}

/// Open a recent file or project, offering to locate it if it moved. The
/// page goes once it is in.
#[pyfunction]
fn open_recent(py: Python<'_>, win: &Bound<'_, PyAny>, path: &Bound<'_, PyAny>) -> PyResult<bool> {
    let pathlib = py.import("pathlib")?.getattr("Path")?;
    let original = path.str()?.to_string();
    let mut path = pathlib.call1((path,))?;
    let suffix = |p: &Bound<'_, PyAny>| -> PyResult<String> { Ok(p.getattr("suffix")?.extract::<String>()?.to_lowercase()) };
    let is_file = |p: &Bound<'_, PyAny>| -> PyResult<bool> { p.call_method0("is_file")?.is_truthy() };
    if !is_file(&path)? && suffix(&path)? == ".igzproj" {
        let converted = path.call_method1("with_suffix", (".builtproject",))?;
        if is_file(&converted)? {
            path = converted; // converted since it was listed
        }
    }
    if !is_file(&path)? {
        let sfx = suffix(&path)?;
        let kind = if sfx == ".builtproject" || sfx == ".igzproj" {
            "Project"
        } else if py.import("formats.built")?.call_method1("is_built", (&path,))?.is_truthy()? {
            "Built"
        } else {
            "IngeTrazo"
        };
        let dialogs = py.import("ingetrazo_render.shell_files")?.getattr("ShellFileDialog")?;
        let located = dialogs.call_method1("getOpenFileName", (win, format!("Locate {}", path.getattr("name")?.str()?), "", format!("{kind} (*{})", path.getattr("suffix")?.str()?)))?.get_item(0)?;
        if !located.is_truthy()? {
            return Ok(false);
        }
        path = pathlib.call1((located,))?;
    }
    if !win.call_method1("open_path", (&path,))?.is_truthy()? {
        return Ok(false);
    }
    let rec = recent(py)?;
    if py.import("formats.project")?.call_method1("is_project", (&path,))?.is_truthy()? {
        // open_project remembered it (under its new name if converted).
        if original != path.str()?.to_string() {
            rec.call_method1("forget", (&original,))?;
        }
        hide(win)?;
        return Ok(true);
    }
    let pending = win.getattr("__dict__")?.call_method1("get", ("_pending_open",))?;
    if pending.is_truthy()? {
        let (p, o) = (path.clone().unbind(), original.clone());
        let then = func(py, move |py, _a| {
            recent(py)?.getattr("remember")?.call((p.bind(py),), Some(&[("replace", &o)].into_py_dict(py)?))?;
            Ok(none(py))
        })?;
        py.import("ingetrazo_render.document_open")?.call_method1("after_open", (win, &path, then))?;
        return Ok(true);
    }
    rec.getattr("remember")?.call((&path,), Some(&[("replace", &original)].into_py_dict(py)?))?;
    hide(win)?;
    Ok(true)
}

fn from_template(win: &Bound<'_, PyAny>, tid: &str) -> PyResult<()> {
    let py = win.py();
    let mut found = None;
    for t in start(py)?.getattr("TEMPLATES")?.try_iter()? {
        let t = t?;
        if t.get_item(0)?.eq(tid)? {
            found = Some(t);
            break;
        }
    }
    let Some(t) = found else { return Ok(()) };
    let (nodes, active, units) = (t.get_item(3)?, t.get_item(4)?, t.get_item(5)?);
    start(py)?.getattr("new_model")?.call((win,), Some(&[("ask_units", units.is_none())].into_py_dict(py)?))?;
    if nodes.is_none() && units.is_none() {
        return Ok(());
    }
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    if units.is_truthy()? {
        py.import("core.units")?.call_method1("apply_units", (&scene, units))?;
    }
    if nodes.is_truthy()? {
        apply_structure(py, &scene, &nodes, Some(active))?;
    }
    vp.call_method0("update")?;
    if let Some(view) = view_of(win) {
        view.setattr("_ui_tree_sig", py.None())?;
        view.call_method0("update")?;
    }
    Ok(())
}

#[pyfunction]
#[pyo3(name = "from_template")]
fn from_template_py(win: &Bound<'_, PyAny>, tid: &str) -> PyResult<()> {
    from_template(win, tid)
}

/// Seed a new document's structure (no undo step: like its units, the
/// template is where the document starts, not an edit).
#[pyfunction]
#[pyo3(signature = (scene, nodes, active=None))]
fn apply_structure(py: Python<'_>, scene: &Bound<'_, PyAny>, nodes: &Bound<'_, PyAny>, active: Option<Bound<'_, PyAny>>) -> PyResult<bool> {
    let bim = py.import("core.bim_registry")?;
    let registry = bim.call_method1("get", (scene,))?;
    let structure = PyDict::new(py);
    structure.set_item("nodes", py.import("copy")?.call_method1("deepcopy", (nodes,))?)?;
    if let Some(a) = active.filter(|a| a.is_truthy().unwrap_or(false)) {
        structure.set_item("active", a)?;
    }
    registry.set_item("structure", structure)?;
    if bim.call_method1("validate", (&registry,))?.is_truthy()? {
        return Ok(false);
    }
    scene.getattr("plugin_data")?.set_item("bim_registry", registry)?;
    Ok(true)
}

/// main.py: the window opens on the page when no file is given.
#[pyfunction]
fn wants_at_start(argv_has_file: bool) -> bool {
    !argv_has_file && std::env::var("BUILT_START").map_or(true, |v| v != "0")
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    let sub = PyModule::new(py, "wgpu_start")?;
    sub.add("SEP", SEP)?;
    sub.add("WATCHDOG_MS", WATCHDOG_MS)?;
    sub.add("THUMB_PX", THUMB_PX)?;
    sub.add("_CACHE", PyDict::new(py))?;
    sub.add("_LOCK", py.import("threading")?.call_method0("Lock")?)?;
    sub.add("_NEWS", PyList::new(py, [false])?)?;
    sub.add_class::<Page>()?;
    sub.add_class::<StartSheet>()?;
    for f in [
        wrap_pyfunction!(page, &sub)?,
        wrap_pyfunction!(shown, &sub)?,
        wrap_pyfunction!(show, &sub)?,
        wrap_pyfunction!(hide, &sub)?,
        wrap_pyfunction!(can_return, &sub)?,
        wrap_pyfunction!(last_file_py, &sub)?,
        wrap_pyfunction!(note_py, &sub)?,
        wrap_pyfunction!(cached_py, &sub)?,
        wrap_pyfunction!(_take_news, &sub)?,
        wrap_pyfunction!(_load_all, &sub)?,
        wrap_pyfunction!(_load_one, &sub)?,
        wrap_pyfunction!(blank_picture, &sub)?,
        wrap_pyfunction!(forget_cache, &sub)?,
        wrap_pyfunction!(before_frame, &sub)?,
        wrap_pyfunction!(action_py, &sub)?,
        wrap_pyfunction!(new_model, &sub)?,
        wrap_pyfunction!(open_recent, &sub)?,
        wrap_pyfunction!(from_template_py, &sub)?,
        wrap_pyfunction!(apply_structure, &sub)?,
        wrap_pyfunction!(wants_at_start, &sub)?,
        wrap_pyfunction!(view_in_front, &sub)?,
    ] {
        sub.add_function(f)?;
    }
    m.add_submodule(&sub)?;
    py.import("sys")?.getattr("modules")?.set_item("ingetrazo_render.wgpu_start", &sub)?;
    // Templates come from start_view.rs, registered before this.
    sub.add("TEMPLATES", templates(m)?)?;
    Ok(())
}
