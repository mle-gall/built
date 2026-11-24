// SPDX-License-Identifier: GPL-3.0-or-later
//! SelectTool callbacks on the live document.
use super::*;

fn px<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.rust_tools")?;
    let kind = a.get_item(2)?;
    let modifiers = a.get_item(5)?;
    let tool = a.get_item(0)?;
    let t = a.get_item(6)?;
    let viewport = a.get_item(1)?;
    let x = a.get_item(3)?;
    let y = a.get_item(4)?;
    Ok(tool.call_method1("_send", (&viewport, fields!(py; "type" => &kind, "px" => (&x, &y,).into_bound_py_any(py)?, "mods" => module.call_method1("_mods", (&modifiers,))?, "t" => &t).into_any(),))?)
}
fn hover_at<'py>(a: &Args<'py>) -> Reply<'py> {
    let modifiers = a.get_item(4)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let x = a.get_item(2)?;
    let y = a.get_item(3)?;
    tool.call_method1("_px", (&viewport, "hover", &x, &y, &modifiers))
}
fn on_hover<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.call_method1(
        "_send",
        (
            ctx.getattr("viewport")?,
            tool.call_method1("_pointer", ("hover", &ctx))?,
        ),
    )?;
    Ok(none(py))
}
fn pressed<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    (tool.call_method1("_state", ("pressed",))?.is_truthy()?).into_bound_py_any(py)
}
fn pick_at<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.rust_tools")?;
    let lines = a.get_item(4)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let x = a.get_item(2)?;
    let y = a.get_item(3)?;
    let select_pick = py.import("ingetrazo_core")?.getattr("select_pick")?;
    let hit = select_pick.call1((
        module.call_method1("_Host", (&viewport, tool.getattr("_handles")?))?,
        py.get_type::<pyo3::types::PyFloat>().call1((&x,))?,
        py.get_type::<pyo3::types::PyFloat>().call1((&y,))?,
        (lines.is_truthy()?).into_bound_py_any(py)?,
    ))?;
    Ok(if hit.is_none() {
        none(py)
    } else {
        tool.getattr("_handles")?.call_method1("get", (&hit,))?
    })
}
fn press<'py>(a: &Args<'py>) -> Reply<'py> {
    let modifiers = a.get_item(4)?;
    let tool = a.get_item(0)?;
    let t = a.get_item(5)?;
    let viewport = a.get_item(1)?;
    let x = a.get_item(2)?;
    let y = a.get_item(3)?;
    tool.call_method1("_px", (&viewport, "press", &x, &y, &modifiers, &t))
}
fn drag<'py>(a: &Args<'py>) -> Reply<'py> {
    let modifiers = a.get_item(4)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let x = a.get_item(2)?;
    let y = a.get_item(3)?;
    tool.call_method1("_px", (&viewport, "drag", &x, &y, &modifiers))?;
    tool.getattr("_machine")?
        .call_method0("preview")?
        .get_item("box")
}
fn release<'py>(a: &Args<'py>) -> Reply<'py> {
    let modifiers = a.get_item(4)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let x = a.get_item(2)?;
    let y = a.get_item(3)?;
    tool.call_method1("_px", (&viewport, "release", &x, &y, &modifiers))
}
fn double<'py>(a: &Args<'py>) -> Reply<'py> {
    let modifiers = a.get_item(4)?;
    let tool = a.get_item(0)?;
    let t = a.get_item(5)?;
    let viewport = a.get_item(1)?;
    let x = a.get_item(2)?;
    let y = a.get_item(3)?;
    tool.call_method1("_px", (&viewport, "double_click", &x, &y, &modifiers, &t))
}
fn on_double_click<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.call_method1(
        "_send",
        (
            ctx.getattr("viewport")?,
            tool.call_method1("_pointer", ("double_click", &ctx))?,
        ),
    )?;
    ctx.getattr("viewport")?.call_method0("update")?;
    Ok(none(py))
}
fn on_triple_click<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.call_method1(
        "_send",
        (
            ctx.getattr("viewport")?,
            tool.call_method1("_pointer", ("triple_click", &ctx))?,
        ),
    )?;
    ctx.getattr("viewport")?.call_method0("update")?;
    Ok(none(py))
}
fn on_box_select<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let additive = a.get_item(4)?;
    let crossing = a.get_item(3)?;
    let mut mode = a.get_item(5)?;
    let rect = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    mode = {
        let value = mode.clone();
        if value.is_truthy()? {
            value
        } else {
            if additive.is_truthy()? {
                ("add").into_bound_py_any(py)?
            } else {
                ("replace").into_bound_py_any(py)?
            }
        }
    };
    tool.call_method1("_send", (&viewport, fields!(py; "type" => "box", "rect" => positional(&rect)?, "crossing" => (crossing.is_truthy()?).into_bound_py_any(py)?, "mods" => tool.getattr("_MODE_MODS")?.get_item(&mode)?).into_any(),))?;
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn on_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.rust_tools")?;
    let key = a.get_item(2)?;
    let modifiers = a.get_item(3)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let qt = py.import("core.input_types")?.getattr("Qt")?;
    let name = fields!(py; qt.getattr("Key_Delete")? => "Delete", qt.getattr("Key_Backspace")? => "Backspace").into_any().call_method1("get", (&key,))?;
    if name.is_none() {
        return false.into_bound_py_any(py);
    }
    Ok(tool.call_method1("_send", (&viewport, fields!(py; "type" => "key", "key" => &name, "mods" => module.call_method1("_mods", (&modifiers,))?).into_any(),))?.get_item("consumed")?)
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("_px", Mode::Method, px, &["self", "viewport", "kind", "x", "y", "modifiers", "t=0.0"], "", ""),
    ("hover_at", Mode::Method, hover_at, &["self", "viewport", "x", "y", "modifiers"], "", ""),
    ("on_hover", Mode::Method, on_hover, &["self", "ctx"], "None", ""),
    ("pressed", Mode::Method, pressed, &["self"], "bool", "A press is held that its release will answer (not a triple)."),
    ("pick_at", Mode::Method, pick_at, &["self", "viewport", "x", "y", "lines=True"], "", "What a click at ``(x, y)`` takes (the machine's cascade)."),
    ("press", Mode::Method, press, &["self", "viewport", "x", "y", "modifiers", "t"], "", ""),
    ("drag", Mode::Method, drag, &["self", "viewport", "x", "y", "modifiers"], "", ""),
    ("release", Mode::Method, release, &["self", "viewport", "x", "y", "modifiers"], "", ""),
    ("double", Mode::Method, double, &["self", "viewport", "x", "y", "modifiers", "t"], "", ""),
    ("on_double_click", Mode::Method, on_double_click, &["self", "ctx"], "None", ""),
    ("on_triple_click", Mode::Method, on_triple_click, &["self", "ctx"], "None", ""),
    ("on_box_select", Mode::Method, on_box_select, &["self", "viewport", "rect", "crossing:bool", "additive:bool=False", "mode:str | None=None"], "None", ""),
    ("on_key", Mode::Method, on_key, &["self", "viewport", "key", "modifiers"], "bool", ""),
];
