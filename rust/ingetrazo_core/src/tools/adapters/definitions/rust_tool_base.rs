// SPDX-License-Identifier: GPL-3.0-or-later
//! Plugin-facing definitions for rust_tool_base.
use super::*;

fn message<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let view = a.get_item(0)?.getattr("vp")?;
    let Some(flash) = view
        .getattr_opt("flash_status")?
        .filter(|f| f.is_callable())
    else {
        return Ok(none(py));
    };
    let args = a.get_item(2)?.cast_into::<PyDict>()?;
    for (key, value) in args.iter() {
        if let Some(length) = value.extract::<String>()?.strip_prefix("len:") {
            let number = py.import("builtins")?.getattr("float")?.call1((length,))?;
            args.set_item(
                key,
                imported(py, "core.units")?.call_method1("fmt_len", (number,))?,
            )?;
        }
    }
    let tr = imported(py, "core.i18n")?.getattr("tr")?;
    flash.call1((tr.call((a.get_item(1)?,), Some(&args))?,))?;
    Ok(none(py))
}
fn initialize<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let module = imported(py, "tools.rust_tool_base")?;
    tool.setattr(
        "_machine",
        module.call_method1("ToolMachine", (tool.getattr("kind")?,))?,
    )?;
    tool.setattr(
        "_handles",
        module.call_method1("ToolHandles", (tool.getattr("handle_limit")?,))?,
    )?;
    tool.setattr("_seen", (-1, py.None()))?;
    tool.setattr("_held_points", PyDict::new(py))?;
    Ok(none(py))
}
#[pyfunction(signature=(cls, **kw))]
fn initialize_subclass(cls: &Obj, kw: Option<&Bound<PyDict>>) -> PyResult<()> {
    let py = cls.py();
    let module = imported(py, "tools.rust_tool_base")?;
    py.import("builtins")?
        .getattr("super")?
        .call1((module.getattr("RustTool")?, cls))?
        .call_method("__init_subclass__", (), kw)?;
    let kind = cls.getattr("kind")?;
    if !kind.is_truthy()? {
        return Ok(());
    }
    let info = module
        .call_method1("ToolMachine", (kind,))?
        .getattr("info")?;
    let own = cls.getattr("__dict__")?;
    for key in [
        "name",
        "description",
        "uses_snap",
        "box_select",
        "vcb_label",
        "axis_source",
        "axis_source_empty_only",
        "shortcut",
        "magnetic_axis_deg",
        "screen_axis_px",
    ] {
        if own.contains(key)? {
            continue;
        }
        let value = info.get_item(key)?;
        if !value.is_none() || !matches!(key, "magnetic_axis_deg" | "screen_axis_px") {
            cls.setattr(key, value)?;
        }
    }
    Ok(())
}
#[pyfunction]
fn read_point(key: &str, tool: &Obj) -> PyResult<Py<PyAny>> {
    Ok(tool.call_method1("_point", (key,))?.unbind())
}
#[pyfunction]
fn write_point(key: &str, tool: &Obj, value: &Obj) -> PyResult<()> {
    let value = imported(tool.py(), "tools.rust_tool_base")?.call_method1("_tuple", (value,))?;
    tool.getattr("_machine")?
        .call_method1("set", (key, value))?;
    Ok(())
}
fn point_property<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let key = a.get_item(0)?;
    let partial = py.import("functools")?.getattr("partial")?;
    let get = partial.call1((wrap_pyfunction!(read_point, py)?, &key))?;
    let set = partial.call1((wrap_pyfunction!(write_point, py)?, key))?;
    py.import("builtins")?
        .getattr("property")?
        .call1((get, set))
}

pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    module.add(
        "selection_bounds",
        module.getattr("ToolBounds")?.getattr("selection")?,
    )?;
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("__doc__","The machine's ``ToolHost`` over a viewport (the Metal view's\n``NativeTools``, the controller, or a stand-in with scene and history).")?;
        let cls = define(
            module,
            "_Host",
            &[module.getattr("ViewportToolHost")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
        install_methods(
            module,
            &cls,
            &[(
                "message",
                Mode::Method,
                message,
                &["self", "template", "args"],
                "",
                "",
            )],
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item(
            "__doc__",
            "A tool whose state machine is Rust. Subclasses name the machine.",
        )?;
        attrs.set_item("kind", "")?;
        attrs.set_item("handle_limit", 4096)?;
        attrs.set_item("_READ_DURING", ("start_point", "hover_point"))?;
        attrs.set_item("_during", py.None())?;
        let cls = define(module, "RustTool", &[module.getattr("Tool")?], &attrs)?;
        register_tool_adapter(&cls)?;
        install_methods(
            module,
            &cls,
            &[("__init__", Mode::Method, initialize, &["self"], "", "")],
        )?;
        cls.setattr(
            "__init_subclass__",
            py.import("builtins")?
                .getattr("classmethod")?
                .call1((wrap_pyfunction!(initialize_subclass, module)?,))?,
        )?;
    }
    register_tool_functions(&module.dict())?;
    install_methods(
        module,
        module.dict().as_any(),
        &[(
            "_point_property",
            Mode::Function,
            point_property,
            &["key"],
            "",
            "",
        )],
    )?;
    Ok(())
}
