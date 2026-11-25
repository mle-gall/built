// SPDX-License-Identifier: GPL-3.0-or-later
//! Plugin-facing definitions for offset_host.
use super::*;

#[pyfunction]
fn read_state(key: &str, tool: &Obj) -> PyResult<Py<PyAny>> {
    Ok(tool.call_method1("_state", (key,))?.unbind())
}
#[pyfunction]
fn write_state(key: &str, tool: &Obj, value: &Obj) -> PyResult<()> {
    tool.getattr("_machine")?
        .call_method1("set", (key, value))?;
    Ok(())
}
fn field<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let key = a.get_item(0)?;
    let partial = py.import("functools")?.getattr("partial")?;
    let get = partial.call1((wrap_pyfunction!(read_state, py)?, &key))?;
    let set = partial.call1((wrap_pyfunction!(write_state, py)?, key))?;
    py.import("builtins")?
        .getattr("property")?
        .call1((get, set))
}
pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    install_methods(
        module,
        module.dict().as_any(),
        &[("_field", Mode::Function, field, &["key"], "", "")],
    )?;
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item(
            "_READ_DURING",
            (
                "dragging",
                "distance",
                "ref_point",
                "ref_inward",
                "reference",
            ),
        )?;
        attrs.set_item("hovered_face", py.None())?;
        attrs.set_item("hovered_edge", attrs.get_item("hovered_face")?.unwrap())?;
        attrs.set_item("_base_face", py.None())?;
        attrs.set_item("dragging", module.getattr("_field")?.call1(("dragging",))?)?;
        attrs.set_item("distance", module.getattr("_field")?.call1(("distance",))?)?;
        attrs.set_item("_armed", module.getattr("_field")?.call1(("armed",))?)?;
        attrs.set_item("_closed", module.getattr("_field")?.call1(("closed",))?)?;
        attrs.set_item(
            "_ref_point",
            module.getattr("_point_property")?.call1(("ref_point",))?,
        )?;
        attrs.set_item(
            "_ref_inward",
            module.getattr("_point_property")?.call1(("ref_inward",))?,
        )?;
        attrs.set_item("allow_inferences", false)?;
        attrs.set_item("wireframe_color", (0.13, 0.17, 0.23, 1.0))?;
        attrs.set_item("wireframe_depth_tested", true)?;
        attrs.set_item(
            "allowed_snap_kinds",
            py.import("builtins")?.getattr("frozenset")?.call1(((
                "endpoint",
                "midpoint",
                "arc_midpoint",
                "center",
                "origin",
                "component_origin",
                "intersection",
                "intersection_edge",
                "intersection_face",
            ),))?,
        )?;
        attrs.set_item("overlay_kind", "offset")?;
        attrs.set_item("draw_overlay", module.getattr("draw_overlay")?)?;
        let cls = define(module, "OffsetHost", &[], &attrs)?;
        register_tool_adapter(&cls)?;
    }
    register_tool_functions(&module.dict())?;
    Ok(())
}
