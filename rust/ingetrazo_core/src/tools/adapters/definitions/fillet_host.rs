// SPDX-License-Identifier: GPL-3.0-or-later
//! Public classes and initial state for fillet_host.
use super::*;
fn fillet_host_init<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    tool.setattr("edges", PyList::empty(py).into_any())?;
    tool.setattr("hover_edge", none(py))?;
    tool.setattr("hover_vertex", none(py))?;
    tool.setattr("hover_point", none(py))?;
    tool.setattr("_plan", none(py))?;
    tool.setattr("_plan_key", none(py))?;
    tool.setattr("_message", none(py))?;
    tool.setattr("_viewport", none(py))?;
    Ok(none(py))
}
pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("icon", "fillet")?;
        attrs.set_item("wireframe_color", (0.13, 0.17, 0.23, 1.0))?;
        attrs.set_item("wireframe_depth_tested", true)?;
        attrs.set_item("radius", 0.1)?;
        attrs.set_item("segments", 8)?;
        attrs.set_item("profile", module.getattr("CIRCULAR")?)?;
        attrs.set_item("measure", "radius")?;
        attrs.set_item(
            "_MEASURES",
            (
                ("radius", "Radius").into_bound_py_any(py)?,
                ("offset", "Distance").into_bound_py_any(py)?,
                ("width", "Width").into_bound_py_any(py)?,
                ("depth", "Depth").into_bound_py_any(py)?,
            ),
        )?;
        attrs.set_item("corners", "round")?;
        attrs.set_item("shape", none(py))?;
        attrs.set_item(
            "_SHAPES",
            (
                ("steps", "Steps").into_bound_py_any(py)?,
                ("supports", "Round with supports").into_bound_py_any(py)?,
                ("cornice", "Cornice").into_bound_py_any(py)?,
                ("crown", "Crown moulding").into_bound_py_any(py)?,
            ),
        )?;
        attrs.set_item("affect", "edges")?;
        attrs.set_item(
            "_nearest_end",
            py.import("builtins")?
                .getattr("staticmethod")?
                .call1((module.getattr("bevel_nearest_end")?,))?,
        )?;
        let annotations = PyDict::new(py);
        annotations.set_item("radius", "float")?;
        annotations.set_item("segments", "int")?;
        annotations.set_item("profile", "float")?;
        annotations.set_item("measure", "str")?;
        annotations.set_item("corners", "str")?;
        annotations.set_item("affect", "str")?;
        attrs.set_item("__annotations__", annotations)?;
        let cls = define(module, "FilletHost", &[module.getattr("Tool")?], &attrs)?;
        register_tool_adapter(&cls)?;
        install_methods(
            module,
            &cls,
            &[(
                "__init__",
                Mode::Method,
                fillet_host_init,
                &["self"],
                "None",
                "",
            )],
        )?;
    }
    Ok(())
}
