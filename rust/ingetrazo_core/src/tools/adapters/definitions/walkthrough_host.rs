// SPDX-License-Identifier: GPL-3.0-or-later
//! Plugin-facing definitions for walkthrough_host.
use super::*;

fn preference(py: Python<'_>, key: &str, default: f64) -> PyResult<f64> {
    let value = imported(py, "core.settings")?
        .call_method0("settings")?
        .call_method1("value", (key, default))?;
    py.import("builtins")?
        .getattr("float")?
        .call1((value,))?
        .extract()
}
fn eye_height<'py>(a: &Args<'py>) -> Reply<'py> {
    let v = preference(a.py(), "walk/eye_height", 1.68)
        .ok()
        .filter(|v| (0.05..=100.).contains(v))
        .unwrap_or(1.68);
    v.into_bound_py_any(a.py())
}
fn look_sensitivity<'py>(a: &Args<'py>) -> Reply<'py> {
    let value = preference(a.py(), "walk/look_sensitivity", 25.)
        .ok()
        .map(|v| v as i64)
        .filter(|n| (1..=100).contains(n))
        .unwrap_or(25);
    value.into_bound_py_any(a.py())
}
fn set_eye_height<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let update = || -> PyResult<()> {
        let n = py
            .import("builtins")?
            .getattr("float")?
            .call1((a.get_item(0)?,))?;
        imported(py, "core.settings")?
            .call_method0("settings")?
            .call_method1("setValue", ("walk/eye_height", n))?;
        Ok(())
    };
    let _ = update();
    Ok(none(py))
}

pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    module.add("os", py.import("os")?)?;
    module.add(
        "_NO_WALK_REACH",
        py.import("os")?
            .getattr("environ")?
            .call_method1("get", ("INGETRAZO_NO_WALK_REACH", ""))?
            .eq("1")?,
    )?;
    module.add("DEFAULT_EYE_HEIGHT", 1.68)?;
    module.add("_SETTINGS_KEY", "walk/eye_height")?;
    module.add("DEFAULT_LOOK_SENSITIVITY", 25)?;
    module.add("_LOOK_KEY", "walk/look_sensitivity")?;
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("_READ_DURING", PyTuple::empty(py))?;
        attrs.set_item("_viewport", py.None())?;
        attrs.set_item("_timer", py.None())?;
        let cls = define(module, "EyeHost", &[], &attrs)?;
        register_tool_adapter(&cls)?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("DRAG_PX", 6.0)?;
        let cls = define(
            module,
            "PositionCameraHost",
            &[module.getattr("EyeHost")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("TICK_MS", 33)?;
        attrs.set_item("FULL_PX", 150.0)?;
        attrs.set_item("CLEARANCE", 0.35)?;
        attrs.set_item("STEP_UP", 0.5)?;
        attrs.set_item("KNEE", 0.55)?;
        attrs.set_item("WALK_SPEED", 1.5)?;
        attrs.set_item("RUN_FACTOR", 3.0)?;
        attrs.set_item("TURN_DEG_PER_S", 60.0)?;
        attrs.set_item("DEAD_PX", 4.0)?;
        attrs.set_item("overlay_kind", "walk")?;
        attrs.set_item("draw_overlay", module.getattr("draw_overlay")?)?;
        let cls = define(module, "WalkHost", &[module.getattr("EyeHost")?], &attrs)?;
        register_tool_adapter(&cls)?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("TICK_MS", 16)?;
        attrs.set_item(
            "WALK_SPEED",
            module.getattr("WalkHost")?.getattr("WALK_SPEED")?,
        )?;
        attrs.set_item(
            "RUN_FACTOR",
            module.getattr("WalkHost")?.getattr("RUN_FACTOR")?,
        )?;
        attrs.set_item(
            "CLEARANCE",
            module.getattr("WalkHost")?.getattr("CLEARANCE")?,
        )?;
        attrs.set_item("STEP_UP", module.getattr("WalkHost")?.getattr("STEP_UP")?)?;
        attrs.set_item("KNEE", module.getattr("WalkHost")?.getattr("KNEE")?)?;
        let cls = define(
            module,
            "FirstPersonHost",
            &[module.getattr("EyeHost")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
    }
    register_tool_functions(&module.dict())?;
    install_methods(
        module,
        module.dict().as_any(),
        &[
            (
                "eye_height",
                Mode::Function,
                eye_height,
                &[],
                "float",
                "The eye height the three tools share, remembered across sessions.",
            ),
            (
                "set_eye_height",
                Mode::Function,
                set_eye_height,
                &["value:float"],
                "None",
                "",
            ),
            (
                "look_sensitivity",
                Mode::Function,
                look_sensitivity,
                &[],
                "int",
                "The mouse-look sensitivity, 1..100, remembered across sessions.",
            ),
        ],
    )?;
    Ok(())
}
