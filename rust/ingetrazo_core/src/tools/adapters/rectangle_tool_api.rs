// SPDX-License-Identifier: GPL-3.0-or-later
//! RectangleTool callbacks on the live document.
use super::*;

fn chain_first_point<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    Ok(none(py))
}
fn set_chain_first_point<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    Ok(none(py))
}
fn from_center<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    (tool.call_method1("_state", ("from_center",))?.is_truthy()?).into_bound_py_any(py)
}
fn set_center_flag<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let on = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.getattr("_machine")?.call_method1(
        "set",
        ("from_center", (on.is_truthy()?).into_bound_py_any(py)?),
    )?;
    Ok(none(py))
}
fn icon<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok(if tool.getattr("_from_center")?.is_truthy()? {
        ("rectangle_center").into_bound_py_any(py)?
    } else {
        ("rectangle").into_bound_py_any(py)?
    })
}
fn set_icon<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    Ok(none(py))
}
fn last<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok(if tool.call_method1("_state", ("retype",))?.is_truthy()? {
        true.into_bound_py_any(py)?
    } else {
        none(py)
    })
}
fn set_from_center<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let on = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.setattr("_from_center", &on)?;
    let apply = optional(&viewport, "_apply_tool_cursor")?;
    if apply.is_callable() {
        apply.call0()?;
    }
    Ok(none(py))
}
fn plane_points<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if tool.getattr("start_point")?.is_none() {
        return Ok(PyList::empty(py).into_any());
    }
    Ok(PyList::new(
        py,
        [tool.getattr("start_point")?, tool.getattr("hover_point")?],
    )?
    .into_any())
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
    tool.call_method1("_set_from_center", (&viewport, false))?;
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
    tool.setattr("wireframe_color", none(py))?;
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn on_click<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.call_method1("note_plane", (ctx.getattr("viewport")?,))?;
    let reply = tool.call_method1(
        "_send_framed",
        (
            ctx.getattr("viewport")?,
            tool.call_method1("_pointer", ("click", &ctx))?,
        ),
    )?;
    if tool.getattr("start_point")?.is_none() {
        tool.setattr("wireframe_color", none(py))?;
    }
    ctx.getattr("viewport")?.call_method0("update")?;
    Ok(reply.clone())
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
    tool.setattr("wireframe_color", tool.call_method0("lock_color")?)?;
    ctx.getattr("viewport")?.call_method0("update")?;
    Ok(none(py))
}
fn on_value<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let value = a.get_item(2)?;
    let viewport = a.get_item(1)?;
    let consumed = tool
        .call_method1(
            "_send_framed",
            (
                &viewport,
                fields!(py; "type" => "value", "value" => &value).into_any(),
            ),
        )?
        .get_item("consumed")?;
    if consumed.is_truthy()? && tool.getattr("start_point")?.is_none() {
        tool.setattr("wireframe_color", none(py))?;
    }
    Ok(consumed.clone())
}
fn on_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let key = a.get_item(2)?;
    let modifiers = a.get_item(3)?;
    let tool = a.get_item(0)?;
    let update: Obj<'py>;
    let viewport = a.get_item(1)?;
    let qt = imported(py, "ingetrazo_core")?.getattr("Input")?;
    let name = fields!(py; qt.getattr("Key_Control")? => "Control", qt.getattr("Key_Down")? => "ArrowDown", qt.getattr("Key_Right")? => "ArrowRight", qt.getattr("Key_Left")? => "ArrowLeft", qt.getattr("Key_Up")? => "ArrowUp").into_any().call_method1("get", (&key,))?;
    if !name.is_none() && tool.call_method1("_send_framed", (&viewport, fields!(py; "type" => "key", "key" => &name, "mods" => module.call_method1("_mods", (&modifiers,))?).into_any(),))?.get_item("consumed")?.is_truthy()? {
if name.eq("Control")? {
tool.call_method1("_set_from_center", (&viewport, tool.getattr("_from_center")?,))?;
tool.call_method1("refresh_hint", (&viewport,))?;
}
update = optional(&viewport, "update")?;
if update.is_callable() {
update.call0()?;
}
return true.into_bound_py_any(py);
}
    tool.call_method1("plane_lock_key", (&viewport, &key))
}
fn on_key_release<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let qt = imported(py, "ingetrazo_core")?.getattr("Input")?;
    if key.ne(qt.getattr("Key_Control")?)? {
        return false.into_bound_py_any(py);
    }
    let tapped = optional(&viewport, "ctrl_tapped")?;
    let reply = tool.call_method1("_send_framed", (&viewport, fields!(py; "type" => "key_release", "key" => "Control", "tapped" => { let value = (!(tapped.is_callable())).into_bound_py_any(py)?; if value.is_truthy()? {value} else {(tapped.call0()?.is_truthy()?).into_bound_py_any(py)?} }).into_any(),))?;
    if reply.get_item("consumed")?.is_truthy()? {
        tool.call_method1(
            "_set_from_center",
            (&viewport, tool.getattr("_from_center")?),
        )?;
    }
    reply.get_item("consumed")
}
fn value_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let du: Obj<'py>;
    let dv: Obj<'py>;
    let mid: Obj<'py>;
    let tool = a.get_item(0)?;
    let square: Obj<'py>;
    let label = tool.call_method1("_state", ("label",))?;
    if label.is_none() {
        return Ok(none(py));
    }
    let fmt_pair = imported(py, "core.units")?.getattr("fmt_pair")?;
    [du, dv, square, mid] = unpack::<4>(&label)?;
    let text = fmt_pair.call1((&du, &dv))?.add(if square.is_truthy()? {
        ("  (Cuadrado)").into_bound_py_any(py)?
    } else {
        ("").into_bound_py_any(py)?
    })?;
    (&text, module.call_method1("Vec3", positional(&mid)?)?).into_bound_py_any(py)
}
fn reset<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    tool.getattr("_machine")?
        .call_method1("set", ("reset", true))?;
    tool.setattr("wireframe_color", none(py))?;
    Ok(none(py))
}
fn query<'py>(a: &Args<'py>) -> Reply<'py> {
    let arg = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let what = a.get_item(1)?;
    tool.call_method1("_frame", (tool.getattr("_viewport")?,))?;
    tool.getattr("_machine")?
        .call_method1("query", (&what, &arg))
}
fn span<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let first: Obj<'py>;
    let b: Obj<'py>;
    let cursor = a.get_item(1)?;
    let tool = a.get_item(0)?;
    [first, b] = unpack::<2>(&tool.call_method1(
        "_query",
        ("span", module.call_method1("_tuple", (&cursor,))?),
    )?)?;
    (
        module.call_method1("Vec3", positional(&first)?)?,
        module.call_method1("Vec3", positional(&b)?)?,
    )
        .into_bound_py_any(py)
}
fn dimensions<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let first = a.get_item(1)?;
    let b = a.get_item(2)?;
    let tool = a.get_item(0)?;
    py.get_type::<pyo3::types::PyTuple>()
        .call1((tool.call_method1(
            "_query",
            (
                "dimensions",
                (
                    module.call_method1("_tuple", (&first,))?,
                    module.call_method1("_tuple", (&b,))?,
                )
                    .into_bound_py_any(py)?,
            ),
        )?,))
}
fn square_locked<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    (tool.call_method1("_state", ("square",))?.is_truthy()?).into_bound_py_any(py)
}
fn square_corner<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let first = a.get_item(1)?;
    let b = a.get_item(2)?;
    let far: Obj<'py>;
    let tool = a.get_item(0)?;
    let square: Obj<'py>;
    [far, square] = unpack::<2>(
        &tool.call_method1(
            "_query",
            (
                "square_corner",
                (
                    module.call_method1("_tuple", (&first,))?,
                    module.call_method1("_tuple", (&b,))?,
                )
                    .into_bound_py_any(py)?,
            ),
        )?,
    )?;
    (
        module.call_method1("Vec3", positional(&far)?)?,
        (square.is_truthy()?).into_bound_py_any(py)?,
    )
        .into_bound_py_any(py)
}
fn corners<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let first = a.get_item(1)?;
    let b = a.get_item(2)?;
    let tool = a.get_item(0)?;
    return Ok({
        let items = PyList::empty(py);
        for item in tool
            .call_method1(
                "_query",
                (
                    "corners",
                    (
                        module.call_method1("_tuple", (&first,))?,
                        module.call_method1("_tuple", (&b,))?,
                    )
                        .into_bound_py_any(py)?,
                ),
            )?
            .try_iter()?
        {
            let p = item?;
            items.append(module.call_method1("Vec3", positional(&p)?)?)?;
        }
        items.into_any()
    });
}

fn degenerate_reason<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let arguments: Obj<'py>;
    let tool = a.get_item(0)?;
    let template: Obj<'py>;
    let world = a.get_item(1)?;
    let tr = imported(py, "ingetrazo_core.i18n")?.getattr("tr")?;
    {
        let [part0, part1] = unpack::<2>(&tool.call_method1(
            "_query",
            ("degenerate", module.call_method1("_tuple", (&world,))?),
        )?)?;
        template = part0;
        arguments = part1;
    }
    Ok(if arguments.is_truthy()? {
        tr.call((&template,), Some(arguments.cast::<PyDict>()?))?
    } else {
        tr.call1((&template,))?
    })
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("chain_first_point", Mode::Property, chain_first_point, &["self"], "", ""),
    ("_from_center", Mode::Property, from_center, &["self"], "", ""),
    ("icon", Mode::Property, icon, &["self"], "", ""),
    ("_last", Mode::Property, last, &["self"], "", ""),
    ("_set_from_center", Mode::Method, set_from_center, &["self", "viewport", "on"], "", ""),
    ("plane_points", Mode::Method, plane_points, &["self"], "", ""),
    ("on_activate", Mode::Method, on_activate, &["self", "viewport"], "None", ""),
    ("on_deactivate", Mode::Method, on_deactivate, &["self", "viewport"], "None", ""),
    ("on_cancel", Mode::Method, on_cancel, &["self", "viewport"], "None", ""),
    ("on_click", Mode::Method, on_click, &["self", "ctx"], "None", ""),
    ("on_hover", Mode::Method, on_hover, &["self", "ctx"], "None", ""),
    ("on_value", Mode::Method, on_value, &["self", "viewport", "value"], "bool", ""),
    ("on_key", Mode::Method, on_key, &["self", "viewport", "key", "modifiers"], "bool", ""),
    ("on_key_release", Mode::Method, on_key_release, &["self", "viewport", "key"], "bool", ""),
    ("value_label", Mode::Method, value_label, &["self"], "", "``(text, point)``: the floating ``width × height`` readout, with\n\"(Cuadrado)\" when the sides are equal."),
    ("_reset", Mode::Method, reset, &["self"], "", ""),
    ("_query", Mode::Method, query, &["self", "what", "arg"], "", ""),
    ("_span", Mode::Method, span, &["self", "cursor"], "", ""),
    ("_dimensions", Mode::Method, dimensions, &["self", "a", "b"], "", ""),
    ("_square_locked", Mode::Method, square_locked, &["self"], "", ""),
    ("_square_corner", Mode::Method, square_corner, &["self", "a", "b"], "", ""),
    ("_corners", Mode::Method, corners, &["self", "a", "b"], "", "The four corners of the rectangle spanning ``a``–``b`` on the\ndrawing plane."),
    ("_degenerate_reason", Mode::Method, degenerate_reason, &["self", "world"], "str", ""),
    ("chain_first_point", Mode::Setter, set_chain_first_point, &["self", "_value"], "", ""),
    ("_from_center", Mode::Setter, set_center_flag, &["self", "on"], "", ""),
    ("icon", Mode::Setter, set_icon, &["self", "_value"], "", ""),
];
