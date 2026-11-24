// SPDX-License-Identifier: GPL-3.0-or-later
//! Drawing tag commands shared by the row menu and the selection inspector.
use super::{translated, READ_ONLY};
use pyo3::{
    prelude::*,
    types::{PyBool, PyDict, PySet},
};
use std::collections::{HashMap, HashSet};

const NEW_HIDDEN: &str = "layers/new_hidden_in_scenes";

pub fn new_hidden(py: Python<'_>) -> PyResult<bool> {
    py.import("core.settings")?
        .call_method0("settings")?
        .call_method1("value", (NEW_HIDDEN, false, py.get_type::<PyBool>()))?
        .extract()
}

#[pyfunction]
pub fn set_new_tags_hidden(py: Python<'_>, on: bool) -> PyResult<()> {
    py.import("core.settings")?
        .call_method0("settings")?
        .call_method1("setValue", (NEW_HIDDEN, on))?;
    Ok(())
}

fn editable_tag_selection(scene: &Bound<'_, PyAny>) -> PyResult<bool> {
    let py = scene.py();
    let group = py.import("core.group")?.getattr("Group")?;
    let links = py.import("core.links")?;
    let filing = py.import("core.structure")?;
    for entity in scene.getattr("selection")?.try_iter()? {
        let entity = entity?;
        if entity.is_instance(&group)?
            && (filing.call_method1("node_of", (&entity,))?.is_truthy()?
                || !links.call_method1("linked_ancestor", (&entity,))?.is_none())
        {
            return Ok(false);
        }
    }
    let edit = scene.getattr("edit_group")?;
    Ok(edit.is_none() || links.call_method1("linked_ancestor", (edit,))?.is_none())
}

/// A mixed selection starts at a placeholder. Folder paths distinguish duplicate labels.
#[pyfunction]
pub fn inspector_tag_row(scene: &Bound<'_, PyAny>) -> PyResult<Vec<(String, String)>> {
    if !super::row_actions::writable(scene)? || !editable_tag_selection(scene)? {
        return Ok(vec![]);
    }
    let layers = scene.py().import("core.layers")?;
    let picked = layers.call_method1("taggable", (scene.getattr("selection")?,))?;
    if picked.len()? == 0 {
        return Ok(vec![]);
    }
    let mut current = HashSet::<String>::new();
    for entity in picked.try_iter()? {
        current.insert(layers.call_method1("layer_of", (entity?,))?.extract()?);
        if current.len() > 1 {
            break;
        }
    }
    let mut options = vec![];
    let mut at = 0;
    if current.len() > 1 {
        options.push((String::new(), "(several)".to_string()));
    }
    let labels: HashMap<String, String> = scene.getattr("layer_folder_names")?.extract()?;
    let parents: HashMap<String, Option<String>> = scene.getattr("folder_parents")?.extract()?;
    let mut paths = HashMap::new();
    for tag in scene.getattr("layers")?.try_iter()? {
        let tag = tag?;
        let key: String = tag.getattr("key")?.extract()?;
        let name: String = tag.getattr("name")?.extract()?;
        if current.len() == 1 && current.contains(&key) {
            at = options.len();
        }
        let folder: Option<String> = tag.getattr("folder")?.extract()?;
        let label = if let Some(folder) = folder.filter(|f| !f.is_empty() && key != "Layer 0") {
            let path = paths.entry(folder.clone()).or_insert_with(|| {
                let mut at = Some(&folder);
                let mut seen = HashSet::new();
                let mut parts = Vec::new();
                while let Some(key) = at {
                    if !seen.insert(key) {
                        break;
                    }
                    parts.push(labels.get(key).unwrap_or(key).as_str());
                    at = parents.get(key).and_then(Option::as_ref);
                }
                parts.reverse();
                parts.join("/")
            });
            format!("{path}/{name}")
        } else {
            name
        };
        options.push((key, label));
    }
    let value = serde_json::json!([at, options]);
    Ok(vec![(
        "Tag".into(),
        format!("\u{1d}tagpicker\u{1d}{value}"),
    )])
}

#[pyfunction]
pub fn tree_edit_tag(
    win: &Bound<'_, PyAny>,
    verb: &str,
    row: &str,
    name: &str,
    expanded: &Bound<'_, PySet>,
) -> PyResult<Option<String>> {
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let py = win.py();
    if !super::row_actions::writable(&scene)? {
        vp.call_method1("flash_status", (translated(py, READ_ONLY, None)?, 4000))?;
        return Ok(None);
    }
    let layers = py.import("core.layers")?;
    let key = row.strip_prefix("preview-tag:");
    let tag = scene.call_method1("layer_by_key", (key.unwrap_or(""),))?;
    if key.is_some() && tag.is_none() {
        return Ok(None);
    }
    let mut created = None;
    match verb {
        "newtag" => {
            let folder = if let Some(key) = row.strip_prefix("preview-folder:") {
                if !layers
                    .call_method1("folders_of", (&scene,))?
                    .contains(key)?
                {
                    return Ok(None);
                }
                Some(key.to_owned())
            } else if !tag.is_none() {
                tag.getattr("folder")?.extract::<Option<String>>()?
            } else if row.is_empty() {
                None
            } else {
                return Ok(None);
            };
            let label = crate::context_edits::next_tag(&scene)?;
            let kw = PyDict::new(py);
            kw.set_item("folder", &folder)?;
            let new = layers.getattr("Layer")?.call((&label,), Some(&kw))?;
            scene.getattr("layers")?.call_method1("append", (new,))?;
            if new_hidden(py)? {
                layers.call_method1("hide_in_scenes", (&scene, [&label], true))?;
            }
            let mut next = folder;
            let mut seen = HashSet::new();
            while let Some(key) = next {
                if !seen.insert(key.clone()) {
                    break;
                }
                expanded.add(format!("preview-folder:{key}"))?;
                next = scene
                    .getattr("folder_parents")?
                    .call_method1("get", (key,))?
                    .extract()?;
            }
            created = Some(label);
        }
        "rename" if !tag.is_none() => {
            if !layers
                .call_method1("rename_layer", (&scene, key, name.trim()))?
                .is_truthy()?
            {
                return Ok(None);
            }
        }
        "tagactive" if !tag.is_none() => {
            layers.call_method1("set_active", (&scene, key))?;
        }
        "tagassign" if !tag.is_none() => {
            let targets = layers.call_method1("taggable", (scene.getattr("selection")?,))?;
            if targets.len()? == 0 {
                vp.call_method1("flash_status", ("Select the objects to move to this tag",))?;
            } else if !editable_tag_selection(&scene)? {
                vp.call_method1(
                    "flash_status",
                    ("Use Filing to change the tag of a filed or linked item",),
                )?;
            } else {
                crate::context_edits::assign_tag(win, &tag.getattr("key")?.extract::<String>()?)?;
                vp.call_method0("notify_scene_changed")?;
            }
            return Ok(None);
        }
        _ => return Ok(None),
    }
    scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)?;
    vp.call_method0("notify_scene_changed")?;
    Ok(created)
}
