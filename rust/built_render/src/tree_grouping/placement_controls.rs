// SPDX-License-Identifier: GPL-3.0-or-later
//! Selection shared with the viewport, plus closed rows picked only in the Browser.
use super::{row_actions, translated, Occurrences, READ_ONLY};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use std::collections::HashSet;

fn model_ids(scene: &Bound<'_, PyAny>) -> PyResult<HashSet<usize>> {
    scene
        .getattr("selection")?
        .try_iter()?
        .map(|e| Ok(e?.as_ptr() as usize))
        .collect()
}

fn kept<'py>(
    scene: &Bound<'py, PyAny>,
    selected: &HashSet<usize>,
) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let Some(state) = scene
        .getattr("__dict__")?
        .cast::<PyDict>()?
        .get_item("_browser_picks")?
    else {
        return Ok(vec![]);
    };
    let expected: HashSet<usize> = state.get_item(0)?.extract()?;
    let mut rows = vec![];
    if &expected == selected {
        let root = scene.getattr("_root")?;
        for group in state.get_item(1)?.try_iter()? {
            let group = group?;
            if !root.call_method1("_occurrence_path", (&group,))?.is_none() {
                rows.push(group);
            }
        }
    }
    if rows.len() != state.get_item(1)?.len()? {
        remember(scene, selected, &rows)?;
    }
    Ok(rows)
}

fn remember(
    scene: &Bound<'_, PyAny>,
    selected: &HashSet<usize>,
    rows: &[Bound<'_, PyAny>],
) -> PyResult<()> {
    if rows.is_empty() {
        scene
            .getattr("__dict__")?
            .call_method1("pop", ("_browser_picks", scene.py().None()))?;
    } else {
        scene.setattr("_browser_picks", (selected, PyList::new(scene.py(), rows)?))?;
    }
    Ok(())
}

#[pyfunction]
pub fn browser_selection(scene: &Bound<'_, PyAny>) -> PyResult<HashSet<usize>> {
    let mut ids = model_ids(scene)?;
    for row in kept(scene, &ids)? {
        ids.insert(row.as_ptr() as usize);
    }
    Ok(ids)
}

fn selectable(scene: &Bound<'_, PyAny>, group: &Bound<'_, PyAny>) -> PyResult<bool> {
    let (hidden, locked): (Option<String>, Option<String>) =
        group.call_method0("_closed_ancestors")?.extract()?;
    Ok(hidden.is_none()
        && locked.is_none()
        && !group.getattr("hidden")?.is_truthy()?
        && scene
            .call_method1("entity_selectable", (group,))?
            .is_truthy()?)
}

pub(super) fn act(win: &Bound<'_, PyAny>, clicked: &Bound<'_, PyAny>, verb: &str) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let tree = Occurrences::read(&scene, clicked)?;
    let Some(index) = tree.rows.iter().position(|r| r.group.is(clicked)) else {
        return Ok(());
    };
    let selected = model_ids(&scene)?;
    let mut kept_rows = kept(&scene, &selected)?;
    let nav = py.import("built_render.tree_navigation")?;
    if matches!(verb, "pickrow" | "toggleitem") {
        let toggle = verb == "toggleitem";
        if !toggle {
            kept_rows.clear();
        }
        if selectable(&scene, clicked)? {
            if !toggle
                && !nav
                    .call_method1("open_to", (&vp, tree.path(tree.rows[index].parent)))?
                    .is_truthy()?
            {
                return Ok(());
            }
            kept_rows.retain(|g| !g.is(clicked));
            scene.call_method1(
                "select",
                ([clicked], false, if toggle { "toggle" } else { "replace" }),
            )?;
        } else {
            if !toggle {
                scene.call_method0("clear_selection")?;
            }
            if let Some(at) = kept_rows.iter().position(|g| g.is(clicked)) {
                kept_rows.remove(at);
            } else {
                kept_rows.push(clicked.clone());
            }
        }
    } else {
        if !row_actions::writable(&scene)? {
            vp.call_method1("flash_status", (translated(py, READ_ONLY, None)?, 4000))?;
            return Ok(());
        }
        let lock = verb == "rowlock";
        let field = if lock { "locked" } else { "hidden" };
        let inheritance = |g: &Bound<'_, PyAny>| -> PyResult<Option<String>> {
            g.call_method0("_closed_ancestors")?
                .get_item(usize::from(lock))?
                .extract()
        };
        let own = clicked.getattr(field)?.is_truthy()?;
        if let Some(ancestor) = inheritance(clicked)?.filter(|_| !own) {
            let kw = PyDict::new(py);
            if ancestor.is_empty() {
                kw.set_item("name", translated(py, "Unnamed group", None)?)?;
            } else {
                kw.set_item("name", ancestor)?;
            }
            let msg = if lock {
                "Locked by «{name}»: unlock it first"
            } else {
                "Hidden by «{name}»: show it first"
            };
            vp.call_method1("flash_status", (translated(py, msg, Some(&kw))?, 3500))?;
            return Ok(());
        }
        let mut picked = selected.clone();
        picked.extend(kept_rows.iter().map(|g| g.as_ptr() as usize));
        let multi = picked.contains(&(clicked.as_ptr() as usize));
        let mut targets = Vec::new();
        let mut close = true;
        let mut editing_target = false;
        let open = scene.getattr("_root")?.getattr("_edit_path")?;
        for row in &tree.rows {
            let g = &row.group;
            if !g.is(clicked) && !(multi && picked.contains(&(g.as_ptr() as usize))) {
                continue;
            }
            let holder = crate::host_context::core_module(py)?.call_method1("link_linked_ancestor", (g,))?;
            if !holder.is_none() && !holder.is(g) {
                continue;
            }
            let own = g.getattr(field)?.is_truthy()?;
            if !own && inheritance(g)?.is_some() {
                continue;
            }
            close &= !own;
            editing_target |= open.contains(g)?;
            targets.push(g);
        }
        if targets.is_empty() {
            return Ok(());
        }
        if close && editing_target {
            vp.call_method0("end_group_edit")?;
        }
        let groups = PyList::new(py, &targets)?;
        let cmd = if lock {
            py.import("built_core")?
                .getattr("NativeCommand")?
                .call_method1("lock", (groups, close))?
        } else {
            py.import("built_core.history")?
                .call_method1("HideCommand", (groups, close))?
        };
        nav.call_method1("execute", (&vp, cmd))?;
        if !picked.is_empty() {
            let mut adopt = vec![];
            kept_rows.clear();
            for row in &tree.rows {
                let g = &row.group;
                if !picked.contains(&(g.as_ptr() as usize)) {
                    continue;
                }
                if selectable(&scene, g)? {
                    if !scene.getattr("selection")?.contains(g)? {
                        adopt.push(g);
                    }
                } else {
                    kept_rows.push(g.clone());
                }
            }
            let selection = scene.getattr("selection")?;
            for g in &kept_rows {
                selection.call_method1("discard", (g,))?;
            }
            if !adopt.is_empty() {
                scene.call_method1("select", (PyList::new(py, adopt)?, true))?;
            }
        }
    }
    remember(&scene, &model_ids(&scene)?, &kept_rows)?;
    vp.call_method0("notify_scene_changed")?;
    Ok(())
}
