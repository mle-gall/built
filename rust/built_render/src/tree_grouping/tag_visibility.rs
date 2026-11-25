// SPDX-License-Identifier: GPL-3.0-or-later
//! Tag isolation uses the document's tag visibility, including loose geometry.
//! Keep the restore point with this document and its undo step.
use super::{row_actions::writable, translated, READ_ONLY};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use std::collections::{BTreeMap, BTreeSet};

const STATE: &str = "_drawing_tag_isolation";
type Restore = (BTreeSet<String>, BTreeMap<String, bool>);
#[pyclass]
struct IsolationStep {
    before: Option<Restore>,
    after: Option<Restore>,
}
#[pymethods]
impl IsolationStep {
    #[classattr]
    #[pyo3(name = "doc_undo")]
    const DOC_UNDO: bool = true;
    #[classattr]
    #[pyo3(name = "keeps_rows")]
    const KEEPS_ROWS: bool = true;
    #[pyo3(name = "do")]
    fn apply(&self, scene: &Bound<'_, PyAny>) -> PyResult<()> {
        scene.setattr(STATE, &self.after)
    }
    fn undo(&self, scene: &Bound<'_, PyAny>) -> PyResult<()> {
        scene.setattr(STATE, &self.before)
    }
    fn doc_after(&self, scene: &Bound<'_, PyAny>, phase: &str) -> PyResult<()> {
        if phase == "undo" {
            self.undo(scene)
        } else {
            self.apply(scene)
        }
    }
}

#[pyfunction]
pub fn isolate_tags(vp: &Bound<'_, PyAny>, rows: Vec<String>) -> PyResult<()> {
    let py = vp.py();
    let scene = vp.getattr("scene")?;
    if !writable(&scene)? {
        vp.call_method1("flash_status", (translated(py, READ_ONLY, None)?, 4000))?;
        return Ok(());
    }
    let layers = py.import("core.layers")?;
    let mut keys = BTreeSet::new();
    for row in &rows {
        if let Some(key) = row.strip_prefix("preview-folder:") {
            keys.extend(
                layers
                    .call_method1("layers_in_folder", (&scene, key))?
                    .extract::<Vec<String>>()?,
            );
        } else if let Some(key) = row.strip_prefix("preview-tag:") {
            if scene.call_method1("layer_by_key", (key,))?.is_none() {
                return Ok(());
            }
            keys.insert(key.to_owned());
        } else {
            return Ok(());
        }
    }
    if !rows.is_empty() && keys.is_empty() {
        return Ok(());
    }
    let before: Option<Restore> = scene
        .getattr("__dict__")?
        .cast::<PyDict>()?
        .get_item(STATE)?
        .map(|v| v.extract())
        .transpose()?
        .flatten();
    let mut original = before.as_ref().map(|r| r.1.clone()).unwrap_or_default();
    let mut requested = BTreeSet::new();
    let mut entries = vec![];
    for tag in scene.getattr("layers")?.try_iter()? {
        let tag = tag?;
        let key: String = tag.getattr("key")?.extract()?;
        let uid: String = tag.getattr("uid")?.extract()?;
        if keys.contains(&key) {
            requested.insert(uid.clone());
        }
        entries.push((key, uid, tag.getattr("visible")?.is_truthy()?));
    }
    let finish = rows.is_empty()
        || before
            .as_ref()
            .is_some_and(|(active, _)| active == &requested);
    let mut changes = [Vec::new(), Vec::new()];
    for (key, uid, visible) in entries {
        let desired = if finish {
            original.get(&uid).copied().unwrap_or(visible)
        } else {
            requested.contains(&uid) || key == "Layer 0"
        };
        if desired != visible {
            original.entry(uid).or_insert(visible);
            changes[usize::from(desired)].push(key);
        }
    }
    if before.is_none() && finish {
        return Ok(());
    }
    let steps = PyList::empty(py);
    let protocol = py.import("built_core")?;
    for (on, tags) in changes.into_iter().enumerate() {
        if !tags.is_empty() {
            steps.append(protocol.call_method1(
                "SetTagStateCommand",
                (tags, PyList::empty(py), on != 0, py.None()),
            )?)?;
        }
    }
    steps.append(Py::new(
        py,
        IsolationStep {
            before,
            after: (!finish).then_some((requested, original)),
        },
    )?)?;
    let cmd = py
        .import("built_core.history")?
        .call_method1("CompoundCommand", (steps,))?;
    py.import("built_render.tree_navigation")?
        .call_method1("execute", (vp, cmd))?;
    Ok(())
}

#[pyfunction]
pub fn choose_merge_tag(win: &Bound<'_, PyAny>, key: &str) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    if !writable(&scene)? || key == "Layer 0" {
        return Ok(());
    }
    let source = scene.call_method1("layer_by_key", (key,))?;
    if source.is_none() {
        return Ok(());
    }
    let layers = py.import("core.layers")?;
    let choices = PyDict::new(py);
    for tag in scene.getattr("layers")?.try_iter()? {
        let tag = tag?;
        if tag.is(&source) {
            continue;
        }
        let key = tag.getattr("key")?;
        let label = crate::host_context::core_module(layers.py())?.getattr("tag_edits")?.call_method1("path_label", (&scene, &key))?;
        let mut label: String = label.extract()?;
        while choices.contains(&label)? {
            label = format!("{label} ({key})");
        }
        choices.set_item(label, tag)?;
    }
    let callback = py.import("functools")?.call_method1(
        "partial",
        (
            wrap_pyfunction!(merge_answer, py)?,
            win,
            &scene,
            key,
            source,
            &choices,
        ),
    )?;
    py.import("built_render.chrome_forms")?.call_method1(
        "ask_choice",
        (
            win,
            "Merge tag",
            "Move all items to",
            choices.keys(),
            0,
            callback,
        ),
    )?;
    Ok(())
}

#[pyfunction]
fn merge_answer(
    win: &Bound<'_, PyAny>,
    scene: &Bound<'_, PyAny>,
    key: &str,
    source: &Bound<'_, PyAny>,
    choices: &Bound<'_, PyDict>,
    answer: Option<String>,
) -> PyResult<()> {
    let Some(answer) = answer else { return Ok(()) };
    let Some(target) = choices.get_item(answer)? else {
        return Ok(());
    };
    let vp = win.getattr("viewport")?;
    if !vp.getattr("scene")?.is(scene)
        || !writable(scene)?
        || !scene.call_method1("layer_by_key", (key,))?.is(source)
    {
        return Ok(());
    }
    let into = target.getattr("key")?;
    if !scene.call_method1("layer_by_key", (&into,))?.is(&target) {
        return Ok(());
    }
    let py = win.py();
    let cmd = py
        .import("built_core.history")?
        .call_method1("DeleteLayerCommand", (key, into))?;
    py.import("built_render.tree_navigation")?
        .call_method1("execute", (vp, cmd))?;
    Ok(())
}

#[pyfunction]
pub fn has_tag_actions(scene: &Bound<'_, PyAny>, entities: &Bound<'_, PyAny>) -> PyResult<bool> {
    Ok(writable(scene)?
        && entities.len()? > 0
        && scene
            .py()
            .import("core.layers")?
            .call_method1("taggable", (entities,))?
            .len()?
            > 0
        && super::tag_controls::editable_tag_selection(scene)?)
}

/// Keep the viewport menu to one row. The choice form shows the current tag
/// and the commands on the tags carried by the selection.
#[pyfunction]
pub fn viewport_tag_menu(win: &Bound<'_, PyAny>, selection: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let scene = win.getattr("viewport")?.getattr("scene")?;
    if !writable(&scene)? || !super::tag_controls::editable_tag_selection(&scene)? {
        return Ok(());
    }
    let targets = py.import("builtins")?.call_method1("set", (selection,))?;
    if !targets.eq(scene.getattr("selection")?)? {
        return Ok(());
    }
    let layers = py.import("core.layers")?;
    let current: BTreeSet<String> = layers
        .call_method1("selection_layers", (&scene,))?
        .extract()?;
    if current.is_empty() {
        return Ok(());
    }
    let options = PyDict::new(py);
    let mut start = 0;
    for layer in scene.getattr("layers")?.try_iter()? {
        let layer = layer?;
        let key: String = layer.getattr("key")?.extract()?;
        let label: String = crate::host_context::core_module(layers.py())?.getattr("tag_edits")?
            .call_method1("path_label", (&scene, &key))?
            .extract()?;
        if current.len() == 1 && current.contains(&key) {
            start = options.len();
        }
        let mut label = format!("Assign to {label}");
        while options.contains(&label)? {
            label = format!("{label} ({key})");
        }
        options.set_item(label, ("assign", key))?;
    }
    options.set_item("New tag…", ("new", ""))?;
    options.set_item("Hide selected tags", ("hide", ""))?;
    options.set_item("Isolate selected tags", ("isolate", ""))?;
    if current.len() == 1 {
        options.set_item("Make active", ("active", ""))?;
    }
    let revision: u64 = scene.getattr("content_version")?.extract()?;
    let done = py.import("functools")?.call_method1(
        "partial",
        (
            wrap_pyfunction!(viewport_tag_answer, py)?,
            win,
            &scene,
            targets,
            revision,
            current,
            &options,
        ),
    )?;
    py.import("built_render.chrome_forms")?.call_method1(
        "ask_choice",
        (win, "Selection tags", "Tag", options.keys(), start, done),
    )?;
    Ok(())
}

#[pyfunction]
fn viewport_tag_answer(
    win: &Bound<'_, PyAny>,
    scene: &Bound<'_, PyAny>,
    targets: &Bound<'_, PyAny>,
    revision: u64,
    current: BTreeSet<String>,
    options: &Bound<'_, PyDict>,
    answer: Option<String>,
) -> PyResult<()> {
    let Some(label) = answer else { return Ok(()) };
    let Some(choice) = options.get_item(label)? else {
        return Ok(());
    };
    let (verb, key): (String, String) = choice.extract()?;
    let vp = win.getattr("viewport")?;
    if !vp.getattr("scene")?.is(scene)
        || !writable(scene)?
        || scene.getattr("content_version")?.extract::<u64>()? != revision
        || !targets.eq(scene.getattr("selection")?)?
    {
        return Ok(());
    }
    let py = win.py();
    match verb.as_str() {
        "assign" => crate::context_edits::assign_tag(win, &key)?,
        "new" => {
            let callback = py.import("functools")?.call_method1(
                "partial",
                (
                    wrap_pyfunction!(assign_new_answer, py)?,
                    win,
                    scene,
                    targets,
                    revision,
                ),
            )?;
            py.import("built_render.chrome_forms")?.call_method1(
                "ask_text",
                (
                    win,
                    "New tag",
                    "Tag name",
                    crate::context_edits::next_tag(scene)?,
                    callback,
                ),
            )?;
        }
        "isolate" => isolate_tags(
            &vp,
            current.iter().map(|k| format!("preview-tag:{k}")).collect(),
        )?,
        "hide" => {
            let tags: Vec<_> = current.into_iter().collect();
            py.import("built_core.command_protocol")?.call_method1(
                "set_tag_state",
                (vp.getattr("history")?, tags, PyList::empty(py), false),
            )?;
        }
        "active" => {
            crate::host_context::core_module(py)?.getattr("tag_edits")?
                .call_method1("set_active", (scene, current.first()))?;
            scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)?;
        }
        _ => return Ok(()),
    }
    vp.call_method0("notify_scene_changed")?;
    Ok(())
}

#[pyfunction]
fn assign_new_answer(
    win: &Bound<'_, PyAny>,
    scene: &Bound<'_, PyAny>,
    targets: &Bound<'_, PyAny>,
    revision: u64,
    answer: Option<String>,
) -> PyResult<()> {
    let Some(name) = answer.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(());
    };
    if win.getattr("viewport")?.getattr("scene")?.is(scene)
        && writable(scene)?
        && scene.getattr("content_version")?.extract::<u64>()? == revision
        && targets.eq(scene.getattr("selection")?)?
    {
        crate::context_edits::assign_tag(win, name)?;
    }
    Ok(())
}
