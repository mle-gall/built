// SPDX-License-Identifier: GPL-3.0-or-later
//! TapeMeasureTool callbacks on the live document.
use super::*;

fn edge<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok(if !tool.call_method1("_state", ("direction",))?.is_none() {
        tool.getattr("_source")?
    } else {
        none(py)
    })
}
fn set_edge<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let edge = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.setattr("_source", &edge)?;
    tool.getattr("_machine")?.call_method1(
        "set",
        (
            "direction",
            (if !edge.is_none() {
                module.call_method1("_tuple", (edge.getattr("b")?.sub(edge.getattr("a")?)?,))?
            } else {
                none(py)
            }),
        ),
    )?;
    Ok(none(py))
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("_edge", Mode::Property, edge, &["self"], "", ""),
    ("_edge", Mode::Setter, set_edge, &["self", "edge"], "", ""),
];
