// SPDX-License-Identifier: GPL-3.0-or-later
//! View templates and title blocks in the Library (docs/library.md).
//!
//! A **view template** is a view's setup without its place: the camera
//! kind (perspective, parallel, two-point, field of view), the display
//! style, which tags and tag folders are hidden, and the section planes
//! (the active plane with its position, planes shown, cuts shown). It is
//! saved as a Library item of kind `scenes` and applied to the view in
//! front; the camera stays where it is.
//!
//! A **title block** is a sheet's title block as a Library item of kind
//! `title_blocks`: its rows, columns, layout, lines and colours, never its
//! place on a particular sheet. Applied to the sheet in front, it replaces
//! that sheet's title block (one History step) or adds one.
//!
//! Both go through the shared Save to Library drawer, so they take the
//! same destinations as every other kind.
use super::destinations;
use super::*;
use crate::settings_rows::{tr, trf};
use serde_json::{json, Value};

pub(crate) const VIEW_FORMAT: &str = "built.view_template";
pub(crate) const BLOCK_FORMAT: &str = "built.title_block";

fn err(text: impl Into<String>) -> PyErr {
    pyo3::exceptions::PyValueError::new_err(text.into())
}

fn to_value(v: &Bound<'_, PyAny>) -> PyResult<Value> {
    let text: String = v.py().import("json")?.call_method1("dumps", (v,))?.extract()?;
    serde_json::from_str(&text).map_err(|e| err(e.to_string()))
}

fn to_py<'py>(py: Python<'py>, v: &Value) -> PyResult<Bound<'py, PyAny>> {
    py.import("json")?.call_method1("loads", (v.to_string(),))
}

/// The view in front and its scene.
fn front<'py>(win: &Bound<'py, PyAny>) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyAny>)> {
    let vp = match win.call_method0("_active_view") {
        Ok(vp) if !vp.is_none() => vp,
        _ => win.getattr("viewport")?,
    };
    let scene = vp.getattr("scene")?;
    Ok((vp, scene))
}

// ---- view templates --------------------------------------------------------------------------

/// The setup of the view in front, as a view template's payload.
pub(crate) fn capture_view(win: &Bound<'_, PyAny>) -> PyResult<Value> {
    let py = win.py();
    let (vp, scene) = front(win)?;
    let camera = vp.getattr("camera")?;
    let style = match win.call_method0("active_style") {
        Ok(s) if !s.is_none() => s,
        _ => scene.getattr("display_style")?,
    };
    let saved = py.import("built_core.saved_views")?.getattr("SavedView")?;
    let view = saved.call_method1("capture", ("", &scene, &camera, &style))?;
    let mut names = std::collections::HashMap::new();
    for ly in scene.getattr("layers")?.try_iter()? {
        let ly = ly?;
        names.insert(ly.getattr("key")?.str()?.to_string(), ly.getattr("name")?.str()?.to_string());
    }
    let hidden: Vec<Value> = to_value(&view.getattr("hidden_layers")?)?
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|k| k.as_str())
        .map(|k| json!({"key": k, "name": names.get(k).cloned().unwrap_or_default()}))
        .collect();
    let section = to_value(&view.getattr("section")?)?;
    let mut active = Value::Null;
    let plane = scene.call_method0("active_section")?;
    if plane.is_truthy()? {
        active = to_value(&plane.call_method0("to_dict")?)?;
    }
    Ok(json!({
        "format": VIEW_FORMAT, "v": 1,
        "camera": {
            "perspective": view.getattr("perspective")?.is_truthy()?,
            "two_point": view.getattr("two_point")?.is_truthy()?,
            "fov_deg": view.getattr("fov_deg")?.extract::<f64>()?,
            "lens": to_value(&view.getattr("lens")?)?,
        },
        "style": to_value(&view.getattr("style")?)?,
        "hidden_tags": hidden,
        "hidden_folders": to_value(&view.getattr("hidden_folders")?)?,
        "section": {
            "active": active,
            "planes_shown": section["planes_shown"].as_bool().unwrap_or(true),
            "cuts_shown": section["cuts_shown"].as_bool().unwrap_or(true),
        },
    }))
}

/// What a template holds, in the words of the detail pane.
pub(crate) fn view_rows(data: &Value) -> Vec<(String, String, String)> {
    let camera = &data["camera"];
    let kind = if camera["perspective"] == false {
        "Parallel".to_owned()
    } else if camera["two_point"] == true {
        "Two-point perspective".to_owned()
    } else {
        format!("Perspective · {:.0}°", camera["fov_deg"].as_f64().unwrap_or(45.0))
    };
    let tags: Vec<&str> = data["hidden_tags"].as_array().into_iter().flatten().filter_map(|t| t["name"].as_str().filter(|n| !n.is_empty()).or(t["key"].as_str())).collect();
    let folders = data["hidden_folders"].as_array().map_or(0, Vec::len);
    let hidden = match (tags.len(), folders) {
        (0, 0) => "Every tag shown".to_owned(),
        (n, 0) if n <= 3 => format!("Hidden: {}", tags.join(", ")),
        (n, 0) => format!("{n} tags hidden"),
        (n, f) => format!("{n} tags and {f} folders hidden"),
    };
    let section = &data["section"];
    let cut = match section["active"]["name"].as_str() {
        Some(name) if !name.is_empty() => format!("Cut by {name}"),
        Some(_) => "Cut by a section plane".to_owned(),
        None if section["active"].is_object() => "Cut by a section plane".to_owned(),
        None => "No active cut".to_owned(),
    };
    let planes = if section["planes_shown"] == false { "planes hidden" } else { "planes shown" };
    vec![
        ("Camera".to_owned(), kind, String::new()),
        ("Style".to_owned(), data["style"]["name"].as_str().unwrap_or("Current style").to_owned(), String::new()),
        ("Tags".to_owned(), hidden, String::new()),
        ("Sections".to_owned(), format!("{cut} · {planes}"), String::new()),
    ]
}

/// Apply a view template to the view in front. Returns what could not be
/// matched in this model (tags by key, then by name), in words; empty
/// when everything applied.
pub(crate) fn apply_view(win: &Bound<'_, PyAny>, data: &Value) -> PyResult<String> {
    let py = win.py();
    if data["format"] != VIEW_FORMAT {
        return Err(err("Not a view template"));
    }
    let (vp, scene) = front(win)?;
    let camera = vp.getattr("camera")?;
    let core = py.import("built_core")?;
    let shown = core.getattr("saved_view_shown_state")?;
    let before = shown.call1((&scene,))?;
    // The camera's kind; where it looks from stays.
    let cam = &data["camera"];
    let mut moved = false;
    for (field, value) in [("perspective", cam["perspective"].as_bool()), ("two_point", cam["two_point"].as_bool())] {
        if let Some(v) = value {
            if camera.hasattr(field)? && camera.getattr(field)?.is_truthy()? != v {
                camera.setattr(field, v)?;
                moved = true;
            }
        }
    }
    if let Some(fov) = cam["fov_deg"].as_f64().filter(|f| (1.0..=170.0).contains(f)) {
        if (camera.getattr("fov_deg")?.extract::<f64>()? - fov).abs() > 1e-9 {
            camera.setattr("fov_deg", fov)?;
            moved = true;
        }
    }
    if cam["lens"].is_object() && camera.hasattr("apply_lens")? {
        camera.call_method1("apply_lens", (to_py(py, &cam["lens"])?,))?;
    }
    // Tags: this model's keys for the template's hidden tags.
    let wanted: Vec<(&str, &str)> = data["hidden_tags"].as_array().into_iter().flatten().map(|t| (t["key"].as_str().unwrap_or(""), t["name"].as_str().unwrap_or(""))).collect();
    let mut found = vec![false; wanted.len()];
    let hidden = PyList::empty(py);
    for ly in scene.getattr("layers")?.try_iter()? {
        let ly = ly?;
        let (key, name) = (ly.getattr("key")?.str()?.to_string(), ly.getattr("name")?.str()?.to_string());
        let at = wanted.iter().position(|(k, _)| !k.is_empty() && *k == key).or_else(|| wanted.iter().position(|(_, n)| !n.is_empty() && *n == name));
        if let Some(i) = at {
            found[i] = true;
            hidden.append(ly.getattr("key")?)?;
        }
    }
    let mut missing: Vec<String> = Vec::new();
    let lost = found.iter().filter(|f| !**f).count();
    if lost > 0 {
        missing.push(if lost == 1 { "1 tag of the template is not in this model".into() } else { format!("{lost} tags of the template are not in this model") });
    }
    // The section plane: this model's own (by id, then by name), else the
    // template's, placed as one step.
    let section = &data["section"];
    let mut active = py.None().into_bound(py);
    if let Some(plane) = section["active"].as_object() {
        let (uid, name) = (plane.get("uid").and_then(Value::as_str).unwrap_or(""), plane.get("name").and_then(Value::as_str).unwrap_or(""));
        let planes: Vec<Bound<'_, PyAny>> = scene.getattr("section_planes")?.try_iter()?.collect::<PyResult<_>>()?;
        let by = |attr: &str, want: &str| -> PyResult<Option<Bound<'_, PyAny>>> {
            if want.is_empty() {
                return Ok(None);
            }
            for p in &planes {
                if p.getattr(attr)?.str()?.to_string() == want {
                    return Ok(Some(p.clone()));
                }
            }
            Ok(None)
        };
        let existing = match by("uid", uid)? {
            Some(p) => Some(p),
            None => by("name", name)?,
        };
        let plane = match existing {
            Some(p) => p,
            None => {
                let class = py.import("built_core.section")?.getattr("SectionPlane")?;
                let fresh = class.call_method1("from_dict", (to_py(py, &section["active"])?,))?;
                fresh.setattr("active", false)?;
                let step = py.import("built_core.history")?.call_method1("PlaceSectionPlaneCommand", (&fresh,))?;
                vp.getattr("history")?.call_method1("execute", (step,))?;
                fresh
            }
        };
        active = plane.getattr("uid")?;
    }
    let raw = PyDict::new(py);
    raw.set_item("name", "")?;
    raw.set_item("hidden_layers", hidden)?;
    raw.set_item("hidden_folders", to_py(py, &data["hidden_folders"])?)?;
    raw.set_item("style", to_py(py, &data["style"])?)?;
    let s = PyDict::new(py);
    s.set_item("active", active)?;
    s.set_item("planes_shown", section["planes_shown"].as_bool().unwrap_or(true))?;
    s.set_item("cuts_shown", section["cuts_shown"].as_bool().unwrap_or(true))?;
    raw.set_item("section", s)?;
    let view = py.import("built_core.saved_views")?.getattr("SavedView")?.call_method1("from_dict", (raw,))?;
    view.call_method1("apply_state", (&scene, win.getattr("set_active_style")?))?;
    if moved || shown.call1((&scene,))?.ne(before)? {
        let version: i64 = scene.getattr("version")?.extract()?;
        scene.setattr("version", version + 1)?;
        vp.call_method0("notify_scene_changed")?;
    }
    vp.call_method0("update")?;
    if let Some(display) = win.getattr_opt("_doc")?.filter(|d| !d.is_none()).and_then(|d| d.getattr("stand_in").ok()).filter(|d| !d.is_none()) {
        let _ = display.setattr("_ui_tabs", py.None());
        let _ = display.call_method0("update");
    }
    Ok(missing.join(" · "))
}

/// View › Save View as Template…, and the Library tab's own button: the
/// shared drawer on the view in front.
#[pyfunction]
pub(crate) fn save_view_template(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let data = capture_view(win)?;
    let style = data["style"]["name"].as_str().unwrap_or("");
    let name = if style.is_empty() { tr(py, "View template")? } else { format!("{} {}", style, tr(py, "view")?) };
    destinations::save_to_library(win, "scenes", &name, &data.to_string(), None, 1, "", None, None, None, None, Some(tr(py, "Save view as template")?))
}

/// The view in front as a template's payload (scripts, tests).
#[pyfunction]
fn view_template<'py>(win: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    to_py(win.py(), &capture_view(win)?)
}

/// Apply a template's payload (JSON) to the view in front.
#[pyfunction]
fn apply_view_template(win: &Bound<'_, PyAny>, data: &str) -> PyResult<String> {
    let data: Value = serde_json::from_str(data).map_err(|e| err(e.to_string()))?;
    apply_view(win, &data)
}

/// Apply from the Library, with what it did as a toast.
pub(crate) fn use_view(win: &Bound<'_, PyAny>, name: &str, data: &Value) -> PyResult<()> {
    let py = win.py();
    match apply_view(win, data) {
        Ok(missing) if missing.is_empty() => {
            win.call_method1("notify", ("success", trf(py, "{name} applied to this view", &[("name", s(py, name))])?))?;
        }
        Ok(missing) => {
            win.call_method1("notify", ("warning", format!("{} · {}", trf(py, "{name} applied to this view", &[("name", s(py, name))])?, missing)))?;
        }
        Err(e) => {
            win.call_method1("notify", ("error", format!("{}: {}", tr(py, "Could not apply the view template")?, e.value(py))))?;
        }
    }
    Ok(())
}

// ---- title blocks ----------------------------------------------------------------------------

/// The sheet workspace in front, when a sheet is open.
fn workspace<'py>(win: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyAny>>> {
    let py = win.py();
    let Some(view) = win.getattr_opt("view")?.filter(|v| !v.is_none()) else { return Ok(None) };
    let ws = py.import("built_render.sheet_workspace")?.call_method1("workspace_of", (view,))?;
    Ok((!ws.is_none()).then_some(ws))
}

/// What a title block is, without its place on a sheet.
const PLACE_FIELDS: [&str; 6] = ["x_mm", "y_mm", "z", "locked", "group_id", "uid"];

/// The title block of the sheet in front (the selected one first), as a
/// Library payload; None when no sheet is open or it has none.
pub(crate) fn capture_block(win: &Bound<'_, PyAny>) -> PyResult<Option<Value>> {
    let Some(ws) = workspace(win)? else { return Ok(None) };
    let block = ws.call_method0("titleblock")?;
    if block.is_none() {
        return Ok(None);
    }
    let mut fields = serde_json::Map::new();
    for field in block.getattr("_record_fields")?.try_iter()? {
        let field: String = field?.extract()?;
        if PLACE_FIELDS.contains(&field.as_str()) {
            continue;
        }
        if let Ok(v) = to_value(&block.getattr(field.as_str())?) {
            fields.insert(field, v);
        }
    }
    Ok(Some(json!({"format": BLOCK_FORMAT, "v": 1, "fields": fields})))
}

/// What a title block holds, for the detail pane.
pub(crate) fn block_rows(data: &Value) -> Vec<(String, String, String)> {
    let f = &data["fields"];
    let rows: Vec<String> = f["campos"].as_array().into_iter().flatten().filter_map(|r| r[0].as_str().map(str::to_owned)).filter(|r| !r.is_empty()).collect();
    let mut out = vec![(
        "Size".to_owned(),
        format!("{:.0} × {:.0} mm", f["w_mm"].as_f64().unwrap_or(0.0), f["h_mm"].as_f64().unwrap_or(0.0)),
        String::new(),
    )];
    let columns = f["columns"].as_f64().unwrap_or(1.0) as i64;
    out.push(("Rows".to_owned(), format!("{} · {}", rows.len(), if columns == 2 { "two columns" } else { "one column" }), String::new()));
    if !rows.is_empty() {
        let shown: Vec<&str> = rows.iter().take(6).map(String::as_str).collect();
        out.push(("Fields".to_owned(), format!("{}{}", shown.join(", "), if rows.len() > 6 { "…" } else { "" }), String::new()));
    }
    out
}

/// Put a Library title block on the sheet in front: its fields onto the
/// sheet's title block, or a new one when the sheet has none. One step.
pub(crate) fn apply_block(win: &Bound<'_, PyAny>, data: &Value) -> PyResult<bool> {
    let py = win.py();
    if data["format"] != BLOCK_FORMAT || !data["fields"].is_object() {
        return Err(err("Not a title block"));
    }
    let Some(ws) = workspace(win)? else { return Ok(false) };
    ws.call_method1("use_titleblock", (to_py(py, &data["fields"])?,))?;
    Ok(true)
}

/// Sheet › title block › Save to Library…: the shared drawer.
#[pyfunction]
pub(crate) fn save_title_block(win: &Bound<'_, PyAny>) -> PyResult<bool> {
    let py = win.py();
    let Some(data) = capture_block(win)? else {
        win.call_method1("notify", ("info", tr(py, "Open a sheet with a title block to save it to the Library")?))?;
        return Ok(false);
    };
    // Named after its first row's value (the project, usually), else plainly.
    let first = data["fields"]["campos"][0][1].as_str().unwrap_or("").trim().to_owned();
    let name = if first.is_empty() { tr(py, "Title block")? } else { format!("{first} · {}", tr(py, "title block")?) };
    destinations::save_to_library(win, "title_blocks", &name, &data.to_string(), None, 1, "", None, None, None, None, Some(tr(py, "Save title block to Library")?))?;
    Ok(true)
}

/// The front sheet's title block as a payload, or None (scripts, tests).
#[pyfunction]
fn title_block<'py>(win: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = win.py();
    match capture_block(win)? {
        Some(v) => to_py(py, &v),
        None => Ok(py.None().into_bound(py)),
    }
}

/// Apply a title block payload (JSON) to the sheet in front.
#[pyfunction]
fn apply_title_block(win: &Bound<'_, PyAny>, data: &str) -> PyResult<bool> {
    let data: Value = serde_json::from_str(data).map_err(|e| err(e.to_string()))?;
    apply_block(win, &data)
}

/// Apply from the Library, with what it did as a toast.
pub(crate) fn use_block(win: &Bound<'_, PyAny>, name: &str, data: &Value) -> PyResult<()> {
    let py = win.py();
    match apply_block(win, data) {
        Ok(true) => {
            win.call_method1("notify", ("success", trf(py, "{name} is this sheet's title block", &[("name", s(py, name))])?))?;
        }
        Ok(false) => {
            win.call_method1("notify", ("info", tr(py, "Open a sheet first: a title block goes on the sheet in front")?))?;
        }
        Err(e) => {
            win.call_method1("notify", ("error", format!("{}: {}", tr(py, "Could not use the title block")?, e.value(py))))?;
        }
    }
    Ok(())
}

// ---- the Library tabs ------------------------------------------------------------------------

/// The rows of a saved kind with each item's facts for the detail pane.
fn rows_with<'py>(py: Python<'py>, ctx: &Bound<'py, PyAny>, kind: &str, facts: fn(&Value) -> Vec<(String, String, String)>, picture: fn(&Value) -> Value) -> PyResult<Bound<'py, PyAny>> {
    let scene = ctx.getattr_opt("scene")?.filter(|s| !s.is_none() && s.hasattr("_root").unwrap_or(false));
    let entries = destinations::entries(py, scene.as_ref(), &[kind])?;
    let mut items = destinations::items(py, scene.as_ref(), &[kind])?;
    for row in &mut items {
        let Some(entry) = entries.iter().find(|l| row["lib"]["dest"] == l.dest.key.as_str() && row["lib"]["id"] == l.entry.id.as_str()) else { continue };
        let mut details: Vec<Value> = row["defaults"].as_array().cloned().unwrap_or_default();
        details.extend(facts(&entry.entry.data).into_iter().map(|r| json!(r)));
        row["defaults"] = json!(details);
        // Its tile: drawn from the item itself, cached like a section's.
        row["thumb"] = json!(crate::sketch_thumbs::key(&picture(&entry.entry.data)));
    }
    py.import("json")?.call_method1("loads", (Value::Array(items).to_string(),))
}

/// Lines as a drawing the thumbnails render (sketch_thumbs.rs).
fn line_drawing(lines: &[[f64; 4]]) -> Value {
    let curves: Vec<Value> = lines
        .iter()
        .filter(|l| (l[0] - l[2]).abs() + (l[1] - l[3]).abs() > 1e-9)
        .enumerate()
        .map(|(i, l)| json!({"id": i + 1, "kind": "line", "a": [l[0], l[1]], "b": [l[2], l[3]]}))
        .collect();
    json!({"use": "section", "curves": curves})
}

/// A title block's tile: its frame, its rows and columns, the label column
/// of each (the proportions the sheet draws it with).
pub(crate) fn block_picture(data: &Value) -> Value {
    let f = &data["fields"];
    let (w, h) = (f["w_mm"].as_f64().unwrap_or(180.0).max(1.0), f["h_mm"].as_f64().unwrap_or(33.0).max(1.0));
    let rows = f["campos"].as_array().map_or(0, Vec::len).max(1);
    let cols = (f["columns"].as_f64().unwrap_or(1.0) as usize).clamp(1, rows);
    let per = rows.div_ceil(cols);
    let col_w = w / cols as f64;
    let label_w = 28f64.min(col_w * 0.3);
    let mut lines = vec![[0.0, 0.0, w, 0.0], [w, 0.0, w, h], [w, h, 0.0, h], [0.0, h, 0.0, 0.0]];
    for c in 0..cols {
        let x = c as f64 * col_w;
        if c > 0 {
            lines.push([x, 0.0, x, h]);
        }
        lines.push([x + label_w, 0.0, x + label_w, h]);
    }
    for r in 1..per {
        let y = h * r as f64 / per as f64;
        lines.push([0.0, y, w, y]);
    }
    line_drawing(&lines)
}

/// A view template's tile: a box as its camera kind shows one (parallel,
/// perspective, two-point), cut across when the template cuts the model.
pub(crate) fn view_picture(data: &Value) -> Value {
    let perspective = data["camera"]["perspective"].as_bool().unwrap_or(true);
    let two_point = data["camera"]["two_point"].as_bool().unwrap_or(false);
    let (yaw, pitch) = (35f64.to_radians(), if two_point { 0.0 } else { 26f64.to_radians() });
    let project = |p: [f64; 3]| -> [f64; 2] {
        // Turned about the vertical, then tipped towards the eye.
        let (x, y) = (p[0] * yaw.cos() - p[1] * yaw.sin(), p[0] * yaw.sin() + p[1] * yaw.cos());
        let (up, depth) = (p[2] * pitch.cos() + y * pitch.sin(), y * pitch.cos() - p[2] * pitch.sin());
        let k = if perspective { 3.4 / (3.4 + depth) } else { 1.0 };
        [x * k, (up + if two_point { 0.35 * (1.0 - k) } else { 0.0 }) * if two_point { 1.0 } else { k }]
    };
    let (a, b, c) = (1.3, 0.9, 0.8);
    let corner = |i: usize, z: f64| [if i == 1 || i == 2 { a } else { -a } * 0.5, if i >= 2 { b } else { -b } * 0.5, z];
    let mut lines = Vec::new();
    let mut ring = |z: f64| {
        for i in 0..4 {
            let (p, q) = (project(corner(i, z)), project(corner((i + 1) % 4, z)));
            lines.push([p[0], p[1], q[0], q[1]]);
        }
    };
    ring(-c * 0.5);
    ring(c * 0.5);
    if data["section"]["active"].is_object() {
        ring(c * 0.12);
    }
    for i in 0..4 {
        let (p, q) = (project(corner(i, -c * 0.5)), project(corner(i, c * 0.5)));
        lines.push([p[0], p[1], q[0], q[1]]);
    }
    line_drawing(&lines)
}

/// View setups to apply to the view in front.
#[pyclass(module = "built_render.wgpu_library", extends = LibraryProvider, dict)]
pub struct ViewTemplatesProvider;

#[pymethods]
impl ViewTemplatesProvider {
    #[new]
    pub(crate) fn new() -> PyClassInitializer<Self> {
        PyClassInitializer::from(LibraryProvider).add_subclass(ViewTemplatesProvider)
    }

    #[classattr]
    fn id() -> &'static str {
        "scenes"
    }

    #[classattr]
    fn label() -> &'static str {
        "View templates"
    }

    fn items<'py>(&self, py: Python<'py>, ctx: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        rows_with(py, ctx, "scenes", view_rows, view_picture)
    }

    /// The header's Save current view….
    fn type_action(&self, ctx: &Bound<'_, PyAny>, action: &str) -> PyResult<()> {
        if action == "save" {
            let win = ctx.getattr("win")?;
            cs(ctx.py())?.call_method1("close", (&win,))?;
            save_view_template(&win)?;
        }
        Ok(())
    }
}

/// Title blocks to use on the sheet in front.
#[pyclass(module = "built_render.wgpu_library", extends = LibraryProvider, dict)]
pub struct TitleBlocksProvider;

#[pymethods]
impl TitleBlocksProvider {
    #[new]
    pub(crate) fn new() -> PyClassInitializer<Self> {
        PyClassInitializer::from(LibraryProvider).add_subclass(TitleBlocksProvider)
    }

    #[classattr]
    fn id() -> &'static str {
        "title_blocks"
    }

    #[classattr]
    fn label() -> &'static str {
        "Title blocks"
    }

    fn items<'py>(&self, py: Python<'py>, ctx: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        rows_with(py, ctx, "title_blocks", block_rows, block_picture)
    }

    /// The header's Save title block….
    fn type_action(&self, ctx: &Bound<'_, PyAny>, action: &str) -> PyResult<()> {
        if action == "save" {
            let win = ctx.getattr("win")?;
            cs(ctx.py())?.call_method1("close", (&win,))?;
            save_title_block(&win)?;
        }
        Ok(())
    }
}

/// A sketch's Library thumbnail: its key and how many thumbnails have
/// been rendered so far (the rest came from the cache). Scripts and tests.
#[pyfunction]
fn sketch_thumbnail(sketch: &str, px: usize) -> PyResult<(Option<String>, usize, u64)> {
    let sketch: Value = serde_json::from_str(sketch).map_err(|e| err(e.to_string()))?;
    let key = crate::sketch_thumbs::key(&sketch);
    let inked = key.as_deref().and_then(|k| crate::sketch_thumbs::pixels(k, px)).map_or(0, |p| p.chunks(4).filter(|c| c[3] > 0).count());
    Ok((key, inked, crate::sketch_thumbs::rendered()))
}

pub(super) fn register(sub: &Bound<'_, PyModule>) -> PyResult<()> {
    sub.add_function(wrap_pyfunction!(sketch_thumbnail, sub)?)?;
    sub.add_class::<ViewTemplatesProvider>()?;
    sub.add_class::<TitleBlocksProvider>()?;
    sub.add_function(wrap_pyfunction!(save_view_template, sub)?)?;
    sub.add_function(wrap_pyfunction!(view_template, sub)?)?;
    sub.add_function(wrap_pyfunction!(apply_view_template, sub)?)?;
    sub.add_function(wrap_pyfunction!(save_title_block, sub)?)?;
    sub.add_function(wrap_pyfunction!(title_block, sub)?)?;
    sub.add_function(wrap_pyfunction!(apply_title_block, sub)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_view_template_reads_in_words() {
        let data = json!({"format": VIEW_FORMAT, "camera": {"perspective": false}, "style": {"name": "Hidden line"},
            "hidden_tags": [{"key": "a", "name": "Furniture"}, {"key": "b", "name": ""}], "hidden_folders": [],
            "section": {"active": {"name": "A-A"}, "planes_shown": false, "cuts_shown": true}});
        let rows = view_rows(&data);
        assert_eq!(rows[0].1, "Parallel");
        assert_eq!(rows[1].1, "Hidden line");
        assert_eq!(rows[2].1, "Hidden: Furniture, b");
        assert_eq!(rows[3].1, "Cut by A-A · planes hidden");
        let plain = json!({"camera": {"perspective": true, "fov_deg": 35.0}, "section": {"active": null}});
        let rows = view_rows(&plain);
        assert_eq!(rows[0].1, "Perspective · 35°");
        assert_eq!(rows[2].1, "Every tag shown");
        assert_eq!(rows[3].1, "No active cut · planes shown");
    }

    #[test]
    fn a_title_block_reads_in_words() {
        let data = json!({"format": BLOCK_FORMAT, "fields": {"w_mm": 180.0, "h_mm": 33.0, "columns": 2,
            "campos": [["PROJECT", "Garage"], ["SHEET", "L-01"], ["", "x"]]}});
        let rows = block_rows(&data);
        assert_eq!(rows[0].1, "180 × 33 mm");
        assert_eq!(rows[1].1, "2 · two columns");
        assert_eq!(rows[2].1, "PROJECT, SHEET");
    }
}
