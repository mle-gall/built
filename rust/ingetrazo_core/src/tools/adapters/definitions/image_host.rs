// SPDX-License-Identifier: GPL-3.0-or-later
//! Public classes and initial state for image_host.
use super::*;
fn image_tool_path<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("path",))
}
fn image_tool_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("label",))
}
fn image_tool_aspect<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("aspect",))
}
fn image_tool_armed<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    (tool.getattr("path")?.is_truthy()?).into_bound_py_any(py)
}
fn image_tool_free_aspect<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("free",))
}
fn image_tool_set_free_aspect<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let v = a.get_item(1)?;
    tool.getattr("_machine")?
        .call_method1("set", ("free", (v.is_truthy()?).into_bound_py_any(py)?))
}
pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "image")?;
        attrs.set_item("shortcut", none(py))?;
        attrs.set_item("description", attrs.get_item("shortcut")?.unwrap())?;
        attrs.set_item("MIN_SIZE", 0.0001)?;
        attrs.set_item("diagonal_span", true)?;
        attrs.set_item("chain_first_point", none(py))?;
        attrs.set_item("_plane", none(py))?;
        attrs.set_item(
            "start_point",
            module.getattr("_point_property")?.call1(("start_point",))?,
        )?;
        attrs.set_item(
            "hover_point",
            module.getattr("_point_property")?.call1(("hover_point",))?,
        )?;
        let cls = define(module, "ImageTool", &[module.getattr("RustTool")?], &attrs)?;
        register_tool_adapter(&cls)?;
        install_methods(
            module,
            &cls,
            &[
                ("path", Mode::Property, image_tool_path, &["self"], "", ""),
                ("label", Mode::Property, image_tool_label, &["self"], "", ""),
                (
                    "aspect",
                    Mode::Property,
                    image_tool_aspect,
                    &["self"],
                    "",
                    "",
                ),
                ("armed", Mode::Property, image_tool_armed, &["self"], "", ""),
                (
                    "_free_aspect",
                    Mode::Property,
                    image_tool_free_aspect,
                    &["self"],
                    "",
                    "",
                ),
                (
                    "_free_aspect",
                    Mode::Setter,
                    image_tool_set_free_aspect,
                    &["self", "v"],
                    "",
                    "",
                ),
            ],
        )?;
    }
    Ok(())
}
