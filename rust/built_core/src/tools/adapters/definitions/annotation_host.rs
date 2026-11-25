// SPDX-License-Identifier: GPL-3.0-or-later
//! Plugin-facing definitions for annotation_host.
use super::*;

#[pyfunction]
fn set_free(tool: &Obj, value: &Obj) -> PyResult<()> {
    let py = tool.py();
    let value = if value.is_truthy()? {
        let point =
            imported(py, "tools.annotation_host")?.call_method1("_tuple", (value.get_item(0)?,))?;
        (point, value.get_item(1)?).into_bound_py_any(py)?
    } else {
        none(py)
    };
    tool.getattr("_machine")?
        .call_method1("set", ("free", value))?;
    Ok(())
}
#[pyfunction]
fn place_label(vp: &Obj, args: &Bound<PyDict>, text: &Obj) -> PyResult<()> {
    if text.is_truthy()? {
        let stripped = text.call_method0("strip")?;
        if stripped.is_truthy()? {
            let fields = args.copy()?;
            fields.set_item("text", stripped)?;
            imported(vp.py(), "built_core.command_protocol")?.call_method(
                "run",
                (vp.getattr("history")?, "add_text_label"),
                Some(&fields),
            )?;
        }
    }
    vp.call_method0("update")?;
    Ok(())
}
fn prompt<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let args = a.get_item(2)?;
    let tr = imported(py, "built_core.i18n")?.getattr("tr")?;
    tool.setattr("work_plane", py.None())?;
    let done = py
        .import("functools")?
        .call_method1("partial", (wrap_pyfunction!(place_label, py)?, &vp, &args))?;
    imported(py, "built_render")?
        .getattr("chrome_forms")?
        .call_method(
            "ask_text",
            (
                vp.call_method0("window")?,
                tr.call1(("Text",))?,
                tr.call1(("Label text:",))?,
                args.get_item("text")?,
            ),
            Some(&fields!(py;"done"=>done,"multiline"=>true)),
        )?;
    Ok(none(py))
}
pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("work_plane", py.None())?;
        attrs.set_item("_ctx", py.None())?;
        attrs.set_item("_vp", attrs.get_item("_ctx")?.unwrap())?;
        let cls = define(
            module,
            "_AnnotationTool",
            &[module.getattr("AxisMagnet")?, module.getattr("RustTool")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "text")?;
        attrs.set_item(
            "anchor",
            module.getattr("_point_property")?.call1(("start_point",))?,
        )?;
        attrs.set_item("start_point", attrs.get_item("anchor")?.unwrap())?;
        attrs.set_item(
            "hover_point",
            module.getattr("_point_property")?.call1(("hover_point",))?,
        )?;
        let cls = define(
            module,
            "TextTool",
            &[module.getattr("_AnnotationTool")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
        install_methods(
            module,
            &cls,
            &[(
                "host_text_prompt",
                Mode::Method,
                prompt,
                &["self", "vp", "args"],
                "",
                "",
            )],
        )?;
    }
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("kind", "dimension")?;
        attrs.set_item("a", module.getattr("_point_property")?.call1(("a",))?)?;
        attrs.set_item("start_point", attrs.get_item("a")?.unwrap())?;
        attrs.set_item("b", module.getattr("_point_property")?.call1(("b",))?)?;
        attrs.set_item(
            "hover_point",
            module.getattr("_point_property")?.call1(("hover_point",))?,
        )?;
        let cls = define(
            module,
            "DimensionTool",
            &[module.getattr("_AnnotationTool")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
        cls.setattr(
            "_free",
            py.import("builtins")?.getattr("property")?.call(
                (),
                Some(&fields!(py;"fset"=>wrap_pyfunction!(set_free,py)?)),
            )?,
        )?;
    }
    Ok(())
}
