// SPDX-License-Identifier: GPL-3.0-or-later
//! Plugin-facing definitions for scale_manager_host.
use super::*;

#[pyfunction]
fn show_editor(tool: &Obj, vp: &Obj, group: &Obj) -> PyResult<()> {
    tool.call_method1("_run", (vp, group))?;
    Ok(())
}
#[pyfunction]
fn closed(vp: &Obj, window: &Obj) -> PyResult<()> {
    vp.call_method0("update")?;
    window.call_method1("_activate_tool", ("select",))?;
    Ok(())
}
fn schedule<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let group = tool.getattr("_target_group")?;
    tool.setattr("_target_group", py.None())?;
    if a.get_item(2)?.is_none() {
        let text = imported(py, "core.i18n")?
            .call_method1("tr", ("Select one group or component first",))?;
        vp.call_method1("flash_status", (text,))?;
    } else {
        let callback = py.import("functools")?.call_method1(
            "partial",
            (wrap_pyfunction!(show_editor, py)?, tool, &vp, group),
        )?;
        imported(py, "ingetrazo_core.timers")?.call_method1("later", (vp, 0, callback))?;
    }
    Ok(none(py))
}
fn run_editor<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let vp = a.get_item(0)?;
    let win = vp.call_method0("window")?;
    let done = py
        .import("functools")?
        .call_method1("partial", (wrap_pyfunction!(closed, py)?, vp, &win))?;
    imported(py, "ingetrazo_render")?.call_method(
        "ask_scale",
        (win, a.get_item(1)?),
        Some(&fields!(py;"done"=>done)),
    )?;
    Ok(none(py))
}
pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "scale_manager")?;
        attrs.set_item("shortcut", py.None())?;
        attrs.set_item("description", attrs.get_item("shortcut")?.unwrap())?;
        attrs.set_item("_target_group", py.None())?;
        let cls = define(
            module,
            "ScaleManagerTool",
            &[module.getattr("RustTool")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
        install_methods(
            module,
            &cls,
            &[
                (
                    "host_scale_editor",
                    Mode::Method,
                    schedule,
                    &["self", "vp", "uid"],
                    "",
                    "",
                ),
                ("_run", Mode::Static, run_editor, &["vp", "group"], "", ""),
            ],
        )?;
    }
    Ok(())
}
