//! The Sketch workspace's host (ui/sketch.rs draws it): how a sketch is
//! entered and what Done, Cancel and Save to Library do in the document.
//!
//! Entered from the window command `sketch:<how>[:<arg>]`:
//! - `new` / `new:section`: a blank profile or section, no plane;
//! - `face`: on the selected face's plane, its outline as the first curves
//!   (a profile from the face), the model around it as context;
//! - `import:<path>`: a DXF or SVG file (DWG through the LibreDWG bridge)
//!   into a new sketch with the clean-up list;
//! - `open:<path>`: a Library `.builtprofile`;
//! - `edit:<uid>`: a sketch row of this model.
//!
//! Done runs `sketch_commit`: the row and every user of it in one History
//! step. Save to Library writes a `.builtprofile` into the user's Library
//! (`<data>/Built/profiles/<Profiles|Sections>/`).
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList},
};
use std::path::PathBuf;

type Obj<'p> = Bound<'p, PyAny>;
const SEP: char = '\u{1f}';

fn tr(py: Python<'_>, text: &str) -> String {
    crate::settings_rows::tr(py, text).unwrap_or_else(|_| text.to_owned())
}

fn view_of<'p>(win: &Obj<'p>) -> PyResult<Obj<'p>> {
    let doc = win.getattr("_doc")?;
    doc.getattr("stand_in")
}

fn renderer<'p>(win: &Obj<'p>) -> PyResult<Option<Obj<'p>>> {
    // Headless windows (tests, scripts) have no document view.
    let Some(doc) = win.getattr_opt("_doc")?.filter(|d| !d.is_none()) else { return Ok(None) };
    let Some(view) = doc.getattr_opt("stand_in")?.filter(|v| !v.is_none()) else { return Ok(None) };
    Ok(view.getattr_opt("renderer")?.filter(|r| !r.is_none() && r.hasattr("ui_sketch_enter").unwrap_or(false)))
}

fn protocol(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("built_core.command_protocol")
}

fn json<'p>(py: Python<'p>, v: &serde_json::Value) -> PyResult<Obj<'p>> {
    py.import("json")?.call_method1("loads", (v.to_string(),))
}
fn to_value(v: &Obj<'_>) -> PyResult<serde_json::Value> {
    let raw: String = v.py().import("json")?.call_method1("dumps", (v,))?.extract()?;
    serde_json::from_str(&raw).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// The user's Library folder for drawn profiles and sections: the one the
/// Profiles tab lists (`ProfilesProvider.saved_directory`); sections are
/// told apart by their `use`.
pub fn library_dir() -> Option<PathBuf> {
    let profile = std::env::var_os("BUILT_SETTINGS_DIR").map(PathBuf::from);
    Some(crate::platform_paths::local_data_directory(profile)?.join("Built").join("profiles"))
}

#[pyfunction]
fn profiles_dir() -> PyResult<String> {
    library_dir().map(|p| p.to_string_lossy().into_owned()).ok_or_else(|| PyValueError::new_err("Library directory unavailable"))
}

/// Write a `.builtprofile` (the JSON Save to Library sends) into the
/// Library folder; returns its path. A name already taken gets a number.
#[pyfunction]
pub fn save_profile(text: &str) -> PyResult<String> {
    let file = built_draft2d::file::ProfileFile::parse(text).map_err(PyValueError::new_err)?;
    let dir = library_dir().ok_or_else(|| PyValueError::new_err("Library directory unavailable"))?;
    std::fs::create_dir_all(&dir).map_err(|e| PyValueError::new_err(e.to_string()))?;
    let base = built_draft2d::file::file_name(&file.name);
    let mut path = dir.join(&base);
    let mut n = 2;
    while path.exists() && std::fs::read_to_string(&path).ok().and_then(|t| built_draft2d::file::ProfileFile::parse(&t).ok()).is_some_and(|old| old.name != file.name) {
        path = dir.join(base.replace(".builtprofile", &format!(" ({n}).builtprofile")));
        n += 1;
    }
    std::fs::write(&path, file.to_string()).map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(path.to_string_lossy().into_owned())
}

/// The breadcrumb: the document, the groups being edited.
fn crumbs(win: &Obj<'_>) -> PyResult<Vec<String>> {
    let py = win.py();
    let mut out = Vec::new();
    let path = win.getattr("_current_path")?;
    let name = if path.is_none() {
        tr(py, "Untitled")
    } else {
        py.import("pathlib")?.getattr("Path")?.call1((path,))?.getattr("stem")?.extract()?
    };
    out.push(name);
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let mut chain = Vec::new();
    let mut g = scene.getattr("edit_group")?;
    while !g.is_none() {
        chain.push(g.getattr("name")?.extract::<Option<String>>()?.unwrap_or_default());
        g = g.getattr_opt("parent")?.unwrap_or_else(|| py.None().into_bound(py));
        if chain.len() > 16 {
            break;
        }
    }
    chain.reverse();
    out.extend(chain.into_iter().filter(|n| !n.is_empty()));
    Ok(out)
}

/// Which way the 3D view looks (eye − target), for the inset.
fn look(win: &Obj<'_>) -> Option<[f64; 3]> {
    let vp = win.getattr("viewport").ok()?;
    let cam = vp.getattr("camera").ok()?;
    let v = |o: Obj<'_>| -> Option<[f64; 3]> {
        if let Ok(t) = o.call_method0("position_tuple") {
            return t.extract().ok();
        }
        Some([o.getattr("x").ok()?.extract().ok()?, o.getattr("y").ok()?.extract().ok()?, o.getattr("z").ok()?.extract().ok()?])
    };
    let eye = v(cam.getattr("eye").or_else(|_| cam.getattr("position")).ok()?)?;
    let target = v(cam.getattr("target").or_else(|_| cam.getattr("center")).ok()?)?;
    Some([eye[0] - target[0], eye[1] - target[1], eye[2] - target[2]])
}

/// The context around a plane, from the model (ghost, behind, inset).
fn context(py: Python<'_>, scene: &Obj<'_>, frame: &serde_json::Value) -> PyResult<serde_json::Value> {
    let get = |k: &str| -> Option<[f64; 3]> {
        let a: Vec<f64> = frame[k].as_array()?.iter().filter_map(|x| x.as_f64()).collect();
        (a.len() == 3).then(|| [a[0], a[1], a[2]])
    };
    let (Some(o), Some(x), Some(y)) = (get("origin"), get("x"), get("y")) else { return Ok(serde_json::Value::Null) };
    let c = protocol(py)?.call_method1("sketch_context", (scene, o, x, y))?;
    to_value(&c)
}

fn enter_with(win: &Obj<'_>, mut spec: serde_json::Value) -> PyResult<bool> {
    let py = win.py();
    let Some(r) = renderer(win)? else { return Ok(false) };
    if spec["crumbs"].is_null() {
        spec["crumbs"] = serde_json::json!(crumbs(win)?);
    }
    if let Some(l) = look(win) {
        spec["look"] = serde_json::json!(l);
    }
    let scene = win.getattr("viewport")?.getattr("scene")?;
    if !spec["frame"].is_null() && spec["context"].is_null() {
        spec["context"] = context(py, &scene, &spec["frame"])?;
    }
    // Leave group editing tools alone; drop the active tool's state.
    if let Ok(tools) = view_of(win)?.getattr("tools") {
        let _ = tools.call_method0("cancel");
    }
    if let Err(e) = r.call_method1("ui_sketch_enter", (spec.to_string(),)) {
        win.call_method1("notify", ("error", format!("{}: {}", tr(py, "Could not open the sketch"), e.value(py))))?;
        return Ok(false);
    }
    view_of(win)?.call_method0("update")?;
    Ok(true)
}

/// The plane frame a sketch row is anchored to, when it names a face.
fn row_frame(scene: &Obj<'_>, row: &serde_json::Value) -> PyResult<serde_json::Value> {
    let Some(id) = row["plane"]["ref"]["id"].as_str() else { return Ok(serde_json::Value::Null) };
    let path: Vec<String> = row["plane"]["ref"]["path"].as_array().into_iter().flatten().filter_map(|p| p.as_str().map(str::to_owned)).collect();
    let p = protocol(scene.py())?.call_method1("sketch_plane", (scene, id, path))?;
    if p.is_none() {
        return Ok(serde_json::Value::Null);
    }
    to_value(&p)
}

/// Users of a sketch row, as the record says them ("2 elements").
fn users_text(py: Python<'_>, rows: &[serde_json::Value], uid: &str) -> String {
    let n = rows
        .iter()
        .filter(|r| r["inputs"].as_array().is_some_and(|i| i.iter().any(|x| x["row"] == uid)))
        .count();
    match n {
        0 => String::new(),
        1 => tr(py, "1 element"),
        n => format!("{n} {}", tr(py, "elements")),
    }
}

/// Enter the workspace: `how` and its argument (module docs).
#[pyfunction]
pub fn enter(win: &Obj<'_>, how: &str, arg: &str) -> PyResult<bool> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    match how {
        // Scripts and other surfaces leave it as the top bar's buttons do.
        "done" | "cancel" => command(win, how),
        "new" => enter_with(win, serde_json::json!({"use": if arg == "section" { "section" } else { "profile" }})),
        "face" | "section" => {
            // `arg`: empty for the selected face, or `<uid>/<uid>…|f:<id>`
            // (scripts and a row's plane reference).
            let (path, face): (Option<Vec<String>>, String) = match arg.split_once('|') {
                Some((p, f)) => (Some(p.split('/').filter(|s| !s.is_empty()).map(str::to_owned).collect()), f.to_owned()),
                None => {
                    let sel = protocol(py)?.call_method1("selection", (&scene,))?;
                    let faces = to_value(&sel)?["ids"]["face"].clone();
                    let Some(face) = faces.as_array().and_then(|f| f.first()).and_then(|f| f.as_str()).map(str::to_owned) else {
                        win.call_method1("notify", ("info", tr(py, "Select a face to sketch on")))?;
                        return Ok(false);
                    };
                    (None, face)
                }
            };
            let plane = protocol(py)?.call_method1("sketch_plane", (&scene, &face, path))?;
            if plane.is_none() {
                win.call_method1("notify", ("info", tr(py, "That face is not flat enough to sketch on")))?;
                return Ok(false);
            }
            let plane = to_value(&plane)?;
            // The face's outline is the first drawing: a profile from the face.
            let section = how == "section";
            let mut sketch = built_draft2d::Sketch::new(if section { built_draft2d::Use::Section } else { built_draft2d::Use::Profile });
            // A profile starts from the face's outline; a section is drawn on it.
            for ring in plane["loops"].as_array().into_iter().flatten().filter(|_| !section) {
                let pts: Vec<glam::DVec2> = ring.as_array().into_iter().flatten().filter_map(|p| Some(glam::DVec2::new(p[0].as_f64()?, p[1].as_f64()?))).collect();
                for i in 0..pts.len() {
                    let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                    if (b - a).length() > 1e-6 {
                        sketch.add(built_draft2d::Shape::Line { a, b });
                    }
                }
            }
            sketch.plane = serde_json::json!({"ref": plane["ref"], "frame": "face", "offset": 0});
            if sketch.usage == built_draft2d::Use::Section {
                sketch.extrude = Some(Default::default());
            }
            enter_with(
                win,
                serde_json::json!({
                    "sketch": sketch.to_json(), "frame": plane,
                    "plane_label": tr(py, "Picked face"), "inset_label": tr(py, "Face · sketch plane"),
                }),
            )
        }
        "import" => {
            let path = PathBuf::from(arg);
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            let shown = path.file_name().and_then(|n| n.to_str()).unwrap_or(arg).to_owned();
            let mut temp = None;
            let readable = if ext == "dwg" {
                // DWG goes through the existing LibreDWG bridge into DXF.
                let bridge = py.import("built_core.formats.dwg_bridge")?;
                if !bridge.call_method0("have_dwg_support")?.is_truthy()? {
                    win.call_method1("notify", ("info", tr(py, "DWG needs the LibreDWG converter: install it, or save the drawing as DXF")))?;
                    return Ok(false);
                }
                match py.import("built_core")?.call_method1("dwg_convert", (arg,)) {
                    Ok(dxf) => {
                        let text = dxf.str()?.extract::<String>()?;
                        temp = Some(dxf);
                        text
                    }
                    Err(e) => {
                        win.call_method1("notify", ("error", format!("{}: {}", tr(py, "Could not read the DWG file"), e.value(py))))?;
                        return Ok(false);
                    }
                }
            } else {
                arg.to_owned()
            };
            let mut crumbs = crumbs(win)?;
            crumbs.push(tr(py, "Import"));
            let done = enter_with(win, serde_json::json!({"import": readable, "import_name": shown, "crumbs": crumbs}));
            if let Some(t) = temp {
                // The workspace read it on entering.
                let _ = py.import("built_core.formats.dwg_bridge")?.call_method1("discard_temp_dxf", (t,));
            }
            done
        }
        "open" => {
            let text = std::fs::read_to_string(arg).map_err(|e| PyValueError::new_err(e.to_string()))?;
            let file = built_draft2d::file::ProfileFile::parse(&text).map_err(PyValueError::new_err)?;
            let Some(sketch) = file.sketch else {
                win.call_method1("notify", ("info", tr(py, "Catalogue sizes open from their family table, not as a drawing")))?;
                return Ok(false);
            };
            let section = file.usage == built_draft2d::Use::Section;
            let crumbs = vec![tr(py, "Library"), tr(py, if section { "Sections" } else { "Profiles" })];
            enter_with(win, serde_json::json!({"sketch": sketch.to_json(), "crumbs": crumbs, "library_path": arg}))
        }
        "edit" => {
            let rows = to_value(&protocol(py)?.call_method1("live_rows", (&scene,))?)?;
            let rows = rows.as_array().cloned().unwrap_or_default();
            let Some(row) = rows.iter().find(|r| r["uid"] == arg && r["kind"] == "sketch").cloned() else {
                win.call_method1("notify", ("info", tr(py, "That sketch is no longer in this model")))?;
                return Ok(false);
            };
            let mut body = row.clone();
            for k in ["uid", "owner", "kind", "v", "state", "inputs", "slots"] {
                body.as_object_mut().map(|o| o.remove(k));
            }
            let frame = row_frame(&scene, &row)?;
            enter_with(
                win,
                serde_json::json!({
                    "sketch": body, "uid": arg, "frame": frame, "users": users_text(py, &rows, arg),
                    "plane_label": if frame.is_null() { "" } else { "Face" },
                }),
            )
        }
        _ => Ok(false),
    }
}

/// `sketch:<verb>␟<payload>` from the chrome.
#[pyfunction]
pub fn action(win: &Obj<'_>, arg: &str) -> PyResult<()> {
    let py = win.py();
    let (verb, payload) = arg.split_once(SEP).unwrap_or((arg, ""));
    let vp = win.getattr("viewport")?;
    match verb {
        "done" => {
            let row: serde_json::Value = serde_json::from_str(payload).map_err(|e| PyValueError::new_err(e.to_string()))?;
            let kwargs = PyDict::new(py);
            kwargs.set_item("row", json(py, &row)?)?;
            let history = vp.getattr("history")?;
            let reply = protocol(py)?.getattr("run")?.call((&history, "sketch_commit"), Some(&kwargs));
            match reply {
                Ok(reply) => {
                    let uid: String = reply.get_item("uid")?.extract()?;
                    let rows = to_value(&protocol(py)?.call_method1("live_rows", (vp.getattr("scene")?,))?)?;
                    let rows = rows.as_array().cloned().unwrap_or_default();
                    let state = rows.iter().find(|r| r["uid"] == uid.as_str()).and_then(|r| r["state"].as_str()).unwrap_or("").to_owned();
                    let users = users_text(py, &rows, &uid);
                    let name = row["name"].as_str().filter(|n| !n.is_empty()).map(str::to_owned).unwrap_or_else(|| tr(py, "Sketch"));
                    let text = match (state.as_str(), users.is_empty()) {
                        ("profile_open", _) => format!("{name} {}", tr(py, "saved, not closed yet: its users keep their last profile")),
                        (_, true) => format!("{name} {}", tr(py, "saved in this model")),
                        (_, false) => format!("{name} {} · {users} {}", tr(py, "saved"), tr(py, "updated")),
                    };
                    let undo = PyList::empty(py);
                    let w = win.clone().unbind();
                    let again = uid.clone();
                    let edit = pyo3::types::PyCFunction::new_closure(py, None, None, move |args, _| -> PyResult<()> {
                        let py = args.py();
                        enter(w.bind(py), "edit", &again).map(drop)
                    })?;
                    undo.append((tr(py, "Edit again"), edit))?;
                    win.call_method1("notify", ("success", text, undo))?;
                    vp.call_method0("update")?;
                }
                Err(e) => {
                    win.call_method1("notify", ("error", format!("{}: {}", tr(py, "The sketch was not saved"), e.value(py))))?;
                }
            }
        }
        "cancel" => {
            let w = win.clone().unbind();
            let back = pyo3::types::PyCFunction::new_closure(py, None, None, move |args, _| -> PyResult<()> {
                let py = args.py();
                let win = w.bind(py);
                if let Some(r) = renderer(win)? {
                    let spec = r.call_method0("ui_sketch_discarded")?;
                    if !spec.is_none() {
                        let spec: serde_json::Value = serde_json::from_str(&spec.extract::<String>()?).map_err(|e| PyValueError::new_err(e.to_string()))?;
                        enter_with(win, spec)?;
                    }
                }
                Ok(())
            })?;
            let actions = PyList::empty(py);
            actions.append((tr(py, "Undo"), back))?;
            win.call_method1("notify", ("info", tr(py, "Sketch discarded"), actions))?;
        }
        "save" => {
            let file = built_draft2d::file::ProfileFile::parse(payload).map_err(PyValueError::new_err)?;
            let section = file.usage == built_draft2d::Use::Section;
            match save_profile(payload) {
                Ok(path) => {
                    let text = format!("{} {} › {}", file.name, tr(py, "saved to Library"), tr(py, if section { "Sections" } else { "Profiles" }));
                    if let Some(r) = renderer(win)? {
                        r.call_method1("ui_sketch_message", (&text,))?;
                    }
                    win.setattr("_last_profile_path", path)?;
                }
                Err(e) => {
                    win.call_method1("notify", ("error", format!("{}: {}", tr(py, "Could not save to the Library"), e.value(py))))?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// A window command while the sketch is up (⌘Z from the menu bar).
pub fn command(win: &Obj<'_>, cmd: &str) -> PyResult<bool> {
    let Some(r) = renderer(win)? else { return Ok(false) };
    // Only a real renderer answers with a bool (stand-ins answer anything).
    if !r.call_method0("ui_sketch_shown")?.extract::<bool>().unwrap_or(false) {
        return Ok(false);
    }
    // Model tools wait until the sketch is left.
    let took = cmd == "tool" || r.call_method1("ui_sketch_command", (cmd,))?.extract::<bool>().unwrap_or(false);
    view_of(win)?.call_method0("update")?;
    Ok(took)
}

/// The workspace's own state for tests and scripts: shown, the row.
#[pyfunction]
pub fn state<'p>(win: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = win.py();
    let Some(r) = renderer(win)? else { return Ok(py.None().into_bound(py)) };
    let d = PyDict::new(py);
    d.set_item("shown", r.call_method0("ui_sketch_shown")?)?;
    let row = r.call_method0("ui_sketch_row")?;
    d.set_item("row", if row.is_none() { row } else { py.import("json")?.call_method1("loads", (row,))? })?;
    Ok(d.into_any())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "sketch_workspace")?;
    sub.add_function(wrap_pyfunction!(enter, &sub)?)?;
    sub.add_function(wrap_pyfunction!(action, &sub)?)?;
    sub.add_function(wrap_pyfunction!(state, &sub)?)?;
    sub.add_function(wrap_pyfunction!(profiles_dir, &sub)?)?;
    sub.add_function(wrap_pyfunction!(save_profile, &sub)?)?;
    m.py().import("sys")?.getattr("modules")?.set_item("built_render.sketch_workspace", &sub)?;
    m.add_submodule(&sub)
}
