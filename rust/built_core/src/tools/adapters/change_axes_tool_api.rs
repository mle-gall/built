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
fn host_axes_message<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let key = a.get_item(2)?;
    let labels: Obj<'py>;
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
    } else {
        if key.eq("done")? {
            vp.call_method1("flash_status", (tr.call1(("Axes changed",))?, 2500))?;
        } else {
            labels = fields!(py; "empty" => "Select one group or component first", "red" => "Click along the new red axis", "green" => "Click to set the green direction").into_any();
            vp.call_method1("flash_status", (tr.call1((labels.get_item(&key)?,))?,))?;
        }
    }
    Ok(none(py))
}
fn on_click<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    let window: Obj<'py>;
    let vp = ctx.getattr("viewport")?;
    if tool
        .call_method1(
            "_send",
            (&vp, tool.call_method1("_pointer", ("click", &ctx))?),
        )?
        .get_item("committed")?
        .is_truthy()?
    {
        tool.setattr("target", none(py))?;
        window = if vp.hasattr("window")? {
            vp.call_method0("window")?
        } else {
            none(py)
        };
        if window.hasattr("_activate_tool")? {
            window.call_method1("_activate_tool", ("select",))?;
        }
    }
    vp.call_method0("update")?;
    Ok(none(py))
}
fn value_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let tr = imported(py, "built_core.i18n")?.getattr("tr")?;
    if !tool.getattr("start_point")?.is_none() && !tool.getattr("hover_point")?.is_none() {
        return (
            (if tool.getattr("red_point")?.is_none() {
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
    ("host_axes_target", Mode::Method, host_axes_target, &["self", "vp", "_"], "", ""),
    ("host_axes_message", Mode::Method, host_axes_message, &["self", "vp", "key"], "", ""),
    ("on_click", Mode::Method, on_click, &["self", "ctx"], "", ""),
    ("value_label", Mode::Method, value_label, &["self"], "", ""),
];
