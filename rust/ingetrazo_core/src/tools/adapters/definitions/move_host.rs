// SPDX-License-Identifier: GPL-3.0-or-later
//! Public classes and initial state for move_host.
use super::*;
fn copy_preview_gather_copy_entities<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.move_host")?;
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    module
        .getattr("CopyWire")?
        .call_method1("capture", (&tool, &ctx))?;
    Ok(none(py))
}
fn copy_preview_fill_groups_ghost<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    tool.getattr("_base_segments")?
        .call_method0("fill_groups")?;
    Ok(none(py))
}
fn move_host_init<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.move_host")?;
    let tool = a.get_item(0)?;
    module
        .getattr("RustTool")?
        .call_method1("__init__", (&tool,))?;
    tool.setattr("chain_first_point", none(py))?;
    tool.setattr("_ctx", none(py))?;
    tool.setattr("_move_gesture", module.getattr("MoveGesture")?.call0()?)?;
    tool.call_method0("_clear_sets")?;
    tool.setattr("_grip_group", none(py))?;
    tool.setattr("_grips", PyList::empty(py).into_any())?;
    tool.setattr("_hot_grip", none(py))?;
    tool.setattr("_turn", module.getattr("GripTurn")?.call0()?)?;
    Ok(none(py))
}
fn move_host_grip_rot<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("_turn")?.getattr("active")
}
pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        let cls = define(
            module,
            "CopyPreview",
            &[py.import("builtins")?.getattr("object")?],
            &attrs,
        )?;
        install_methods(
            module,
            &cls,
            &[
                (
                    "_gather_copy_entities",
                    Mode::Method,
                    copy_preview_gather_copy_entities,
                    &["self", "ctx"],
                    "",
                    "",
                ),
                (
                    "_fill_groups_ghost",
                    Mode::Method,
                    copy_preview_fill_groups_ghost,
                    &["self"],
                    "",
                    "",
                ),
            ],
        )?;
    }
    module.add("GRIP_PX", 9.0)?;
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "")?;
        attrs.set_item("_READ_DURING", ("grab", "hover_point", "copy", "window"))?;
        attrs.set_item("prefers_vertical_drag", true)?;
        attrs.set_item("accepts_array", true)?;
        attrs.set_item(
            "start_point",
            module.getattr("_point_property")?.call1(("grab",))?,
        )?;
        attrs.set_item("grab", module.getattr("_point_property")?.call1(("grab",))?)?;
        attrs.set_item(
            "hover_point",
            module.getattr("_point_property")?.call1(("hover_point",))?,
        )?;
        attrs.set_item("overlay_kind", "move")?;
        attrs.set_item("draw_overlay", module.getattr("draw_overlay")?)?;
        attrs.set_item("__doc__","Move (``rust/ingetrazo_core/src/tools/move_tool.rs`` decides: the\ngrab, the Ctrl copy, typed distances and deltas, the array window).\nThis class is the host: what a click grabs (it is picking), the live\npreview of the grabbed geometry, the rotation grips (#115, run by the\nRotate tool) and the commit through ``core.move_ops``.")?;
        let cls = define(
            module,
            "MoveHost",
            &[module.getattr("CopyPreview")?, module.getattr("RustTool")?],
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
                    move_host_init,
                    &["self"],
                    "None",
                    "",
                ),
                (
                    "_grip_rot",
                    Mode::Property,
                    move_host_grip_rot,
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
