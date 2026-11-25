// SPDX-License-Identifier: GPL-3.0-or-later
//! Public classes and initial state for scale_host.
use super::*;
fn grip_init<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let mask = a.get_item(2)?;
    let params = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.setattr("params", &params)?;
    tool.setattr("mask", &mask)?;
    Ok(none(py))
}
fn scale_host_init<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.scale_host")?;
    let tool = a.get_item(0)?;
    py.import("builtins")?
        .getattr("super")?
        .call1((module.getattr("ScaleHost")?, &tool))?
        .call_method0("__init__")?;
    tool.setattr("_lo", none(py))?;
    tool.setattr("_hi", none(py))?;
    tool.setattr("_axes", none(py))?;
    tool.setattr("_gesture", module.getattr("MatrixGesture")?.call0()?)?;
    tool.setattr("_orig", none(py))?;
    tool.setattr("_grips", PyList::empty(py).into_any())?;
    tool.setattr("_box_version", -1)?;
    tool.setattr("_groups", PyList::empty(py).into_any())?;
    tool.setattr("_positions", PyList::empty(py).into_any())?;
    tool.setattr("_verts", PyList::empty(py).into_any())?;
    tool.setattr("_images", PyList::empty(py).into_any())?;
    tool.setattr("_grip", none(py))?;
    tool.setattr("_hover_grip", none(py))?;
    tool.setattr("_anchor", none(py))?;
    tool.setattr("about_center", false)?;
    tool.setattr("uniform", false)?;
    Ok(none(py))
}
fn scale_host_factors<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("_gesture")?.getattr("factors")
}
pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    module.add("_FLAT", (1e-06).into_bound_py_any(py)?)?;
    module.add("_AXIS_NAMES", ("Red", "Green", "Blue"))?;
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("__slots__", ("params", "mask"))?;
        attrs.set_item(
            "__doc__",
            "One scaling grip: box-parameter position + the axes it scales.",
        )?;
        let cls = define(
            module,
            "_Grip",
            &[py.import("builtins")?.getattr("object")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
        install_methods(
            module,
            &cls,
            &[(
                "__init__",
                Mode::Method,
                grip_init,
                &["self", "params:tuple", "mask:tuple"],
                "None",
                "",
            )],
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("accepts_absolute_length", true)?;
        attrs.set_item("_GRIP_HALF_PX", 5.0)?;
        attrs.set_item(
            "_TARGET_KINDS",
            py.import("builtins")?
                .getattr("frozenset")?
                .call1((PySet::new(
                    py,
                    [
                        "endpoint",
                        "midpoint",
                        "intersection",
                        "intersection_edge",
                        "intersection_face",
                        "center",
                        "origin",
                        "on_edge",
                        "on_line",
                        "from_point",
                        "close",
                        "guide",
                        "component_origin",
                        "arc_midpoint",
                        "face_center",
                    ],
                )?
                .into_any(),))?,
        )?;
        let cls = define(
            module,
            "ScaleHost",
            &[py.import("builtins")?.getattr("object")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
        install_methods(
            module,
            &cls,
            &[
                (
                    "__init__",
                    Mode::Method,
                    scale_host_init,
                    &["self"],
                    "None",
                    "",
                ),
                (
                    "_factors",
                    Mode::Property,
                    scale_host_factors,
                    &["self"],
                    "",
                    "",
                ),
            ],
        )?;
    }
    Ok(())
}
