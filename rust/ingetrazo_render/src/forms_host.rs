// SPDX-License-Identifier: GPL-3.0-or-later
//! Forms drawn by the chrome (`ui/dialog.rs`) instead of Qt dialogs: name
//! and number prompts, operation options, Units, Soften, export options.
//! The submodule `ingetrazo_render.chrome_forms`, with
//! `ingetrazo_render.units_question` beside it.
//!
//! A [`Form`] is plain data: fields with values, buttons, callbacks. `show`
//! puts it on the window and returns at once; the chrome reports every
//! edit and the button pressed; `on_done(answer, values)` then runs on a
//! later turn of whichever loop drives the window. Forms whose rows come
//! from a Rust model (an operation's options, the Browser's batch rename,
//! the first-run units) are the same `Form` with that model as its source;
//! Python fills a form and never subclasses it.
//!
//! Multi-step questions read top to bottom with `flow` and `request`.
use crate::dialogs_host::stderr;
use crate::settings_rows::{func, none, obj, opt, s, tr, trf};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};
use std::sync::atomic::{AtomicU64, Ordering};

/// Dialog field flags (ui/dialog.rs).
pub const DISABLED: i64 = 1;
const FOCUS: i64 = 2;
const MULTILINE: i64 = 4;
const PASSWORD: i64 = 8;
const FULL: i64 = 16;
const COMMIT_ON_BLUR: i64 = 32;

static SEQ: AtomicU64 = AtomicU64::new(1);

/// This submodule, through its attributes (tests replace `show`).
fn forms_mod(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("ingetrazo_render")?.getattr("chrome_forms")
}

fn dialogs(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("ingetrazo_render.chrome_dialogs")
}

fn truth(v: &Bound<'_, PyAny>, form: &Bound<'_, PyAny>) -> PyResult<bool> {
    if v.is_callable() { v.call1((form,))?.is_truthy() } else { v.is_truthy() }
}

/// A number as a field shows it: whole with no decimals, else trailing
/// zeros trimmed.
#[pyfunction]
pub fn number_text(py: Python<'_>, value: &Bound<'_, PyAny>, decimals: i64) -> PyResult<String> {
    if value.is_none() {
        return Ok(String::new());
    }
    let builtins = py.import("builtins")?;
    let v: f64 = builtins.getattr("float")?.call1((value,))?.extract()?;
    if decimals <= 0 {
        return Ok(builtins.getattr("round")?.call1((v,))?.str()?.to_string());
    }
    let text = format!("{v:.prec$}", prec = decimals as usize);
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    Ok(if trimmed.is_empty() { "0".into() } else { trimmed.into() })
}

// ---- fields ---------------------------------------------------------------------------

/// One row. `kind`: page, caption, text, edit, number, check, choice,
/// color, slider, path, button. `enabled` / `visible` may be callables of
/// the form, evaluated each time the form is drawn.
#[pyclass(module = "ingetrazo_render.chrome_forms", dict, weakref, get_all, set_all)]
pub struct Field {
    id: String,
    kind: String,
    label: Py<PyAny>,
    value: Py<PyAny>,
    options: Py<PyAny>,
    lo: f64,
    hi: f64,
    decimals: i64,
    suffix: Py<PyAny>,
    tip: Py<PyAny>,
    enabled: Py<PyAny>,
    visible: Py<PyAny>,
    focus: bool,
    multiline: bool,
    password: bool,
    full: bool,
    commit_on_blur: bool,
    /// check: the left column's label.
    row_label: Py<PyAny>,
    /// button: a note right of it.
    note: Py<PyAny>,
    /// path: "open" | "save" | "folder".
    pick: Py<PyAny>,
    /// path: a Qt filter string.
    filters: Py<PyAny>,
    /// slider: value → shown text.
    fmt: Py<PyAny>,
}

fn normalized<'py>(py: Python<'py>, options: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    let out = PyList::empty(py);
    for o in options.try_iter()? {
        let o = o?;
        if o.is_instance_of::<pyo3::types::PyString>() {
            out.append((&o, &o))?;
        } else {
            out.append((o.get_item(0)?, o.get_item(1)?))?;
        }
    }
    Ok(out)
}

#[pymethods]
impl Field {
    #[new]
    #[pyo3(signature = (fid, kind, label=None, value=None, *, options=None, lo=0.0, hi=0.0, decimals=0, suffix=None, tip=None,
                        enabled=None, visible=None, focus=false, multiline=false, password=false, full=false, commit_on_blur=false,
                        row_label=None, note=None, pick=None, filters=None, fmt=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        py: Python<'_>,
        fid: &Bound<'_, PyAny>,
        kind: String,
        label: Option<Py<PyAny>>,
        value: Option<Py<PyAny>>,
        options: Option<Bound<'_, PyAny>>,
        lo: f64,
        hi: f64,
        decimals: i64,
        suffix: Option<Py<PyAny>>,
        tip: Option<Py<PyAny>>,
        enabled: Option<Py<PyAny>>,
        visible: Option<Py<PyAny>>,
        focus: bool,
        multiline: bool,
        password: bool,
        full: bool,
        commit_on_blur: bool,
        row_label: Option<Py<PyAny>>,
        note: Option<Py<PyAny>>,
        pick: Option<Py<PyAny>>,
        filters: Option<Py<PyAny>>,
        fmt: Option<Py<PyAny>>,
    ) -> PyResult<Self> {
        let e = || s(py, "").unbind();
        let t = || obj(py, true);
        let options = match options {
            Some(o) => normalized(py, &o)?.into_any().unbind(),
            None => PyList::empty(py).into_any().unbind(),
        };
        Ok(Field {
            id: fid.str()?.to_string(),
            kind,
            label: label.unwrap_or_else(e),
            value: value.unwrap_or_else(|| py.None()),
            options,
            lo,
            hi,
            decimals,
            suffix: suffix.unwrap_or_else(e),
            tip: tip.unwrap_or_else(e),
            enabled: match enabled { Some(v) => v, None => t()? },
            visible: match visible { Some(v) => v, None => t()? },
            focus,
            multiline,
            password,
            full,
            commit_on_blur,
            row_label: row_label.unwrap_or_else(e),
            note: note.unwrap_or_else(e),
            pick: pick.unwrap_or_else(|| py.None()),
            filters: filters.unwrap_or_else(e),
            fmt: fmt.unwrap_or_else(|| py.None()),
        })
    }

    fn keys<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyAny>>> {
        self.options.bind(py).try_iter()?.map(|o| o?.get_item(0)).collect()
    }

    fn flags(&self, py: Python<'_>, form: &Bound<'_, PyAny>) -> PyResult<i64> {
        let mut f = 0;
        if !truth(self.enabled.bind(py), form)? {
            f |= DISABLED;
        }
        for (on, flag) in [(self.focus, FOCUS), (self.multiline, MULTILINE), (self.password, PASSWORD), (self.full, FULL), (self.commit_on_blur, COMMIT_ON_BLUR)] {
            if on {
                f |= flag;
            }
        }
        Ok(f)
    }
}

// ---- forms ----------------------------------------------------------------------------

/// Where a form's rows and edits come from.
#[derive(Clone, Copy, PartialEq)]
enum Source {
    /// Its own fields.
    Fields,
    /// A Rust operation model (`rows()`, `receive`, `values()`, `invalid_range()`).
    Operation,
    /// The Browser's batch rename model (`rows()`, `error()`, `apply(window)`).
    Rename,
    /// The first-run units draft (`units_question.model_rows`).
    Units,
}

/// A dialog's data and its answers. `buttons`: [(id, label)] in reading
/// order; `default` answers Enter, `cancel` Esc (and closes without
/// validating), `discard` sits apart on the left.
///
/// `on_done(answer, values)`: once. `on_change(form, field_id, value)`:
/// each edit, live. `on_button(form, field_id)`: an inline button.
/// `validate(form, answer)`: an error text keeps the form open.
#[pyclass(module = "ingetrazo_render.chrome_forms", dict, weakref)]
pub struct Form {
    #[pyo3(get, set)]
    seq: String,
    #[pyo3(get, set)]
    title: Py<PyAny>,
    #[pyo3(get, set)]
    note: Py<PyAny>,
    #[pyo3(get, set)]
    error: String,
    #[pyo3(get, set)]
    fields: Py<PyAny>,
    #[pyo3(get, set)]
    buttons: Py<PyAny>,
    #[pyo3(get, set)]
    default: Py<PyAny>,
    #[pyo3(get, set)]
    cancel: Py<PyAny>,
    #[pyo3(get, set)]
    discard: Py<PyAny>,
    #[pyo3(get, set)]
    width: f64,
    #[pyo3(get, set)]
    on_done: Py<PyAny>,
    #[pyo3(get, set)]
    on_change: Py<PyAny>,
    #[pyo3(get, set)]
    on_button: Py<PyAny>,
    #[pyo3(get, set)]
    validate: Py<PyAny>,
    /// One per window: a second show raises it.
    #[pyo3(get, set)]
    key: Py<PyAny>,
    #[pyo3(get, set)]
    page: i64,
    #[pyo3(get, set)]
    answer: Py<PyAny>,
    #[pyo3(get, set)]
    closed: bool,
    /// Drawn at least once.
    #[pyo3(get, set)]
    shown: bool,
    #[pyo3(get, set)]
    win: Py<PyAny>,
    /// Number fields: the text being typed.
    #[pyo3(get, set, name = "_texts")]
    texts: Py<PyAny>,
    #[pyo3(get, set)]
    version: i64,
    /// The model the rows come from, if any.
    #[pyo3(get)]
    model: Py<PyAny>,
    /// The batch rename's cached (rows, error).
    #[pyo3(get, set, name = "_layout")]
    layout: Py<PyAny>,
    source: Source,
}

impl Form {
    #[allow(clippy::too_many_arguments)]
    fn make(
        py: Python<'_>,
        title: Py<PyAny>,
        fields: &Bound<'_, PyAny>,
        note: Option<Py<PyAny>>,
        buttons: Option<Bound<'_, PyAny>>,
        default: Py<PyAny>,
        cancel: Py<PyAny>,
        discard: Option<Py<PyAny>>,
        width: f64,
        on_done: Option<Py<PyAny>>,
        on_change: Option<Py<PyAny>>,
        on_button: Option<Py<PyAny>>,
        validate: Option<Py<PyAny>>,
        key: Option<Py<PyAny>>,
    ) -> PyResult<Self> {
        let n = || py.None();
        let buttons = match buttons {
            Some(b) => PyList::new(py, b.try_iter()?.collect::<PyResult<Vec<_>>>()?)?,
            None => PyList::new(py, [("cancel", tr(py, "Cancel")?), ("ok", tr(py, "OK")?)])?,
        };
        Ok(Form {
            seq: SEQ.fetch_add(1, Ordering::Relaxed).to_string(),
            title,
            note: note.unwrap_or_else(|| s(py, "").unbind()),
            error: String::new(),
            fields: PyList::new(py, fields.try_iter()?.collect::<PyResult<Vec<_>>>()?)?.into_any().unbind(),
            buttons: buttons.into_any().unbind(),
            default,
            cancel,
            discard: discard.unwrap_or_else(n),
            width,
            on_done: on_done.unwrap_or_else(n),
            on_change: on_change.unwrap_or_else(n),
            on_button: on_button.unwrap_or_else(n),
            validate: validate.unwrap_or_else(n),
            key: key.unwrap_or_else(n),
            page: 0,
            answer: n(),
            closed: false,
            shown: false,
            win: n(),
            texts: PyDict::new(py).into_any().unbind(),
            version: 0,
            model: n(),
            layout: n(),
            source: Source::Fields,
        })
    }

    fn source(&self, _py: Python<'_>) -> Source {
        self.source
    }
}


fn find_field<'py>(form: &Bound<'py, Form>, fid: &str) -> PyResult<Option<Bound<'py, PyAny>>> {
    let py = form.py();
    for f in form.borrow().fields.bind(py).try_iter()? {
        let f = f?;
        if f.getattr("id")?.eq(fid)? {
            return Ok(Some(f));
        }
    }
    Ok(None)
}

fn parse_float(py: Python<'_>, raw: &str) -> Option<f64> {
    py.import("builtins").ok()?.getattr("float").ok()?.call1((raw.replace(',', ".").trim(),)).ok()?.extract().ok()
}

fn round_half_even(v: f64) -> i64 {
    v.round_ties_even() as i64
}

#[pymethods]
impl Form {
    #[new]
    #[pyo3(signature = (title, fields, *, note=None, buttons=None, default=None, cancel=None, discard=None, width=420.0,
                        on_done=None, on_change=None, on_button=None, validate=None, key=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        py: Python<'_>,
        title: Py<PyAny>,
        fields: &Bound<'_, PyAny>,
        note: Option<Py<PyAny>>,
        buttons: Option<Bound<'_, PyAny>>,
        default: Option<Py<PyAny>>,
        cancel: Option<Py<PyAny>>,
        discard: Option<Py<PyAny>>,
        width: f64,
        on_done: Option<Py<PyAny>>,
        on_change: Option<Py<PyAny>>,
        on_button: Option<Py<PyAny>>,
        validate: Option<Py<PyAny>>,
        key: Option<Py<PyAny>>,
    ) -> PyResult<Self> {
        let default = match default { Some(d) => d, None => obj(py, "ok")? };
        let cancel = match cancel { Some(c) => c, None => obj(py, "cancel")? };
        Form::make(py, title, fields, note, buttons, default, cancel, discard, width, on_done, on_change, on_button, validate, key)
    }

    // ---- values ---------------------------------------------------------------------

    fn field<'py>(slf: &Bound<'py, Self>, fid: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyAny>>> {
        find_field(slf, &fid.str()?.to_string())
    }

    fn value<'py>(slf: &Bound<'py, Self>, fid: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let py = slf.py();
        Ok(match find_field(slf, &fid.str()?.to_string())? {
            Some(f) => f.getattr("value")?,
            None => py.None().into_bound(py),
        })
    }

    fn values<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        let py = slf.py();
        if slf.borrow().source(py) == Source::Operation {
            return slf.borrow().model.bind(py).call_method0("values");
        }
        let out = PyDict::new(py);
        for f in slf.borrow().fields.bind(py).try_iter()? {
            let f = f?;
            let kind: String = f.getattr("kind")?.extract()?;
            if !matches!(kind.as_str(), "page" | "caption" | "text" | "button") {
                out.set_item(f.getattr("id")?, f.getattr("value")?)?;
            }
        }
        Ok(out.into_any())
    }

    /// Change a value from Python (restore defaults, a picked path).
    fn set(slf: &Bound<'_, Self>, fid: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = slf.py();
        let Some(f) = find_field(slf, &fid.str()?.to_string())? else { return Ok(()) };
        f.setattr("value", value)?;
        slf.borrow().texts.bind(py).call_method1("pop", (f.getattr("id")?, py.None()))?;
        slf.call_method0("changed")?;
        Ok(())
    }

    fn changed(slf: &Bound<'_, Self>) -> PyResult<()> {
        let py = slf.py();
        slf.borrow_mut().version += 1;
        let win = slf.borrow().win.clone_ref(py);
        if !win.is_none(py) {
            forms_mod(py)?.call_method1("_repaint", (win,))?;
        }
        Ok(())
    }

    /// An edit from the chrome: typed into its value.
    fn receive(slf: &Bound<'_, Self>, fid: &str, raw: &str) -> PyResult<()> {
        let py = slf.py();
        let source = slf.borrow().source(py);
        match source {
            Source::Operation | Source::Units => {
                slf.borrow().model.bind(py).call_method1("receive", (fid, raw))?;
                return slf.call_method0("changed").map(|_| ());
            }
            Source::Rename => {
                slf.borrow().model.bind(py).call_method1("receive", (fid, raw))?;
                {
                    let mut me = slf.borrow_mut();
                    me.layout = py.None();
                    me.error.clear();
                }
                return slf.call_method0("changed").map(|_| ());
            }
            Source::Fields => {}
        }
        let Some(f) = find_field(slf, fid)? else { return Ok(()) };
        let kind: String = f.getattr("kind")?.extract()?;
        let decimals: i64 = f.getattr("decimals")?.extract()?;
        let value: Bound<'_, PyAny> = match kind.as_str() {
            "edit" | "path" | "color" => s(py, raw),
            "number" => {
                slf.borrow().texts.bind(py).set_item(f.getattr("id")?, raw)?;
                let Some(mut v) = parse_float(py, raw) else { return Ok(()) }; // half typed: the value waits
                let (lo, hi): (f64, f64) = (f.getattr("lo")?.extract()?, f.getattr("hi")?.extract()?);
                if hi > lo {
                    v = v.max(lo).min(hi);
                }
                if decimals <= 0 { obj(py, round_half_even(v))?.into_bound(py) } else { obj(py, v)?.into_bound(py) }
            }
            "check" => obj(py, matches!(raw, "1" | "true" | "True"))?.into_bound(py),
            "choice" => {
                let Some(i) = py.import("builtins")?.getattr("float")?.call1((raw,)).ok().and_then(|v| v.extract::<f64>().ok()) else { return Ok(()) };
                let keys: Vec<Bound<'_, PyAny>> = f.call_method0("keys")?.extract()?;
                let i = i.trunc() as i64;
                if i < 0 || i as usize >= keys.len() {
                    return Ok(());
                }
                keys[i as usize].clone()
            }
            "slider" => {
                let Some(v) = py.import("builtins")?.getattr("float")?.call1((raw,)).ok().and_then(|v| v.extract::<f64>().ok()) else { return Ok(()) };
                if decimals <= 0 { obj(py, round_half_even(v))?.into_bound(py) } else { obj(py, v)?.into_bound(py) }
            }
            _ => return Ok(()),
        };
        f.setattr("value", &value)?;
        let on_change = slf.borrow().on_change.clone_ref(py);
        if !on_change.is_none(py) {
            on_change.bind(py).call1((slf, f.getattr("id")?, &value))?;
        }
        slf.call_method0("changed")?;
        Ok(())
    }

    // ---- answers --------------------------------------------------------------------

    fn press(slf: &Bound<'_, Self>, bid: &str) -> PyResult<()> {
        let py = slf.py();
        if slf.borrow().closed {
            return Ok(());
        }
        let mut is_button = false;
        for b in slf.borrow().buttons.bind(py).try_iter()? {
            if b?.get_item(0)?.eq(bid)? {
                is_button = true;
                break;
            }
        }
        if is_button {
            let (cancel, validate) = {
                let me = slf.borrow();
                (me.cancel.clone_ref(py), me.validate.clone_ref(py))
            };
            if !cancel.bind(py).eq(bid)? && !validate.is_none(py) {
                let err = validate.bind(py).call1((slf, bid))?;
                if err.is_truthy()? {
                    slf.borrow_mut().error = err.str()?.to_string();
                    slf.call_method0("changed")?;
                    return Ok(());
                }
            }
            slf.call_method1("finish", (bid,))?;
            return Ok(());
        }
        let Some(f) = find_field(slf, bid)? else { return Ok(()) };
        if !truth(&f.getattr("enabled")?, slf.as_any())? {
            return Ok(());
        }
        let on_button = slf.borrow().on_button.clone_ref(py);
        if f.getattr("kind")?.eq("path")? && f.getattr("pick")?.is_truthy()? {
            forms_mod(py)?.call_method1("_pick_path", (slf, &f))?;
        } else if !on_button.is_none(py) {
            on_button.bind(py).call1((slf, f.getattr("id")?))?;
        }
        slf.call_method0("changed")?;
        Ok(())
    }

    fn finish(slf: &Bound<'_, Self>, answer: Py<PyAny>) -> PyResult<()> {
        let py = slf.py();
        if slf.borrow().closed {
            return Ok(());
        }
        {
            let mut me = slf.borrow_mut();
            me.closed = true;
            me.answer = answer.clone_ref(py);
        }
        let win = slf.borrow().win.clone_ref(py);
        if !win.is_none(py) {
            forms_mod(py)?.call_method1("_remove", (win, slf))?;
        }
        let on_done = slf.borrow().on_done.clone_ref(py);
        if !on_done.is_none(py) {
            let values = slf.call_method0("values")?;
            on_done.bind(py).call1((answer, values))?;
        }
        Ok(())
    }

    /// Take it down from Python, as Cancel would.
    fn close(slf: &Bound<'_, Self>) -> PyResult<()> {
        let cancel = slf.borrow().cancel.clone_ref(slf.py());
        slf.call_method1("finish", (cancel,)).map(|_| ())
    }

    // ---- what the chrome draws ----------------------------------------------------------

    fn wire<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyTuple>> {
        let py = slf.py();
        let me = slf.borrow();
        let source = me.source(py);
        let rows = PyList::empty(py);
        let any = slf.as_any();
        let empty = || PyList::empty(py);
        for f in me.fields.bind(py).try_iter()? {
            let f = f?;
            if !truth(&f.getattr("visible")?, any)? {
                continue;
            }
            let flags: i64 = f.call_method1("flags", (any,))?.extract()?;
            let k: String = f.getattr("kind")?.extract()?;
            let (id, label, tip) = (f.getattr("id")?, f.getattr("label")?, f.getattr("tip")?);
            let value = f.getattr("value")?;
            let text_of = |v: &Bound<'py, PyAny>| -> PyResult<String> { Ok(if v.is_truthy()? { v.str()?.to_string() } else { String::new() }) };
            let float_of = |v: &Bound<'py, PyAny>| -> PyResult<f64> { Ok(if v.is_truthy()? { py.import("builtins")?.getattr("float")?.call1((v,))?.extract()? } else { 0.0 }) };
            let (lo, hi, decimals): (f64, f64, i64) = (f.getattr("lo")?.extract()?, f.getattr("hi")?.extract()?, f.getattr("decimals")?.extract()?);
            let suffix: String = f.getattr("suffix")?.str()?.to_string();
            let row: Bound<'py, PyTuple> = match k.as_str() {
                "page" | "caption" => PyTuple::new(py, [id, s(py, &k), label, s(py, ""), obj(py, 0.0)?.into_bound(py), obj(py, 0.0)?.into_bound(py), obj(py, 0.0)?.into_bound(py), empty().into_any(), obj(py, flags)?.into_bound(py), tip])?,
                "text" | "edit" | "color" => PyTuple::new(py, [id, s(py, &k), label, s(py, &text_of(&value)?), obj(py, 0.0)?.into_bound(py), obj(py, 0.0)?.into_bound(py), obj(py, 0.0)?.into_bound(py), empty().into_any(), obj(py, flags)?.into_bound(py), tip])?,
                "number" => {
                    let texts = me.texts.bind(py);
                    let text = match texts.call_method1("get", (&id,))?.extract::<Option<String>>()? {
                        Some(t) => t,
                        None => number_text(py, &value, decimals)?,
                    };
                    let options = if suffix.is_empty() { empty() } else { PyList::new(py, [&suffix])? };
                    PyTuple::new(py, [id, s(py, &k), label, s(py, &text), obj(py, float_of(&value)?)?.into_bound(py), obj(py, lo)?.into_bound(py), obj(py, hi)?.into_bound(py), options.into_any(), obj(py, flags)?.into_bound(py), tip])?
                }
                "check" => PyTuple::new(py, [id, s(py, &k), label, f.getattr("row_label")?, obj(py, if value.is_truthy()? { 1.0 } else { 0.0 })?.into_bound(py), obj(py, 0.0)?.into_bound(py), obj(py, 1.0)?.into_bound(py), empty().into_any(), obj(py, flags)?.into_bound(py), tip])?,
                "choice" => {
                    let keys: Vec<Bound<'py, PyAny>> = f.call_method0("keys")?.extract()?;
                    let mut at = 0usize;
                    for (i, key) in keys.iter().enumerate() {
                        if key.eq(&value)? {
                            at = i;
                            break;
                        }
                    }
                    let labels = PyList::new(py, f.getattr("options")?.try_iter()?.map(|o| o?.get_item(1)).collect::<PyResult<Vec<_>>>()?)?;
                    PyTuple::new(py, [id, s(py, &k), label, s(py, ""), obj(py, at as f64)?.into_bound(py), obj(py, 0.0)?.into_bound(py), obj(py, keys.len() as f64)?.into_bound(py), labels.into_any(), obj(py, flags)?.into_bound(py), tip])?
                }
                "slider" => {
                    let fmt = f.getattr("fmt")?;
                    let shown = if !fmt.is_none() {
                        fmt.call1((&value,))?
                    } else {
                        let mut t = number_text(py, &value, decimals)?;
                        if !suffix.is_empty() {
                            t.push(' ');
                            t.push_str(&suffix);
                        }
                        s(py, &t)
                    };
                    PyTuple::new(py, [id, s(py, &k), label, shown, obj(py, float_of(&value)?)?.into_bound(py), obj(py, lo)?.into_bound(py), obj(py, hi)?.into_bound(py), empty().into_any(), obj(py, flags)?.into_bound(py), tip])?
                }
                "path" => PyTuple::new(py, [id, s(py, &k), label, s(py, &text_of(&value)?), obj(py, 0.0)?.into_bound(py), obj(py, 0.0)?.into_bound(py), obj(py, 0.0)?.into_bound(py), PyList::new(py, [tr(py, "Choose…")?])?.into_any(), obj(py, flags)?.into_bound(py), tip])?,
                "button" => PyTuple::new(py, [id, s(py, &k), label, f.getattr("note")?, obj(py, 0.0)?.into_bound(py), obj(py, 0.0)?.into_bound(py), obj(py, 0.0)?.into_bound(py), empty().into_any(), obj(py, flags)?.into_bound(py), tip])?,
                _ => continue,
            };
            rows.append(row)?;
        }
        // Enter answers; in a form with a multiline field, the platform's submit chord does.
        let mut multiline = false;
        for f in me.fields.bind(py).try_iter()? {
            if f?.getattr("multiline")?.is_truthy()? {
                multiline = true;
                break;
            }
        }
        let submit: String = if multiline { py.import("ingetrazo_render")?.call_method0("multiline_submit_hint")?.extract()? } else { "↩".into() };
        let buttons = PyList::empty(py);
        for b in me.buttons.bind(py).try_iter()? {
            let b = b?;
            let (bid, label) = (b.get_item(0)?, b.get_item(1)?);
            let role: i64 = if bid.eq(me.default.bind(py))? { 1 } else if bid.eq(me.discard.bind(py))? { 2 } else if bid.eq(me.cancel.bind(py))? { 3 } else { 0 };
            let hint = match role { 1 => submit.as_str(), 3 => "esc", _ => "" };
            buttons.append((bid, label, hint, role))?;
        }
        let note = me.note.bind(py).str()?.to_string();
        let note = if me.error.is_empty() { note } else if note.is_empty() { me.error.clone() } else { format!("{}\n{note}", me.error) };
        let rows: Bound<'py, PyAny> = match source {
            Source::Fields => rows.into_any(),
            Source::Operation => {
                let out = PyList::empty(py);
                for r in me.model.bind(py).call_method0("rows")?.try_iter()? {
                    let r = r?;
                    let kind = r.get_item(1)?;
                    let options = r.get_item(7)?;
                    let options = if kind.eq("choice")? {
                        PyList::new(py, options.try_iter()?.map(|o| Ok(tr(py, &o?.str()?.to_string())?)).collect::<PyResult<Vec<_>>>()?)?.into_any()
                    } else {
                        options
                    };
                    out.append(PyTuple::new(py, [r.get_item(0)?, kind, s(py, &tr(py, &r.get_item(2)?.str()?.to_string())?), r.get_item(3)?, r.get_item(4)?, r.get_item(5)?, r.get_item(6)?, options, r.get_item(8)?, s(py, &tr(py, &r.get_item(9)?.str()?.to_string())?)])?)?;
                }
                out.into_any()
            }
            Source::Units => py.import("ingetrazo_render")?.getattr("units_question")?.call_method1("model_rows", (me.model.bind(py),))?,
            Source::Rename => {
                if me.layout.is_none(py) {
                    let model = me.model.bind(py);
                    let layout = PyTuple::new(py, [model.call_method0("rows")?, model.call_method0("error")?])?;
                    drop(me);
                    slf.borrow_mut().layout = layout.into_any().unbind();
                    return Form::wire_with_layout(slf, buttons, note);
                }
                drop(me);
                return Form::wire_with_layout(slf, buttons, note);
            }
        };
        PyTuple::new(py, [s(py, &me.seq), me.title.bind(py).clone(), s(py, &note), obj(py, me.width)?.into_bound(py), rows, buttons.into_any()])
    }
}

impl Form {
    /// The batch rename's rows and error; with an error its default button
    /// greys out (role 129).
    fn wire_with_layout<'py>(slf: &Bound<'py, Self>, buttons: Bound<'py, PyList>, note: String) -> PyResult<Bound<'py, PyTuple>> {
        let py = slf.py();
        let me = slf.borrow();
        let layout = me.layout.bind(py);
        let (rows, error) = (layout.get_item(0)?, layout.get_item(1)?);
        if error.is_truthy()? && !buttons.is_empty() {
            let last = buttons.get_item(buttons.len() - 1)?;
            buttons.set_item(buttons.len() - 1, (last.get_item(0)?, last.get_item(1)?, last.get_item(2)?, 129))?;
        }
        PyTuple::new(py, [s(py, &me.seq), me.title.bind(py).clone(), s(py, &note), obj(py, me.width)?.into_bound(py), rows, buttons.into_any()])
    }
}

/// A form over a Rust operation model: its rows translated, its values,
/// the model's range check; `done(values)` on OK.
#[pyfunction]
#[pyo3(name = "OperationForm", signature = (title, model, done, *, note=None))]
fn operation_form<'py>(py: Python<'py>, title: Py<PyAny>, model: Py<PyAny>, done: Py<PyAny>, note: Option<Py<PyAny>>) -> PyResult<Bound<'py, Form>> {
    let m = model.clone_ref(py);
    let validate = func(py, move |py, _a| {
        let bounds = m.bind(py).call_method0("invalid_range")?;
        if bounds.is_none() {
            return Ok(none(py));
        }
        obj(py, trf(py, "Between {lo} and {hi}.", &[("lo", bounds.get_item(0)?), ("hi", bounds.get_item(1)?)])?)
    })?;
    let on_done = func(py, move |py, a| {
        if a.get_item(0)?.eq("ok")? {
            done.bind(py).call1((a.get_item(1)?,))?;
        }
        Ok(none(py))
    })?;
    let mut form = Form::make(py, title, PyList::empty(py).as_any(), note, None, obj(py, "ok")?, obj(py, "cancel")?, None, 480.0, Some(on_done), None, None, Some(validate), None)?;
    form.model = model;
    form.source = Source::Operation;
    Bound::new(py, form)
}

/// The Browser's batch rename: the Rust preview, its validated targets
/// committed on Rename.
#[pyfunction]
#[pyo3(name = "BrowserRenameForm")]
fn browser_rename_form<'py>(py: Python<'py>, window: Py<PyAny>, model: Py<PyAny>) -> PyResult<Bound<'py, Form>> {
    let m = model.clone_ref(py);
    let validate = func(py, move |py, _a| Ok(m.bind(py).call_method0("error")?.unbind()))?;
    let (m2, w) = (model.clone_ref(py), window.clone_ref(py));
    let on_done = func(py, move |py, a| {
        if a.get_item(0)?.eq("ok")? {
            m2.bind(py).call_method1("apply", (w.bind(py),))?;
        }
        Ok(none(py))
    })?;
    let buttons = PyList::new(py, [("cancel", tr(py, "Cancel")?), ("ok", tr(py, "Rename")?)])?;
    let mut form = Form::make(py, obj(py, tr(py, "Rename selected")?)?, PyList::empty(py).as_any(), None, Some(buttons.into_any()), obj(py, "ok")?, obj(py, "cancel")?, None, 520.0, Some(on_done), None, None, Some(validate), None)?;
    form.model = model;
    form.source = Source::Rename;
    Bound::new(py, form)
}

// ---- on the window ----------------------------------------------------------------------

#[pyfunction]
fn forms<'py>(win: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    win.getattr("__dict__")?.call_method1("setdefault", ("_chrome_forms", PyList::empty(win.py())))
}

#[pyfunction]
fn current<'py>(py: Python<'py>, win: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    if win.is_none() {
        return Ok(py.None().into_bound(py));
    }
    let stack = win.getattr("__dict__")?.call_method1("get", ("_chrome_forms",))?;
    if stack.is_truthy()? { stack.get_item(-1) } else { Ok(py.None().into_bound(py)) }
}

/// Release the window's input guards while its host is still alive.
#[pyfunction]
fn close_all(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let stack: Vec<Bound<'_, PyAny>> = forms(win)?.try_iter()?.collect::<PyResult<_>>()?;
    for form in stack.into_iter().rev() {
        form.call_method0("close")?;
    }
    Ok(())
}

/// Post a form, or cancel the operation if its host is unavailable.
#[pyfunction]
fn show<'py>(py: Python<'py>, parent: &Bound<'py, PyAny>, form: &Bound<'py, PyAny>) -> PyResult<&'static str> {
    let d = dialogs(py)?;
    let win = d.call_method1("async_host", (parent,))?;
    if win.is_none() {
        stderr(py, &format!("Built could not show the form: {}", form.getattr("title")?.str()?))?;
        form.call_method1("finish", (form.getattr("cancel")?,))?;
        return Ok("none");
    }
    let stack = forms(&win)?;
    let key = form.getattr("key")?;
    if !key.is_none() {
        let mut old = None;
        for f in stack.try_iter()? {
            let f = f?;
            if f.getattr("key")?.eq(&key)? {
                old = Some(f);
                break;
            }
        }
        if let Some(old) = old {
            stack.call_method1("remove", (&old,))?;
            stack.call_method1("append", (&old,))?; // raised, not opened twice
            forms_mod(py)?.call_method1("_repaint", (&win,))?;
            return Ok("chrome");
        }
    }
    form.setattr("win", &win)?;
    stack.call_method1("append", (form,))?;
    d.call_method1("engage", (&win,))?;
    let view = d.call_method1("live", (parent,))?;
    if !view.is_none() {
        watch(&view, &win, form)?;
    }
    forms_mod(py)?.call_method1("_repaint", (&win,))?;
    Ok("chrome")
}

#[pyfunction]
fn _remove(win: &Bound<'_, PyAny>, form: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let stack = forms(win)?;
    if stack.contains(form)? {
        stack.call_method1("remove", (form,))?;
        dialogs(py)?.call_method1("release", (win,))?;
    }
    _repaint(win)
}

#[pyfunction]
fn _repaint(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let view = dialogs(py)?.call_method1("view_of", (win,))?;
    if !view.is_none() {
        view.setattr("_ui_dialog_sig", py.None())?;
        view.call_method0("update")?;
    }
    Ok(())
}

/// Cancel a form that was never drawn (the Qt host).
fn watch(view: &Bound<'_, PyAny>, win: &Bound<'_, PyAny>, form: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = view.py();
    let d = dialogs(py)?;
    if d.call_method0("in_shell")?.is_truthy()? {
        return Ok(());
    }
    let frames = view.getattr("_ui_frames").unwrap_or_else(|_| obj(py, 0).expect("int").into_bound(py)).unbind();
    let (v, w, f) = (view.clone().unbind(), win.clone().unbind(), form.clone().unbind());
    let check = func(py, move |py, _a| {
        let (view, form) = (v.bind(py), f.bind(py));
        let now = view.getattr("_ui_frames").unwrap_or_else(|_| obj(py, 0).expect("int").into_bound(py));
        if form.getattr("closed")?.is_truthy()? || form.getattr("shown")?.is_truthy()? || !now.eq(frames.bind(py))? {
            return Ok(none(py));
        }
        w.bind(py).getattr("viewport")?.call_method1("flash_status", (format!("Built could not draw the {} form.", form.getattr("title")?.str()?),))?;
        form.call_method0("close")?;
        Ok(none(py))
    })?;
    py.import("PySide6.QtCore")?.getattr("QTimer")?.call_method1("singleShot", (d.getattr("WATCHDOG_MS")?, check))?;
    Ok(())
}

/// `dlg:<seq>\x1f<v|b|p>\x1f<id>[\x1f<value>]` from the chrome.
#[pyfunction]
fn action(py: Python<'_>, win: &Bound<'_, PyAny>, arg: &str) -> PyResult<()> {
    let (seq, rest) = arg.split_once('\x1f').unwrap_or((arg, ""));
    let form = current(py, win)?;
    if form.is_none() || !form.getattr("seq")?.eq(seq)? {
        return Ok(()); // a click for a form already gone
    }
    let (kind, rest) = rest.split_once('\x1f').unwrap_or((rest, ""));
    let (fid, value) = rest.split_once('\x1f').unwrap_or((rest, ""));
    match kind {
        "v" => {
            form.call_method1("receive", (fid, value))?;
        }
        "b" => {
            form.call_method1("press", (fid,))?;
        }
        "p" => {
            if let Ok(page) = fid.parse::<i64>() {
                form.setattr("page", page)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Send the form on top (only when it changed).
#[pyfunction]
fn before_frame(view: &Bound<'_, PyAny>, win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = view.py();
    let form = current(py, win)?;
    let renderer = view.getattr("renderer")?;
    let dict = view.getattr("__dict__")?;
    if form.is_none() {
        let empty = PyTuple::empty(py);
        if !dict.call_method1("get", ("_ui_dialog_sig", 0))?.eq(&empty)? {
            view.setattr("_ui_dialog_sig", empty)?;
            renderer.call_method1("ui_set_dialog", ("",))?;
        }
        return Ok(());
    }
    let wire = form.call_method0("wire")?.cast_into::<PyTuple>()?;
    let sig = PyTuple::new(py, [wire.clone().into_any(), form.getattr("version")?])?;
    if !sig.eq(dict.call_method1("get", ("_ui_dialog_sig",))?)? {
        view.setattr("_ui_dialog_sig", &sig)?;
        renderer.call_method1("ui_set_dialog", wire)?;
    }
    if let Some(marks) = opt(&form, "join_marks").filter(|m| m.is_truthy().unwrap_or(false)) {
        let vp = win.getattr("viewport")?;
        let display = opt(&vp, "keys_stand_in").filter(|d| d.is_truthy().unwrap_or(false)).unwrap_or_else(|| view.clone());
        let camera = vp.getattr("camera")?;
        let matrix = camera.call_method0("projection_matrix")?.mul(camera.call_method0("view_matrix")?)?;
        let origin = display.getattr("chrome_offset").unwrap_or_else(|_| PyTuple::new(py, [0, 0]).expect("tuple").into_any());
        renderer.call_method1("ui_set_join_marks", (form.getattr("seq")?, marks, matrix.call_method0("data")?, display.call_method0("width")?, display.call_method0("height")?, origin))?;
    }
    form.setattr("shown", true)?;
    Ok(())
}

/// A path field's Choose…: the app's file panels (Qt's in the Qt app, the
/// Mac's own in the shell).
#[pyfunction]
fn _pick_path(form: &Bound<'_, PyAny>, f: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = form.py();
    let dialogs = py.import("views.filedialogs")?.getattr("file_dialogs")?;
    let win = form.getattr("win")?;
    let parent = if win.hasattr("isWindow")? { win } else { py.None().into_bound(py) };
    let value = f.getattr("value")?;
    let current = if value.is_truthy()? { value.str()?.to_string() } else { String::new() };
    let label = f.getattr("label")?;
    let title = |fallback: &str| -> PyResult<String> { Ok(if label.is_truthy()? { label.str()?.to_string() } else { tr(py, fallback)? }) };
    let pick: String = f.getattr("pick")?.str()?.to_string();
    let got = match pick.as_str() {
        "folder" => dialogs.call_method1("getExistingDirectory", (&parent, title("Choose a folder")?, &current))?,
        "save" => dialogs.call_method1("getSaveFileName", (&parent, title("Save as")?, &current, f.getattr("filters")?))?.get_item(0)?,
        _ => dialogs.call_method1("getOpenFileName", (&parent, title("Open")?, &current, f.getattr("filters")?))?.get_item(0)?,
    };
    if got.is_truthy()? {
        f.setattr("value", &got)?;
        let on_change = form.getattr("on_change")?;
        if !on_change.is_none() {
            on_change.call1((form, f.getattr("id")?, &got))?;
        }
    }
    Ok(())
}

// ---- prompts --------------------------------------------------------------------------

fn answer_of(py: Python<'_>, done: Option<Py<PyAny>>, key: &'static str) -> PyResult<Py<PyAny>> {
    func(py, move |py, a| {
        if let Some(done) = &done {
            let v = if a.get_item(0)?.eq("ok")? { a.get_item(1)?.get_item(key)? } else { py.None().into_bound(py) };
            done.bind(py).call1((v,))?;
        }
        Ok(none(py))
    })
}

fn field<'py>(py: Python<'py>, args: (&str, &str, Bound<'py, PyAny>, Bound<'py, PyAny>), kw: &[(&str, Bound<'py, PyAny>)]) -> PyResult<Bound<'py, PyAny>> {
    let d = PyDict::new(py);
    for (k, v) in kw {
        d.set_item(*k, v)?;
    }
    forms_mod(py)?.getattr("Field")?.call(args, Some(&d))
}

fn post(py: Python<'_>, parent: &Bound<'_, PyAny>, form: Bound<'_, Form>) -> PyResult<()> {
    forms_mod(py)?.call_method1("show", (parent, form))?;
    Ok(())
}

/// A name prompt: `done(text)`, or `done(None)` on Cancel.
#[pyfunction]
#[pyo3(signature = (parent, title, label, text=None, done=None, *, multiline=false, ok=None, note=None))]
#[allow(clippy::too_many_arguments)]
fn ask_text(py: Python<'_>, parent: &Bound<'_, PyAny>, title: Py<PyAny>, label: &Bound<'_, PyAny>, text: Option<Bound<'_, PyAny>>, done: Option<Py<PyAny>>, multiline: bool, ok: Option<Py<PyAny>>, note: Option<Py<PyAny>>) -> PyResult<()> {
    let t = obj(py, true)?.into_bound(py);
    let ml = obj(py, multiline)?.into_bound(py);
    let f = field(py, ("text", "edit", label.clone(), text.unwrap_or_else(|| s(py, ""))), &[("focus", t), ("multiline", ml.clone()), ("full", ml)])?;
    let ok = match ok.filter(|o| o.bind(py).is_truthy().unwrap_or(false)) { Some(o) => o, None => obj(py, tr(py, "OK")?)? };
    let buttons = PyList::new(py, [(s(py, "cancel"), s(py, &tr(py, "Cancel")?)), (s(py, "ok"), ok.into_bound(py))])?;
    let form = Form::make(py, title, PyList::new(py, [f])?.as_any(), note, Some(buttons.into_any()), obj(py, "ok")?, obj(py, "cancel")?, None, if multiline { 520.0 } else { 440.0 }, Some(answer_of(py, done, "text")?), None, None, None, None)?;
    post(py, parent, Bound::new(py, form)?)
}

/// A number prompt: `done(value)`, or `done(None)` on Cancel.
#[pyfunction]
#[pyo3(signature = (parent, title, label, value, lo, hi, decimals=0, done=None, *, suffix=None, step=1, note=None))]
#[allow(clippy::too_many_arguments)]
fn ask_number(py: Python<'_>, parent: &Bound<'_, PyAny>, title: Py<PyAny>, label: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>, lo: f64, hi: f64, decimals: i64, done: Option<Py<PyAny>>, suffix: Option<Bound<'_, PyAny>>, step: i64, note: Option<Py<PyAny>>) -> PyResult<()> {
    let _ = step;
    let check = func(py, move |py, a| {
        let form = a.get_item(0)?;
        let raw = form.getattr("_texts")?.call_method1("get", ("value",))?;
        if !raw.is_none() {
            let Some(v) = parse_float(py, &raw.str()?.to_string()) else { return obj(py, tr(py, "Type a number.")?) };
            if !(lo <= v && v <= hi) {
                let (lo_t, hi_t) = (number_text(py, &obj(py, lo)?.into_bound(py), decimals)?, number_text(py, &obj(py, hi)?.into_bound(py), decimals)?);
                return obj(py, trf(py, "Between {lo} and {hi}.", &[("lo", s(py, &lo_t)), ("hi", s(py, &hi_t))])?);
            }
        }
        Ok(none(py))
    })?;
    let f = field(py, ("value", "number", label.clone(), value.clone()), &[
        ("lo", obj(py, lo)?.into_bound(py)), ("hi", obj(py, hi)?.into_bound(py)), ("decimals", obj(py, decimals)?.into_bound(py)),
        ("suffix", suffix.unwrap_or_else(|| s(py, ""))), ("focus", obj(py, true)?.into_bound(py)),
    ])?;
    let form = Form::make(py, title, PyList::new(py, [f])?.as_any(), note, None, obj(py, "ok")?, obj(py, "cancel")?, None, 400.0, Some(answer_of(py, done, "value")?), None, None, Some(check), None)?;
    post(py, parent, Bound::new(py, form)?)
}

/// A choice prompt: `done(item)`, or `done(None)` on Cancel.
#[pyfunction]
#[pyo3(signature = (parent, title, label, items, current=0, done=None, *, note=None))]
#[allow(clippy::too_many_arguments)]
fn ask_choice(py: Python<'_>, parent: &Bound<'_, PyAny>, title: Py<PyAny>, label: &Bound<'_, PyAny>, items: &Bound<'_, PyAny>, current: i64, done: Option<Py<PyAny>>, note: Option<Py<PyAny>>) -> PyResult<()> {
    let items: Vec<String> = items.try_iter()?.map(|i| Ok(i?.str()?.to_string())).collect::<PyResult<_>>()?;
    let start = if current >= 0 && (current as usize) < items.len() { Some(items[current as usize].clone()) } else { items.first().cloned() };
    let f = field(py, ("item", "choice", label.clone(), obj(py, start)?.into_bound(py)), &[("options", PyList::new(py, &items)?.into_any())])?;
    let form = Form::make(py, title, PyList::new(py, [f])?.as_any(), note, None, obj(py, "ok")?, obj(py, "cancel")?, None, 420.0, Some(answer_of(py, done, "item")?), None, None, None, None)?;
    post(py, parent, Bound::new(py, form)?)
}

// ---- steps that read top to bottom --------------------------------------------------

/// `yield request(ask_text, parent, title, label, text=…)`: the call, with
/// `done` left for `flow` to fill in.
#[pyclass(module = "ingetrazo_render.chrome_forms", name = "request")]
struct Request {
    f: Py<PyAny>,
    args: Py<PyTuple>,
    kwargs: Py<PyDict>,
}

#[pymethods]
impl Request {
    #[new]
    #[pyo3(signature = (f, *args, **kwargs))]
    fn new(py: Python<'_>, f: Py<PyAny>, args: Py<PyTuple>, kwargs: Option<Bound<'_, PyDict>>) -> Self {
        Request { f, args, kwargs: kwargs.map_or_else(|| PyDict::new(py).unbind(), Bound::unbind) }
    }

    fn start(&self, py: Python<'_>, resume: &Bound<'_, PyAny>) -> PyResult<()> {
        let kw = self.kwargs.bind(py).copy()?;
        kw.set_item("done", resume)?;
        self.f.bind(py).call(self.args.bind(py), Some(&kw))?;
        Ok(())
    }
}

/// One step of a flow: send the answer in, start the next request.
fn step(py: Python<'_>, gen: &Py<PyAny>, value: Option<Bound<'_, PyAny>>) -> PyResult<()> {
    let g = gen.bind(py);
    let next = match value {
        Some(v) => g.call_method1("send", (v,)),
        None => py.import("builtins")?.getattr("next")?.call1((g,)),
    };
    let req = match next {
        Ok(r) => r,
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyStopIteration>(py) => return Ok(()),
        Err(e) => return Err(e),
    };
    let gen2 = gen.clone_ref(py);
    let resume = func(py, move |py, a| {
        let v = if a.is_empty() { py.None().into_bound(py) } else { a.get_item(0)? };
        step(py, &gen2, Some(v))?;
        Ok(none(py))
    })?;
    req.call_method1("start", (resume,))?;
    Ok(())
}

/// Run a generator of `request`: each answer is sent back in, on the turn
/// the chrome delivers it.
#[pyfunction]
fn flow(py: Python<'_>, gen: Py<PyAny>) -> PyResult<()> {
    step(py, &gen, None)
}

// ---- the first-run units question (ingetrazo_render.units_question) -------------------

fn units(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("core.units")
}

/// The units editor's rows for `model` (a `UnitsDraft`).
#[pyfunction]
fn model_rows<'py>(py: Python<'py>, model: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let u = units(py)?;
    let settings = py.import("json")?.call_method1("loads", (model.call_method0("settings")?,))?;
    let length = settings.get_item("length")?;
    let mut auto = Vec::new();
    for name in ["area_unit_for", "volume_unit_for"] {
        let unit = u.getattr("UNITS")?.get_item(u.call_method1(name, (&length,))?)?;
        auto.push(trf(py, "By the length ({unit})", &[("unit", unit.getattr("symbol")?)])?);
    }
    let tr_fn = py.import("core.i18n")?.getattr("tr")?;
    model.call_method1("rows", (tr_fn, u.getattr("unit_label")?, u.call_method1("preview_text", (&settings,))?, PyTuple::new(py, auto)?, u.getattr("FRACTION_LABELS")?))
}

/// A `UnitsDraft` for a scene: new-document defaults, the model's units,
/// the profiles and the choices.
#[pyfunction]
fn draft<'py>(py: Python<'py>, scene: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let u = units(py)?;
    let json = py.import("json")?;
    let dumps = |v: Bound<'py, PyAny>| json.call_method1("dumps", (v,));
    let choices = PyList::new(py, [u.getattr("UNIT_CHOICES")?, u.getattr("AREA_CHOICES")?, u.getattr("VOLUME_CHOICES")?, u.getattr("ANGLE_CHOICES")?])?;
    py.import("ingetrazo_render")?.getattr("UnitsDraft")?.call1((
        dumps(u.call_method0("new_document_units")?)?,
        dumps(u.call_method1("model_units_of", (scene,))?)?,
        dumps(u.getattr("PROFILES")?)?,
        choices,
    ))
}

/// The custom units form over `model`.
#[pyfunction]
#[pyo3(name = "UnitsQuestion")]
fn units_form<'py>(py: Python<'py>, model: Py<PyAny>, done: Py<PyAny>) -> PyResult<Bound<'py, Form>> {
    let note: String = model.bind(py).getattr("note")?.str()?.to_string();
    let mut form = Form::make(py, obj(py, tr(py, "Units")?)?, PyList::empty(py).as_any(), Some(obj(py, tr(py, &note)?)?), None, obj(py, "ok")?, obj(py, "cancel")?, None, 520.0, Some(done), None, None, None, Some(obj(py, "start_units")?))?;
    form.model = model;
    form.source = Source::Units;
    Bound::new(py, form)
}

/// The first-run question: which units this new model uses (and, custom,
/// the full editor).
#[pyfunction]
fn offer_at_start(py: Python<'_>, window: &Bound<'_, PyAny>) -> PyResult<()> {
    let u = units(py)?;
    if !u.call_method0("ask_at_start")?.is_truthy()? {
        return Ok(());
    }
    let view = window.getattr("viewport")?;
    let scene = view.getattr("scene")?;
    let model = draft(py, &scene)?;
    let done = Bound::new(py, UnitsDone { window: window.clone().unbind(), view: view.unbind(), scene: scene.unbind(), model: model.clone().unbind() })?;
    let form = units_form(py, model.unbind(), done.into_any().unbind())?;
    forms_mod(py)?.call_method1("show", (window, form))?;
    Ok(())
}

/// The units question answered.
#[pyclass(module = "ingetrazo_render.units_question")]
struct UnitsDone {
    window: Py<PyAny>,
    view: Py<PyAny>,
    scene: Py<PyAny>,
    model: Py<PyAny>,
}

#[pymethods]
impl UnitsDone {
    fn __call__(slf: &Bound<'_, Self>, answer: &str, _values: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = slf.py();
        let me = slf.borrow();
        let (window, view, scene, model) = (me.window.bind(py), me.view.bind(py), me.scene.bind(py), me.model.bind(py));
        if !window.getattr("viewport")?.is(view) || !view.getattr("scene")?.is(scene) {
            return Ok(());
        }
        let u = units(py)?;
        let custom = model.getattr("custom")?.is_truthy()?;
        let operation: String = model.call_method1("answer", (answer == "ok",))?.extract()?;
        if !custom && !model.getattr("ask_again")?.is_truthy()? {
            u.call_method1("set_ask_at_start", (false,))?;
        }
        if operation == "custom" {
            let form = units_form(py, model.clone().unbind(), slf.clone().into_any().unbind())?;
            forms_mod(py)?.call_method1("show", (window, form))?;
        } else if operation == "apply" {
            let chosen = py.import("json")?.call_method1("loads", (model.call_method0("settings")?,))?;
            if custom {
                let untouched = scene.getattr("version")?.eq(window.getattr("_saved_version")?)?;
                let apply = py.import("views.settings_apply")?;
                apply.call_method1("apply_document_units", (window, &chosen))?;
                if untouched {
                    window.setattr("_saved_version", scene.getattr("version")?)?;
                    window.call_method0("_update_title")?;
                }
                let key = apply.getattr("_UNITS_FOR_NEW_KEY")?;
                let for_new = if model.getattr("for_new")?.is_truthy()? { "1" } else { "0" };
                py.import("core.settings")?.call_method0("settings")?.call_method1("setValue", (key, for_new))?;
                u.call_method1("set_ask_at_start", (model.getattr("ask_again")?,))?;
            } else {
                u.call_method1("apply_units", (scene, &chosen))?;
                view.call_method0("update")?;
            }
            if model.getattr("for_new")?.is_truthy()? {
                u.call_method1("remember_new_document_units", (chosen,))?;
            }
        }
        Ok(())
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    let sub = PyModule::new(py, "chrome_forms")?;
    for (name, v) in [("DISABLED", DISABLED), ("FOCUS", FOCUS), ("MULTILINE", MULTILINE), ("PASSWORD", PASSWORD), ("FULL", FULL), ("COMMIT_ON_BLUR", COMMIT_ON_BLUR)] {
        sub.add(name, v)?;
    }
    sub.add_class::<Field>()?;
    sub.add_class::<Form>()?;
    sub.add_class::<Request>()?;
    for f in [
        wrap_pyfunction!(number_text, &sub)?,
        wrap_pyfunction!(operation_form, &sub)?,
        wrap_pyfunction!(browser_rename_form, &sub)?,
        wrap_pyfunction!(forms, &sub)?,
        wrap_pyfunction!(current, &sub)?,
        wrap_pyfunction!(close_all, &sub)?,
        wrap_pyfunction!(show, &sub)?,
        wrap_pyfunction!(_remove, &sub)?,
        wrap_pyfunction!(_repaint, &sub)?,
        wrap_pyfunction!(action, &sub)?,
        wrap_pyfunction!(before_frame, &sub)?,
        wrap_pyfunction!(_pick_path, &sub)?,
        wrap_pyfunction!(ask_text, &sub)?,
        wrap_pyfunction!(ask_number, &sub)?,
        wrap_pyfunction!(ask_choice, &sub)?,
        wrap_pyfunction!(flow, &sub)?,
    ] {
        sub.add_function(f)?;
    }
    m.add_submodule(&sub)?;
    py.import("sys")?.getattr("modules")?.set_item("ingetrazo_render.chrome_forms", &sub)?;

    let units_sub = PyModule::new(py, "units_question")?;
    units_sub.add_class::<UnitsDone>()?;
    for f in [wrap_pyfunction!(model_rows, &units_sub)?, wrap_pyfunction!(draft, &units_sub)?, wrap_pyfunction!(units_form, &units_sub)?, wrap_pyfunction!(offer_at_start, &units_sub)?] {
        units_sub.add_function(f)?;
    }
    m.add_submodule(&units_sub)?;
    py.import("sys")?.getattr("modules")?.set_item("ingetrazo_render.units_question", &units_sub)?;
    Ok(())
}
