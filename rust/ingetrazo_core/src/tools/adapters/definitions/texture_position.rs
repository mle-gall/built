// SPDX-License-Identifier: GPL-3.0-or-later
//! Plugin-facing definitions for texture_position.
use super::*;

fn initialize<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    for key in ["face", "_tex0", "_placement", "_plane"] {
        tool.setattr(key, a.py().None())?;
    }
    tool.setattr("side", "front")?;
    Ok(none(a.py()))
}
fn context_menu<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if !tool.getattr("active")?.is_truthy()? {
        return false.into_bound_py_any(py);
    }
    let vp = a.get_item(1)?;
    let point = a.get_item(2)?;
    let rows = tool.getattr("_placement")?.call_method0("menu_rows")?;
    if imported(py, "core.input_types")?
        .getattr("NATIVE")?
        .is_truthy()?
    {
        let renderer = imported(py, "ingetrazo_render")?;
        let view = vp.getattr("view")?;
        let pos = view.call_method1("mapFromGlobal", (point,))?;
        let tr = imported(py, "tools.texture_position")?.getattr("tr")?;
        let groups = PyList::empty(py);
        for row in rows.try_iter()? {
            let [group, items] = unpack(&row?)?;
            let labels = PyList::empty(py);
            for item in items.try_iter()? {
                let [key, label, on] = unpack(&item?)?;
                labels.append((key, tr.call1((label,))?, on))?;
            }
            let title = if group.is_truthy()? {
                tr.call1((group,))?
            } else {
                "".into_bound_py_any(py)?
            };
            groups.append((title, labels))?;
        }
        let menu = renderer.getattr("ViewportMenu")?.call_method1(
            "for_tool",
            (
                tool,
                tr.call1(("Position Texture",))?,
                groups,
                (pos.call_method0("x")?, pos.call_method0("y")?),
            ),
        )?;
        renderer.call_method1("context_menu_send", (view, menu))?;
    } else {
        imported(py, "tools._classic")?.call_method1("texture_menu", (tool, vp, point, rows))?;
    }
    true.into_bound_py_any(py)
}
pub(super) fn install(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    for (value, name) in [
        "PIN_MOVE",
        "PIN_SCALE_ROTATE",
        "PIN_SCALE_SHEAR",
        "PIN_DISTORT",
    ]
    .iter()
    .enumerate()
    {
        module.add(*name, value)?;
    }
    module.add(
        "PIN_COLORS",
        (
            (0.86, 0.22, 0.27),
            (0.16, 0.62, 0.36),
            (0.2, 0.4, 0.78),
            (0.95, 0.78, 0.2),
        ),
    )?;
    module.add(
        "PIN_NAMES",
        ("Move", "Scale / Rotate", "Scale / Shear", "Distort"),
    )?;
    {
        let attrs = PyDict::new(py);
        attrs.set_item("__module__", module.name()?)?;
        attrs.set_item("name", "Position Texture")?;
        attrs.set_item("icon", "texture_position")?;
        attrs.set_item("uses_snap", false)?;
        attrs.set_item(
            "qt_cursor",
            module.getattr("Qt")?.getattr("OpenHandCursor")?,
        )?;
        attrs.set_item("preview_opacity", 0.75)?;
        attrs.set_item("GRAB_PX", 12.0)?;
        attrs.set_item("CLICK_PX", 4.0)?;
        attrs.set_item("PIN_PX", 7.0)?;
        attrs.set_item("DISC_PX", 30.0)?;
        attrs.set_item("TICK_DEG", 15.0)?;
        attrs.set_item("SNAP_PX", 10.0)?;
        attrs.set_item("overlay_kind", "texture_position")?;
        attrs.set_item("draw_overlay", module.getattr("draw_overlay")?)?;
        let cls = define(
            module,
            "TexturePositionTool",
            &[module.getattr("Tool")?],
            &attrs,
        )?;
        register_tool_adapter(&cls)?;
        install_methods(
            module,
            &cls,
            &[
                ("__init__", Mode::Method, initialize, &["self"], "None", ""),
                (
                    "context_menu",
                    Mode::Method,
                    context_menu,
                    &["self", "viewport", "global_pos"],
                    "bool",
                    "The tool's own right-click menu (replaces the viewport's).",
                ),
            ],
        )?;
    }
    Ok(())
}
