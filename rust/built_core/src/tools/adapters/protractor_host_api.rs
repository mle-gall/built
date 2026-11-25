// SPDX-License-Identifier: GPL-3.0-or-later
//! ProtractorHost callbacks on the live document.
use super::*;

fn cursor_plus<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("_guides")
}
fn on_activate<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1(
        "_send",
        (&viewport, fields!(py; "type" => "activate").into_any()),
    )?;
    {
        let value = none(py);
        tool.setattr("_axis_pick", &value)?;
        tool.setattr("_held_guide", &value)?;
    }
    Ok(none(py))
}
fn on_deactivate<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1(
        "_send",
        (&viewport, fields!(py; "type" => "deactivate").into_any()),
    )?;
    {
        let value = none(py);
        tool.setattr("_axis_pick", &value)?;
        tool.setattr("_held_guide", &value)?;
        tool.setattr("hover_point", &value)?;
    }
    Ok(none(py))
}
fn on_cancel<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1(
        "_send",
        (&viewport, fields!(py; "type" => "cancel").into_any()),
    )?;
    {
        let value = none(py);
        tool.setattr("_axis_pick", &value)?;
        tool.setattr("_held_guide", &value)?;
    }
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn send_basis<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_send", (&viewport, fields!(py; "type" => "protractor_frame", "basis" => tool.call_method0("_basis")?, "near" => tool.getattr("_snap_ticks")?, "exact" => tool.getattr("_exact_snap")?).into_any(),))?;
    Ok(none(py))
}
fn on_click<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    if !tool.getattr("ref_point")?.is_none() {
        tool.call_method1("_note_snap", (&ctx,))?;
        tool.call_method1("_send_basis", (ctx.getattr("viewport")?,))?;
    }
    tool.setattr("_held_guide", none(py))?;
    tool.call_method1(
        "_send",
        (
            ctx.getattr("viewport")?,
            tool.call_method1("_pointer", ("click", &ctx))?,
        ),
    )?;
    ctx.getattr("viewport")?.call_method0("update")?;
    Ok(none(py))
}
fn on_hover<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.setattr("hover_point", ctx.getattr("world")?)?;
    tool.call_method1("_track_axis_drag", (ctx.getattr("viewport")?,))?;
    tool.call_method1("_infer_plane", (&ctx,))?;
    tool.call_method1("_update_screen_metrics", (&ctx,))?;
    tool.call_method1("_note_snap", (&ctx,))?;
    tool.call_method1(
        "_send",
        (
            ctx.getattr("viewport")?,
            tool.call_method1("_pointer", ("hover", &ctx))?,
        ),
    )?;
    ctx.getattr("viewport")?.call_method0("update")?;
    Ok(none(py))
}
fn on_release<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if tool
        .call_method1("_release_axis_drag", (&viewport,))?
        .is_truthy()?
    {
        let tr = imported(py, "built_core.i18n")?.getattr("tr")?;
        viewport.call_method1(
            "flash_status",
            (tr.call1(("Protractor axis set along the drag",))?,),
        )?;
    }
    Ok(none(py))
}
fn on_value<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let value = a.get_item(2)?;
    let viewport = a.get_item(1)?;
    if !(imported(py, "builtins")?
        .getattr("isinstance")?
        .call1((
            &value,
            (
                py.get_type::<pyo3::types::PyInt>().into_any(),
                py.get_type::<pyo3::types::PyFloat>().into_any(),
            )
                .into_bound_py_any(py)?,
        ))?
        .is_truthy()?)
    {
        return false.into_bound_py_any(py);
    }
    tool.call_method1("_send_basis", (&viewport,))?;
    tool.getattr("_machine")?
        .call_method1("set", ("hover_point", tool.getattr("hover_point")?))?;
    let result = tool.call_method1(
        "_send",
        (
            &viewport,
            fields!(py; "type" => "value", "value" => &value).into_any(),
        ),
    )?;
    if result.get_item("redraw")?.is_truthy()? {
        viewport.call_method0("update")?;
    }
    result.get_item("consumed")
}
fn toggle_guides<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let say = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_send", (&viewport, fields!(py; "type" => "key", "key" => if say.is_truthy()? { ("ToggleGuides").into_bound_py_any(py)? } else { ("ToggleGuidesQuiet").into_bound_py_any(py)? }).into_any(),))?;
    Ok(none(py))
}
fn host_reset_disc<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    tool.call_method0("_reset_protractor")?;
    Ok(none(py))
}
fn host_guide_toggle<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let arguments = a.get_item(2)?;
    let mut callback: Obj<'py>;
    let guides: Obj<'py>;
    let say: Obj<'py>;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let tr = imported(py, "built_core.i18n")?.getattr("tr")?;
    [guides, say] = unpack::<2>(&arguments)?;
    tool.getattr("_during")?
        .set_item("guides", guides.clone())?;
    if say.is_truthy()? {
        viewport.call_method1(
            "flash_status",
            ((if guides.is_truthy()? {
                tr.call1(("Create guides: on",))?
            } else {
                tr.call1(("Create guides: off — measure only",))?
            }),),
        )?;
    }
    for item in ("_apply_tool_cursor", "refresh_status_hint")
        .into_bound_py_any(py)?
        .try_iter()?
    {
        let name = item?;
        callback =
            imported(py, "builtins")?
                .getattr("getattr")?
                .call1((&viewport, &name, py.None()))?;
        if callback.is_truthy()? {
            callback.call0()?;
        }
    }
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn host_disc_centre<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.protractor_host")?;
    let point = a.get_item(2)?;
    let tool = a.get_item(0)?;
    tool.setattr(
        "_centre_plane",
        (
            module.call_method1("QVector3D", positional(&point)?)?,
            module.call_method1("QVector3D", (tool.call_method0("_axis")?,))?,
        )
            .into_bound_py_any(py)?,
    )?;
    tool.setattr("_axis_drag_armed", true)?;
    Ok(none(py))
}
fn host_remember_guide<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.setattr(
        "_held_guide",
        viewport
            .getattr("scene")?
            .getattr("guides")?
            .get_item(1.into_bound_py_any(py)?.neg()?)?,
    )?;
    Ok(none(py))
}
fn host_measured_angle<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let degrees = a.get_item(2)?;
    let viewport = a.get_item(1)?;
    let tr = imported(py, "built_core.i18n")?.getattr("tr")?;
    let fmt_angle_deg = imported(py, "core.units")?.getattr("fmt_angle_deg")?;
    viewport.call_method1("flash_status", (tr.call1(("Angle: {a}",))?.call_method("format", (), Some(&fields!(py; "a" => fmt_angle_deg.call((&degrees,), Some(&fields!(py; "signed" => true)))?)))?, 4000,))?;
    Ok(none(py))
}
fn rubber_band_lines<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method0("_disc_lines")
}
fn guide_preview_lines<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.protractor_host")?;
    let tool = a.get_item(0)?;
    if !(tool.getattr("_guides")?.is_truthy()?)
        || tool.getattr("start_point")?.is_none()
        || tool.getattr("ref_point")?.is_none()
        || tool.getattr("hover_point")?.is_none()
    {
        return Ok(PyList::empty(py).into_any());
    }
    let deg = tool.call_method1("_commit_deg", (tool.getattr("hover_point")?,))?;
    if deg.is_none() {
        return Ok(PyList::empty(py).into_any());
    }
    Ok(PyList::new(
        py,
        [module
            .call_method1(
                "Guide",
                (
                    tool.getattr("start_point")?,
                    tool.call_method1("_direction_at", (&deg,))?,
                ),
            )?
            .call_method0("segment")?],
    )?
    .into_any())
}
fn value_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if tool.getattr("ref_point")?.is_none() || tool.getattr("hover_point")?.is_none() {
        return Ok(none(py));
    }
    let deg = tool.call_method1("_display_deg", (tool.getattr("hover_point")?,))?;
    if deg.is_none() {
        return Ok(none(py));
    }
    let fmt_angle_deg = imported(py, "core.units")?.getattr("fmt_angle_deg")?;
    Ok((
        fmt_angle_deg.call((&deg,), Some(&fields!(py; "signed" => true)))?,
        tool.getattr("hover_point")?,
    )
        .into_bound_py_any(py)?)
}
fn vcb_caption<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    ("Angle").into_bound_py_any(py)
}

fn status_clause<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let guia: Obj<'py>;
    let medir: Obj<'py>;
    let tool = a.get_item(0)?;
    let _tr = imported(py, "built_core.i18n")?.getattr("tr")?;
    {
        let [part0, part1] = unpack::<2>(
            &(_tr.call1(("guide",))?, _tr.call1(("measure",))?).into_bound_py_any(py)?,
        )?;
        guia = part0;
        medir = part1;
    }
    ("Ctrl = ")
        .into_bound_py_any(py)?
        .add(if tool.getattr("_guides")?.is_truthy()? {
            [
                "[".to_owned(),
                guia.call_method1("__format__", ("",))?
                    .extract::<String>()?,
                "] / ".to_owned(),
                medir
                    .call_method1("__format__", ("",))?
                    .extract::<String>()?,
            ]
            .concat()
            .into_bound_py_any(py)?
        } else {
            [
                guia.call_method1("__format__", ("",))?
                    .extract::<String>()?,
                " / [".to_owned(),
                medir
                    .call_method1("__format__", ("",))?
                    .extract::<String>()?,
                "]".to_owned(),
            ]
            .concat()
            .into_bound_py_any(py)?
        })
}
fn on_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.protractor_host")?;
    let key = a.get_item(2)?;
    let modifiers = a.get_item(3)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if key.eq(module.getattr("Qt")?.getattr("Key_Control")?)? {
        return Ok(tool
            .call_method1(
                "_send",
                (
                    &viewport,
                    fields!(py; "type" => "key", "key" => "Control").into_any(),
                ),
            )?
            .get_item("consumed")?);
    }
    imported(py, "builtins")?
        .getattr("super")?
        .call1((module.getattr("ProtractorHost")?, &tool))?
        .call_method1("on_key", (&viewport, &key, &modifiers))
}
fn on_key_release<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.protractor_host")?;
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if key.ne(module.getattr("Qt")?.getattr("Key_Control")?)? {
        return false.into_bound_py_any(py);
    }
    Ok(tool.call_method1("_send", (&viewport, fields!(py; "type" => "key_release", "key" => "Control", "tapped" => if let Some(call)=viewport.getattr_opt("ctrl_tapped")? {call.call0()?} else {true.into_bound_py_any(py)?}).into_any(),))?.get_item("consumed")?)
}

fn host_live_guide_index<'py>(a: &Args<'py>) -> Reply<'py> {
    let held = a.get_item(0)?.getattr("_held_guide")?;
    for (i, guide) in a
        .get_item(1)?
        .getattr("scene")?
        .getattr("guides")?
        .try_iter()?
        .enumerate()
    {
        if guide?.is(&held) {
            return i.into_bound_py_any(a.py());
        }
    }
    Ok(none(a.py()))
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("cursor_plus", Mode::Property, cursor_plus, &["self"], "bool", ""),
    ("status_clause", Mode::Method, status_clause, &["self"], "str", "Kept on screen while the tool is active. Two\nmodes here: the Protractor cannot drop a guide POINT."),
    ("on_activate", Mode::Method, on_activate, &["self", "viewport"], "", ""),
    ("on_deactivate", Mode::Method, on_deactivate, &["self", "viewport"], "", ""),
    ("on_cancel", Mode::Method, on_cancel, &["self", "viewport"], "", ""),
    ("_send_basis", Mode::Method, send_basis, &["self", "viewport"], "", ""),
    ("on_click", Mode::Method, on_click, &["self", "ctx"], "", ""),
    ("on_hover", Mode::Method, on_hover, &["self", "ctx"], "", ""),
    ("on_release", Mode::Method, on_release, &["self", "viewport"], "None", ""),
    ("on_value", Mode::Method, on_value, &["self", "viewport", "value"], "", ""),
    ("_toggle_guides", Mode::Method, toggle_guides, &["self", "viewport", "say=True"], "", ""),
    ("on_key", Mode::Method, on_key, &["self", "viewport", "key", "modifiers"], "", ""),
    ("on_key_release", Mode::Method, on_key_release, &["self", "viewport", "key"], "", ""),
    ("host_reset_disc", Mode::Method, host_reset_disc, &["self", "viewport", "_"], "", ""),
    ("host_guide_toggle", Mode::Method, host_guide_toggle, &["self", "viewport", "args"], "", ""),
    ("host_disc_centre", Mode::Method, host_disc_centre, &["self", "viewport", "point"], "", ""),
    ("host_remember_guide", Mode::Method, host_remember_guide, &["self", "viewport", "_"], "", ""),
    ("host_live_guide_index", Mode::Method, host_live_guide_index, &["self", "viewport", "_"], "", ""),
    ("host_measured_angle", Mode::Method, host_measured_angle, &["self", "viewport", "degrees"], "", ""),
    ("rubber_band_lines", Mode::Method, rubber_band_lines, &["self"], "", ""),
    ("guide_preview_lines", Mode::Method, guide_preview_lines, &["self"], "", "The angled guide the next click leaves, through the vertex."),
    ("value_label", Mode::Method, value_label, &["self"], "", ""),
    ("vcb_caption", Mode::Method, vcb_caption, &["self"], "str", ""),
];
