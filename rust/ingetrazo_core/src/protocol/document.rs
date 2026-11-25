// SPDX-License-Identifier: GPL-3.0-or-later
//! Document edit verbs resolve the active scene and publish a History step.
use super::{answer, execute, history_command, placements::resolve_placements, Obj, Reply};
use pyo3::{prelude::*, types::PyList};

#[pyfunction]
fn edit_loose_geometry<'py>(history: &Obj<'py>, field: &Obj<'py>) -> Reply<'py> {
    let scene = history.getattr("scene")?;
    let step = history
        .py()
        .import("ingetrazo_core")?
        .call_method1("loose_edit", (scene, field))?;
    execute(history, &step)
}

#[pyfunction]
fn promote_tag<'py>(history: &Obj<'py>, tag: &Obj<'py>, ifc_class: &Obj<'py>) -> Reply<'py> {
    let scene = history.getattr("scene")?;
    let step = history
        .py()
        .import("ingetrazo_core")?
        .call_method1("promote_tag", (scene, tag, ifc_class))?;
    execute(history, &step)
}

#[pyfunction]
fn file_placements<'py>(history: &Obj<'py>, placements: &Obj<'py>, node: &Obj<'py>) -> Reply<'py> {
    let scene = history.getattr("scene")?;
    let step = history
        .py()
        .import("core.structure")?
        .call_method1("file", (scene, placements, node))?;
    execute(history, &step)
}

#[pyfunction]
fn edit_structure_node<'py>(
    history: &Obj<'py>,
    node: &Obj<'py>,
    field: &Obj<'py>,
    value: &Obj<'py>,
) -> Reply<'py> {
    let scene = history.getattr("scene")?;
    let step = history
        .py()
        .import("ingetrazo_core")?
        .call_method1("plan_node_edit", (scene, node, field, value))?;
    execute(history, &step)
}

#[pyfunction]
fn reset_link_tags<'py>(history: &Obj<'py>, path: &Obj<'py>) -> Reply<'py> {
    let scene = history.getattr("scene")?;
    let step = history
        .py()
        .import("ingetrazo_core")?
        .call_method1("reset_source_tags", (scene, path))?;
    execute(history, &step)
}

#[pyfunction]
fn delete_placements<'py>(history: &Obj<'py>, placements: &Obj<'py>) -> Reply<'py> {
    let groups = resolve_placements(history, placements)?;
    let before = history.getattr("revision")?;
    let steps = PyList::empty(history.py());
    for group in groups.iter() {
        steps.append(history_command(
            history.py(),
            "DeleteGroupCommand",
            (group,),
        )?)?;
    }
    if !steps.is_empty() {
        history.call_method1(
            "execute",
            (history_command(history.py(), "CompoundCommand", (steps,))?,),
        )?;
    }
    answer(history, &before, true)
}
#[pyfunction]
fn unlink_placements<'py>(history: &Obj<'py>, placements: &Obj<'py>) -> Reply<'py> {
    let groups = resolve_placements(history, placements)?;
    let step = history_command(history.py(), "UnlinkCommand", (groups,))?;
    let before = history.getattr("revision")?;
    if step.getattr("groups")?.is_truthy()? {
        history.call_method1("execute", (step,))?;
    }
    answer(history, &before, true)
}
#[pyfunction]
fn edit_tag<'py>(
    history: &Obj<'py>,
    tags: &Obj<'py>,
    field: &Obj<'py>,
    value: &Obj<'py>,
) -> Reply<'py> {
    let edit = history
        .getattr("scene")?
        .getattr("_root")?
        .call_method1("_tag_edit", (tags, field, value))?;
    let before = history.getattr("revision")?;
    if !edit.is_none() {
        let step = history
            .py()
            .import("ingetrazo_core")?
            .call_method1("EditTagCommand", (edit,))?;
        history.call_method1("execute", (step,))?;
    }
    answer(history, &before, true)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(edit_loose_geometry, m)?)?;
    m.add_function(wrap_pyfunction!(promote_tag, m)?)?;
    m.add_function(wrap_pyfunction!(file_placements, m)?)?;
    m.add_function(wrap_pyfunction!(edit_structure_node, m)?)?;
    m.add_function(wrap_pyfunction!(reset_link_tags, m)?)?;
    m.add_function(wrap_pyfunction!(delete_placements, m)?)?;
    m.add_function(wrap_pyfunction!(unlink_placements, m)?)?;
    m.add_function(wrap_pyfunction!(edit_tag, m)?)?;
    Ok(())
}
