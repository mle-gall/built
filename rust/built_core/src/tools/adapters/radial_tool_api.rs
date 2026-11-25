// SPDX-License-Identifier: GPL-3.0-or-later
//! _RadialTool callbacks on the live document.
use super::*;

fn sides<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    py.get_type::<pyo3::types::PyInt>()
        .call1((tool.call_method1("_state", ("sides",))?,))
}
fn set_sides<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let n = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.getattr("_machine")?.call_method1(
        "set",
        ("sides", py.get_type::<pyo3::types::PyInt>().call1((&n,))?),
    )?;
    Ok(none(py))
}
fn guide_mode<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    (tool.call_method1("_state", ("guide_mode",))?.is_truthy()?).into_bound_py_any(py)
}
fn set_guide_mode<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let on = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.getattr("_machine")?.call_method1(
        "set",
        ("guide_mode", (on.is_truthy()?).into_bound_py_any(py)?),
    )?;
    Ok(none(py))
}
fn last<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("last_token",))
}
fn vcb_caption<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if !tool.getattr("start_point")?.is_none()
        || tool.call_method0("_radius_retype_open")?.is_truthy()?
    {
        return ("Radius").into_bound_py_any(py);
    }
    ("Sides").into_bound_py_any(py)
}
fn radius_retype_open<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let tool = a.get_item(0)?;
    let token = tool.getattr("_last")?;
    Ok({
        let value = (tool.getattr("start_point")?.is_none()).into_bound_py_any(py)?;
        if !value.is_truthy()? {
            value
        } else {
            {
                let value = (!token.is_none()).into_bound_py_any(py)?;
                if !value.is_truthy()? {
                    value
                } else {
                    {
                        let value =
                            (!tool.getattr("_viewport")?.is_none()).into_bound_py_any(py)?;
                        if !value.is_truthy()? {
                            value
                        } else {
                            (module
                                .call_method1("_Host", (tool.getattr("_viewport")?,))?
                                .call_method0("revision")?
                                .eq(&token)?)
                            .into_bound_py_any(py)?
                        }
                    }
                }
            }
        }
    })
}
fn value_is_unitless<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok({
        let value = (tool.getattr("start_point")?.is_none()).into_bound_py_any(py)?;
        if !value.is_truthy()? {
            value
        } else {
            (!(tool.call_method0("_radius_retype_open")?.is_truthy()?)).into_bound_py_any(py)?
        }
    })
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
    tool.setattr("wireframe_color", none(py))?;
    viewport.call_method0("update")?;
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
    if tool.getattr("start_point")?.is_none() {
        tool.setattr("wireframe_color", none(py))?;
    }
    ctx.getattr("viewport")?.call_method0("update")?;
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
    let snap = ctx.getattr("snap")?;
    if !snap.is_none()
        && snap.getattr("kind")?.eq("axis")?
        && tool.getattr("plane_lock")?.is_none()
        && tool.getattr("plane_ref")?.is_none()
    {
        tool.setattr("wireframe_color", none(py))?;
    } else {
        tool.setattr("wireframe_color", tool.call_method0("lock_color")?)?;
    }
    ctx.getattr("viewport")?.call_method0("update")?;
    Ok(none(py))
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
    if consumed.is_truthy()? {
        if tool.getattr("start_point")?.is_none() {
            tool.setattr("wireframe_color", none(py))?;
        }
        viewport.call_method0("update")?;
    }
    Ok(consumed.clone())
}
fn refresh<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let viewport = a.get_item(1)?;
    let hint = optional(&viewport, "refresh_status_hint")?;
    if hint.is_callable() {
        hint.call0()?;
    }
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn on_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let key = a.get_item(2)?;
    let modifiers = a.get_item(3)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let qt = imported(py, "built_core")?.getattr("Input")?;
    if key.eq(qt.getattr("Key_Control")?)? && tool.getattr("_guide_capable")?.is_truthy()? {
        tool.call_method1("_send_framed", (&viewport, fields!(py; "type" => "key", "key" => "Control", "mods" => module.call_method1("_mods", (&modifiers,))?).into_any(),))?;
        tool.call_method1("_refresh", (&viewport,))?;
        return true.into_bound_py_any(py);
    }
    tool.call_method1("plane_lock_key", (&viewport, &key))
}
fn on_key_release<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let qt = imported(py, "built_core")?.getattr("Input")?;
    if key.ne(qt.getattr("Key_Control")?)? {
        return false.into_bound_py_any(py);
    }
    let tapped = optional(&viewport, "ctrl_tapped")?;
    let reply = tool.call_method1("_send_framed", (&viewport, fields!(py; "type" => "key_release", "key" => "Control", "tapped" => { let value = (!(tapped.is_callable())).into_bound_py_any(py)?; if value.is_truthy()? {value} else {(tapped.call0()?.is_truthy()?).into_bound_py_any(py)?} }).into_any(),))?;
    if reply.get_item("consumed")?.is_truthy()? {
        tool.call_method1("_refresh", (&viewport,))?;
    }
    reply.get_item("consumed")
}
fn status_clause<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if !tool.getattr("_guide_capable")?.is_truthy()? {
        return ("").into_bound_py_any(py);
    }
    let tr = imported(py, "built_core.i18n")?.getattr("tr")?;
    tr.call1(("Ctrl = guide",))
}
fn guide_preview_lines<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let mut first: Obj<'py>;
    let mut b: Obj<'py>;
    let tool = a.get_item(0)?;
    return Ok({
        let items = PyList::empty(py);
        for item in ({
            let value = tool.call_method1("_state", ("guide_ring",))?;
            if value.is_truthy()? {
                value
            } else {
                PyTuple::empty(py).into_any()
            }
        })
        .try_iter()?
        {
            [first, b] = unpack::<2>(&item?)?;
            items.append(
                (
                    module.call_method1("Vec3", positional(&first)?)?,
                    module.call_method1("Vec3", positional(&b)?)?,
                )
                    .into_bound_py_any(py)?,
            )?;
        }
        items.into_any()
    });
}
fn value_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let r = tool.call_method1("_state", ("radius",))?;
    if r.is_none() {
        return Ok(none(py));
    }
    let tr = imported(py, "built_core.i18n")?.getattr("tr")?;
    let fmt_len = imported(py, "core.units")?.getattr("fmt_len")?;
    Ok((
        ("R ")
            .into_bound_py_any(py)?
            .add(fmt_len.call1((&r,))?)?
            .add("  (")?
            .add(tr.call(
                ("{n} sides",),
                Some(&fields!(py; "n" => tool.getattr("sides")?)),
            )?)?
            .add(")")?,
        tool.getattr("hover_point")?,
    )
        .into_bound_py_any(py)?)
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
    let module = imported(py, "tools.rust_tools")?;
    let tool = a.get_item(0)?;
    let u: Obj<'py>;
    let v: Obj<'py>;
    [u, v] = unpack::<2>(&tool.call_method1("_query", ("axes", py.None()))?)?;
    (
        module.call_method1("Vec3", positional(&u)?)?,
        module.call_method1("Vec3", positional(&v)?)?,
    )
        .into_bound_py_any(py)
}
fn points<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let center = a.get_item(1)?;
    let rim = a.get_item(2)?;
    let tool = a.get_item(0)?;
    return Ok({
        let items = PyList::empty(py);
        for item in tool
            .call_method1(
                "_query",
                (
                    "points",
                    (
                        module.call_method1("_tuple", (&center,))?,
                        module.call_method1("_tuple", (&rim,))?,
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
fn commit<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let pts = a.get_item(2)?;
    let viewport = a.get_item(1)?;
    module.call_method1("_Host", (&viewport,))?.call_method1("run", ("add_face", module.getattr("json")?.call_method1("dumps", (fields!(py; "outer" => {let items=PyList::empty(py); for item in pts.try_iter()? { let p = item?;
items.append(py.get_type::<pyo3::types::PyList>().call1((module.call_method1("_tuple", (&p,))?,))?)?;
} items.into_any()}, "curve" => true).into_any(),))?,))?;
    viewport.call_method0("update")?;
    Ok(none(py))
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
    ("sides", Mode::Property, sides, &["self"], "", ""),
    ("_guide_mode", Mode::Property, guide_mode, &["self"], "", ""),
    ("_last", Mode::Property, last, &["self"], "", "The retype token of the shape just drawn, or None."),
    ("vcb_caption", Mode::Method, vcb_caption, &["self"], "str", "'Sides' before the centre, 'Radius' after — and right after a\nshape is drawn, while a typed number still resizes it (#414)."),
    ("_radius_retype_open", Mode::Method, radius_retype_open, &["self"], "bool", ""),
    ("value_is_unitless", Mode::Method, value_is_unitless, &["self"], "bool", "A side count before the centre (#176); a radius after it, and\nright after a shape (#414)."),
    ("on_activate", Mode::Method, on_activate, &["self", "viewport"], "None", ""),
    ("on_deactivate", Mode::Method, on_deactivate, &["self", "viewport"], "None", ""),
    ("on_cancel", Mode::Method, on_cancel, &["self", "viewport"], "None", ""),
    ("on_click", Mode::Method, on_click, &["self", "ctx"], "None", ""),
    ("on_hover", Mode::Method, on_hover, &["self", "ctx"], "None", ""),
    ("on_segments_value", Mode::Method, on_segments_value, &["self", "viewport", "n:int"], "bool", "«24s»: the side count, typed at any moment; right after a shape,\nthat shape is rebuilt with it, one undo step (#305)."),
    ("on_value", Mode::Method, on_value, &["self", "viewport", "value"], "bool", ""),
    ("_refresh", Mode::Method, refresh, &["self", "viewport"], "", ""),
    ("on_key", Mode::Method, on_key, &["self", "viewport", "key", "modifiers"], "bool", ""),
    ("on_key_release", Mode::Method, on_key_release, &["self", "viewport", "key"], "bool", ""),
    ("status_clause", Mode::Method, status_clause, &["self"], "str", ""),
    ("guide_preview_lines", Mode::Method, guide_preview_lines, &["self"], "", "The dashed rim of the guide circle the next click leaves."),
    ("value_label", Mode::Method, value_label, &["self"], "", "The radius DRAWN, the rim projected onto the plane, not the\nstraight distance to the cursor."),
    ("plane_points", Mode::Method, plane_points, &["self"], "", ""),
    ("_query", Mode::Method, query, &["self", "what", "arg"], "", ""),
    ("_axes", Mode::Method, axes, &["self"], "", ""),
    ("_points", Mode::Method, points, &["self", "center", "rim"], "", ""),
    ("_commit", Mode::Method, commit, &["self", "viewport", "pts", "axes=None", "center=None"], "None", "A closed curve with its face through the protocol (``add_face``\nwith ``curve``), outside the machine's retype window."),
    ("_reset", Mode::Method, reset, &["self"], "", ""),
    ("sides", Mode::Setter, set_sides, &["self", "n"], "", ""),
    ("_guide_mode", Mode::Setter, set_guide_mode, &["self", "on"], "", ""),
];
