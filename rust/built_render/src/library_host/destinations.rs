// SPDX-License-Identifier: GPL-3.0-or-later
//! The Library's destinations on the window (`library_store.rs` keeps the
//! folders): the preferences that name them, the items kept inside the
//! open model, the one-time migration of the old saved folders, the
//! browser rows of saved items, the "Save to Library…" drawer every kind
//! uses, and the verbs between destinations (Copy to…, Move to…, Update,
//! Reveal in Finder, Delete).
//!
//! An item kept in the model is its entry under the document's plugin data
//! `library` (`{kind: {id: entry}}`), written through History, so it is
//! saved with the file and undone like any edit. A component's "This
//! model" is the model's own definition kept when unused, and a preset's is
//! the model's preset table (the Sweep and Cut pickers read both already).
use super::*;
use crate::library_store as store;
use crate::settings_rows::{tr, trf};
use pyo3::exceptions::PyValueError;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::Ordering;

pub(crate) const ROOT_KEY: &str = "library/root";
pub(crate) const SHARED_KEY: &str = "library/shared";
const LAST_KEY: &str = "library/last_destination";
const MODEL_KEY: &str = "library";
/// The library folders migrated in this process.
static MIGRATED: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

fn err(text: impl Into<String>) -> PyErr {
    PyValueError::new_err(text.into())
}

fn to_value(py: Python<'_>, v: &Bound<'_, PyAny>) -> PyResult<Value> {
    serde_json::from_str(&dumps(py, v)?).map_err(|e| err(e.to_string()))
}

fn to_py<'py>(py: Python<'py>, v: &Value) -> PyResult<Bound<'py, PyAny>> {
    json(py)?.call_method1("loads", (v.to_string(),))
}

// ---- preferences ------------------------------------------------------------------------------

pub(crate) fn pref(py: Python<'_>, key: &str) -> PyResult<String> {
    let v = store(py)?.call_method1("value", (key,))?;
    Ok(if v.is_none() { String::new() } else { v.str()?.to_string() })
}

pub(crate) fn set_pref(py: Python<'_>, key: &str, value: &str) -> PyResult<()> {
    let st = store(py)?;
    st.call_method1("setValue", (key, value))?;
    if st.hasattr("sync")? {
        st.call_method0("sync")?;
    }
    Ok(())
}

/// The settings profile (tests and `--settings` keep everything inside it).
pub(crate) fn profile() -> Option<PathBuf> {
    std::env::var_os("BUILT_SETTINGS_DIR").map(PathBuf::from).filter(|p| !p.as_os_str().is_empty())
}

pub(crate) fn config(py: Python<'_>) -> PyResult<store::Config> {
    Ok(store::Config::from_settings(&pref(py, ROOT_KEY)?, &pref(py, SHARED_KEY)?))
}

pub(crate) fn default_mine() -> Option<PathBuf> {
    store::default_mine(profile().as_deref())
}

pub(crate) fn mine_root(py: Python<'_>) -> PyResult<Option<PathBuf>> {
    Ok(config(py)?.mine.or_else(default_mine))
}

fn live_scene<'py>(scene: Option<&Bound<'py, PyAny>>) -> Option<Bound<'py, PyAny>> {
    scene.filter(|s| !s.is_none() && s.hasattr("_root").unwrap_or(false) && s.hasattr("plugin_data").unwrap_or(false)).cloned()
}

/// The `.builtproject` the model belongs to, if any.
pub(crate) fn manifest(py: Python<'_>, scene: Option<&Bound<'_, PyAny>>) -> Option<PathBuf> {
    let scene = live_scene(scene)?;
    let p = py.import("built_core.project").ok()?.call_method1("path_for", (scene,)).ok()?;
    if p.is_none() {
        return None;
    }
    p.extract::<PathBuf>().ok().filter(|p| p.is_file())
}

pub(crate) fn all(py: Python<'_>, scene: Option<&Bound<'_, PyAny>>) -> PyResult<Vec<store::Destination>> {
    let cfg = config(py)?;
    let has_model = live_scene(scene).is_some();
    Ok(store::destinations(&cfg, has_model, manifest(py, scene).as_deref(), default_mine().as_deref()))
}

fn last(py: Python<'_>, kind: &str) -> PyResult<Option<String>> {
    let map: Value = serde_json::from_str(&pref(py, LAST_KEY)?).unwrap_or(json!({}));
    Ok(map[kind].as_str().map(str::to_owned))
}

fn remember(py: Python<'_>, kind: &str, key: &str) -> PyResult<()> {
    let mut map: Value = serde_json::from_str(&pref(py, LAST_KEY)?).unwrap_or(json!({}));
    if !map.is_object() {
        map = json!({});
    }
    map[kind] = json!(key);
    set_pref(py, LAST_KEY, &map.to_string())
}

fn changed() {
    collections::REVISION.fetch_add(1, Ordering::Relaxed);
}

// ---- inside the model ------------------------------------------------------------------------

fn model_map(py: Python<'_>, scene: &Bound<'_, PyAny>) -> PyResult<Value> {
    let raw = scene.getattr("plugin_data")?.call_method1("get", (MODEL_KEY,))?;
    Ok(if raw.is_none() { json!({}) } else { to_value(py, &raw)? })
}

pub(crate) fn model_entries(py: Python<'_>, scene: &Bound<'_, PyAny>, kind: &str) -> PyResult<Vec<store::Entry>> {
    let map = model_map(py, scene)?;
    let mut out: Vec<store::Entry> = map[kind]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(_, v)| serde_json::from_value::<store::Entry>(v.clone()).ok())
        .filter(|e| e.check().is_ok() && e.kind == kind)
        .collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(out)
}

/// Change the model's entries in one History step.
fn model_edit(py: Python<'_>, vp: &Bound<'_, PyAny>, scene: &Bound<'_, PyAny>, edit: impl FnOnce(&mut Value)) -> PyResult<()> {
    let mut map = model_map(py, scene)?;
    if !map.is_object() {
        map = json!({});
    }
    edit(&mut map);
    map.as_object_mut().expect("object").retain(|_, v| v.as_object().is_some_and(|o| !o.is_empty()));
    let cmd = py.import("built_core.history")?.call_method1("SetPluginDataCommand", (MODEL_KEY, to_py(py, &map)?))?;
    vp.getattr("history")?.call_method1("execute", (cmd,))?;
    if vp.hasattr("notify_scene_changed")? {
        vp.call_method0("notify_scene_changed")?;
    }
    changed();
    Ok(())
}

// ---- migration -------------------------------------------------------------------------------

/// Where the old saved profiles and presets were (still read).
pub(crate) fn legacy_dirs() -> Option<(PathBuf, PathBuf)> {
    let base = crate::platform_paths::local_data_directory(profile())?.join("Built");
    Some((base.join("profiles"), base.join("presets")))
}

/// Copy the old saved folders and My library's materials into My library,
/// once per library folder. The old files stay, and stay listed until
/// their copy is there.
pub(crate) fn ensure_migrated(py: Python<'_>) -> PyResult<Option<usize>> {
    let Some(root) = mine_root(py)? else { return Ok(None) };
    {
        let mut done = MIGRATED.lock().map_err(|_| err("migration lock"))?;
        if done.contains(&root) {
            return Ok(None);
        }
        done.push(root.clone());
    }
    let (profiles, presets) = legacy_dirs().unzip();
    let materials = pref(py, MINE_KEY)?;
    let styles = pref(py, "styles/user")?;
    let legacy = store::Legacy {
        profiles: profiles.as_deref(),
        presets: presets.as_deref(),
        materials_json: Some(materials.as_str()).filter(|m| !m.is_empty()),
    };
    let result = store::migrate(&root, &legacy).and_then(|n| {
        // Saved styles (kept in the preferences) come along too.
        if n.is_some() {
            for e in legacy_styles(&styles) {
                if store::read(&root, "styles", &e.id).is_none() {
                    store::write(&root, &e, None, &[])?;
                }
            }
        }
        Ok(n)
    });
    match result {
        Ok(n) => {
            if n.is_some() {
                changed();
            }
            Ok(n)
        }
        // A library folder that cannot be written is reported when saving.
        Err(_) => Ok(None),
    }
}

fn legacy_styles(json_text: &str) -> Vec<store::Entry> {
    let rows: Vec<Value> = serde_json::from_str(json_text).unwrap_or_default();
    rows.into_iter()
        .filter_map(|r| {
            let name = r["name"].as_str().filter(|n| !n.trim().is_empty())?.to_owned();
            Some(store::Entry::new("styles", &store::stable_id(&format!("style:{name}")), &name, r))
        })
        .collect()
}

#[pyfunction]
#[pyo3(name = "migrate")]
fn migrate_now(py: Python<'_>) -> PyResult<Option<usize>> {
    if let Some(root) = mine_root(py)? {
        MIGRATED.lock().map_err(|_| err("migration lock"))?.retain(|r| r != &root);
    }
    ensure_migrated(py)
}

// ---- listing ---------------------------------------------------------------------------------

/// One saved item and where it is.
pub(crate) struct Listed {
    pub dest: store::Destination,
    pub entry: store::Entry,
}

/// Every saved item of `kinds` in every destination (the model's first).
pub(crate) fn entries(py: Python<'_>, scene: Option<&Bound<'_, PyAny>>, kinds: &[&str]) -> PyResult<Vec<Listed>> {
    ensure_migrated(py)?;
    let mut out = Vec::new();
    for dest in all(py, scene)? {
        for kind in kinds {
            if dest.key == "model" {
                if let Some(scene) = live_scene(scene) {
                    out.extend(model_entries(py, &scene, kind)?.into_iter().map(|entry| Listed { dest: dest.clone(), entry }));
                }
            } else if let Some(root) = dest.root.as_deref().filter(|_| dest.writable || dest.key.starts_with("shared:")) {
                out.extend(store::list(root, kind).into_iter().map(|(_, entry)| Listed { dest: dest.clone(), entry }));
            }
        }
    }
    Ok(out)
}

fn find(py: Python<'_>, scene: Option<&Bound<'_, PyAny>>, dest_key: &str, kind: &str, id: &str) -> PyResult<Option<Listed>> {
    Ok(entries(py, scene, &[kind])?.into_iter().find(|l| l.dest.key == dest_key && l.entry.id == id))
}

/// A saved item's browser key: `lib:<kind>:<id>:<destination>` (kinds and
/// ids have no colon; a shared library's name may).
pub(crate) fn item_key(dest: &str, kind: &str, id: &str) -> String {
    format!("lib:{kind}:{id}:{dest}")
}

/// (destination, kind, id) of a saved item's browser key.
pub(crate) fn parse_key(key: &str) -> Option<(String, String, String)> {
    let (kind, rest) = key.strip_prefix("lib:")?.split_once(':')?;
    let (id, dest) = rest.split_once(':')?;
    Some((dest.to_owned(), kind.to_owned(), id.to_owned()))
}

/// A path inside an item's files, absolute (a material's image).
fn resolve(dest: &store::Destination, e: &store::Entry, file: &str) -> String {
    match &dest.root {
        Some(root) if !file.is_empty() && !Path::new(file).is_absolute() => {
            store::files_dir(root, &e.kind, &e.id).join(file).to_string_lossy().into_owned()
        }
        _ => file.to_owned(),
    }
}

/// A material's dictionary with its image path made absolute, for Paint.
fn material_recipe(l: &Listed) -> Value {
    let mut data = l.entry.data.clone();
    if let Some(path) = data["texture"]["path"].as_str().map(str::to_owned) {
        data["texture"]["path"] = json!(resolve(&l.dest, &l.entry, &path));
    }
    if !data["name"].is_string() {
        data["name"] = json!(l.entry.name);
    }
    data
}

fn hex_of(rgb: &Value) -> String {
    let c: Vec<f64> = rgb.as_array().into_iter().flatten().filter_map(Value::as_f64).collect();
    if c.len() < 3 {
        return String::new();
    }
    format!("#{:02x}{:02x}{:02x}", (c[0].clamp(0., 1.) * 255.) as u8, (c[1].clamp(0., 1.) * 255.) as u8, (c[2].clamp(0., 1.) * 255.) as u8)
}

/// The browser rows of every saved item of `kinds`: a source label, the
/// item's words, and the verbs its destination allows.
pub(crate) fn items(py: Python<'_>, scene: Option<&Bound<'_, PyAny>>, kinds: &[&str]) -> PyResult<Vec<Value>> {
    let listed = entries(py, scene, kinds)?;
    let newest = store::newest(listed.iter().map(|l| (l.dest.key.as_str(), &l.entry)));
    let dests = all(py, scene)?;
    let labels: std::collections::HashMap<&str, &str> = dests.iter().map(|d| (d.key.as_str(), d.label.as_str())).collect();
    let mut out = Vec::new();
    for l in &listed {
        let e = &l.entry;
        let writable = l.dest.writable;
        let mut verbs = vec!["apply", "-"];
        if writable {
            verbs.push("libedit");
        }
        verbs.push("copyto");
        if writable {
            verbs.push("moveto");
        }
        let (rev, from) = &newest[&(e.kind.clone(), e.id.clone())];
        let update = *rev > e.rev && writable;
        if update {
            verbs.push("update");
        }
        if l.dest.root.is_some() {
            verbs.push("reveal");
        }
        if writable {
            verbs.extend(["-", "libdelete"]);
        }
        let mut details = vec![json!(["Saved in", l.dest.label, ""]), json!(["Revision", e.rev.to_string(), ""])];
        if !e.description.is_empty() {
            details.push(json!(["Description", e.description, ""]));
        }
        if !e.tags.is_empty() {
            details.push(json!(["Tags", e.tags.join(", "), ""]));
        }
        if update {
            details.push(json!(["Update", format!("Revision {rev} in {}", labels.get(from.as_str()).copied().unwrap_or(from)), "warn"]));
        }
        let mut row = json!({
            "key": item_key(&l.dest.key, &e.kind, &e.id),
            "name": e.name,
            "source": l.dest.key,
            "source_label": l.dest.label,
            "category": e.tags.first().cloned().unwrap_or_default(),
            "uses": 0,
            "defaults": details,
            "verbs": verbs,
            "lib": {"dest": l.dest.key, "kind": e.kind, "id": e.id, "rev": e.rev, "writable": writable,
                    "files": l.dest.root.as_ref().is_some_and(|r| store::files_dir(r, &e.kind, &e.id).is_dir()),
                    "update_from": if update { json!(from) } else { Value::Null }},
        });
        let thumb = l.dest.root.as_ref().map(|r| store::thumb_path(r, &e.kind, &e.id)).filter(|p| p.is_file());
        if let Some(t) = &thumb {
            row["thumb"] = json!(t.to_string_lossy());
        }
        match e.kind.as_str() {
            "materials" => {
                let recipe = material_recipe(l);
                let image = recipe["texture"]["path"].as_str().filter(|p| !p.is_empty()).map(str::to_owned);
                row["base"] = json!(if image.is_some() { "image" } else { "colour" });
                row["hex"] = json!(hex_of(&recipe["color"]));
                if thumb.is_none() {
                    row["thumb"] = json!(image);
                }
                row["recipe"] = recipe;
            }
            "profiles" | "sections" => {
                let mut sketch = e.data["sketch"].clone();
                sketch["name"] = json!(e.name);
                row["sketch"] = sketch;
            }
            "presets" => {
                let mut p = e.data.clone();
                p["uid"] = json!(e.id);
                p["name"] = json!(e.name);
                row["preset"] = p;
                row["category"] = json!(e.data["pack"].as_str().unwrap_or(""));
                if !matches!(e.data["primitive"].as_str(), Some("sweep" | "cut")) {
                    row["verbs"] = json!(verbs.iter().filter(|v| **v != "apply").collect::<Vec<_>>());
                }
            }
            "styles" => {
                row["hex"] = json!(hex_of(&e.data["face_color"]));
            }
            _ => {}
        }
        out.push(row);
    }
    Ok(out)
}

/// Browser rows as Python dicts.
pub(crate) fn py_items<'py>(py: Python<'py>, scene: Option<&Bound<'py, PyAny>>, kinds: &[&str]) -> PyResult<Vec<Bound<'py, PyAny>>> {
    items(py, scene, kinds)?.iter().map(|v| to_py(py, v)).collect()
}

/// Ids already saved (a legacy row with the same id is not listed twice).
pub(crate) fn saved_ids(py: Python<'_>, scene: Option<&Bound<'_, PyAny>>, kind: &str) -> PyResult<std::collections::HashSet<String>> {
    Ok(entries(py, scene, &[kind])?.into_iter().map(|l| l.entry.id).collect())
}

// ---- saving ----------------------------------------------------------------------------------

/// Save `entry` to the destination `key`. The id stays; saving over an
/// item already there raises its revision. Returns the destination's label.
pub(crate) fn save(py: Python<'_>, win: &Bound<'_, PyAny>, key: &str, mut entry: store::Entry, thumb: Option<Vec<u8>>, files: Vec<(String, PathBuf)>) -> PyResult<String> {
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let dest = all(py, Some(&scene))?.into_iter().find(|d| d.key == key).ok_or_else(|| err(format!("No Library destination “{key}”")))?;
    if !dest.writable {
        return Err(err(if dest.reason.is_empty() { format!("{} cannot be written", dest.label) } else { dest.reason.clone() }));
    }
    entry.saved = store::now();
    entry.check().map_err(err)?;
    if dest.key == "model" {
        let old = model_entries(py, &scene, &entry.kind)?.into_iter().find(|e| e.id == entry.id);
        entry.rev = store::next_revision(old.as_ref(), entry.rev);
        let value = serde_json::to_value(&entry).map_err(|e| err(e.to_string()))?;
        model_edit(py, &vp, &scene, |map| {
            if !map[&entry.kind].is_object() {
                map[&entry.kind] = json!({});
            }
            map[&entry.kind][&entry.id] = value;
        })?;
    } else {
        let root = dest.root.clone().expect("folder destination");
        store::probe_writable(&root).map_err(|e| err(format!("{} cannot be written: {e}", dest.label)))?;
        let old = store::read(&root, &entry.kind, &entry.id);
        entry.rev = store::next_revision(old.as_ref(), entry.rev);
        store::write(&root, &entry, thumb.as_deref(), &files).map_err(|e| err(format!("{} cannot be written: {e}", dest.label)))?;
        changed();
    }
    remember(py, &entry.kind, key)?;
    Ok(dest.label)
}

/// What a save drawer was opened with.
struct Pending {
    kind: String,
    id: String,
    rev: u64,
    data: Value,
    thumb: Option<Vec<u8>>,
    files: Vec<(String, PathBuf)>,
    /// Only this destination (editing an item where it is).
    fixed: Option<String>,
    exclude: Vec<String>,
    /// Kind options as checks: (id, label, on).
    checks: Vec<(String, String, bool)>,
}

fn tags_of(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in text.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        if !out.iter().any(|o| o.eq_ignore_ascii_case(t)) {
            out.push(t.to_owned());
        }
    }
    out
}

/// The Save to Library drawer: name, description, tags and where it goes.
/// Destinations that cannot be written are listed last, greyed, with why.
fn drawer(win: &Bound<'_, PyAny>, title: &str, name: &str, description: &str, tags: &[String], p: Pending) -> PyResult<()> {
    let py = win.py();
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let dests: Vec<store::Destination> = all(py, Some(&scene))?
        .into_iter()
        .filter(|d| p.fixed.as_ref().is_none_or(|f| f == &d.key) && !p.exclude.contains(&d.key))
        .collect();
    let mut ordered: Vec<&store::Destination> = dests.iter().filter(|d| d.writable).collect();
    let open = ordered.len();
    ordered.extend(dests.iter().filter(|d| !d.writable));
    let chosen = store::default_destination(&dests, &p.kind, last(py, &p.kind)?.as_deref()).map(|d| d.key.clone());
    let options: Vec<(String, String)> = ordered
        .iter()
        .map(|d| (d.key.clone(), if d.writable { tr(py, &d.label).unwrap_or(d.label.clone()) } else { format!("{} — {}", tr(py, &d.label).unwrap_or(d.label.clone()), tr(py, &d.reason).unwrap_or(d.reason.clone())) }))
        .collect();
    let forms = py.import("built_render.chrome_forms")?;
    let field = forms.getattr("Field")?;
    let fields = PyList::empty(py);
    fields.append(field.call(("name", "edit", tr(py, "Name")?, name), Some(&[("focus", true)].into_py_dict(py)?))?)?;
    fields.append(field.call1(("description", "edit", tr(py, "Description")?, description))?)?;
    fields.append(field.call(("tags", "edit", tr(py, "Tags")?, tags.join(", ")), Some(&[("tip", tr(py, "Separate tags with commas.")?)].into_py_dict(py)?))?)?;
    let kw = PyDict::new(py);
    kw.set_item("options", PyList::new(py, &options)?)?;
    kw.set_item("choice_limit", open)?;
    fields.append(field.call(("destination", "choice", tr(py, "Save to")?, chosen.clone()), Some(&kw))?)?;
    for (id, label, on) in &p.checks {
        fields.append(field.call1((id, "check", tr(py, label)?, *on))?)?;
    }
    let validate = func(py, move |py, a| {
        let form = a.get_item(0)?;
        let name: String = form.call_method1("value", ("name",))?.extract().unwrap_or_default();
        let key: Option<String> = form.call_method1("value", ("destination",))?.extract().unwrap_or_default();
        Ok(s(py, &if name.trim().is_empty() { tr(py, "Give it a name.")? } else if key.is_none() { tr(py, "No destination can be written: add one in Settings › Library.")? } else { String::new() }).unbind())
    })?;
    let w = win.clone().unbind();
    let pending = std::sync::Mutex::new(Some(p));
    let done = func(py, move |py, a| {
        if !a.get_item(0)?.eq("ok")? {
            return Ok(none(py));
        }
        let Some(p) = pending.lock().ok().and_then(|mut g| g.take()) else { return Ok(none(py)) };
        let vals = a.get_item(1)?;
        let name: String = vals.get_item("name")?.extract()?;
        let key: Option<String> = vals.get_item("destination")?.extract()?;
        let Some(key) = key else { return Ok(none(py)) };
        let mut data = p.data;
        for (id, _, _) in &p.checks {
            let on = vals.get_item(id)?.is_truthy()?;
            if p.kind == "presets" && id == "record_defaults" && !on {
                // Without record values a preset keeps only what classifies it.
                if let Some(record) = data["record"].as_object_mut() {
                    record.retain(|k, _| matches!(k.as_str(), "class" | "material" | "predefined_type" | "object_type"));
                }
            }
        }
        if p.kind == "presets" {
            data["name"] = json!(name.trim());
        }
        let mut entry = store::Entry::new(&p.kind, &p.id, name.trim(), data);
        entry.rev = p.rev;
        entry.description = vals.get_item("description")?.extract::<String>()?.trim().to_owned();
        entry.tags = tags_of(&vals.get_item("tags")?.extract::<String>()?);
        let win = w.bind(py);
        let out = if p.kind == "components" && key == "model" {
            keep_component_here(win, &entry)
        } else if p.kind == "presets" && key == "model" {
            preset_here(win, &entry)
        } else {
            save(py, win, &key, entry.clone(), p.thumb, p.files)
        };
        match out {
            Ok(label) => {
                win.call_method1("notify", ("success", trf(py, "{name} saved to {place}", &[("name", s(py, &entry.name)), ("place", s(py, &tr(py, &label)?))])?))?;
            }
            Err(e) => {
                win.call_method1("notify", ("error", format!("{}: {}", tr(py, "Could not save to the Library")?, e.value(py))))?;
            }
        }
        if let Ok(lib) = lib(py) {
            let _ = lib.call_method1("refresh", (win,));
        }
        Ok(none(py))
    })?;
    let kw = PyDict::new(py);
    kw.set_item("buttons", [("cancel", tr(py, "Cancel")?), ("ok", tr(py, "Save")?)])?;
    kw.set_item("on_done", done)?;
    kw.set_item("validate", validate)?;
    let form = forms.getattr("Form")?.call((title, fields), Some(&kw))?;
    form.setattr("placement", "drawer")?;
    forms.call_method1("show", (win, form))?;
    Ok(())
}

/// Open the Save to Library drawer for one item of `kind`. `data` is the
/// kind's payload as JSON; `thumb` a PNG file; `files` the files it
/// carries; `id` keeps an item's identity (a re-save is a new revision).
#[pyfunction]
#[pyo3(signature = (win, kind, name, data, *, id=None, rev=1, description="", tags=None, thumb=None, files=None, exclude=None, title=None))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn save_to_library(win: &Bound<'_, PyAny>, kind: &str, name: &str, data: &str, id: Option<String>, rev: u64, description: &str, tags: Option<Vec<String>>,
                              thumb: Option<PathBuf>, files: Option<Vec<(String, PathBuf)>>, exclude: Option<Vec<String>>, title: Option<String>) -> PyResult<()> {
    let py = win.py();
    store::kind(kind).ok_or_else(|| err(format!("Unknown Library kind “{kind}”")))?;
    let data: Value = serde_json::from_str(data).map_err(|e| err(e.to_string()))?;
    let id = id.filter(|i| store::valid_id(i)).unwrap_or_else(store::new_id);
    let p = Pending { kind: kind.into(), id, rev, data, thumb: thumb.and_then(|t| std::fs::read(t).ok()), files: files.unwrap_or_default(), fixed: None, exclude: exclude.unwrap_or_default(), checks: Vec::new() };
    let title = title.unwrap_or(tr(py, "Save to Library")?);
    drawer(win, &title, name, description, &tags.unwrap_or_default(), p)
}

/// Save as preset: the shared drawer, with the preset's own option.
pub(crate) fn save_preset(win: &Bound<'_, PyAny>, name: &str, preset: Value) -> PyResult<()> {
    let py = win.py();
    let id = preset["uid"].as_str().map(str::to_lowercase).filter(|i| store::valid_id(i)).unwrap_or_else(store::new_id);
    let p = Pending { kind: "presets".into(), id, rev: 1, data: preset, thumb: None, files: Vec::new(), fixed: None, exclude: Vec::new(),
                      checks: vec![("record_defaults".into(), "Include record values".into(), true)] };
    drawer(win, &tr(py, "Save as preset")?, name, "", &[], p)
}

/// Save without the drawer (scripts, tests, Copy to).
#[pyfunction]
#[pyo3(signature = (win, kind, destination, name, data, *, id=None, description="", tags=None, files=None))]
#[allow(clippy::too_many_arguments)]
fn save_item(win: &Bound<'_, PyAny>, kind: &str, destination: &str, name: &str, data: &str, id: Option<String>, description: &str, tags: Option<Vec<String>>,
             files: Option<Vec<(String, PathBuf)>>) -> PyResult<String> {
    let py = win.py();
    let data: Value = serde_json::from_str(data).map_err(|e| err(e.to_string()))?;
    let id = id.filter(|i| store::valid_id(i)).unwrap_or_else(store::new_id);
    let mut e = store::Entry::new(kind, &id, name, data);
    e.description = description.into();
    e.tags = tags.unwrap_or_default();
    save(py, win, destination, e.clone(), None, files.unwrap_or_default())?;
    Ok(id)
}

/// A component's "This model": its definition, kept when unused.
fn keep_component_here(win: &Bound<'_, PyAny>, e: &store::Entry) -> PyResult<String> {
    let py = win.py();
    let template = component_template(py, win, e)?;
    win.call_method1("keep_component", (template, true))?;
    changed();
    Ok("This model".into())
}

/// A preset's "This model": the model's preset table.
fn preset_here(win: &Bound<'_, PyAny>, e: &store::Entry) -> PyResult<String> {
    let py = win.py();
    let mut p = e.data.clone();
    p["uid"] = json!(e.id);
    p["name"] = json!(e.name);
    p["rev"] = json!(e.rev);
    let args = PyDict::new(py);
    args.set_item("rows", to_py(py, &json!([p]))?)?;
    let ctx = ctx_of(py, win, &win.getattr("view")?)?;
    let reply = core(&ctx, "preset_update", &args)?;
    if !reply.get_item("ok")?.is_truthy()? {
        return Err(err(reply.call_method1("get", ("error", "Preset update failed"))?.str()?.to_string()));
    }
    notify(&ctx.getattr("vp")?)?;
    changed();
    Ok("This model".into())
}

// ---- kind payloads ---------------------------------------------------------------------------

/// A component's file (`component.built` in its files) loaded as a
/// placeable template.
fn component_template<'py>(py: Python<'py>, win: &Bound<'py, PyAny>, e: &store::Entry) -> PyResult<Bound<'py, PyAny>> {
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let dest = all(py, Some(&scene))?;
    let file = e.data["file"].as_str().ok_or_else(|| err("This component has no file"))?;
    let path = PathBuf::from(file);
    let path = if path.is_absolute() {
        path
    } else {
        dest.iter()
            .filter_map(|d| d.root.as_ref())
            .map(|r| store::files_dir(r, &e.kind, &e.id).join(file))
            .find(|p| p.is_file())
            .ok_or_else(|| err("The component's file is missing"))?
    };
    let temp = py.import("built_core.scene")?.call_method0("Scene")?;
    py.import("built_core.formats.igz")?.call_method1("load_into", (&temp, py.import("pathlib")?.call_method1("Path", (&path,))?))?;
    let groups = temp.getattr("groups")?;
    if !groups.is_truthy()? {
        return Err(err("The component's file is empty"));
    }
    let kc = py.import("built_core.kept_components")?;
    let template = kc.call_method1("make_template", (groups.get_item(0)?,))?;
    template.setattr("name", &e.name)?;
    Ok(template)
}

/// Write a component definition to a `.built` of its own, for its files.
pub(crate) fn component_file(py: Python<'_>, definition: &Bound<'_, PyAny>, folder: &Path) -> PyResult<PathBuf> {
    std::fs::create_dir_all(folder)?;
    let kc = py.import("built_core.kept_components")?;
    let template = kc.call_method1("make_template", (definition,))?;
    let copy = kc.call_method1("place_copy", (template,))?;
    let scene = py.import("built_core.scene")?.call_method0("Scene")?;
    scene.getattr("groups")?.call_method1("append", (copy,))?;
    let path = folder.join("component.built");
    let kw = [("backup", false)].into_py_dict(py)?;
    py.import("built_core.formats.igz")?.getattr("save_scene")?.call((scene, py.import("pathlib")?.call_method1("Path", (&path,))?), Some(&kw))?;
    Ok(path)
}

/// Use a saved item: paint, place, sweep, cut, or apply a style.
fn apply(ctx: &Bound<'_, PyAny>, item: &Bound<'_, PyAny>, kind: &str, entry: &store::Entry) -> PyResult<()> {
    let py = ctx.py();
    let win = ctx.getattr("win")?;
    match kind {
        "materials" => {
            let recipe = item.get_item("recipe")?;
            lib(py)?.getattr("_pick_paint")?.call((&win,), Some(&[("recipe", recipe)].into_py_dict(py)?))?;
        }
        "profiles" | "sections" => {
            let tool = if kind == "sections" { "cut" } else { "sweep" };
            win.call_method1("_activate_tool", (tool,))?;
            let machine = win.getattr("_tools")?.get_item(tool)?.getattr("_machine")?;
            machine.call_method1("set", ("sketch", item.get_item("sketch")?))?;
        }
        "presets" => {
            let p = item.get_item("preset")?;
            let primitive: String = p.get_item("primitive")?.extract()?;
            win.call_method1("_activate_tool", (&primitive,))?;
            win.getattr("_tools")?.get_item(&primitive)?.getattr("_machine")?.call_method1("set", ("preset", p))?;
        }
        "components" => {
            let template = component_template(py, &win, entry)?;
            let group = py.import("built_core.kept_components")?.call_method1("place_copy", (template,))?;
            win.call_method1("_start_place", (group,))?;
        }
        "styles" => {
            let style = py.import("built_core.style")?.getattr("Style")?.call_method1("from_dict", (to_py(py, &entry.data)?,))?;
            style.setattr("name", &entry.name)?;
            win.call_method1("set_active_style", (style,))?;
        }
        _ => flash(ctx, "This kind is used from its own tool")?,
    }
    Ok(())
}

// ---- verbs between destinations --------------------------------------------------------------

fn transfer(py: Python<'_>, win: &Bound<'_, PyAny>, from: &Listed, to: &store::Destination, moving: bool) -> PyResult<String> {
    let e = &from.entry;
    if !to.writable {
        return Err(err(to.reason.clone()));
    }
    let outcome = match (&from.dest.root, &to.root) {
        (Some(a), Some(b)) => {
            let r = if moving { store::move_item(a, b, &e.kind, &e.id) } else { store::copy(a, b, &e.kind, &e.id, false) };
            changed();
            r.map_err(|r| err(r.to_string()))?
        }
        (_, None) if to.key == "model" => {
            // Into the model: its files stay where they are and are referenced.
            let mut entry = e.clone();
            if e.kind == "materials" {
                entry.data = material_recipe(from);
            }
            if e.kind == "components" {
                keep_component_here(win, &entry)?;
            } else if e.kind == "presets" {
                preset_here(win, &entry)?;
            } else {
                let vp = win.getattr("viewport")?;
                let scene = vp.getattr("scene")?;
                let there = model_entries(py, &scene, &e.kind)?.into_iter().find(|x| x.id == e.id);
                store::compare(e, there.as_ref()).map_err(|r| err(r.to_string()))?;
                let value = serde_json::to_value(&entry).map_err(|x| err(x.to_string()))?;
                model_edit(py, &vp, &scene, |map| {
                    if !map[&entry.kind].is_object() {
                        map[&entry.kind] = json!({});
                    }
                    map[&entry.kind][&entry.id] = value;
                })?;
            }
            if moving {
                remove(py, win, from)?;
            }
            store::Copied::Added
        }
        (None, Some(b)) => {
            let there = store::read(b, &e.kind, &e.id);
            let outcome = store::compare(e, there.as_ref());
            let outcome = match outcome {
                Err(store::Refused::Newer(_)) if moving => store::Copied::Updated,
                o => o.map_err(|r| err(r.to_string()))?,
            };
            store::write(b, e, None, &[]).map_err(|x| err(x.to_string()))?;
            changed();
            if moving {
                remove(py, win, from)?;
            }
            outcome
        }
        _ => return Err(err("Nothing to copy")),
    };
    Ok(match outcome {
        store::Copied::Added => format!("{} {} {}", e.name, if moving { "moved to" } else { "copied to" }, to.label),
        store::Copied::Updated => format!("{} updated in {} (revision {})", e.name, to.label, e.rev),
        store::Copied::Same => format!("{} is already in {}", e.name, to.label),
    })
}

fn remove(py: Python<'_>, win: &Bound<'_, PyAny>, l: &Listed) -> PyResult<()> {
    if !l.dest.writable {
        return Err(err(l.dest.reason.clone()));
    }
    match &l.dest.root {
        Some(root) => {
            store::remove(root, &l.entry.kind, &l.entry.id)?;
            changed();
        }
        None => {
            let vp = win.getattr("viewport")?;
            let scene = vp.getattr("scene")?;
            let (kind, id) = (l.entry.kind.clone(), l.entry.id.clone());
            model_edit(py, &vp, &scene, |map| {
                if let Some(m) = map[&kind].as_object_mut() {
                    m.remove(&id);
                }
            })?;
        }
    }
    Ok(())
}

fn listed_of(py: Python<'_>, ctx: &Bound<'_, PyAny>, item: &Bound<'_, PyAny>) -> PyResult<Option<Listed>> {
    let key: String = item.get_item("key")?.extract()?;
    let Some((dest, kind, id)) = parse_key(&key) else { return Ok(None) };
    let scene = ctx.getattr("scene")?;
    find(py, Some(&scene), &dest, &kind, &id)
}

/// Run a verb on a saved item; false when the verb is not one of these.
pub(crate) fn run(ctx: &Bound<'_, PyAny>, verb: &str, item: &Bound<'_, PyAny>) -> PyResult<bool> {
    let py = ctx.py();
    if !item.contains("lib")? || !matches!(verb, "apply" | "copyto" | "moveto" | "update" | "reveal" | "libdelete" | "libedit") {
        return Ok(false);
    }
    let Some(l) = listed_of(py, ctx, item)? else {
        flash(ctx, "The item is no longer there")?;
        return Ok(true);
    };
    let win = ctx.getattr("win")?;
    let scene = ctx.getattr("scene")?;
    match verb {
        "apply" => apply(ctx, item, &l.entry.kind.clone(), &l.entry)?,
        "copyto" | "moveto" => {
            let moving = verb == "moveto";
            let has_files = item.get_item("lib")?.get_item("files")?.is_truthy()?;
            let choices: Vec<store::Destination> = all(py, Some(&scene))?
                .into_iter()
                .filter(|d| d.writable && d.key != l.dest.key && !(moving && has_files && d.key == "model"))
                .filter(|d| !(d.key == "model" && l.entry.kind == "components" && l.dest.root.is_none()))
                .collect();
            if choices.is_empty() {
                flash(ctx, "No other destination can be written: add one in Settings › Library")?;
                return Ok(true);
            }
            let labels: Vec<String> = choices.iter().map(|d| tr(py, &d.label).unwrap_or(d.label.clone())).collect();
            let labels2 = labels.clone();
            let w = win.clone().unbind();
            let c = ctx.clone().unbind();
            let done = func(py, move |py, a| {
                let answer = a.get_item(0)?;
                if answer.is_none() {
                    return Ok(none(py));
                }
                let chosen: String = answer.extract()?;
                let Some(i) = labels.iter().position(|l| *l == chosen) else { return Ok(none(py)) };
                match transfer(py, w.bind(py), &l, &choices[i], moving) {
                    Ok(text) => flash(c.bind(py), &text)?,
                    Err(e) => flash(c.bind(py), &e.value(py).str()?.to_string())?,
                }
                lib(py)?.call_method1("refresh", (w.bind(py),))?;
                Ok(none(py))
            })?;
            let title = tr(py, if moving { "Move to" } else { "Copy to" })?;
            py.import("built_render.chrome_forms")?
                .getattr("ask_choice")?
                .call((&win, title, tr(py, "Destination")?, labels2), Some(&[("done", done)].into_py_dict(py)?))?;
        }
        "update" => {
            let from: Option<String> = item.get_item("lib")?.get_item("update_from")?.extract()?;
            let Some(from) = from else { return Ok(true) };
            let Some(newer) = find(py, Some(&scene), &from, &l.entry.kind, &l.entry.id)? else { return Ok(true) };
            match transfer(py, &win, &newer, &l.dest, false) {
                Ok(text) => flash(ctx, &text)?,
                Err(e) => flash(ctx, &e.value(py).str()?.to_string())?,
            }
        }
        "reveal" => {
            if let Some(root) = &l.dest.root {
                let path = store::entry_path(root, &l.entry.kind, &l.entry.id);
                py.import("built_core.desktop_open")?.call_method1("reveal", (path.to_string_lossy().as_ref(),))?;
            }
        }
        "libdelete" => {
            ask(&ctx.getattr("view")?, ("libdelete", "library", item.get_item("key")?.extract()?), &format!("Delete “{}” from {}?", l.entry.name, l.dest.label),
                if l.dest.root.is_some() { "Its file goes; copies elsewhere stay." } else { "It goes from this model (Undo brings it back)." }, "Delete")?;
        }
        "libedit" => {
            let e = l.entry.clone();
            let p = Pending { kind: e.kind.clone(), id: e.id.clone(), rev: e.rev, data: e.data.clone(), thumb: None, files: Vec::new(), fixed: Some(l.dest.key.clone()), exclude: Vec::new(), checks: Vec::new() };
            drawer(&win, &tr(py, "Edit Library item")?, &e.name, &e.description, &e.tags, p)?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// A confirmed Delete of a saved item.
pub(crate) fn confirmed(win: &Bound<'_, PyAny>, view: &Bound<'_, PyAny>, key: &str) -> PyResult<()> {
    let py = win.py();
    let Some((dest, kind, id)) = parse_key(key) else { return Ok(()) };
    let ctx = ctx_of(py, win, view)?;
    let scene = ctx.getattr("scene")?;
    if let Some(l) = find(py, Some(&scene), &dest, &kind, &id)? {
        remove(py, win, &l)?;
        flash(&ctx, &format!("{} deleted from {}", l.entry.name, l.dest.label))?;
    }
    Ok(())
}

// ---- Python ----------------------------------------------------------------------------------

/// Every destination: (key, label, folder or None, writable, reason).
#[pyfunction]
#[pyo3(name = "destinations", signature = (scene=None))]
fn list_destinations(py: Python<'_>, scene: Option<Bound<'_, PyAny>>) -> PyResult<Vec<(String, String, Option<String>, bool, String)>> {
    Ok(all(py, scene.as_ref())?.into_iter().map(|d| (d.key, d.label, d.root.map(|r| r.to_string_lossy().into_owned()), d.writable, d.reason)).collect())
}

/// The saved items of `kind` as browser rows.
#[pyfunction]
#[pyo3(signature = (kind, scene=None))]
fn library_items<'py>(py: Python<'py>, kind: &str, scene: Option<Bound<'py, PyAny>>) -> PyResult<Vec<Bound<'py, PyAny>>> {
    py_items(py, scene.as_ref(), &[kind])
}

/// My library's folder and the shared libraries, as Settings shows them.
#[pyfunction]
fn settings_state(py: Python<'_>) -> PyResult<(String, String, Vec<(String, String, bool)>)> {
    let cfg = config(py)?;
    Ok((
        cfg.mine.map(|m| m.to_string_lossy().into_owned()).unwrap_or_default(),
        default_mine().map(|m| m.to_string_lossy().into_owned()).unwrap_or_default(),
        cfg.shared.into_iter().map(|s| (s.name, s.path.to_string_lossy().into_owned(), s.read_only)).collect(),
    ))
}

/// Change My library's folder ("" for the default).
#[pyfunction]
fn set_mine(py: Python<'_>, path: &str) -> PyResult<()> {
    let path = path.trim();
    if !path.is_empty() && !Path::new(path).is_absolute() {
        return Err(err("My library's folder must be a full path"));
    }
    set_pref(py, ROOT_KEY, path)?;
    changed();
    Ok(())
}

/// Add or change a shared library (by name).
#[pyfunction]
#[pyo3(signature = (name, path, read_only=false, old_name=None))]
fn set_shared(py: Python<'_>, name: &str, path: &str, read_only: bool, old_name: Option<String>) -> PyResult<()> {
    let (name, path) = (name.trim(), path.trim());
    if name.is_empty() || matches!(name.to_lowercase().as_str(), "this model" | "this project" | "my library" | "bundled") {
        return Err(err("Give the shared library a name of its own"));
    }
    if !Path::new(path).is_absolute() {
        return Err(err("A shared library's folder must be a full path"));
    }
    let mut cfg = config(py)?;
    let old = old_name.unwrap_or_else(|| name.to_owned());
    if cfg.shared.iter().any(|s| s.name == name && s.name != old) {
        return Err(err(format!("A shared library is already called {name}")));
    }
    let entry = store::SharedLibrary { name: name.into(), path: path.into(), read_only };
    match cfg.shared.iter_mut().find(|s| s.name == old) {
        Some(s) => *s = entry,
        None => cfg.shared.push(entry),
    }
    set_pref(py, SHARED_KEY, &cfg.shared_json())?;
    changed();
    Ok(())
}

#[pyfunction]
fn remove_shared(py: Python<'_>, name: &str) -> PyResult<bool> {
    let mut cfg = config(py)?;
    let n = cfg.shared.len();
    cfg.shared.retain(|s| s.name != name);
    set_pref(py, SHARED_KEY, &cfg.shared_json())?;
    changed();
    Ok(cfg.shared.len() != n)
}

pub(super) fn register(sub: &Bound<'_, PyModule>) -> PyResult<()> {
    for f in [
        wrap_pyfunction!(save_to_library, sub)?,
        wrap_pyfunction!(save_item, sub)?,
        wrap_pyfunction!(list_destinations, sub)?,
        wrap_pyfunction!(library_items, sub)?,
        wrap_pyfunction!(settings_state, sub)?,
        wrap_pyfunction!(set_mine, sub)?,
        wrap_pyfunction!(set_shared, sub)?,
        wrap_pyfunction!(remove_shared, sub)?,
        wrap_pyfunction!(migrate_now, sub)?,
    ] {
        sub.add_function(f)?;
    }
    sub.add("KINDS", store::KINDS.iter().map(|k| (k.id, k.label)).collect::<Vec<_>>())?;
    Ok(())
}
