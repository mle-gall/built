// SPDX-License-Identifier: GPL-3.0-or-later
//! Public classes and initial state for eraser_host.
use super::*;
fn eraser_host_stroke<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("stroke",))
}
fn eraser_host_set_stroke<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    let value = a.get_item(1)?;
    tool.getattr("_machine")?
        .call_method1("set", ("stroke", &value))
}
fn eraser_host_hide<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("hide",))
}
fn eraser_host_set_hide<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let hide = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.getattr("_machine")?
        .call_method1("set", ("hide", (hide.is_truthy()?).into_bound_py_any(py)?))
}
fn eraser_host_marked<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let handles = tool.getattr("_handles")?;
    let marked = PySet::empty(py)?;
    for id in tool.call_method1("_state", ("marked",))?.try_iter()? {
        marked.add(handles.call_method1("get", (id?,))?)?;
    }
    Ok(marked.into_any())
}
fn eraser_host_set_marked<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    let handles = tool.getattr("_handles")?;
    let ids = PyList::empty(a.py());
    for entity in a.get_item(1)?.try_iter()? {
        ids.append(handles.call_method1("id", (entity?,))?)?;
    }
    tool.getattr("_machine")?
        .call_method1("set", ("marked", ids))
}

pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("wireframe_color", (0.9, 0.2, 0.15, 1.0))?;
        attrs.set_item("_viewport", none(py))?;
        attrs.set_item("handle_limit", none(py))?;
        let cls = define(
            module,
            "EraserHost",
            &[py.import("builtins")?.getattr("object")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
        install_methods(
            module,
            &cls,
            &[
                (
                    "_stroke",
                    Mode::Property,
                    eraser_host_stroke,
                    &["self"],
                    "",
                    "",
                ),
                (
                    "_stroke",
                    Mode::Setter,
                    eraser_host_set_stroke,
                    &["self", "value"],
                    "",
                    "",
                ),
                ("_hide", Mode::Property, eraser_host_hide, &["self"], "", ""),
                (
                    "_hide",
                    Mode::Setter,
                    eraser_host_set_hide,
                    &["self", "hide"],
                    "",
                    "",
                ),
                (
                    "marked",
                    Mode::Property,
                    eraser_host_marked,
                    &["self"],
                    "",
                    "",
                ),
                (
                    "marked",
                    Mode::Setter,
                    eraser_host_set_marked,
                    &["self", "items"],
                    "",
                    "",
                ),
            ],
        )?;
    }
    Ok(())
}
