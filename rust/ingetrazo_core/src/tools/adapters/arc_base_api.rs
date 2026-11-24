// SPDX-License-Identifier: GPL-3.0-or-later
//! _ArcBase callbacks on the live document.
use super::*;

fn segments<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("segments",))
}
fn set_segments<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let n = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.getattr("_machine")?.call_method1(
        "set",
        (
            "segments",
            (if n.is_none() {
                none(py)
            } else {
                py.get_type::<pyo3::types::PyInt>().call1((&n,))?
            }),
        ),
    )?;
    Ok(none(py))
}
fn after<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if tool.getattr("start_point")?.is_none() {
        tool.setattr("wireframe_color", none(py))?;
    }
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn value_is_unitless<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    (tool.getattr("start_point")?.is_none()).into_bound_py_any(py)
}
fn on_activate<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1(
        "_send_framed",
        (&viewport, fields!(py; "type" => "activate").into_any()),
    )?;
    tool.setattr("wireframe_color", none(py))?;
    Ok(none(py))
}
fn on_deactivate<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1(
        "_send_framed",
        (&viewport, fields!(py; "type" => "deactivate").into_any()),
    )?;
    tool.setattr("wireframe_color", none(py))?;
    Ok(none(py))
}
fn on_cancel<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1(
        "_send_framed",
        (&viewport, fields!(py; "type" => "cancel").into_any()),
    )?;
    tool.call_method1("_after", (&viewport,))?;
    Ok(none(py))
}
fn on_click<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.call_method1("note_plane", (ctx.getattr("viewport")?,))?;
    tool.setattr("_viewport", ctx.getattr("viewport")?)?;
    tool.call_method1(
        "_send_framed",
        (
            ctx.getattr("viewport")?,
            tool.call_method1("_pointer", ("click", &ctx))?,
        ),
    )?;
    tool.call_method1("_after", (ctx.getattr("viewport")?,))?;
    Ok(none(py))
}
fn on_hover<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.call_method1("note_plane", (ctx.getattr("viewport")?,))?;
    tool.setattr("_viewport", ctx.getattr("viewport")?)?;
    tool.call_method1(
        "_send_framed",
        (
            ctx.getattr("viewport")?,
            tool.call_method1("_pointer", ("hover", &ctx))?,
        ),
    )?;
    tool.setattr("wireframe_color", tool.call_method0("_cue_colour")?)?;
    ctx.getattr("viewport")?.call_method0("update")?;
    Ok(none(py))
}
fn cue_colour<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method0("lock_color")
}
fn on_value<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let value = a.get_item(2)?;
    let viewport = a.get_item(1)?;
    tool.setattr("_viewport", &viewport)?;
    if value.is_instance(&py.get_type::<pyo3::types::PyTuple>().into_any())? {
        return false.into_bound_py_any(py);
    }
    let consumed = tool.call_method1("_send_framed", (&viewport, fields!(py; "type" => "value", "value" => py.get_type::<pyo3::types::PyFloat>().call1((&value,))?).into_any(),))?.get_item("consumed")?;
    tool.call_method1("_after", (&viewport,))?;
    Ok(consumed.clone())
}
fn on_segments_value<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let n = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.setattr("_viewport", &viewport)?;
    let consumed = tool.call_method1("_send_framed", (&viewport, fields!(py; "type" => "segments", "count" => py.get_type::<pyo3::types::PyInt>().call1((&n,))?).into_any(),))?.get_item("consumed")?;
    if consumed.is_truthy()? {
        viewport.call_method0("update")?;
    }
    Ok(consumed.clone())
}
fn on_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("plane_lock_key", (&viewport, &key))
}
fn query<'py>(a: &Args<'py>) -> Reply<'py> {
    let arg = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let what = a.get_item(1)?;
    if !tool.getattr("_viewport")?.is_none() {
        tool.call_method1("_frame", (tool.getattr("_viewport")?,))?;
    }
    tool.getattr("_machine")?
        .call_method1("query", (&what, &arg))
}
fn axes<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.rust_tools")?;
    let tool = a.get_item(0)?;
    let u: Obj<'py>;
    let v: Obj<'py>;
    [u, v] = unpack::<2>(&tool.call_method1("_query", ("axes",))?)?;
    (
        module.call_method1("Vec3", positional(&u)?)?,
        module.call_method1("Vec3", positional(&v)?)?,
    )
        .into_bound_py_any(py)
}
fn reset<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    tool.getattr("_machine")?
        .call_method1("set", ("reset", true))?;
    tool.setattr("wireframe_color", none(py))?;
    Ok(none(py))
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("segments", Mode::Property, segments, &["self"], "", ""),
    ("_after", Mode::Method, after, &["self", "viewport"], "", ""),
    ("value_is_unitless", Mode::Method, value_is_unitless, &["self"], "bool", "Before the first click the typed number is a COUNT (#176)."),
    ("on_activate", Mode::Method, on_activate, &["self", "viewport"], "None", ""),
    ("on_deactivate", Mode::Method, on_deactivate, &["self", "viewport"], "None", ""),
    ("on_cancel", Mode::Method, on_cancel, &["self", "viewport"], "None", ""),
    ("on_click", Mode::Method, on_click, &["self", "ctx"], "None", ""),
    ("on_hover", Mode::Method, on_hover, &["self", "ctx"], "None", ""),
    ("_cue_colour", Mode::Method, cue_colour, &["self"], "", ""),
    ("on_value", Mode::Method, on_value, &["self", "viewport", "value"], "bool", ""),
    ("on_segments_value", Mode::Method, on_segments_value, &["self", "viewport", "n"], "bool", ""),
    ("on_key", Mode::Method, on_key, &["self", "viewport", "key", "modifiers"], "bool", ""),
    ("_query", Mode::Method, query, &["self", "what", "arg=None"], "", ""),
    ("_axes", Mode::Method, axes, &["self"], "", ""),
    ("_reset", Mode::Method, reset, &["self"], "", ""),
    ("segments", Mode::Setter, set_segments, &["self", "n"], "", ""),
];
