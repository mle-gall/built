// SPDX-License-Identifier: GPL-3.0-or-later
//! Route document constraints and the acquired reference into the tool context.
use super::*;
#[pyfunction]
fn release_linear_mode(v: &Obj) -> PyResult<()> {
    if !v.getattr("linear_inference_mode")?.eq("all")? && !busy(v, &tool(v)?)? {
        v.setattr("linear_inference_mode", "all")?;
        refresh(v)?;
        update(v)?
    }
    Ok(())
}
#[pyfunction]
fn release_axis_lock_after_operation(v: &Obj, committed: bool) -> PyResult<()> {
    if !v.getattr("axis_lock")?.is_none() && (committed || !busy(v, &tool(v)?)?) {
        reset(v, &["axis_lock"])?;
        refresh(v)?;
        update(v)?
    }
    Ok(())
}
#[pyfunction]
fn cycle_linear_inference_mode(v: &Obj) -> PyResult<()> {
    let mode = v.getattr("linear_inference_mode")?.extract::<String>()?;
    let (next, label) = match mode.as_str() {
        "all" => ("off", "Linear inferences: off"),
        "off" => (
            "parallel_perp",
            "Linear inferences: parallel / perpendicular only",
        ),
        "parallel_perp" => ("all", "Linear inferences: all on"),
        _ => return Err(pyo3::exceptions::PyKeyError::new_err(mode)),
    };
    v.setattr("linear_inference_mode", next)?;
    v.getattr("measurementChanged")?
        .call_method1("emit", (label,))?;
    refresh(v)
}
#[pyfunction]
fn release_constraints(v: &Obj) -> PyResult<bool> {
    if v.getattr("axis_lock")?.is_none() && v.getattr("reference_mode")?.is_none() {
        return Ok(false);
    }
    reset(v, &["axis_lock", "reference_edge", "reference_mode"])?;
    refresh(v)?;
    Ok(true)
}
#[pyfunction]
fn cycle_reference_mode(v: &Obj, edge: &Obj) -> PyResult<()> {
    let mode = v.getattr("reference_mode")?;
    if mode.is_none() {
        let mut e = edge.clone();
        if e.is_none() {
            e = v.getattr("_hover_edge")?;
            if e.is_none() {
                e = v.getattr("inference")?.getattr("_acquired_edge")?
            }
        }
        if !e.is_none() {
            v.setattr("reference_edge", e)?;
            v.setattr("reference_mode", "parallel")?
        }
    } else if mode.eq("parallel")? {
        v.setattr("reference_mode", "perpendicular")?
    } else {
        reset(v, &["reference_edge", "reference_mode"])?
    }
    Ok(())
}
fn context<'py>(
    v: &Obj<'py>,
    snap: &Obj<'py>,
    screen: Obj<'py>,
    mods: Obj<'py>,
) -> PyResult<Obj<'py>> {
    host(v.py())?.call_method1(
        "ToolContext",
        (v, snap.getattr("point")?, screen, mods, snap),
    )
}
#[pyfunction]
fn refresh_snap(v: &Obj) -> PyResult<()> {
    v.call_method0("_sync_axes")?;
    update(v)?;
    let screen = v.getattr("_last_mouse_pos")?;
    if screen.is_none() || tool(v)?.is_none() || !tool(v)?.getattr("uses_snap")?.is_truthy()? {
        return Ok(());
    }
    let mods = v
        .py()
        .import("built_core.window_input")?
        .call_method1("modifiers", (v,))?;
    let p = screen.call_method0("toPoint")?;
    let snap = v.call_method1(
        "_snap_at",
        (p.call_method0("x")?, p.call_method0("y")?, &mods),
    )?;
    if !snap.is_none() {
        v.call_method1("_note_snap", (&snap,))?;
        let ctx = context(v, &snap, screen, mods)?;
        tool(v)?.call_method1("on_hover", (ctx,))?;
        emit(
            v,
            "measurementChanged",
            &v.call_method0("_measurement_text")?,
        )?
    }
    Ok(())
}
#[pyfunction]
fn capture_shift_lock(v: &Obj) -> PyResult<()> {
    reset(v, &["_shift_lock", "_shift_lock_ref"])?;
    v.call_method0("_mark_plane_for_shift")?;
    let snap = v.getattr("last_snap")?;
    let t = tool(v)?;
    let owns = opt(&t, "owns_shift_lock")?;
    if !owns.is_none() && owns.call0()?.is_truthy()?
        || snap.is_none()
        || !v
            .getattr("_SHIFT_LOCKABLE")?
            .contains(snap.getattr("kind")?)?
    {
        return Ok(());
    }
    let start = opt(&t, "start_point")?;
    let axis = snap.getattr("axis")?;
    let h = host(v.py())?;
    let axes = h.getattr("_AXIS_VECTORS")?;
    let has_axis = axes.contains(&axis)?;
    let d = if has_axis {
        let vec = h.getattr("QVector3D")?;
        let mut d = vec.call1((axes.get_item(&axis)?,))?;
        if !start.is_none()
            && vec
                .call_method1("dotProduct", (snap.getattr("point")?.sub(&start)?, &d))?
                .lt(0.)?
        {
            d = d.neg()?
        }
        d
    } else if !start.is_none() {
        snap.getattr("point")?.sub(start)?
    } else {
        return Ok(());
    };
    if d.call_method0("length")?.gt(1e-6)? {
        v.setattr(
            "_shift_lock",
            (d.call_method0("normalized")?, snap.getattr("color")?),
        )?;
        if has_axis && snap.getattr("kind")?.eq("from_point")? {
            let guide = opt(&snap, "guide")?;
            if !guide.is_none() {
                v.setattr("_shift_lock_ref", (guide.get_item(0)?, axis))?
            }
        }
    }
    Ok(())
}
#[pyfunction]
fn mark_plane_for_shift(v: &Obj) -> PyResult<()> {
    reset(v, &["_shift_plane_mark"])?;
    let t = tool(v)?;
    if !t.is_none()
        && t.is_instance(&v.py().import("tools.base")?.getattr("PlaneLock")?)?
        && t.call_method1("mark_plane", (v,))?.is_truthy()?
    {
        v.setattr("_shift_plane_mark", t)?
    }
    Ok(())
}
#[pyfunction]
fn release_shift_plane_mark(v: &Obj) -> PyResult<()> {
    let t = opt(v, "_shift_plane_mark")?;
    reset(v, &["_shift_plane_mark"])?;
    let plane = opt(&t, "plane_ref")?;
    if !plane.is_none() && plane.get_item(1)?.eq("plane")? {
        reset(&t, &["plane_ref"])?
    }
    Ok(())
}
#[pyfunction]
fn document_snap_at<'py>(v: &Obj<'py>, x: f64, y: f64, mods: &Obj<'py>) -> PyResult<Obj<'py>> {
    let snap = read_only(v.py(), || {
        v.getattr("inference")?.call_method1("snap", (x, y, mods))
    })?;
    if !snap.is_none()
        && !host(v.py())?
            .call_method1("_finite_point", (snap.getattr("point")?,))?
            .is_truthy()?
    {
        return Ok(none(v.py()));
    }
    Ok(snap)
}
#[pyfunction]
fn build_ctx<'py>(v: &Obj<'py>, ev: &Obj<'py>) -> PyResult<Obj<'py>> {
    v.call_method0("_sync_axes")?;
    let screen = ev.call_method0("position")?;
    let pos = screen.call_method0("toPoint")?;
    let mods = ev.call_method0("modifiers")?;
    let snap = v.call_method1(
        "_snap_at",
        (pos.call_method0("x")?, pos.call_method0("y")?, &mods),
    )?;
    if snap.is_none() {
        Ok(snap)
    } else {
        context(v, &snap, screen, mods)
    }
}
#[pyfunction]
fn note_snap(v: &Obj, snap: &Obj) -> PyResult<()> {
    v.setattr("last_snap", snap)?;
    let t = tool(v)?;
    if !t.is_none() {
        t.setattr("snap_kind", opt(snap, "kind")?)?
    }
    Ok(())
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(release_linear_mode, m)?)?;
    m.add_function(wrap_pyfunction!(release_axis_lock_after_operation, m)?)?;
    m.add_function(wrap_pyfunction!(cycle_linear_inference_mode, m)?)?;
    m.add_function(wrap_pyfunction!(release_constraints, m)?)?;
    m.add_function(wrap_pyfunction!(cycle_reference_mode, m)?)?;
    m.add_function(wrap_pyfunction!(refresh_snap, m)?)?;
    m.add_function(wrap_pyfunction!(capture_shift_lock, m)?)?;
    m.add_function(wrap_pyfunction!(mark_plane_for_shift, m)?)?;
    m.add_function(wrap_pyfunction!(release_shift_plane_mark, m)?)?;
    m.add_function(wrap_pyfunction!(document_snap_at, m)?)?;
    m.add_function(wrap_pyfunction!(build_ctx, m)?)?;
    m.add_function(wrap_pyfunction!(note_snap, m)?)?;
    Ok(())
}
