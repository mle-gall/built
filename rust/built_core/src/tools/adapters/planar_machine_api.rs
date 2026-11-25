// SPDX-License-Identifier: GPL-3.0-or-later
//! _PlanarMachine callbacks on the live document.
use super::*;

fn plane_state<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("_machine")
}
fn frame<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let axes = imported(py, "built_core.axes")?.into_any();
    let point_snap_kinds = imported(py, "core.snap")?.getattr("POINT_SNAP_KINDS")?;
    let kind = optional(&tool, "snap_kind")?;
    tool.call_method1("_send", (&viewport, fields!(py; "type" => "frame", "axes" => axes.call_method0("frame")?, "names_point" => if kind.is_none() { none(py) } else { (point_snap_kinds.contains(&kind)?).into_bound_py_any(py)? }).into_any(),))?;
    Ok(none(py))
}
fn send_framed<'py>(a: &Args<'py>) -> Reply<'py> {
    let event = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_frame", (&viewport,))?;
    tool.call_method1("_send", (&viewport, &event))
}
fn cursor_preview<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let mut _unused: Obj<'py>;
    let normal: Obj<'py>;
    let point: Obj<'py>;
    let tool = a.get_item(0)?;
    let u: Obj<'py>;
    let v: Obj<'py>;
    let vp: Obj<'py>;
    [vp, point] = unpack::<2>(
        &(tool.getattr("_viewport")?, tool.getattr("hover_point")?).into_bound_py_any(py)?,
    )?;
    if vp.is_none() || point.is_none() {
        return Ok(PyList::empty(py).into_any());
    }
    let plane_axes = imported(py, "built_core.axes")?.getattr("plane_axes")?;
    [_unused, normal] = unpack::<2>(&tool.call_method1("preview_plane", (&vp, &point))?)?;
    [u, v] = unpack::<2>(&plane_axes.call1((&normal,))?)?;
    let scale = tool.call_method1("plane_preview_scale", (&vp, &point, &u, &v))?;
    if scale.is_none() {
        return Ok(PyList::empty(py).into_any());
    }
    tool.getattr("_machine")?.call_method1(
        "cursor_lines",
        (
            &point,
            (u.call_method0("toTuple")?, v.call_method0("toTuple")?).into_bound_py_any(py)?,
            tool.getattr("PREVIEW_PX")?.mul(&scale)?,
        ),
    )
}

fn rubber_band_lines<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let tool = a.get_item(0)?;
    Ok(if tool.getattr("start_point")?.is_none() {
        tool.call_method0("_cursor_preview")?
    } else {
        imported(py, "builtins")?
            .getattr("super")?
            .call1((module.getattr("_PlanarMachine")?, &tool))?
            .call_method0("rubber_band_lines")?
    })
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("_plane_state", Mode::Property, plane_state, &["self"], "", ""),
    ("_frame", Mode::Method, frame, &["self", "viewport"], "", ""),
    ("_send_framed", Mode::Method, send_framed, &["self", "viewport", "event"], "", ""),
    ("_cursor_preview", Mode::Method, cursor_preview, &["self"], "", ""),
    ("rubber_band_lines", Mode::Method, rubber_band_lines, &["self"], "", ""),
];
