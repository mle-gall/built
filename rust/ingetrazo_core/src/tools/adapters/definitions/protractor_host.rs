// SPDX-License-Identifier: GPL-3.0-or-later
//! Plugin-facing definitions for protractor_host.
use super::*;

fn initialize<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    for key in ["start_point", "ref_point", "hover_point"] {
        tool.setattr(key, a.py().None())?;
    }
    Ok(none(a.py()))
}
fn infer_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let guard = imported(py, "views.array_bake")?.call_method0("reading_only")?;
    guard.call_method0("__enter__")?;
    let infer = || -> PyResult<()> {
        let shift = imported(py, "tools.protractor_host")?
            .getattr("Qt")?
            .getattr("ShiftModifier")?;
        a.get_item(0)?.call_method1(
            "infer_disc",
            (&ctx, ctx.getattr("modifiers")?.bitand(shift)?.is_truthy()?),
        )?;
        Ok(())
    };
    match infer() {
        Ok(()) => {
            guard.call_method1("__exit__", (py.None(), py.None(), py.None()))?;
        }
        Err(err) => {
            if !guard
                .call_method1(
                    "__exit__",
                    (err.get_type(py), err.value(py), err.traceback(py)),
                )?
                .is_truthy()?
            {
                return Err(err);
            }
        }
    }
    Ok(none(py))
}
pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();

    let axes = PyDict::new(py);
    for (key, color) in [
        ("x", "COLOR_AXIS_X"),
        ("y", "COLOR_AXIS_Y"),
        ("z", "COLOR_AXIS_Z"),
    ] {
        let rgb = module.getattr(color)?.extract::<[f64; 3]>()?;
        axes.set_item(key, (rgb[0], rgb[1], rgb[2], 1.0))?;
    }
    module.add("_AXIS_RGBA", axes)?;
    module.add("DISC_PX", 60.0)?;
    module.add("TICK_DEG", 15.0)?;
    module.add("_OFF_AXIS_RGBA", (0.24, 0.27, 0.32, 1.0))?;
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item(
            "__doc__",
            "Shared protractor state + behaviour (see module docstring).",
        )?;
        attrs.set_item("magnetic_axis_deg", 3.0)?;
        attrs.set_item("radial_arm", true)?;
        let cls = define(
            module,
            "ProtractorBase",
            &[module.getattr("ToolDisc")?, module.getattr("Tool")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
        install_methods(
            module,
            &cls,
            &[
                ("__init__", Mode::Method, initialize, &["self"], "None", ""),
                (
                    "_infer_plane",
                    Mode::Method,
                    infer_plane,
                    &["self", "ctx"],
                    "",
                    "",
                ),
            ],
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("accepts_angle_ratio", true)?;
        attrs.set_item("_held_guide", py.None())?;
        let cls = define(
            module,
            "ProtractorHost",
            &[module.getattr("ProtractorBase")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
    }
    Ok(())
}
