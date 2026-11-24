// SPDX-License-Identifier: GPL-3.0-or-later
//! ScaleManagerTool callbacks on the live document.
use super::*;

fn target<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("_target_group")
}
fn set_target<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let group = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.setattr("_target_group", &group)?;
    tool.getattr("_machine")?.call_method1(
        "set",
        (
            "target",
            (if !group.is_none() {
                group.getattr("uid")?
            } else {
                none(py)
            }),
        ),
    )?;
    Ok(none(py))
}

fn on_deactivate<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.scale_manager_host")?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    tool.setattr("_target_group", none(py))?;
    py.import("builtins")?
        .getattr("super")?
        .call1((module.getattr("ScaleManagerTool")?, &tool))?
        .call_method1("on_deactivate", (&vp,))?;
    Ok(none(py))
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("target", Mode::Property, target, &["self"], "", ""),
    ("on_deactivate", Mode::Method, on_deactivate, &["self", "vp"], "", ""),
    ("target", Mode::Setter, set_target, &["self", "group"], "", ""),
];
