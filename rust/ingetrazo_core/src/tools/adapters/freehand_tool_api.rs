// SPDX-License-Identifier: GPL-3.0-or-later
//! FreehandTool callbacks on the live document.
use super::*;

fn clear<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let event = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.setattr("work_plane", none(py))?;
    tool.call_method1(
        "_send",
        (&viewport, fields!(py; "type" => &event).into_any()),
    )?;
    Ok(none(py))
}
fn on_activate<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_clear", (&viewport, "activate"))?;
    Ok(none(py))
}
fn on_deactivate<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_clear", (&viewport, "deactivate"))?;
    Ok(none(py))
}
fn on_cancel<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_clear", (&viewport, "cancel"))?;
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn on_release<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_clear", (&viewport, "release"))?;
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn on_hover<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    if tool
        .call_method1(
            "_send",
            (
                ctx.getattr("viewport")?,
                tool.call_method1("_pointer", ("hover", &ctx))?,
            ),
        )?
        .get_item("redraw")?
        .is_truthy()?
    {
        ctx.getattr("viewport")?.call_method0("update")?;
    }
    Ok(none(py))
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("_clear", Mode::Method, clear, &["self", "viewport", "event"], "", ""),
    ("on_activate", Mode::Method, on_activate, &["self", "viewport"], "", ""),
    ("on_deactivate", Mode::Method, on_deactivate, &["self", "viewport"], "", ""),
    ("on_cancel", Mode::Method, on_cancel, &["self", "viewport"], "", ""),
    ("on_release", Mode::Method, on_release, &["self", "viewport"], "", ""),
    ("on_hover", Mode::Method, on_hover, &["self", "ctx"], "", ""),
];
