// SPDX-License-Identifier: GPL-3.0-or-later
//! Public classes and initial state for geopath_host.
use super::*;
fn geo_path_tool_nodes<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.geopath_host")?;
    let tool = a.get_item(0)?;
    let nodes = PyList::empty(py);
    let vector = module.getattr("Vec3")?;
    for point in tool.call_method1("_state", ("nodes",))?.try_iter()? {
        nodes.append(vector.call1(positional(&point?)?)?)?;
    }
    Ok(nodes.into_any())
}
fn geo_path_tool_hover_point<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_point", ("hover_point",))
}
fn geo_path_tool_hover_elevation<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("elevation",))
}
pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "geopath")?;
        attrs.set_item("_saved_view", none(py))?;
        attrs.set_item(
            "work_plane",
            (
                module.getattr("Vec3")?.call0()?,
                module.getattr("Vec3")?.call1((0, 0, 1))?,
            ),
        )?;
        let cls = define(
            module,
            "GeoPathTool",
            &[module.getattr("RustTool")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
        install_methods(
            module,
            &cls,
            &[
                (
                    "nodes",
                    Mode::Property,
                    geo_path_tool_nodes,
                    &["self"],
                    "",
                    "",
                ),
                (
                    "hover_point",
                    Mode::Property,
                    geo_path_tool_hover_point,
                    &["self"],
                    "",
                    "",
                ),
                (
                    "hover_elevation",
                    Mode::Property,
                    geo_path_tool_hover_elevation,
                    &["self"],
                    "",
                    "",
                ),
            ],
        )?;
    }
    Ok(())
}
