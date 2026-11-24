// SPDX-License-Identifier: GPL-3.0-or-later
//! NorthTool callbacks on the live document.
use super::*;

fn value_is_unitless<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    true.into_bound_py_any(py)
}
fn vcb_caption<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    ("Angle").into_bound_py_any(py)
}
fn work_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.north_host")?;
    let tool = a.get_item(0)?;
    Ok(if !tool.getattr("centre")?.is_none() {
        (
            tool.getattr("centre")?,
            module.call_method1("Vec3", (0, 0, 1))?,
        )
            .into_bound_py_any(py)?
    } else {
        none(py)
    })
}
fn set_work_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    Ok(none(py))
}
fn pending_deg<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("pending",))
}
fn host_north_angle<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let vp = a.get_item(1)?;
    let scene_north_deg = py.import("core.sun")?.getattr("scene_north_deg")?;
    scene_north_deg.call1((optional(&vp, "scene")?,))
}
fn host_north_done<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.north_host")?;
    let changed: Obj<'py>;
    let deg: Obj<'py>;
    let result = a.get_item(2)?;
    let vp = a.get_item(1)?;
    let tr = py.import("core.i18n")?.getattr("tr")?;
    let fmt_angle_deg = py.import("core.units")?.getattr("fmt_angle_deg")?;
    [deg, changed] = unpack::<2>(&result)?;
    if changed.is_truthy()? {
        module.call_method1("refresh_north_display", (&vp,))?;
    }
    if vp.hasattr("flash_status")? {
        vp.call_method1(
            "flash_status",
            (
                tr.call1(("North: {a}",))?.call_method(
                    "format",
                    (),
                    Some(&fields!(py; "a" => fmt_angle_deg.call1((&deg,))?)),
                )?,
                4000,
            ),
        )?;
    }
    vp.call_method0("update")?;
    Ok(none(py))
}
fn commit<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let deg = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    tool.call_method1("on_value", (&vp, &deg))?;
    Ok(none(py))
}
fn value_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let fmt_angle_deg = py.import("core.units")?.getattr("fmt_angle_deg")?;
    let deg = tool.call_method0("pending_deg")?;
    if !deg.is_none() && !tool.getattr("hover_point")?.is_none() {
        return (fmt_angle_deg.call1((&deg,))?, tool.getattr("hover_point")?).into_bound_py_any(py);
    }
    Ok(none(py))
}

fn send<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.north_host")?;
    let event = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    tool.setattr("_scene", optional(&vp, "scene")?)?;
    py.import("builtins")?
        .getattr("super")?
        .call1((module.getattr("NorthTool")?, &tool))?
        .call_method1("_send", (&vp, &event))
}
fn rubber_band_lines<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.north_host")?;
    let tool = a.get_item(0)?;
    let scene_north_deg = py.import("core.sun")?.getattr("scene_north_deg")?;
    if tool.getattr("_during")?.is_none() {
        tool.getattr("_machine")?.call_method1(
            "set",
            (
                "north",
                scene_north_deg.call1((optional(&tool, "_scene")?,))?,
            ),
        )?;
    }
    py.import("builtins")?
        .getattr("super")?
        .call1((module.getattr("NorthTool")?, &tool))?
        .call_method0("rubber_band_lines")
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("value_is_unitless", Mode::Method, value_is_unitless, &["self"], "", ""),
    ("vcb_caption", Mode::Method, vcb_caption, &["self"], "", ""),
    ("work_plane", Mode::Property, work_plane, &["self"], "", ""),
    ("pending_deg", Mode::Method, pending_deg, &["self"], "", ""),
    ("_send", Mode::Method, send, &["self", "vp", "event"], "", ""),
    ("rubber_band_lines", Mode::Method, rubber_band_lines, &["self"], "", ""),
    ("host_north_angle", Mode::Method, host_north_angle, &["self", "vp", "_"], "", ""),
    ("host_north_done", Mode::Method, host_north_done, &["self", "vp", "result"], "", ""),
    ("commit", Mode::Method, commit, &["self", "vp", "deg"], "", ""),
    ("value_label", Mode::Method, value_label, &["self"], "", ""),
    ("work_plane", Mode::Setter, set_work_plane, &["self", "_"], "", ""),
];
