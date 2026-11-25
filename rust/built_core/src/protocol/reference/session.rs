// SPDX-License-Identifier: GPL-3.0-or-later
//! Close a singular placement edit as one undo step.
use super::{restore_sharing, share_back, version, Obj};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySlice, PyTuple},
};
#[pyfunction]
pub(super) fn record_leave(history: &Obj<'_>, level: &Obj<'_>, share: &Obj<'_>) -> PyResult<bool> {
    let py = history.py();
    let h = crate::model::module(py, "built_core.history")?;
    let steps = crate::model::module(py, "built_core.reference_edit")?;
    let parts = level.call_method1("get", ("parts",))?;
    let g = share.get_item(0)?;
    let proto = share.get_item(1)?;
    let mat = share.get_item(2)?;
    let state = share.get_item(3)?;
    let edited = g.getattr("mesh")?;
    let stack = history.getattr("undo_stack")?;
    let n = stack.len()? as isize;
    let mark = if level.contains("session")? {
        h.getattr("OccurrenceCommands")?
            .call_method1("group_start", (history, level.get_item("session")?))?
            .extract()?
    } else {
        level
            .call_method1("get", ("undo_mark", n))?
            .extract::<isize>()?
            .min(n)
    };
    let shared = !edited.call_method0("capture_state")?.eq(&state)?;
    if !shared && (parts.is_none() || n <= mark) {
        restore_sharing(&g, &proto, &mat, Some(&parts).filter(|p| !p.is_none()))?;
        return Ok(shared);
    }
    let entry = level.call_method1("get", ("entry_transaction",))?;
    let inner = stack.get_item(PySlice::new(py, mark, n, 1))?;
    let step = if !entry.is_none()
        && !crate::model::module(py, "built_core.doc_model")?
            .getattr("NATIVE")?
            .call_method0("doc_depth")?
            .is_truthy()?
    {
        steps.call_method1("_StoreSession", (share, parts, inner, shared, entry))?
    } else if !parts.is_none() {
        steps.call_method1(
            "ReshareAssemblyCommand",
            (g, proto, mat, edited, state, parts, inner),
        )?
    } else {
        steps.call_method1("ReshareInstanceCommand", (g, proto, mat, edited, inner))?
    };
    history
        .getattr("_native")?
        .call_method1("close_group", (history, mark, step))?;
    Ok(shared)
}
#[pyclass(subclass, module = "built_core.command_protocol")]
pub(super) struct StoreSessionStep;
#[pymethods]
impl StoreSessionStep {
    #[new]
    #[pyo3(signature=(*_args,**_kw))]
    fn new(_args: &Bound<'_, PyTuple>, _kw: Option<&Bound<'_, PyDict>>) -> Self {
        Self
    }
    fn geometry_meshes<'py>(slf: &Bound<'py, Self>, scene: &Obj<'py>) -> PyResult<(Obj<'py>,)> {
        let _ = scene;
        Ok((slf.getattr("proto")?,))
    }
    fn r#do(slf: &Bound<'_, Self>, scene: &Obj<'_>) -> PyResult<()> {
        let replay = slf.getattr("replay")?;
        if !replay.is_none() {
            replay.call_method1("do", (scene,))?;
        } else {
            let g = slf.getattr("group")?;
            let proto = slf.getattr("proto")?;
            let mat = slf.getattr("xform")?;
            let p = slf.getattr("parts")?;
            let parts = Some(&p).filter(|v| !v.is_none());
            if slf.getattr("shared")?.is_truthy()? {
                share_back(&g, &proto, &mat, &slf.getattr("edited")?, parts)?;
            } else {
                restore_sharing(&g, &proto, &mat, parts)?;
            }
        }
        version(scene)
    }
    fn undo(slf: &Bound<'_, Self>, scene: &Obj<'_>) -> PyResult<()> {
        slf.getattr("replay")?.call_method1("undo", (scene,))?;
        version(scene)
    }
    fn doc_after(slf: &Bound<'_, Self>, scene: &Obj<'_>, phase: &str) -> PyResult<()> {
        crate::model::module(scene.py(), "built_core.history")?
            .call_method1("OccurrenceCommands", (slf.getattr("inner")?,))?
            .call_method1("doc_after", (scene, phase))?;
        version(scene)
    }
    fn bind_transaction(slf: &Bound<'_, Self>, txn: &Obj<'_>) -> PyResult<()> {
        let py = slf.py();
        let inner = slf.getattr("inner")?;
        let entry = slf.getattr("entry")?;
        let edits = PyList::empty(py);
        edits.append(&entry)?;
        let mut complete = true;
        for c in inner.try_iter()? {
            if let Some(tx) = c?.getattr_opt("_doc_txn")?.filter(|v| !v.is_none()) {
                edits.append(tx)?;
            } else {
                complete = false;
                break;
            }
        }
        if complete {
            edits.append(txn)?;
            return slf.setattr("_doc_txn", txn.call_method1("compose", (edits,))?);
        }
        let h = crate::model::module(py, "built_core.history")?;
        let compound = h.getattr("CompoundCommand")?;
        let open = compound.call1((PyList::empty(py),))?;
        let close = compound.call1((PyList::empty(py),))?;
        open.setattr("_doc_txn", entry)?;
        close.setattr("_doc_txn", txn)?;
        let replay = PyList::empty(py);
        replay.append(open)?;
        replay.call_method1("extend", (inner,))?;
        replay.append(close)?;
        slf.setattr("replay", h.call_method1("OccurrenceCommands", (replay,))?)?;
        slf.setattr("_doc_txn", py.None())
    }
}
