// SPDX-License-Identifier: GPL-3.0-or-later
//! Shortcut override and key handling follow the same tool/value/document order
//! as the host. Returning false delegates an unclaimed event to the widget.
use super::*;
const ESCAPE: i32 = 0x1000000;
const DELETE: i32 = 0x1000007;
const LEFT: i32 = 0x1000012;
const UP: i32 = 0x1000013;
const RIGHT: i32 = 0x1000014;
const DOWN: i32 = 0x1000015;
const SHIFT: i32 = 0x1000020;
const CONTROL: i32 = 0x1000021;
const ALT: i32 = 0x1000023;

#[pyfunction]
fn document_event(v: &Obj, ev: &Obj) -> PyResult<bool> {
    let h = host(v.py())?;
    let codes = h.getattr("QEvent")?;
    let kind = ev.call_method0("type")?;
    if kind.eq(codes.getattr("NativeGesture")?)?
        && v.py()
            .import("built_render")?
            .getattr("trackpad_nav")?
            .call_method1("native_gesture", (v, ev))?
            .is_truthy()?
    {
        return Ok(true);
    }
    let override_ = kind.eq(codes.getattr("ShortcutOverride")?)?;
    if (override_ || kind.eq(codes.getattr("KeyPress")?)?)
        && !v
            .getattr("_MODIFIER_KEYS")?
            .contains(ev.call_method0("key")?)?
    {
        v.setattr("_ctrl_alone", false)?
    }
    if override_
        && (v.call_method1("_tool_claims_key", (ev,))?.is_truthy()?
            || v.call_method1("_measurement_claims_key", (ev, v.getattr("_value_buffer")?))?
                .is_truthy()?)
    {
        ev.call_method0("accept")?;
        return Ok(true);
    }
    Ok(false)
}
#[pyfunction]
fn measurement_claims_key(v: &Obj, ev: &Obj, buffer: &str) -> PyResult<bool> {
    super::value::claims(v, ev, buffer)
}
#[pyfunction]
fn tool_claims_key(v: &Obj, ev: &Obj) -> PyResult<bool> {
    let claims = opt(&tool(v)?, "claims_key")?;
    Ok(claims.is_callable()
        && claims
            .call1((ev.call_method0("key")?, ev.call_method0("modifiers")?))?
            .is_truthy()?)
}
#[pyfunction]
fn document_key_press(v: &Obj, ev: &Obj) -> PyResult<bool> {
    let code = ev.call_method0("key")?;
    let key: i32 = code.extract()?;
    let first = !ev.call_method0("isAutoRepeat")?.is_truthy()?;
    if first && key == CONTROL {
        v.setattr("_ctrl_alone", true)?
    }
    if v.call_method1("_tool_claims_key", (ev,))?.is_truthy()? {
        if first {
            tool(v)?.call_method1("on_key", (v, code, ev.call_method0("modifiers")?))?;
        }
        return Ok(true);
    }
    if first {
        if key == DOWN {
            reset(v, &["_shift_plane_mark"])?
        }
        if key == SHIFT {
            v.call_method1("_set_middle_drag_pan_mode", (true,))?;
            v.call_method0("_capture_shift_lock")?;
            refresh(v)?
        }
        if key == ALT {
            v.call_method1("_apply_alt_cursor", (true,))?;
        } else if v.getattr("_alt_tap")?.is_truthy()? {
            v.setattr("_alt_tap", false)?
        }
    }
    if v.call_method1("_handle_value_key", (ev,))?.is_truthy()? {
        return Ok(true);
    }
    let t = tool(v)?;
    if !t.is_none()
        && t.call_method1("on_key", (v, code, ev.call_method0("modifiers")?))?
            .is_truthy()?
    {
        return Ok(true);
    }
    match key {
        DELETE if first => {
            let t = tool(v)?;
            if !t.is_none()
                && v.call_method1("_tool_busy_for_delete", (&t,))?
                    .is_truthy()?
            {
                return Ok(false);
            }
            v.py()
                .import("tools.select")?
                .call_method1("delete_selection_or_hover", (v,))?;
        }
        ESCAPE => {
            v.call_method0("escape")?;
        }
        80 => {
            v.call_method0("toggle_projection")?;
        }
        ALT if first => {
            if busy(v, &tool(v)?)? {
                v.setattr("_alt_tap", true)?;
                v.call_method0("_cycle_linear_inference_mode")?;
            }
        }
        LEFT | UP | RIGHT => {
            let axis = match key {
                LEFT => "y",
                UP => "z",
                _ => "x",
            };
            if v.getattr("axis_lock")?.eq(axis)? {
                reset(v, &["axis_lock"])?
            } else {
                v.setattr("axis_lock", axis)?
            }
            refresh(v)?
        }
        DOWN => {
            v.call_method0("_cycle_reference_mode")?;
            refresh(v)?
        }
        _ => return Ok(false),
    }
    Ok(true)
}
#[pyfunction]
fn document_key_release(v: &Obj, ev: &Obj) -> PyResult<bool> {
    let release = opt(&tool(v)?, "on_key_release")?;
    let first = !ev.call_method0("isAutoRepeat")?.is_truthy()?;
    let code = ev.call_method0("key")?;
    if release.is_callable()
        && (if first {
            release.call1((v, &code))?.is_truthy()?
        } else {
            v.call_method1("_tool_claims_key", (ev,))?.is_truthy()?
        })
    {
        return Ok(true);
    }
    match code.extract::<i32>()? {
        ALT if first => {
            v.call_method1("_apply_alt_cursor", (false,))?;
            if v.getattr("_alt_tap")?.is_truthy()? {
                v.setattr("_alt_tap", false)?;
                ev.call_method0("accept")?;
                return Ok(true);
            }
        }
        SHIFT if first => {
            v.call_method1("_set_middle_drag_pan_mode", (false,))?;
            reset(v, &["_shift_lock", "_shift_lock_ref"])?;
            v.call_method0("_release_shift_plane_mark")?;
            refresh(v)?
        }
        _ => (),
    }
    Ok(false)
}
#[pyfunction]
fn document_escape(v: &Obj) -> PyResult<()> {
    if v.getattr("_value_buffer")?.is_truthy()? {
        v.call_method1("_set_value_buffer", ("",))?;
        return Ok(());
    }
    if v.call_method0("release_constraints")?.is_truthy()? {
        return Ok(());
    }
    let t = tool(v)?;
    let close = opt(&t, "close_array_window")?;
    if close.is_callable() && v.call_method0("_array_pending")?.is_truthy()? {
        close.call0()?;
        return Ok(());
    }
    if t.get_type().name()? == "PasteTool" {
        let activate = opt(&v.call_method0("window")?, "_activate_tool")?;
        if !activate.is_none() {
            activate.call1(("select",))?;
            return Ok(());
        }
    }
    if busy(v, &t)? {
        t.call_method1("on_cancel", (v,))?;
        v.call_method0("_release_linear_mode")?;
        return Ok(());
    }
    let scene = v.getattr("scene")?;
    if !scene.getattr("edit_group")?.is_none() {
        v.call_method0("clear_extension_pick")?;
        v.call_method0("end_one_group_edit")?;
    } else if scene.getattr("selection")?.is_truthy()? || !v.getattr("extension_pick")?.is_none() {
        scene.call_method0("clear_selection")?;
        v.call_method0("clear_extension_pick")?;
        update(v)?
    } else if !t.is_none() {
        t.call_method1("on_cancel", (v,))?;
    }
    Ok(())
}
#[pyfunction]
fn document_focus(v: &Obj, args: &Bound<PyTuple>) -> PyResult<bool> {
    let shown = v.getattr("keys_stand_in")?;
    if shown.is_none() || v.call_method0("isVisible")?.is_truthy()? {
        return Ok(false);
    }
    match shown.call_method0("isVisible") {
        Ok(visible) if visible.is_truthy()? => {
            shown.call_method1("setFocus", args)?;
            Ok(true)
        }
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyRuntimeError>(v.py()) => {
            reset(v, &["keys_stand_in"])?;
            Ok(false)
        }
        Err(e) => Err(e),
        _ => Ok(false),
    }
}
#[pyfunction]
fn document_focus_out(v: &Obj) -> PyResult<()> {
    for name in ["_alt_tap", "_alt_down", "_ctrl_alone"] {
        v.setattr(name, false)?
    }
    v.call_method0("_release_shift_plane_mark")?;
    let lost = opt(&tool(v)?, "on_focus_out")?;
    if lost.is_callable() {
        lost.call1((v,))?;
    }
    Ok(())
}
#[pyfunction]
fn alt_tapped(v: &Obj) -> PyResult<bool> {
    if !v.getattr("nav_mode")?.is_none() || !opt(&tool(v)?, "icon")?.eq("paint")? {
        return Ok(false);
    }
    let a = opt(&v.call_method0("window")?, "_act_eyedropper")?;
    if !a.is_none() {
        a.call_method1("setChecked", (!a.call_method0("isChecked")?.is_truthy()?,))?;
    } else {
        let paint = v.py().import("tools.paint")?.getattr("PaintTool")?;
        paint.setattr("sample_armed", !paint.getattr("sample_armed")?.is_truthy()?)?
    }
    v.call_method0("_apply_tool_cursor")?;
    Ok(true)
}
#[pyfunction]
fn apply_alt_cursor(v: &Obj, down: bool) -> PyResult<()> {
    v.setattr("_alt_down", down)?;
    if v.getattr("nav_mode")?.is_none() && opt(&tool(v)?, "icon")?.eq("paint")? {
        v.call_method0("_apply_tool_cursor")?;
    }
    Ok(())
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(document_event, m)?)?;
    m.add_function(wrap_pyfunction!(tool_claims_key, m)?)?;
    m.add_function(wrap_pyfunction!(measurement_claims_key, m)?)?;
    m.add_function(wrap_pyfunction!(document_key_press, m)?)?;
    m.add_function(wrap_pyfunction!(document_key_release, m)?)?;
    m.add_function(wrap_pyfunction!(document_escape, m)?)?;
    m.add_function(wrap_pyfunction!(document_focus, m)?)?;
    m.add_function(wrap_pyfunction!(document_focus_out, m)?)?;
    m.add_function(wrap_pyfunction!(alt_tapped, m)?)?;
    m.add_function(wrap_pyfunction!(apply_alt_cursor, m)?)?;
    Ok(())
}
