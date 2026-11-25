// SPDX-License-Identifier: GPL-3.0-or-later
//! Project tree rows for loose geometry, section planes and the open occurrence.
use pyo3::prelude::*;
use std::collections::HashSet;
pub(crate) type Row = (Vec<String>, Vec<i64>);

pub(crate) use crate::text_search::text_matches;

#[pyfunction]
pub(crate) fn project_name_matches(name: &str, kind: &str, words: Vec<String>) -> bool {
    let name = name.to_lowercase();
    let kind = kind.to_lowercase();
    words.iter().all(|word| {
        let (negative, token) = word
            .strip_prefix('-')
            .map_or((false, word.as_str()), |s| (true, s));
        negative != (text_matches(&name, token) || text_matches(&kind, token))
    })
}

#[pyfunction]
pub(crate) fn project_item_matches(
    scene: &Bound<'_, PyAny>,
    entity: &Bound<'_, PyAny>,
    kind: &str,
    query: &str,
    problem: bool,
) -> PyResult<bool> {
    item_matches(scene, entity, kind, "", query, problem, false, false)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn item_matches(
    scene: &Bound<'_, PyAny>,
    entity: &Bound<'_, PyAny>,
    kind: &str,
    extension_kind: &str,
    query: &str,
    problem: bool,
    parent_hidden: bool,
    parent_locked: bool,
) -> PyResult<bool> {
    let name: String = entity
        .getattr("name")?
        .extract::<Option<String>>()?
        .unwrap_or_default();
    let name = name.to_lowercase();
    let query = query.trim().to_lowercase();
    if query == "!" { return Ok(false); }
    let (invert, query) = query.strip_prefix("! ").map_or((false, query.as_str()), |q| (true, q));
    if query.contains(['*', '?']) && !query.contains(':') && !query.split_whitespace().any(|w| w.starts_with('-'))
        && text_matches(&name, query) {
        return Ok(!invert);
    }
    for word in query.split_whitespace() {
        let (negative, token) = word.strip_prefix('-').map_or((false, word), |s| (true, s));
        let hit = match token {
            "is:hidden" | "is:visible" => {
                let hidden = parent_hidden || !scene.call_method1("entity_visible", (entity,))?.is_truthy()?
                    || (kind == "section" && entity.getattr("hidden")?.is_truthy()?);
                hidden == (token == "is:hidden")
            }
            "is:locked" => parent_locked || scene.call_method1("entity_locked", (entity,))?.is_truthy()?
                || scene.call_method1("_layer_state", (entity,))?.get_item(1)?.is_truthy()?,
            "is:selected" => scene.getattr("selection")?.contains(entity)?,
            "is:image" => entity.getattr("billboard").is_ok_and(|v| v.is_truthy().unwrap_or(false)),
            "is:component" => kind == "component",
            "is:section" => kind == "section",
            "is:link" => kind == "link",
            "has:problem" => problem,
            _ => {
                let mut matched = text_matches(&name, token) || text_matches(kind, token) || (!extension_kind.is_empty() && text_matches(extension_kind, token));
                if !matched {
                    if let Ok(record) = entity.getattr("ifc") {
                        if !record.is_none() {
                            let class = record
                                .call_method1("get", ("class",))?
                                .extract::<Option<String>>()?;
                            matched = class.is_some_and(|s| text_matches(&s.to_lowercase(), token));
                        }
                    }
                    if !matched {
                        if let Ok(tag) = entity.getattr("layer") {
                            matched = tag
                                .extract::<Option<String>>()?
                                .is_some_and(|s| text_matches(&s.to_lowercase(), token));
                        }
                    }
                }
                matched
            }
        };
        if hit == negative {
            return Ok(invert);
        }
    }
    Ok(!invert)
}

pub(crate) fn row(
    key: String,
    name: String,
    kind: &str,
    icon: &str,
    depth: usize,
    row_kind: i64,
    count: usize,
    selected: usize,
) -> Row {
    (
        vec![key, name, kind.into(), String::new(), icon.into()],
        vec![
            depth as i64,
            count as i64,
            0,
            0,
            row_kind,
            -1,
            -1,
            0,
            0,
            selected as i64,
            i64::from(selected > 0),
        ],
    )
}

/// A closed control can belong to the row itself or to an enclosing placement.
pub(crate) fn placement_controls(row: &mut Row, group: &Bound<'_, PyAny>) -> PyResult<()> {
    row.1[5] = i64::from(!group.getattr("hidden")?.is_truthy()?);
    row.1[6] = i64::from(group.getattr("locked")?.is_truthy()?);
    let (hidden, locked) = group.cast::<crate::doc::py::PyPlacement>()?.get()._closed_ancestors();
    let mut tips = [String::new(), String::new()];
    for (i, ancestor, open, message) in [
        (0, hidden, 1, "Hidden by «{name}»"),
        (1, locked, 0, "Locked by «{name}»"),
    ] {
        if let Some(name) = ancestor.filter(|_| row.1[5 + i] == open) {
            let tr = group.py().import("core.i18n")?.getattr("tr")?;
            let fields = pyo3::types::PyDict::new(group.py());
            fields.set_item("name", if name.is_empty() {
                tr.call1(("Unnamed group",))?.extract()?
            } else { name })?;
            tips[i] = tr.call((message,), Some(&fields))?.extract()?;
            row.1[5 + i] = 2;
        }
    }
    if row.1[5] == 2 || row.1[6] == 2 {
        row.0.extend(tips);
    }
    Ok(())
}

fn loose_visible(mesh: &Bound<'_, PyAny>) -> PyResult<bool> {
    let hidden = mesh
        .py()
        .import("ingetrazo_core")?
        .getattr("is_hidden")?;
    for field in ["faces", "edges"] {
        for entity in mesh.getattr(field)?.try_iter()? {
            if !hidden.call1((entity?,))?.is_truthy()? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[pyfunction]
pub(crate) fn loose_edit<'py>(
    scene: &Bound<'py, PyAny>,
    field: &str,
) -> PyResult<Bound<'py, PyAny>> {
    let py = scene.py();
    let history = py.import("core.history")?;
    if field == "eye" {
        let mesh = scene.getattr("mesh")?;
        let entities = py
            .import("itertools")?
            .call_method1("chain", (mesh.getattr("faces")?, mesh.getattr("edges")?))?;
        return history.call_method1("HideCommand", (entities, loose_visible(&mesh)?));
    }
    if field != "lock" {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "Unknown loose geometry field",
        ));
    }
    let key = "project_tree_loose_locks";
    let scope: Vec<String> = scene
        .getattr("_root")?
        .getattr("_edit_path")?
        .try_iter()?
        .map(|g| g?.getattr("uid")?.extract())
        .collect::<PyResult<_>>()?;
    let mut states: Vec<Vec<String>> = scene
        .getattr("plugin_data")?
        .call_method1("get", (key, pyo3::types::PyList::empty(py)))?
        .extract()?;
    if states.contains(&scope) {
        states.retain(|s| s != &scope);
    } else {
        states.push(scope);
    }
    history.call_method1(
        "SetPluginDataCommand",
        (key, (!states.is_empty()).then_some(states)),
    )
}

fn loose(scene: &Bound<'_, PyAny>, depth: usize, query: &str) -> PyResult<Option<Row>> {
    let mesh = scene.getattr("mesh")?;
    let faces = mesh.getattr("faces")?.len()?;
    let edges = mesh.getattr("edges")?.len()?;
    if faces + edges == 0 {
        return Ok(None);
    }
    let mut selected = 0;
    for item in scene.getattr("selection")?.try_iter()? {
        let item = item?;
        selected += usize::from(item.hasattr("loop")? || item.hasattr("v0")?);
    }
    let mut r = row(
        "geometry:loose".into(),
        "Loose geometry".into(),
        "",
        "",
        depth,
        7,
        faces + edges,
        selected,
    );
    r.0[3] = format!("{faces} faces · {edges} edges");
    r.1[5] = i64::from(loose_visible(&mesh)?);
    r.1[6] = i64::from(
        scene
            .getattr("_root")?
            .call_method0("_loose_locked")?
            .is_truthy()?,
    );
    let matches = query.to_lowercase().split_whitespace().all(|word| {
        let (neg, token) = word.strip_prefix('-').map_or((false, word), |s| (true, s));
        let hit = match token {
            "is:hidden" => r.1[5] == 0,
            "is:locked" => r.1[6] == 1,
            _ => text_matches("loose geometry", token),
        };
        neg != hit
    });
    Ok(matches.then_some(r))
}

#[pyfunction(signature = (scene, query, expanded, browser_selected=None))]
pub(crate) fn project_context_rows(
    scene: &Bound<'_, PyAny>,
    query: &str,
    expanded: HashSet<String>,
    browser_selected: Option<HashSet<usize>>,
) -> PyResult<Vec<Row>> {
    let group = scene.getattr("edit_group")?;
    let mut rows = Vec::new();
    let selection = scene.getattr("selection")?;
    if group.is_none() {
        rows.extend(loose(scene, 0, query)?);
        rows.extend(crate::project_sections::rows(
            scene,
            query,
            expanded.contains("sections:all"),
        )?);
        return Ok(rows);
    }

    let open = !expanded.contains("context:fold") || !query.is_empty();
    let mut fold = row("context:fold".into(), format!("In {}", group.getattr("name")?.extract::<String>()?), "", "", 0, 7, 0, 0);
    fold.1[7] = 1;
    fold.1[8] = i64::from(open);
    rows.push(fold);
    if !open { return Ok(rows); }
    rows.extend(loose(scene, 1, query)?);
    let mut stack = vec![(group, 0usize)];
    while let Some((g, depth)) = stack.pop() {
        let children = g.getattr("children")?;
        if depth > 0 {
            let kind = if g.call_method0("is_component")?.is_truthy()? {
                "component"
            } else {
                "group"
            };
            let address = scene.call_method1("occurrence_path", (&g,))?;
            let id: String = scene
                .py()
                .import("core.link_structure")?
                .call_method1("row_id", (address,))?
                .extract()?;
            let key = format!("i:{id}");
            let open = !query.is_empty() || expanded.contains(&key);
            if project_item_matches(scene, &g, kind, query, false)? {
                let name = g
                    .getattr("name")?
                    .extract::<Option<String>>()?
                    .unwrap_or_else(|| "Group".into());
                let mut r = row(
                    key,
                    name,
                    "",
                    kind,
                    depth,
                    1,
                    0,
                    usize::from(match &browser_selected {
                        Some(ids) => ids.contains(&(g.as_ptr() as usize)),
                        None => selection.contains(&g)?,
                    }),
                );
                placement_controls(&mut r, &g)?;
                r.1[7] = i64::from(children.is_truthy()?);
                r.1[8] = i64::from(open);
                rows.push(r);
            }
            if !open {
                continue;
            }
        }
        if children.is_truthy()? {
            for child in children
                .try_iter()?
                .collect::<PyResult<Vec<_>>>()?
                .into_iter()
                .rev()
            {
                stack.push((child, depth + 1));
            }
        }
    }
    Ok(rows)
}

/// Reading the source state must not create a registry entry for a nested file.
#[pyfunction]
pub(crate) fn source_state(path: &str, packed: bool, loaded_ns: Option<u128>) -> &'static str {
    if packed {
        return "packed";
    }
    let Ok(info) = std::fs::metadata(path) else {
        return "missing";
    };
    if !info.is_file() {
        return "missing";
    }
    let current = info
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
    if let (Some(loaded), Some(current)) = (loaded_ns, current) {
        if loaded != current.as_nanos() {
            return "modified";
        }
    }
    "resolved"
}

/// Restore the source visibility recorded by the last load. Clear only this
/// source's overrides in the same history transaction as its tag changes.
#[pyfunction]
pub(crate) fn reset_source_tags<'py>(
    scene: &Bound<'py, PyAny>,
    path: &str,
) -> PyResult<Option<Bound<'py, PyAny>>> {
    use pyo3::types::{PyDict, PyList};
    let py = scene.py();
    let data = crate::model::links::registry_view(scene)?;
    if !data.is_truthy()? {
        return Ok(None);
    }
    let saved = py.import("copy")?.call_method1("deepcopy", (data,))?;
    let key = crate::model::links::norm(&pyo3::types::PyString::new(py, path))?;
    let entry = saved.call_method1("get", (key,))?;
    if entry.is_none() {
        return Ok(None);
    }
    let overrides = entry.call_method1("get", ("overrides",))?;
    if !overrides.is_truthy()? {
        return Ok(None);
    }
    let original = entry.call_method1("get", ("source_visible", PyDict::new(py)))?;
    let mut tags = [Vec::<String>::new(), Vec::<String>::new()];
    for name in overrides.try_iter()? {
        let name = name?;
        let state = original.call_method1("get", (&name,))?;
        if !state.is_none() && !scene.call_method1("layer_by_key", (&name,))?.is_none() {
            tags[usize::from(state.is_truthy()?)].push(name.extract()?);
        }
    }
    entry.set_item("overrides", PyDict::new(py))?;
    let commands = PyList::empty(py);
    let flags = py
        .import("ingetrazo_core")?
        .getattr("SetTagStateCommand")?;
    for (index, keys) in tags.iter().enumerate() {
        if !keys.is_empty() {
            commands.append(flags.call1((keys, PyList::empty(py), index == 1, py.None()))?)?;
        }
    }
    let history = py.import("core.history")?;
    commands.append(py.import("core.links")?.call_method1("set_registry_command", (saved,))?)?;
    Ok(Some(history.call_method1("CompoundCommand", (commands,))?))
}

/// A source containing just loose geometry or nested references has no filed
/// children to count. Each placed copy contributes its own source contents.
#[pyfunction]
pub(crate) fn source_direct_count(copies: Vec<Bound<'_, PyAny>>) -> PyResult<usize> {
    let mut count = 0;
    for g in copies {
        let mesh = g.getattr("mesh")?;
        count += usize::from(mesh.getattr("faces")?.len()? + mesh.getattr("edges")?.len()? > 0);
        let children = g.getattr("children")?;
        if children.is_truthy()? {
            for child in children.try_iter()? {
                count += usize::from(crate::model::links::is_linked(&child?)?);
            }
        }
    }
    Ok(count)
}

#[pyfunction]
pub(crate) fn source_placement_rows(
    scene: &Bound<'_, PyAny>,
    copies: Vec<Bound<'_, PyAny>>,
    depth: usize,
) -> PyResult<Vec<Row>> {
    let mut rows = Vec::new();
    if copies.len() < 2 {
        return Ok(rows);
    }
    let tree = scene.py().import("core.link_structure")?;
    let selection = scene.getattr("selection")?;
    for (index, g) in copies.iter().enumerate() {
        let address = scene.call_method1("occurrence_path", (g,))?;
        let key: String = tree.call_method1("row_id", (address,))?.extract()?;
        let name: String = g
            .getattr("name")?
            .extract::<Option<String>>()?
            .unwrap_or_default();
        let mut placement = row(
            format!("i:{key}"),
            format!("Placement {} · {name}", index + 1),
            "Reference",
            "link",
            depth,
            7,
            0,
            usize::from(selection.contains(g)?),
        );
        placement.1[5] = i64::from(!g.getattr("hidden")?.is_truthy()?);
        rows.push(placement);
    }
    Ok(rows)
}

#[pyfunction]
pub(crate) fn expand_project_folders(
    scene: &Bound<'_, PyAny>,
    nodes: &Bound<'_, pyo3::types::PyDict>,
) -> PyResult<Vec<String>> {
    let mut keys = Vec::new();
    for node in nodes.keys() {
        keys.push(format!("n:{}", node.extract::<String>()?));
    }
    for folder in scene
        .py()
        .import("core.layers")?
        .call_method1("folders_of", (scene,))?
        .try_iter()?
    {
        keys.push(format!("preview-folder:{}", folder?.extract::<String>()?));
    }
    Ok(keys)
}

/// Candidate branches for one explicit reveal. This read follows the selected
/// occurrence paths without expanding every linked file to search for a row.
#[pyfunction]
pub(crate) fn reveal_branches(
    scene: &Bound<'_, PyAny>,
    tree: &Bound<'_, PyAny>,
) -> PyResult<HashSet<String>> {
    let mut branches = HashSet::from(["n:".to_owned()]);
    let nodes = tree.getattr("nodes")?;
    let members = tree.getattr("member")?;
    let formatter = scene
        .py()
        .import("core.link_structure")?
        .getattr("row_id")?;
    for item in scene.getattr("selection")?.try_iter()? {
        let item = item?;
        if !item.hasattr("mesh")? || !item.hasattr("uid")? {
            continue;
        }
        let path: Vec<String> = scene.call_method1("occurrence_path", (&item,))?.extract()?;
        for depth in 1..=path.len() {
            let address = path[..depth].to_vec();
            let group = scene.call_method1("resolve_occurrence", (&address,))?;
            if group.is_none() {
                continue;
            }
            let id: String = tree.call_method1("row_id", (&group,))?.extract()?;
            branches.insert(format!("i:{id}"));
            branches.insert(format!("c:{id}"));
            let scoped: String = formatter.call1((address,))?.extract()?;
            branches.insert(format!("i:{scoped}"));
            let key: Option<String> = members
                .call_method1("get", (group.as_ptr() as usize,))?
                .extract()?;
            let mesh = group.getattr("mesh")?;
            let folder = key.as_deref().unwrap_or("");
            branches.insert(format!("d:{folder}:{}", mesh.as_ptr() as usize));
            let mut parent = key;
            while let Some(k) = parent {
                if !branches.insert(format!("n:{k}")) {
                    break;
                }
                parent = nodes.get_item(k)?.get_item("parent")?.extract()?;
            }
        }
    }
    Ok(branches)
}

/// Locate a selected occurrence in the unpaged outline. The display host
/// applies only the ancestor expansions and page limits needed to show it.
#[pyfunction]
pub(crate) fn reveal_selection(
    rows: Vec<Row>,
) -> Option<(String, Vec<String>, Vec<(String, usize)>)> {
    let mut path: Vec<(String, usize)> = Vec::new();
    let mut position = Vec::<usize>::new();
    let has_item = rows.iter().any(|(t, f)| {
        t.first().is_some_and(|k| k.starts_with("i:")) && f.get(9).copied().unwrap_or(0) > 0
    });
    for (text, flags) in &rows {
        let key = text.first()?;
        let depth = usize::try_from(*flags.first()?).ok()?;
        path.truncate(depth);
        position.resize(depth + 1, 0);
        position[depth] += 1;
        if flags.get(9).copied().unwrap_or(0) > 0
            && (key.starts_with("i:")
                || key.starts_with("c:")
                || key.starts_with("section:")
                || key == "geometry:loose"
                || (text.get(4)?.starts_with("file:") && !has_item))
        {
            let ancestors = path.iter().map(|(k, _)| k.clone()).collect();
            let pages = path
                .iter()
                .enumerate()
                .map(|(i, (k, _))| {
                    let limit = path.get(i + 1).map_or(position[depth], |(_, n)| *n);
                    (k.strip_prefix("n:").unwrap_or(k).to_owned(), limit)
                })
                .collect();
            return Some((key.clone(), ancestors, pages));
        }
        path.push((key.clone(), position[depth]));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reveal_opens_only_the_occurrences_ancestors_and_its_page() {
        let rows = [
            ("n:other", 0, 0),
            ("n:site", 0, 1),
            ("n:doors", 1, 1),
            ("d:doors:8", 2, 1),
            ("i:copy-a", 3, 0),
            ("i:copy-b", 3, 1),
        ]
        .iter()
        .map(|&(k, d, s)| row(k.into(), k.into(), "", "", d, 1, 0, s))
        .collect();
        let (target, branches, pages) = reveal_selection(rows).unwrap();
        assert_eq!(target, "i:copy-b");
        assert_eq!(branches, ["n:site", "n:doors", "d:doors:8"]);
        assert_eq!(pages.last(), Some(&("d:doors:8".into(), 2)));
    }
}
