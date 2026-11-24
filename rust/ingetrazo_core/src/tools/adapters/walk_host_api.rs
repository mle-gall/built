// SPDX-License-Identifier: GPL-3.0-or-later
//! WalkHost callbacks on the live document.
use super::*;

fn step<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.walkthrough_host")?;
    let dt = a.get_item(4)?;
    let dx = a.get_item(2)?;
    let dy = a.get_item(3)?;
    let modifiers = a.get_item(5)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_send", (&viewport, fields!(py; "type" => "walk_step", "delta" => (&dx, &dy,).into_bound_py_any(py)?, "dt" => &dt, "mods" => module.call_method1("_mods", (&modifiers,))?).into_any(),))?;
    Ok(none(py))
}
fn walking<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    (!tool.call_method1("_state", ("anchor",))?.is_none()).into_bound_py_any(py)
}
fn anchor_px<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.walkthrough_host")?;
    let tool = a.get_item(0)?;
    let p = tool.call_method1("_state", ("anchor",))?;
    Ok(if p.is_none() {
        none(py)
    } else {
        module.call_method1("QPointF", positional(&p)?)?
    })
}
fn set_anchor_px<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let p = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.getattr("_machine")?.call_method1(
        "set",
        (
            "anchor",
            (if p.is_none() {
                none(py)
            } else {
                (p.call_method0("x")?, p.call_method0("y")?).into_bound_py_any(py)?
            }),
        ),
    )?;
    Ok(none(py))
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("step", Mode::Method, step, &["self", "viewport", "dx", "dy", "dt", "modifiers=Qt.NoModifier"], "", ""),
    ("walking", Mode::Property, walking, &["self"], "", ""),
    ("_anchor_px", Mode::Property, anchor_px, &["self"], "", ""),
    ("_anchor_px", Mode::Setter, set_anchor_px, &["self", "p"], "", ""),
];
