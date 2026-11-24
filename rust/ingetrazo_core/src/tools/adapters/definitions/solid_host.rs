// SPDX-License-Identifier: GPL-3.0-or-later
//! Public classes and initial state for solid_host.
use super::*;
fn solid_tool_init<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.solid_host")?;
    let tool = a.get_item(0)?;
    py.import("builtins")?
        .getattr("super")?
        .call1((module.getattr("SolidTool")?, &tool))?
        .call_method0("__init__")?;
    tool.setattr("_cache", PyDict::new(py).into_any())?;
    tool.setattr("_cache_version", none(py))?;
    tool.setattr("_viewport", none(py))?;
    Ok(none(py))
}
fn solid_tool_dragging<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    (!tool.call_method1("_state", ("first",))?.is_none()).into_bound_py_any(py)
}
fn solid_tool_badge<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("badge",))
}
fn solid_tool_solid_cursor_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    format!("solid-{}", tool.getattr("_badge")?.extract::<String>()?).into_bound_py_any(py)
}
fn solid_tool_op<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("kind")?
        .call_method1("removeprefix", ("solid_",))
}
pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    module.add("_SOLID_ERRORS", fields!(py; "not a solid" => "That is not a solid — a group or component closed all round, with nothing nested inside", "no overlap" => "The two solids do not overlap", "empty" => "The operation leaves nothing", "not manifold" => "That solid has a flaw the operation cannot read", "two solids" => "Pick two solids").into_any())?;
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "solid_union")?;
        attrs.set_item("shortcut", none(py))?;
        let cls = define(module, "SolidTool", &[module.getattr("RustTool")?], &attrs)?;
        register_tool_adapter(&cls)?;
        install_methods(
            module,
            &cls,
            &[
                ("__init__", Mode::Method, solid_tool_init, &["self"], "", ""),
                (
                    "dragging",
                    Mode::Property,
                    solid_tool_dragging,
                    &["self"],
                    "",
                    "",
                ),
                (
                    "_badge",
                    Mode::Property,
                    solid_tool_badge,
                    &["self"],
                    "",
                    "",
                ),
                (
                    "solid_cursor_key",
                    Mode::Property,
                    solid_tool_solid_cursor_key,
                    &["self"],
                    "",
                    "",
                ),
                ("op", Mode::Property, solid_tool_op, &["self"], "", ""),
            ],
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "outer_shell")?;
        attrs.set_item("op", "outer_shell")?;
        attrs.set_item("shortcut", none(py))?;
        define(
            module,
            "OuterShellTool",
            &[module.getattr("SolidTool")?],
            &attrs,
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "solid_union")?;
        attrs.set_item("op", "union")?;
        attrs.set_item("shortcut", none(py))?;
        define(module, "UnionTool", &[module.getattr("SolidTool")?], &attrs)?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "solid_subtract")?;
        attrs.set_item("op", "subtract")?;
        attrs.set_item("shortcut", none(py))?;
        define(
            module,
            "SubtractTool",
            &[module.getattr("SolidTool")?],
            &attrs,
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "solid_trim")?;
        attrs.set_item("op", "trim")?;
        attrs.set_item("shortcut", none(py))?;
        define(module, "TrimTool", &[module.getattr("SolidTool")?], &attrs)?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "solid_intersect")?;
        attrs.set_item("op", "intersect")?;
        attrs.set_item("shortcut", none(py))?;
        define(
            module,
            "IntersectTool",
            &[module.getattr("SolidTool")?],
            &attrs,
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "solid_split")?;
        attrs.set_item("op", "split")?;
        attrs.set_item("shortcut", none(py))?;
        define(module, "SplitTool", &[module.getattr("SolidTool")?], &attrs)?;
    }
    module.add(
        "SOLID_TOOLS",
        (
            ("outer_shell", module.getattr("OuterShellTool")?).into_bound_py_any(py)?,
            ("solid_union", module.getattr("UnionTool")?).into_bound_py_any(py)?,
            ("solid_subtract", module.getattr("SubtractTool")?).into_bound_py_any(py)?,
            ("solid_trim", module.getattr("TrimTool")?).into_bound_py_any(py)?,
            ("solid_intersect", module.getattr("IntersectTool")?).into_bound_py_any(py)?,
            ("solid_split", module.getattr("SplitTool")?).into_bound_py_any(py)?,
        ),
    )?;
    Ok(())
}
