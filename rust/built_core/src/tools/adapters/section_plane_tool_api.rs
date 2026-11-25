// SPDX-License-Identifier: GPL-3.0-or-later
//! SectionPlaneTool callbacks on the live document.
use super::*;

fn current_normal<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_point", ("current",))
}
fn on_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.section_host")?;
    let key = a.get_item(2)?;
    let modifiers = a.get_item(3)?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let qt = imported(py, "built_core")?.getattr("Input")?;
    let arrow = fields!(py; qt.getattr("Key_Right")? => "ArrowRight", qt.getattr("Key_Left")? => "ArrowLeft", qt.getattr("Key_Up")? => "ArrowUp", qt.getattr("Key_Down")? => "ArrowDown").into_any().call_method1("get", (&key,))?;
    if !arrow.is_truthy()? {
        return false.into_bound_py_any(py);
    }
    tool.call_method1("_send", (&vp, fields!(py; "type" => "key", "key" => &arrow, "mods" => module.call_method1("_mods", (&modifiers,))?).into_any(),))?;
    vp.call_method0("update")?;
    true.into_bound_py_any(py)
}
fn on_click<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    let tr = imported(py, "built_core.i18n")?.getattr("tr")?;
    let vp = ctx.getattr("viewport")?;
    if !(tool
        .call_method1(
            "_send",
            (&vp, tool.call_method1("_pointer", ("click", &ctx))?),
        )?
        .get_item("committed")?
        .is_truthy()?)
    {
        return Ok(none(py));
    }
    let plane = vp
        .getattr("scene")?
        .getattr("section_planes")?
        .get_item(1.into_bound_py_any(py)?.neg()?)?;
    let window = if vp.hasattr("window")? {
        vp.call_method0("window")?
    } else {
        none(py)
    };
    if !(vp
        .getattr("scene")?
        .getattr("show_section_cuts")?
        .is_truthy()?
        && vp
            .getattr("scene")?
            .getattr("show_section_planes")?
            .is_truthy()?)
    {
        {
            let value = true.into_bound_py_any(py)?;
            vp.getattr("scene")?.setattr("show_section_cuts", &value)?;
            vp.getattr("scene")?
                .setattr("show_section_planes", &value)?;
        }
        if window.hasattr("_sync_section_menu")? {
            window.call_method0("_sync_section_menu")?;
        }
    }
    if window.hasattr("prompt_section_name")? {
        window.call_method1("prompt_section_name", (&plane,))?;
    }
    vp.call_method1(
        "flash_status",
        (
            tr.call1((
                "Section plane placed — double-click toggles the cut; Move/Rotate reposition it",
            ))?,
            4000,
        ),
    )?;
    vp.call_method0("update")?;
    if window.hasattr("_activate_tool")? {
        window.call_method1("_activate_tool", ("select",))?;
    }
    Ok(none(py))
}

fn wireframe_color<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let plane_lock = imported(py, "tools.base")?.getattr("PlaneLock")?;
    let axis = plane_lock.call_method1("plane_color", (tool.call_method0("_current_normal")?,))?;
    if !axis.is_none() {
        return Ok(axis.clone());
    }
    let color_reference = imported(py, "core.snap")?.getattr("COLOR_REFERENCE")?;
    Ok({
        let values = PyList::empty(py);
        values.call_method1("extend", (&color_reference,))?;
        values.append(1.0)?;
        positional(values.as_any())?.into_any()
    })
}
fn send<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.section_host")?;
    let event = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let axes = imported(py, "built_core.axes")?.into_any();
    if ("hover", "click", "key")
        .into_bound_py_any(py)?
        .contains(event.get_item("type")?)?
    {
        imported(py, "builtins")?
            .getattr("super")?
            .call1((module.getattr("SectionPlaneTool")?, &tool))?
            .call_method1(
                "_send",
                (
                    &vp,
                    fields!(py; "type" => "frame", "axes" => axes.call_method0("frame")?)
                        .into_any(),
                ),
            )?;
    }
    imported(py, "builtins")?
        .getattr("super")?
        .call1((module.getattr("SectionPlaneTool")?, &tool))?
        .call_method1("_send", (&vp, &event))
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("wireframe_color", Mode::Property, wireframe_color, &["self"], "", "The floating plane wears the inference colours (@pacaeiro, #62):\nred/green/blue square to an axis, magenta on any other plane."),
    ("_current_normal", Mode::Method, current_normal, &["self"], "", ""),
    ("_send", Mode::Method, send, &["self", "vp", "event"], "", ""),
    ("on_key", Mode::Method, on_key, &["self", "vp", "key", "modifiers"], "", ""),
    ("on_click", Mode::Method, on_click, &["self", "ctx"], "", ""),
];
