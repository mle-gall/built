// SPDX-License-Identifier: GPL-3.0-or-later
//! Public classes and initial state for section_host.
use super::*;
fn section_plane_tool_hover_point<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_point", ("hover_point",))
}
fn section_plane_tool_axis_pick<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("axis",))
}
pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "section")?;
        attrs.set_item("shortcut", none(py))?;
        attrs.set_item(
            "_normal",
            module.getattr("_point_property")?.call1(("normal",))?,
        )?;
        let cls = define(
            module,
            "SectionPlaneTool",
            &[module.getattr("RustTool")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
        install_methods(
            module,
            &cls,
            &[
                (
                    "hover_point",
                    Mode::Property,
                    section_plane_tool_hover_point,
                    &["self"],
                    "",
                    "",
                ),
                (
                    "_axis_pick",
                    Mode::Property,
                    section_plane_tool_axis_pick,
                    &["self"],
                    "",
                    "",
                ),
            ],
        )?;
    }
    Ok(())
}
