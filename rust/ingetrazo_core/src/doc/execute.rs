// SPDX-License-Identifier: GPL-3.0-or-later
//! The document transaction enclosing a host tool invocation.
use super::py::PyTransaction;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use std::collections::HashSet;

fn owner<'py>(mesh: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    mesh.getattr("__dict__")?
        .call_method1("get", ("_instance_definition", mesh))
}

pub(super) fn execute<'py>(
    py: Python<'py>,
    host: &Bound<'py, PyAny>,
    cmd: &Bound<'py, PyAny>,
    tag_objects: &Bound<'py, PyAny>,
) -> PyResult<()> {
    let api = py.import("core.history_commands")?;
    let doc = py.import("core.doc_model")?;
    let scene = host.getattr("scene")?;
    let document = scene.getattr("_root")?.getattr("_doc_key")?.extract::<super::Key>()?;
    let _writer = super::Writer::acquire(py, Some(document))?;
    let active = scene.getattr("mesh")?;
    let parts = PyList::empty(py);
    let mutation = api.getattr("SnapshotMutation")?;
    let compound = api.getattr("SnapshotCompound")?;
    let snapshot_commands = (&mutation, &compound).into_pyobject(py)?;
    let auto_tags = (
        api.getattr("AddDimensionCommand")?,
        api.getattr("AddTextLabelCommand")?,
        api.getattr("AddGuideCommand")?,
    )
        .into_pyobject(py)?;
    let replay = api
        .getattr("_geometry_plan")?
        .call1((cmd, &scene, &parts))?
        .is_truthy()?;
    if !replay && cmd.get_type().is(&api.getattr("CompoundCommand")?) && super::lock().depth() == 0
    {
        return execute_mixed(py, host, cmd, tag_objects);
    }
    let depth: usize = doc.call_method1("begin", (document,))?.extract()?;
    let mut rollback = None;
    let mut annotation = None;
    let result = (|| {
        if !parts.is_empty() && replay {
            cmd.setattr("_journal_geometry", depth == 1)?;
        }
        let mut owners = HashSet::new();
        for pair in parts.iter() {
            let part = pair.get_item(0)?;
            part.setattr("_journal_geometry", replay && depth == 1)?;
            let staged = part.is_instance(&mutation)? && !part.getattr("after")?.is_none();
            for mesh in pair.get_item(1)?.try_iter()? {
                let mesh = mesh?;
                let initial = if staged {
                    part.getattr("before")?
                } else {
                    py.None().into_bound(py)
                };
                super::geometry::touch(
                    &mesh,
                    if initial.is_none() {
                        None
                    } else {
                        Some(&initial)
                    },
                )?;
                owners.insert(owner(&mesh)?.as_ptr());
            }
        }
        if !owners.contains(&owner(&active)?.as_ptr()) {
            rollback = Some(active.call_method0("capture_state")?);
        }
        // A fresh materialised drawing step and the transaction both need the
        // same pre-edit state. Keep the transaction's strong reference until
        // document commit, including any failing tag or attachment callbacks.
        let mut kwargs = None;
        if let Some(before) = &rollback {
            if cmd.get_type().is(&compound)
                && cmd.getattr("after")?.is_none()
                && !cmd.getattr("__dict__")?.contains("do")?
            {
                let kw = PyDict::new(py);
                kw.set_item("rollback_state", before)?;
                kwargs = Some(kw);
            }
        }
        cmd.call_method("do", (&scene,), kwargs.as_ref())?;
        if !tag_objects.is_none() {
            host.call_method0("_follow_text_placements")?;
        }
        // Auto-created annotation tags survive undo but belong to the published diff.
        if !tag_objects.is_none() && depth == 1 && cmd.is_instance(&auto_tags)? {
            annotation = Some(doc.call_method0("commit")?);
            doc.call_method1("begin", (document,))?;
        }
        if !tag_objects.is_none() {
            tag_objects.call0()?;
        }
        doc.call_method0("commit")
    })();
    let txn = match result {
        Ok(txn) => txn,
        Err(error) => {
            if depth > 0 {
                doc.call_method0("rollback")?;
            }
            if let Some(edit) = annotation {
                edit.call_method0("undo")?;
            }
            if let Some(state) = rollback {
                active.call_method1("restore_state", (state,))?;
            }
            return Err(error);
        }
    };
    if !txn.is_none() {
        for pair in parts.iter() {
            let part = pair.get_item(0)?;
            if part.getattr("_journal_geometry")?.is_truthy()?
                && part.is_instance(&snapshot_commands)?
            {
                let mesh = pair.get_item(1)?.get_item(0)?;
                let states = txn.call_method1("geometry_states", (owner(&mesh)?,))?;
                part.setattr("before", states.get_item(0)?)?;
                part.setattr("after", states.get_item(1)?)?;
            }
        }
    }
    let replay = if let Some(edit) = annotation {
        let combined = txn.call_method1("compose", (PyList::new(py, [&edit, &txn])?,))?;
        host.setattr("last_txn", (combined, false))?;
        edit
    } else {
        if !txn.is_none() {
            host.setattr("last_txn", (&txn, false))?;
        }
        txn
    };
    if !replay.is_none()
        && api
            .getattr("_uses_doc_undo")?
            .call1((cmd, &scene))?
            .is_truthy()?
    {
        cmd.call_method1("bind_transaction", (replay,))?;
    } else {
        cmd.setattr("_doc_txn", py.None())?;
    }
    Ok(())
}

/// A compound containing an opaque inverse needs ordered records. Commit
/// each component privately, then expose one stack entry and one feed step.
/// The trailing transaction contains automatic tags and placement followers.
fn execute_mixed(
    py: Python<'_>,
    host: &Bound<'_, PyAny>,
    cmd: &Bound<'_, PyAny>,
    tags: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let api = py.import("core.history_commands")?;
    let group = api
        .getattr("OccurrenceCommands")?
        .call1((PyList::empty(py),))?;
    let finished = group.getattr("commands")?.cast_into::<PyList>()?;
    let mut records = Vec::new();
    let scene = host.getattr("scene")?;
    let document = scene.getattr("_root")?.getattr("_doc_key")?;
    let result: PyResult<()> = (|| {
        for part in cmd.getattr("commands")?.try_iter()? {
            let part = part?;
            host.setattr("last_txn", py.None())?;
            execute(py, host, &part, py.None().bind(py))?;
            finished.append(&part)?;
            let diff = host.getattr("last_txn")?;
            if !diff.is_none() {
                records.push(diff.get_item(0)?.extract::<Py<PyTransaction>>()?);
            }
        }
        if !tags.is_none() {
            let doc = py.import("core.doc_model")?;
            doc.call_method1("begin", (document,))?;
            let tail = host
                .call_method0("_follow_text_placements")
                .and_then(|_| tags.call0())
                .and_then(|_| doc.call_method0("commit"));
            let txn = match tail {
                Ok(txn) => txn,
                Err(error) => {
                    doc.call_method0("rollback")?;
                    return Err(error);
                }
            };
            if txn.len()? != 0 {
                let edit = api
                    .getattr("CompoundCommand")?
                    .call1((PyList::empty(py),))?;
                edit.setattr("_doc_txn", &txn)?;
                finished.append(edit)?;
                records.push(txn.extract()?);
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        group.call_method1("undo", (&scene,))?;
        host.setattr("last_txn", py.None())?;
        return Err(error);
    }
    let diff = Py::new(py, PyTransaction::compose(py, records)?)?;
    cmd.setattr("_mixed_group", group)?;
    cmd.setattr("_group_diff", &diff)?;
    cmd.setattr("_doc_txn", py.None())?;
    host.setattr("last_txn", (diff, false))
}
