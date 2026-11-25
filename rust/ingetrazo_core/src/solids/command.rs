// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use pyo3::types::{PySet, PySlice};
fn select(scene: &Obj, seq: &Obj) -> PyResult<()> {
    let py = scene.py();
    scene.setattr("selection", py.get_type::<PySet>().call1((seq,))?)?;
    scene.setattr("version", scene.getattr("version")?.add(1)?)
}
#[pyfunction]
fn init(cmd: &Obj, op: &str, groups: &Obj) -> PyResult<()> {
    let py = cmd.py();
    cmd.setattr("op", op)?;
    cmd.setattr("groups", py.get_type::<PyList>().call1((groups,))?)?;
    for k in ["removed", "created", "_home", "_selection"] {
        cmd.setattr(k, py.None())?;
    }
    cmd.setattr("_slots", PyList::empty(py))
}
#[pyfunction]
fn prepare(cmd: &Obj) -> PyResult<()> {
    if cmd.getattr("created")?.is_none() {
        let (removed, made) = run(
            &cmd.getattr("op")?.extract::<String>()?,
            &cmd.getattr("groups")?,
        )?;
        cmd.setattr("removed", removed)?;
        cmd.setattr("created", made)?;
    }
    Ok(())
}
#[pyfunction]
fn doc_after(cmd: &Obj, scene: &Obj, phase: &str) -> PyResult<()> {
    select(
        scene,
        &cmd.getattr(if phase == "undo" {
            "_selection"
        } else {
            "created"
        })?,
    )
}
#[pyfunction]
fn apply(cmd: &Obj, scene: &Obj) -> PyResult<()> {
    prepare(cmd)?;
    let py = cmd.py();
    cmd.setattr(
        "_selection",
        py.get_type::<PySet>()
            .call1((scene.getattr("selection")?,))?,
    )?;
    let dest = if matches!(
        cmd.getattr("op")?.extract::<String>()?.as_str(),
        "trim" | "subtract"
    ) {
        -1
    } else {
        0
    };
    let home =
        crate::definition_edits::group_owner(scene, &cmd.getattr("groups")?.get_item(dest)?)?
            .unwrap_or(scene.getattr("groups")?);
    cmd.setattr("_home", &home)?;
    let slots = PyList::empty(py);
    let mut at = home.len()?;
    for g in cmd.getattr("removed")?.try_iter()? {
        let g = g?;
        let Some(list) = crate::definition_edits::group_owner(scene, &g)? else {
            continue;
        };
        let i: usize = list.call_method1("index", (&g,))?.extract()?;
        if list.is(&home) {
            at = at.min(i);
        }
        slots.append((list, i, g))?;
    }
    cmd.setattr("_slots", &slots)?;
    for row in slots {
        let (list, _, g): (Obj, usize, Obj) = row.extract()?;
        list.call_method1("remove", (&g,))?;
        scene.getattr("selection")?.call_method1("discard", (g,))?;
    }
    let created = cmd.getattr("created")?;
    home.set_item(PySlice::new(py, at as isize, at as isize, 1), &created)?;
    select(scene, &created)
}
#[pyfunction]
fn undo(cmd: &Obj, scene: &Obj) -> PyResult<()> {
    let home = cmd.getattr("_home")?;
    for g in cmd.getattr("created")?.try_iter()? {
        let g = g?;
        if home.contains(&g)? {
            home.call_method1("remove", (g,))?;
        }
    }
    let mut slots = cmd.getattr("_slots")?.extract::<Vec<(Obj, usize, Obj)>>()?;
    slots.sort_by_key(|s| s.1);
    for (list, i, g) in slots {
        list.call_method1("insert", (i.min(list.len()?), g))?;
    }
    select(scene, &cmd.getattr("_selection")?)
}
pub(super) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    let py = m.py();
    let ns = PyDict::new(py);
    ns.set_item("__module__", "core.solids")?;
    ns.set_item("doc_undo", true)?;
    let method = py.import("functools")?.getattr("partialmethod")?;
    for (k, f) in [
        ("__init__", wrap_pyfunction!(init, py)?),
        ("prepare", wrap_pyfunction!(prepare, py)?),
        ("do", wrap_pyfunction!(apply, py)?),
        ("undo", wrap_pyfunction!(undo, py)?),
        ("doc_after", wrap_pyfunction!(doc_after, py)?),
    ] {
        ns.set_item(k, method.call1((f,))?)?;
    }
    m.add(
        "SolidOperationCommand",
        py.import("builtins")?.call_method1(
            "type",
            (
                "SolidOperationCommand",
                (py.import("ingetrazo_core.history")?.getattr("Command")?,),
                ns,
            ),
        )?,
    )
}
