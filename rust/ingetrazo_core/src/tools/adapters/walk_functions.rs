// SPDX-License-Identifier: GPL-3.0-or-later
//! Functions exported for legacy tool callers.
use super::*;

fn blocked<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.walkthrough_host")?;
    let clearance = a.get_item(4)?;
    let delta = a.get_item(2)?;
    let eye = a.get_item(1)?;
    let h = a.get_item(3)?;
    let knee = a.get_item(5)?;
    let viewport = a.get_item(0)?;
    Ok(module.getattr("ToolMachine")?.call_method1(
        "walk_blocked",
        (
            module.call_method(
                "_Host",
                (&viewport,),
                Some(&fields!(py; "tool" => module.call_method0("EyeHost")?)),
            )?,
            module.call_method1("_tuple", (&eye,))?,
            module.call_method1("_tuple", (&delta,))?,
            &h,
            &clearance,
            &knee,
        ),
    )?)
}

fn look_deg_per_px<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let sensitivity = a.get_item(0)?;
    let walkthrough = py.import("tools.walkthrough")?.into_any();
    let s = if sensitivity.is_none() {
        walkthrough.call_method0("look_sensitivity")?
    } else {
        sensitivity.clone()
    };
    s.div(500.0)
}

#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("look_deg_per_px", Mode::Function, look_deg_per_px, &["sensitivity:int | None=None"], "float", "Degrees of head turn per pixel for a sensitivity (the stored one\nwhen ``None``): 1 → 0.002°, 25 → 0.05°, 100 → 0.2°."),
    ("_blocked", Mode::Function, blocked, &["viewport", "eye", "delta", "h", "clearance", "knee"], "", "Compatibility entry point for the Rust walker's wall query."),
];
