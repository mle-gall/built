// SPDX-License-Identifier: GPL-3.0-or-later
//! Settings on the large sheet: the submodule `built_render.settings_sheet`.
//!
//! Sections on the left in two groups, **Built · this Mac** (General,
//! Navigation, Display, 3D mouse, Import, Keyboard, AI assistant), kept in
//! the native preference store, and **This model** (Units, Dimension style,
//! Location & north), kept in the document and saved with it. A search lists
//! the matching rows of every section. Changes apply at once; those that
//! need a restart are named at the top. `settings_rows.rs` binds each row to
//! where its value lives; `preferences.rs` owns the catalogue, the device
//! model and the sheet's state (`PreferencesView`).
use crate::settings_rows::{env, func, none, obj, opt, s, section_groups, tr, trf};
mod site_requests;
use pyo3::{
    prelude::*,
    types::{IntoPyDict, PyDict, PyList},
};

const MAC: &str = "mac";
const MODEL: &str = "model";

fn module(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("built_render")?.getattr("settings_sheet")
}

fn cf(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("built_render.chrome_forms")
}

fn chrome(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("built_render")?.getattr("chrome_sheet")
}

fn pending(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    module(py)?.getattr("_PENDING")
}

fn sections(py: Python<'_>) -> PyResult<Vec<(String, String, String)>> {
    module(py)?.getattr("SECTIONS")?.extract()
}

fn shortcuts(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("built_render.keyboard_bindings")
}

fn str_of(v: &Bound<'_, PyAny>) -> PyResult<String> {
    Ok(if v.is_none() { String::new() } else { v.str()?.to_string() })
}

/// The keys a user wrote, as portable text, or why they are no shortcut.
#[pyfunction]
fn _parse_keys(py: Python<'_>, text: Option<&str>) -> PyResult<Py<PyAny>> {
    let t = text.unwrap_or("").trim_matches(|c| matches!(c, ' ' | '\t' | '\r' | '\n'));
    if t.is_empty() {
        return Ok(PyList::empty(py).into_any().unbind());
    }
    let Some(seq) = crate::shortcut_text::shortcut_text(t).filter(|q| q.2.is_some()) else {
        return obj(py, trf(py, "«{keys}» is not a key combination Built knows (write it like Ctrl+Shift+K).", &[("keys", s(py, t))])?);
    };
    if let Some(why) = seq.3 {
        return obj(py, trf(py, "«{keys}» cannot be a shortcut: {why}.", &[("keys", s(py, &seq.1)), ("why", s(py, &tr(py, why)?))])?);
    }
    Ok(PyList::new(py, [seq.0])?.into_any().unbind())
}

/// The keys as the platform writes them, comma separated.
#[pyfunction]
fn _keys_text(keys: &Bound<'_, PyAny>) -> PyResult<String> {
    let mut out = Vec::new();
    for k in keys.try_iter()? {
        if let Some(seq) = crate::shortcut_text::shortcut_text(&k?.str()?.to_string()) {
            out.push(seq.1);
        }
    }
    Ok(out.join(", "))
}

/// Settings: sections on the left, the rows of the one picked (or every
/// row the search matches) in the middle.
#[pyclass(module = "built_render.settings_sheet", extends = crate::sheet_runtime::Provider, dict, get_all, set_all)]
pub struct SettingsSheet {
    registry: Py<PyAny>,
    site: Py<PyAny>,
    site_requests: Py<PyAny>,
    site_scene: Py<PyAny>,
    site_done: Py<PyAny>,
    site_source: Py<PyAny>,
    #[pyo3(name = "_device")]
    device: Py<PyAny>,
    #[pyo3(name = "_readings")]
    readings: Py<PyAny>,
    state: Py<PyAny>,
    #[pyo3(name = "_actions")]
    actions_cache: Py<PyAny>,
    #[pyo3(name = "_paths")]
    paths_cache: Py<PyAny>,
    #[pyo3(name = "_rows")]
    rows: Py<PyAny>,
    #[pyo3(name = "_win")]
    win: Py<PyAny>,
}

type Slf<'py> = Bound<'py, SettingsSheet>;

fn win<'py>(slf: &Slf<'py>) -> PyResult<Bound<'py, PyAny>> {
    slf.getattr("_win")
}

fn state<'py>(slf: &Slf<'py>) -> PyResult<Bound<'py, PyAny>> {
    slf.getattr("state")
}

fn note(slf: &Slf<'_>, text: &str, tone: &str) -> PyResult<()> {
    state(slf)?.setattr("note", (text, tone))
}

fn refresh(slf: &Slf<'_>) -> PyResult<()> {
    chrome(slf.py())?.call_method1("refresh", (win(slf)?,))?;
    Ok(())
}

/// A question on the window's form: `then()` on OK.
fn confirm(slf: &Slf<'_>, title: &str, text: &str, ok: &str, then: Py<PyAny>) -> PyResult<()> {
    let py = slf.py();
    let cf = cf(py)?;
    let field = cf.getattr("Field")?.call(("t", "text", "", text), Some(&[("full", true)].into_py_dict(py)?))?;
    let done = func(py, move |py, a| {
        if a.get_item(0)?.eq("ok")? {
            then.bind(py).call0()?;
        }
        Ok(none(py))
    })?;
    let kw = PyDict::new(py);
    kw.set_item("buttons", vec![("cancel", tr(py, "Cancel")?), ("ok", ok.to_owned())])?;
    kw.set_item("on_done", done)?;
    let form = cf.getattr("Form")?.call((title, vec![field]), Some(&kw))?;
    cf.getattr("show")?.call1((win(slf)?, form))?;
    Ok(())
}

impl SettingsSheet {
    fn new_state(py: Python<'_>, section: &str) -> PyResult<Self> {
        let n = || none(py);
        Ok(SettingsSheet {
            registry: n(),
            site: n(),
            site_requests: n(),
            site_scene: n(),
            site_done: n(),
            site_source: n(),
            device: n(),
            readings: n(),
            state: py.import("built_render")?.getattr("PreferencesView")?.call1((section,))?.unbind(),
            actions_cache: n(),
            paths_cache: n(),
            rows: PyDict::new(py).into_any().unbind(),
            win: n(),
        })
    }

    /// Search hits: `{section: [(group id, caption, [Row])] or keyboard count}`.
    fn hits<'py>(slf: &Slf<'py>, envb: &Bound<'py, PyAny>, query: &str, keyboard_count: i64) -> PyResult<Bound<'py, PyDict>> {
        let py = slf.py();
        let words: Vec<&str> = query.split_whitespace().collect();
        let out = PyDict::new(py);
        for (sid, label, _scope) in sections(py)? {
            if sid == "keyboard" {
                if keyboard_count != 0 {
                    out.set_item(&sid, keyboard_count)?;
                }
                continue;
            }
            let found = PyList::empty(py);
            let section = s(py, &tr(py, &label)?);
            for g in slf.call_method1("groups", (envb, &sid))?.try_iter()? {
                let g = g?;
                let caption = g.get_item(1)?;
                let rows = PyList::empty(py);
                for r in g.get_item(2)?.try_iter()? {
                    let r = r?;
                    let kind: String = r.getattr("kind")?.extract()?;
                    if kind == "text" || kind == "caption" || !r.call_method1("on", ("visible", envb))?.is_truthy()? {
                        continue;
                    }
                    let hay: String = r.call_method1("haystack", (envb, &section, &caption))?.extract()?;
                    if words.iter().all(|w| hay.contains(w)) {
                        rows.append(r)?;
                    }
                }
                if !rows.is_empty() {
                    found.append((g.get_item(0)?, caption, rows))?;
                }
            }
            if !found.is_empty() {
                out.set_item(&sid, found)?;
            }
        }
        Ok(out)
    }

    fn top(slf: &Slf<'_>, c: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = slf.py();
        let names: Vec<String> = pending(py)?.call_method0("labels")?.extract()?;
        if !names.is_empty() {
            c.call_method("text", (trf(py, "Restart Built to apply: {names}.", &[("names", s(py, &names.join(", ")))])?,), Some(&[("tone", "warn")].into_py_dict(py)?))?;
        }
        let n = state(slf)?.getattr("note")?;
        if n.is_truthy()? {
            c.call_method("text", (n.get_item(0)?,), Some(&[("tone", n.get_item(1)?)].into_py_dict(py)?))?;
        }
        Ok(())
    }

    fn keyboard_pane(sheet: &Bound<'_, PyAny>, content: &Bound<'_, PyAny>, keyboard: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = sheet.py();
        content.getattr("blocks")?.call_method1("extend", (keyboard.get_item("content")?,))?;
        let kw = PyDict::new(py);
        kw.set_item("width", 320)?;
        kw.set_item("empty", tr(py, "Pick a command to change its keys.")?)?;
        let detail = sheet.call_method("detail", (), Some(&kw))?;
        detail.getattr("blocks")?.call_method1("extend", (keyboard.get_item("detail")?,))?;
        Ok(())
    }

    fn section_pane(slf: &Slf<'_>, sheet: &Bound<'_, PyAny>, envb: &Bound<'_, PyAny>, keyboard: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = slf.py();
        let sid: String = state(slf)?.getattr("section")?.extract()?;
        let (title, scope, subtitle): (String, String, String) = state(slf)?.call_method0("heading")?.extract()?;
        if sid == "ndof" && !crate::settings_rows::NDOF_SUPPORTED {
            sheet.call_method1("content", (tr(py, &title)?, tr(py, "3D mouse input is unavailable on this platform.")?))?;
            return Ok(());
        }
        let mut sub = tr(py, &subtitle)?;
        if scope == MODEL {
            sub = format!("{}{}", if sub.is_empty() { String::new() } else { format!("{sub} ") }, tr(py, "(Saved in the document, not on this Mac.)")?);
        }
        let c = sheet.call_method1("content", (tr(py, &title)?, sub))?;
        Self::top(slf, &c)?;
        if sid == "general" {
            let lang = py.import("built_core.i18n")?;
            let saved = envb.getattr("st")?.call_method1("value", ("language",))?;
            if saved.is_truthy()? && !saved.eq("system")? && !lang.call_method1("resolve_language", (&saved,))?.eq(&saved)? {
                let name = crate::settings_rows::language_name(&lang.getattr("LANGUAGE_NAMES")?, &saved)?;
                c.call_method1("text", (trf(py, "Using English because your saved language ({name}) is unavailable.", &[("name", s(py, &name))])?,))?;
            }
        }
        if sid == "registry" {
            let scene = envb.getattr("scene")?;
            let mut editor = slf.getattr("registry")?;
            if editor.is_none() || !editor.getattr("scene")?.is(&scene) {
                editor = py.import("built_render")?.getattr("RegistryEditor")?.call1((scene,))?;
                slf.setattr("registry", &editor)?;
            }
            editor.call_method1("build", (&c,))?;
            return Ok(());
        }
        let site = slf.getattr("site")?;
        if sid == "location" && !site.is_none() {
            let tr_fn = py.import("built_core.i18n")?.getattr("tr")?;
            let edit = slf.getattr("site_edit")?.unbind();
            let put = func(py, move |py, a| Ok(edit.bind(py).call1((a.get_item(0)?, a.get_item(1)?, a.get_item(2)?))?.unbind()))?;
            let groups = crate::settings_rows::native_groups(py, &site.call_method1("groups", (&tr_fn,))?.extract::<String>()?, put)?;
            for g in groups.iter() {
                let gid = g.get_item(0)?;
                c.call_method1("caption", (g.get_item(1)?,))?;
                let form = slf.call_method1("_form", (envb, g.get_item(2)?, &gid))?;
                c.call_method("fields", (form,), Some(&[("id", &gid)].into_py_dict(py)?))?;
            }
            let src = slf.getattr("site_source")?;
            let requests = slf.getattr("site_requests")?;
            let root = if requests.is_truthy()? { requests.getattr("tiles")?.getattr("cache")?.getattr("root")?.str()?.to_string() } else { String::new() };
            let blocks: String = site.call_method1("map_blocks", (&tr_fn, src.getattr("id")?, root, src.getattr("max_zoom")?, src.getattr("attribution")?))?.extract()?;
            let json = py.import("json")?;
            c.getattr("blocks")?.call_method1("extend", (json.call_method1("loads", (blocks,))?,))?;
            let buttons = json.call_method1("loads", (site.call_method1("buttons", (&tr_fn,))?,))?;
            c.call_method("buttons", (buttons,), Some(&[("align", "left")].into_py_dict(py)?))?;
            return Ok(());
        }
        if sid == "keyboard" {
            return Self::keyboard_pane(sheet, &c, keyboard);
        }
        for g in slf.call_method1("groups", (envb, &sid))?.try_iter()? {
            let g = g?;
            let rows = g.get_item(2)?;
            let mut any = false;
            for r in rows.try_iter()? {
                if r?.call_method1("on", ("visible", envb))?.is_truthy()? {
                    any = true;
                    break;
                }
            }
            if !any {
                continue;
            }
            let gid = g.get_item(0)?;
            c.call_method1("caption", (g.get_item(1)?,))?;
            let form = slf.call_method1("_form", (envb, rows, &gid))?;
            c.call_method("fields", (form,), Some(&[("id", &gid)].into_py_dict(py)?))?;
        }
        if sid == "ndof" {
            let verb = chrome(py)?.getattr("verb")?;
            let buttons = vec![
                verb.call1(("ndof_load", tr(py, "Load profile…")?))?,
                verb.call1(("ndof_save", tr(py, "Save profile…")?))?,
                verb.call1(("ndof_reset", tr(py, "Reset to defaults")?))?,
            ];
            c.call_method("buttons", (buttons,), Some(&[("align", "left")].into_py_dict(py)?))?;
        }
        Ok(())
    }

    fn search_pane(slf: &Slf<'_>, sheet: &Bound<'_, PyAny>, envb: &Bound<'_, PyAny>, hits: &Bound<'_, PyDict>, keyboard: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = slf.py();
        let mut n: i64 = 0;
        for (sid, groups) in hits.iter() {
            if sid.eq("keyboard")? {
                n += groups.extract::<i64>()?;
            } else {
                for g in groups.try_iter()? {
                    n += g?.get_item(2)?.len()? as i64;
                }
            }
        }
        let q = state(slf)?.getattr("query")?.str()?.to_string();
        let q = q.trim();
        let kw = [("empty", trf(py, "No setting matches “{q}”.", &[("q", s(py, q))])?)].into_py_dict(py)?;
        let c = sheet.call_method("content", (trf(py, "Results for “{q}”", &[("q", s(py, q))])?, trf(py, "{n} matching settings in every section", &[("n", n.into_pyobject(py)?.into_any())])?), Some(&kw))?;
        Self::top(slf, &c)?;
        for (sid, label, _scope) in sections(py)? {
            let Some(groups) = hits.get_item(&sid)? else { continue };
            if sid == "keyboard" {
                Self::keyboard_pane(sheet, &c, keyboard)?;
                continue;
            }
            let label = tr(py, &label)?;
            for g in groups.try_iter()? {
                let g = g?;
                let caption = g.get_item(1)?.str()?.to_string();
                c.call_method1("caption", (if caption != label { format!("{label} · {caption}") } else { label.clone() },))?;
                let gid = format!("q.{}", g.get_item(0)?.str()?);
                let form = slf.call_method1("_form", (envb, g.get_item(2)?, &gid))?;
                c.call_method("fields", (form,), Some(&[("id", &gid)].into_py_dict(py)?))?;
            }
        }
        Ok(())
    }

    fn ndof_add(slf: &Slf<'_>, envb: &Bound<'_, PyAny>, name: &str, profile: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = slf.py();
        let named = crate::settings_rows::apply(py)?.call_method0("read_named_ndof_profiles")?;
        named.set_item(name, profile)?;
        let edits = PyDict::new(py);
        edits.set_item("ndof_preset", format!("named:{name}"))?;
        crate::settings_rows::device_edit(envb, &edits, Some(named))?;
        refresh(slf)
    }
}

#[pymethods]
impl SettingsSheet {
    #[new]
    #[pyo3(signature = (section="general"))]
    fn new(py: Python<'_>, section: &str) -> PyResult<PyClassInitializer<Self>> {
        Ok(PyClassInitializer::from(crate::sheet_runtime::Provider).add_subclass(Self::new_state(py, section)?))
    }

    #[classattr]
    fn id() -> &'static str {
        "settings"
    }

    #[getter]
    fn section(slf: &Slf<'_>) -> PyResult<Py<PyAny>> {
        Ok(state(slf)?.getattr("section")?.unbind())
    }

    #[getter]
    fn kb_focus(slf: &Slf<'_>) -> PyResult<Py<PyAny>> {
        Ok(state(slf)?.getattr("kb_focus")?.unbind())
    }

    #[pyo3(signature = (page=None, shortcut_of=None))]
    fn goto(slf: &Slf<'_>, page: Option<String>, shortcut_of: Option<Bound<'_, PyAny>>) -> PyResult<()> {
        let py = slf.py();
        let key = match shortcut_of {
            Some(a) => Some(shortcuts(py)?.call_method1("action_key", (a,))?.extract::<String>()?),
            None => None,
        };
        state(slf)?.call_method1("goto", (page.unwrap_or_default(), key))?;
        Ok(())
    }

    /// The 3D mouse's live readings while its section is up.
    fn _monitor(slf: &Slf<'_>, active: bool) -> PyResult<()> {
        let py = slf.py();
        let has = !slf.getattr("_device")?.is_none();
        if active && !has {
            let dev = py.import("views.ndof_input")?.call_method1("shared_input", (win(slf)?,))?;
            if dev.getattr("backend_name")?.is_truthy()? {
                slf.setattr("_readings", py.import("built_render")?.getattr("DeviceReadings")?.call0()?)?;
                slf.setattr("_device", &dev)?;
                dev.getattr("raw")?.call_method1("connect", (slf.getattr("_raw_sample")?,))?;
                dev.getattr("motion")?.call_method1("connect", (slf.getattr("_motion_sample")?,))?;
            }
        } else if !active && has {
            let dev = slf.getattr("_device")?;
            dev.getattr("raw")?.call_method1("disconnect", (slf.getattr("_raw_sample")?,))?;
            dev.getattr("motion")?.call_method1("disconnect", (slf.getattr("_motion_sample")?,))?;
            slf.setattr("_device", py.None())?;
            slf.setattr("_readings", py.None())?;
        }
        Ok(())
    }

    fn _raw_sample(slf: &Slf<'_>, values: &Bound<'_, PyAny>) -> PyResult<()> {
        let r = slf.getattr("_readings")?;
        if !r.is_none() {
            r.call_method1("raw", (values,))?;
            refresh(slf)?;
        }
        Ok(())
    }

    fn _motion_sample(slf: &Slf<'_>, sample: &Bound<'_, PyAny>, _dt: &Bound<'_, PyAny>) -> PyResult<()> {
        let r = slf.getattr("_readings")?;
        if !r.is_none() {
            let v = (sample.getattr("right")?, sample.getattr("up")?, sample.getattr("forward")?, sample.getattr("tilt")?, sample.getattr("spin")?);
            r.call_method1("motion", (v,))?;
            refresh(slf)?;
        }
        Ok(())
    }

    fn signature(&self, py: Python<'_>, ctx: &Bound<'_, PyAny>) -> PyResult<(usize, Py<PyAny>)> {
        let scene = opt(&ctx.getattr("win")?, "viewport").and_then(|v| opt(&v, "scene"));
        Ok(match scene {
            Some(sc) => (sc.as_ptr() as usize, sc.getattr("version").map_or_else(|_| obj(py, 0), |v| Ok(v.unbind()))?),
            None => (py.None().as_ptr() as usize, obj(py, 0)?),
        })
    }

    /// `[(group id, caption, [Row])]` of a section (not Keyboard); one
    /// broken section never takes the sheet down.
    fn groups<'py>(&self, py: Python<'py>, envb: &Bound<'py, PyAny>, section: &str) -> PyResult<Bound<'py, PyAny>> {
        match section_groups(py, envb, section) {
            Ok(g) => Ok(g.into_any()),
            Err(e) => {
                e.print(py);
                Ok(PyList::empty(py).into_any())
            }
        }
    }

    fn _form<'py>(slf: &Slf<'py>, envb: &Bound<'py, PyAny>, rows: &Bound<'py, PyAny>, gid: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let py = slf.py();
        let known = slf.getattr("_rows")?;
        let fields = PyList::empty(py);
        for r in rows.try_iter()? {
            let r = r?;
            if !r.call_method1("on", ("visible", envb))?.is_truthy()? {
                continue;
            }
            known.set_item(r.getattr("id")?, &r)?;
            fields.append(r.call_method1("field", (envb,))?)?;
        }
        let kw = PyDict::new(py);
        kw.set_item("buttons", PyList::empty(py))?;
        kw.set_item("on_change", slf.getattr("_changed")?)?;
        kw.set_item("on_button", slf.getattr("_pressed")?)?;
        kw.set_item("key", gid)?;
        let form = cf(py)?.getattr("Form")?.call(("", &fields), Some(&kw))?;
        let texts = form.getattr("_texts")?;
        for f in fields.iter() {
            if f.getattr("kind")?.eq("number")? {
                let v = f.getattr("value")?;
                let v: f64 = if v.is_truthy()? { py.import("builtins")?.getattr("float")?.call1((v,))?.extract()? } else { 0.0 };
                let text = state(slf)?.call_method1("number_text", (f.getattr("id")?, v))?;
                if !text.is_none() {
                    texts.set_item(f.getattr("id")?, text)?;
                }
            }
        }
        Ok(form)
    }

    fn _changed(slf: &Slf<'_>, form: &Bound<'_, PyAny>, fid: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let Some(row) = slf.getattr("_rows")?.call_method1("get", (fid,))?.extract::<Option<Bound<'_, PyAny>>>()? else { return Ok(()) };
        if row.getattr("put")?.is_none() {
            return Ok(());
        }
        let f = form.call_method1("field", (fid,))?;
        let texts = form.getattr("_texts")?;
        if !f.is_none() && f.getattr("kind")?.eq("number")? && texts.contains(fid)? {
            state(slf)?.call_method1("type_number", (fid, texts.get_item(fid)?))?;
        }
        slf.call_method1("apply", (row, value))?;
        Ok(())
    }

    /// `row` set to `value` at once; a refusal (a file that is no picture,
    /// a colour that is none) shows on top of the sheet.
    fn apply(slf: &Slf<'_>, row: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = slf.py();
        let envb = env(&win(slf)?)?;
        envb.setattr("state", state(slf)?)?;
        let start = row.getattr("start")?;
        let before = if start.is_none() { row.call_method1("value", (&envb,))? } else { start.call1((&envb,))? };
        if let Err(e) = row.getattr("put")?.call1((&envb, value)) {
            let known = e.is_instance_of::<pyo3::exceptions::PyValueError>(py) || e.is_instance_of::<pyo3::exceptions::PyKeyError>(py) || e.is_instance_of::<pyo3::exceptions::PyOSError>(py);
            if known {
                let text = e.value(py).str()?.to_string();
                return note(slf, text.trim_matches(|c| c == '\'' || c == '"'), "error");
            }
            if !e.is_instance_of::<pyo3::exceptions::PyException>(py) {
                return Err(e);
            }
            e.print(py);
            let label = row.getattr("label")?;
            let what = if label.is_truthy()? { label } else { row.getattr("id")? };
            return note(slf, &trf(py, "{what} could not be changed: {why}", &[("what", what), ("why", e.value(py).clone().into_any())])?, "error");
        }
        let restart = row.getattr("restart")?;
        if restart.is_truthy()? {
            let rv = row.getattr("restart_value")?;
            let value = if rv.is_none() { value.clone() } else { rv.call1((value,))? };
            let json = py.import("json")?;
            pending(py)?.call_method1("record", (row.getattr("id")?, restart, json.call_method1("dumps", (before,))?, json.call_method1("dumps", (value,))?))?;
        }
        Ok(())
    }

    fn _pressed(slf: &Slf<'_>, _form: &Bound<'_, PyAny>, fid: &Bound<'_, PyAny>) -> PyResult<()> {
        let row = slf.getattr("_rows")?.call_method1("get", (fid,))?;
        if !row.is_none() && !row.getattr("press")?.is_none() {
            row.getattr("press")?.call1((env(&win(slf)?)?,))?;
        }
        Ok(())
    }

    fn placement_sheet<'py>(
        slf: &Slf<'py>,
        ctx: &Bound<'py, PyAny>,
        envb: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let py = slf.py();
        let site = slf.getattr("site")?;
        let cs = chrome(py)?;
        let sheet = cs.getattr("Sheet")?.call(
            ("site-placement", tr(py, "Site placement")?),
            Some(&[("width", 1440)].into_py_dict(py)?),
        )?;
        let map = sheet.call_method0("content")?;
        let inspector = sheet.call_method1("detail", (330.,))?;
        let source = slf.getattr("site_source")?;
        let mut requests = slf.getattr("site_requests")?;
        if requests.is_none() {
            requests = site_requests::create(
                &site,
                source.clone(),
                ctx.getattr("refresh")?,
                ctx.getattr("win")?,
            )?
            .into_any();
            slf.setattr("site_requests", &requests)?;
        }
        let root = requests
            .getattr("tiles")?
            .getattr("cache")?
            .getattr("root")?
            .str()?;
        let translate = py.import("built_core.i18n")?.getattr("tr")?;
        let json = py.import("json")?;
        let blocks = json.call_method1(
            "loads",
            (site.call_method1(
                "map_blocks",
                (
                    &translate,
                    source.getattr("id")?,
                    root,
                    source.getattr("max_zoom")?,
                    source.getattr("attribution")?,
                ),
            )?,),
        )?;
        map.getattr("blocks")?
            .call_method1("append", (blocks.get_item(0)?,))?;
        let edit = slf.getattr("site_edit")?.unbind();
        let put = func(py, move |py, a| Ok(edit.bind(py).call(a, None)?.unbind()))?;
        let groups = crate::settings_rows::native_groups(
            py,
            &site
                .call_method1("groups", (&translate,))?
                .extract::<String>()?,
            put,
        )?;
        for group in groups.iter() {
            let id = group.get_item(0)?;
            inspector.call_method1("caption", (group.get_item(1)?,))?;
            let form = slf.call_method1("_form", (envb, group.get_item(2)?, &id))?;
            inspector.call_method("fields", (form,), Some(&[("id", &id)].into_py_dict(py)?))?;
            if id.eq("locate")? {
                for block in blocks.try_iter()?.skip(1) {
                    inspector
                        .getattr("blocks")?
                        .call_method1("append", (block?,))?;
                }
            }
            if id.eq("crs")? {
                inspector.call_method1(
                    "buttons",
                    (vec![cs.call_method1(
                        "verb",
                        ("site_load", tr(py, "Load .prj or calibration…")?),
                    )?],),
                )?;
            }
        }
        inspector.call_method1("caption", (tr(py, "Check alignment")?,))?;
        let spec = blocks.get_item(0)?.get_item("map")?;
        let layers = PyDict::new(py);
        layers.set_item("kind", "site_layers")?;
        if spec.get_item("footprint")?.len()? >= 3 {
            layers.set_item("text", tr(py, "Show footprint")?)?;
        }
        if spec.get_item("markers")?.len()? > 0 {
            layers.set_item("sub", tr(py, "Show survey reference")?)?;
            layers.set_item("right", tr(py, "Reference opacity")?)?;
        }
        inspector.getattr("blocks")?.call_method1("append", (layers,))?;
        inspector.call_method1(
            "text",
            (site.call_method1("placement_check", (&translate,))?,),
        )?;
        if site.getattr("note")?.is_truthy()? {
            inspector.call_method1("text", (site.getattr("note")?,))?;
        }
        sheet.call_method1("footer", (vec![("site_cancel", tr(py, "Cancel")?)],))?;
        let action = cs.call_method1("verb", ("site_apply", tr(py, "Apply")?))?;
        action.set_item("role", 1)?;
        action.set_item("enabled", !site.call_method0("parameters")?.is_none())?;
        sheet.call_method1("footer_action", (action,))?;
        Ok(sheet)
    }

    fn build<'py>(slf: &Slf<'py>, ctx: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let py = slf.py();
        slf.setattr("_win", ctx.getattr("win")?)?;
        let envb = env(&ctx.getattr("win")?)?;
        envb.setattr("state", state(slf)?)?;
        let section: String = state(slf)?.getattr("section")?.extract()?;
        if !slf.getattr("site")?.is_none() && (!envb.getattr("scene")?.is(&slf.getattr("site_scene")?) || section != "location") {
            slf.call_method0("end_site")?;
        }
        let query = state(slf)?.getattr("query")?.str()?.to_string().trim().to_lowercase();
        slf.call_method1("_monitor", (crate::settings_rows::NDOF_SUPPORTED && section == "ndof" && query.is_empty(),))?;
        envb.setattr("device_readings", slf.getattr("_readings")?)?;
        slf.setattr("_rows", PyDict::new(py))?;
        let draft=slf.getattr("site")?;
        if !draft.is_none() {
            if draft.getattr("placing")?.is_truthy()? {return Self::placement_sheet(slf,ctx,&envb);}
            let sheet=chrome(py)?.getattr("Sheet")?.call1(("site-capture",tr(py,"Capture area")?))?;
            Self::section_pane(slf,&sheet,&envb,&py.None().into_bound(py))?;
            return Ok(sheet);
        }
        let cs = chrome(py)?;
        let sheet = cs.getattr("Sheet")?.call(("settings", tr(py, "Settings")?), Some(&[("width", 1100)].into_py_dict(py)?))?;
        sheet.call_method1("search", (tr(py, "Search settings")?,))?;
        let keyboard = if !query.is_empty() || section == "keyboard" { slf.call_method0("_keyboard_snapshot")? } else { py.None().into_bound(py) };
        let hits = if query.is_empty() { None } else { Some(Self::hits(slf, &envb, &query, keyboard.get_item("count")?.extract()?)?) };
        for (scope, caption) in [(MAC, tr(py, "Built · this Mac")?), (MODEL, tr(py, "This model")?)] {
            sheet.call_method1("side_caption", (caption,))?;
            for (sid, label, sc) in sections(py)? {
                if sc != scope {
                    continue;
                }
                let mut right = String::new();
                if let Some(h) = &hits {
                    let n = match h.get_item(&sid)? {
                        Some(v) if sid == "keyboard" => v.extract::<i64>()?,
                        Some(v) => v.len()? as i64,
                        None => 0,
                    };
                    if n != 0 {
                        right = n.to_string();
                    }
                }
                let kw = PyDict::new(py);
                kw.set_item("on", hits.is_none() && sid == section)?;
                kw.set_item("right", &right)?;
                kw.set_item("dim", hits.is_some() && right.is_empty())?;
                sheet.call_method("side_item", (&sid, tr(py, &label)?), Some(&kw))?;
            }
        }
        match &hits {
            Some(h) => Self::search_pane(slf, &sheet, &envb, h, &keyboard)?,
            None => Self::section_pane(slf, &sheet, &envb, &keyboard)?,
        }
        if section != "registry" && slf.getattr("site")?.is_none() {
            let hint = format!("{} · {}", tr(py, "Changes apply at once")?, tr(py, "esc back to the model")?);
            sheet.call_method1("footer", (vec![("restore", tr(py, "Restore defaults")?)], hint))?;
        } else {
            sheet.call_method("footer", (), Some(&[("hint", tr(py, "esc back to the model")?)].into_py_dict(py)?))?;
        }
        let focus = state(slf)?.getattr("kb_focus")?;
        if focus.is_truthy()? && (section == "keyboard" || hits.is_some()) {
            let take = state(slf)?.call_method0("take_focus")?;
            sheet.call_method("focus", (focus,), Some(&[("force", take)].into_py_dict(py)?))?;
        }
        Ok(sheet)
    }

    fn site_edit(slf: &Slf<'_>, envb: &Bound<'_, PyAny>, key: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let site = slf.getattr("site")?;
        if !site.is_none() && envb.getattr("scene")?.is(&slf.getattr("site_scene")?) && site.call_method1("edit", (key, value.str()?))?.is_truthy()? {
            envb.getattr("st")?.call_method1("setValue", ("georef/coord_mode", site.getattr("mode")?))?;
        }
        Ok(())
    }

    fn end_site(slf: &Slf<'_>) -> PyResult<()> {
        let py = slf.py();
        let requests = slf.getattr("site_requests")?;
        if !requests.is_none() {
            requests.call_method0("close")?;
            slf.setattr("site_requests", py.None())?;
        }
        for name in ["site", "site_scene", "site_done", "site_source"] {
            slf.setattr(name, py.None())?;
        }
        Ok(())
    }

    #[pyo3(signature = (ctx, action, args=None))]
    fn site_action(slf: &Slf<'_>, ctx: &Bound<'_, PyAny>, action: &str, args: Option<Vec<String>>) -> PyResult<()> {
        let py = slf.py();
        let args = args.unwrap_or_default();
        let site = slf.getattr("site")?;
        let envb = env(&ctx.getattr("win")?)?;
        let placing=!site.is_none() && site.getattr("placing")?.is_truthy()?;
        if action=="site_cancel" || !envb.getattr("scene")?.is(&slf.getattr("site_scene")?){
            return chrome(py)?.call_method1("close",(ctx.getattr("win")?,)).map(|_|());
        }
        if placing {
            match action {
                "site_use_camera"=>{site.call_method0("use_camera")?;return Ok(());}
                "site_load"=>{
                    let path=py.import("built_render.shell_files")?.getattr("ShellFileDialog")?.call_method1("getOpenFileName",(ctx.getattr("win")?,tr(py,"Load site placement")?,"","Site placement (*.prj *.json)"))?.get_item(0)?;
                    if path.is_truthy()?{site.call_method1("load_placement",(path,))?;}
                    return Ok(());
                }
                "site_search" if site.call_method0("search_coordinates")?.is_truthy()?=>return Ok(()),
                "site_apply"=>{
                    let params=site.call_method0("parameters")?;
                    if params.is_none(){return Ok(());}
                    let pos=site.call_method0("position")?;
                    let history=crate::settings_rows::apply(py)?.call_method1("document_history",(envb.getattr("vp")?,))?;
                    let result=py.import("built_core.command_protocol")?.call_method1("set_location",(history,pos.get_item(0)?,pos.get_item(1)?,params))?;
                    if result.get_item("ok")?.is_truthy()?{
                        crate::settings_rows::_scene_changed(&envb)?;
                        chrome(py)?.call_method1("close",(ctx.getattr("win")?,))?;
                    }else{site.setattr("note",result.get_item("error")?)?;}
                    return Ok(());
                }
                _=>()
            }
        }
        if site.is_none() {
            return slf.call_method0("end_site").map(|_| ());
        }
        if action == "site_map" {
            site.call_method0("map_toggle")?;
            let map_on = site.getattr("map")?.is_truthy()?;
            let requests = slf.getattr("site_requests")?;
            if map_on && requests.is_none() {
                let made = site_requests::create(&site,slf.getattr("site_source")?,ctx.getattr("refresh")?,ctx.getattr("win")?)?;
                slf.setattr("site_requests", made)?;
            } else if !map_on && !requests.is_none() {
                requests.call_method0("close")?;
                slf.setattr("site_requests", py.None())?;
            }
            return Ok(());
        }
        let requests = slf.getattr("site_requests")?;
        if site.getattr("map")?.is_truthy()? && !requests.is_none() {
            let f = |i: usize| -> PyResult<f64> { args.get(i).map(|a| a.parse::<f64>()).transpose().map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?.ok_or_else(|| pyo3::exceptions::PyIndexError::new_err(i)) };
            match action {
                "site_tiles" => {
                    requests.call_method1("request", (args.clone(),))?;
                }
                "site_center" => {
                    let zoom: i64 = args.get(2).ok_or_else(|| pyo3::exceptions::PyIndexError::new_err(2))?.parse::<f64>().map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))? as i64;
                    site.call_method1("move_pin", (f(0)?, f(1)?, zoom))?;
                }
                "site_rect" => {
                    let v: PyResult<Vec<f64>> = (0..args.len()).map(f).collect();
                    site.call_method1("bounds", (v?,))?;
                }
                "site_search" => {
                    requests.getattr("searcher")?.call_method1("search", (site.getattr("query")?,))?;
                }
                "site_locate" => {
                    requests.getattr("locator")?.call_method0("locate")?;
                }
                _ => {
                    if site.call_method1("choose", (action,))?.is_truthy()? {
                        return Ok(());
                    }
                }
            }
        }
        let selected = site.call_method0("selected")?;
        if action != "site_apply" || selected.is_none() {
            return Ok(());
        }
        let (lat, lon, width, length) = (selected.get_item(0)?, selected.get_item(1)?, selected.get_item(2)?, selected.get_item(3)?);
        let done = slf.getattr("site_done")?;
        if !done.is_none() {
            slf.call_method0("end_site")?;
            chrome(py)?.call_method1("close", (ctx.getattr("win")?,))?;
            done.call1((lat, lon, width, length))?;
            return Ok(());
        }
        let datum = envb.getattr("scene")?.getattr("georef")?;
        if !datum.is_none() && !site.call_method1("moved", (datum.getattr("lat")?, datum.getattr("lon")?))?.is_truthy()? {
            return slf.call_method0("end_site").map(|_| ());
        }
        if !datum.is_none() {
            let message = tr(py, "This project already has a location. Moving it makes the new point the model's origin (0,0): everything drawn keeps its local coordinates and shows up at the new spot on the map.\n\nMove the origin?")?;
            let kw = [("default", "cancel"), ("cancel", "cancel")].into_py_dict(py)?;
            let answer = py.import("built_render.chrome_dialogs")?.getattr("ask")?.call(
                (ctx.getattr("win")?, tr(py, "Move the project origin?")?, message, vec![("cancel", tr(py, "Cancel")?), ("move", tr(py, "Move")?)]),
                Some(&kw),
            )?;
            if !answer.eq("move")? {
                return Ok(());
            }
        }
        if !ctx.getattr("win")?.getattr("viewport")?.getattr("scene")?.is(&slf.getattr("site_scene")?) {
            return slf.call_method0("end_site").map(|_| ());
        }
        let history = crate::settings_rows::apply(py)?.call_method1("document_history", (envb.getattr("vp")?,))?;
        let result = py.import("built_core.command_protocol")?.call_method1("set_location", (history, lat, lon))?;
        if result.get_item("ok")?.is_truthy()? {
            slf.call_method0("end_site")?;
            crate::settings_rows::_scene_changed(&envb)?;
        } else {
            let mut why = str_of(&result.get_item("error")?)?;
            if why.is_empty() {
                why = str_of(&result.get_item("rejected")?)?;
            }
            if why.is_empty() {
                why = tr(py, "Location could not be changed")?;
            }
            note(slf, &why, "error")?;
        }
        Ok(())
    }

    // -- keyboard ----------------------------------------------------------------------

    fn actions<'py>(slf: &Slf<'py>) -> PyResult<Bound<'py, PyAny>> {
        let py = slf.py();
        if slf.getattr("_actions")?.is_none() {
            let sc = shortcuts(py)?;
            let w = win(slf)?;
            let collect = || -> PyResult<Bound<'py, PyList>> {
                if let Ok(sync) = w.get_type().getattr("refresh_commands") {
                    sync.call1((&w,))?;
                }
                sc.call_method1("remember_defaults", (&w,))?;
                let out = PyList::empty(py);
                for scope in sc.call_method1("keyboard_scopes", (&w,))?.try_iter()? {
                    for a in sc.call_method1("collect_actions", (scope?,))?.try_iter()? {
                        out.append(a?)?;
                    }
                }
                Ok(out)
            };
            // A window with no actions (the shell's offscreen tests).
            let actions = collect().unwrap_or_else(|_| PyList::empty(py));
            slf.setattr("_actions", actions)?;
        }
        slf.getattr("_actions")
    }

    fn paths<'py>(slf: &Slf<'py>) -> PyResult<Bound<'py, PyAny>> {
        let py = slf.py();
        if slf.getattr("_paths")?.is_none() {
            let w = win(slf)?;
            let collect = || -> PyResult<Bound<'py, PyDict>> {
                let out = PyDict::new(py);
                for scope in shortcuts(py)?.call_method1("keyboard_scopes", (&w,))?.try_iter()? {
                    for (k, v) in crate::command_metadata::menu_paths(&scope?)?.iter() {
                        out.set_item(k, v)?;
                    }
                }
                Ok(out)
            };
            slf.setattr("_paths", collect().unwrap_or_else(|_| PyDict::new(py)))?;
        }
        slf.getattr("_paths")
    }

    fn _keyboard_snapshot<'py>(slf: &Slf<'py>) -> PyResult<Bound<'py, PyAny>> {
        let py = slf.py();
        let sc = shortcuts(py)?;
        let paths = slf.call_method0("paths")?;
        let rows = PyList::empty(py);
        for act in slf.call_method0("actions")?.try_iter()? {
            let act = act?;
            let key = sc.call_method1("action_key", (&act,))?;
            let keys = _keys_text(&sc.call_method1("action_keys", (&act,))?)?;
            let defaults = _keys_text(&sc.call_method1("default_keys", (&act,))?)?;
            rows.append((&key, sc.call_method1("action_label", (act.call_method0("text")?,))?, paths.call_method1("get", (&key, ""))?, keys, defaults))?;
        }
        let tr_fn = py.import("built_core.i18n")?.getattr("tr")?;
        let text = state(slf)?.call_method1("keyboard", (rows, tr_fn))?;
        py.import("json")?.call_method1("loads", (text,))
    }

    fn _action<'py>(slf: &Slf<'py>, item: Option<&str>) -> PyResult<Option<Bound<'py, PyAny>>> {
        let py = slf.py();
        let Some(key) = item.and_then(|i| i.strip_prefix("sc:")) else { return Ok(None) };
        let sc = shortcuts(py)?;
        for a in slf.call_method0("actions")?.try_iter()? {
            let a = a?;
            if sc.call_method1("action_key", (&a,))?.eq(key)? {
                return Ok(Some(a));
            }
        }
        Ok(None)
    }

    /// Give `act` these keys, taking them from any other command that holds
    /// them (after asking): two commands on one key both go dead.
    #[pyo3(signature = (act, seqs, ask=true))]
    fn _assign(slf: &Slf<'_>, act: &Bound<'_, PyAny>, seqs: &Bound<'_, PyAny>, ask: bool) -> PyResult<()> {
        let py = slf.py();
        let sc = shortcuts(py)?;
        let actions = PyDict::new(py);
        for a in slf.call_method0("actions")?.try_iter()? {
            let a = a?;
            actions.set_item(sc.call_method1("action_key", (&a,))?, a)?;
        }
        let seqs: Vec<String> = crate::keyboard_bindings::portable_keys(seqs)?;
        let mut current = Vec::new();
        for (k, a) in actions.iter() {
            current.push((k.extract::<String>()?, sc.call_method1("action_keys", (a,))?.extract::<Vec<String>>()?));
        }
        let target: String = sc.call_method1("action_key", (act,))?.extract()?;
        let clash = crate::commands::shortcut_conflicts(&target, seqs.iter().cloned().collect(), current);
        let me = slf.clone().unbind();
        let (act_ref, actions_ref, seqs_go, clash_go) = (act.clone().unbind(), actions.clone().unbind(), seqs.clone(), clash.clone());
        let go = func(py, move |py, _a| {
            let sc = shortcuts(py)?;
            let actions = actions_ref.bind(py);
            for (key, remaining) in &clash_go {
                let a = actions.get_item(key)?.ok_or_else(|| pyo3::exceptions::PyKeyError::new_err(key.clone()))?;
                a.call_method1("setShortcuts", (remaining.clone(),))?;
                sc.call_method1("save_shortcut", (&a, remaining.clone()))?;
            }
            act_ref.bind(py).call_method1("setShortcuts", (seqs_go.clone(),))?;
            sc.call_method1("save_shortcut", (act_ref.bind(py), seqs_go.clone()))?;
            let me = me.bind(py);
            state(me)?.setattr("note", py.None())?;
            refresh(me)?;
            Ok(none(py))
        })?;
        if !clash.is_empty() && ask {
            let mut names = Vec::new();
            for (k, _) in &clash {
                let a = actions.get_item(k)?.ok_or_else(|| pyo3::exceptions::PyKeyError::new_err(k.clone()))?;
                names.push(sc.call_method1("action_label", (a.call_method0("text")?,))?.extract::<String>()?);
            }
            let keys_list = PyList::new(py, &seqs)?;
            let text = trf(py, "«{keys}» is already used by: {names}. Give it to «{action}» instead?", &[
                ("keys", s(py, &_keys_text(keys_list.as_any())?)),
                ("names", s(py, &names.join(", "))),
                ("action", sc.call_method1("action_label", (act.call_method0("text")?,))?),
            ])?;
            return confirm(slf, &tr(py, "Keyboard shortcuts")?, &text, &tr(py, "Give it")?, go);
        }
        go.bind(py).call0()?;
        Ok(())
    }

    fn _confirm(slf: &Slf<'_>, title: &str, text: &str, ok: &str, then: Py<PyAny>) -> PyResult<()> {
        confirm(slf, title, text, ok, then)
    }

    fn _change_keys(slf: &Slf<'_>, act: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = slf.py();
        let sc = shortcuts(py)?;
        let cf = cf(py)?;
        let check = func(py, |py, a| {
            let got = _parse_keys(py, a.get_item(0)?.call_method1("value", ("keys",))?.extract::<Option<String>>()?.as_deref())?;
            Ok(if got.bind(py).is_instance_of::<pyo3::types::PyString>() { got } else { none(py) })
        })?;
        let (me, act_ref) = (slf.clone().unbind(), act.clone().unbind());
        let done = func(py, move |py, a| {
            if !a.get_item(0)?.eq("ok")? {
                return Ok(none(py));
            }
            let keys = a.get_item(1)?.call_method1("get", ("keys", ""))?;
            let got = _parse_keys(py, keys.extract::<Option<String>>()?.as_deref())?;
            if !got.bind(py).is_instance_of::<pyo3::types::PyString>() {
                me.bind(py).call_method1("_assign", (act_ref.bind(py), got))?;
            }
            Ok(none(py))
        })?;
        let first: Vec<String> = sc.call_method1("action_keys", (act,))?.extract()?;
        let first = PyList::new(py, first.into_iter().take(1))?;
        let field = cf.getattr("Field")?.call(("keys", "edit", tr(py, "Keys")?, _keys_text(first.as_any())?), Some(&[("focus", true)].into_py_dict(py)?))?;
        let kw = PyDict::new(py);
        kw.set_item("note", tr(py, "Write the keys, like ⌘⇧K or Ctrl+Shift+K. Empty: no keys.")?)?;
        kw.set_item("validate", check)?;
        kw.set_item("on_done", done)?;
        kw.set_item("width", 420)?;
        let title = trf(py, "Keys for «{action}»", &[("action", sc.call_method1("action_label", (act.call_method0("text")?,))?)])?;
        let form = cf.getattr("Form")?.call((title, vec![field]), Some(&kw))?;
        cf.getattr("show")?.call1((win(slf)?, form))?;
        Ok(())
    }

    // -- answers -----------------------------------------------------------------------

    fn act(slf: &Slf<'_>, ctx: &Bound<'_, PyAny>, verb: &str, args: Vec<String>) -> PyResult<()> {
        let py = slf.py();
        slf.setattr("_win", ctx.getattr("win")?)?;
        let arg = args.first().cloned().unwrap_or_default();
        match verb {
            "search" => state(slf)?.setattr("query", &arg)?,
            "site_center" | "site_rect" | "site_tiles" => {
                slf.call_method1("site_action", (ctx, verb, args))?;
            }
            "side" => {
                let searched = state(slf)?.getattr("query")?.is_truthy()?;
                state(slf)?.call_method1("goto", (&arg, py.None()))?;
                if searched {
                    chrome(py)?.getattr("open")?.call((ctx.getattr("win")?, slf), Some(&[("search", "")].into_py_dict(py)?))?;
                }
            }
            "focus" if arg.starts_with("sc:") => state(slf)?.setattr("kb_focus", &arg)?,
            "open" if arg.starts_with("sc:") => {
                state(slf)?.setattr("kb_focus", &arg)?;
                if let Some(a) = slf.call_method1("_action", (&arg,))?.extract::<Option<Bound<'_, PyAny>>>()? {
                    slf.call_method1("_change_keys", (a,))?;
                }
            }
            "menu" if args.len() > 1 && arg.starts_with("sc:") => {
                state(slf)?.setattr("kb_focus", &arg)?;
                slf.call_method1("_kb_verb", (format!("kb_{}", args[1]),))?;
            }
            "verb" => {
                if arg.starts_with("registry_") && state(slf)?.getattr("section")?.eq("registry")? {
                    let editor = slf.getattr("registry")?;
                    let w = win(slf)?;
                    let mut path = s(py, "");
                    if arg == "registry_csv" {
                        path = py.import("built_render.shell_files")?.getattr("ShellFileDialog")?.call_method1("getOpenFileName", (&w, "Codes CSV", "", "CSV (*.csv)"))?.get_item(0)?;
                    }
                    if editor.call_method1("act", (&w, &arg, path))?.is_truthy()? && arg == "registry_apply" {
                        chrome(py)?.call_method1("close", (&w,))?;
                    }
                } else if arg.starts_with("site_") {
                    slf.call_method1("site_action", (ctx, &arg))?;
                } else if arg.starts_with("kb_") {
                    slf.call_method1("_kb_verb", (&arg,))?;
                } else if arg.starts_with("ndof_") {
                    slf.call_method1("_ndof_verb", (&arg,))?;
                } else if arg == "restore" {
                    let (me, w) = (slf.clone().unbind(), ctx.getattr("win")?.unbind());
                    let then = func(py, move |py, _a| {
                        me.bind(py).call_method1("restore_defaults", (w.bind(py),))?;
                        Ok(none(py))
                    })?;
                    confirm(slf, &tr(py, "Restore defaults")?, &tr(py, "Restore default application settings? This keeps your language, keyboard shortcuts and model settings.")?, &tr(py, "Restore defaults")?, then)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn _kb_verb(slf: &Slf<'_>, verb: &str) -> PyResult<()> {
        let py = slf.py();
        let sc = shortcuts(py)?;
        let focus = state(slf)?.getattr("kb_focus")?;
        let act: Option<Bound<'_, PyAny>> = slf.call_method1("_action", (focus,))?.extract()?;
        match (verb, act) {
            ("kb_change", Some(a)) => {
                slf.call_method1("_change_keys", (a,))?;
            }
            ("kb_clear", Some(a)) => {
                slf.call_method("_assign", (a, PyList::empty(py)), Some(&[("ask", false)].into_py_dict(py)?))?;
            }
            ("kb_default", Some(a)) => {
                let keys = sc.call_method1("default_keys", (&a,))?;
                slf.call_method1("_assign", (a, keys))?;
            }
            ("kb_export", _) => {
                slf.call_method0("_kb_export")?;
            }
            ("kb_import", _) => {
                slf.call_method0("_kb_import")?;
            }
            ("kb_reset_all", _) => {
                let me = slf.clone().unbind();
                let reset = func(py, move |py, _a| {
                    let me = me.bind(py);
                    let sc = shortcuts(py)?;
                    let st = py.import("built_core.settings")?.call_method0("settings")?;
                    st.call_method1("remove", (crate::keyboard_bindings::GROUP,))?;
                    st.call_method0("sync")?;
                    for a in me.call_method0("actions")?.try_iter()? {
                        let a = a?;
                        a.call_method1("setShortcuts", (sc.call_method1("default_keys", (&a,))?,))?;
                    }
                    let w = win(me)?;
                    for host in py.import("built_render.shell_windows")?.call_method0("hosts")?.try_iter()? {
                        let host = host?;
                        if !host.getattr("window")?.is(&w) {
                            for scope in sc.call_method1("keyboard_scopes", (host.getattr("window")?,))?.try_iter()? {
                                for a in scope?.getattr("_registry_actions")?.call_method0("values")?.try_iter()? {
                                    let a = a?;
                                    a.call_method1("setShortcuts", (sc.call_method1("default_keys", (&a,))?,))?;
                                }
                            }
                        }
                    }
                    refresh(me)?;
                    Ok(none(py))
                })?;
                confirm(slf, &tr(py, "Keyboard shortcuts")?, &tr(py, "Put every action back on its factory shortcut?")?, &tr(py, "Restore all")?, reset)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn _parent(slf: &Slf<'_>) -> PyResult<Py<PyAny>> {
        let w = win(slf)?;
        Ok(if w.hasattr("isWindow")? { w.unbind() } else { slf.py().None() })
    }

    fn _kb_export(slf: &Slf<'_>) -> PyResult<()> {
        let py = slf.py();
        let dialogs = py.import("built_render.shell_files")?.getattr("ShellFileDialog")?;
        let path = dialogs.call_method1("getSaveFileName", (Self::_parent(slf)?, tr(py, "Export shortcuts")?, "built-shortcuts.json", tr(py, "Shortcuts (*.json)")?))?.get_item(0)?;
        if !path.is_truthy()? {
            return Ok(());
        }
        let data = shortcuts(py)?.call_method1("export_shortcuts", (win(slf)?,))?;
        let kw = PyDict::new(py);
        kw.set_item("indent", 1)?;
        kw.set_item("ensure_ascii", false)?;
        let text: String = py.import("json")?.getattr("dumps")?.call((data,), Some(&kw))?.extract()?;
        let path = path.str()?.to_string();
        if let Err(e) = std::fs::write(&path, text) {
            return note(slf, &e.to_string(), "error");
        }
        note(slf, &trf(py, "Shortcuts saved to {path}.", &[("path", s(py, &path))])?, "ok")
    }

    fn _kb_import(slf: &Slf<'_>) -> PyResult<()> {
        let py = slf.py();
        let sc = shortcuts(py)?;
        let dialogs = py.import("built_render.shell_files")?.getattr("ShellFileDialog")?;
        let path = dialogs.call_method1("getOpenFileName", (Self::_parent(slf)?, tr(py, "Import shortcuts")?, "", tr(py, "Shortcuts (*.json)")?))?.get_item(0)?;
        if !path.is_truthy()? {
            return Ok(());
        }
        let read = (|| -> PyResult<Bound<'_, PyAny>> {
            let text = std::fs::read_to_string(path.str()?.to_string()).map_err(|e| pyo3::exceptions::PyOSError::new_err(e.to_string()))?;
            let data = py.import("json")?.call_method1("loads", (text,))?;
            sc.call_method1("import_shortcuts", (win(slf)?, data))
        })();
        let result = match read {
            Ok(r) => r,
            Err(e) if e.is_instance_of::<pyo3::exceptions::PyOSError>(py) || e.is_instance_of::<pyo3::exceptions::PyValueError>(py) => {
                return note(slf, &e.value(py).str()?.to_string(), "error");
            }
            Err(e) => return Err(e),
        };
        let (applied, skipped, clashes) = (result.get_item(0)?, result.get_item(1)?, result.get_item(2)?);
        let mut msg = trf(py, "{n} shortcuts imported.", &[("n", applied)])?;
        if skipped.is_truthy()? {
            msg.push(' ');
            msg.push_str(&trf(py, "{n} belong to actions this installation does not have (a plugin, say) and were left out.", &[("n", skipped)])?);
        }
        let any_clash = clashes.is_truthy()?;
        if any_clash {
            let mut parts = Vec::new();
            for c in clashes.try_iter()? {
                let c = c?;
                let keys = PyList::new(py, [c.get_item(0)?])?;
                parts.push(trf(py, "{key}: kept for «{kept}», taken from «{dropped}»", &[
                    ("key", s(py, &_keys_text(keys.as_any())?)),
                    ("kept", sc.call_method1("action_label", (c.get_item(1)?.call_method0("text")?,))?),
                    ("dropped", sc.call_method1("action_label", (c.get_item(2)?.call_method0("text")?,))?),
                ])?);
            }
            msg.push(' ');
            msg.push_str(&tr(py, "The file gave the same key to more than one command; each stays with the first:")?);
            msg.push(' ');
            msg.push_str(&parts.join("; "));
        }
        note(slf, &msg, if any_clash { "warn" } else { "ok" })
    }

    fn _ndof_verb(slf: &Slf<'_>, verb: &str) -> PyResult<()> {
        let py = slf.py();
        let envb = env(&win(slf)?)?;
        match verb {
            "ndof_reset" => {
                let edits = PyDict::new(py);
                edits.set_item("ndof_preset", "original")?;
                edits.set_item("ndof_lock", false)?;
                crate::settings_rows::device_edit(&envb, &edits, None)?;
            }
            "ndof_load" => {
                let dialogs = py.import("built_render.shell_files")?.getattr("ShellFileDialog")?;
                let path = dialogs.call_method1("getOpenFileName", (Self::_parent(slf)?, tr(py, "Load SpaceMouse profile")?, "", "JSON (*.json)"))?.get_item(0)?;
                if !path.is_truthy()? {
                    return Ok(());
                }
                let profile = match crate::preferences::device_profile::load_profile(py, path.extract()?) {
                    Ok(p) => p,
                    Err(e) if e.is_instance_of::<pyo3::exceptions::PyOSError>(py) || e.is_instance_of::<pyo3::exceptions::PyValueError>(py) || e.is_instance_of::<pyo3::exceptions::PyUnicodeError>(py) => {
                        return note(slf, &format!("{}: {}", tr(py, "Invalid SpaceMouse profile")?, e.value(py).str()?), "error");
                    }
                    Err(e) => return Err(e),
                };
                let stem = py.import("pathlib")?.getattr("Path")?.call1((&path,))?.getattr("stem")?;
                let name: String = py.import("built_render")?.getattr("DevicePreferences")?.call_method1("profile_name", (stem, true))?.extract()?;
                Self::ndof_add(slf, &envb, &name, &profile)?;
            }
            "ndof_save" => {
                let me = slf.clone().unbind();
                let named = func(py, move |py, a| {
                    let me = me.bind(py);
                    let name = a.get_item(0)?;
                    if name.is_none() {
                        return Ok(none(py));
                    }
                    let name: String = match py.import("built_render")?.getattr("DevicePreferences")?.call_method1("profile_name", (name, false)) {
                        Ok(n) => n.extract()?,
                        Err(e) if e.is_instance_of::<pyo3::exceptions::PyValueError>(py) => {
                            note(me, &tr(py, &e.value(py).str()?.to_string())?, "error")?;
                            refresh(me)?;
                            return Ok(none(py));
                        }
                        Err(e) => return Err(e),
                    };
                    let envb = env(&win(me)?)?;
                    let (model, _) = crate::settings_rows::device_preferences(&envb, None)?;
                    let json = py.import("json")?.call_method1("loads", (model.call_method0("profile")?,))?;
                    let profile = crate::preferences::device_profile::profile_from_dict(&json)?;
                    if crate::settings_rows::apply(py)?.call_method0("read_named_ndof_profiles")?.contains(&name)? {
                        let (me2, name2, profile2) = (me.clone().unbind(), name.clone(), profile.clone().unbind());
                        let replace = func(py, move |py, _a| {
                            let me = me2.bind(py);
                            SettingsSheet::ndof_add(me, &env(&win(me)?)?, &name2, profile2.bind(py))?;
                            Ok(none(py))
                        })?;
                        confirm(me, &tr(py, "Replace profile?")?, &trf(py, "Replace the existing profile '{name}'?", &[("name", s(py, &name))])?, &tr(py, "Replace")?, replace)?;
                        return Ok(none(py));
                    }
                    SettingsSheet::ndof_add(me, &envb, &name, &profile)?;
                    Ok(none(py))
                })?;
                let kw = [("done", named)].into_py_dict(py)?;
                cf(py)?.getattr("ask_text")?.call((win(slf)?, tr(py, "Save SpaceMouse profile")?, tr(py, "Profile name:")?), Some(&kw))?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Every Built setting with a default back to it (Restore defaults,
    /// applied at once). Language, keyboard shortcuts and model settings
    /// keep their values.
    fn restore_defaults(slf: &Slf<'_>, w: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = slf.py();
        slf.setattr("_win", w)?;
        for (sid, _label, scope) in sections(py)? {
            if scope != MAC || sid == "keyboard" {
                continue;
            }
            for g in slf.call_method1("groups", (env(w)?, &sid))?.try_iter()? {
                for r in g?.get_item(2)?.try_iter()? {
                    let r = r?;
                    let default = r.getattr("default")?;
                    if crate::settings_rows::is_no(&default) || r.getattr("put")?.is_none() {
                        continue;
                    }
                    if r.call_method1("value", (env(w)?,))?.ne(&default)? {
                        slf.call_method1("apply", (&r, &default))?;
                    }
                }
            }
        }
        let envb = env(w)?;
        envb.getattr("st")?.call_method1("setValue", (crate::settings_rows::live::NEW_UNITS, "1"))?;
        py.import("built_core.units")?.call_method1("set_ask_at_start", (true,))?;
        chrome(py)?.call_method1("refresh", (w,))?;
        Ok(())
    }

    fn closed(slf: &Slf<'_>, _ctx: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = slf.py();
        slf.setattr("registry", py.None())?;
        slf.call_method0("end_site")?;
        slf.call_method1("_monitor", (false,))?;
        state(slf)?.setattr("note", py.None())?;
        module(py)?.setattr("_north_cmd", py.None())?;
        Ok(())
    }
}

/// Settings on the sheet, on `page` (a section id such as "units") or on
/// Keyboard with `shortcut_of` (a command) focused.
#[pyfunction]
#[pyo3(signature = (win, page=None, shortcut_of=None))]
fn show(py: Python<'_>, win: &Bound<'_, PyAny>, page: Option<String>, shortcut_of: Option<Bound<'_, PyAny>>) -> PyResult<bool> {
    let cs = chrome(py)?;
    let cur = cs.call_method1("current", (win,))?;
    let reuse = cur.is_instance_of::<SettingsSheet>() && cur.getattr("site")?.is_none();
    let sheet = if reuse { cur.clone() } else { module(py)?.getattr("SettingsSheet")?.call0()? };
    sheet.setattr("_win", win)?;
    let has_shortcut = shortcut_of.is_some();
    sheet.call_method1("goto", (page.clone(), shortcut_of))?;
    let focus = if has_shortcut { sheet.getattr("kb_focus")? } else { py.None().into_bound(py) };
    let kw = PyDict::new(py);
    let search_reset = (reuse && cur.is(&sheet)) || page.as_deref().is_some_and(|p| !p.is_empty()) || has_shortcut;
    kw.set_item("search", if search_reset { Some("") } else { None })?;
    kw.set_item("focus", focus)?;
    cs.getattr("open")?.call((win, sheet), Some(&kw))?.is_truthy()
}

pub fn edit_location_now(win: &Bound<'_, PyAny>, source: Option<Bound<'_, PyAny>>, center: Option<(f64, f64)>, done: Option<Bound<'_, PyAny>>) -> PyResult<()> {
    let py = win.py();
    let sheet = module(py)?.getattr("SettingsSheet")?.call1(("location",))?;
    sheet.setattr("_win",win)?;
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let datum = scene.getattr("georef")?;
    let (lat, lon) = match center {
        Some(c) => c,
        None if !datum.is_none() => (datum.getattr("lat")?.extract()?, datum.getattr("lon")?.extract()?),
        None => {
            let sun = py.import("built_core.sun")?;
            (sun.getattr("DEFAULT_LAT")?.extract()?, sun.getattr("DEFAULT_LON")?.extract()?)
        }
    };
    sheet.call_method0("end_site")?;
    let mode = env(win)?.getattr("st")?.call_method1("value", ("georef/coord_mode", "geo"))?;
    sheet.setattr("site", py.import("built_core")?.getattr("LocationDraft")?.call1((lat, lon, mode))?)?;
    if done.is_none() && center.is_none() {
        sheet.getattr("site")?.call_method1("start_placement",(&scene,lat,lon))?;
    }
    sheet.setattr("site_scene", scene)?;
    sheet.setattr("site_done", done)?;
    let source = match source {
        Some(s) => s,
        None => crate::model_panels::basemap::current(&env(win)?.getattr("st")?)?,
    };
    sheet.setattr("site_source", source)?;
    chrome(py)?.call_method1("open", (win,sheet))?;
    Ok(())
}

/// The site placement sheet or capture editor (`done(lat, lon, width,
/// length)` instead of setting the document's location when given).
#[pyfunction]
#[pyo3(signature = (win, *, source=None, center=None, done=None))]
fn edit_location(win: &Bound<'_, PyAny>, source: Option<Bound<'_, PyAny>>, center: Option<(f64, f64)>, done: Option<Bound<'_, PyAny>>) -> PyResult<()> {
    edit_location_now(win, source, center, done)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    let sub = PyModule::new(py, "settings_sheet")?;
    let view = m.getattr("PreferencesView")?;
    sub.add("MAC", MAC)?;
    sub.add("MODEL", MODEL)?;
    sub.add("SECTIONS", view.call_method0("sections")?)?;
    sub.add("_PENDING", m.getattr("RestartPreferences")?.call0()?)?;
    sub.add("_NO", py.import("builtins")?.getattr("object")?.call0()?)?;
    sub.add("_north_cmd", py.None())?;
    // `cf`: the forms module the rows are drawn as (the same module object,
    // so a test replacing `cf.show` replaces `chrome_forms.show`).
    sub.add("cf", m.getattr("chrome_forms")?)?;
    sub.add_class::<SettingsSheet>()?;
    crate::settings_rows::register(&sub)?;
    for f in [wrap_pyfunction!(show, &sub)?, wrap_pyfunction!(edit_location, &sub)?, wrap_pyfunction!(_parse_keys, &sub)?, wrap_pyfunction!(_keys_text, &sub)?] {
        sub.add_function(f)?;
    }
    m.add_submodule(&sub)?;
    py.import("sys")?.getattr("modules")?.set_item("built_render.settings_sheet", &sub)?;
    Ok(())
}

