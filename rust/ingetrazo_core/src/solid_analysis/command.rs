// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use pyo3::types::PySet;

#[pyfunction]
#[pyo3(signature=(cmd,meshes,fix=None))]
fn init(cmd: &Obj<'_>, meshes: &Obj<'_>, fix: Option<&Obj<'_>>) -> PyResult<()> {
    let py = cmd.py();
    cmd.setattr("meshes", rows(meshes)?)?;
    let fix = match fix {
        Some(f) if f.is_truthy()? => f.clone(),
        _ => py.import("core.solid_analysis")?.getattr("repair_mesh")?,
    };
    cmd.setattr("fix", fix)?;
    for attr in ["before", "after", "interior_before", "interior_after"] {
        cmd.setattr(attr, PyList::empty(py))?;
    }
    cmd.setattr("done", PyDict::new(py))
}
#[pyfunction]
fn interior<'p>(mesh: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let flags = PyDict::new(mesh.py());
    for f in mesh.getattr("faces")?.try_iter()? {
        let f = f?;
        flags.set_item(&f, f.getattr("interior")?)?;
    }
    Ok(flags)
}
#[pyfunction]
fn touch(scene: &Obj<'_>) -> PyResult<()> {
    let mesh = scene.getattr("mesh")?;
    let alive = PySet::empty(scene.py())?;
    for k in ["edges", "faces"] {
        for entity in mesh.getattr(k)?.try_iter()? {
            alive.add(entity?)?;
        }
    }
    let selected = scene.getattr("selection")?;
    for entity in rows(&selected)? {
        if (entity.hasattr("v0")? || entity.hasattr("loop")?) && !alive.contains(&entity)? {
            selected.call_method1("discard", (entity,))?;
        }
    }
    scene
        .py()
        .import("ingetrazo_core")?
        .call_method1("dirty_group_chunks", (scene,))?;
    scene.setattr("version", scene.getattr("version")?.add(1)?)
}
#[pyfunction]
fn restore(cmd: &Obj<'_>, scene: &Obj<'_>, snaps: &Obj<'_>, marks: &Obj<'_>) -> PyResult<()> {
    let meshes = cmd.getattr("meshes")?;
    for ((m, snap), flags) in meshes
        .try_iter()?
        .zip(snaps.try_iter()?)
        .zip(marks.try_iter()?)
    {
        let mesh = m?;
        mesh.call_method1("restore_state", (snap?,))?;
        for (f, v) in flags?.cast::<PyDict>()?.iter() {
            f.setattr("interior", v)?;
        }
        repair::winding(&mesh)?;
    }
    touch(scene)
}
#[pyfunction]
fn apply(cmd: &Obj<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let py = cmd.py();
    if cmd.getattr("after")?.is_truthy()? {
        return restore(
            cmd,
            scene,
            &cmd.getattr("after")?,
            &cmd.getattr("interior_after")?,
        );
    }
    let before = PyList::empty(py);
    let after = PyList::empty(py);
    let old_marks = PyList::empty(py);
    let new_marks = PyList::empty(py);
    let totals = PyDict::new(py);
    let capture = (|| -> PyResult<()> {
        for mesh in cmd.getattr("meshes")?.try_iter()? {
            let mesh = mesh?;
            before.append(mesh.call_method0("capture_state")?)?;
            old_marks.append(interior(&mesh)?)?;
            let counts = cmd.call_method1("fix", (&mesh,))?;
            for (name, count) in counts.cast::<PyDict>()?.iter() {
                let old = totals
                    .get_item(&name)?
                    .map(|n| n.extract::<i64>())
                    .transpose()?
                    .unwrap_or(0);
                totals.set_item(name, old + count.extract::<i64>()?)?;
            }
            after.append(mesh.call_method0("capture_state")?)?;
            new_marks.append(interior(&mesh)?)?;
        }
        Ok(())
    })();
    if let Err(e) = capture {
        // Restore the current mesh too if a custom fixer failed after editing it.
        restore(cmd, scene, before.as_any(), old_marks.as_any())?;
        return Err(e);
    }
    for (key, v) in [
        ("before", before),
        ("after", after),
        ("interior_before", old_marks),
        ("interior_after", new_marks),
    ] {
        cmd.setattr(key, v)?;
    }
    cmd.setattr("done", totals)?;
    touch(scene)
}
#[pyfunction]
fn undo(cmd: &Obj<'_>, scene: &Obj<'_>) -> PyResult<()> {
    restore(
        cmd,
        scene,
        &cmd.getattr("before")?,
        &cmd.getattr("interior_before")?,
    )
}
pub(super) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    let py = m.py();
    let ns = PyDict::new(py);
    ns.set_item("__module__", "core.solid_analysis")?;
    let bind = py.import("functools")?.getattr("partialmethod")?;
    for (name, func) in [
        ("__init__", wrap_pyfunction!(init, py)?),
        ("do", wrap_pyfunction!(apply, py)?),
        ("undo", wrap_pyfunction!(undo, py)?),
        ("_restore", wrap_pyfunction!(restore, py)?),
    ] {
        ns.set_item(name, bind.call1((func,))?)?;
    }
    let builtin = py.import("builtins")?;
    for (name, func) in [
        ("_interior", wrap_pyfunction!(interior, py)?),
        ("_touch", wrap_pyfunction!(touch, py)?),
    ] {
        ns.set_item(name, builtin.call_method1("staticmethod", (func,))?)?;
    }
    m.add(
        "RepairSolidsCommand",
        builtin.call_method1(
            "type",
            (
                "RepairSolidsCommand",
                (py.import("ingetrazo_core.history")?.getattr("Command")?,),
                ns,
            ),
        )?,
    )
}
