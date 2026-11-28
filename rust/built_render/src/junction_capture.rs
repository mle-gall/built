// SPDX-License-Identifier: GPL-3.0-or-later
//! Capture (docs/junctions.md §5, §11): after a step leaves two or more
//! cuts or parts following the same two members, a quiet chip "Save as
//! junction · N similar" at the meeting; it fades after 8 s or when the
//! user works elsewhere. Its popover lists the roles painted on the members
//! (Swap), what it includes (ticked, fixable), the rule with its live count,
//! the parameters found with their relations, and saves the detail and its
//! first use in one History step. Nothing is saved without Save.
use pyo3::prelude::*;
use pyo3::types::PyDict;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

type Obj<'p> = Bound<'p, PyAny>;
const ATTR: &str = "_junction_capture";
const FADE: Duration = Duration::from_secs(8);

struct Chip {
    key: String,
    at: [f64; 3],
    seed: Value,
    similar: usize,
    born: Instant,
}

#[pyclass(unsendable)]
pub struct Capture {
    seed: Value,
    options: Value,
    draft: Value,
    open: bool,
    chip: Option<Chip>,
    offers: HashMap<String, usize>,
    dismissed: HashSet<String>,
    revision: Option<u64>,
    error: String,
    /// Editing inside a junction: (junction, assembly, its children and
    /// local rows when entered).
    inside: Option<(String, String, HashSet<String>, HashSet<String>)>,
    /// Left it with new items: (junction, items, uses, detail name).
    done: Option<(String, Vec<String>, u64, String)>,
}

fn api(py: Python<'_>) -> PyResult<Obj<'_>> {
    Ok(py.import("built_core.command_protocol")?.into_any())
}
fn to_json(v: &Obj<'_>) -> PyResult<Value> {
    let raw: String = v.py().import("json")?.call_method1("dumps", (v,))?.extract()?;
    serde_json::from_str(&raw).map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
}
fn to_py<'p>(py: Python<'p>, v: &Value) -> PyResult<Obj<'p>> {
    py.import("json")?.call_method1("loads", (v.to_string(),))
}
fn tr(py: Python<'_>, text: &str) -> PyResult<String> {
    crate::settings_rows::tr(py, text)
}

/// Whether the capture popover is open (without creating the state).
pub fn popover_open(scene: &Obj<'_>) -> PyResult<bool> {
    if scene.is_none() {
        return Ok(false);
    }
    Ok(scene.getattr_opt(ATTR)?.and_then(|r| r.cast_into::<Capture>().ok()).is_some_and(|s| s.borrow().open))
}
fn state<'p>(scene: &Obj<'p>) -> PyResult<Bound<'p, Capture>> {
    if let Some(s) = scene.getattr_opt(ATTR)?.and_then(|r| r.cast_into::<Capture>().ok()) {
        return Ok(s);
    }
    let s = Bound::new(
        scene.py(),
        Capture {
            seed: Value::Null,
            options: json!({}),
            draft: Value::Null,
            open: false,
            chip: None,
            offers: HashMap::new(),
            dismissed: HashSet::new(),
            revision: None,
            error: String::new(),
            inside: None,
            done: None,
        },
    )?;
    scene.setattr(ATTR, &s)?;
    Ok(s)
}

fn preview(scene: &Obj<'_>, seed: &Value, options: &Value) -> PyResult<Value> {
    let py = scene.py();
    to_json(&api(py)?.call_method1("junction_capture_preview", (scene, to_py(py, seed)?, to_py(py, options)?))?)
}

/// Open the popover on a seed (the chip, or an item's context menu).
pub fn open(scene: &Obj<'_>, seed: Value) -> PyResult<String> {
    let s = state(scene)?;
    match preview(scene, &seed, &json!({})) {
        Ok(draft) => {
            let mut c = s.borrow_mut();
            c.seed = seed;
            c.options = json!({});
            c.draft = draft;
            c.open = true;
            c.error.clear();
            Ok(String::new())
        }
        Err(e) => Ok(e.value(scene.py()).str()?.to_string()),
    }
}

fn close(scene: &Obj<'_>) -> PyResult<()> {
    let s = state(scene)?;
    let mut c = s.borrow_mut();
    c.open = false;
    if let Some(chip) = c.chip.take() {
        c.dismissed.insert(chip.key);
    }
    Ok(())
}

fn refresh(scene: &Obj<'_>) -> PyResult<()> {
    let s = state(scene)?;
    let (seed, options) = {
        let c = s.borrow();
        (c.seed.clone(), c.options.clone())
    };
    match preview(scene, &seed, &options) {
        Ok(d) => {
            let mut c = s.borrow_mut();
            c.draft = d;
            c.error.clear();
        }
        Err(e) => s.borrow_mut().error = e.value(scene.py()).str()?.to_string(),
    }
    Ok(())
}

/// A popover action: `open`, `swap`, `tick:<id>`, `name:<text>`,
/// `tol:<deg>`, `mirror`, `param:<key>:<expr>`, `save`, `cancel`.
pub fn action(win: &Obj<'_>, text: &str) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let (scene, history) = (vp.getattr("scene")?, vp.getattr("history")?);
    let (verb, arg) = text.split_once(':').unwrap_or((text, ""));
    let s = state(&scene)?;
    let mut message = String::new();
    match verb {
        "open" => {
            let seed = if arg.is_empty() {
                s.borrow().chip.as_ref().map(|c| c.seed.clone())
            } else {
                Some(json!(arg))
            };
            if let Some(seed) = seed {
                message = open(&scene, seed)?;
            }
        }
        "cancel" => close(&scene)?,
        "swap" => {
            {
                let mut c = s.borrow_mut();
                let on = c.options["swap"] == true;
                c.options["swap"] = json!(!on);
            }
            refresh(&scene)?;
        }
        "tick" => {
            {
                let mut c = s.borrow_mut();
                let mut ticked: Vec<String> = c.draft["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|i| i["ticked"] == true)
                    .filter_map(|i| i["id"].as_str().map(String::from))
                    .collect();
                if let Some(at) = ticked.iter().position(|i| i == arg) {
                    ticked.remove(at);
                } else {
                    ticked.push(arg.to_string());
                }
                c.options["include"] = json!(ticked);
            }
            refresh(&scene)?;
        }
        "name" => {
            s.borrow_mut().options["name"] = json!(arg);
            refresh(&scene)?;
        }
        "tol" => {
            if let Ok(v) = arg.parse::<f64>() {
                s.borrow_mut().options["tol"] = json!(v.clamp(0., 45.));
                refresh(&scene)?;
            }
        }
        "mirror" => {
            {
                let mut c = s.borrow_mut();
                let exact = c.options["mirror"] == "exact";
                c.options["mirror"] = json!(if exact { "allow" } else { "exact" });
            }
            refresh(&scene)?;
        }
        "param" => {
            if let Some((key, expr)) = arg.split_once(':') {
                s.borrow_mut().options["exprs"][key] = json!(expr);
                refresh(&scene)?;
            }
        }
        "done_all" | "done_one" => {
            let done = s.borrow_mut().done.take();
            if let Some((junction, items, _, _)) = done {
                let kw = PyDict::new(py);
                kw.set_item("junction", &junction)?;
                kw.set_item("rows", items)?;
                kw.set_item("detail", verb == "done_all")?;
                let reply = outcome(&py.import("built_core.protocol.commands")?.call_method("run", (&history, "junction_adopt"), Some(&kw))?)?;
                if reply["ok"] != true {
                    message = reply["error"]["message"].as_str().or(reply["error"].as_str()).unwrap_or("Could not add them").to_string();
                }
            }
        }
        "done_dismiss" => {
            s.borrow_mut().done = None;
        }
        "save" => {
            let (seed, options) = {
                let c = s.borrow();
                (c.seed.clone(), c.options.clone())
            };
            let kw = PyDict::new(py);
            kw.set_item("seed", to_py(py, &seed)?)?;
            kw.set_item("options", to_py(py, &options)?)?;
            let reply = outcome(&py.import("built_core.protocol.commands")?.call_method("run", (&history, "junction_capture"), Some(&kw))?)?;
            if reply["ok"] == true {
                close(&scene)?;
                s.borrow_mut().revision = history.getattr("revision")?.extract().ok();
                // The Junction tool next applies the detail just saved, not
                // the one it was armed with (a bolt group, say).
                if let Some(uid) = reply["detail"].as_str() {
                    remember_detail(win, &scene, uid)?;
                }
            } else {
                let why = reply["error"]["message"].as_str().or(reply["error"].as_str()).unwrap_or("Could not save the junction").to_string();
                s.borrow_mut().error = why.clone();
                message = why;
            }
        }
        _ => {}
    }
    if !message.is_empty() {
        vp.call_method1("flash_status", (message,))?;
    }
    win.getattr("view")?.call_method0("update")?;
    Ok(())
}

fn remember_detail(win: &Obj<'_>, scene: &Obj<'_>, uid: &str) -> PyResult<()> {
    let py = win.py();
    win.getattr("viewport")?.setattr("_junction_detail", uid)?;
    if let Some(view) = win.getattr_opt("view")? {
        view.setattr("_junction_detail", uid)?;
    }
    let catalog = api(py)?.call_method1("preset_catalog", (scene,))?;
    let preset = catalog.try_iter()?.filter_map(|e| e.ok()?.get_item(0).ok()).find(|p| p.get_item("uid").is_ok_and(|u| u.eq(uid).unwrap_or(false)));
    let tool = win.getattr_opt("_tools")?.and_then(|t| t.get_item("junction").ok());
    if let (Some(preset), Some(tool)) = (preset, tool) {
        tool.getattr("_machine")?.call_method1("set", ("preset", preset))?;
    }
    Ok(())
}

fn pixel(view: &Obj<'_>, at: [f64; 3]) -> PyResult<Option<(f64, f64)>> {
    let p = view.py().import("built_core")?.getattr("Vec3")?.call1((at[0], at[1], at[2]))?;
    Ok(view.call_method1("_world_to_pixel", (p,))?.extract::<(f64, f64)>().ok())
}

fn children(g: &Obj<'_>) -> PyResult<HashSet<String>> {
    let mut out = HashSet::new();
    for c in g.getattr("children")?.try_iter()? {
        out.insert(c?.getattr("uid")?.extract::<String>()?);
    }
    Ok(out)
}
fn local_rows(record: &Value) -> HashSet<String> {
    record["contents"].as_array().into_iter().flatten().filter(|c| c["local"] == true).filter_map(|c| c["row"].as_str().or(c["junction"].as_str()).map(String::from)).collect()
}
/// Editing inside a junction (option B): remember what it holds when the
/// context opens; when it closes with new cuts or parts, ask once whether
/// they go to every use of the detail or to this one only.
fn track_inside(scene: &Obj<'_>) -> PyResult<()> {
    let py = scene.py();
    let s = state(scene)?;
    let open = scene.getattr("edit_group")?;
    let current = if open.is_none() { None } else { Some(open.getattr("uid")?.extract::<String>()?) };
    let entered = s.borrow().inside.as_ref().map(|i| i.1.clone());
    if current == entered {
        return Ok(());
    }
    // Taken first: the borrow must end before the Done chip is set below.
    let left = s.borrow_mut().inside.take();
    if let Some((junction, assembly, before, local)) = left {
        if let (Ok(record), Some(g)) = (
            api(py)?.call_method1("junction_record", (scene, &junction)).and_then(|r| to_json(&r)),
            scene.call_method0("groups_by_uid")?.get_item(&assembly).ok(),
        ) {
            let mut items: Vec<String> = children(&g)?.difference(&before).cloned().collect();
            items.extend(local_rows(&record).difference(&local).cloned());
            if !items.is_empty() {
                let uses = record["uses"].as_u64().unwrap_or(1);
                s.borrow_mut().done = Some((junction, items, uses, record["detail"]["name"].as_str().unwrap_or("").to_string()));
            }
        }
    }
    if let Some(uid) = current {
        if let Ok(record) = api(py)?.call_method1("junction_record", (scene, &uid)).and_then(|r| to_json(&r)) {
            let junction = record["uid"].as_str().unwrap_or_default().to_string();
            s.borrow_mut().inside = Some((junction, uid, children(&open)?, local_rows(&record)));
            s.borrow_mut().done = None;
        }
    }
    Ok(())
}

/// After a step: offer the chip where new items gathered at a meeting.
fn follow(scene: &Obj<'_>, history: &Obj<'_>) -> PyResult<()> {
    let py = scene.py();
    let s = state(scene)?;
    let revision: u64 = history.getattr("revision")?.extract().unwrap_or(0);
    if s.borrow().revision == Some(revision) {
        return Ok(());
    }
    let first = s.borrow().revision.is_none();
    s.borrow_mut().revision = Some(revision);
    if s.borrow().open {
        let seed_alive = preview(scene, &s.borrow().seed, &s.borrow().options).is_ok();
        if seed_alive {
            refresh(scene)?;
        } else {
            close(scene)?;
        }
        return Ok(());
    }
    let offers = to_json(&api(py)?.call_method1("junction_capture_offers", (scene,))?)?;
    let mut counts = HashMap::new();
    let mut fresh = None;
    for o in offers.as_array().into_iter().flatten() {
        let key = o["key"].as_str().unwrap_or_default().to_string();
        let n = o["items"].as_array().map_or(0, Vec::len);
        let before = s.borrow().offers.get(&key).copied().unwrap_or(0);
        if !first && n > before && n >= 2 && !s.borrow().dismissed.contains(&key) {
            fresh = Some((key.clone(), o.clone()));
        }
        counts.insert(key, n);
    }
    let mut c = s.borrow_mut();
    c.offers = counts;
    if let Some(chip) = &c.chip {
        // Working elsewhere: the meeting no longer gathers anything new.
        if !c.offers.contains_key(&chip.key) || fresh.as_ref().is_some_and(|(k, _)| *k != chip.key) {
            c.chip = None;
        }
    }
    if let Some((key, o)) = fresh {
        let seed = o["items"].as_array().and_then(|i| i.last()).cloned().unwrap_or(Value::Null);
        drop(c);
        if let Ok(draft) = preview(scene, &seed, &json!({})) {
            let at: [f64; 3] = serde_json::from_value(o["at"].clone()).unwrap_or_default();
            s.borrow_mut().chip = Some(Chip { key, at, seed, similar: draft["similar"].as_u64().unwrap_or(0) as usize, born: Instant::now() });
        }
    }
    Ok(())
}

fn fmt(v: &Value) -> String {
    match v.as_f64() {
        Some(n) if (n - n.round()).abs() < 1e-6 => format!("{}", n.round() as i64),
        Some(n) => format!("{:.1}", n),
        None => v.as_str().unwrap_or("").to_string(),
    }
}

/// What the chrome draws: the chip, the painted roles and the popover.
fn layer(view: &Obj<'_>, scene: &Obj<'_>) -> PyResult<Value> {
    let py = view.py();
    let s = state(scene)?;
    let mut c = s.borrow_mut();
    // Faded, not dismissed: more work at the meeting offers it again.
    if c.chip.as_ref().is_some_and(|ch| !c.open && ch.born.elapsed() > FADE) {
        c.chip = None;
    }
    let mut out = json!({"chip": null, "tags": [], "popover": null, "done": null});
    if let Some((_, items, uses, name)) = &c.done {
        let all = if *uses > 1 { tr(py, "Apply to all {n}")?.replace("{n}", &uses.to_string()) } else { tr(py, "Add to the detail")? };
        out["done"] = json!({"text": format!("{} · {} {}", name, items.len(), tr(py, if items.len() == 1 { "new item" } else { "new items" })?),
                             "all": all, "one": tr(py, "This one only")?});
    }
    if let Some(ch) = c.chip.as_ref().filter(|_| !c.open) {
        if let Some((x, y)) = pixel(view, ch.at)? {
            let text = match ch.similar {
                0 => tr(py, "Save as junction")?,
                1 => tr(py, "Save as junction · 1 similar")?,
                n => tr(py, "Save as junction · {n} similar")?.replace("{n}", &n.to_string()),
            };
            let fade = 1. - (ch.born.elapsed().as_secs_f64() / FADE.as_secs_f64()).clamp(0., 1.);
            out["chip"] = json!({"text": text, "x": x, "y": y, "fade": fade});
        }
    }
    if !c.open {
        return Ok(out);
    }
    let d = &c.draft;
    let mut tags = Vec::new();
    for (i, role) in [tr(py, "Main")?, tr(py, "Joining")?].iter().enumerate() {
        if let Some(at) = d["paint"].get(i).and_then(|p| serde_json::from_value::<[f64; 3]>(p.clone()).ok()) {
            if let Some((x, y)) = pixel(view, at)? {
                tags.push(json!({"text": format!("{role} · {}", d["names"][i].as_str().unwrap_or("")), "x": x, "y": y, "main": i == 0}));
            }
        }
    }
    out["tags"] = json!(tags);
    let rule = &d["detail"]["values"]["rule"];
    let range = |role: &str| {
        let r = &rule["roles"][role];
        let names: Vec<String> = r["profile"].as_array().into_iter().flatten().filter_map(|p| p["library"].as_str()).map(profile_label).collect();
        // "190" when the range is one size, "200–240" otherwise.
        let size = r["size"]["height"].as_array().map(|h| if fmt(&h[0]) == fmt(&h[1]) { format!("{} mm", fmt(&h[0])) } else { format!("{}–{} mm", fmt(&h[0]), fmt(&h[1])) }).unwrap_or_default();
        if names.is_empty() { size } else { format!("{} · {size}", names.join(", ")) }
    };
    let how = format!("{} · {}° ± {}°", d["meeting"]["kind"].as_str().unwrap_or("").replace('_', " "), fmt(&rule["angle"]["value"]), fmt(&rule["angle"]["tol"]));
    let items: Vec<Value> = d["items"].as_array().into_iter().flatten().map(|i| json!({
        "id": i["id"], "label": i["label"], "meta": if let Some(p) = i["problem"].as_str() { format!("{} · {p}", i["meta"].as_str().unwrap_or("")) } else { i["meta"].as_str().unwrap_or("").to_string() },
        "ticked": i["ticked"], "problem": i["problem"].is_string(), "near": i["how"] == "near",
    })).collect();
    let params: Vec<Value> = d["params"].as_array().into_iter().flatten().map(|p| {
        let alternatives: Vec<Value> = p["alternatives"].as_array().cloned().unwrap_or_default();
        let current = p["expr"].clone();
        // A click on the relation tries the next one, then a fixed value.
        let mut cycle: Vec<String> = alternatives.iter().filter_map(|a| a.as_str().map(String::from)).collect();
        cycle.push(String::new());
        let at = cycle.iter().position(|e| Value::String(e.clone()) == current).map_or(0, |i| (i + 1) % cycle.len());
        json!({"key": p["key"], "name": p["name"], "value": fmt(&p["value"]), "unit": p["unit"], "relation": p["relation"], "next": format!("{}:{}", p["key"].as_str().unwrap_or(""), cycle[at])})
    }).collect();
    let similar = d["similar"].as_u64().unwrap_or(0);
    out["popover"] = json!({
        "title": tr(py, "Save as junction detail")?,
        "at": format!("{} {} × {}", tr(py, "at")?, d["names"][0].as_str().unwrap_or(""), d["names"][1].as_str().unwrap_or("")),
        "name": d["detail"]["name"],
        "members": [
            {"role": tr(py, "Main")?, "name": d["names"][0], "meta": d["meeting"]["roles"][if c.options["swap"] == true {1} else {0}]["face"].as_str().map(|f| f.replace('_', " ")).unwrap_or_default()},
            {"role": tr(py, "Joining")?, "name": d["names"][1], "meta": d["meeting"]["roles"][if c.options["swap"] == true {0} else {1}]["end"].as_str().map(|e| format!("{e} end")).unwrap_or_default()},
        ],
        "items": items,
        "matches": [[tr(py, "Main")?, range("main")], [tr(py, "Joining")?, range("joining")], [tr(py, "Meeting")?, how]],
        "tol": rule["angle"]["tol"],
        "mirrored": rule["mirror"] != "exact",
        "params": params,
        "similar": if similar == 1 { tr(py, "1 similar")? } else { tr(py, "{n} similar")?.replace("{n}", &similar.to_string()) },
        "near": d["near"],
        "save_in": tr(py, "This project")?,
        "meeting_x": pixel(view, serde_json::from_value::<[f64; 3]>(d["meeting"]["at"].clone()).unwrap_or_default())?.map(|p| p.0),
        "error": c.error,
    });
    Ok(out)
}

/// Before each frame: follow History, fade the chip, send the layer.
pub fn before_frame(view: &Obj<'_>, win: &Obj<'_>) -> PyResult<()> {
    let py = view.py();
    let vp = win.getattr("viewport")?;
    let (scene, history) = (vp.getattr("scene")?, vp.getattr("history")?);
    if scene.is_none() {
        return Ok(());
    }
    follow(&scene, &history)?;
    track_inside(&scene)?;
    // Stamps an open left for later are checked a few milliseconds a frame.
    let (left, stale): (usize, bool) = api(py)?.call_method1("live_verify", (&scene, 4.0))?.extract()?;
    if left > 0 || stale {
        view.call_method0("update")?;
    }
    let renderer = view.getattr("renderer")?;
    if !renderer.hasattr("ui_set_junction_capture")? {
        return Ok(());
    }
    let layer = layer(view, &scene)?;
    let text = layer.to_string();
    if view.getattr_opt("_ui_capture_sent")?.and_then(|v| v.extract::<String>().ok()).as_deref() != Some(text.as_str()) {
        renderer.call_method1("ui_set_junction_capture", (text,))?;
        view.setattr("_ui_capture_sent", layer.to_string())?;
    }
    if layer["popover"].is_object() {
        let hint = tr(py, "Tick what belongs to the junction · Swap the roles · Save detail, or Esc")?;
        py.import("built_render")?.call_method1("sync_status_readout", (view, tr(py, "Save as junction")?, hint))?;
    } else if layer["chip"].is_object() {
        // The chip fades: keep frames coming until it is gone.
        view.call_method0("update")?;
    }
    Ok(())
}

/// Esc closes the popover. True when the key was the popover's.
pub fn key(view: &Obj<'_>, escape: bool) -> PyResult<bool> {
    let scene = match view.getattr_opt("scene")?.filter(|s| !s.is_none()) {
        Some(s) => s,
        None => return Ok(false),
    };
    if !escape || !state(&scene)?.borrow().open {
        return Ok(false);
    }
    close(&scene)?;
    view.call_method0("update")?;
    Ok(true)
}

/// Scripts and the agent bridge: `junction_capture_action(win, "open:<seed>")`.
#[pyfunction]
pub fn junction_capture_action(win: &Obj<'_>, text: &str) -> PyResult<()> {
    action(win, text)
}

/// The capture layer's state, for tests.
#[pyfunction]
pub fn junction_capture_state<'p>(scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = scene.py();
    let s = state(scene)?;
    let c = s.borrow();
    to_py(py, &json!({
        "open": c.open, "chip": c.chip.as_ref().map(|ch| json!({"key": ch.key, "similar": ch.similar, "seed": ch.seed})),
        "draft": c.draft, "options": c.options, "error": c.error,
    }))
}

fn live_row(scene: &Obj<'_>, uid: &str) -> PyResult<Option<Value>> {
    let rows = to_json(&api(scene.py())?.call_method1("live_rows", (scene,))?)?;
    Ok(rows.as_array().into_iter().flatten().find(|r| r["uid"] == uid).cloned())
}
fn run(history: &Obj<'_>, verb: &str, kw: Value) -> PyResult<Value> {
    let py = history.py();
    let d = PyDict::new(py);
    for (k, v) in kw.as_object().into_iter().flatten() {
        d.set_item(k, to_py(py, v)?)?;
    }
    outcome(&py.import("built_core.protocol.commands")?.call_method("run", (history, verb), Some(&d))?)
}
/// A protocol reply's outcome (its `step` is not data).
fn outcome(reply: &Obj<'_>) -> PyResult<Value> {
    let ok = reply.get_item("ok")?.is_truthy()?;
    let error = reply.get_item("error")?;
    let message = if error.is_none() {
        reply.get_item("rejected").ok().filter(|r| !r.is_none()).map(|r| r.str().map(|s| s.to_string())).transpose()?
    } else {
        Some(error.get_item("message").unwrap_or(error.clone()).str()?.to_string())
    };
    Ok(json!({"ok": ok, "error": {"message": message}}))
}
fn failed(reply: &Value) -> Option<String> {
    (reply["ok"] != true).then(|| reply["error"]["message"].as_str().or(reply["error"].as_str()).unwrap_or("Not applied").to_string())
}

/// The junction record's actions (inspector `junction.<verb>`).
pub fn record_action(win: &Obj<'_>, verb: &str, v: &Obj<'_>) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let (scene, history) = (vp.getattr("scene")?, vp.getattr("history")?);
    let v = to_json(v)?;
    let uid = v.as_str().or(v["uid"].as_str()).unwrap_or_default().to_string();
    let mut note = String::new();
    let reply = match verb {
        "param" => {
            let Some(mut row) = live_row(&scene, &uid)? else { return Ok(()) };
            let key = v["key"].as_str().unwrap_or_default();
            if v["scope"] == "one" || row["detail"].is_null() {
                if row["overrides"].is_null() {
                    row["overrides"] = json!({});
                }
                row["overrides"][key] = v["value"].clone();
                run(&history, "junction_set", json!({"row": row}))?
            } else {
                let detail = row["detail"]["uid"].as_str().unwrap_or_default().to_string();
                let catalog = to_json(&api(py)?.call_method1("preset_catalog", (&scene,))?)?;
                let Some(mut preset) = catalog.as_array().into_iter().flatten().filter_map(|e| e.get(0)).find(|p| p["uid"] == detail.as_str()).cloned() else { return Ok(()) };
                for p in preset["values"]["params"].as_array_mut().into_iter().flatten() {
                    if p["key"] == key {
                        p["expr"] = v["value"].clone();
                    }
                }
                let r = run(&history, "preset_update", json!({"rows": [preset]}))?;
                if r["ok"] == true {
                    let record = to_json(&api(py)?.call_method1("junction_record", (&scene, &uid))?)?;
                    let own = record["params"].as_array().into_iter().flatten().find(|p| p["key"] == key).and_then(|p| p["overridden_uses"].as_u64()).unwrap_or(0);
                    note = format!("{} · rev {} · {} uses", record["detail"]["name"].as_str().unwrap_or("Detail"), record["rev"], record["uses"]);
                    if own > 0 {
                        note.push_str(&format!(" · {own} keep their own value"));
                    }
                }
                r
            }
        }
        "swap" => {
            let Some(mut row) = live_row(&scene, &uid)? else { return Ok(()) };
            let roles = row["roles"].clone();
            row["roles"]["main"] = roles["joining"].clone();
            row["roles"]["joining"] = roles["main"].clone();
            for r in ["main", "joining"] {
                row["roles"][r].as_object_mut().map(|o| o.remove("face"));
            }
            run(&history, "junction_set", json!({"row": row}))?
        }
        "reset" | "detach" => {
            let Some(mut row) = live_row(&scene, &uid)? else { return Ok(()) };
            if verb == "reset" {
                row["overrides"] = json!({});
            } else {
                row["detail"] = Value::Null;
            }
            run(&history, "junction_set", json!({"row": row}))?
        }
        "upgrade" => run(&history, "junction_upgrade", json!({"uid": uid}))?,
        "release" => run(&history, "junction_release", json!({"uid": uid}))?,
        "release_all" => run(&history, "junction_release", json!({"uid": uid, "deep": true}))?,
        "widen" => {
            let record = to_json(&api(py)?.call_method1("junction_record", (&scene, &uid))?)?;
            let reason = &record["reason"];
            let detail = record["detail"]["uid"].as_str().unwrap_or_default().to_string();
            let catalog = to_json(&api(py)?.call_method1("preset_catalog", (&scene,))?)?;
            let Some(mut preset) = catalog.as_array().into_iter().flatten().filter_map(|e| e.get(0)).find(|p| p["uid"] == detail.as_str()).cloned() else { return Ok(()) };
            let rule = &mut preset["values"]["rule"];
            let value = &reason["value"];
            match (reason["criterion"].as_str(), reason["role"].as_str()) {
                (Some("angle"), _) => {
                    let want = rule["angle"]["value"].as_f64().unwrap_or(0.);
                    rule["angle"]["tol"] = json!((value.as_f64().unwrap_or(want) - want).abs().ceil());
                }
                (Some("size"), Some(role)) => {
                    let v = value.as_f64().unwrap_or(0.);
                    for (_, range) in rule["roles"][role]["size"].as_object_mut().into_iter().flatten() {
                        let lo = range[0].as_f64().unwrap_or(v).min(v);
                        let hi = range[1].as_f64().unwrap_or(v).max(v);
                        *range = json!([lo, hi]);
                    }
                }
                (Some("profile"), Some(role)) => {
                    if let Some(list) = rule["roles"][role]["profile"].as_array_mut() {
                        list.push(json!({"library": value}));
                    }
                }
                _ => {}
            }
            run(&history, "preset_update", json!({"rows": [preset]}))?
        }
        "edit" => {
            let record = to_json(&api(py)?.call_method1("junction_record", (&scene, &uid))?)?;
            let owner = record["owner"].as_str().unwrap_or_default();
            let g = scene.call_method0("groups_by_uid")?.get_item(owner)?;
            vp.call_method1("begin_group_edit", (g,))?;
            json!({"ok": true})
        }
        "select_all" => {
            let rows = to_json(&api(py)?.call_method1("live_rows", (&scene,))?)?;
            let groups = scene.call_method0("groups_by_uid")?;
            let detail = if v.is_string() { uid.clone() } else { v["detail"].as_str().unwrap_or_default().to_string() };
            let picks = pyo3::types::PyList::empty(py);
            for r in rows.as_array().into_iter().flatten().filter(|r| r["kind"] == "junction" && r["detail"]["uid"] == detail.as_str() && r.get("parent").is_none()) {
                if let Ok(g) = groups.get_item(r["owner"].as_str().unwrap_or_default()) {
                    picks.append(g)?;
                }
            }
            scene.call_method1("select", (picks,))?;
            json!({"ok": true})
        }
        "review" => {
            let detail = if v.is_string() { uid.clone() } else { v["detail"].as_str().unwrap_or_default().to_string() };
            crate::junction_review::start(&scene, &history, &detail)?;
            json!({"ok": true})
        }
        _ => json!({"ok": true}),
    };
    if let Some(why) = failed(&reply) {
        win.call_method1("notify", ("warning", why))?;
    } else if !note.is_empty() {
        win.call_method1("notify", ("info", note))?;
    }
    vp.call_method0("notify_scene_changed")?;
    win.getattr("view")?.call_method0("update")?;
    Ok(())
}

/// A catalogue profile's label as the Library shows it ("HE 200 A"), else
/// the key's last part.
fn profile_label(key: &str) -> String {
    static LABELS: std::sync::LazyLock<std::collections::HashMap<String, String>> = std::sync::LazyLock::new(|| {
        let catalogue: Value = serde_json::from_str(include_str!("../../../resources/profiles/starter.builtprofile")).unwrap_or_default();
        catalogue["table"]["sizes"].as_array().into_iter().flatten()
            .filter_map(|s| Some((s["key"].as_str()?.to_owned(), s["label"].as_str()?.to_owned())))
            .collect()
    });
    LABELS.get(key).cloned().unwrap_or_else(|| key.rsplit('/').next().unwrap_or(key).to_uppercase())
}
