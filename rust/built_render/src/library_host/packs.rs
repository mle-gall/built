// SPDX-License-Identifier: GPL-3.0-or-later
//! Junction details in Library › Presets: the rule in words, what a detail
//! adds, its pack, revision, author and licence, what it still needs;
//! Apply… on the selection in one step; Share pack…; Import pack… into a
//! Library destination with every file checked first; updates offered,
//! never applied by themselves. The pack format is `built_core`'s
//! (`live/pack.rs`); this side only lists, asks and stores.
use super::*;
use crate::library_store as store;
use pyo3::exceptions::PyValueError;
use serde_json::{json, Value};
use std::collections::HashMap;

fn err(text: impl Into<String>) -> PyErr {
    PyValueError::new_err(text.into())
}
fn proto(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("built_core.command_protocol")
}
fn val(py: Python<'_>, v: &Bound<'_, PyAny>) -> PyResult<Value> {
    serde_json::from_str(&dumps(py, v)?).map_err(|e| err(e.to_string()))
}
fn obj_of<'py>(py: Python<'py>, v: &Value) -> PyResult<Bound<'py, PyAny>> {
    json(py)?.call_method1("loads", (v.to_string(),))
}
fn text(v: &Value) -> String {
    v.as_str().map(str::to_owned).unwrap_or_else(|| if v.is_null() { String::new() } else { v.to_string() })
}

/// How many junctions of the model follow each detail.
pub(super) fn uses(py: Python<'_>, scene: Option<&Bound<'_, PyAny>>) -> PyResult<HashMap<String, usize>> {
    let mut out = HashMap::new();
    let Some(scene) = scene else { return Ok(out) };
    for row in val(py, &proto(py)?.call_method1("live_rows", (scene,))?)?.as_array().into_iter().flatten() {
        if row["kind"] == "junction" {
            if let Some(uid) = row["detail"]["uid"].as_str() {
                *out.entry(uid.to_owned()).or_default() += 1;
            }
        }
    }
    Ok(out)
}

/// The model's own presets by uid (to offer a newer pack revision).
pub(super) fn model_presets(py: Python<'_>, scene: Option<&Bound<'_, PyAny>>) -> PyResult<HashMap<String, Value>> {
    let catalog = proto(py)?.call_method1("preset_catalog", (scene.cloned(),))?;
    let rows: Vec<(Value, String)> = serde_json::from_value(val(py, &catalog)?).map_err(|e| err(e.to_string()))?;
    Ok(rows.into_iter().filter(|(_, s)| s == "model").map(|(p, _)| (text(&p["uid"]), p)).collect())
}

fn pack_rev(p: &Value) -> u64 {
    p["source"]["rev"].as_u64().or_else(|| p["rev"].as_u64()).unwrap_or(1)
}

/// A newer revision of a model detail, from a pack in the Library.
pub(super) fn update_offer(p: &Value, model: &HashMap<String, Value>) -> Option<(u64, u64)> {
    let here = model.get(&text(&p["uid"]))?;
    let source = &here["source"];
    if source.is_null() || p["source"]["pack"].is_null() || source["pack"] != p["source"]["pack"] {
        return None;
    }
    let (old, new) = (pack_rev(here), pack_rev(p));
    (new > old && source["hash"] != p["source"]["hash"]).then_some((old, new))
}

/// The details pane of a junction detail and its thumbnail key.
pub(super) fn describe(py: Python<'_>, p: &Value, used: usize, offer: Option<(u64, u64)>) -> PyResult<(Vec<Value>, String)> {
    let words = val(py, &proto(py)?.call_method1("junction_words", (obj_of(py, p)?,))?)?;
    let mut rows = Vec::new();
    if !text(&words["rule"]).is_empty() {
        rows.push(json!(["Meeting", words["rule"], ""]));
    }
    if !text(&words["adds"]).is_empty() {
        rows.push(json!(["Adds", words["adds"], ""]));
    }
    if let Some(needs) = p["needs"].as_array().filter(|n| !n.is_empty()) {
        let list: Vec<String> = needs.iter().map(text).collect();
        rows.push(json!(["Needs", list.join(", "), "warn"]));
    }
    if let Some((old, new)) = offer {
        rows.push(json!(["Update available", format!("rev {old} → {new} · {used} use{}", if used == 1 { "" } else { "s" }), "warn"]));
    }
    for d in p["values"]["params"].as_array().into_iter().flatten() {
        if d["hidden"] == true {
            continue;
        }
        let name = d["name"].as_str().or(d["key"].as_str()).unwrap_or("");
        let unit = match d["unit"].as_str() {
            Some("mm") => " mm",
            Some("deg") => "°",
            _ => "",
        };
        let expr = text(&d["expr"]);
        let shown = if expr.parse::<f64>().is_ok() { format!("{expr}{unit}") } else { format!("= {expr}") };
        rows.push(json!([name, shown, if d["lock"] == "fixed" { "fixed" } else { "" }]));
    }
    let class = p["record"]["class"].as_str().unwrap_or("IfcElementAssembly");
    rows.push(json!(["Files as", match p["record"]["predefined_type"].as_str() {
        Some(t) => format!("{class} · {t}"),
        None => class.to_owned(),
    }, ""]));
    rows.push(json!(["Used", if used == 0 { "not in this model".to_owned() } else { format!("{used} in this model") }, ""]));
    let source = &p["source"];
    let pack = source["name"].as_str().or(p["pack"].as_str()).unwrap_or("");
    if !pack.is_empty() {
        let version = source["version"].as_str().map(|v| format!(" {v}")).unwrap_or_default();
        rows.push(json!(["Pack", format!("{pack}{version}"), ""]));
    }
    rows.push(json!(["Detail revision", pack_rev(p).to_string(), ""]));
    for (key, label) in [("author", "Author"), ("licence", "Licence")] {
        let v = if p[key].is_null() { &p["source"][key] } else { &p[key] };
        let shown = v["name"].as_str().map(str::to_owned).unwrap_or_else(|| text(v));
        if !shown.is_empty() {
            rows.push(json!([label, shown, ""]));
        }
    }
    if let Some(from) = p["provenance"]["derived_from"]["uid"].as_str() {
        rows.push(json!(["Derived from", from, ""]));
    }
    Ok((rows, text(&words["thumb"])))
}

/// A detail's thumbnail key.
pub(super) fn junction_thumb(py: Python<'_>, p: &Value) -> PyResult<String> {
    let words = val(py, &proto(py)?.call_method1("junction_words", (obj_of(py, p)?,))?)?;
    Ok(text(&words["thumb"]))
}

/// Every preset row the Library lists, with what each one is (for the
/// definitions a detail needs and for Share pack).
pub(super) fn all_rows(ctx: &Bound<'_, PyAny>) -> PyResult<Vec<Value>> {
    let py = ctx.py();
    let mut out: Vec<Value> = super::presets::listed(ctx)?.into_iter().map(|(p, _)| p).collect();
    let scene = ctx.getattr_opt("scene")?.filter(|s| s.hasattr("_root").unwrap_or(false));
    for row in super::destinations::items(py, scene.as_ref(), &["presets"])? {
        let p = row["preset"].clone();
        if !out.iter().any(|o| o["uid"] == p["uid"]) {
            out.push(p);
        }
    }
    Ok(out)
}

/// Apply… a detail to the selected members (main first), embedding the
/// detail and what it needs in the same step.
pub(super) fn apply(ctx: &Bound<'_, PyAny>, p: &Value) -> PyResult<()> {
    let py = ctx.py();
    if let Some(needs) = p["needs"].as_array().filter(|n| !n.is_empty()) {
        let list: Vec<String> = needs.iter().map(text).collect();
        return flash(ctx, &format!("{} needs {}", text(&p["name"]), list.join(", ")));
    }
    let scene = ctx.getattr("scene")?;
    let mut picks = Vec::new();
    for g in scene.getattr("selection")?.try_iter()? {
        if let Some(uid) = g?.getattr_opt("uid")? {
            picks.push(uid.extract::<String>()?);
        }
    }
    let api = proto(py)?;
    let bindings = match api.call_method1("junction_bind", (&scene, obj_of(py, p)?, picks)) {
        Ok(b) => b,
        Err(e) => return flash(ctx, &e.value(py).str()?.to_string()),
    };
    let rows = all_rows(ctx)?;
    let definitions = match api.call_method1("junction_definitions", (obj_of(py, p)?, obj_of(py, &json!(rows))?)) {
        Ok(d) => d,
        Err(e) => return flash(ctx, &e.value(py).str()?.to_string()),
    };
    let kw = PyDict::new(py);
    kw.set_item("detail", text(&p["uid"]))?;
    kw.set_item("bindings", bindings)?;
    kw.set_item("definitions", definitions)?;
    let reply = core(ctx, "junction_apply", &kw)?;
    if !reply.get_item("ok")?.is_truthy()? {
        return flash(ctx, &reply.call_method1("get", ("error", "The detail does not apply here"))?.str()?.to_string());
    }
    notify(&ctx.getattr("vp")?)?;
    flash(ctx, &format!("{} applied", text(&p["name"])))
}

/// Update the model's copy of a detail to the Library's newer revision: its
/// uses follow in one step.
pub(super) fn update_model(ctx: &Bound<'_, PyAny>, p: &Value) -> PyResult<()> {
    let py = ctx.py();
    let rows = all_rows(ctx)?;
    let defs = match proto(py)?.call_method1("junction_definitions", (obj_of(py, p)?, obj_of(py, &json!(rows))?)) {
        Ok(d) => d,
        Err(e) => return flash(ctx, &e.value(py).str()?.to_string()),
    };
    let args = PyDict::new(py);
    args.set_item("rows", defs)?;
    let reply = core(ctx, "preset_update", &args)?;
    if !reply.get_item("ok")?.is_truthy()? {
        return flash(ctx, &reply.call_method1("get", ("error", "Update failed"))?.str()?.to_string());
    }
    notify(&ctx.getattr("vp")?)?;
    flash(ctx, &format!("{} updated to revision {}", text(&p["name"]), pack_rev(p)))
}

fn slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let s = s.split('-').filter(|w| !w.is_empty()).collect::<Vec<_>>().join("-");
    if s.is_empty() { "pack".into() } else { s }
}

/// Write a pack: `details` and what they need, to `dest`.
pub(crate) fn export(py: Python<'_>, scene: Option<&Bound<'_, PyAny>>, details: &[Value], rows: &[Value], meta: &Value, dest: &str) -> PyResult<Value> {
    if dest.trim().is_empty() {
        return Err(err("Choose where to write the pack"));
    }
    let report = proto(py)?.call_method1(
        "pack_export",
        (obj_of(py, &json!(details))?, obj_of(py, meta)?, dest.trim(), obj_of(py, &json!(rows))?, scene.cloned()),
    )?;
    val(py, &report)
}

/// Share pack…: the pack's name, version, author and licence (no default
/// licence), where to write it, and whether only this detail goes.
pub(super) fn share(ctx: &Bound<'_, PyAny>, p: &Value) -> PyResult<()> {
    let py = ctx.py();
    let forms = py.import("built_render.chrome_forms")?;
    let field = forms.getattr("Field")?;
    let pack = p["source"]["name"].as_str().or(p["pack"].as_str()).filter(|s| !s.is_empty()).unwrap_or("My details").to_owned();
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let dest = home.join(format!("{}.builtpack", slug(&pack)));
    let fields = PyList::empty(py);
    fields.append(field.call1(("name", "edit", "Pack name", &pack))?)?;
    fields.append(field.call1(("version", "edit", "Version", "1.0.0"))?)?;
    fields.append(field.call1(("author", "edit", "Author", ""))?)?;
    fields.append(field.call1(("licence", "edit", "Licence (SPDX id)", ""))?)?;
    fields.append(field.call1(("dest", "edit", "Write to", dest.to_string_lossy().to_string()))?)?;
    fields.append(field.call1(("only", "check", format!("Only {}", text(&p["name"])), false))?)?;
    let c = ctx.clone().unbind();
    let detail = p.clone();
    let done = func(py, move |py, args| {
        if !args.get_item(0)?.eq("ok")? {
            return Ok(none(py));
        }
        let vals = args.get_item(1)?;
        let get = |k: &str| -> PyResult<String> { Ok(vals.get_item(k)?.extract::<String>().unwrap_or_default()) };
        let ctx = c.bind(py);
        let rows = all_rows(ctx)?;
        let name = get("name")?;
        let details: Vec<Value> = if vals.get_item("only")?.is_truthy()? {
            vec![detail.clone()]
        } else {
            let pack_of = |r: &Value| r["source"]["name"].as_str().or(r["pack"].as_str()).unwrap_or("").to_owned();
            let wanted = pack_of(&detail);
            rows.iter().filter(|r| r["primitive"] == detail["primitive"] && pack_of(r) == wanted).cloned().collect()
        };
        let mut meta = json!({"name": name, "version": get("version")?, "author": get("author")?, "licence": get("licence")?});
        if let Some(id) = detail["source"]["pack"].as_str().filter(|_| detail["source"]["name"] == name.as_str()) {
            meta["id"] = json!(id);
        }
        let scene = ctx.getattr_opt("scene")?.filter(|s| s.hasattr("_root").unwrap_or(false));
        match export(py, scene.as_ref(), &details, &rows, &meta, &get("dest")?) {
            Ok(report) => flash(ctx, &format!("Pack written: {} detail{} to {}", report["rows"], if report["rows"] == 1 { "" } else { "s" }, text(&report["path"])))?,
            Err(e) => flash(ctx, &e.value(py).str()?.to_string())?,
        }
        Ok(none(py))
    })?;
    let kw = PyDict::new(py);
    kw.set_item("on_done", done)?;
    kw.set_item("buttons", [("cancel", "Cancel"), ("ok", "Write pack")])?;
    let form = forms.getattr("Form")?.call(("Share pack", fields), Some(&kw))?;
    forms.call_method1("show", (ctx.getattr("win")?, form))?;
    Ok(())
}

/// What the Library holds already, for dependency resolution: preset uid →
/// hash, and `component:<id>` for saved components.
fn known(py: Python<'_>, ctx: &Bound<'_, PyAny>) -> PyResult<Value> {
    let mut out = serde_json::Map::new();
    for r in all_rows(ctx)? {
        out.insert(text(&r["uid"]).to_lowercase(), json!(text(&r["hash"])));
    }
    let scene = ctx.getattr_opt("scene")?.filter(|s| s.hasattr("_root").unwrap_or(false));
    for l in super::destinations::entries(py, scene.as_ref(), &["components"])? {
        out.insert(format!("component:{}", l.entry.id), json!(""));
    }
    Ok(Value::Object(out))
}

/// Read and check a pack, then store it at a Library destination: every
/// row or none. Rows that need something missing are stored listed with
/// what they need (they cannot be applied); "This model" takes only the
/// rows that are complete. Returns {"ok", "needs", "skipped", "destination"}.
pub(crate) fn import(py: Python<'_>, ctx: &Bound<'_, PyAny>, path: &str, destination: &str) -> PyResult<Value> {
    let known = known(py, ctx)?;
    let plan = proto(py)?.call_method1("pack_read", (path, obj_of(py, &known)?))?;
    let rows: Vec<Value> = serde_json::from_value(val(py, &plan.get_item("rows")?)?).map_err(|e| err(e.to_string()))?;
    let pack = val(py, &plan.get_item("pack")?)?;
    let components = plan.get_item("components")?;
    let scene = ctx.getattr_opt("scene")?.filter(|s| s.hasattr("_root").unwrap_or(false));
    let dest = super::destinations::all(py, scene.as_ref())?
        .into_iter()
        .find(|d| d.key == destination)
        .ok_or_else(|| err(format!("No Library destination “{destination}”")))?;
    if !dest.writable {
        return Err(err(if dest.reason.is_empty() { format!("{} cannot be written", dest.label) } else { dest.reason.clone() }));
    }
    let (ready, needing): (Vec<Value>, Vec<Value>) = rows.into_iter().partition(|r| r.get("needs").is_none());
    let tag = pack["name"].as_str().unwrap_or("").to_owned();
    if dest.key == "model" {
        if ready.is_empty() {
            return Err(err("Nothing in this pack can be applied yet: every detail needs something missing"));
        }
        let args = PyDict::new(py);
        args.set_item("rows", obj_of(py, &json!(ready))?)?;
        let reply = core(ctx, "preset_update", &args)?;
        if !reply.get_item("ok")?.is_truthy()? {
            return Err(err(reply.call_method1("get", ("error", "Import failed"))?.str()?.to_string()));
        }
        notify(&ctx.getattr("vp")?)?;
        return Ok(json!({"ok": ready.len(), "needs": 0, "skipped": needing.len(), "destination": dest.label}));
    }
    let root = dest.root.clone().ok_or_else(|| err("Not a folder destination"))?;
    store::probe_writable(&root).map_err(|e| err(format!("{} cannot be written: {e}", dest.label)))?;
    // Every write is undone if one fails: nothing is half-imported.
    let mut written: Vec<(String, String, Option<store::Entry>)> = Vec::new();
    let result = (|| -> PyResult<()> {
        let tmp = py.import("tempfile")?.call_method0("mkdtemp")?.extract::<PathBuf>()?;
        for item in components.call_method0("items")?.try_iter()? {
            let item = item?;
            let uid: String = item.get_item(0)?.extract()?;
            let bytes: Vec<u8> = item.get_item(1)?.extract()?;
            let id = uid.to_lowercase();
            if !store::valid_id(&id) {
                return Err(err(format!("Invalid component id {uid}")));
            }
            let file = tmp.join(format!("{id}.built"));
            std::fs::write(&file, &bytes)?;
            let mut e = store::Entry::new("components", &id, &format!("Component {uid}"), json!({"file": "component.built", "uid": uid}));
            e.tags = vec![tag.clone()];
            e.saved = store::now();
            let old = store::read(&root, "components", &id);
            written.push(("components".into(), id.clone(), old.clone()));
            e.rev = store::next_revision(old.as_ref(), 1);
            store::write(&root, &e, None, &[("component.built".into(), file)]).map_err(|e| err(e.to_string()))?;
        }
        let _ = std::fs::remove_dir_all(&tmp);
        for row in ready.iter().chain(&needing) {
            let id = text(&row["uid"]).to_lowercase();
            if !store::valid_id(&id) {
                return Err(err("A detail has an invalid id"));
            }
            let mut e = store::Entry::new("presets", &id, &text(&row["name"]), row.clone());
            e.tags = vec![if tag.is_empty() { text(&row["pack"]) } else { tag.clone() }];
            e.description = text(&pack["description"]);
            e.saved = store::now();
            let old = store::read(&root, "presets", &id);
            written.push(("presets".into(), id.clone(), old.clone()));
            e.rev = store::next_revision(old.as_ref(), row["rev"].as_u64().unwrap_or(1));
            e.check().map_err(err)?;
            store::write(&root, &e, None, &[]).map_err(|e| err(e.to_string()))?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        for (kind, id, old) in written.iter().rev() {
            match old {
                Some(entry) => {
                    let _ = store::write(&root, entry, None, &[]);
                }
                None => {
                    let _ = store::remove(&root, kind, id);
                }
            }
        }
        return Err(e);
    }
    super::destinations::changed();
    Ok(json!({"ok": ready.len(), "needs": needing.len(), "skipped": 0, "destination": dest.label}))
}

/// Import pack…: the path, then where to keep it.
pub(super) fn ask_import(ctx: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = ctx.py();
    let c = ctx.clone().unbind();
    let done = func(py, move |py, args| {
        let path = args.get_item(0)?;
        if path.is_none() {
            return Ok(none(py));
        }
        let path: String = path.extract()?;
        let ctx = c.bind(py);
        // Format 1 files keep their old behaviour: into this model.
        let plan = match proto(py)?.call_method1("pack_read", (&path,)) {
            Ok(p) => p,
            Err(e) => {
                flash(ctx, &e.value(py).str()?.to_string())?;
                return Ok(none(py));
            }
        };
        if plan.get_item("v")?.extract::<u64>()? == 1 {
            let rows = val(py, &plan.get_item("rows")?)?;
            super::presets::import_v1(ctx, rows)?;
            return Ok(none(py));
        }
        let scene = ctx.getattr_opt("scene")?.filter(|s| s.hasattr("_root").unwrap_or(false));
        let dests: Vec<(String, String)> = super::destinations::all(py, scene.as_ref())?
            .into_iter()
            .filter(|d| d.writable)
            .map(|d| (d.key, d.label))
            .collect();
        let rows = val(py, &plan.get_item("rows")?)?;
        let count = rows.as_array().map_or(0, Vec::len);
        let needs = rows.as_array().into_iter().flatten().filter(|r| r.get("needs").is_some()).count();
        let name = val(py, &plan.get_item("pack")?)?["name"].as_str().unwrap_or("").to_owned();
        let forms = py.import("built_render.chrome_forms")?;
        let field = forms.getattr("Field")?;
        let fields = PyList::empty(py);
        let summary = if needs > 0 {
            format!("{count} detail{} · {needs} need something this Library lacks and will be listed, not applied", if count == 1 { "" } else { "s" })
        } else {
            format!("{count} detail{}, all ready", if count == 1 { "" } else { "s" })
        };
        fields.append(field.call1(("summary", "text", if name.is_empty() { "Pack".to_owned() } else { name }, summary))?)?;
        let options: Vec<(String, String)> = dests.clone();
        let first = dests.iter().find(|d| d.0 == "mine").or(dests.first()).map(|d| d.0.clone()).unwrap_or_default();
        let kw = PyDict::new(py);
        kw.set_item("options", options)?;
        fields.append(field.call(("dest", "choice", "Keep in", first), Some(&kw))?)?;
        let c2 = ctx.clone().unbind();
        let path2 = path.clone();
        let finish = func(py, move |py, args| {
            if !args.get_item(0)?.eq("ok")? {
                return Ok(none(py));
            }
            let ctx = c2.bind(py);
            let dest: String = args.get_item(1)?.get_item("dest")?.extract()?;
            match import(py, ctx, &path2, &dest) {
                Ok(r) => flash(ctx, &format!("Imported {} detail{} into {}{}", r["ok"], if r["ok"] == 1 { "" } else { "s" }, text(&r["destination"]),
                    if r["needs"].as_u64().unwrap_or(0) > 0 { format!(" · {} need something missing", r["needs"]) } else { String::new() }))?,
                Err(e) => flash(ctx, &e.value(py).str()?.to_string())?,
            }
            Ok(none(py))
        })?;
        let kw = PyDict::new(py);
        kw.set_item("on_done", finish)?;
        kw.set_item("buttons", [("cancel", "Cancel"), ("ok", "Import")])?;
        let form = forms.getattr("Form")?.call(("Import pack", fields), Some(&kw))?;
        forms.call_method1("show", (ctx.getattr("win")?, form))?;
        Ok(none(py))
    })?;
    py.import("built_render.chrome_forms")?
        .getattr("ask_text")?
        .call((ctx.getattr("win")?, "Import pack", "Pack folder, pack.json or .builtpack"), Some(&[("done", done)].into_py_dict(py)?))?;
    Ok(())
}

/// Import a pack without the dialogs (scripts and tests).
#[pyfunction]
#[pyo3(signature=(win, path, destination="mine"))]
fn import_pack<'py>(win: &Bound<'py, PyAny>, path: &str, destination: &str) -> PyResult<Bound<'py, PyAny>> {
    let py = win.py();
    let ctx = ctx_of(py, win, &win.getattr("view")?)?;
    obj_of(py, &import(py, &ctx, path, destination)?)
}

/// Share pack without the dialog: details by uid from everything the
/// Library lists.
#[pyfunction]
#[pyo3(signature=(win, uids, dest, meta=None))]
fn share_pack<'py>(win: &Bound<'py, PyAny>, uids: Vec<String>, dest: &str, meta: Option<&Bound<'py, PyAny>>) -> PyResult<Bound<'py, PyAny>> {
    let py = win.py();
    let ctx = ctx_of(py, win, &win.getattr("view")?)?;
    let rows = all_rows(&ctx)?;
    let details: Vec<Value> = uids
        .iter()
        .map(|u| rows.iter().find(|r| r["uid"] == u.as_str()).cloned().ok_or_else(|| err(format!("Unknown detail {u}"))))
        .collect::<PyResult<_>>()?;
    let meta = match meta {
        Some(m) => val(py, m)?,
        None => json!({"name": "My details"}),
    };
    let scene = win.getattr("viewport")?.getattr("scene")?;
    obj_of(py, &export(py, Some(&scene), &details, &rows, &meta, dest)?)
}

pub(super) fn register(sub: &Bound<'_, PyModule>) -> PyResult<()> {
    sub.add_function(wrap_pyfunction!(import_pack, sub)?)?;
    sub.add_function(wrap_pyfunction!(share_pack, sub)?)
}
