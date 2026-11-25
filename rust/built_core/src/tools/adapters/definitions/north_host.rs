// SPDX-License-Identifier: GPL-3.0-or-later
//! Public classes and initial state for north_host.
use super::*;
fn north_tool_centre<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_point", ("centre",))
}
fn north_tool_hover_point<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_point", ("hover_point",))
}
fn north_tool_radius<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("radius",))
}
pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    module.add("COMPASS_PX", 70.0)?;
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "north")?;
        attrs.set_item("icon", attrs.get_item("kind")?.unwrap())?;
        attrs.set_item("wireframe_color", (0.8, 0.16, 0.14, 1.0))?;
        let cls = define(module, "NorthTool", &[module.getattr("RustTool")?], &attrs)?;
        register_tool_adapter(&cls)?;
        install_methods(
            module,
            &cls,
            &[
                (
                    "centre",
                    Mode::Property,
                    north_tool_centre,
                    &["self"],
                    "",
                    "",
                ),
                (
                    "hover_point",
                    Mode::Property,
                    north_tool_hover_point,
                    &["self"],
                    "",
                    "",
                ),
                (
                    "_radius",
                    Mode::Property,
                    north_tool_radius,
                    &["self"],
                    "",
                    "",
                ),
            ],
        )?;
    }
    register_tool_functions(&module.dict())?;
    Ok(())
}
