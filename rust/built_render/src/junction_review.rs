// SPDX-License-Identifier: GPL-3.0-or-later
//! Reviewing a junction detail's similar meetings (docs/junctions.md §4.4,
//! §11): a badge and a ghost at each meeting in the view, the list in the
//! chrome with why each one fits, near misses with Widen, Skip remembered
//! per meeting, and Enter applying the ticked meetings in one History step.
//! Nothing is applied without Enter or the list's Apply.
use crate::ui::tokens::{ACCENT, DIM, OFF, WARN};
use pyo3::prelude::*;
use pyo3::types::{PyCFunction, PyDict, PyList, PyTuple};
use serde_json::{json, Value};
use std::collections::HashMap;

type Obj<'p> = Bound<'p, PyAny>;
const ATTR: &str = "_junction_review";
/// Ghosts built at once; the rest when they are ticked or hovered.
const GHOSTS: usize = 32;
const BADGE: (f64, f64) = (14., 9.);

#[derive(Clone, Debug)]
pub struct Item {
    pub key: String,
    pub fit: String,
    pub at: [f64; 3],
    pub label: String,
    pub why: Vec<String>,
    pub roles: Value,
    pub near: Value,
    pub existing: Value,
    pub ticked: bool,
}

#[pyclass(unsendable)]
pub struct Review {
    pub detail: String,
    pub name: String,
    pub items: Vec<Item>,
    ghosts: HashMap<String, Vec<[[f64; 3]; 2]>>,
    revision: u64,
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

/// The review open on this document, if any.
pub fn current<'p>(scene: &Obj<'p>) -> PyResult<Option<Bound<'p, Review>>> {
    if scene.is_none() {
        return Ok(None);
    }
    Ok(scene
        .getattr_opt(ATTR)?
        .and_then(|r| r.cast_into::<Review>().ok()))
}

fn items(reply: &Value, keep: &HashMap<String, bool>) -> Vec<Item> {
    let mut out: Vec<Item> = reply["suggestions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["fit"] != "applied")
        .map(|s| {
            let key = s["key"].as_str().unwrap_or_default().to_string();
            let fit = s["fit"].as_str().unwrap_or_default().to_string();
            Item {
                ticked: keep.get(&key).copied().unwrap_or(fit == "match") && fit == "match",
                at: serde_json::from_value(s["at"].clone()).unwrap_or_default(),
                label: s["label"].as_str().unwrap_or_default().to_string(),
                why: serde_json::from_value(s["why"].clone()).unwrap_or_default(),
                roles: s["roles"].clone(),
                near: s["near"].clone(),
                existing: s["existing"].clone(),
                key,
                fit,
            }
        })
        .collect();
    let rank = |f: &str| match f {
        "match" => 0,
        "near" => 1,
        "detailed" => 2,
        _ => 3,
    };
    out.sort_by(|a, b| rank(&a.fit).cmp(&rank(&b.fit)).then(a.label.cmp(&b.label)));
    out
}

impl Review {
    fn load(&mut self, scene: &Obj<'_>) -> PyResult<()> {
        let py = scene.py();
        let reply = to_json(&api(py)?.call_method1("junction_suggest", (scene, &self.detail))?)?;
        self.name = reply["detail"]["name"].as_str().unwrap_or_default().to_string();
        let keep: HashMap<String, bool> = self.items.iter().map(|i| (i.key.clone(), i.ticked)).collect();
        self.items = items(&reply, &keep);
        let wanted: Vec<String> = self
            .items
            .iter()
            .filter(|i| matches!(i.fit.as_str(), "match" | "near") && !self.ghosts.contains_key(&i.key))
            .take(GHOSTS)
            .map(|i| i.key.clone())
            .collect();
        if !wanted.is_empty() {
            let ghosts = api(py)?.call_method1("junction_ghosts", (scene, &self.detail, wanted))?;
            for g in to_json(&ghosts)?.as_array().into_iter().flatten() {
                if let (Some(key), Ok(lines)) = (g["key"].as_str(), serde_json::from_value(g["lines"].clone())) {
                    self.ghosts.insert(key.to_string(), lines);
                }
            }
        }
        Ok(())
    }
    pub fn ticked(&self) -> usize {
        self.items.iter().filter(|i| i.ticked).count()
    }
}

/// Open the review of a detail on this document.
pub fn start(scene: &Obj<'_>, history: &Obj<'_>, detail: &str) -> PyResult<()> {
    let py = scene.py();
    let mut review = Review {
        detail: detail.to_string(),
        name: String::new(),
        items: Vec::new(),
        ghosts: HashMap::new(),
        revision: history.getattr("revision")?.extract().unwrap_or(0),
    };
    review.load(scene)?;
    scene.setattr(ATTR, Bound::new(py, review)?)?;
    Ok(())
}

/// Leave the review: nothing is applied.
pub fn close(scene: &Obj<'_>) -> PyResult<()> {
    if current(scene)?.is_some() {
        scene.setattr(ATTR, scene.py().None())?;
    }
    Ok(())
}

fn run(history: &Obj<'_>, verb: &str, kwargs: &Bound<'_, PyDict>) -> PyResult<Value> {
    let py = history.py();
    let reply = py
        .import("built_core.protocol.commands")?
        .call_method("run", (history, verb), Some(kwargs))?;
    to_json(&reply)
}

/// The step after History moved (undo, another edit): re-read the list.
fn follow(scene: &Obj<'_>, history: &Obj<'_>) -> PyResult<()> {
    let Some(review) = current(scene)? else { return Ok(()) };
    let revision: u64 = history.getattr("revision")?.extract().unwrap_or(0);
    let mut r = review.borrow_mut();
    if r.revision != revision {
        r.revision = revision;
        if let Err(e) = r.load(scene) {
            drop(r);
            close(scene)?;
            return Err(e);
        }
    }
    Ok(())
}

/// Enter: the ticked meetings, one History step.
pub fn apply(scene: &Obj<'_>, history: &Obj<'_>) -> PyResult<String> {
    let py = scene.py();
    let Some(review) = current(scene)? else { return Ok(String::new()) };
    let (detail, name, bindings) = {
        let r = review.borrow();
        let b: Vec<Value> = r.items.iter().filter(|i| i.ticked).map(|i| i.roles.clone()).collect();
        (r.detail.clone(), r.name.clone(), b)
    };
    close(scene)?;
    if bindings.is_empty() {
        return Ok(tr(py, "Nothing ticked: nothing applied")?);
    }
    let kw = PyDict::new(py);
    kw.set_item("detail", &detail)?;
    kw.set_item("bindings", to_py(py, &json!(bindings))?)?;
    let reply = run(history, "junction_apply", &kw)?;
    if reply["ok"] != true {
        return Ok(reply["error"]["message"].as_str().or(reply["error"].as_str()).unwrap_or("Could not apply").to_string());
    }
    Ok(tr(py, "{name} applied to {count} meetings")?
        .replace("{name}", &name)
        .replace("{count}", &bindings.len().to_string()))
}

fn skip(scene: &Obj<'_>, history: &Obj<'_>, keys: Vec<String>, on: bool) -> PyResult<()> {
    let py = scene.py();
    let Some(review) = current(scene)? else { return Ok(()) };
    let detail = review.borrow().detail.clone();
    if keys.is_empty() {
        return Ok(());
    }
    let kw = PyDict::new(py);
    kw.set_item("detail", detail)?;
    kw.set_item("meeting", keys)?;
    kw.set_item("skip", on)?;
    run(history, "junction_skip", &kw)?;
    follow(scene, history)
}

/// Widen the detail by the near miss's one change: a detail revision.
fn widen(scene: &Obj<'_>, history: &Obj<'_>, index: usize) -> PyResult<String> {
    let py = scene.py();
    let Some(review) = current(scene)? else { return Ok(String::new()) };
    let (detail, near) = {
        let r = review.borrow();
        let Some(item) = r.items.get(index) else { return Ok(String::new()) };
        (r.detail.clone(), item.near.clone())
    };
    let catalog = to_json(&api(py)?.call_method1("preset_catalog", (scene,))?)?;
    let Some(mut row) = catalog
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get(0))
        .find(|p| p["uid"] == detail.as_str())
        .cloned()
    else {
        return Ok(String::new());
    };
    let rule = &mut row["values"]["rule"];
    let to = near["widen_to"].clone();
    match (near["criterion"].as_str(), near["role"].as_str()) {
        (Some("angle"), _) => rule["angle"] = to,
        (Some("offset"), _) => rule["offset"] = to,
        (Some("mirror"), _) => rule["mirror"] = to,
        (Some("size"), Some(role)) => {
            for (dim, range) in to.as_object().into_iter().flatten() {
                rule["roles"][role]["size"][dim] = range.clone();
            }
        }
        (Some(criterion), Some(role)) => rule["roles"][role][criterion] = to,
        _ => return Ok(String::new()),
    }
    let kw = PyDict::new(py);
    kw.set_item("rows", to_py(py, &json!([row]))?)?;
    let reply = run(history, "preset_update", &kw)?;
    follow(scene, history)?;
    Ok(if reply["ok"] == true {
        tr(py, "Detail widened: a new revision")?
    } else {
        reply["error"]["message"].as_str().unwrap_or("Could not widen the detail").to_string()
    })
}

/// A list or badge action: `tick:<i>`, `skip:<i>`, `unskip:<i>`,
/// `widen:<i>`, `apply`, `skip_all`, `close`.
pub fn action(win: &Obj<'_>, text: &str) -> PyResult<()> {
    let vp = win.getattr("viewport")?;
    let (scene, history) = (vp.getattr("scene")?, vp.getattr("history")?);
    let (verb, arg) = text.split_once(':').unwrap_or((text, ""));
    let index: Option<usize> = arg.parse().ok();
    let key = |i: usize| -> PyResult<Option<String>> {
        Ok(current(&scene)?.and_then(|r| r.borrow().items.get(i).map(|item| item.key.clone())))
    };
    let mut message = String::new();
    match verb {
        "tick" => {
            if let (Some(r), Some(i)) = (current(&scene)?, index) {
                if let Some(item) = r.borrow_mut().items.get_mut(i).filter(|item| item.fit == "match") {
                    item.ticked = !item.ticked;
                }
            }
        }
        "skip" | "unskip" => {
            if let Some(k) = index.map(key).transpose()?.flatten() {
                skip(&scene, &history, vec![k], verb == "skip")?;
            }
        }
        "skip_all" => {
            let keys: Vec<String> = current(&scene)?
                .map(|r| r.borrow().items.iter().filter(|i| i.fit == "match").map(|i| i.key.clone()).collect())
                .unwrap_or_default();
            skip(&scene, &history, keys, true)?;
            close(&scene)?;
        }
        "widen" => {
            if let Some(i) = index {
                message = widen(&scene, &history, i)?;
            }
        }
        "apply" => message = apply(&scene, &history)?,
        "close" => close(&scene)?,
        _ => {}
    }
    if !message.is_empty() {
        vp.call_method1("flash_status", (message,))?;
    }
    win.getattr("view")?.call_method0("update")?;
    Ok(())
}

fn badge_at(view: &Obj<'_>, at: [f64; 3]) -> PyResult<Option<(f64, f64)>> {
    let py = view.py();
    let p = py.import("built_core")?.getattr("Vec3")?.call1((at[0], at[1], at[2]))?;
    let px = view.call_method1("_world_to_pixel", (p,))?;
    Ok(px.extract::<(f64, f64)>().ok())
}

/// A click on a badge ticks or unticks it; with Alt it skips the meeting
/// (or un-skips it). True when the click was the review's.
pub fn press(view: &Obj<'_>, x: f64, y: f64, alt: bool) -> PyResult<bool> {
    let scene = view.getattr("scene")?;
    let Some(review) = current(&scene)? else { return Ok(false) };
    let hit = {
        let r = review.borrow();
        let mut hit = None;
        for (i, item) in r.items.iter().enumerate() {
            if let Some((bx, by)) = badge_at(view, item.at)? {
                if (x - bx).abs() <= BADGE.0 && (y - by).abs() <= BADGE.1 {
                    hit = Some(i);
                    break;
                }
            }
        }
        hit
    };
    let Some(i) = hit else { return Ok(false) };
    let win = view.call_method0("window")?;
    let fit = review.borrow().items[i].fit.clone();
    let verb = match (alt, fit.as_str()) {
        (_, "skipped") => "unskip",
        (true, "match" | "near") => "skip",
        (false, "match") => "tick",
        _ => return Ok(true),
    };
    action(&win, &format!("{verb}:{i}"))?;
    Ok(true)
}

/// Enter applies, Esc leaves. True when the key was the review's.
pub fn key(view: &Obj<'_>, enter: bool, escape: bool) -> PyResult<bool> {
    let scene = view.getattr("scene")?;
    if current(&scene)?.is_none() || !(enter || escape) {
        return Ok(false);
    }
    let win = view.call_method0("window")?;
    action(&win, if enter { "apply" } else { "close" })?;
    Ok(true)
}

fn hex(c: egui::Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

/// Badges at each meeting and ghost lines for the ticked ones, as overlay
/// primitives (`views.wgpu_overlay.Primitive`).
pub fn primitives<'p>(view: &Obj<'p>, scene: &Obj<'p>, out: &Bound<'p, PyList>) -> PyResult<()> {
    let py = view.py();
    let Some(review) = current(scene)? else { return Ok(()) };
    let primitive = py.import("views.wgpu_overlay")?.getattr("Primitive")?;
    let vec3 = py.import("built_core")?.getattr("Vec3")?;
    let r = review.borrow();
    for item in r.items.iter().filter(|i| i.ticked || i.fit == "near") {
        let colour = hex(if item.ticked { ACCENT } else { WARN });
        for [a, b] in r.ghosts.get(&item.key).into_iter().flatten() {
            let seg = view.call_method1("_segment_to_pixels", (vec3.call1((a[0], a[1], a[2]))?, vec3.call1((b[0], b[1], b[2]))?))?;
            if let Ok([p, q]) = seg.extract::<[(f64, f64); 2]>() {
                let kw = PyDict::new(py);
                kw.set_item("dash", (4., 3.))?;
                out.append(primitive.call(("polyline", [p, q], &colour, 1.5), Some(&kw))?)?;
            }
        }
    }
    for item in &r.items {
        let Some((x, y)) = badge_at(view, item.at)? else { continue };
        let (fill, edge) = match (item.fit.as_str(), item.ticked) {
            ("match", true) => (ACCENT, ACCENT),
            ("match", false) => (egui::Color32::from_gray(40), ACCENT),
            ("near", _) => (egui::Color32::from_gray(40), WARN),
            ("skipped", _) => (egui::Color32::from_gray(40), OFF),
            _ => (egui::Color32::from_gray(40), DIM),
        };
        let kw = PyDict::new(py);
        kw.set_item("fill", hex(fill))?;
        let rect = ((x - BADGE.0, y - BADGE.1), (x + BADGE.0, y + BADGE.1));
        out.append(primitive.call(("rectangle", rect, hex(edge), 1.5), Some(&kw))?)?;
        // A tick for a ticked match, a dash for the others.
        let ink = hex(if item.ticked { egui::Color32::WHITE } else { edge });
        let mark: Vec<(f64, f64)> = if item.ticked {
            vec![(x - 6., y), (x - 2., y + 4.), (x + 6., y - 4.)]
        } else if item.fit == "skipped" {
            vec![(x - 5., y - 4.), (x + 5., y + 4.)]
        } else {
            vec![(x - 5., y), (x + 5., y)]
        };
        out.append(primitive.call1(("polyline", mark, ink, 2.))?)?;
    }
    Ok(())
}

/// The chrome's list: (title, count, rows, apply count), each row
/// (index, label, meta, status, tone, ticked, tickable, actions).
pub type Row = (usize, String, String, String, String, bool, bool, Vec<(String, String)>);
pub fn panel(py: Python<'_>, scene: &Obj<'_>) -> PyResult<Option<(String, String, Vec<Row>, usize)>> {
    let Some(review) = current(scene)? else { return Ok(None) };
    let r = review.borrow();
    let shown = r.items.iter().filter(|i| i.fit != "applied").count();
    let mut rows = Vec::new();
    for (i, item) in r.items.iter().enumerate() {
        let (status, tone, tickable, actions) = match item.fit.as_str() {
            "match" => (tr(py, "fits")?, "ok", true, vec![("skip".into(), tr(py, "Skip")?)]),
            "near" => {
                let n = &item.near;
                let status = format!("{} · {}", n["value"].as_f64().map(|v| format!("{v}")).unwrap_or_else(|| n["value"].to_string()), tr(py, "outside")?);
                (status, "warn", false, vec![("widen".into(), tr(py, "Widen")?), ("skip".into(), tr(py, "Skip")?)])
            }
            "skipped" => (tr(py, "skipped")?, "off", false, vec![("unskip".into(), tr(py, "Un-skip")?)]),
            _ => {
                let name = item.existing["name"].as_str().unwrap_or("custom").to_string();
                (format!("{} {name}", tr(py, "has")?), "dim", false, vec![])
            }
        };
        let mut meta = item.why.join(" · ");
        if item.fit == "near" {
            let n = &item.near;
            let what = match n["criterion"].as_str().unwrap_or("") {
                "angle" => format!("{} ± {}°", tr(py, "Widen the angle to")?, n["widen_to"]["tol"]),
                "size" => tr(py, "Widen the size range to include it")?,
                "profile" => tr(py, "Add this profile to the detail")?,
                "offset" => format!("{} {} mm", tr(py, "Widen the offset to")?, n["widen_to"]["tol_mm"]),
                _ => tr(py, "Widen the detail to include it")?,
            };
            meta = what;
        }
        rows.push((i, item.label.clone(), meta, status, tone.to_string(), item.ticked, tickable, actions));
    }
    let title = tr(py, "Apply {name}")?.replace("{name}", &r.name);
    let count = tr(py, "{n} similar")?.replace("{n}", &shown.to_string());
    Ok(Some((title, count, rows, r.ticked())))
}

/// Before each frame: follow History, offer a review after a detail is
/// saved, and send the list to the chrome.
pub fn before_frame(view: &Obj<'_>, win: &Obj<'_>) -> PyResult<()> {
    let py = view.py();
    let vp = win.getattr("viewport")?;
    let (scene, history) = (vp.getattr("scene")?, vp.getattr("history")?);
    if scene.is_none() {
        return Ok(());
    }
    if let Some(saved) = scene.getattr_opt("_junction_saved")?.filter(|s| !s.is_none()) {
        scene.setattr("_junction_saved", py.None())?;
        for entry in saved.try_iter()? {
            let (uid, name): (String, String) = entry?.extract()?;
            offer(win, &scene, &history, &uid, &name)?;
        }
    }
    follow(&scene, &history)?;
    let renderer = view.getattr("renderer")?;
    if renderer.hasattr("ui_set_junction_review")? {
        let panel = panel(py, &scene)?;
        let key = format!("{panel:?}");
        if view.getattr_opt("_ui_review_sent")?.and_then(|v| v.extract::<String>().ok()).as_deref() != Some(key.as_str()) {
            renderer.call_method1("ui_set_junction_review", (panel,))?;
            view.setattr("_ui_review_sent", key)?;
            let hint = if current(&scene)?.is_some() {
                tr(py, "Review · Click a badge to tick, Alt-click to skip · Enter applies the ticked · Esc leaves, nothing applied")?
            } else {
                String::new()
            };
            if !hint.is_empty() {
                vp.call_method1("flash_status", (hint,))?;
            }
        }
    }
    Ok(())
}

/// "Knee end plate saved · 7 similar meetings · Review", when it has some.
fn offer(win: &Obj<'_>, scene: &Obj<'_>, history: &Obj<'_>, uid: &str, name: &str) -> PyResult<()> {
    let py = scene.py();
    let reply = to_json(&api(py)?.call_method1("junction_suggest", (scene, uid))?)?;
    let similar = reply["counts"]["match"].as_u64().unwrap_or(0);
    if similar == 0 {
        return Ok(());
    }
    let text = if similar == 1 {
        tr(py, "{name} saved · 1 similar meeting")?
    } else {
        tr(py, "{name} saved · {n} similar meetings")?
    }
    .replace("{name}", name)
    .replace("{n}", &similar.to_string());
    let (held_scene, held_history, detail) = (scene.clone().unbind(), history.clone().unbind(), uid.to_string());
    let held_win = win.clone().unbind();
    let review = PyCFunction::new_closure(py, None, None, move |args: &Bound<'_, PyTuple>, _: Option<&Bound<'_, PyDict>>| -> PyResult<()> {
        let py = args.py();
        start(held_scene.bind(py), held_history.bind(py), &detail)?;
        held_win.bind(py).getattr("view")?.call_method0("update")?;
        Ok(())
    })?;
    let actions = PyList::empty(py);
    actions.append((tr(py, "Review")?, review))?;
    win.call_method1("notify", ("info", text, actions))?;
    Ok(())
}

/// Open the review from a script or the agent bridge: `review(win, detail)`.
#[pyfunction]
pub fn junction_review(win: &Obj<'_>, detail: &str) -> PyResult<()> {
    let vp = win.getattr("viewport")?;
    start(&vp.getattr("scene")?, &vp.getattr("history")?, detail)?;
    win.getattr("view")?.call_method0("update")?;
    Ok(())
}

/// A list action from a script or the agent bridge (`tick:3`, `apply`…).
#[pyfunction]
pub fn junction_review_action(win: &Obj<'_>, text: &str) -> PyResult<()> {
    action(win, text)
}

/// The open review's state, for tests: detail, items (key, fit, ticked).
#[pyfunction]
pub fn junction_review_state<'p>(scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = scene.py();
    let Some(review) = current(scene)? else { return Ok(py.None().into_bound(py)) };
    let r = review.borrow();
    let items: Vec<Value> = r
        .items
        .iter()
        .map(|i| json!({"key": i.key, "fit": i.fit, "ticked": i.ticked, "label": i.label, "ghost": r.ghosts.get(&i.key).map_or(0, Vec::len)}))
        .collect();
    to_py(py, &json!({"detail": r.detail, "name": r.name, "items": items}))
}
