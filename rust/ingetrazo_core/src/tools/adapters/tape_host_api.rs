// SPDX-License-Identifier: GPL-3.0-or-later
//! TapeHost callbacks on the live document.
use super::*;

fn guides<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    (tool.getattr("_mode")?.ne("measure")?).into_bound_py_any(py)
}
fn cursor_plus<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("_guides")
}
fn start_from_hover<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.tape_host")?;
    let mut pixel = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let q_point_f = imported(py, "ingetrazo_core")?.getattr("ScreenPoint")?;
    let input_state = imported(py, "views.input_state")?.into_any();
    let _hover_event = imported(py, "views.document_interaction")?.getattr("_HoverEvent")?;
    if !tool.getattr("start_point")?.is_none() || tool.getattr("_mode")?.ne("line")? {
        return false.into_bound_py_any(py);
    }
    if pixel.is_none() {
        pixel = viewport.call_method1(
            "mapFromGlobal",
            (input_state.call_method1("cursor_position", (&viewport,))?,),
        )?;
    }
    if !(viewport
        .call_method0("rect")?
        .call_method1("contains", (&pixel,))?
        .is_truthy()?)
    {
        return false.into_bound_py_any(py);
    }
    let ev = _hover_event.call1((
        q_point_f.call1((&pixel,))?,
        module.getattr("Qt")?.getattr("NoModifier")?,
    ))?;
    let ctx = viewport.call_method1("_build_ctx", (&ev,))?;
    if ctx.is_none()
        || (!ctx.getattr("snap")?.is_none()
            && (ctx.getattr("snap")?.getattr("kind")?.eq("close")?
                || tool
                    .getattr("_machine")?
                    .call_method1("query", ("named", ctx.getattr("snap")?.getattr("kind")?))?
                    .is_truthy()?))
    {
        return false.into_bound_py_any(py);
    }
    let edge = viewport.call_method1(
        "pick_edge_any",
        (pixel.call_method0("x")?, pixel.call_method0("y")?),
    )?;
    let guide = if edge.is_none() {
        viewport.call_method1(
            "pick_guide",
            (pixel.call_method0("x")?, pixel.call_method0("y")?),
        )?
    } else {
        none(py)
    };
    let axis = if edge.is_none()
        && !(guide
            .getattr_opt("is_line")?
            .unwrap_or(false.into_bound_py_any(py)?)
            .is_truthy()?)
    {
        viewport.call_method1(
            "pick_axis",
            (pixel.call_method0("x")?, pixel.call_method0("y")?),
        )?
    } else {
        none(py)
    };
    if edge.is_none()
        && !(guide
            .getattr_opt("is_line")?
            .unwrap_or(false.into_bound_py_any(py)?)
            .is_truthy()?)
        && axis.is_none()
    {
        return false.into_bound_py_any(py);
    }
    viewport.call_method1("_dispatch_tool_click", (&ev,))?;
    if tool.getattr("_edge")?.is_none() {
        tool.call_method1(
            "_send",
            (&viewport, fields!(py; "type" => "cancel").into_any()),
        )?;
        return false.into_bound_py_any(py);
    }
    tool.setattr(
        "hover_point",
        module.call_method1("QVector3D", (tool.getattr("start_point")?,))?,
    )?;
    viewport
        .getattr("measurementChanged")?
        .call_method1("emit", (viewport.call_method0("_measurement_text")?,))?;
    viewport.call_method0("update")?;
    true.into_bound_py_any(py)
}
fn host_clear_source<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    {
        let value = none(py);
        tool.setattr("_source", &value)?;
        tool.setattr("work_plane", &value)?;
    }
    Ok(none(py))
}
fn on_value<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let value = a.get_item(2)?;
    let vp = a.get_item(1)?;
    let result = tool.call_method1(
        "_send",
        (
            &vp,
            fields!(py; "type" => "value", "value" => &value).into_any(),
        ),
    )?;
    if result.get_item("redraw")?.is_truthy()? {
        vp.call_method0("update")?;
    }
    result.get_item("consumed")
}
fn toggle_mode<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    Ok(tool
        .call_method1(
            "_send",
            (
                &vp,
                fields!(py; "type" => "key_release", "key" => "Control", "tapped" => true)
                    .into_any(),
            ),
        )?
        .get_item("consumed")?)
}
fn host_tape_mode<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.tape_host")?;
    let mut callback: Obj<'py>;
    let mode = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    tool.getattr("_during")?.set_item("mode", mode.clone())?;
    vp.call_method1(
        "flash_status",
        (module.call_method1(
            "tr",
            (py.get_type::<pyo3::types::PyDict>()
                .call1((tool.getattr("_MODES")?,))?
                .get_item(&mode)?,),
        )?,),
    )?;
    for item in ("_apply_tool_cursor", "refresh_status_hint")
        .into_bound_py_any(py)?
        .try_iter()?
    {
        let name = item?;
        callback = imported(py, "builtins")?
            .getattr("getattr")?
            .call1((&vp, &name, py.None()))?;
        if callback.is_truthy()? {
            callback.call0()?;
        }
    }
    vp.call_method0("update")?;
    Ok(none(py))
}
fn on_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.tape_host")?;
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    Ok({
        let value =
            (key.eq(module.getattr("Qt")?.getattr("Key_Control")?)?).into_bound_py_any(py)?;
        if !value.is_truthy()? {
            value
        } else {
            tool.call_method1(
                "_send",
                (
                    &vp,
                    fields!(py; "type" => "key", "key" => "Control").into_any(),
                ),
            )?
            .get_item("consumed")?
        }
    })
}
fn host_tape_message<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.tape_host")?;
    let arguments = a.get_item(2)?;
    let distance: Obj<'py>;
    let label: Obj<'py>;
    let vp = a.get_item(1)?;
    [label, distance] = unpack::<2>(&arguments)?;
    let captions = fields!(py; "line" => "Guide at {d}", "point" => "Guide point at {d}", "segment" => "Guide point at {d}, with its segment", "measure" => "Distance: {d}").into_any();
    vp.call_method1(
        "flash_status",
        (
            module
                .call_method1("tr", (captions.get_item(&label)?,))?
                .call_method(
                    "format",
                    (),
                    Some(&fields!(py; "d" => module.call_method1("fmt_len_fine", (&distance,))?)),
                )?,
            (if label.eq("measure")? {
                4000.into_bound_py_any(py)?
            } else {
                3000.into_bound_py_any(py)?
            }),
        ),
    )?;
    Ok(none(py))
}
fn value_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.tape_host")?;
    let tool = a.get_item(0)?;
    let reading = tool
        .getattr("_machine")?
        .call_method1("query", ("label", py.None()))?;
    Ok(if reading.is_truthy()? {
        (
            module.call_method1("fmt_len", (reading.get_item(0)?,))?,
            module.call_method1("QVector3D", positional(&reading.get_item(1)?)?)?,
        )
            .into_bound_py_any(py)?
    } else {
        none(py)
    })
}

fn status_clause<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.tape_host")?;
    let mut k: Obj<'py>;
    let tool = a.get_item(0)?;
    let mut v: Obj<'py>;
    let names = fields!(py; "line" => module.call_method1("tr", ("guide lines",))?, "point" => module.call_method1("tr", ("guide points",))?, "measure" => module.call_method1("tr", ("measure",))?).into_any();
    let parts = {
        let items = PyList::empty(py);
        for item in names.call_method0("items")?.try_iter()? {
            {
                let [part0, part1] = unpack::<2>(&item?)?;
                k = part0;
                v = part1;
            }
            items.append(if k.eq(tool.getattr("_mode")?)? {
                [
                    "[".to_owned(),
                    v.call_method1("__format__", ("",))?.extract::<String>()?,
                    "]".to_owned(),
                ]
                .concat()
                .into_bound_py_any(py)?
            } else {
                v.clone()
            })?;
        }
        items.into_any()
    };
    ("Ctrl = ").into_bound_py_any(py)?.add(
        (" / ")
            .into_bound_py_any(py)?
            .call_method1("join", (&parts,))?,
    )
}
fn on_click<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.tape_host")?;
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    {
        let [part0, part1] =
            unpack::<2>(&(ctx.getattr("snap")?, ctx.getattr("world")?).into_bound_py_any(py)?)?;
        tool.setattr("_snap", part0)?;
        tool.setattr("_click_point", part1)?;
    }
    let action = (|| -> Reply<'py> {
        imported(py, "builtins")?
            .getattr("super")?
            .call1((module.getattr("TapeHost")?, &tool))?
            .call_method1("on_click", (&ctx,))?;
        Ok(none(py))
    })();
    {
        let value = none(py);
        tool.setattr("_snap", &value)?;
        tool.setattr("_click_point", &value)?;
    }
    action?;
    Ok(none(py))
}
fn on_key_release<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.tape_host")?;
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    Ok({
        let value =
            (key.eq(module.getattr("Qt")?.getattr("Key_Control")?)?).into_bound_py_any(py)?;
        if !value.is_truthy()? {
            value
        } else {
            tool.call_method1("_send", (&vp, fields!(py; "type" => "key_release", "key" => "Control", "tapped" => if let Some(call)=vp.getattr_opt("ctrl_tapped")? {call.call0()?} else {true.into_bound_py_any(py)?}).into_any(),))?.get_item("consumed")?
        }
    })
}

fn guide_preview_lines<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let reading = a
        .get_item(0)?
        .getattr("_machine")?
        .call_method1("query", ("guide", py.None()))?;
    let result = PyList::empty(py);
    if reading.is_truthy()? {
        let module = imported(py, "tools.tape_host")?;
        let vec = module.getattr("QVector3D")?;
        let points = reading
            .try_iter()?
            .map(|p| vec.call1(positional(&p?)?))
            .collect::<PyResult<Vec<_>>>()?;
        result.append(
            module
                .call_method1("Guide", PyTuple::new(py, points)?)?
                .call_method0("segment")?,
        )?;
    }
    Ok(result.into_any())
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("status_clause", Mode::Method, status_clause, &["self"], "str", "The usual wording, kept on screen: «Ctrl = Líneas guía del\nciclo/Puntos guía/Medida». The active one is bracketed, because the\ncursor's + says \"this leaves something\" but not WHICH of the two\nguide modes is on."),
    ("_guides", Mode::Property, guides, &["self"], "bool", "Whether this mode leaves anything behind."),
    ("cursor_plus", Mode::Property, cursor_plus, &["self"], "bool", "The little + beside the cursor: this one will leave a\nguide — a line or a point."),
    ("start_from_hover", Mode::Method, start_from_hover, &["self", "viewport", "pixel=None"], "bool", "Picking T over a line starts its guide without a first click."),
    ("host_clear_source", Mode::Method, host_clear_source, &["self", "vp", "_"], "", ""),
    ("on_click", Mode::Method, on_click, &["self", "ctx"], "", ""),
    ("on_value", Mode::Method, on_value, &["self", "vp", "value"], "", ""),
    ("_toggle_mode", Mode::Method, toggle_mode, &["self", "vp"], "", ""),
    ("host_tape_mode", Mode::Method, host_tape_mode, &["self", "vp", "mode"], "", ""),
    ("on_key", Mode::Method, on_key, &["self", "vp", "key", "modifiers"], "", ""),
    ("on_key_release", Mode::Method, on_key_release, &["self", "vp", "key"], "", ""),
    ("host_tape_message", Mode::Method, host_tape_message, &["self", "vp", "args"], "", ""),
    ("guide_preview_lines", Mode::Method, guide_preview_lines, &["self"], "", ""),
    ("value_label", Mode::Method, value_label, &["self"], "", ""),
];
