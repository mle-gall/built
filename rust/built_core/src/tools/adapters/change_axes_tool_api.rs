// SPDX-License-Identifier: GPL-3.0-or-later
//! ChangeAxesTool callbacks on the live document.
use super::*;

fn host_axes_target<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok(if !tool.getattr("target")?.is_none() {
        tool.getattr("target")?.getattr("uid")?
    } else {
        none(py)
    })
}
fn host_axes_frame<'py>(a: &Args<'py>) -> Reply<'py> {
    let scene = a.get_item(1)?.getattr("scene")?;
    let frame = imported(a.py(), "built_core.group")?
        .call_method1("frame_axes", (scene.getattr("tool_drawing_frame")?,))?;
    let values = frame
        .try_iter()?
        .map(|p| p?.call_method0("toTuple"))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(PyList::new(a.py(), values)?.into_any())
}

fn on_deactivate<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.setattr("target", none(a.py()))?;
    tool.setattr("align_face", false)?;
    imported(a.py(), "tools.rust_tool_base")?
        .getattr("RustTool")?
        .call_method1("on_deactivate", (tool, a.get_item(1)?))
}

fn host_axes_message<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let key = a.get_item(2)?;
    let vp = a.get_item(1)?;
    let tr = imported(py, "built_core.i18n")?.getattr("tr")?;
    if key.eq("collinear")? {
        vp.call_method1(
            "flash_status",
            (
                tr.call1(("The green direction cannot run along red — click off the red axis",))?,
                4000,
            ),
        )?;
    } else if key.eq("done")? {
        vp.call_method1("flash_status", (tr.call1(("Axes changed",))?, 2500))?;
    }
    Ok(none(py))
}
fn on_click<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    let vp = ctx.getattr("viewport")?;
    if tool.getattr("align_face")?.is_truthy()? {
        let screen = ctx.getattr("screen")?;
        let (face, place): (Obj, Obj) = vp
            .call_method1(
                "pick_face_placement",
                (screen.call_method0("x")?, screen.call_method0("y")?),
            )?
            .extract()?;
        if face.is_none() {
            return Ok(none(py));
        }
        let matrix = place
            .getattr_opt("xform")?
            .filter(|m| !m.is_none())
            .map(|m| crate::math::mat(&m))
            .transpose()?
            .unwrap_or(glam::DMat4::IDENTITY);
        let points = face
            .getattr("vertices")?
            .try_iter()?
            .map(|p| {
                let p = p?;
                let p = p.getattr_opt("position")?.unwrap_or(p);
                Ok(matrix.transform_point3(crate::math::xyz(&p)?))
            })
            .collect::<PyResult<Vec<_>>>()?;
        let Some([o, r, g]) = super::super::change_axes::face_frame(&points) else {
            return Ok(none(py));
        };
        for point in [o, r, g] {
            let reply = tool.call_method1(
                "_send",
                (
                    &vp,
                    fields!(py; "type" => "click", "world" => point.to_array()),
                ),
            )?;
            finish(&tool, &vp, &reply)?;
        }
    } else {
        let reply = tool.call_method1(
            "_send",
            (&vp, tool.call_method1("_pointer", ("click", &ctx))?),
        )?;
        finish(&tool, &vp, &reply)?;
    }
    vp.call_method0("update")?;
    Ok(none(py))
}
fn finish(tool: &Obj, vp: &Obj, reply: &Obj) -> PyResult<()> {
    if reply.get_item("committed")?.is_truthy()? {
        tool.setattr("target", vp.py().None())?;
        tool.setattr("align_face", false)?;
        if let Some(window) = vp.getattr_opt("window")? {
            let win = window.call0()?;
            if win.hasattr("_activate_tool")? {
                win.call_method1("_activate_tool", ("select",))?;
            }
        }
    }
    Ok(())
}
fn on_value<'py>(a: &Args<'py>) -> Reply<'py> {
    let (tool, vp, value) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?);
    let reply = tool.call_method1(
        "_send",
        (&vp, fields!(a.py(); "type" => "value", "value" => value)),
    )?;
    finish(&tool, &vp, &reply)?;
    vp.call_method0("update")?;
    reply.get_item("consumed")
}
fn unitless<'py>(a: &Args<'py>) -> Reply<'py> {
    (!a.get_item(0)?.getattr("start_point")?.is_none()).into_bound_py_any(a.py())
}
fn value_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let tr = imported(py, "built_core.i18n")?.getattr("tr")?;
    if !tool.getattr("hover_point")?.is_none() {
        return (
            (if tool.getattr("start_point")?.is_none() {
                tr.call1(("Origin X;Y;Z",))?
            } else if tool.getattr("red_point")?.is_none() {
                tr.call1(("Red axis",))?
            } else {
                tr.call1(("Green axis",))?
            }),
            tool.getattr("hover_point")?,
        )
            .into_bound_py_any(py);
    }
    Ok(none(py))
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("host_axes_frame", Mode::Method, host_axes_frame, &["self", "vp", "_"], "", ""),
    ("on_deactivate", Mode::Method, on_deactivate, &["self", "vp"], "", ""),
    ("host_axes_target", Mode::Method, host_axes_target, &["self", "vp", "_"], "", ""),
    ("host_axes_message", Mode::Method, host_axes_message, &["self", "vp", "key"], "", ""),
    ("value_is_unitless", Mode::Method, unitless, &["self"], "", ""),
    ("on_value", Mode::Method, on_value, &["self", "viewport", "value"], "", ""),
    ("on_click", Mode::Method, on_click, &["self", "ctx"], "", ""),
    ("value_label", Mode::Method, value_label, &["self"], "", ""),
];
