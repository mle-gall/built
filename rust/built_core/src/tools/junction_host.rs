// SPDX-License-Identifier: GPL-3.0-or-later
//! The view side of the Junction tool: the meeting under the cursor with
//! the chosen detail's fit and ghost, the plies a parts detail picks, the
//! detail palette, ↓ cycling and applying (asking once on a Repeat copy).
use super::{
    host_geometry::Projection,
    view_queries::{hit, ViewportToolHost},
};
use crate::math::xyz;
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyCFunction, PyDict, PyList, PyTuple},
};
use serde_json::{json, Value};

type Obj<'p> = Bound<'p, PyAny>;

fn api(py: Python<'_>) -> PyResult<Obj<'_>> {
    Ok(py.import("built_core.command_protocol")?.into_any())
}
fn to_json(v: &Obj<'_>) -> PyResult<Value> {
    super::py::py_to_json(v)
}
/// A protocol reply's outcome (its `step` is not data).
fn outcome(reply: &Obj<'_>) -> PyResult<Value> {
    let ok = reply.get_item("ok")?.is_truthy()?;
    let error = reply.get_item("error")?;
    let message = if error.is_none() {
        reply.get_item("rejected").ok().filter(|r| !r.is_none()).map(|r| r.str().map(|s| s.to_string())).transpose()?
    } else {
        Some(error.get_item("message").or_else(|_| Ok::<_, PyErr>(error.clone()))?.str()?.to_string())
    };
    Ok(json!({"ok": ok, "error": message}))
}
fn tr(py: Python<'_>, text: &str) -> PyResult<String> {
    py.import("built_core.i18n")?.call_method1("tr", (text,))?.extract()
}
fn to_py<'p>(py: Python<'p>, v: &Value) -> PyResult<Obj<'p>> {
    py.import("json")?.call_method1("loads", (v.to_string(),))
}

/// Include installed Library presets when browsing and resolving child details.
fn catalog(scene: &Obj<'_>) -> PyResult<Vec<Value>> {
    let py = scene.py();
    let mut rows: Vec<Value> = to_json(&api(py)?.call_method1("preset_catalog", (scene,))?)?
        .as_array().into_iter().flatten().filter_map(|e| e.get(0).cloned()).collect();
    let saved = py.import("built_render.wgpu_library")?.call_method1("library_items", ("presets", scene))?;
    for entry in saved.try_iter()? {
        let row = to_json(&entry?.get_item("preset")?)?;
        if row.get("needs").is_none() && !rows.iter().any(|p| p["uid"] == row["uid"]) {
            rows.push(row);
        }
    }
    Ok(rows)
}
fn details(scene: &Obj<'_>) -> PyResult<Vec<Value>> {
    Ok(catalog(scene)?.into_iter().filter(|p| p["primitive"] == "junction").collect())
}
fn by_uid(scene: &Obj<'_>, uid: &str) -> PyResult<Value> {
    Ok(details(scene)?.into_iter().find(|p| p["uid"] == uid).unwrap_or(Value::Null))
}

/// The placement under the cursor, the source occurrence of a shown copy.
fn picked<'p>(view: &Obj<'p>, px: (f64, f64)) -> PyResult<Option<(Obj<'p>, Obj<'p>)>> {
    let Some(pair) = hit(view, "pick_face_placement", px)? else { return Ok(None) };
    let (face, placed) = (pair.get_item(0)?, pair.get_item(1)?);
    if placed.is_none() {
        return Ok(None);
    }
    let group = placed.getattr_opt("source_occurrence")?.filter(|v| !v.is_none()).unwrap_or(placed);
    Ok(Some((group, face)))
}

impl ViewportToolHost {
    pub(super) fn junction_service(&self, py: Python<'_>, verb: &str, arg: &Value) -> PyResult<Value> {
        let view = self.vp.bind(py);
        let scene = view.getattr("scene")?;
        Ok(match verb {
            "junction_pack_hint" => {
                let installed = details(&scene)?.iter().any(|p| p["source"]["path"].is_string());
                if installed { Value::Null } else { json!(tr(py, "More profiles and details come as packs: Import pack…")?) }
            }
            "junction_default" => {
                let all = details(&scene)?;
                let remembered = view.getattr_opt("_junction_detail")?.and_then(|v| v.extract::<String>().ok());
                let saved = scene.getattr_opt("_junction_last")?.and_then(|v| v.extract::<String>().ok());
                remembered
                    .or(saved)
                    .and_then(|uid| all.iter().find(|p| p["uid"] == uid.as_str()).cloned())
                    .or_else(|| {
                        let t = api(py).ok()?.call_method1("junction_counts", (&scene,)).ok()?;
                        let counts = to_json(&t).ok()?;
                        let first = counts["details"].as_object()?.keys().next()?.clone();
                        all.iter().find(|p| p["uid"] == first.as_str()).cloned()
                    })
                    .unwrap_or(Value::Null)
            }
            "junction_picker" => {
                let canvas = view.call_method0("window")?.getattr("_host")?.getattr("view")?;
                canvas.getattr("renderer")?.call_method1("ui_open_commands", ("", "Junction details"))?;
                canvas.call_method0("update")?;
                Value::Null
            }
            "junction_selection" => {
                let selected = scene.getattr("selection")?;
                if selected.len()? != 1 {
                    return Ok(Value::Null);
                }
                let g = selected.try_iter()?.next().transpose()?;
                match g.filter(|g| g.hasattr("uid").unwrap_or(false)) {
                    Some(g) => json!({"placement": g.getattr("uid")?.extract::<String>()?, "label": g.getattr("name")?.extract::<String>().unwrap_or_default()}),
                    None => Value::Null,
                }
            }
            "junction_ply" => {
                let px: (f64, f64) = serde_json::from_value(arg["px"].clone()).unwrap_or_default();
                let Some((g, face)) = picked(view, px)? else { return Ok(Value::Null) };
                let uid: String = g.getattr("uid")?.extract()?;
                let name: String = g.getattr("name")?.extract().unwrap_or_default();
                let mut out = json!({"placement": uid, "label": name, "fit": "ply"});
                // A flange of an I-section member: bolts measure through it.
                let row = api(py)?.call_method1("live_sweep", (&scene, &uid))?;
                if !row.is_none() && !face.is_none() {
                    let profile = row.get_item("profile")?;
                    if profile.call_method1("get", ("family", ""))?.eq("i")? || profile.call_method1("get", ("library", ""))?.eq("basic/i")? {
                        let normal = xyz(&face.call_method0("normal")?)?;
                        let path: Vec<[f64; 3]> = row.call_method1("get", ("resolved_points",))?.extract().unwrap_or_default();
                        if path.len() >= 2 {
                            let axis = (DVec3::from(path[1]) - DVec3::from(path[0])).normalize_or_zero();
                            if normal.dot(axis).abs() < 0.1 {
                                out["face"] = json!("flange_outer");
                            }
                        }
                    }
                }
                out
            }
            "junction_hover" => {
                let detail = to_py(py, &arg["detail"])?;
                let px: (f64, f64) = serde_json::from_value(arg["px"].clone()).unwrap_or_default();
                let meetings = to_json(&api(py)?.call_method1("meetings", (&scene,))?)?;
                let path: Vec<String> = match picked(view, px)? {
                    Some((g, _)) => scene.call_method1("occurrence_path", (&g,))?.extract().unwrap_or_default(),
                    None => Vec::new(),
                };
                let world = arg["world"].as_array().and_then(|w| Some(DVec3::new(w.first()?.as_f64()?, w.get(1)?.as_f64()?, w.get(2)?.as_f64()?)));
                let at = |m: &Value| -> DVec3 { serde_json::from_value::<[f64; 3]>(m["at"].clone()).map(DVec3::from).unwrap_or(DVec3::ZERO) };
                let camera = Projection::from_view(view)?;
                let cursor = glam::DVec2::new(px.0, px.1);
                let mut best: Option<(f64, &Value)> = None;
                for m in meetings.as_array().into_iter().flatten() {
                    let members: Vec<&str> = m["members"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                    let touched = members.iter().any(|u| path.iter().any(|p| p == u));
                    let screen = camera.pixel(at(m))?.map(|p| cursor.distance(glam::DVec2::from(p)));
                    let score = if touched {
                        world.map_or(screen.unwrap_or(0.), |w| w.distance(at(m)) * 1000.)
                    } else if screen.is_some_and(|d| d < 36.) {
                        1e6 + screen.unwrap_or(0.)
                    } else {
                        continue;
                    };
                    if best.is_none_or(|(b, _)| score < b) {
                        best = Some((score, m));
                    }
                }
                let Some((_, m)) = best else { return Ok(Value::Null) };
                let key = m["key"].as_str().unwrap_or_default();
                let names: Vec<String> = m["members"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|u| scene.call_method0("groups_by_uid").ok()?.get_item(u.as_str()?).ok()?.getattr("name").ok()?.extract::<String>().ok())
                    .collect();
                let mut out = json!({"key": key, "kind": m["kind"], "at": m["at"], "label": names.join(" × "), "fit": "other",
                                     "roles": {"main": m["roles"]["main"], "joining": m["roles"]["other"]}, "arrangement": m["arrangement"]});
                if arg["detail"].is_object() {
                    let reply = to_json(&api(py)?.call_method1("junction_suggest", (&scene, &detail))?)?;
                    if let Some(s) = reply["suggestions"].as_array().into_iter().flatten().find(|s| s["key"] == key) {
                        for f in ["fit", "roles", "why", "near", "existing", "mirrored"] {
                            out[f] = s[f].clone();
                        }
                    }
                    if matches!(out["fit"].as_str(), Some("match" | "near" | "other")) {
                        if let Ok(g) = api(py)?.call_method1("junction_ghosts", (&scene, &detail, vec![key])) {
                            if let Some(lines) = to_json(&g)?.get(0).map(|g| g["lines"].clone()) {
                                out["ghost"] = lines;
                            }
                        }
                    }
                }
                // A Repeat copy (or the source of one) on either side.
                let repeat = out["roles"].as_object().into_iter().flatten().any(|(_, b)| b.get("station").is_some())
                    || m["members"].as_array().into_iter().flatten().filter_map(Value::as_str).any(|u| {
                        api(py).and_then(|a| a.call_method1("live_repeat", (&scene, u))).is_ok_and(|r| !r.is_none())
                    });
                out["repeat"] = json!(repeat);
                out
            }
            "junction_cycle" => {
                let key = arg["key"].as_str().unwrap_or_default();
                let current = arg["detail"].as_str().unwrap_or_default();
                let all: Vec<Value> = details(&scene)?.into_iter().filter(|p| p["values"]["meeting"] != "parts").collect();
                let start = all.iter().position(|p| p["uid"] == current).map_or(0, |i| i + 1);
                let mut found = Value::Null;
                for i in 0..all.len() {
                    let p = &all[(start + i) % all.len()];
                    if p["uid"] == current {
                        continue;
                    }
                    let reply = to_json(&api(py)?.call_method1("junction_suggest", (&scene, to_py(py, p)?))?)?;
                    if key.is_empty() || reply["suggestions"].as_array().into_iter().flatten().any(|s| s["key"] == key && s["fit"] == "match") {
                        found = p.clone();
                        break;
                    }
                }
                if let Some(uid) = found["uid"].as_str() {
                    view.setattr("_junction_detail", uid)?;
                }
                found
            }
            "junction_apply" => {
                let detail = arg["detail"].as_str().unwrap_or_default().to_string();
                let win = view.call_method0("window")?;
                let history = view.getattr("history")?;
                let rows = catalog(&scene)?;
                let defs = api(py)?.call_method1("junction_definitions", (to_py(py, &arg["preset"])?, to_py(py, &json!(rows))?))?.unbind();
                if arg["repeat"] == true && arg["force"] != true {
                    let name = by_uid(&scene, &detail)?["name"].as_str().unwrap_or("Junction").to_string();
                    let tr = |s: &str| tr(py, s);
                    let items = vec![tr("Every copy, both ends")?, tr("Every copy, this end")?, tr("This meeting only")?];
                    let bindings = arg["bindings"].clone();
                    let end = bindings["joining"]["end"].as_str().unwrap_or("end").to_string();
                    let (held_history, held_win) = (history.clone().unbind(), win.clone().unbind());
                    let choices = items.clone();
                    let done = PyCFunction::new_closure(py, None, None, move |args: &Bound<'_, PyTuple>, _: Option<&Bound<'_, PyDict>>| -> PyResult<()> {
                        let py = args.py();
                        let Ok(choice) = args.get_item(0)?.extract::<String>() else { return Ok(()) };
                        let kw = PyDict::new(py);
                        kw.set_item("detail", &detail)?;
                        kw.set_item("definitions", defs.bind(py))?;
                        kw.set_item("bindings", to_py(py, &bindings)?)?;
                        if choice == choices[0] {
                            kw.set_item("along", to_py(py, &json!({"end": "both"}))?)?;
                        } else if choice == choices[1] {
                            kw.set_item("along", to_py(py, &json!({"end": end}))?)?;
                        }
                        let reply = py.import("built_core.protocol.commands")?.call_method("run", (held_history.bind(py), "junction_apply"), Some(&kw))?;
                        let win = held_win.bind(py);
                        if !reply.get_item("ok")?.is_truthy()? {
                            let error = reply.get_item("error")?;
                            win.call_method1("notify", ("warning", error.str()?))?;
                        }
                        win.getattr("view")?.call_method0("update")?;
                        Ok(())
                    })?;
                    py.import("built_render")?.getattr("chrome_forms")?.call_method(
                        "ask_choice",
                        (&win, tr("Apply along the Repeat")?, format!("{name} · {}", tr("on a Repeat copy")?), PyList::new(py, &items)?),
                        Some(&{
                            let kw = PyDict::new(py);
                            kw.set_item("done", done)?;
                            kw
                        }),
                    )?;
                    return Ok(json!({"ok": true, "asked": true, "message": "Choose where to apply"}));
                }
                let kw = PyDict::new(py);
                kw.set_item("detail", &detail)?;
                        kw.set_item("definitions", defs.bind(py))?;
                kw.set_item("bindings", to_py(py, &arg["bindings"])?)?;
                kw.set_item("force", arg["force"] == true)?;
                let reply = outcome(&py.import("built_core.protocol.commands")?.call_method("run", (&history, "junction_apply"), Some(&kw))?)?;
                if reply["ok"] == true {
                    view.setattr("_junction_detail", &detail)?;
                    json!({"ok": true})
                } else {
                    json!({"ok": false, "error": reply["error"].as_str().unwrap_or("Could not apply the detail")})
                }
            }
            _ => Value::Null,
        })
    }
}
