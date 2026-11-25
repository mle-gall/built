// SPDX-License-Identifier: GPL-3.0-or-later
//! Solid and placement operations use the edit context's own sibling list.
use super::placements::one;
use super::*;
fn siblings<'py>(scene: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let owner = scene.getattr("edit_group")?;
    let items = if owner.is_truthy()? {
        owner.getattr("children")?
    } else {
        scene.getattr("groups")?
    };
    let by_uid = PyDict::new(scene.py());
    for g in items.try_iter()? {
        let g = g?;
        by_uid.set_item(g.getattr("uid")?, g)?;
    }
    Ok(by_uid)
}
fn group_list<'py>(scene: &Obj<'py>, uids: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let by = siblings(scene)?;
    let groups = PyList::empty(scene.py());
    for uid in uids.try_iter()? {
        groups.append(by.as_any().get_item(uid?)?)?;
    }
    Ok(groups)
}
#[pyfunction]
#[pyo3(signature=(history,mode,faces=None,placements=None))]
fn intersect_faces<'py>(
    history: &Obj<'py>,
    mode: &str,
    faces: Option<&Obj<'py>>,
    placements: Option<&Obj<'py>>,
) -> Reply<'py> {
    let py = history.py();
    let module = py.import("core.intersect")?;
    let scene = history.getattr("scene")?;
    let known = [
        module.getattr("WITH_CONTEXT")?,
        module.getattr("WITH_SELECTION")?,
        module.getattr("WITH_MODEL")?,
    ];
    if mode != "placements" && !known.iter().any(|k| k.eq(mode).unwrap_or(false)) {
        return Err(PyValueError::new_err(mode.to_owned()));
    }
    if placements.is_some() && (mode != "placements" || faces.is_some()) {
        return Err(PyValueError::new_err(
            "placement uids require placements mode",
        ));
    }
    let groups = placements
        .map(|p| {
            group_list(
                &scene,
                &py.get_type::<PyDict>().call_method1("fromkeys", (p,))?,
            )
        })
        .transpose()?;
    let keys = if let Some(faces) = faces {
        let keys = PyList::empty(py);
        let key = py.import("core.protocol.keys")?.getattr("from_json_key")?;
        for f in faces.try_iter()? {
            keys.append(key.call1((f?,))?)?;
        }
        Some(keys)
    } else {
        None
    };
    let result = module.call_method1("build_intersect_faces", (scene, mode, keys, groups))?;
    execute(history, &result.get_item(1)?)
}
#[pyfunction]
fn solid_operation<'py>(history: &Obj<'py>, operation: &str, placements: &Obj<'py>) -> Reply<'py> {
    let py = history.py();
    let m = py.import("core.solids")?;
    if !m.getattr("OPS")?.contains(operation)? {
        return Err(PyValueError::new_err(operation.to_owned()));
    }
    if py
        .get_type::<pyo3::types::PySet>()
        .call1((placements,))?
        .len()?
        != placements.len()?
    {
        return Err(PyValueError::new_err("duplicate placement uid"));
    }
    let scene = history.getattr("scene")?;
    let groups = group_list(&scene, placements)?;
    let before = history.getattr("revision")?;
    let cmd = m.call_method1("SolidOperationCommand", (operation, groups))?;
    if let Err(e) = cmd.call_method0("prepare") {
        if !e.value(py).is_instance(&m.getattr("SolidError")?)? {
            return Err(e);
        }
        let reply = answer(history, &before, false)?;
        reply.set_item("error", e.value(py).str()?)?;
        return Ok(reply);
    }
    execute(history, &cmd)
}
#[pyfunction]
#[pyo3(signature=(history,placement,fault=None))]
fn repair_solid<'py>(
    history: &Obj<'py>,
    placement: &Obj<'py>,
    fault: Option<&Obj<'py>>,
) -> Reply<'py> {
    let py = history.py();
    let group = if placement.is_none() {
        None
    } else {
        Some(one(history, placement)?)
    };
    let cmd = py.import("ingetrazo_core")?.call_method1(
        "plan_solid_fault",
        (history.getattr("scene")?, group, fault),
    )?;
    let reply = execute(history, &cmd)?;
    if cmd.is_none() {
        reply.set_item("error", py.None())?;
        reply.set_item("rejected", py.None())?;
        reply.set_item("repaired", PyDict::new(py))?;
    } else {
        reply.set_item(
            "repaired",
            py.get_type::<PyDict>().call1((cmd.getattr("done")?,))?,
        )?;
    }
    Ok(reply)
}
#[pyfunction]
fn explode_group<'py>(history: &Obj<'py>, placement: &Obj<'py>) -> Reply<'py> {
    for g in history.getattr("scene")?.getattr("groups")?.try_iter()? {
        let g = g?;
        if g.getattr_opt("uid")?
            .is_some_and(|id| id.eq(placement).unwrap_or(false))
        {
            return execute(
                history,
                &history_command(history.py(), "ExplodeGroupCommand", (g,))?,
            );
        }
    }
    Err(PyKeyError::new_err(format!(
        "no placement with uid {} in the open context",
        placement.repr()?
    )))
}
#[pyfunction]
fn split_into_pieces<'py>(history: &Obj<'py>, placement: &Obj<'py>) -> Reply<'py> {
    let g = one(history, placement)?;
    let py = history.py();
    let pieces = py
        .import("core.pieces")?
        .call_method1("split_into_pieces", (&g,))?;
    if !pieces.is_truthy()? {
        return answer(history, &history.getattr("revision")?, false);
    }
    execute(
        history,
        &history_command(py, "SplitIntoPiecesCommand", (g, pieces))?,
    )
}
#[pyfunction]
#[pyo3(signature=(history,placement,factor,mode="outward"))]
fn explode_view<'py>(
    history: &Obj<'py>,
    placement: &Obj<'py>,
    factor: &Obj<'py>,
    mode: &str,
) -> Reply<'py> {
    let py = history.py();
    let modes = py.import("core.explode")?.getattr("MODES")?;
    if !modes.contains(mode)? {
        return Err(PyValueError::new_err(format!(
            "mode must be one of {}",
            modes.str()?
        )));
    }
    execute(
        history,
        &history_command(
            py,
            "ExplodeViewCommand",
            (
                one(history, placement)?,
                py.get_type::<pyo3::types::PyFloat>().call1((factor,))?,
                mode,
            ),
        )?,
    )
}
#[pyfunction]
#[pyo3(signature=(history,layers=true,materials=true))]
fn purge_unused<'py>(
    history: &Obj<'py>,
    #[pyo3(from_py_with=PyAnyMethods::is_truthy)] layers: bool,
    #[pyo3(from_py_with=PyAnyMethods::is_truthy)] materials: bool,
) -> Reply<'py> {
    let py = history.py();
    let kw = PyDict::new(py);
    kw.set_item("layers", layers)?;
    kw.set_item("materials", materials)?;
    let cmd = py
        .import("core.history")?
        .call_method("PurgeUnusedCommand", (), Some(&kw))?;
    if cmd
        .call_method1("counts", (history.getattr("scene")?,))?
        .eq((0, 0))?
    {
        return answer(history, &history.getattr("revision")?, false);
    }
    execute(history, &cmd)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(intersect_faces, m)?)?;
    m.add_function(wrap_pyfunction!(solid_operation, m)?)?;
    m.add_function(wrap_pyfunction!(repair_solid, m)?)?;
    m.add_function(wrap_pyfunction!(explode_group, m)?)?;
    m.add_function(wrap_pyfunction!(split_into_pieces, m)?)?;
    m.add_function(wrap_pyfunction!(explode_view, m)?)?;
    m.add_function(wrap_pyfunction!(purge_unused, m)?)?;
    Ok(())
}
