//! Definition choices and replacement history belong to the editing document.
use super::model_edit::{matching, translated};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use std::collections::{BTreeSet, HashMap, HashSet};

fn component_groups<'py>(items: &Bound<'py, PyAny>) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let mut groups = Vec::new();
    for g in matching(items, "core.group", "Group")?.iter() {
        if g.call_method0("is_component")?.is_truthy()? {
            groups.push(g)
        }
    }
    Ok(groups)
}
#[pyfunction]
fn replacement_choices<'py>(
    scene: &Bound<'py, PyAny>,
    selection: &Bound<'py, PyAny>,
) -> PyResult<Vec<(String, Bound<'py, PyAny>)>> {
    let py = scene.py();
    let mut own = HashSet::new();
    for group in component_groups(selection)? {
        own.insert(group.getattr("mesh")?.as_ptr());
    }
    let mut pool = component_groups(&scene.getattr("groups")?)?;
    let ctx = scene.getattr("edit_group")?;
    if !ctx.is_none() {
        let children = ctx.getattr("children")?;
        if !children.is_none() {
            let placed: HashSet<_> = pool.iter().map(|g| g.as_ptr()).collect();
            pool.extend(
                component_groups(&children)?
                    .into_iter()
                    .filter(|g| !placed.contains(&g.as_ptr())),
            );
        }
    }
    let mut rows: Vec<(Bound<'py, PyAny>, usize, String)> = Vec::new();
    let mut by_mesh = HashMap::<_, usize>::new();
    for group in pool {
        let key = group.getattr("mesh")?.as_ptr();
        if own.contains(&key) || group.getattr("billboard")?.is_truthy()? {
            continue;
        }
        if let Some(&i) = by_mesh.get(&key) {
            rows[i].1 += 1;
        } else {
            by_mesh.insert(key, rows.len());
            let name = group
                .getattr("name")?
                .extract::<Option<String>>()?
                .unwrap_or_default();
            rows.push((group, 1, name));
        }
    }
    for group in scene.getattr("kept_components")?.try_iter()? {
        let group = group?;
        let key = group.getattr("mesh")?.as_ptr();
        if !own.contains(&key) && !by_mesh.contains_key(&key) {
            let name = group
                .getattr("name")?
                .extract::<Option<String>>()?
                .unwrap_or_default();
            rows.push((group, 0, name));
        }
    }
    rows.sort_by_cached_key(|(_, n, name)| (name.to_lowercase(), std::cmp::Reverse(*n)));
    let fallback = translated(py, "Component", None)?.extract::<String>()?;
    let mut seen = HashSet::new();
    let mut choices = Vec::new();
    for (group, n, name) in rows {
        let name = if name.is_empty() { &fallback } else { &name };
        let mut label = format!("{name}  ({n}×)");
        let mut k = 2;
        while !seen.insert(label.clone()) {
            label = format!("{name} [{k}]  ({n}×)");
            k += 1;
        }
        choices.push((label, group));
    }
    Ok(choices)
}

#[pyfunction]
fn choose_replacement(win: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let targets = PyList::new(py, component_groups(&scene.getattr("selection")?)?)?;
    let choices = replacement_choices(&scene, &targets)?;
    if targets.is_empty() || choices.is_empty() {
        let text = if targets.is_empty() {
            "Select a component first"
        } else {
            "There is no other component in the model to replace it with"
        };
        vp.call_method1("flash_status", (translated(py, text, None)?,))?;
        return Ok(());
    }
    let map = PyDict::new(py);
    for (label, source) in choices {
        map.set_item(label, source)?;
    }
    let kw = PyDict::new(py);
    kw.set_item(
        "done",
        py.import("functools")?.call_method1(
            "partial",
            (
                wrap_pyfunction!(replacement_answer, py)?,
                win,
                &scene,
                scene.getattr("edit_group")?,
                &targets,
                &map,
            ),
        )?,
    )?;
    let args = PyDict::new(py);
    args.set_item("n", targets.len())?;
    py.import("ingetrazo_render.chrome_forms")?
        .getattr("ask_choice")?
        .call(
            (
                win,
                translated(py, "Replace Component", None)?,
                translated(py, "Replace {n} selected instances with:", Some(&args))?,
                map.keys(),
            ),
            Some(&kw),
        )?;
    Ok(())
}
#[pyfunction]
#[pyo3(signature=(win, scene, context, targets, choices, answer=None))]
fn replacement_answer(
    win: &Bound<'_, PyAny>,
    scene: &Bound<'_, PyAny>,
    context: &Bound<'_, PyAny>,
    targets: &Bound<'_, PyList>,
    choices: &Bound<'_, PyDict>,
    answer: Option<String>,
) -> PyResult<()> {
    let Some(answer) = answer else { return Ok(()) };
    let active = win.getattr("viewport")?.getattr("scene")?;
    if !active.is(scene) || !active.getattr("edit_group")?.is(context) {
        return Ok(());
    }
    let current = scene.getattr("selection")?;
    for g in targets.iter() {
        if !current.contains(&g)? {
            return Ok(());
        }
    }
    if let Some(source) = choices.get_item(answer)? {
        replace_groups(win, targets, &source)?;
    }
    Ok(())
}

#[pyfunction]
fn ask_keep_orphans(win: &Bound<'_, PyAny>, orphans: &Bound<'_, PyAny>) -> PyResult<Option<bool>> {
    let py = win.py();
    let mut names = BTreeSet::new();
    for g in orphans.try_iter()? {
        let name = g?
            .getattr("name")?
            .extract::<Option<String>>()?
            .unwrap_or_default();
        names.insert(if name.is_empty() {
            translated(py, "Component", None)?.extract()?
        } else {
            name
        });
    }
    let vars = PyDict::new(py);
    vars.set_item("names", names.into_iter().collect::<Vec<_>>().join(", "))?;
    let buttons = PyList::empty(py);
    for (k, v) in [
        ("drop", "Let It Go"),
        ("cancel", "Cancel"),
        ("keep", "Keep in File"),
    ] {
        buttons.append((k, translated(py, v, None)?))?;
    }
    let kw = PyDict::new(py);
    kw.set_item("default", "keep")?;
    kw.set_item("cancel", "cancel")?;
    let answer = py.import("ingetrazo_render.chrome_dialogs")?.getattr("ask")?.call(
        (
            win,
            translated(py, "Keep {names} in the file?", Some(&vars))?,
            translated(py, "You can place it or swap back to it later.", None)?,
            buttons,
        ),
        Some(&kw),
    )?;
    Ok(match answer.extract::<Option<String>>()?.as_deref() {
        Some("keep") => Some(true),
        Some("drop") => Some(false),
        _ => None,
    })
}

#[pyfunction]
fn replace_selection(win: &Bound<'_, PyAny>, source: &Bound<'_, PyAny>) -> PyResult<usize> {
    let targets = matching(
        &win.getattr("viewport")?
            .getattr("scene")?
            .getattr("selection")?,
        "core.group",
        "Group",
    )?;
    replace_groups(win, &targets, source)
}

fn replace_groups(
    win: &Bound<'_, PyAny>,
    targets: &Bound<'_, PyList>,
    source: &Bound<'_, PyAny>,
) -> PyResult<usize> {
    let py = win.py();
    let vp = win.getattr("viewport")?;
    let scene = vp.getattr("scene")?;
    let lib = py.import("ingetrazo_core.history")?;
    let cmd = lib.call_method1("ReplaceComponentCommand", (targets, source))?;
    let affected = cmd.getattr("targets")?;
    let n = affected.len()?;
    if n == 0 {
        return Ok(0);
    }
    let orphans = py
        .import("core.kept_components")?
        .call_method1("orphaned_by", (&scene, &affected))?;
    let steps = PyList::empty(py);
    if orphans.is_truthy()? {
        match win
            .call_method1("_ask_keep_orphans", (&orphans,))?
            .extract::<Option<bool>>()?
        {
            None => return Ok(0),
            Some(true) => {
                for g in orphans.try_iter()? {
                    steps.append(lib.call_method1("KeepComponentCommand", (g?,))?)?;
                }
            }
            Some(false) => (),
        }
    }
    let command = if steps.is_empty() {
        cmd
    } else {
        steps.append(cmd)?;
        lib.call_method1("CompoundCommand", (steps,))?
    };
    let history = vp.getattr("history")?;
    history.call_method1("execute", (command,))?;
    if history.getattr("last_error")?.is_truthy()?
        || history.getattr("last_rejected")?.is_truthy()?
    {
        return Ok(0);
    }
    vp.call_method0("notify_scene_changed")?;
    vp.call_method0("update")?;
    let vars = PyDict::new(py);
    vars.set_item("n", n)?;
    vars.set_item("name", source.getattr("name")?)?;
    win.call_method0("statusBar")?.call_method1(
        "showMessage",
        (
            translated(py, "{n} instances replaced with {name}", Some(&vars))?,
            4000,
        ),
    )?;
    Ok(n)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(replacement_choices, m)?)?;
    m.add_function(wrap_pyfunction!(choose_replacement, m)?)?;
    m.add_function(wrap_pyfunction!(replace_selection, m)?)?;
    m.add_function(wrap_pyfunction!(ask_keep_orphans, m)?)?;
    Ok(())
}
