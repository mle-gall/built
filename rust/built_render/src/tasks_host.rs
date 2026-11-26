// SPDX-License-Identifier: GPL-3.0-or-later
//! Agent tasks on the window (`tasks.rs` holds the model): the document's
//! BCF topics in `plugin_data["bcf"]`, the attribution of history steps to
//! the task that made them, Revert as one undoable step, the Tasks
//! destination's data and verbs (`task:` actions), the viewport preview of
//! a task's changes and its 3D pin.
//!
//! Python callers: the built-in assistant (plugins/_assistant_panel.py) and
//! the MCP bridge (plugins/ai_bridge.py) create and report tasks through
//! the `task_*` functions; their edits run between `task_edit_begin` and
//! `task_edit_end`.
use crate::tasks::{self, ChangeKind, Changed, State, Task, Tasks, Viewpoint};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyList, PyTuple};
use serde_json::{json, Value};

type Obj<'py> = Bound<'py, PyAny>;

/// Where the topics live in the document.
pub const KEY: &str = "bcf";
pub const ASSISTANT: &str = "Built-in assistant";
pub const USER: &str = "You";
const SEP: char = '\x1f';

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}
fn err(e: impl Into<String>) -> PyErr {
    PyValueError::new_err(e.into())
}
fn opt<'py>(o: &Obj<'py>, name: &str) -> Option<Obj<'py>> {
    o.getattr(name).ok().filter(|v| !v.is_none())
}
fn viewport<'py>(win: &Obj<'py>) -> PyResult<Obj<'py>> {
    win.getattr("viewport")
}
fn scene_of<'py>(win: &Obj<'py>) -> PyResult<Obj<'py>> {
    viewport(win)?.getattr("scene")
}
fn json_text(v: &Obj<'_>) -> PyResult<String> {
    v.py().import("json")?.call_method1("dumps", (v,))?.extract()
}
fn py_json<'py>(py: Python<'py>, v: &Value) -> PyResult<Obj<'py>> {
    py.import("json")?.call_method1("loads", (v.to_string(),))
}

// ------------------------------------------------------------- storage

/// The document's tasks. A copy rides on the scene too: an agent's
/// snapshot step that restores `plugin_data` on undo must not take the
/// task records back with it.
pub fn load(scene: &Obj<'_>) -> PyResult<Tasks> {
    if let Some(text) = opt(scene, "_tasks_json") {
        let text: String = text.extract()?;
        let value: Value = serde_json::from_str(&text).map_err(|e| err(e.to_string()))?;
        return Ok(Tasks::from_bcf(&value));
    }
    let data = scene.getattr("plugin_data")?;
    let stored = data.call_method1("get", (KEY,))?;
    if stored.is_none() {
        return Ok(Tasks::default());
    }
    let value: Value = serde_json::from_str(&json_text(&stored)?).map_err(|e| err(e.to_string()))?;
    Ok(Tasks::from_bcf(&value))
}

fn serial(scene: &Obj<'_>) -> u64 {
    opt(scene, "_tasks_serial").and_then(|v| v.extract().ok()).unwrap_or(0)
}

/// Keep `tasks` in the document; `edit`: the document has unsaved changes.
pub fn store(scene: &Obj<'_>, tasks: &Tasks, edit: bool) -> PyResult<()> {
    let value = tasks.to_bcf();
    let text = value.to_string();
    scene.setattr("_tasks_json", &text)?;
    write_plugin_data(scene, &value)?;
    scene.setattr("_tasks_serial", serial(scene) + 1)?;
    if edit {
        let v: i64 = scene.getattr("version")?.extract()?;
        scene.setattr("version", v + 1)?;
    }
    Ok(())
}

fn write_plugin_data(scene: &Obj<'_>, value: &Value) -> PyResult<()> {
    let py = scene.py();
    let data = scene.getattr("plugin_data")?;
    if value["topics"].as_array().is_some_and(|t| t.is_empty()) {
        data.call_method1("pop", (KEY, py.None()))?;
    } else {
        data.set_item(KEY, py_json(py, value)?)?;
    }
    scene.setattr("_tasks_written", data.call_method1("get", (KEY,))?)
}

/// Put the records back if a step restored an older `plugin_data` (it
/// puts back a copy: a different object from the one written).
fn repair(scene: &Obj<'_>) -> PyResult<()> {
    let Some(text) = opt(scene, "_tasks_json") else { return Ok(()) };
    let stored = scene.getattr("plugin_data")?.call_method1("get", (KEY,))?;
    let written = scene.getattr_opt("_tasks_written")?.unwrap_or_else(|| none(scene.py()));
    if stored.is(&written) {
        return Ok(());
    }
    let ours: Value = serde_json::from_str(&text.extract::<String>()?).map_err(|e| err(e.to_string()))?;
    write_plugin_data(scene, &ours)
}

/// Change one task and keep it.
fn modify<R>(win: &Obj<'_>, guid: &str, f: impl FnOnce(&mut Task) -> Result<R, String>) -> PyResult<R> {
    let scene = scene_of(win)?;
    let mut all = load(&scene)?;
    let task = all.get_mut(guid).map_err(err)?;
    let out = f(task).map_err(err)?;
    store(&scene, &all, true)?;
    Ok(out)
}

// ------------------------------------------------------------- camera

fn xyz(v: &Obj<'_>) -> PyResult<[f64; 3]> {
    Ok([v.call_method0("x")?.extract()?, v.call_method0("y")?.extract()?, v.call_method0("z")?.extract()?])
}

/// The camera and selection now, as a BCF viewpoint.
fn viewpoint_now(win: &Obj<'_>) -> PyResult<Viewpoint> {
    let vp = viewport(win)?;
    let cam = vp.getattr("camera")?;
    let eye = xyz(&cam.call_method0("eye")?)?;
    let target = xyz(&cam.getattr("target")?)?;
    let up = xyz(&cam.call_method0("up_vector")?)?;
    let d = [target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]];
    let n = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-12);
    let pose: Vec<f64> = win.call_method0("_camera_pose")?.extract()?;
    let ortho = !cam.getattr("perspective")?.is_truthy()?;
    let distance: f64 = cam.getattr("distance")?.extract()?;
    let mut selection = Vec::new();
    for e in vp.getattr("scene")?.getattr("selection")?.try_iter()? {
        if let Some(uid) = opt(&e?, "uid") {
            selection.push(uid.str()?.to_string());
        }
    }
    Ok(Viewpoint {
        guid: tasks::guid(),
        eye,
        direction: [d[0] / n, d[1] / n, d[2] / n],
        up,
        fov: cam.getattr("fov_deg")?.extract()?,
        ortho,
        scale: if ortho { distance } else { 0.0 },
        pose: (pose.len() == 6).then(|| [pose[0], pose[1], pose[2], pose[3], pose[4], pose[5]]),
        selection,
    })
}

fn restore_view(win: &Obj<'_>, v: &Viewpoint) -> PyResult<()> {
    let Some([x, y, z, distance, yaw, pitch]) = v.pose else { return Ok(()) };
    let vp = viewport(win)?;
    let cam = vp.getattr("camera")?;
    if cam.getattr("perspective")?.is_truthy()? == v.ortho {
        cam.call_method1("set_projection", (if v.ortho { "parallel" } else { "perspective" },))?;
    }
    cam.setattr("distance", distance)?;
    cam.setattr("yaw", yaw)?;
    cam.setattr("pitch", pitch)?;
    cam.setattr("target", (x, y, z))?;
    vp.call_method0("update")?;
    Ok(())
}

/// The middle of the selection, where a pinned request is anchored.
fn selection_centre(win: &Obj<'_>) -> PyResult<Option<[f64; 3]>> {
    let scene = scene_of(win)?;
    if !scene.getattr("selection")?.is_truthy()? {
        return Ok(None);
    }
    let b = scene.call_method0("selection_bounds")?;
    if b.is_none() {
        return Ok(None);
    }
    let lo = b.get_item(0)?;
    let hi = b.get_item(1)?;
    if lo.is_none() || hi.is_none() {
        return Ok(None);
    }
    let (lo, hi) = (xyz(&lo)?, xyz(&hi)?);
    Ok(Some([(lo[0] + hi[0]) / 2., (lo[1] + hi[1]) / 2., hi[2]]))
}

// ------------------------------------------------------------- the model's elements

/// uid → (group, name) of every placement in the model.
fn placements<'py>(scene: &Obj<'py>) -> PyResult<std::collections::HashMap<String, (Obj<'py>, String)>> {
    let py = scene.py();
    let mut out = std::collections::HashMap::new();
    for entry in py.import("built_core.structure")?.call_method1("_placements", (scene,))?.try_iter()? {
        let g = entry?.get_item(0)?;
        let uid = g.getattr("uid")?.str()?.to_string();
        let name = opt(&g, "name").map(|n| n.str().map(|s| s.to_string())).transpose()?.unwrap_or_default();
        out.insert(uid, (g, name));
    }
    Ok(out)
}

fn label_of(names: &std::collections::HashMap<String, (Obj<'_>, String)>, key: &str, before: &str) -> String {
    match names.get(key) {
        Some((_, n)) if !n.is_empty() => n.clone(),
        _ if !before.is_empty() => before.to_owned(),
        Some(_) => "Unnamed group".into(),
        None => "Removed element".into(),
    }
}

/// What one attributed history step changed: `(uid, kind)` pairs kept on
/// the step when it was attributed, labelled with today's names.
fn step_changes(cmd: &Obj<'_>, names: &std::collections::HashMap<String, (Obj<'_>, String)>, known: &[Changed]) -> PyResult<Vec<Changed>> {
    let Some(pairs) = opt(cmd, "_task_changes") else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for pair in pairs.try_iter()? {
        let (key, kind): (String, String) = pair?.extract()?;
        let Some(kind) = ChangeKind::parse(&kind) else { continue };
        let before = known.iter().find(|c| c.key == key).map(|c| c.label.as_str()).unwrap_or("");
        let label = if key == MODEL { "Loose geometry".to_owned() } else { label_of(names, &key, before) };
        out.push(Changed { label, key, kind });
    }
    Ok(out)
}

/// The key of loose geometry (edges and faces outside any group).
const MODEL: &str = "model";

/// `(uid, kind)` of a change-feed step's placements; loose geometry as
/// one "model" element.
fn feed_pairs<'py>(step: &Obj<'py>) -> PyResult<(Vec<(String, String)>, bool)> {
    let placed = step.getattr("placements")?;
    let mut out = Vec::new();
    for field in ["created", "removed", "modified"] {
        for key in placed.get_item(field)?.try_iter()? {
            out.push((key?.str()?.to_string(), field.to_owned()));
        }
    }
    let meshes = step.getattr("meshes")?.is_truthy()?;
    if out.is_empty() && meshes {
        out.push((MODEL.to_owned(), "modified".to_owned()));
    }
    Ok((out, meshes))
}

fn stack<'py>(win: &Obj<'py>, name: &str) -> PyResult<Bound<'py, PyList>> {
    Ok(viewport(win)?.getattr("history")?.getattr(name)?.cast_into::<PyList>()?)
}

fn task_of(cmd: &Obj<'_>) -> Option<String> {
    opt(cmd, "_task").and_then(|t| t.extract().ok())
}

/// The task's changes again from its steps still in the history (undoing
/// one of them takes its changes out). Kept as stored when none is there
/// any more (a reopened document).
fn recompute(win: &Obj<'_>, task: &mut Task) -> PyResult<()> {
    let undo = stack(win, "undo_stack")?;
    let redo = stack(win, "redo_stack")?;
    let mine: Vec<Obj<'_>> = undo.iter().filter(|c| task_of(c).as_deref() == Some(&task.guid)).collect();
    let undone = redo.iter().any(|c| task_of(&c).as_deref() == Some(&task.guid));
    if mine.is_empty() && !undone {
        return Ok(());
    }
    let scene = scene_of(win)?;
    let names = placements(&scene)?;
    let known = std::mem::take(&mut task.changes);
    task.reset_changes();
    for cmd in mine {
        let revision = opt(&cmd, "_task_revision").and_then(|r| r.extract().ok()).unwrap_or(0);
        let changes = step_changes(&cmd, &names, &known)?;
        task.record(&changes, revision);
    }
    Ok(())
}

// ------------------------------------------------------------- Python API

/// A new task. `pin`: anchored at the selection; `view`: remembers the
/// camera and selection. Returns its GUID.
#[pyfunction]
#[pyo3(signature = (win, title, description="", agent="", source="agent", author="", pin=false, view=true))]
#[allow(clippy::too_many_arguments)]
pub fn task_new(win: &Obj<'_>, title: &str, description: &str, agent: &str, source: &str, author: &str, pin: bool, view: bool) -> PyResult<String> {
    let title = if title.trim().is_empty() { description.lines().next().unwrap_or("").trim() } else { title.trim() };
    if title.is_empty() {
        return Err(err("A task needs a request"));
    }
    let title: String = title.chars().take(120).collect();
    let author = if author.is_empty() { if source == "agent" { agent } else { USER } } else { author };
    let guid = tasks::guid();
    let mut task = Task::new(&guid, &title, description, author, agent, source, &tasks::now());
    if view {
        task.viewpoint = Some(viewpoint_now(win)?);
    }
    if pin {
        task.pin = selection_centre(win)?;
    }
    let scene = scene_of(win)?;
    let mut all = load(&scene)?;
    all.topics.push(task);
    store(&scene, &all, true)?;
    Ok(guid)
}

/// The agent reports `state` (working, waiting, question) or `done`.
#[pyfunction]
#[pyo3(signature = (win, guid, state, message="", author=""))]
pub fn task_report(win: &Obj<'_>, guid: &str, state: &str, message: &str, author: &str) -> PyResult<String> {
    if state.trim().eq_ignore_ascii_case("done") || state.trim().eq_ignore_ascii_case("finished") {
        return task_finish(win, guid, message, author);
    }
    let state = State::parse(state).ok_or_else(|| err(format!("Unknown state {state:?}: working, waiting, question or done")))?;
    modify(win, guid, |t| {
        let who = if author.is_empty() { t.agent.clone() } else { author.to_owned() };
        t.report(state, message, &who, &tasks::guid(), &tasks::now()).map(|_| t.state.key().to_owned())
    })
}

/// The agent is done: Done without net edits, To review with them.
#[pyfunction]
#[pyo3(signature = (win, guid, summary="", author=""))]
pub fn task_finish(win: &Obj<'_>, guid: &str, summary: &str, author: &str) -> PyResult<String> {
    let scene = scene_of(win)?;
    let mut all = load(&scene)?;
    let task = all.get_mut(guid).map_err(err)?;
    recompute(win, task)?;
    let who = if author.is_empty() { task.agent.clone() } else { author.to_owned() };
    let state = task.finish(summary, &who, &tasks::guid(), &tasks::now()).map_err(err)?;
    store(&scene, &all, true)?;
    Ok(state.key().to_owned())
}

/// A comment on the task (the agent's progress note), state unchanged.
#[pyfunction]
#[pyo3(signature = (win, guid, text, author=""))]
pub fn task_comment(win: &Obj<'_>, guid: &str, text: &str, author: &str) -> PyResult<()> {
    modify(win, guid, |t| {
        let who = if author.is_empty() { t.agent.clone() } else { author.to_owned() };
        t.comment(&tasks::guid(), &who, text, &tasks::now());
        Ok(())
    })
}

fn follow_up(win: &Obj<'_>, guid: &str, text: &str) -> PyResult<()> {
    let task = load(&scene_of(win)?)?.get(guid).cloned();
    if task.is_some_and(|t| t.source == "assistant") {
        win.py().import("plugins._assistant_panel")?.call_method1("task_follow_up", (win, guid, text))?;
    }
    Ok(())
}

/// The user answers the task's question or request for action.
#[pyfunction]
#[pyo3(signature = (win, guid, text, author=USER))]
pub fn task_reply(win: &Obj<'_>, guid: &str, text: &str, author: &str) -> PyResult<()> {
    modify(win, guid, |t| t.reply(text, author, &tasks::guid(), &tasks::now()))?;
    follow_up(win, guid, text)
}

/// The user reopens a finished task with what to change.
#[pyfunction]
#[pyo3(signature = (win, guid, text, author=USER))]
pub fn task_request_changes(win: &Obj<'_>, guid: &str, text: &str, author: &str) -> PyResult<()> {
    modify(win, guid, |t| t.request_changes(text, author, &tasks::guid(), &tasks::now()))?;
    follow_up(win, guid, text)
}

#[pyfunction]
pub fn task_accept(win: &Obj<'_>, guid: &str) -> PyResult<()> {
    modify(win, guid, |t| t.accept(USER, &tasks::now()))
}

/// A history marker before a task's edit: the step on top and the
/// revision.
#[pyfunction]
pub fn task_edit_begin<'py>(win: &Obj<'py>) -> PyResult<Obj<'py>> {
    let undo = stack(win, "undo_stack")?;
    let py = win.py();
    let top = if undo.is_empty() { none(py) } else { undo.get_item(undo.len() - 1)? };
    let revision = viewport(win)?.getattr("history")?.getattr("revision")?;
    Ok(PyTuple::new(py, [top, revision])?.into_any())
}

/// The steps pushed since `marker` belong to the task: each keeps what it
/// changed (from the change feed) and, when the document recorded one, its
/// transaction (for a revert under later work). Returns how many.
#[pyfunction]
pub fn task_edit_end(win: &Obj<'_>, guid: &str, marker: &Obj<'_>) -> PyResult<usize> {
    let py = win.py();
    let history = viewport(win)?.getattr("history")?;
    let (top, since): (Obj<'_>, u64) = (marker.get_item(0)?, marker.get_item(1)?.extract()?);
    let undo = stack(win, "undo_stack")?;
    let mut fresh = Vec::new();
    for at in (0..undo.len()).rev() {
        let cmd = undo.get_item(at)?;
        if cmd.is(&top) || task_of(&cmd).is_some() {
            break;
        }
        fresh.push(cmd);
    }
    fresh.reverse();
    if fresh.is_empty() {
        return Ok(0);
    }
    let steps: Vec<Obj<'_>> = history.getattr("feed")?.call_method1("since", (since,))?.get_item("steps")?
        .try_iter()?.collect::<PyResult<Vec<_>>>()?
        .into_iter().filter(|s| s.getattr("phase").and_then(|p| p.extract::<String>()).is_ok_and(|p| p == "execute" || p == "do"))
        .collect();
    let last_txn = history.getattr("last_txn")?;
    let revision: u64 = history.getattr("revision")?.extract().unwrap_or(0);
    let n = fresh.len();
    for (i, cmd) in fresh.iter().enumerate() {
        // One feed step per pushed step; when they do not pair up, the
        // whole call's changes go to its last step.
        let (pairs, loose) = if steps.len() == n {
            feed_pairs(&steps[i])?
        } else if i + 1 == n {
            let mut all = Vec::new();
            let mut loose = false;
            for s in &steps {
                let (p, l) = feed_pairs(s)?;
                all.extend(p);
                loose |= l;
            }
            (all, loose)
        } else {
            (Vec::new(), true)
        };
        let txn = match opt(cmd, "_doc_txn") {
            Some(t) => Some(t),
            None if i + 1 == n && !last_txn.is_none() => opt(&last_txn.get_item(0)?, "changes").and(Some(last_txn.get_item(0)?)),
            None => None,
        };
        cmd.setattr("_task", guid)?;
        cmd.setattr("_task_revision", steps.get(i).and_then(|s| s.getattr("revision").ok()).and_then(|r| r.extract::<u64>().ok()).unwrap_or(revision))?;
        cmd.setattr("_task_changes", PyList::new(py, pairs)?)?;
        cmd.setattr("_task_loose", loose)?;
        cmd.setattr("_task_txn", txn.unwrap_or_else(|| none(py)))?;
    }
    let scene = scene_of(win)?;
    let mut all = load(&scene)?;
    let task = all.get_mut(guid).map_err(err)?;
    recompute(win, task)?;
    store(&scene, &all, true)?;
    Ok(n)
}

/// Undo a task's steps as one new step. `own`: the steps are the last in
/// the history, so their own inverses run; otherwise their recorded
/// transactions do, which touch nothing else.
#[pyclass(module = "built_render", dict)]
pub struct TaskRevert {
    steps: Py<PyList>,
    own: bool,
    guid: String,
    before: String,
    after: String,
}

impl TaskRevert {
    fn replay(&self, scene: &Obj<'_>, backwards: bool) -> PyResult<()> {
        let py = scene.py();
        let steps = self.steps.bind(py);
        let doc = py.import("built_core.doc_model")?;
        let order: Vec<usize> = if backwards { (0..steps.len()).rev().collect() } else { (0..steps.len()).collect() };
        for at in order {
            let cmd = steps.get_item(at)?;
            let current = scene.getattr("mesh")?;
            let target = opt(&cmd, "_history_mesh").filter(|m| !m.is(&current));
            if let Some(m) = &target {
                scene.setattr("mesh", m)?;
            }
            let result = (|| -> PyResult<()> {
                let txn = if self.own { opt(&cmd, "_doc_txn") } else { opt(&cmd, "_task_txn") };
                match txn {
                    Some(txn) => {
                        doc.call_method0("flush")?;
                        txn.call_method1("replay_editor", (&cmd, scene, backwards))?;
                    }
                    None if self.own => {
                        cmd.call_method1(if backwards { "undo" } else { "do" }, (scene,))?;
                    }
                    None => return Err(err("A step of this task cannot be replayed")),
                }
                Ok(())
            })();
            if target.is_some() {
                scene.setattr("mesh", &current)?;
            }
            result?;
        }
        Ok(())
    }
    fn state(&self, scene: &Obj<'_>, text: &str) -> PyResult<()> {
        let mut all = load(scene)?;
        let value: Value = serde_json::from_str(text).map_err(|e| err(e.to_string()))?;
        if let (Some(task), Ok(slot)) = (Task::from_bcf(&value), all.get_mut(&self.guid)) {
            *slot = task;
        }
        store(scene, &all, false)
    }
}

#[pymethods]
impl TaskRevert {
    #[getter]
    fn keeps_rows(&self) -> bool {
        false
    }
    #[getter]
    fn label(&self) -> &'static str {
        "Revert task"
    }
    fn r#do(&self, scene: &Obj<'_>) -> PyResult<()> {
        self.replay(scene, true)?;
        self.state(scene, &self.after)
    }
    fn undo(&self, scene: &Obj<'_>) -> PyResult<()> {
        self.replay(scene, false)?;
        self.state(scene, &self.before)
    }
}

/// Revert a task's edits as one undoable step. Refused, with the reason,
/// when later work changed the same elements or the steps are gone.
#[pyfunction]
pub fn task_revert(win: &Obj<'_>, guid: &str) -> PyResult<()> {
    let py = win.py();
    let scene = scene_of(win)?;
    let mut all = load(&scene)?;
    let task = all.get_mut(guid).map_err(err)?.clone();
    if !matches!(task.state, State::Review | State::Done) {
        return Err(err("Only a finished task can be reverted"));
    }
    let undo = stack(win, "undo_stack")?;
    let steps: Vec<Obj<'_>> = undo.iter().filter(|c| task_of(c).as_deref() == Some(guid)).collect();
    if steps.is_empty() || task.changes.is_empty() {
        return Err(err("This task's edits are no longer in the undo history"));
    }
    let first = (0..undo.len()).find(|&i| undo.get_item(i).is_ok_and(|c| c.is(&steps[0]))).unwrap_or(0);
    let keys: std::collections::HashSet<String> = task.changes.iter().map(|c| c.key.clone()).collect();
    let names = placements(&scene)?;
    let mut overlap: Vec<String> = Vec::new();
    let mut later = 0;
    for at in first..undo.len() {
        let cmd = undo.get_item(at)?;
        if task_of(&cmd).as_deref() == Some(guid) {
            continue;
        }
        later += 1;
        let pairs = if opt(&cmd, "_task_changes").is_some() {
            step_changes(&cmd, &names, &task.changes)?
        } else if let Some(txn) = opt(&cmd, "_doc_txn") {
            let placed = txn.call_method0("changes")?.get_item("placements")?;
            let mut v = Vec::new();
            for field in ["created", "removed", "modified"] {
                for key in placed.get_item(field)?.try_iter()? {
                    let key = key?.str()?.to_string();
                    v.push(Changed { label: label_of(&names, &key, ""), key, kind: ChangeKind::Modified });
                }
            }
            v
        } else {
            // An opaque step: it may have changed anything.
            task.changes.clone()
        };
        for c in pairs {
            if keys.contains(&c.key) && !overlap.contains(&c.label) {
                overlap.push(c.label);
            }
        }
    }
    if !overlap.is_empty() {
        return Err(err(format!("Later edits changed {} too. Undo them first, or keep the task.", overlap.join(", "))));
    }
    let own = later == 0;
    if !own && steps.iter().any(|c| opt(c, "_task_txn").is_none() || opt(c, "_task_loose").is_some_and(|l| l.is_truthy().unwrap_or(true))) {
        return Err(err(format!("{later} later step{} would have to be undone first: this task drew loose geometry.", if later == 1 { "" } else { "s" })));
    }
    let before = task.to_bcf().to_string();
    let mut reverted = task.clone();
    reverted.revert(USER, &tasks::now()).map_err(err)?;
    let after = reverted.to_bcf().to_string();
    let cmd = Bound::new(py, TaskRevert { steps: PyList::new(py, &steps)?.unbind(), own, guid: guid.into(), before, after })?;
    let history = viewport(win)?.getattr("history")?;
    history.call_method1("execute", (&cmd,))?;
    let failed = opt(&history, "last_error").or_else(|| opt(&history, "last_rejected"));
    if let Some(e) = failed {
        return Err(err(format!("The task could not be reverted: {}", e.str()?)));
    }
    viewport(win)?.call_method0("notify_scene_changed")?;
    Ok(())
}

fn task_value(t: &Task) -> Value {
    let mut v = t.to_bcf();
    v["state"] = json!(t.state.key());
    v["state_label"] = json!(t.state.label());
    v["needs_attention"] = json!(t.state.needs_attention());
    v["changed"] = json!(t.changes.len());
    v
}

/// One task as JSON (the BCF topic, plus `state`).
#[pyfunction]
pub fn task_get(win: &Obj<'_>, guid: &str) -> PyResult<String> {
    let all = load(&scene_of(win)?)?;
    let t = all.get(guid).ok_or_else(|| err(format!("No task {guid}")))?;
    Ok(task_value(t).to_string())
}

/// The document's tasks as JSON, newest first; settled ones on request.
#[pyfunction]
#[pyo3(signature = (win, settled=false))]
pub fn task_list(win: &Obj<'_>, settled: bool) -> PyResult<String> {
    let all = load(&scene_of(win)?)?;
    let rows: Vec<Value> = all.topics.iter().rev().filter(|t| settled || !t.state.settled()).map(task_value).collect();
    Ok(Value::Array(rows).to_string())
}

// ------------------------------------------------------------- the destination

fn tone(state: State) -> &'static str {
    match state {
        State::Question | State::Waiting | State::Review => "warn",
        State::Working => "dim",
        State::Done => "ok",
        State::Reverted => "faint",
    }
}

fn selected(scene: &Obj<'_>) -> String {
    opt(scene, "_task_selected").and_then(|v| v.extract().ok()).unwrap_or_default()
}

/// Agents the New task composer offers: the assistant, then the clients
/// connected to this window.
fn agents(win: &Obj<'_>) -> PyResult<Vec<String>> {
    let mut out = vec![ASSISTANT.to_owned()];
    for name in clients(win)? {
        if !out.contains(&name) {
            out.push(name);
        }
    }
    Ok(out)
}

/// Names of the MCP clients connected now.
pub fn clients(win: &Obj<'_>) -> PyResult<Vec<String>> {
    let Some(bridge) = win.getattr_opt("_ai_bridge")?.filter(|b| !b.is_none()) else { return Ok(Vec::new()) };
    if !bridge.hasattr("clients")? {
        return Ok(Vec::new());
    }
    bridge.call_method0("clients")?.extract()
}

/// Whether external agents may edit this document.
pub fn access(scene: &Obj<'_>) -> bool {
    opt(scene, "_agent_access").is_some_and(|v| v.is_truthy().unwrap_or(false))
}

fn destination_data(win: &Obj<'_>) -> PyResult<Value> {
    let scene = scene_of(win)?;
    let all = load(&scene)?;
    let chosen = selected(&scene);
    let row = |t: &Task| json!({
        "guid": t.guid, "title": t.title, "agent": t.agent, "state": t.state.key(),
        "label": t.state.label(), "tone": tone(t.state), "count": t.changes.len(),
        "pinned": t.pin.is_some(),
    });
    let [attention, progress, settled] = all.groups();
    let groups = json!([
        {"name": "Needs attention", "rows": attention.iter().map(|t| row(t)).collect::<Vec<_>>()},
        {"name": "In progress", "rows": progress.iter().map(|t| row(t)).collect::<Vec<_>>()},
        {"name": "Settled", "rows": settled.iter().map(|t| row(t)).collect::<Vec<_>>()},
    ]);
    let detail = all.get(&chosen).map(|t| {
        let mut d = json!({
            "guid": t.guid, "title": t.title, "description": t.description, "agent": t.agent,
            "author": t.author, "state": t.state.key(), "label": t.state.label(), "tone": tone(t.state),
            "summary": t.summary, "question": t.question, "pinned": t.pin.is_some(),
            "changes": t.changes.iter().map(|c| json!({"key": c.key, "label": c.label, "change": c.kind.label()})).collect::<Vec<_>>(),
            "comments": t.comments.iter().map(|c| json!({"author": c.author, "text": c.text, "date": c.date})).collect::<Vec<_>>(),
            "can_accept": t.state == State::Review,
            "can_request": matches!(t.state, State::Review | State::Done),
            "can_revert": matches!(t.state, State::Review | State::Done) && !t.changes.is_empty(),
            "can_reply": matches!(t.state, State::Question | State::Waiting),
        });
        if let Some(note) = opt(&scene, "_task_note").and_then(|n| n.extract::<(String, String)>().ok()).filter(|n| n.0 == t.guid) {
            d["note"] = json!(note.1);
        }
        d
    });
    let empty = all.topics.is_empty();
    Ok(json!({
        "unresolved": all.attention(),
        "selected": chosen,
        "empty": empty,
        "groups": groups,
        "detail": detail,
        "agents": agents(win)?,
        "access": access(&scene),
        "clients": clients(win)?,
        "selection": scene.getattr("selection")?.len()?,
    }))
}

fn select_task(win: &Obj<'_>, guid: &str) -> PyResult<()> {
    let scene = scene_of(win)?;
    let all = load(&scene)?;
    let Some(t) = all.get(guid) else { return Ok(()) };
    scene.setattr("_task_selected", guid)?;
    scene.setattr("_task_note", none(win.py()))?;
    if let Some(v) = &t.viewpoint {
        restore_view(win, v)?;
    }
    bump(&scene)
}

fn bump(scene: &Obj<'_>) -> PyResult<()> {
    scene.setattr("_tasks_serial", serial(scene) + 1)
}

fn select_elements(win: &Obj<'_>, keys: &[String]) -> PyResult<()> {
    let py = win.py();
    let vp = viewport(win)?;
    let scene = vp.getattr("scene")?;
    let names = placements(&scene)?;
    let chosen = PyList::empty(py);
    for k in keys {
        if let Some((g, _)) = names.get(k) {
            if scene.call_method1("entity_selectable", (g,))?.is_truthy()? {
                chosen.append(g)?;
            }
        }
    }
    scene.call_method1("select", (chosen.try_iter()?,))?;
    let update = vp.getattr("update")?;
    vp.getattr_opt("notify_scene_changed")?.unwrap_or(update).call0()?;
    Ok(())
}

fn note(win: &Obj<'_>, guid: &str, text: &str) -> PyResult<()> {
    let scene = scene_of(win)?;
    scene.setattr("_task_note", (guid, text))?;
    bump(&scene)
}

fn send_request(win: &Obj<'_>, agent: &str, pin: bool, text: &str) -> PyResult<()> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    let assistant = agent.is_empty() || agent == ASSISTANT;
    let agent = if assistant { ASSISTANT } else { agent };
    let guid = task_new(win, text.lines().next().unwrap_or(text), text, agent, if assistant { "assistant" } else { "user" }, USER, pin, true)?;
    let scene = scene_of(win)?;
    scene.setattr("_task_selected", &guid)?;
    if assistant {
        win.py().import("plugins._assistant_panel")?.call_method1("task_follow_up", (win, &guid, text))?;
    }
    Ok(())
}

/// One verb from the Tasks destination: `verb:guid[SEP text]`.
#[pyfunction]
pub fn task_action(win: &Obj<'_>, arg: &str) -> PyResult<()> {
    let (verb, rest) = arg.split_once(':').unwrap_or((arg, ""));
    let (guid, text) = rest.split_once(SEP).unwrap_or((rest, ""));
    let scene = scene_of(win)?;
    let result = match verb {
        "select" => select_task(win, guid),
        "clear" => {
            scene.setattr("_task_selected", "")?;
            bump(&scene)
        }
        "accept" => task_accept(win, guid),
        "changes" => task_request_changes(win, guid, text, USER),
        "reply" => task_reply(win, guid, text, USER),
        "revert" => task_revert(win, guid),
        "element" => select_elements(win, &[text.to_owned()]),
        "settings" => {
            let kw = pyo3::types::PyDict::new(win.py());
            kw.set_item("page", "ai")?;
            win.call_method("open_preferences", (), Some(&kw)).map(drop)
        }
        "project" => {
            let all = load(&scene)?;
            let keys = all.get(guid).map(Task::changed_keys).unwrap_or_default();
            select_elements(win, &keys)?;
            let view = crate::dialogs_host::view_of(win.py(), win);
            if !view.is_none() {
                view.getattr("renderer")?.call_method1("ui_show_destination", ("project",))?;
            }
            Ok(())
        }
        "new" => {
            // new:agent SEP pin SEP text
            let mut parts = text.splitn(2, SEP);
            let pin = parts.next() == Some("1");
            send_request(win, guid, pin, parts.next().unwrap_or(""))
        }
        _ => Ok(()),
    };
    match result {
        Err(e) if e.is_instance_of::<PyValueError>(win.py()) => {
            let text = e.value(win.py()).str()?.to_string();
            note(win, guid, &text)
        }
        other => {
            if !matches!(verb, "select" | "clear" | "element" | "project" | "settings") {
                scene.setattr("_task_note", none(win.py()))?;
            }
            bump(&scene)?;
            other
        }
    }
}

/// Mirror the tasks into the chrome before a frame: the destination's
/// data when it changed, the preview of the selected task's changes and
/// the pins in view.
#[pyfunction]
pub fn tasks_before_frame(view: &Obj<'_>, win: &Obj<'_>) -> PyResult<()> {
    let py = view.py();
    let scene = scene_of(win)?;
    repair(&scene)?;
    let Some(renderer) = opt(view, "renderer").filter(|r| r.hasattr("ui_panel_set").unwrap_or(false)) else {
        return Ok(());
    };
    let packed = opt(view, "packed").map_or(0, |p| p.as_ptr() as usize);
    let sig = (scene.as_ptr() as usize, serial(&scene), access(&scene), clients(win)?, scene.getattr("selection")?.len()?, packed).into_pyobject(py)?.into_any();
    let state = view.getattr("__dict__")?;
    if sig.ne(state.call_method1("get", ("_ui_tasks_sig",))?)? {
        view.setattr("_ui_tasks_sig", &sig)?;
        renderer.call_method1("ui_panel_set", ("tasks", destination_data(win)?.to_string()))?;
        let all = load(&scene)?;
        let chosen = selected(&scene);
        // The preview: the selected task's changed elements, lit.
        let rows = PyList::empty(py);
        if let (Some(t), Some(uid_rows)) = (all.get(&chosen), opt(view, "uid_rows")) {
            for key in t.changed_keys() {
                rows.call_method1("extend", (uid_rows.call_method1("get", (key, PyTuple::empty(py)))?,))?;
            }
        }
        let rows = PyTuple::new(py, rows.iter())?;
        if rows.as_any().ne(state.call_method1("get", ("_task_rows", PyTuple::empty(py)))?)? {
            view.setattr("_task_rows", rows)?;
            view.call_method0("update")?;
        }
        // The pins to draw: open tasks', and the selected one's.
        let points = PyList::empty(py);
        for t in all.topics.iter().filter(|t| t.pin.is_some() && (!t.state.settled() || t.guid == chosen)) {
            let p = t.pin.expect("filtered");
            points.append((t.guid.clone(), p[0], p[1], p[2], t.guid == chosen, t.state.needs_attention(), t.changes.len()))?;
        }
        view.setattr("_task_pin_points", points)?;
    }
    // Where they are this frame (the camera moves).
    let Some(points) = opt(view, "_task_pin_points") else { return Ok(()) };
    if !renderer.hasattr("ui_set_task_pins")? {
        return Ok(());
    }
    let pins = PyList::empty(py);
    if let (true, Some(to_pixel)) = (points.is_truthy()?, opt(view, "_world_to_pixel")) {
        let vector = py.import("built_core")?.getattr("Vec3")?;
        for p in points.try_iter()? {
            let (guid, x, y, z, chosen, attention, count): (String, f64, f64, f64, bool, bool, usize) = p?.extract()?;
            if let Ok((px, py_)) = to_pixel.call1((vector.call1((x, y, z))?,))?.extract::<(f64, f64)>() {
                pins.append((guid, px as f32, py_ as f32, chosen, attention, count))?;
            }
        }
    }
    if pins.is_empty() && state.call_method1("get", ("_task_pins_empty", false))?.is_truthy()? {
        return Ok(());
    }
    view.setattr("_task_pins_empty", pins.is_empty())?;
    renderer.call_method1("ui_set_task_pins", (pins,))?;
    Ok(())
}

/// The viewport rows tinted for the selected task.
pub fn preview_rows<'py>(view: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    Ok(opt(view, "_task_rows").filter(|r| r.is_truthy().unwrap_or(false)))
}

/// Whether external agents may edit the front document; turning it off
/// ends their access to it.
#[pyfunction]
pub fn set_agent_access(win: &Obj<'_>, on: bool) -> PyResult<()> {
    let scene = scene_of(win)?;
    scene.setattr("_agent_access", on)?;
    bump(&scene)?;
    win.py().import("plugins.ai_bridge")?.call_method1("sync_access", (win,))?;
    Ok(())
}

#[pyfunction]
pub fn agent_access(win: &Obj<'_>) -> PyResult<bool> {
    Ok(access(&scene_of(win)?))
}

/// The running program: inside the app, the Built binary an MCP client
/// starts with `--mcp` (Python's `sys.executable` is the interpreter).
#[pyfunction]
pub fn app_executable() -> Option<String> {
    std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<TaskRevert>()?;
    m.add_function(wrap_pyfunction!(app_executable, m)?)?;
    for f in [
        wrap_pyfunction!(task_new, m)?,
        wrap_pyfunction!(task_report, m)?,
        wrap_pyfunction!(task_finish, m)?,
        wrap_pyfunction!(task_comment, m)?,
        wrap_pyfunction!(task_reply, m)?,
        wrap_pyfunction!(task_request_changes, m)?,
        wrap_pyfunction!(task_accept, m)?,
        wrap_pyfunction!(task_revert, m)?,
        wrap_pyfunction!(task_edit_begin, m)?,
        wrap_pyfunction!(task_edit_end, m)?,
        wrap_pyfunction!(task_get, m)?,
        wrap_pyfunction!(task_list, m)?,
        wrap_pyfunction!(task_action, m)?,
        wrap_pyfunction!(tasks_before_frame, m)?,
        wrap_pyfunction!(set_agent_access, m)?,
        wrap_pyfunction!(agent_access, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
