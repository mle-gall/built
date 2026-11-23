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

#[pyfunction]
pub(crate) fn project_sidebar(
    scene: &Bound<'_, PyAny>,
    tree: &Bound<'_, PyAny>,
    mut rows: Vec<Row>,
    query: &str,
    expanded: HashSet<String>,
    filename: &str,
) -> PyResult<(Vec<Row>, PathBar)> {
    let group = scene.getattr("edit_group")?;
    let tags_only = tree.getattr("_host_roots")?.extract::<i64>()? < 0;
    if tags_only {
        let root = scene.getattr("_root")?;
        let mut tags: Vec<Row> = root
            .call_method1(
                "_drawing_preview",
                (expanded.clone(), query, tree.getattr("links")?.is_truthy()?),
            )?
            .extract()?;
        let mut counts = HashMap::<String, i64>::new();
        for g in tree.getattr("unfiled")?.try_iter()? {
            let tag = g?
                .getattr("layer")?
                .extract::<Option<String>>()?
                .unwrap_or_else(|| "Layer 0".into());
            *counts.entry(tag).or_default() += 1;
        }
        for (text, flags) in &mut tags {
            if let Some(tag) = text[0].strip_prefix("preview-tag:") {
                flags[1] = counts.get(tag).copied().unwrap_or_default();
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
    let mut path = Vec::new();
    let mut context = project_context_rows(scene, query, expanded.clone())?;
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
