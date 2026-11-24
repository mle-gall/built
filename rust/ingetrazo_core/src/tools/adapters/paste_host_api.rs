// SPDX-License-Identifier: GPL-3.0-or-later
//! PasteHost callbacks on the live document.
use super::*;

fn clip<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("_clipboard")
}
fn set_clip<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let clip = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.setattr("_clipboard", &clip)?;
    tool.getattr("_machine")?.call_method1(
        "set_paste_clipboard",
        (({
            let value = clip.clone();
            if value.is_truthy()? {
                value
            } else {
                none(py)
            }
        }),),
    )?;
    Ok(none(py))
}
fn on_activate<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    tool.setattr("_clip", optional(&vp, "clipboard")?)?;
    tool.call_method1("_send", (&vp, fields!(py; "type" => "activate").into_any()))?;
    Ok(none(py))
}
fn on_hover<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.call_method1(
        "_send",
        (
            ctx.getattr("viewport")?,
            tool.call_method1("_pointer", ("hover", &ctx))?,
        ),
    )?;
    Ok(none(py))
}
fn on_click<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    let mut window: Obj<'py>;
    if tool
        .call_method1(
            "_send",
            (
                ctx.getattr("viewport")?,
                tool.call_method1("_pointer", ("click", &ctx))?,
            ),
        )?
        .get_item("committed")?
        .is_truthy()?
    {
        window = optional(&ctx.getattr("viewport")?, "window")?;
        window = if window.is_callable() {
            window.call0()?
        } else {
            none(py)
        };
        if !window.is_none() && window.hasattr("_activate_tool")? {
            window.call_method1("_activate_tool", ("select",))?;
        }
        ctx.getattr("viewport")?.call_method0("update")?;
    }
    Ok(none(py))
}
fn on_cancel<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    tool.call_method1("_send", (&vp, fields!(py; "type" => "cancel").into_any()))?;
    tool.setattr("_clipboard", none(py))?;
    vp.call_method0("update")?;
    Ok(none(py))
}
fn on_deactivate<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    tool.call_method1(
        "_send",
        (&vp, fields!(py; "type" => "deactivate").into_any()),
    )?;
    tool.setattr("_clipboard", none(py))?;
    Ok(none(py))
}
fn in_place<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.paste_host")?;
    let cls = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let tool = cls.call0()?;
    tool.setattr("_clip", optional(&vp, "clipboard")?)?;
    if !tool.getattr("_clip")?.is_truthy()? {
        return false.into_bound_py_any(py);
    }
    let ok = tool.call_method1("stamp", (&vp, module.call_method0("QVector3D")?))?;
    vp.call_method0("update")?;
    Ok(ok.clone())
}
fn stamp<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let off = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    Ok(tool
        .call_method1(
            "_send",
            (
                &vp,
                fields!(py; "type" => "paste_at", "offset" => &off).into_any(),
            ),
        )?
        .get_item("committed")?)
}
fn preview_faces<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if tool.getattr("_clip")?.is_none() {
        return Ok(PyList::empty(py).into_any());
    }
    tool.getattr("_machine")?.call_method1(
        "paste_faces",
        (
            tool.getattr("_clip")?
                .call_method1("get", ("faces", PyTuple::empty(py).into_any()))?,
            tool.getattr("_offset")?,
        ),
    )
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("_clip", Mode::Property, clip, &["self"], "", ""),
    ("on_activate", Mode::Method, on_activate, &["self", "vp"], "", ""),
    ("on_hover", Mode::Method, on_hover, &["self", "ctx"], "", ""),
    ("on_click", Mode::Method, on_click, &["self", "ctx"], "", ""),
    ("on_cancel", Mode::Method, on_cancel, &["self", "vp"], "", ""),
    ("on_deactivate", Mode::Method, on_deactivate, &["self", "vp"], "", ""),
    ("in_place", Mode::Class, in_place, &["cls", "vp"], "", ""),
    ("stamp", Mode::Method, stamp, &["self", "vp", "off"], "", ""),
    ("preview_faces", Mode::Method, preview_faces, &["self"], "", ""),
    ("_clip", Mode::Setter, set_clip, &["self", "clip"], "", ""),
];
