// SPDX-License-Identifier: GPL-3.0-or-later
//! Named snapshots of the eye switches in Project, independent of the camera.
//! Link contents belong to the source. Its drawing tags have host overrides.
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList},
};
use std::collections::HashMap;

type Obj<'py> = Bound<'py, PyAny>;
const STORE: &str = "built.visibility_sets";

fn records<'py>(scene: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let mut empty = PyDict::new(scene.py());
    if let Some(data) = scene.getattr_opt("plugin_data")? {
        if let Some(stored) = data.cast::<PyDict>()?.get_item(STORE)? {
            empty = stored.cast_into::<PyDict>()?.copy()?;
        }
    }
    Ok(empty)
}

/// Only local occurrences and link roots have writable eye switches.
fn placements<'py>(scene: &Obj<'py>) -> PyResult<Vec<(Vec<String>, Obj<'py>)>> {
    let mut out = Vec::new();
    let mut stack = Vec::new();
    for g in scene.getattr("groups")?.try_iter()? {
        stack.push((Vec::new(), g?));
    }
    while let Some((mut path, g)) = stack.pop() {
        path.push(g.getattr("uid")?.extract()?);
        if !crate::model::links::is_linked(&g)? {
            for c in g.getattr("children")?.try_iter()? {
                stack.push((path.clone(), c?));
            }
        }
        out.push((path, g));
    }
    Ok(out)
}

fn capture<'py>(scene: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let py = scene.py();
    let entry = PyDict::new(py);
    let tags = PyDict::new(py);
    for t in scene.getattr("layers")?.try_iter()? {
        let t = t?;
        tags.set_item(t.getattr("uid")?, t.getattr("visible")?)?;
    }
    entry.set_item("tags", tags)?;
    let folders = PyDict::new(py);
    let visibility = scene.getattr("folder_visibility")?;
    for f in py
        .import("core.layers")?
        .call_method1("folders_of", (scene,))?
        .try_iter()?
    {
        let f = f?;
        folders.set_item(&f, visibility.call_method1("get", (&f, true))?)?;
    }
    entry.set_item("folders", folders)?;
    let objects = PyList::empty(py);
    for (path, group) in placements(scene)? {
        let item = PyList::empty(py);
        item.append(path)?;
        item.append(group.getattr("hidden")?)?;
        objects.append(item)?;
    }
    entry.set_item("objects", objects)?;
    Ok(entry)
}

fn recall<'py>(scene: &Obj<'py>, data: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    let py = scene.py();
    let history = py.import("built_core.history")?;
    let switches = py
        .import("built_core")?
        .getattr("SetTagStateCommand")?;
    let mut commands = Vec::new();
    let tag_states: HashMap<String, bool> = data.get_item("tags")?.extract()?;
    let folder_states: HashMap<String, bool> = data.get_item("folders")?.extract()?;
    let mut tags = [Vec::new(), Vec::new()];
    let mut folders = [Vec::new(), Vec::new()];
    for t in scene.getattr("layers")?.try_iter()? {
        let t = t?;
        let uid: String = t.getattr("uid")?.extract()?;
        if let Some(&on) = tag_states.get(&uid) {
            if t.getattr("visible")?.is_truthy()? != on {
                tags[on as usize].push(t.getattr("key")?);
            }
        }
    }
    let visibility = scene.getattr("folder_visibility")?;
    for f in py
        .import("core.layers")?
        .call_method1("folders_of", (scene,))?
        .try_iter()?
    {
        let f = f?;
        let key: String = f.extract()?;
        if let Some(&on) = folder_states.get(&key) {
            if visibility.call_method1("get", (&f, true))?.is_truthy()? != on {
                folders[on as usize].push(f);
            }
        }
    }
    for ((ts, fs), on) in tags.into_iter().zip(folders).zip([false, true]) {
        if !ts.is_empty() || !fs.is_empty() {
            commands.push(switches.call1((ts, fs, on, py.None()))?);
        }
    }
    let mut by_path = HashMap::<Vec<String>, bool>::new();
    for item in data.get_item("objects")?.try_iter()? {
        let item = item?;
        by_path.insert(item.get_item(0)?.extract()?, item.get_item(1)?.extract()?);
    }
    let mut changes = [Vec::new(), Vec::new()];
    for (path, group) in placements(scene)? {
        if let Some(&hidden) = by_path.get(&path) {
            if group.getattr("hidden")?.is_truthy()? != hidden {
                changes[hidden as usize].push(group);
            }
        }
    }
    for (targets, hidden) in changes.into_iter().zip([false, true]) {
        if !targets.is_empty() {
            commands.push(history.call_method1("HideCommand", (targets, hidden))?);
        }
    }
    if commands.is_empty() {
        return Ok(None);
    }
    Ok(Some(history.call_method1("CompoundCommand", (commands,))?))
}

#[pyfunction]
pub(crate) fn visibility_set_rows(
    scene: &Obj<'_>,
) -> PyResult<Vec<(String, String, String, String, i8, bool)>> {
    let mut rows = Vec::new();
    for (id, r) in records(scene)?.iter() {
        rows.push((
            format!("visibility:apply:{}", id.str()?),
            r.get_item("name")?.extract()?,
            String::new(),
            String::new(),
            -1,
            true,
        ));
    }
    Ok(rows)
}

#[pyfunction]
pub(crate) fn visibility_set_command<'py>(
    scene: &Obj<'py>,
    verb: &str,
    id: &str,
    name: &str,
) -> PyResult<Option<Obj<'py>>> {
    let py = scene.py();
    let records = records(scene)?;
    let existing = records.get_item(id)?;
    if verb != "save" && existing.is_none() {
        return Ok(None);
    }
    let name = name.trim();
    if matches!(verb, "save" | "rename") {
        if name.is_empty() {
            return Err(PyValueError::new_err("Give the visibility set a name"));
        }
        for (key, r) in records.iter() {
            if r.get_item("name")?.extract::<String>()? == name {
                if verb == "rename" && key.extract::<String>()? == id {
                    return Ok(None);
                }
                return Err(PyValueError::new_err(
                    "Another visibility set already has that name",
                ));
            }
        }
    }
    match (verb, existing) {
        ("save", _) => {
            let snapshot = capture(scene)?;
            snapshot.set_item("name", name)?;
            records.set_item(py.import("core.group")?.call_method0("new_uid")?, snapshot)?;
        }
        ("update", Some(r)) => {
            let snapshot = capture(scene)?;
            snapshot.set_item("name", r.get_item("name")?)?;
            if snapshot.eq(&r)? {
                return Ok(None);
            }
            records.set_item(id, snapshot)?;
        }
        ("rename", Some(r)) => {
            let record = r.cast::<PyDict>()?.copy()?;
            record.set_item("name", name)?;
            records.set_item(id, record)?;
        }
        ("delete", Some(_)) => records.del_item(id)?,
        ("apply", Some(r)) => return recall(scene, &r),
        _ => return Err(PyValueError::new_err("Unknown visibility set command")),
    }
    Ok(Some(py.import("built_core.history")?.call_method1(
        "SetPluginDataCommand",
        (
            STORE,
            if records.is_empty() {
                None
            } else {
                Some(records)
            },
        ),
    )?))
}
