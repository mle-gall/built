// SPDX-License-Identifier: GPL-3.0-or-later
//! PositionCameraHost callbacks on the live document.
use super::*;

fn drag_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    Ok(none(py))
}
fn value_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.walkthrough_host")?;
    let tool = a.get_item(0)?;
    (
        module.call_method1(
            "fmt_len",
            (tool.call_method1("host_eye_height", (py.None(), py.None()))?,),
        )?,
        py.None(),
    )
        .into_bound_py_any(py)
}
fn place<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.walkthrough_host")?;
    let base = a.get_item(2)?;
    let look_at = a.get_item(3)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_send", (&viewport, fields!(py; "type" => "place_camera", "base" => module.call_method1("_tuple", (&base,))?, "look" => module.call_method1("_tuple", (&look_at,))?).into_any(),))?;
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn host_set_eye_height<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let value = a.get_item(2)?;
    let walkthrough = imported(py, "tools.walkthrough")?.into_any();
    walkthrough.call_method1("set_eye_height", (&value,))?;
    Ok(none(py))
}
fn host_eye_placed<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.walkthrough_host")?;
    let height = a.get_item(2)?;
    let viewport = a.get_item(1)?;
    let flash = optional(&viewport, "flash_status")?;
    if !flash.is_none() {
        flash.call1((
            module.call_method(
                "tr",
                ("Eye placed {h} above the point — drag to look around, Walk to move",),
                Some(&fields!(py; "h" => module.call_method1("fmt_len", (&height,))?)),
            )?,
            4000,
        ))?;
    }
    Ok(none(py))
}
fn host_handoff<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    tool.setattr("_handoff", &key)?;
    Ok(none(py))
}

#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("drag_plane", Mode::Method, drag_plane, &["self", "viewport"], "", ""),
    ("value_label", Mode::Method, value_label, &["self"], "", ""),
    ("place", Mode::Method, place, &["self", "viewport", "base", "look_at=None"], "", ""),
    ("host_set_eye_height", Mode::Method, host_set_eye_height, &["self", "viewport", "value"], "", ""),
    ("host_eye_placed", Mode::Method, host_eye_placed, &["self", "viewport", "height"], "", ""),
    ("host_handoff", Mode::Method, host_handoff, &["self", "viewport", "key"], "", ""),
];
