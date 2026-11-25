// SPDX-License-Identifier: GPL-3.0-or-later
//! Settings' rows (`ingetrazo_render.settings_sheet`): what each setting
//! reads and how it applies, section by section.
//!
//! `preferences.rs` owns the application rows (catalogue, constraints,
//! defaults: `PreferencePage`), the 3D mouse model (`DevicePreferences`) and
//! the Settings state (`PreferencesView`). Here each row is bound to where
//! its value lives: the native preference store (`env.st`), the window's
//! appliers in `settings_rows/live.rs`, or the document (units, dimension
//! style, north and location, through the protocol and its history).
mod appearance;
pub(crate) mod live;

use pyo3::{
    prelude::*,
    types::{IntoPyDict, PyCFunction, PyDict, PyList, PyTuple},
};

pub type Res = PyResult<Py<PyAny>>;

pub fn opt<'py>(o: &Bound<'py, PyAny>, name: &str) -> Option<Bound<'py, PyAny>> {
    o.getattr(name).ok().filter(|v| !v.is_none())
}

/// A Python callable made of a Rust closure.
pub fn func<F>(py: Python<'_>, f: F) -> Res
where
    F: Fn(Python<'_>, &Bound<'_, PyTuple>) -> Res + Send + Sync + 'static,
{
    Ok(PyCFunction::new_closure(py, None, None, move |args: &Bound<'_, PyTuple>, _kw: Option<&Bound<'_, PyDict>>| f(args.py(), args))?
        .into_any()
        .unbind())
}

pub fn none(py: Python<'_>) -> Py<PyAny> {
    py.None()
}

pub fn obj<'py, T: IntoPyObject<'py>>(py: Python<'py>, v: T) -> Res {
    use pyo3::BoundObject;
    Ok(v.into_pyobject(py).map_err(Into::into)?.into_any().unbind())
}

/// `tr(text)`.
pub fn tr(py: Python<'_>, text: &str) -> PyResult<String> {
    py.import("core.i18n")?.call_method1("tr", (text,))?.extract()
}

/// `tr(text, **kw)`.
pub fn trf(py: Python<'_>, text: &str, kw: &[(&str, Bound<'_, PyAny>)]) -> PyResult<String> {
    let d = PyDict::new(py);
    for (k, v) in kw {
        d.set_item(k, v)?;
    }
    py.import("core.i18n")?.getattr("tr")?.call((text,), Some(&d))?.extract()
}

pub fn s<'py>(py: Python<'py>, v: &str) -> Bound<'py, PyAny> {
    pyo3::types::PyString::new(py, v).into_any()
}

/// Live appliers on the native settings sheet.
pub fn apply(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("ingetrazo_render.settings_sheet")
}

/// The sentinel for "no default" (`settings_sheet._NO`).
pub fn no(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("ingetrazo_render")?.getattr("settings_sheet")?.getattr("_NO")
}

pub fn is_no(v: &Bound<'_, PyAny>) -> bool {
    no(v.py()).is_ok_and(|n| n.is(v))
}

/// `_b`: a stored truth ("0", "false" and "" are no).
pub fn truth(raw: &Bound<'_, PyAny>, default: &str) -> PyResult<bool> {
    let text = if raw.is_none() { default.to_owned() } else { raw.str()?.to_string() };
    Ok(!matches!(text.to_lowercase().as_str(), "0" | "false" | ""))
}

/// `_int`: an integer from what is stored, else `default`.
pub fn int_of(raw: &Bound<'_, PyAny>, default: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = raw.py();
    let builtins = py.import("builtins")?;
    match builtins.getattr("float")?.call1((raw,)).and_then(|f| builtins.getattr("int")?.call1((f,))) {
        Ok(v) => Ok(v.unbind()),
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyTypeError>(py) || e.is_instance_of::<pyo3::exceptions::PyValueError>(py) => Ok(default.clone().unbind()),
        Err(e) => Err(e),
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Str,
    Bool,
    Int,
}

fn shaped(raw: &Bound<'_, PyAny>, default: &Bound<'_, PyAny>, kind: Kind, bool_default: &str) -> Res {
    let py = raw.py();
    match kind {
        Kind::Bool => obj(py, truth(raw, bool_default)?),
        Kind::Int => int_of(raw, default),
        Kind::Str => Ok((if raw.is_none() { default.str()? } else { raw.str()? }).into_any().unbind()),
    }
}

/// `_key`: a getter for a plain preference key.
pub fn key_get(py: Python<'_>, key: &'static str, default: Py<PyAny>, kind: Kind) -> Res {
    func(py, move |py, a| {
        let env = a.get_item(0)?;
        let default = default.bind(py);
        let raw = env.getattr("st")?.call_method1("value", (key, default))?;
        let flag = if default.is_truthy()? && (default.eq(true)? || default.eq("1")?) { "1" } else { "0" };
        shaped(&raw, default, kind, flag)
    })
}

/// `_set_key`: store the value (bools as "1"/"0") and sync.
pub fn key_set(py: Python<'_>, key: &'static str, kind: Kind) -> Res {
    func(py, move |py, a| {
        let (env, value) = (a.get_item(0)?, a.get_item(1)?);
        let st = env.getattr("st")?;
        let stored = if kind == Kind::Bool { s(py, if value.is_truthy()? { "1" } else { "0" }) } else { value };
        st.call_method1("setValue", (key, stored))?;
        st.call_method0("sync")?;
        Ok(none(py))
    })
}

/// `_gkey`: a getter for a `core.general_settings` key.
pub fn gkey_get(py: Python<'_>, name: &'static str, default: Py<PyAny>, kind: Kind) -> Res {
    func(py, move |py, a| gkey_value(&a.get_item(0)?, name, default.bind(py), kind))
}

pub fn gkey_value(env: &Bound<'_, PyAny>, name: &str, default: &Bound<'_, PyAny>, kind: Kind) -> Res {
    let py = env.py();
    let raw = py.import("core.general_settings")?.call_method1("value", (name, default, env.getattr("st")?))?;
    shaped(&raw, default, kind, &default.str()?.to_string())
}

// ---- rows -----------------------------------------------------------------------------

/// One setting: how to read it, how to apply it, how to draw it. `get(env)`
/// is the value; `put(env, value)` applies it at once. `options`,
/// `enabled` and `visible` may be callables of `env`. `restart`: what the
/// restart note names when the value changed; `default`: what Restore
/// defaults puts back (`_NO`: left alone).
#[pyclass(module = "ingetrazo_render.settings_sheet", dict, get_all, set_all)]
pub struct Row {
    pub id: Py<PyAny>,
    pub kind: Py<PyAny>,
    pub label: Py<PyAny>,
    pub get: Py<PyAny>,
    pub put: Py<PyAny>,
    pub press: Py<PyAny>,
    pub options: Py<PyAny>,
    pub lo: Py<PyAny>,
    pub hi: Py<PyAny>,
    pub decimals: Py<PyAny>,
    pub suffix: Py<PyAny>,
    pub tip: Py<PyAny>,
    pub restart: Py<PyAny>,
    pub default: Py<PyAny>,
    pub enabled: Py<PyAny>,
    pub visible: Py<PyAny>,
    pub row_label: Py<PyAny>,
    pub words: Py<PyAny>,
    pub pick: Py<PyAny>,
    pub filters: Py<PyAny>,
    pub password: Py<PyAny>,
    pub fmt: Py<PyAny>,
    pub restart_value: Py<PyAny>,
    pub start: Py<PyAny>,
}

const ROW_KEYS: [&str; 18] = ["options", "lo", "hi", "decimals", "suffix", "tip", "restart", "default", "enabled", "visible",
    "row_label", "words", "pick", "filters", "password", "fmt", "restart_value", "start"];

impl Row {
    fn call_env(&self, py: Python<'_>, f: &Py<PyAny>, env: &Bound<'_, PyAny>) -> Res {
        Ok(f.bind(py).call1((env,))?.unbind())
    }
}

#[pymethods]
impl Row {
    #[new]
    #[pyo3(signature = (rid, kind, label, get=None, put=None, **kw))]
    fn new(py: Python<'_>, rid: Py<PyAny>, kind: Py<PyAny>, label: Py<PyAny>, get: Option<Py<PyAny>>, put: Option<Py<PyAny>>, kw: Option<&Bound<'_, PyDict>>) -> PyResult<Self> {
        let e = || obj(py, "");
        let mut row = Row {
            id: rid,
            kind,
            label,
            get: get.unwrap_or_else(|| none(py)),
            put: put.unwrap_or_else(|| none(py)),
            press: none(py),
            options: PyTuple::empty(py).into_any().unbind(),
            lo: obj(py, 0.0)?,
            hi: obj(py, 0.0)?,
            decimals: obj(py, 0)?,
            suffix: e()?,
            tip: e()?,
            restart: e()?,
            default: no(py)?.unbind(),
            enabled: obj(py, true)?,
            visible: obj(py, true)?,
            row_label: e()?,
            words: e()?,
            pick: none(py),
            filters: e()?,
            password: obj(py, false)?,
            fmt: none(py),
            restart_value: none(py),
            start: none(py),
        };
        if let Some(kw) = kw {
            for (k, v) in kw.iter() {
                let name: String = k.extract()?;
                let v = v.unbind();
                match name.as_str() {
                    "press" => row.press = v,
                    "options" => row.options = v,
                    "lo" => row.lo = v,
                    "hi" => row.hi = v,
                    "decimals" => row.decimals = v,
                    "suffix" => row.suffix = v,
                    "tip" => row.tip = v,
                    "restart" => row.restart = v,
                    "default" => row.default = v,
                    "enabled" => row.enabled = v,
                    "visible" => row.visible = v,
                    "row_label" => row.row_label = v,
                    "words" => row.words = v,
                    "pick" => row.pick = v,
                    "filters" => row.filters = v,
                    "password" => row.password = v,
                    "fmt" => row.fmt = v,
                    "restart_value" => row.restart_value = v,
                    "start" => row.start = v,
                    other => return Err(pyo3::exceptions::PyTypeError::new_err(format!("Row() got an unexpected keyword argument '{other}'"))),
                }
            }
        }
        let _ = ROW_KEYS;
        Ok(row)
    }

    /// `[(id, label)]`, from a list or a callable of `env`.
    fn opts<'py>(&self, py: Python<'py>, env: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
        let o = self.options.bind(py);
        let o = if o.is_callable() { o.call1((env,))? } else { o.clone() };
        let out = PyList::empty(py);
        for x in o.try_iter()? {
            let x = x?;
            if x.is_instance_of::<pyo3::types::PyString>() {
                out.append((&x, &x))?;
            } else {
                out.append((x.get_item(0)?, x.get_item(1)?))?;
            }
        }
        Ok(out)
    }

    fn on(&self, py: Python<'_>, attr: &str, env: &Bound<'_, PyAny>) -> PyResult<bool> {
        let v = match attr {
            "enabled" => self.enabled.bind(py),
            "visible" => self.visible.bind(py),
            "password" => self.password.bind(py),
            _ => return Err(pyo3::exceptions::PyAttributeError::new_err(attr.to_owned())),
        };
        if v.is_callable() { v.call1((env,))?.is_truthy() } else { v.is_truthy() }
    }

    fn value(&self, py: Python<'_>, env: &Bound<'_, PyAny>) -> Res {
        if self.get.is_none(py) { Ok(none(py)) } else { self.call_env(py, &self.get, env) }
    }

    /// The row as a `ingetrazo_render.chrome_forms.Field`.
    fn field<'py>(&self, py: Python<'py>, env: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let kind: String = self.kind.extract(py)?;
        let mut value = self.value(py, env)?.into_bound(py);
        let label = self.label.bind(py);
        if kind == "text" && label.is_truthy()? && value.is_truthy()? {
            value = s(py, &format!("{}: {}", label.str()?, value.str()?)); // a text row draws its text only
        }
        let kw = PyDict::new(py);
        kw.set_item("options", if kind == "choice" { self.opts(py, env)?.into_any() } else { PyTuple::empty(py).into_any() })?;
        kw.set_item("lo", &self.lo)?;
        kw.set_item("hi", &self.hi)?;
        kw.set_item("decimals", &self.decimals)?;
        kw.set_item("suffix", &self.suffix)?;
        kw.set_item("tip", &self.tip)?;
        kw.set_item("enabled", self.on(py, "enabled", env)?)?;
        kw.set_item("row_label", &self.row_label)?;
        kw.set_item("password", &self.password)?;
        kw.set_item("pick", &self.pick)?;
        kw.set_item("filters", &self.filters)?;
        kw.set_item("fmt", &self.fmt)?;
        let note = if kind == "button" && !self.get.is_none(py) { self.value(py, env)?.into_bound(py) } else { s(py, "") };
        kw.set_item("note", note)?;
        py.import("ingetrazo_render.chrome_forms")?.getattr("Field")?.call((&self.id, &self.kind, label, value), Some(&kw))
    }

    /// The words a search matches the row on.
    fn haystack(&self, py: Python<'_>, env: &Bound<'_, PyAny>, section: &Bound<'_, PyAny>, group: &Bound<'_, PyAny>) -> PyResult<String> {
        let mut parts: Vec<Bound<'_, PyAny>> = vec![section.clone(), group.clone(), self.label.bind(py).clone(), self.row_label.bind(py).clone(), self.tip.bind(py).clone(), self.words.bind(py).clone()];
        if self.kind.extract::<String>(py)? == "choice" {
            for o in self.opts(py, env)?.iter() {
                parts.push(o.get_item(1)?);
            }
        }
        let mut words = Vec::new();
        for p in parts {
            if p.is_truthy()? {
                words.push(p.str()?.to_string());
            }
        }
        Ok(words.join(" ").to_lowercase())
    }
}

pub fn row<'py>(py: Python<'py>, rid: &str, kind: &str, label: &str, get: Option<Py<PyAny>>, put: Option<Py<PyAny>>, kw: &[(&str, Py<PyAny>)]) -> PyResult<Bound<'py, PyAny>> {
    let d = PyDict::new(py);
    for (k, v) in kw {
        d.set_item(*k, v)?;
    }
    py.import("ingetrazo_render")?.getattr("settings_sheet")?.getattr("Row")?.call((rid, kind, label, get, put), Some(&d))
}

/// What a row's getter and setter see: the window, its viewport and scene,
/// and one preference store for the whole build or change.
#[pyclass(module = "ingetrazo_render.settings_sheet", dict, get_all, set_all)]
pub struct Env {
    pub win: Py<PyAny>,
    pub st: Py<PyAny>,
    pub vp: Py<PyAny>,
    pub scene: Py<PyAny>,
}

#[pymethods]
impl Env {
    #[new]
    fn new(py: Python<'_>, win: &Bound<'_, PyAny>) -> PyResult<Self> {
        let st = py.import("ingetrazo_core.settings")?.call_method0("settings")?;
        let vp = opt(win, "viewport");
        let scene = vp.as_ref().and_then(|v| opt(v, "scene"));
        Ok(Env { win: win.clone().unbind(), st: st.unbind(), vp: vp.map_or_else(|| none(py), Bound::unbind), scene: scene.map_or_else(|| none(py), Bound::unbind) })
    }
}

pub fn env<'py>(win: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    Ok(Bound::new(win.py(), Env::new(win.py(), win)?)?.into_any())
}

// ---- Built · this Mac ---------------------------------------------------------------

/// Read and apply one Rust-defined preference through its owning subsystem.
pub struct Binding {
    key: &'static str,
    read: Py<PyAny>,
    write: Py<PyAny>,
    fmt: Option<Py<PyAny>>,
    start: Option<Py<PyAny>>,
    restart_value: Option<Py<PyAny>>,
    host: Vec<(&'static str, Py<PyAny>)>,
}

impl Binding {
    pub fn new(key: &'static str, read: Py<PyAny>, write: Py<PyAny>) -> Self {
        Binding { key, read, write, fmt: None, start: None, restart_value: None, host: Vec::new() }
    }

    fn with(mut self, k: &'static str, v: Py<PyAny>) -> Self {
        self.host.push((k, v));
        self
    }

    fn snapshot<'py>(&self, py: Python<'py>, env: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("value", self.read.bind(py).call1((env,))?)?;
        for (k, item) in &self.host {
            let item = item.bind(py);
            let value = if item.is_callable() { item.call1((env,))? } else { item.clone() };
            if !is_no(&value) {
                d.set_item(*k, value)?;
            }
        }
        Ok(d)
    }
}

/// Bind the sheet's field callbacks to rows returned by Rust.
pub fn native_groups<'py>(py: Python<'py>, spec: &str, put: Py<PyAny>) -> PyResult<Bound<'py, PyList>> {
    let groups = py.import("json")?.call_method1("loads", (spec,))?;
    let out = PyList::empty(py);
    let row_cls = py.import("ingetrazo_render")?.getattr("settings_sheet")?.getattr("Row")?;
    for g in groups.try_iter()? {
        let g = g?;
        let rows = PyList::empty(py);
        for data in g.get_item(2)?.try_iter()? {
            let data = data?.cast_into::<PyDict>()?;
            let key = data.get_item("id")?.expect("row id");
            data.del_item("id")?;
            let kind = data.get_item("kind")?.expect("row kind");
            data.del_item("kind")?;
            let value = data.get_item("value")?.map_or_else(|| none(py), Bound::unbind);
            data.del_item("value").ok();
            let label = data.get_item("label")?.map_or_else(|| s(py, ""), |l| l);
            data.del_item("label").ok();
            let get = func(py, move |py, _a| Ok(value.clone_ref(py)))?;
            let put = if kind.eq("text")? {
                none(py)
            } else {
                let (put, key) = (put.clone_ref(py), key.clone().unbind());
                func(py, move |py, a| Ok(put.bind(py).call1((a.get_item(0)?, key.bind(py), a.get_item(1)?))?.unbind()))?
            };
            rows.append(row_cls.call((&key, &kind, label, get, put), Some(&data))?)?;
        }
        out.append((g.get_item(0)?, g.get_item(1)?, rows))?;
    }
    Ok(out)
}

/// The rows of an application section from its `PreferencePage`, each
/// bound to its subsystem.
fn pref_page<'py>(py: Python<'py>, env: &Bound<'py, PyAny>, section: &str, bindings: Vec<Binding>) -> PyResult<Bound<'py, PyList>> {
    let snapshot = PyDict::new(py);
    for b in &bindings {
        snapshot.set_item(b.key, b.snapshot(py, env)?)?;
    }
    let json = py.import("json")?;
    let text: String = json.call_method1("dumps", (snapshot,))?.extract()?;
    let tr_fn = py.import("core.i18n")?.getattr("tr")?;
    let page = py.import("ingetrazo_render")?.getattr("PreferencePage")?.call1((section, text, &tr_fn))?;
    let writers: Vec<(&'static str, Py<PyAny>)> = bindings.iter().map(|b| (b.key, b.write.clone_ref(py))).collect();
    let page_ref = page.clone().unbind();
    let put = func(py, move |py, a| {
        let (env, key, value) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?);
        let k: String = key.extract()?;
        let json = py.import("json")?;
        let edited = page_ref.bind(py).call_method1("edit", (&key, json.call_method1("dumps", (value,))?))?;
        let value = json.call_method1("loads", (edited,))?;
        let write = writers.iter().find(|(name, _)| *name == k).map(|(_, w)| w.clone_ref(py)).ok_or_else(|| pyo3::exceptions::PyKeyError::new_err(k.clone()))?;
        write.bind(py).call1((env, value))?;
        Ok(none(py))
    })?;
    let groups = native_groups(py, &page.call_method0("groups")?.extract::<String>()?, put)?;
    for g in groups.iter() {
        for r in g.get_item(2)?.try_iter()? {
            let r = r?;
            let id: String = r.getattr("id")?.extract()?;
            if let Some(b) = bindings.iter().find(|b| b.key == id) {
                let or_none = |v: &Option<Py<PyAny>>| v.as_ref().map_or_else(|| none(py), |v| v.clone_ref(py));
                r.setattr("fmt", or_none(&b.fmt))?;
                r.setattr("start", or_none(&b.start))?;
                r.setattr("restart_value", or_none(&b.restart_value))?;
            }
        }
    }
    Ok(groups)
}

/// `(env, value) → applier(win, value, st); sync`, the common shape.
fn applier(py: Python<'_>, name: &'static str) -> Res {
    func(py, move |py, a| {
        let (env, value) = (a.get_item(0)?, a.get_item(1)?);
        let st = env.getattr("st")?;
        apply(py)?.call_method1(name, (env.getattr("win")?, value, &st))?;
        st.call_method0("sync")?;
        Ok(none(py))
    })
}

fn ai<'py>(py: Python<'py>, env: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    let ai = py.import("core.ai")?;
    let binding = |key: &'static str, default: &str| -> PyResult<Binding> {
        let detect = ai.getattr("detect_provider")?.unbind();
        let write = func(py, move |py, a| {
            let (env, value) = (a.get_item(0)?, a.get_item(1)?);
            let value: String = value.str()?.to_string();
            crate::preferences::edit_ai_connection(&env.getattr("st")?, key, &value, detect.bind(py))?;
            Ok(none(py))
        })?;
        Ok(Binding::new(key, key_get(py, key, obj(py, default)?, Kind::Str)?, write))
    };
    let options = PyList::new(py, [(s(py, "auto"), s(py, &tr(py, "Auto (by key prefix)")?))])?;
    let info = ai.getattr("PROVIDER_INFO")?;
    for k in ai.getattr("PROVIDERS")?.try_iter()? {
        let k = k?;
        options.append((&k, info.get_item(&k)?.get_item(0)?))?;
    }
    let provider = binding("ia/proveedor", "auto")?.with("options", options.into_any().unbind());
    pref_page(py, env, "ai", vec![provider, binding("ia/modelo", "")?, binding("ia/api_key", "")?, binding("ia/ollama_url", "http://localhost:11434")?])
}

fn autosave_put(py: Python<'_>, which: &'static str) -> Res {
    func(py, move |py, a| {
        let (env, value) = (a.get_item(0)?, a.get_item(1)?);
        let on = if which == "on" { value.clone().unbind() } else { gkey_value(&env, "autosave", &s(py, "1"), Kind::Bool)? };
        let minutes = if which == "minutes" { value.clone().unbind() } else { gkey_value(&env, "autosave_min", &5i32.into_pyobject(py)?.into_any(), Kind::Int)? };
        let backup = if which == "backup" { value.unbind() } else { none(py) };
        let st = env.getattr("st")?;
        apply(py)?.call_method1("apply_autosave", (env.getattr("win")?, on, minutes, backup, &st))?;
        st.call_method0("sync")?;
        Ok(none(py))
    })
}

fn general<'py>(py: Python<'py>, env: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    let i18n = py.import("core.i18n")?;
    let system = i18n.getattr("SYSTEM")?;
    let names = i18n.getattr("LANGUAGE_NAMES")?;
    let options = PyList::new(py, [(system.clone(), s(py, &tr(py, "Same as the system")?))])?;
    for c in i18n.call_method0("available_languages")?.try_iter()? {
        let c = c?;
        options.append((&c, names.call_method1("get", (&c, &c))?))?;
    }
    let mut language = Binding::new(
        "language",
        key_get(py, "language", system.unbind(), Kind::Str)?,
        func(py, |py, a| {
            let (env, value) = (a.get_item(0)?, a.get_item(1)?);
            apply(py)?.call_method1("apply_language", (value, env.getattr("st")?))?;
            Ok(none(py))
        })?,
    )
    .with("options", options.into_any().unbind());
    language.restart_value = Some(func(py, |py, a| Ok(py.import("core.i18n")?.call_method1("resolve_language", (a.get_item(0)?,))?.unbind()))?);
    language.start = Some(func(py, |py, _a| Ok(py.import("core.i18n")?.call_method0("current_language")?.unbind()))?);
    let mut rows = vec![
        language,
        Binding::new("hover_picture", key_get(py, "tabs/hover_picture", obj(py, "1")?, Kind::Bool)?, key_set(py, "tabs/hover_picture", Kind::Bool)?),
        Binding::new("autosave", gkey_get(py, "autosave", obj(py, "1")?, Kind::Bool)?, autosave_put(py, "on")?),
        Binding::new("autosave_min", gkey_get(py, "autosave_min", obj(py, 5)?, Kind::Int)?, autosave_put(py, "minutes")?),
        Binding::new("backup", gkey_get(py, "backup", obj(py, "1")?, Kind::Bool)?, autosave_put(py, "backup")?),
        Binding::new("undo_steps", gkey_get(py, "undo_steps", obj(py, 200)?, Kind::Int)?, applier(py, "apply_undo_steps")?),
    ];
    let update = py.import("ingetrazo_render.release_check")?;
    if !update.call_method0("store_managed")?.is_truthy()? {
        let key: String = update.getattr("KEY_AUTO")?.extract()?;
        let key: &'static str = Box::leak(key.into_boxed_str());
        rows.push(Binding::new("check_updates", key_get(py, key, obj(py, "1")?, Kind::Bool)?, key_set(py, key, Kind::Bool)?));
    }
    rows.extend(figure_rows(py)?);
    pref_page(py, env, "general", rows)
}

fn figure_key(py: Python<'_>, name: &str) -> PyResult<String> {
    py.import("ingetrazo_core.scale_figure")?.getattr(name)?.extract()
}

fn put_image(env: &Bound<'_, PyAny>, path: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = env.py();
    if !path.is_truthy()? {
        return Ok(());
    }
    let sf = py.import("ingetrazo_core.scale_figure")?;
    let readable = py.import("ingetrazo_render")?.call_method1("raster_dimensions", (path,))?;
    if !readable.is_truthy()? {
        return Err(pyo3::exceptions::PyValueError::new_err(tr(py, "That file is not a picture Built can read.")?));
    }
    let stored = match sf.call_method1("store_picture", (path,)) {
        Ok(p) => p.str()?,
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyOSError>(py) || e.is_instance_of::<pyo3::exceptions::PyValueError>(py) => {
            return Err(pyo3::exceptions::PyValueError::new_err(tr(py, "The picture could not be copied; the previous figure is kept.")?));
        }
        Err(e) => return Err(e),
    };
    let st = env.getattr("st")?;
    st.call_method1("setValue", (figure_key(py, "KEY_IMAGE")?, &stored))?;
    st.call_method1("setValue", (figure_key(py, "KEY_PICK")?, sf.getattr("CUSTOM")?))?;
    let name_key = figure_key(py, "KEY_NAME")?;
    let name = st.call_method1("value", (&name_key, ""))?;
    if name.is_none() || name.str()?.to_string().trim().is_empty() {
        let stem: String = py.import("pathlib")?.getattr("Path")?.call1((path,))?.getattr("stem")?.extract()?;
        st.call_method1("setValue", (&name_key, stem.replace('_', " ")))?;
    }
    let height_key = figure_key(py, "KEY_HEIGHT")?;
    let height = st.call_method1("value", (&height_key, 1.70))?;
    let height: f64 = if height.is_truthy()? { py.import("builtins")?.getattr("float")?.call1((height,))?.extract()? } else { 1.70 };
    st.call_method1("setValue", (&height_key, py.import("builtins")?.getattr("round")?.call1((height, 3))?))?;
    st.call_method0("sync")?;
    // A picture without transparency shows as a rectangle.
    if !crate::image_assets::texture_has_alpha(py, stored.to_string().into())? {
        if let Some(state) = opt(env, "state") {
            let note = (tr(py, "This picture has no transparent background, so it will show as a rectangle. A PNG cut out around the person looks best.")?, "warn");
            state.setattr("note", note)?;
        }
    }
    Ok(())
}

fn figure_rows(py: Python<'_>) -> PyResult<Vec<Binding>> {
    let put_pick = func(py, |py, a| {
        let (env, v) = (a.get_item(0)?, a.get_item(1)?);
        let sf = py.import("ingetrazo_core.scale_figure")?;
        let st = env.getattr("st")?;
        if v.eq(sf.getattr("CUSTOM")?)? {
            let saved = st.call_method1("value", (figure_key(py, "KEY_IMAGE")?, ""))?;
            let saved = if saved.is_truthy()? { saved.str()?.to_string() } else { String::new() };
            if saved.is_empty() || !std::path::Path::new(&saved).is_file() {
                let dialogs = py.import("ingetrazo_render.shell_files")?.getattr("ShellFileDialog")?;
                let filter = format!("{} (*.png *.webp)", tr(py, "Images")?);
                let path = dialogs.call_method1("getOpenFileName", (env.getattr("win")?, tr(py, "My own picture…")?, "", filter))?.get_item(0)?;
                if path.is_truthy()? {
                    put_image(&env, &path)?;
                }
                return Ok(none(py));
            }
        }
        st.call_method1("setValue", (figure_key(py, "KEY_PICK")?, v))?;
        st.call_method0("sync")?;
        Ok(none(py))
    })?;
    let options = func(py, |py, _a| {
        let sf = py.import("ingetrazo_core.scale_figure")?;
        let out = PyList::empty(py);
        for p in sf.call_method0("people")?.try_iter()? {
            let p = p?;
            let height: f64 = py.import("builtins")?.getattr("float")?.call1((p.get_item("height")?,))?.extract()?;
            let name: String = py.import("core.i18n")?.call_method1("tr", (p.get_item("name")?,))?.extract()?;
            out.append((p.get_item("key")?, format!("{name} ({height:.2} m)")))?;
        }
        out.append((sf.getattr("CUSTOM")?, tr(py, "My own picture…")?))?;
        Ok(out.into_any().unbind())
    })?;
    let filters = format!("{} (*.png *.webp);;{} (*)", tr(py, "Images")?, tr(py, "All files")?);
    let stored = |name: &'static str| -> Res {
        func(py, move |py, a| {
            let v = a.get_item(0)?.getattr("st")?.call_method1("value", (figure_key(py, name)?, ""))?;
            Ok(s(py, &if v.is_truthy()? { v.str()?.to_string() } else { String::new() }).unbind())
        })
    };
    let height = func(py, |py, a| {
        let v = a.get_item(0)?.getattr("st")?.call_method1("value", (figure_key(py, "KEY_HEIGHT")?, 1.70))?;
        Ok(obj(py, py.import("builtins")?.getattr("float")?.call1((v,)).and_then(|f| f.extract::<f64>()).unwrap_or(1.70))?)
    })?;
    Ok(vec![
        Binding::new("scale_figure", key_get(py, "new_document/scale_figure", obj(py, "1")?, Kind::Bool)?, key_set(py, "new_document/scale_figure", Kind::Bool)?),
        Binding::new("figure", func(py, |py, a| {
            let sf = py.import("ingetrazo_core.scale_figure")?;
            let default = sf.getattr("DEFAULT_PICK")?;
            let v = a.get_item(0)?.getattr("st")?.call_method1("value", (figure_key(py, "KEY_PICK")?, &default))?;
            Ok((if v.is_truthy()? { v.str()? } else { default.str()? }).into_any().unbind())
        })?, put_pick)
        .with("options", options),
        Binding::new("figure_image", stored("KEY_IMAGE")?, func(py, |py, a| {
            put_image(&a.get_item(0)?, &a.get_item(1)?)?;
            Ok(none(py))
        })?)
        .with("filters", obj(py, filters)?),
        Binding::new("figure_height", height, func(py, |py, a| {
            let (env, v) = (a.get_item(0)?, a.get_item(1)?);
            let builtins = py.import("builtins")?;
            let v = builtins.getattr("round")?.call1((builtins.getattr("float")?.call1((v,))?, 3))?;
            let st = env.getattr("st")?;
            st.call_method1("setValue", (figure_key(py, "KEY_HEIGHT")?, v))?;
            st.call_method0("sync")?;
            Ok(none(py))
        })?),
        Binding::new("figure_name", stored("KEY_NAME")?, func(py, |py, a| {
            let (env, v) = (a.get_item(0)?, a.get_item(1)?);
            let st = env.getattr("st")?;
            st.call_method1("setValue", (figure_key(py, "KEY_NAME")?, v.str()?.to_string().trim()))?;
            st.call_method0("sync")?;
            Ok(none(py))
        })?),
    ])
}

fn navigation<'py>(py: Python<'py>, env: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    let rows = vec![
        Binding::new("invert_wheel", key_get(py, "nav/invert_wheel", obj(py, "0")?, Kind::Bool)?, applier(py, "apply_invert_wheel")?),
        Binding::new("invert_orbit", key_get(py, "nav/invert_orbit_y", obj(py, "0")?, Kind::Bool)?, applier(py, "apply_invert_orbit")?),
        Binding::new("look_sens", func(py, |py, _a| Ok(py.import("tools.walkthrough")?.call_method0("look_sensitivity")?.unbind()))?, key_set(py, "walk/look_sensitivity", Kind::Str)?),
        Binding::new("touch_slide", key_get(py, "nav/touch_slide", obj(py, "orbit")?, Kind::Str)?, key_set(py, "nav/touch_slide", Kind::Str)?),
        Binding::new("trackpad_inertia", key_get(py, "nav/trackpad_inertia", obj(py, "1")?, Kind::Bool)?, key_set(py, "nav/trackpad_inertia", Kind::Bool)?),
        Binding::new("pinch_sensitivity", key_get(py, "nav/pinch_sensitivity", obj(py, 100)?, Kind::Int)?, key_set(py, "nav/pinch_sensitivity", Kind::Str)?),
        Binding::new("trackpad_slide", func(py, |py, _a| Ok(py.import("ingetrazo_render")?.getattr("trackpad_nav")?.call_method0("load_slide_mode")?.unbind()))?, applier(py, "apply_slides")?),
        Binding::new("cursor_style", func(py, |py, a| {
            let v = gkey_value(&a.get_item(0)?, "cursor_style", &s(py, "tool"), Kind::Str)?;
            Ok(v.bind(py).call_method0("lower")?.unbind())
        })?, applier(py, "apply_cursor_style")?),
        Binding::new("ask_section", key_get(py, "section/ask_name", obj(py, "1")?, Kind::Bool)?, key_set(py, "section/ask_name", Kind::Bool)?),
    ];
    pref_page(py, env, "navigation", rows)
}

fn soft_spread(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("views.document_interaction")?.getattr("SOFT_SHADOW_SPREAD")
}

fn soft_level(env: &Bound<'_, PyAny>) -> PyResult<i64> {
    let py = env.py();
    let st = env.getattr("st")?;
    let zero = 0i32.into_pyobject(py)?.into_any();
    let soft = int_of(&st.call_method1("value", ("display/soft_shadows", 0))?, &zero)?.into_bound(py);
    let spread = soft_spread(py)?;
    if spread.contains(&soft)? {
        return soft.extract();
    }
    let two = 2i32.into_pyobject(py)?.into_any();
    let level = int_of(&st.call_method1("value", ("display/soft_shadows_level", 2))?, &two)?.into_bound(py);
    Ok(if spread.contains(&level)? { level.extract()? } else { 2 })
}

fn soft_on(env: &Bound<'_, PyAny>) -> PyResult<bool> {
    let py = env.py();
    let zero = 0i32.into_pyobject(py)?.into_any();
    let soft = int_of(&env.getattr("st")?.call_method1("value", ("display/soft_shadows", 0))?, &zero)?;
    soft_spread(py)?.contains(soft)
}

fn rest_mode(env: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let py = env.py();
    Ok(env.getattr("vp")?.getattr("edit_rest_mode").unwrap_or_else(|_| s(py, "fade")).unbind())
}

fn rest_fade(env: &Bound<'_, PyAny>) -> PyResult<i64> {
    let py = env.py();
    let default: f64 = py.import("views.document_interaction")?.getattr("EDIT_REST_FADE")?.extract()?;
    let v = env.getattr("vp")?.getattr("edit_rest_fade").ok();
    let f = v.and_then(|v| py.import("builtins").ok()?.getattr("float").ok()?.call1((v,)).ok()?.extract::<f64>().ok()).unwrap_or(default);
    Ok((f * 100.0).round_ties_even() as i64)
}

fn display<'py>(py: Python<'py>, env: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    let put_soft = |which: &'static str| -> Res {
        func(py, move |py, a| {
            let (env, v) = (a.get_item(0)?, a.get_item(1)?);
            let on = if which == "on" { v.is_truthy()? } else { soft_on(&env)? };
            let level = if which == "level" { py.import("builtins")?.getattr("int")?.call1((v,))?.extract()? } else { soft_level(&env)? };
            let st = env.getattr("st")?;
            apply(py)?.call_method1("apply_soft_shadows", (env.getattr("win")?, on, level, &st))?;
            st.call_method0("sync")?;
            Ok(none(py))
        })
    };
    let msaa = Binding::new("msaa", key_get(py, "display/msaa", obj(py, 4)?, Kind::Int)?, func(py, |py, a| {
        let v: u32 = a.get_item(1)?.extract()?;
        crate::preferences::apply_msaa(&a.get_item(0)?, v)?;
        Ok(none(py))
    })?)
    .with("levels", func(py, |py, a| Ok(obj(py, crate::preferences::msaa_levels(&a.get_item(0)?)?)?))?);
    let mut fade = Binding::new("rest_fade", func(py, |py, a| obj(py, rest_fade(&a.get_item(0)?)?))?, func(py, |py, a| {
        let (env, v) = (a.get_item(0)?, a.get_item(1)?);
        let pct: i64 = py.import("builtins")?.getattr("int")?.call1((v,))?.extract()?;
        apply(py)?.call_method1("apply_rest_mode", (env.getattr("win")?, rest_mode(&env)?, pct as f64 / 100.0))?;
        Ok(none(py))
    })?);
    fade.fmt = Some(func(py, |py, a| {
        let v = a.get_item(0)?;
        let n: i64 = if v.is_truthy()? { py.import("builtins")?.getattr("int")?.call1((v,))?.extract()? } else { 0 };
        obj(py, format!("{n} %"))
    })?);
    let rows = vec![
        msaa,
        Binding::new("soft_shadows", func(py, |py, a| obj(py, soft_on(&a.get_item(0)?)?))?, put_soft("on")?),
        Binding::new("soft_level", func(py, |py, a| obj(py, soft_level(&a.get_item(0)?)?))?, put_soft("level")?),
        Binding::new("rest_mode", func(py, |_py, a| rest_mode(&a.get_item(0)?))?, func(py, |py, a| {
            let (env, v) = (a.get_item(0)?, a.get_item(1)?);
            apply(py)?.call_method1("apply_rest_mode", (env.getattr("win")?, v, rest_fade(&env)? as f64 / 100.0))?;
            Ok(none(py))
        })?),
        fade,
    ];
    pref_page(py, env, "display", rows)
}

/// The 3D mouse model with the saved profiles, and those profiles.
pub fn device_preferences<'py>(env: &Bound<'py, PyAny>, named: Option<Bound<'py, PyAny>>) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyAny>)> {
    let py = env.py();
    let nd = py.import("views.ndof_input")?.call_method0("load_settings")?;
    let profile = crate::preferences::device_profile::profile_to_dict(&nd)?;
    if nd.getattr("mapping_preset")?.eq("original")? {
        profile.set_item("inversion", apply(py)?.call_method1("legacy_ndof_inversion", (&nd,))?)?;
    }
    let named = match named {
        Some(n) => n,
        None => apply(py)?.call_method0("read_named_ndof_profiles")?,
    };
    let dicts = PyDict::new(py);
    for (k, v) in named.cast::<PyDict>()?.iter() {
        dicts.set_item(k, crate::preferences::device_profile::profile_to_dict(&v)?)?;
    }
    let json = py.import("json")?;
    let selected = env.getattr("st")?.call_method1("value", ("ndof/selected_profile", ""))?;
    let selected = if selected.is_truthy()? { selected.str()?.to_string() } else { String::new() };
    let model = py.import("ingetrazo_render")?.getattr("DevicePreferences")?.call1((
        json.call_method1("dumps", (profile,))?,
        json.call_method1("dumps", (dicts,))?,
        nd.getattr("mapping_preset")?,
        selected,
        nd.getattr("enabled")?,
        nd.getattr("lock_rotation")?,
    ))?;
    Ok((model, named))
}

/// Apply 3D mouse edits and save the profile.
pub fn device_edit<'py>(env: &Bound<'py, PyAny>, edits: &Bound<'py, PyDict>, named: Option<Bound<'py, PyAny>>) -> PyResult<()> {
    let py = env.py();
    let (model, named) = device_preferences(env, named)?;
    let json = py.import("json")?;
    for (k, v) in edits.iter() {
        model.call_method1("edit", (k, json.call_method1("dumps", (v,))?))?;
    }
    let nd = crate::preferences::device_profile::profile_from_dict(&json.call_method1("loads", (model.call_method0("profile")?,))?)?;
    nd.setattr("enabled", model.getattr("enabled")?)?;
    nd.setattr("lock_rotation", model.getattr("locked")?)?;
    let st = env.getattr("st")?;
    apply(py)?.call_method1("save_ndof", (nd, named, model.getattr("selected")?, &st))?;
    st.call_method0("sync")?;
    Ok(())
}

fn ndof<'py>(py: Python<'py>, env: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    let (model, _) = device_preferences(env, None)?;
    let shared = py.import("views.ndof_input")?.call_method1("shared_input", (env.getattr("win")?,))?;
    let driver = shared.getattr("backend_name")?;
    let driver = if driver.is_truthy()? { driver.str()?.to_string() } else { String::new() };
    let tr_fn = py.import("core.i18n")?.getattr("tr")?;
    let live = match opt(env, "device_readings").filter(|r| r.is_truthy().unwrap_or(false)) {
        Some(r) => r.call_method1("text", (&tr_fn,))?,
        None => PyTuple::new(py, ["—", "—"])?.into_any(),
    };
    let spec: String = model.call_method1("groups", (&tr_fn, driver, live))?.extract()?;
    let put = func(py, |py, a| {
        let edits = PyDict::new(py);
        edits.set_item(a.get_item(1)?, a.get_item(2)?)?;
        device_edit(&a.get_item(0)?, &edits, None)?;
        Ok(none(py))
    })?;
    native_groups(py, &spec, put)
}

fn import<'py>(py: Python<'py>, env: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    pref_page(py, env, "import", vec![Binding::new("dxf_unit", key_get(py, "import/dxf_unit", obj(py, "mm")?, Kind::Str)?, key_set(py, "import/dxf_unit", Kind::Str)?)])
}

// ---- This model ----------------------------------------------------------------------

/// The document was edited: dirty, and every view hears of it.
#[pyfunction]
pub fn _scene_changed(env: &Bound<'_, PyAny>) -> PyResult<()> {
    let vp = env.getattr("vp")?;
    if let Some(notify) = opt(&vp, "notify_scene_changed").filter(|n| n.is_callable()) {
        notify.call0()?;
    } else if !vp.is_none() && vp.hasattr("update")? {
        vp.call_method0("update")?;
    }
    Ok(())
}

fn units_draft<'py>(env: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = env.py();
    let key = live::NEW_UNITS;
    let model = py.import("ingetrazo_render.units_question")?.call_method1("draft", (env.getattr("scene")?,))?;
    let for_new = truth(&env.getattr("st")?.call_method1("value", (key, "1"))?, "1")?;
    model.call_method1("edit_document", (for_new, py.import("core.units")?.call_method0("ask_at_start")?))?;
    Ok(model)
}

#[pyfunction]
pub fn _units<'py>(env: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    let py = env.py();
    let put = |key: Py<PyAny>| -> Res {
        func(py, move |py, a| {
            let (env, value) = (a.get_item(0)?, a.get_item(1)?);
            let key = key.bind(py);
            let units = py.import("core.units")?;
            let model = units_draft(&env)?;
            let n: i64 = py.import("builtins")?.getattr("int")?.call1((value,))?.extract()?;
            model.call_method1("receive", (key, n.to_string()))?;
            if key.eq("ask_again")? {
                units.call_method1("set_ask_at_start", (model.getattr("ask_again")?,))?;
                return Ok(none(py));
            }
            let chosen = py.import("json")?.call_method1("loads", (model.call_method0("settings")?,))?;
            let apply = apply(py)?;
            if key.eq("for_new")? {
                let st = env.getattr("st")?;
                st.call_method1("setValue", (live::NEW_UNITS, if model.getattr("for_new")?.is_truthy()? { "1" } else { "0" }))?;
                st.call_method0("sync")?;
            } else if !apply.call_method1("apply_document_units", (env.getattr("win")?, &chosen))?.is_truthy()? {
                return Ok(none(py));
            }
            if model.getattr("for_new")?.is_truthy()? {
                units.call_method1("remember_new_document_units", (chosen,))?;
            }
            Ok(none(py))
        })
    };
    let disabled: i64 = py.import("ingetrazo_render.chrome_forms")?.getattr("DISABLED")?.extract()?;
    let (shown, beyond) = (PyList::empty(py), PyList::empty(py));
    for r in py.import("ingetrazo_render.units_question")?.call_method1("model_rows", (units_draft(env)?,))?.try_iter()? {
        let r = r?;
        let (key, kind, label, text, value) = (r.get_item(0)?, r.get_item(1)?, r.get_item(2)?, r.get_item(3)?, r.get_item(4)?);
        let (options, flags, tip) = (r.get_item(7)?, r.get_item(8)?.extract::<i64>()?, r.get_item(9)?);
        let key_s: String = key.extract()?;
        let rid = if key_s == "ask_again" { "u_ask_start".to_owned() } else { format!("u_{key_s}") };
        let val = if kind.eq("text")? { text.unbind() } else { py.import("builtins")?.getattr("int")?.call1((value,))?.unbind() };
        let get = func(py, move |py, _a| Ok(val.clone_ref(py)))?;
        let put = if kind.eq("text")? { none(py) } else { put(key.clone().unbind())? };
        let enumerated = py.import("builtins")?.getattr("list")?.call1((py.import("builtins")?.getattr("enumerate")?.call1((options,))?,))?;
        let row = row(py, &rid, &kind.str()?.to_string(), &label.str()?.to_string(), Some(get), Some(put), &[
            ("options", enumerated.unbind()),
            ("enabled", obj(py, flags & disabled == 0)?),
            ("tip", tip.unbind()),
        ])?;
        if key_s == "for_new" || key_s == "ask_again" { beyond.append(row)? } else { shown.append(row)? }
    }
    PyList::new(py, [
        PyTuple::new(py, [s(py, "units"), s(py, &tr(py, "Units of this model")?), shown.into_any()])?,
        PyTuple::new(py, [s(py, "unitsnew"), s(py, &tr(py, "New documents")?), beyond.into_any()])?,
    ])
}

fn style<'py>(env: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = env.py();
    let s = env.getattr("scene")?.getattr("dimension_style").ok().filter(|s| s.is_instance_of::<PyDict>());
    Ok(s.unwrap_or_else(|| PyDict::new(py).into_any()))
}

fn dimensions<'py>(py: Python<'py>, _env: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    let units = py.import("core.units")?;
    let g = |key: &'static str, default: Py<PyAny>| -> Res {
        func(py, move |py, a| Ok(style(&a.get_item(0)?)?.call_method1("get", (key, default.bind(py)))?.unbind()))
    };
    let typed = |key: &'static str, default: Py<PyAny>, conv: &'static str| -> Res {
        func(py, move |py, a| {
            let v = style(&a.get_item(0)?)?.call_method1("get", (key, default.bind(py)))?;
            Ok(py.import("builtins")?.getattr(conv)?.call1((v,))?.unbind())
        })
    };
    // put(key, conv): one command per change, nothing when unchanged.
    let put = |key: &'static str, conv: &'static str| -> Res {
        func(py, move |py, a| {
            let (env, v) = (a.get_item(0)?, a.get_item(1)?);
            let value = match conv {
                "" => v,
                "rgb" => {
                    let h = v.str()?.to_string();
                    let h = h.trim_start_matches('#');
                    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
                        return Err(pyo3::exceptions::PyValueError::new_err(tr(py, "A colour is #rrggbb.")?));
                    }
                    let rgb: Vec<i64> = (0..3).map(|i| i64::from_str_radix(&h[i * 2..i * 2 + 2], 16).unwrap_or(0)).collect();
                    PyList::new(py, rgb)?.into_any()
                }
                other => py.import("builtins")?.getattr(other)?.call1((v,))?,
            };
            if env.getattr("scene")?.is_none() || style(&env)?.call_method1("get", (key,))?.eq(&value)? {
                return Ok(none(py));
            }
            let history = apply(py)?.call_method1("document_history", (env.getattr("vp")?,))?;
            let done = py.import("ingetrazo_core.command_protocol")?.call_method1("set_dimension_style", (history, key, value))?;
            if done.get_item("ok")?.is_truthy()? {
                _scene_changed(&env)?;
            }
            Ok(none(py))
        })
    };
    let unit_options = PyList::empty(py);
    for c in units.getattr("UNIT_CHOICES")?.try_iter()? {
        let c = c?;
        unit_options.append((&c, units.call_method1("unit_label", (&c,))?))?;
    }
    let color = func(py, |py, a| {
        let c = style(&a.get_item(0)?)?.call_method1("get", ("color", vec![45, 55, 75]))?;
        let parts: PyResult<Vec<i64>> = (0..3).map(|i| py.import("builtins")?.getattr("int")?.call1((c.get_item(i)?,))?.extract()).collect();
        Ok(s(py, &match parts {
            Ok(p) => format!("#{:02x}{:02x}{:02x}", p[0], p[1], p[2]),
            Err(_) => "#2d374b".into(),
        })
        .unbind())
    })?;
    let ends = func(py, |py, a| {
        let v = style(&a.get_item(0)?)?.call_method1("get", ("ends", "arrow"))?;
        Ok(s(py, &if v.is_truthy()? { v.str()?.to_string() } else { "arrow".into() }).unbind())
    })?;
    let rows = PyList::new(py, [
        row(py, "dim_decimals", "number", &tr(py, "Decimals")?, Some(typed("decimals", obj(py, 2)?, "int")?), Some(put("decimals", "int")?),
            &[("lo", obj(py, 0)?), ("hi", obj(py, 4)?), ("words", obj(py, "precision")?)])?,
        row(py, "dim_units", "choice", &tr(py, "Unit")?, Some(g("units", obj(py, "m")?)?), Some(put("units", "")?),
            &[("options", unit_options.into_any().unbind())])?,
        row(py, "dim_show_unit", "check", &tr(py, "Show unit")?, Some(typed("show_unit", obj(py, true)?, "bool")?), Some(put("show_unit", "bool")?),
            &[("tip", obj(py, tr(py, "«1.30» instead of «1.30 m»")?)?)])?,
        row(py, "dim_family", "button", &tr(py, "Font family…")?, Some(g("font_family", obj(py, "")?)?), None,
            &[("press", func(py, |py, a| {
                let env = a.get_item(0)?;
                crate::sheet_runtime::close(&env.getattr("win")?)?;
                py.import("ingetrazo_render.font_picker")?.call_method1("annotation", (env.getattr("win")?,))?;
                Ok(none(py))
            })?)])?,
        row(py, "dim_font", "number", &tr(py, "Font size")?, Some(typed("font_size", obj(py, 9)?, "int")?), Some(put("font_size", "int")?),
            &[("lo", obj(py, 6)?), ("hi", obj(py, 28)?), ("suffix", obj(py, "pt")?), ("words", obj(py, "text")?)])?,
        row(py, "dim_color", "color", &tr(py, "Colour")?, Some(color), Some(put("color", "rgb")?), &[("words", obj(py, "color")?)])?,
        row(py, "dim_ends", "choice", &tr(py, "Ends")?, Some(ends), Some(put("ends", "")?),
            &[("options", PyList::new(py, [("arrow", tr(py, "Arrows")?), ("tick", tr(py, "Oblique ticks")?), ("none", tr(py, "None")?)])?.into_any().unbind()),
              ("words", obj(py, "arrow tick")?)])?,
        row(py, "dim_base_step", "number", &tr(py, "Baseline step")?, Some(typed("base_step_mm", obj(py, 8.0)?, "float")?), Some(put("base_step_mm", "float")?),
            &[("lo", obj(py, 1.0)?), ("hi", obj(py, 60.0)?), ("decimals", obj(py, 1)?), ("suffix", obj(py, "mm")?),
              ("tip", obj(py, tr(py, "How far apart the rows of a baseline dimension run sit on paper.")?)?)])?,
    ])?;
    PyList::new(py, [PyTuple::new(py, [s(py, "dimensions"), s(py, &tr(py, "Dimensions")?), rows.into_any()])?])
}

/// North through the command the North tool and the Shadows panel use
/// (one undo step): typing «30» is one step, not one per key.
fn put_north(env: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = env.py();
    let vp = env.getattr("vp")?;
    let Some(history) = opt(&vp, "history") else { return Ok(()) };
    let value: f64 = py.import("builtins")?.getattr("float")?.call1((value,))?.extract()?;
    let north: f64 = py.import("core.sun")?.call_method1("scene_north_deg", (vp.getattr("scene")?,))?.extract()?;
    if (value - north).abs() < 1e-9 {
        return Ok(());
    }
    let module = py.import("ingetrazo_render")?.getattr("settings_sheet")?;
    let last = module.getattr("_north_cmd")?;
    let undo = history.getattr("undo_stack")?;
    if !last.is_none() && undo.is_truthy()? && undo.get_item(-1)?.is(&last) {
        history.call_method0("undo")?; // the step being typed: replaced
    }
    let kwargs = [("degrees", value)].into_py_dict(py)?;
    let changed = py.import("ingetrazo_core.command_protocol")?.getattr("run")?.call((&history, "set_north"), Some(&kwargs))?;
    let undo = history.getattr("undo_stack")?;
    let cmd = if changed.get_item("ok")?.is_truthy()? && undo.is_truthy()? { undo.get_item(-1)? } else { py.None().into_bound(py) };
    module.setattr("_north_cmd", cmd)?;
    py.import("tools.north")?.call_method1("refresh_north_display", (&vp,))?;
    Ok(())
}

fn location<'py>(py: Python<'py>, env: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    let north = func(py, |py, a| {
        let scene = a.get_item(0)?.getattr("scene")?;
        if scene.is_none() {
            return obj(py, 0.0);
        }
        let deg: f64 = py.import("core.sun")?.call_method1("scene_north_deg", (scene,))?.extract()?;
        Ok(py.import("builtins")?.getattr("round")?.call1((deg, 1))?.unbind())
    })?;
    let where_ = func(py, |py, a| {
        let datum = a.get_item(0)?.getattr("scene")?.getattr("georef").ok().filter(|d| !d.is_none());
        let Some(datum) = datum else { return obj(py, tr(py, "No location yet: north turns only the sun.")?) };
        let lat: f64 = datum.getattr("lat")?.extract()?;
        let lon: f64 = datum.getattr("lon")?.extract()?;
        obj(py, trf(py, "{lat}, {lon} — the base map and the terrain turn with north.", &[("lat", s(py, &format!("{lat:.6}"))), ("lon", s(py, &format!("{lon:.6}")))])?)
    })?;
    let scene = env.getattr("scene")?;
    let has = !scene.is_none() && scene.getattr("georef").is_ok_and(|g| !g.is_none());
    let rows = PyList::new(py, [
        row(py, "north", "number", &tr(py, "North")?, Some(north), Some(func(py, |py, a| {
            put_north(&a.get_item(0)?, &a.get_item(1)?)?;
            Ok(none(py))
        })?), &[("lo", obj(py, -180.0)?), ("hi", obj(py, 180.0)?), ("decimals", obj(py, 1)?), ("suffix", obj(py, "°")?),
               ("words", obj(py, "sun shadows angle")?),
               ("tip", obj(py, tr(py, "Where true north lies, in degrees clockwise from the green (Y) axis. The shadows turn with it; the model does not move.")?)?)])?,
        row(py, "north_tool", "button", &tr(py, "Aim north on screen")?, None, None, &[
            ("press", func(py, |py, a| {
                let win = a.get_item(0)?.getattr("win")?;
                crate::sheet_runtime::close(&win)?;
                if let Some(activate) = opt(&win, "_activate_tool").filter(|f| f.is_callable()) {
                    activate.call1(("north",))?;
                }
                Ok(none(py))
            })?),
            ("tip", obj(py, tr(py, "Tools ▸ North: click a centre, then click toward north.")?)?)])?,
        row(py, "location", "text", &tr(py, "Location")?, Some(where_), None, &[])?,
        row(py, "add_location", "button", &tr(py, if has { "Edit location…" } else { "Add location…" })?, None, None, &[
            ("press", func(py, |py, a| {
                crate::settings_host::edit_location_now(&a.get_item(0)?.getattr("win")?, None, None, None)?;
                Ok(none(py))
            })?),
            ("enabled", obj(py, !scene.is_none())?),
            ("words", obj(py, "georeference map site")?),
            ("tip", obj(py, tr(py, "Pick the site on the map: the sun and the base map stand there.")?)?)])?,
    ])?;
    if has && opt(&scene, "tile_layer").is_some_and(|l| !l.is_none()) {
        rows.append(row(py, "terrain3d", "check", &tr(py, "3D terrain")?, Some(func(py, |py, a| {
            let vp = a.get_item(0)?.getattr("win")?.getattr("viewport")?;
            let terrain = vp.getattr("scene")?.getattr("terrain")?;
            obj(py, crate::document_terrain::enabled(&vp)? && (terrain.is_none() || terrain.getattr("visible")?.is_truthy()?))
        })?), Some(func(py, |py, a| {
            let vp = a.get_item(0)?.getattr("win")?.getattr("viewport")?;
            crate::document_terrain::set_terrain_enabled(&vp, a.get_item(1)?.extract()?)?;
            Ok(none(py))
        })?), &[])?)?;
    }
    PyList::new(py, [PyTuple::new(py, [s(py, "location"), s(py, &tr(py, "Location & north")?), rows.into_any()])?])
}

/// `[(group id, caption, [Row])]` of a section (not Keyboard).
pub fn section_groups<'py>(py: Python<'py>, env: &Bound<'py, PyAny>, section: &str) -> PyResult<Bound<'py, PyList>> {
    match section {
        "ai" => ai(py, env),
        "general" => general(py, env),
        "navigation" => navigation(py, env),
        "display" => display(py, env),
        "ndof" => ndof(py, env),
        "import" => import(py, env),
        "units" => _units(env),
        "dimensions" => dimensions(py, env),
        "appearance" => {
            let groups = appearance::groups(py, env)?;
            crate::model_panels::model_appearance::append(py, env, &groups)?;
            crate::model_panels::shadows::append(py, env, &groups)?;
            Ok(groups)
        },
        "location" => {
            let groups = location(py, env)?;
            crate::model_panels::basemap::append(py, env, &groups)?;
            Ok(groups)
        },
        other => Err(pyo3::exceptions::PyKeyError::new_err(other.to_owned())),
    }
}

pub fn register(sub: &Bound<'_, PyModule>) -> PyResult<()> {
    live::register(sub)?;
    sub.add_class::<Row>()?;
    sub.add_class::<Env>()?;
    sub.add_function(wrap_pyfunction!(_scene_changed, sub)?)?;
    sub.add_function(wrap_pyfunction!(_units, sub)?)?;
    Ok(())
}
