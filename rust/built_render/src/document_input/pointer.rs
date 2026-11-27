// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use std::time::Instant;
#[pyfunction]
fn dispatch_tool_click(v: &Obj, event: &Obj, double: bool) -> PyResult<()> {
    let py = v.py();
    let t = tool(v)?;
    let started = !opt(&t, "start_point")?.is_none();
    let plane = !opt(&t, "work_plane")?.is_none();
    let (face, group) = if !started && !plane {
        read_only(py, || {
            let pos = event.call_method0("position")?;
            v.call_method1(
                "pick_face_placement",
                (pos.call_method0("x")?, pos.call_method0("y")?),
            )?
            .extract::<(Obj, Obj)>()
        })?
    } else {
        (none(py), none(py))
    };
    let ctx = v.call_method1("_build_ctx", (event,))?;
    if ctx.is_none() {
        return Ok(());
    }
    v.call_method1("_note_snap", (ctx.getattr("snap")?,))?;
    let hist = v.getattr("history")?;
    let before = hist.getattr("undo_stack")?.len()?;
    tool(v)?.call_method1(
        if double {
            "on_double_click"
        } else {
            "on_click"
        },
        (ctx,),
    )?;
    let drew = hist.getattr("undo_stack")?.len()? > before;
    let active = tool(v)?;
    if !started
        && !face.is_none()
        && !opt(&active, "start_point")?.is_none()
        && active
            .getattr_opt("work_plane")?
            .is_some_and(|p| p.is_none())
    {
        active.setattr(
            "work_plane",
            py.import("built_core.snap")?
                .call_method1("face_plane_world", (face, opt(&group, "xform")?))?,
        )?
    }
    v.call_method1("_set_value_buffer", ("",))?;
    let err = hist.getattr("last_error")?;
    if err.is_truthy()? {
        let fields = PyDict::new(py);
        fields.set_item("err", err)?;
        v.call_method1(
            "flash_status",
            (
                host(py)?
                    .getattr("tr")?
                    .call(("Operation failed and was undone: {err}",), Some(&fields))?,
                8000,
            ),
        )?;
    }
    let kw = PyDict::new(py);
    kw.set_item("committed", drew)?;
    v.call_method("_release_axis_lock_after_operation", (), Some(&kw))?;
    if drew {
        v.call_method0("_reassert_tool_cursor")?;
    }
    update(v)
}
#[pyfunction]
fn reassert_tool_cursor(v: &Obj) -> PyResult<()> {
    if !tool(v)?.is_none() && v.getattr("nav_mode")?.is_none() && v.getattr("_look_drag")?.is_none()
    {
        v.call_method0("unsetCursor")?;
        v.call_method0("_apply_tool_cursor")?;
    }
    Ok(())
}
#[pyfunction]
fn process_hover(v: &Obj, pos: &Obj, mods: &Obj) -> PyResult<()> {
    if !v.getattr("_last_pos")?.is_none()
        || v.getattr("_box_active")?.is_truthy()?
        || !v.getattr("nav_mode")?.is_none()
    {
        return Ok(());
    }
    let tick = v
        .getattr_opt("_tick")?
        .map(|n| n.extract::<u64>())
        .transpose()?
        .unwrap_or(0);
    v.setattr("_tick", tick + 1)?;
    let py = v.py();
    let h = host(py)?;
    let ev = h.call_method1("_HoverEvent", (pos, mods))?;
    let mut clock = h.getattr("_PERF")?.is_truthy()?.then(Instant::now);
    let mut mark = |name: &str| -> PyResult<()> {
        if let Some(start) = clock.as_mut() {
            let now = Instant::now();
            h.call_method1(
                "_plog",
                (name, (now - *start).as_secs_f64() * 1000., "", 200.),
            )?;
            *start = now
        }
        Ok(())
    };
    let p = ev.call_method0("position")?;
    v.setattr("_last_mouse_pos", &p)?;
    v.call_method1("_emit_coordinate", (&ev,))?;
    mark("hover.coord")?;
    let ctx = read_only(py, || {
        v.getattr("inference")?
            .call_method1("acquire", (p.call_method0("x")?, p.call_method0("y")?))?;
        if tool(v)?.is_none() {
            return Ok(None);
        }
        mark("hover.acquire")?;
        Ok(Some(v.call_method1("_build_ctx", (&ev,))?))
    })?;
    let Some(ctx) = ctx else { return Ok(()) };
    mark("hover.ctx")?;
    if ctx.is_none() {
        return Ok(());
    }
    let snap = ctx.getattr("snap")?;
    v.call_method1("_note_snap", (&snap,))?;
    let t = tool(v)?;
    if t.getattr("uses_snap")?.is_truthy()?
        && t.getattr_opt("allow_inferences")?
            .map(|f| f.is_truthy())
            .transpose()?
            .unwrap_or(true)
    {
        let center = opt(v, "_hover_center")?;
        let mut point = if v
            .getattr("_ENCOURAGING_KINDS")?
            .contains(snap.getattr("kind")?)?
        {
            snap.getattr("point")?
        } else {
            none(py)
        };
        if !center.is_none() && (point.is_none() || center.get_item(2)?.get_item(0)?.eq("edge")?) {
            point = h.call_method1("QVector3D", (center.get_item(0)?,))?
        }
        v.call_method1("_dwell_on", (point,))?;
    }
    tool(v)?.call_method1("on_hover", (ctx,))?;
    mark("hover.tool")?;
    emit(
        v,
        "measurementChanged",
        &v.call_method0("_measurement_text")?,
    )?;
    mark("hover.measure")?;
    update(v)
}
#[pyfunction]
fn document_wheel(v: &Obj, ev: &Obj) -> PyResult<()> {
    let py = v.py();
    if py
        .import("views.ndof_input")?
        .call_method0("cap_in_use")?
        .is_truthy()?
    {
        ev.call_method0("accept")?;
        return Ok(());
    }
    v.setattr("_input_t", py.import("time")?.call_method0("monotonic")?)?;
    let serial = v
        .getattr_opt("_input_serial")?
        .map(|n| n.extract::<u64>())
        .transpose()?
        .unwrap_or(0);
    v.setattr("_input_serial", serial + 1)?;
    if ev
        .call_method0("modifiers")?
        .bitand(host(py)?.getattr("Qt")?.getattr("ShiftModifier")?)?
        .is_truthy()?
    {
        let tool = tool(v)?;
        if opt(&tool, "accepts_array")?.is_truthy()?
            && v.call_method0("_array_pending")?.is_truthy()?
        {
            let direction = ev
                .call_method0("angleDelta")?
                .call_method0("y")?
                .extract::<f64>()?
                .signum() as i64;
            let n = tool
                .call_method1("_state", ("repeat_count",))?
                .extract::<i64>()
                .unwrap_or(1);
            if direction != 0 {
                tool.call_method1("on_array_value", (v, (n + direction).clamp(1, 10000), "x"))?;
            }
            ev.call_method0("accept")?;
            return update(v);
        }
    }
    if crate::navigation::slide(v, ev)? {
        return update(v);
    }
    let steps = crate::navigation::zoom_steps(ev, v.getattr("_invert_wheel")?.is_truthy()?)?;
    if steps == 0. {
        ev.call_method0("accept")?;
        return Ok(());
    }
    let now = Instant::now();
    let (projected, reused): (Obj, Obj) = v
        .call_method1("_zoom_at", (steps, ev.call_method0("position")?))?
        .extract()?;
    let h = host(py)?;
    if h.getattr("_PERF")?.is_truthy()? {
        let reuse = if projected.is_truthy()? {
            "proj".to_owned()
        } else {
            reused.str()?.to_string()
        };
        h.call_method1(
            "_plog",
            (
                "wheel",
                now.elapsed().as_secs_f64() * 1000.,
                format!("reused={reuse}"),
                10.,
            ),
        )?;
    }
    update(v)
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(dispatch_tool_click, m)?)?;
    m.add_function(wrap_pyfunction!(reassert_tool_cursor, m)?)?;
    m.add_function(wrap_pyfunction!(process_hover, m)?)?;
    m.add_function(wrap_pyfunction!(document_wheel, m)?)?;
    Ok(())
}
