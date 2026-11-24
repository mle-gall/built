// SPDX-License-Identifier: GPL-3.0-or-later
use super::{translated, Occurrences, READ_ONLY};
use pyo3::{prelude::*, types::PyList};
use std::collections::HashSet;

/// A tag describes top-level placements regardless of which filing node owns them.
#[pyfunction]
pub fn tree_tag_action(win: &Bound<'_, PyAny>, tag: &str, isolate: bool) -> PyResult<()> {
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    if scene.call_method1("layer_by_key", (tag,))?.is_none() {
        return Ok(());
    }
    let ui = win.py().import("views.wgpu_ui")?;
    if !isolate {
        win.py()
            .import("views.tree_navigation")?
            .call_method1("open_to", (&vp, PyList::empty(win.py())))?;
        let entities = win
            .py()
            .import("core.select_ops")?
            .call_method1("on_layers", (&scene, [tag]))?;
        ui.call_method1("_select", (win, entities))?;
        return Ok(());
    }
    let matches = PyList::empty(win.py());
    for group in scene.getattr("groups")?.try_iter()? {
        let group = group?;
        let layer: Option<String> = group.getattr("layer")?.extract()?;
        if layer.as_deref().unwrap_or("Layer 0") == tag {
            matches.append(group)?;
        }
    }
    ui.call_method1("_isolate", (win, matches))?;
    Ok(())
}

/// Delete the addressed occurrence or section, including the other picked rows
/// when the addressed one is selected. Protect both sides of a locked ancestor.
#[pyfunction]
pub fn tree_delete_rows(vp: &Bound<'_, PyAny>, clicked: &Bound<'_, PyAny>) -> PyResult<()> {
    if clicked.is_none() {
        return Ok(());
    }
    let py = vp.py();
    let scene = vp.getattr("scene")?;
    if scene.hasattr("worksession_read_only")?
        && scene.getattr("worksession_read_only")?.is_truthy()?
    {
        vp.call_method1("flash_status", (translated(py, READ_ONLY, None)?, 4000))?;
        return Ok(());
    }
    let selection = scene.getattr("selection")?;
    let planes = scene.getattr("section_planes")?;
    let plane = planes.contains(clicked)?;
    let multi = !clicked.is_instance_of::<PyList>() && selection.contains(clicked)?;
    let tree = if plane && multi {
        let picked = py.import("builtins")?.call_method1("list", (&selection,))?;
        Occurrences::read(&scene, &picked)?
    } else {
        Occurrences::read(&scene, clicked)?
    };
    if tree.targets.is_empty() && !plane {
        return Ok(());
    }
    let mut locked = vec![false; tree.rows.len()];
    for (i, row) in tree.rows.iter().enumerate().rev() {
        locked[i] |= row.group.getattr("locked")?.is_truthy()?;
        if let Some(parent) = row.parent {
            locked[parent] |= locked[i];
        }
    }
    let history = py.import("core.history")?;
    let steps = PyList::empty(py);
    let picked: HashSet<_> = tree.targets.iter().copied().collect();
    let mut reason = "";
    for &i in &tree.targets {
        if std::iter::successors(tree.rows[i].parent, |&j| tree.rows[j].parent)
            .any(|j| picked.contains(&j))
        {
            continue;
        }
        let why = tree.refusal(&scene, &[i], false)?;
        if locked[i] || !why.is_empty() {
            reason = if locked[i] {
                "Locked rows are not deleted: unlock them first"
            } else {
                why
            };
            continue;
        }
        steps.append(history.call_method1("DeleteGroupCommand", (&tree.rows[i].group,))?)?;
    }
    let remove = PyList::empty(py);
    for entry in planes.try_iter()? {
        let entry = entry?;
        if entry.is(clicked) || (multi && selection.contains(&entry)?) {
            remove.append(entry)?;
        }
    }
    if !remove.is_empty() {
        steps.append(history.call_method1("DeleteSectionPlanesCommand", (remove,))?)?;
    }
    if !reason.is_empty() {
        vp.call_method1("flash_status", (translated(py, reason, None)?, 3500))?;
    }
    if !steps.is_empty() {
        vp.call_method0("end_group_edit")?;
        py.import("views.tree_navigation")?.call_method1(
            "execute",
            (vp, history.call_method1("CompoundCommand", (steps,))?),
        )?;
    }
    Ok(())
}

#[pyfunction]
pub fn tree_delete_tag(win: &Bound<'_, PyAny>, key: &str) -> PyResult<()> {
    let py = win.py();
    let scene = win.getattr("viewport")?.getattr("scene")?;
    let tag = scene.call_method1("layer_by_key", (key,))?;
    if tag.is_none() || key == "Layer 0" {
        return Ok(());
    }
    if scene.hasattr("worksession_read_only")?
        && scene.getattr("worksession_read_only")?.is_truthy()?
    {
        win.getattr("viewport")?
            .call_method1("flash_status", (translated(py, READ_ONLY, None)?, 4000))?;
        return Ok(());
    }
    let counts = py
        .import("core.purge")?
        .call_method1("layer_counts", (&scene,))?;
    let count: usize = counts.call_method1("get", (key, 0))?.extract()?;
    let revision = scene.getattr("content_version")?.extract()?;
    if count == 0 {
        return delete_tag_answer(win, &scene, &tag, key, revision, "delete");
    }
    let vars = pyo3::types::PyDict::new(py);
    vars.set_item("layer", tag.getattr("name")?)?;
    vars.set_item("n", count)?;
    vars.set_item("default", "Untagged")?;
    let message = translated(
        py,
        "Delete the layer '{layer}'? Its {n} entities go to '{default}'.",
        Some(&vars),
    )?;
    let callback = py.import("functools")?.call_method1(
        "partial",
        (
            wrap_pyfunction!(delete_tag_answer, py)?,
            win,
            &scene,
            &tag,
            key,
            revision,
        ),
    )?;
    confirm_tag_removal(win, message.extract()?, callback)
}

fn confirm_tag_removal(
    win: &Bound<'_, PyAny>,
    message: String,
    callback: Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = win.py();
    let kw = pyo3::types::PyDict::new(py);
    kw.set_item("cancel", "cancel")?;
    kw.set_item("default", "cancel")?;
    let buttons = [
        ("cancel", translated(py, "Cancel", None)?),
        ("delete", translated(py, "Delete", None)?),
    ];
    py.import("views.chrome_dialogs")?
        .getattr("ask_later")?
        .call(
            (
                win,
                translated(py, "Layers", None)?,
                message,
                buttons,
                callback,
            ),
            Some(&kw),
        )?;
    Ok(())
}

#[pyfunction]
fn delete_tag_answer(
    win: &Bound<'_, PyAny>,
    scene: &Bound<'_, PyAny>,
    tag: &Bound<'_, PyAny>,
    key: &str,
    revision: u64,
    answer: &str,
) -> PyResult<()> {
    let vp = win.getattr("viewport")?;
    if answer == "delete"
        && vp.getattr("scene")?.is(scene)
        && scene.getattr("content_version")?.extract::<u64>()? == revision
        && !(scene.hasattr("worksession_read_only")?
            && scene.getattr("worksession_read_only")?.is_truthy()?)
        && scene.call_method1("layer_by_key", (key,))?.is(tag)
    {
        let cmd = win
            .py()
            .import("core.history")?
            .call_method1("DeleteLayerCommand", (key,))?;
        win.py()
            .import("views.tree_navigation")?
            .call_method1("execute", (vp, cmd))?;
    }
    Ok(())
}

/// Apply header switches to the complete document, even when the tree is filtered.
#[pyfunction]
pub fn tree_header_state(vp: &Bound<'_, PyAny>, scope: &str, lock: bool) -> PyResult<()> {
    let py = vp.py();
    let scene = vp.getattr("scene")?;
    if !writable(&scene)? {
        vp.call_method1("flash_status", (translated(py, READ_ONLY, None)?, 4000))?;
        return Ok(());
    }
    let (items, field) = match (scope, lock) {
        ("placements", true) => ("groups", "locked"),
        ("placements", false) => ("groups", "hidden"),
        ("tags", true) => ("layers", "locked"),
        ("tags", false) => ("layers", "visible"),
        _ => return Ok(()),
    };
    let rows = scene.getattr(items)?;
    if rows.len()? == 0 {
        return Ok(());
    }
    let tags = scope == "tags";
    let mut all = true;
    let mut any = false;
    let keys = PyList::empty(py);
    for row in rows.try_iter()? {
        let row = row?;
        let state = row.getattr(field)?.is_truthy()?;
        all &= state;
        any |= state;
        if tags {
            keys.append(row.getattr("key")?)?;
        }
    }
    if tags {
        let kw = pyo3::types::PyDict::new(py);
        kw.set_item("tags", keys)?;
        kw.set_item(field, !all)?;
        py.import("core.protocol.commands")?
            .getattr("set_tag_state")?
            .call((vp.getattr("history")?,), Some(&kw))?;
        vp.call_method0("refresh_after_edit")?;
    } else {
        if !any && !scene.getattr("edit_group")?.is_none() {
            vp.call_method0("end_group_edit")?;
        }
        let name = if lock { "LockCommand" } else { "HideCommand" };
        let cmd = py
            .import("core.history")?
            .call_method1(name, (rows, !any))?;
        py.import("views.tree_navigation")?
            .call_method1("execute", (vp, cmd))?;
    }
    Ok(())
}

pub(super) fn writable(scene: &Bound<'_, PyAny>) -> PyResult<bool> {
    Ok(!(scene.hasattr("worksession_read_only")?
        && scene.getattr("worksession_read_only")?.is_truthy()?))
}

#[pyfunction]
pub fn tree_purge_tags(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    if !writable(&scene)? {
        vp.call_method1("flash_status", (translated(py, READ_ONLY, None)?, 4000))?;
        return Ok(());
    }
    let candidates = py
        .import("core.purge")?
        .call_method1("unused_layers", (&scene,))?;
    let count = candidates.len()?;
    if count == 0 {
        vp.call_method1(
            "flash_status",
            (translated(py, "Every layer is in use.", None)?, 3000),
        )?;
        return Ok(());
    }
    let mut preview = Vec::new();
    for entry in candidates.try_iter()?.take(12) {
        preview.push(entry?.getattr("name")?.extract::<String>()?);
    }
    let vars = pyo3::types::PyDict::new(py);
    vars.set_item("n", count)?;
    let question: String =
        translated(py, "Delete {n} layers no entity carries?", Some(&vars))?.extract()?;
    let suffix = if count > preview.len() { "…" } else { "" };
    let message = format!("{question}\n\n{}{suffix}", preview.join(", "));
    let callback = py.import("functools")?.call_method1(
        "partial",
        (
            wrap_pyfunction!(purge_tags_answer, py)?,
            win,
            &scene,
            scene.getattr("content_version")?,
        ),
    )?;
    confirm_tag_removal(win, message, callback)
}

#[pyfunction]
fn purge_tags_answer(
    win: &Bound<'_, PyAny>,
    scene: &Bound<'_, PyAny>,
    revision: u64,
    answer: &str,
) -> PyResult<()> {
    let vp = win.getattr("viewport")?;
    if answer == "delete"
        && vp.getattr("scene")?.is(scene)
        && writable(scene)?
        && scene.getattr("content_version")?.extract::<u64>()? == revision
    {
        let cmd = win
            .py()
            .import("core.history")?
            .call_method1("PurgeUnusedCommand", (true, false))?;
        win.py()
            .import("views.tree_navigation")?
            .call_method1("execute", (vp, cmd))?;
    }
    Ok(())
}
