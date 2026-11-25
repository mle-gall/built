// SPDX-License-Identifier: GPL-3.0-or-later
//! Public classes and initial state for pushpull.
use super::*;
fn push_pull_tool_init<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.pushpull")?;
    let tool = a.get_item(0)?;
    module
        .getattr("RustTool")?
        .call_method1("__init__", (&tool,))?;
    module
        .getattr("PushPullOperation")?
        .call_method1("__init__", (&tool,))?;
    tool.setattr("_ctx", none(py))?;
    Ok(none(py))
}
pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    module.add("math", py.import("math")?)?;
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "pushpull")?;
        attrs.set_item("_READ_DURING", PyTuple::empty(py).into_any())?;
        attrs.set_item("name", "Push / Pull")?;
        attrs.set_item("shortcut", "P")?;
        attrs.set_item("shortcut_alt", "U")?;
        attrs.set_item(
            "description",
            "Push or pull a face to extrude it into a solid or cut it back.",
        )?;
        attrs.set_item("uses_snap", false)?;
        attrs.set_item("hover_group_edges", true)?;
        attrs.set_item("vcb_label", "Distance")?;
        attrs.set_item("wireframe_color", (0.13, 0.17, 0.23, 1.0))?;
        attrs.set_item("wireframe_depth_tested", true)?;
        attrs.set_item("preview_cull_back", true)?;
        attrs.set_item("_SLOW_STAGE_S", 0.25)?;
        let cls = define(
            module,
            "PushPullTool",
            &[
                module.getattr("PushPullOperation")?,
                module.getattr("RustTool")?,
            ],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
        install_methods(
            module,
            &cls,
            &[(
                "__init__",
                Mode::Method,
                push_pull_tool_init,
                &["self"],
                "",
                "",
            )],
        )?;
    }
    Ok(())
}
