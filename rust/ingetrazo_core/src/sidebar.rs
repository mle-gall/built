// SPDX-License-Identifier: GPL-3.0-or-later
//! Filing and containment are independent addresses in the Project dock.
use crate::project_tree::{project_context_rows, Row};
use pyo3::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::Path;

type PathBar = Vec<(String, String)>;

fn context_keys(group: &Bound<'_, PyAny>, tree: &Bound<'_, PyAny>) -> PyResult<HashSet<String>> {
    let member = tree.getattr("member")?;
    let nodes = tree.getattr("nodes")?;
    let mut inside = HashSet::new();
    let mut stack = vec![group.clone()];
    while let Some(g) = stack.pop() {
        let id: String = tree.call_method1("row_id", (&g,))?.extract()?;
        for prefix in ["i:", "g:", "c:"] {
            inside.insert(format!("{prefix}{id}"));
        }
        if let Some(tag) = g.getattr("layer")?.extract::<Option<String>>()? {
            inside.insert(format!("preview-tag:{tag}"));
        }
        let mut key: Option<String> = member
            .call_method1("get", (g.as_ptr() as usize,))?
            .extract()?;
        if key.is_none() {
            inside.insert("n:".into());
        }
        while let Some(k) = key {
            if !inside.insert(format!("n:{k}")) {
                break;
            }
            key = nodes.get_item(k)?.get_item("parent")?.extract()?;
        }
        for c in g.getattr("children")?.try_iter()? {
            stack.push(c?);
        }
    }
    Ok(inside)
}

// Families are stored as [angle, x, y, shift, spacing, dashes] in a .built file.
fn imported_families(scene: &Bound<'_, PyAny>, name: &str) -> Option<String> {
    let entries = scene
        .getattr("hatch_patterns")
        .ok()?
        .get_item(name)
        .ok()?
        .get_item("families")
        .ok()?;
    let families: Vec<_> = entries
        .try_iter()
        .ok()?
        .take(64)
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let number = |i| {
                entry
                    .get_item(i)
                    .ok()?
                    .extract::<f64>()
                    .ok()
                    .filter(|v| v.is_finite())
            };
            let dashes: Vec<f64> = entry
                .get_item(5)
                .ok()?
                .try_iter()
                .ok()?
                .take(32)
                .map(|v| v.ok()?.extract::<f64>().ok().filter(|v| v.is_finite()))
                .collect::<Option<_>>()?;
            Some((
                number(0)?,
                number(1)?,
                number(2)?,
                number(3)?,
                number(4)?,
                dashes,
            ))
        })
        .collect();
    serde_json::to_string(&families).ok()
}

#[pyfunction(signature = (scene, tree, rows, query, expanded, filename, drawing_lens=false, browser_selected=None))]
pub(crate) fn project_sidebar(
    scene: &Bound<'_, PyAny>,
    tree: &Bound<'_, PyAny>,
    mut rows: Vec<Row>,
    query: &str,
    expanded: HashSet<String>,
    filename: &str,
    drawing_lens: bool,
    browser_selected: Option<HashSet<usize>>,
) -> PyResult<(Vec<Row>, PathBar)> {
    let group = scene.getattr("edit_group")?;
    let tags_only = tree.getattr("_host_roots")?.extract::<i64>()? < 0;
    if tags_only {
        let root = scene.getattr("_root")?;
        let mut counts = HashMap::<String, i64>::new();
        for g in tree.getattr("unfiled")?.try_iter()? {
            let tag = g?
                .getattr("layer")?
                .extract::<Option<String>>()?
                .unwrap_or_else(|| "Layer 0".into());
            *counts.entry(tag).or_default() += 1;
        }
        let selected: HashSet<String> = if drawing_lens {
            scene.py().import("core.layers")?.call_method1("selection_layers", (scene,))?.extract()?
        } else {
            HashSet::new()
        };
        let wants = |predicate: &str| query.split_whitespace()
            .any(|word| word.trim_start_matches('-').eq_ignore_ascii_case(predicate));
        let mut used: HashSet<String> = HashSet::new();
        let mut kept: HashSet<String> = HashSet::new();
        if wants("is:used") || wants("is:unused") {
            let purge = scene.py().import("core.purge")?;
            used = purge.call_method1("used_layers", (scene,))?.extract()?;
            if wants("is:unused") {
                kept = purge.call_method1("layers_held_by_scenes", (scene,))?.extract()?;
            }
        }
        let mut tags: Vec<Row> = root
            .call_method1(
                "_drawing_preview",
                (expanded.clone(), query, tree.getattr("links")?.is_truthy()?, used, kept, selected.clone()),
            )?
            .extract()?;
        let mut folder_selected = HashMap::<String, i64>::new();
        let parents = scene.getattr("folder_parents")?;
        let mut folder_counts = HashMap::<String, i64>::new();
        for layer in scene.getattr("layers")?.try_iter()? {
            let layer = layer?;
            let key: String = layer.getattr("key")?.extract()?;
            let count = counts.get(&key).copied().unwrap_or_default();
            let mut folder = layer.getattr("folder")?.extract::<Option<String>>()?;
            let mut seen = HashSet::new();
            while let Some(name) = folder.filter(|f| !f.is_empty()) {
                if !seen.insert(name.clone()) {
                    break;
                }
                *folder_counts.entry(name.clone()).or_default() += count;
                if selected.contains(&key) {
                    *folder_selected.entry(name.clone()).or_default() += 1;
                }
                folder = parents.call_method1("get", (name,))?.extract()?;
            }
        }
        for (text, flags) in &mut tags {
            if let Some(tag) = text[0].strip_prefix("preview-tag:") {
                flags[1] = counts.get(tag).copied().unwrap_or_default();
                flags[9] = i64::from(selected.contains(tag));
            } else if let Some(folder) = text[0].strip_prefix("preview-folder:") {
                flags[1] = folder_counts.get(folder).copied().unwrap_or_default();
                flags[9] = folder_selected.get(folder).copied().unwrap_or_default();
            }
        }
        // Tagged placements are represented by their tag. Keep the original
        // unfiled branch only in documents with an authored filing structure.
        if let Some(index) = rows.iter().position(|(t, _)| t[0] == "n:") {
            if query.is_empty() && !expanded.contains("n:") {
                rows.truncate(index);
            } else {
                rows.remove(index);
                for (_, flags) in &mut rows[index..] {
                    flags[0] = (flags[0] - 1).max(0);
                }
            }
        }
        tags.append(&mut rows);
        rows = tags;
    }
    // Imported PAT definitions belong to the document, while the angle and
    // scale live on each tag. Supply the families to both kinds of Drawing row.
    for (text, _) in &mut rows {
        if let Some(name) = text.get(9).and_then(|hatch| hatch.strip_prefix("pat:")) {
            if let Some(families) = imported_families(scene, name) {
                if text.len() > 15 { text[15] = families; } else { text.push(families); }
            }
        }
    }
    let mut path = Vec::new();
    let mut context = project_context_rows(scene, query, expanded.clone(), browser_selected)?;
    if group.is_none() {
        rows.append(&mut context);
    } else {
        let file = Path::new(filename)
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("Untitled");
        path.push((file.into(), "documentroot\x1f".into()));
        for g in scene.getattr("_root")?.getattr("_edit_path")?.try_iter()? {
            let g = g?;
            let id: String = tree.call_method1("row_id", (&g,))?.extract()?;
            path.push((g.getattr("name")?.extract()?, format!("open\x1fi:{id}")));
        }
        // The filing projection is revision-scoped. Reuse the subtree walk
        // across filter changes, selection changes and disclosure clicks.
        let cached = tree.getattr("_sidebar_scope").ok();
        let inside =
            if let Some(entry) = cached.filter(|v| v.get_item(0).is_ok_and(|g| g.is(&group))) {
                entry.get_item(1)?
            } else {
                let keys = context_keys(&group, tree)?.into_pyobject(tree.py())?;
                tree.setattr("_sidebar_scope", (&group, &keys))?;
                keys.into_any()
            };

        for (t, f) in &mut rows {
            f.resize(12, 0);
            f[11] = i64::from(!inside.contains(&t[0])?);
        }
        context.append(&mut rows);
        rows = context;
    }
    Ok((rows, path))
}

/// Limit the normal bootstrap plan to one local tag and its folder chain.
/// Existing filing nodes must never be replaced by this tags-only command.
#[pyfunction]
pub(crate) fn promote_tag<'py>(
    scene: &Bound<'py, PyAny>,
    tag: &str,
    class: &str,
) -> PyResult<Bound<'py, PyAny>> {
    use pyo3::{exceptions::PyValueError, types::PyDict};
    let py = scene.py();
    let api = py.import("core.structure")?;
    let registry = api.getattr("bim_registry")?.call_method1("get", (scene,))?;
    if api.call_method1("_nodes", (&registry,))?.is_truthy()?
        || !py
            .import("core.ifc_tables")?
            .getattr("TABLES")?
            .contains(class)?
    {
        return Err(PyValueError::new_err(
            "Choose a kind for a tag in an unstructured document",
        ));
    }
    let layer = scene.call_method1("layer_by_key", (tag,))?;
    if layer.is_none() || tag == "Layer 0" {
        return Err(PyValueError::new_err("Choose a named tag"));
    }
    let key = format!("layer:{}", layer.getattr("uid")?.extract::<String>()?);
    let proposal = crate::structure_bootstrap::structure_proposal(scene, &registry)?;
    let structure = proposal.as_any().get_item("structure")?;
    let nodes = structure.get_item("nodes")?;
    let leaf = nodes.get_item(&key)?;
    if leaf.call_method1("get", ("source",))?.is_truthy()? {
        return Err(PyValueError::new_err(
            "Edit the linked source to change its filing",
        ));
    }
    let recipe = PyDict::new(py);
    recipe.set_item("tag", tag)?;
    recipe.set_item("class", class)?;
    leaf.set_item("recipe", recipe)?;
    let kept = PyDict::new(py);
    let mut parent = Some(key.clone());
    while let Some(k) = parent {
        let n = nodes.get_item(&k)?;
        kept.set_item(k, &n)?;
        parent = n.get_item("parent")?.extract()?;
    }
    structure.set_item("nodes", kept)?;
    let filing = PyDict::new(py);
    if let Some(gs) = proposal
        .as_any()
        .get_item("filing")?
        .cast::<PyDict>()?
        .get_item(&key)?
    {
        filing.set_item(key, gs)?;
    }
    proposal.set_item("filing", filing)?;
    crate::structure::apply_structure_bootstrap(scene, proposal.as_any())
}
