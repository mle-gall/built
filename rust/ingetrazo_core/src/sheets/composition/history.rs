// SPDX-License-Identifier: GPL-3.0-or-later
//! Reversible sheet edits backed by the document journal. Script commands may
//! still provide their own undo and redo methods.
use super::*;
use crate::presentation as edit;
pub(super) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    let py = m.py();
    let b = py.import("builtins")?;
    for (name, parent) in [
        ("ComposerCommand", "object"),
        ("CompoundCommand", "ComposerCommand"),
        ("AddItemCommand", "ComposerCommand"),
        ("RemoveItemCommand", "ComposerCommand"),
        ("EditItemCommand", "ComposerCommand"),
        ("ComposerHistory", "object"),
    ] {
        let ns = PyDict::new(py);
        ns.set_item("__module__", "core.composition")?;
        match name {
            "ComposerCommand" => {
                methods!(ns,py;"journalled"=>unjournalled,"restore_preview"=>no_preview,"do"=>abstract_command,"undo"=>abstract_command)
            }
            "CompoundCommand" => {
                methods!(ns,py;"__init__"=>init_compound,"journalled"=>journalled_children,"restore_preview"=>restore_children,"do"=>apply_children,"undo"=>undo_children)
            }
            "AddItemCommand" => {
                methods!(ns,py;"__init__"=>init_item,"journalled"=>sheet_record,"do"=>append,"undo"=>restore_removed)
            }
            "RemoveItemCommand" => {
                methods!(ns,py;"__init__"=>init_item,"journalled"=>sheet_record,"do"=>remove,"undo"=>undo_remove)
            }
            "EditItemCommand" => {
                methods!(ns,py;"__init__"=>init_edit,"journalled"=>journalled_edit,"restore_preview"=>restore_preview,"do"=>apply_edit,"undo"=>undo_edit)
            }
            _ => {
                methods!(ns,py;"__init__"=>init_history,"execute"=>execute,"undo"=>undo,"redo"=>redo)
            }
        }
        let base = if parent == "object" {
            b.getattr(parent)?
        } else {
            m.getattr(parent)?
        };
        m.add(name, b.call_method1("type", (name, (base,), ns))?)?;
    }
    Ok(())
}
#[pyfunction]
fn unjournalled(_cmd: &Obj) -> bool {
    false
}
#[pyfunction]
fn no_preview(_cmd: &Obj) {}
#[pyfunction]
fn abstract_command(_cmd: &Obj) -> PyResult<()> {
    Err(pyo3::exceptions::PyNotImplementedError::new_err(()))
}
#[pyfunction]
fn init_compound(cmd: &Obj, commands: &Obj) -> PyResult<()> {
    cmd.setattr(
        "commands",
        cmd.py().get_type::<PyList>().call1((commands,))?,
    )
}
#[pyfunction]
fn journalled_children(cmd: &Obj) -> PyResult<bool> {
    for child in cmd.getattr("commands")?.try_iter()? {
        if !child?.call_method0("journalled")?.is_truthy()? {
            return Ok(false);
        }
    }
    Ok(true)
}
fn visit(cmd: &Obj, action: &str, reverse: bool) -> PyResult<()> {
    let list = cmd.getattr("commands")?;
    for i in 0..list.len()? {
        let j = if reverse { list.len()? - 1 - i } else { i };
        list.get_item(j)?.call_method0(action)?;
    }
    Ok(())
}
#[pyfunction]
fn apply_children(cmd: &Obj) -> PyResult<()> {
    visit(cmd, "do", false)
}
#[pyfunction]
fn undo_children(cmd: &Obj) -> PyResult<()> {
    visit(cmd, "undo", true)
}
#[pyfunction]
fn restore_children(cmd: &Obj) -> PyResult<()> {
    visit(cmd, "restore_preview", true)
}
#[pyfunction]
fn init_item(cmd: &Obj, comp: &Obj, item: &Obj) -> PyResult<()> {
    cmd.setattr("comp", comp)?;
    cmd.setattr("item", item)
}
fn is_record(item: &Obj) -> PyResult<bool> {
    item.is_instance(&item.py().import("core.doc_model")?.getattr("Record")?)
}
#[pyfunction]
fn sheet_record(cmd: &Obj) -> PyResult<bool> {
    is_record(&cmd.getattr("comp")?)
}
fn objects<'p>(cmd: &Obj<'p>) -> PyResult<(Obj<'p>, Obj<'p>, Obj<'p>)> {
    let comp = cmd.getattr("comp")?;
    let item = cmd.getattr("item")?;
    let ls = edit::sheet_container(&comp, &item)?
        .unwrap_or_else(|| cmd.py().None().into_bound(cmd.py()));
    Ok((comp, item, ls))
}
#[pyfunction]
fn append(cmd: &Obj) -> PyResult<()> {
    let (s, i, l) = objects(cmd)?;
    cmd.setattr("_prev", edit::append_sheet_item(&s, &i, &l)?)
}
#[pyfunction]
fn undo_remove(cmd: &Obj) -> PyResult<()> {
    let (s, i, l) = objects(cmd)?;
    edit::append_sheet_item(&s, &i, &l)?;
    Ok(())
}
#[pyfunction]
fn remove(cmd: &Obj) -> PyResult<()> {
    let (s, i, l) = objects(cmd)?;
    edit::remove_sheet_item(&s, &i, &l, cmd.py().None().bind(cmd.py()))
}
#[pyfunction]
fn restore_removed(cmd: &Obj) -> PyResult<()> {
    let (s, i, l) = objects(cmd)?;
    edit::remove_sheet_item(&s, &i, &l, &cmd.getattr("_prev")?)
}
#[pyfunction(signature=(cmd,item,changes,before=None))]
fn init_edit(cmd: &Obj, item: &Obj, changes: &Obj, before: Option<&Obj>) -> PyResult<()> {
    let py = cmd.py();
    cmd.setattr("item", item)?;
    cmd.setattr("after", py.get_type::<PyDict>().call1((changes,))?)?;
    cmd.setattr("previewed", before.is_some())?;
    cmd.setattr(
        "before",
        edit::capture_sheet_fields(item, changes, before.unwrap_or(py.None().bind(py)))?,
    )
}
#[pyfunction]
fn journalled_edit(cmd: &Obj) -> PyResult<bool> {
    let item = cmd.getattr("item")?;
    if !is_record(&item)? {
        return Ok(false);
    }
    let fields = item.getattr("_record_fields")?;
    for key in cmd.getattr("after")?.try_iter()? {
        if !fields.contains(key?)? {
            return Ok(false);
        }
    }
    Ok(true)
}
#[pyfunction]
fn restore_preview(cmd: &Obj) -> PyResult<()> {
    if flag(cmd, "previewed")? {
        cmd.call_method0("undo")?;
    }
    Ok(())
}
#[pyfunction]
fn apply_edit(cmd: &Obj) -> PyResult<()> {
    edit::restore_fields(&cmd.getattr("item")?, &cmd.getattr("after")?)
}
#[pyfunction]
fn undo_edit(cmd: &Obj) -> PyResult<()> {
    edit::restore_fields(&cmd.getattr("item")?, &cmd.getattr("before")?)
}
pub(crate) fn edits_of<'p>(cmd: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    let m = api(cmd.py())?;
    let edit = m.getattr("EditItemCommand")?;
    if cmd.is_instance(&edit)? {
        return Ok(vec![cmd.clone()]);
    }
    let mut out = Vec::new();
    if cmd.is_instance(&m.getattr("CompoundCommand")?)? {
        for c in cmd.getattr("commands")?.try_iter()? {
            let c = c?;
            if !c.is_instance(&edit)? {
                return Ok(vec![]);
            }
            out.push(c);
        }
    }
    Ok(out)
}
pub(crate) fn edit_signature(
    cmd: &Obj,
) -> PyResult<Vec<(usize, std::collections::BTreeSet<String>)>> {
    edits_of(cmd)?
        .into_iter()
        .map(|e| {
            Ok((
                e.getattr("item")?.as_ptr() as usize,
                e.getattr("after")?
                    .call_method0("keys")?
                    .try_iter()?
                    .map(|k| k?.extract())
                    .collect::<PyResult<_>>()?,
            ))
        })
        .collect()
}
#[pyfunction(signature=(history,on_change=None))]
fn init_history(history: &Obj, on_change: Option<&Obj>) -> PyResult<()> {
    let native = history
        .py()
        .import("ingetrazo_core")?
        .call_method0("DocumentHistory")?;
    history.setattr("_undo", native.getattr("undo")?)?;
    history.setattr("_redo", native.getattr("redo")?)?;
    history.setattr("_native", native)?;
    history.setattr("_on_change", on_change)
}
fn notify(history: &Obj) -> PyResult<()> {
    let cb = history.getattr("_on_change")?;
    if cb.is_truthy()? {
        cb.call0()?;
    }
    Ok(())
}
#[pyfunction(signature=(history,cmd,notify=true,coalesce=false))]
fn execute(history: &Obj, cmd: &Obj, notify: bool, coalesce: bool) -> PyResult<()> {
    let py = history.py();
    let mut txn = py.None().into_bound(py);
    if cmd.call_method0("journalled")?.is_truthy()? {
        cmd.call_method0("restore_preview")?;
        let doc = py.import("core.doc_model")?;
        let depth = doc.call_method0("begin")?.is_truthy()?;
        let result = cmd.call_method0("do").and_then(|_| {
            if depth {
                doc.call_method0("commit")
            } else {
                Ok(py.None().into_bound(py))
            }
        });
        match result {
            Ok(v) => txn = v,
            Err(e) => {
                if depth {
                    doc.call_method0("rollback")?;
                }
                return Err(e);
            }
        }
    } else {
        cmd.call_method0("do")?;
    }
    cmd.setattr("_txn", txn)?;
    history
        .getattr("_native")?
        .call_method1("push_sheet", (cmd, coalesce))?;
    history.getattr("_redo")?.call_method0("clear")?;
    if notify {
        self::notify(history)?;
    }
    Ok(())
}
fn replay(history: &Obj, undo: bool) -> PyResult<bool> {
    if !flag(history, if undo { "_undo" } else { "_redo" })? {
        return Ok(false);
    }
    history
        .getattr("_native")?
        .call_method1("replay", (history, undo, true))?;
    notify(history)?;
    Ok(true)
}
#[pyfunction]
fn undo(history: &Obj) -> PyResult<bool> {
    replay(history, true)
}
#[pyfunction]
fn redo(history: &Obj) -> PyResult<bool> {
    replay(history, false)
}
