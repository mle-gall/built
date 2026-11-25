// SPDX-License-Identifier: GPL-3.0-or-later
//! Placement changes that a command can identify outside its document journal.
use pyo3::prelude::*;
type Obj<'py> = Bound<'py, PyAny>;
fn flag(obj: &Obj<'_>, field: &str) -> PyResult<bool> {
    obj.getattr_opt(field)?
        .map(|v| v.is_truthy())
        .transpose()
        .map(Option::unwrap_or_default)
}
pub(super) fn known(
    cmd: &Obj<'_>,
    phase: &str,
    step: &Obj<'_>,
    scene: &Obj<'_>,
    journalled: bool,
) -> PyResult<bool> {
    let py = cmd.py();
    let h = crate::model::module(py, "built_core.history")?;
    let is = |name: &str| cmd.is_instance(&h.getattr(name)?);
    if is("CompoundCommand")? {
        for child in cmd.getattr("commands")?.try_iter()? {
            if !known(&child?, phase, step, scene, journalled)? {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    let p = step.getattr("placements")?;
    let modified = p.get_item("modified")?;
    if let Some(target) = cmd.getattr_opt("_history_mesh")? {
        if let Some(def) = target
            .getattr_opt("_instance_definition")?
            .filter(|v| !v.is_none())
        {
            if !step.call_method1("mesh_change", (&def,))?.is_none() {
                modified.call_method1(
                    "extend",
                    (scene
                        .getattr("_root")?
                        .call_method1("_definition_uids", (def,))?,),
                )?;
            }
        }
    }
    if journalled && flag(cmd, "journal_covers")? {
        return Ok(true);
    }
    let group = cmd.getattr_opt("group")?;
    let uid = group
        .as_ref()
        .map(|g| g.getattr_opt("uid"))
        .transpose()?
        .flatten()
        .filter(|v| !v.is_none());
    if journalled
        && is("MoveGroupCommand")?
        && group
            .as_ref()
            .map(|g| g.getattr_opt("xform"))
            .transpose()?
            .flatten()
            .is_some_and(|v| !v.is_none())
    {
        return Ok(true);
    }
    let groups = || crate::model::module(py, "core.group")?.getattr("Group");
    if journalled && (is("HideCommand")? || is("AssignLayerCommand")?) {
        let g = groups()?;
        let mut all = true;
        for e in cmd.getattr("_entities")?.try_iter()? {
            if !e?.is_instance(&g)? {
                all = false;
                break;
            }
        }
        if all {
            return Ok(true);
        }
    }
    let append_uids = |field: &str, items: &Obj<'_>, lifted: bool| -> PyResult<()> {
        if items.is_none() {
            return Ok(());
        }
        let dst = p.get_item(field)?;
        for item in items.try_iter()? {
            let item = item?;
            let g = if lifted { item.get_item(0)? } else { item };
            dst.call_method1("append", (g.getattr("uid")?,))?;
        }
        Ok(())
    };
    if is("NativeCommand")? {
        if !journalled {
            append_uids("modified", &cmd.getattr("targets")?, false)?;
        }
        return flag(cmd, "journal_covers");
    }
    if flag(cmd, "keeps_rows")? {
        return Ok(true);
    }
    for name in [
        "SnapshotMutation",
        "SnapshotCompound",
        "MoveVerticesCommand",
        "AddEdgeCommand",
        "AddFaceCommand",
        "EraseSelectionCommand",
        "DeleteEdgesCommand",
        "DeleteFaceCommand",
    ] {
        if is(name)? {
            return Ok(true);
        }
    }
    let undo = phase == "undo";
    let made = if undo { "removed" } else { "created" };
    let gone = if undo { "created" } else { "removed" };
    if cmd
        .is_instance(&crate::model::module(py, "core.solids")?.getattr("SolidOperationCommand")?)?
    {
        append_uids("created", &cmd.getattr(made)?, false)?;
        append_uids("removed", &cmd.getattr(gone)?, false)?;
        return Ok(true);
    }
    if let Some(uid) = uid.filter(|v| v.is_truthy().unwrap_or(false)) {
        let field = if is("InsertGroupCommand")? {
            Some(made)
        } else if is("DeleteGroupCommand")? || is("ExplodeGroupCommand")? {
            Some(gone)
        } else if is("MoveGroupCommand")? || is("SplitIntoPiecesCommand")? {
            Some("modified")
        } else {
            None
        };
        if let Some(field) = field {
            p.get_item(field)?.call_method1("append", (uid,))?;
            if is("ExplodeGroupCommand")? {
                append_uids(made, &cmd.getattr("_lifted")?, true)?;
            } else if is("SplitIntoPiecesCommand")? {
                append_uids(made, &cmd.getattr("pieces")?, false)?;
            }
            return Ok(true);
        }
    }
    if flag(cmd, "reference_instance")? {
        let proto = cmd.getattr("proto")?;
        let mut stack = scene
            .getattr("groups")?
            .try_iter()?
            .collect::<PyResult<Vec<_>>>()?;
        while let Some(g) = stack.pop() {
            if g.getattr_opt("mesh")?.is_some_and(|v| v.is(&proto)) {
                modified.call_method1("append", (g.getattr("uid")?,))?;
            }
            if let Some(kids) = g.getattr_opt("children")?.filter(|v| !v.is_none()) {
                for k in kids.try_iter()? {
                    stack.push(k?);
                }
            }
        }
        modified.call_method0("sort")?;
        return Ok(true);
    }
    if is("ExplodeViewCommand")? {
        if let Some(k) = cmd.getattr("container")?.getattr_opt("children")? {
            append_uids("modified", &k, false)?;
        }
        return Ok(true);
    }
    if is("PurgeUnusedCommand")? {
        return Ok(true);
    }
    if is("HideCommand")? || is("AssignLayerCommand")? {
        let g = groups()?;
        for v in cmd.getattr("_entities")?.try_iter()? {
            let v = v?;
            if v.is_instance(&g)? {
                modified.call_method1("append", (v.getattr("uid")?,))?;
            }
        }
        return Ok(true);
    }
    if is("SetRecordCommand")? {
        step.setattr("complete", false)?;
        return Ok(true);
    }
    Ok(cmd.get_type().name()?.to_str()?.starts_with("SetFace") || is("SetRegistryCommand")?)
}
