// SPDX-License-Identifier: GPL-3.0-or-later
//! MoveHost callbacks on the live document.
use super::*;

fn clear_sets<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    tool.getattr("_move_gesture")?.call_method1(
        "finish",
        (
            &tool,
            (!tool.call_method1("_state", ("window",))?.is_none()).into_bound_py_any(py)?,
        ),
    )?;
    Ok(none(py))
}
fn copy<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    (tool.call_method1("_state", ("copy",))?.is_truthy()?).into_bound_py_any(py)
}
fn set_copy<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let on = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.getattr("_machine")?
        .call_method1("set", ("copy", (on.is_truthy()?).into_bound_py_any(py)?))?;
    Ok(none(py))
}
fn cursor_plus<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("_copy")
}
fn after<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if tool.getattr("grab")?.is_none() {
        tool.call_method0("_clear_sets")?;
    }
    let update = optional(&viewport, "update")?;
    if update.is_callable() {
        update.call0()?;
    }
    Ok(none(py))
}
fn on_activate<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.getattr("_turn")?.call_method0("clear")?;
    tool.call_method0("_clear_grips")?;
    tool.call_method1(
        "_send",
        (&viewport, fields!(py; "type" => "activate").into_any()),
    )?;
    tool.call_method1("_after", (&viewport,))?;
    Ok(none(py))
}
fn on_deactivate<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.getattr("_turn")?
        .call_method1("cancel", (&viewport,))?;
    tool.call_method0("_clear_grips")?;
    tool.call_method1(
        "_send",
        (&viewport, fields!(py; "type" => "deactivate").into_any()),
    )?;
    tool.getattr("_handles")?.call_method0("clear")?;
    tool.call_method1("_after", (&viewport,))?;
    Ok(none(py))
}
fn on_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.move_host")?;
    let key = a.get_item(2)?;
    let modifiers = a.get_item(3)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if !tool.getattr("_grip_rot")?.is_none() {
        return tool
            .getattr("_grip_rot")?
            .call_method1("on_key", (&viewport, &key, &modifiers));
    }
    if key.eq(module.getattr("Qt")?.getattr("Key_Control")?)? {
        tool.call_method1("_send", (&viewport, fields!(py; "type" => "key", "key" => "Control", "mods" => module.call_method1("_mods", (&modifiers,))?).into_any(),))?;
        tool.call_method1("_copy_changed", (&viewport,))?;
        return true.into_bound_py_any(py);
    }
    false.into_bound_py_any(py)
}
fn copy_changed<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let viewport = a.get_item(1)?;
    let apply = optional(&viewport, "_apply_tool_cursor")?;
    if !apply.is_none() {
        apply.call0()?;
    }
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn toggle_copy<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let say = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_send", (&viewport, fields!(py; "type" => "key", "key" => if say.is_truthy()? { ("ToggleCopy").into_bound_py_any(py)? } else { ("ToggleCopyQuiet").into_bound_py_any(py)? }).into_any(),))?;
    tool.call_method1("_copy_changed", (&viewport,))?;
    Ok(none(py))
}
fn on_key_release<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.move_host")?;
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if !tool.getattr("_grip_rot")?.is_none() {
        return tool
            .getattr("_grip_rot")?
            .call_method1("on_key_release", (&viewport, &key));
    }
    if key.ne(module.getattr("Qt")?.getattr("Key_Control")?)? {
        return false.into_bound_py_any(py);
    }
    let tapped = optional(&viewport, "ctrl_tapped")?;
    let reply = tool.call_method1("_send", (&viewport, fields!(py; "type" => "key_release", "key" => "Control", "tapped" => { let value = (!(tapped.is_callable())).into_bound_py_any(py)?; if value.is_truthy()? {value} else {(tapped.call0()?.is_truthy()?).into_bound_py_any(py)?} }).into_any(),))?;
    if reply.get_item("consumed")?.is_truthy()? {
        tool.call_method1("_copy_changed", (&viewport,))?;
    }
    reply.get_item("consumed")
}
fn on_hover<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    if !tool.getattr("_grip_rot")?.is_none() {
        tool.getattr("_grip_rot")?
            .call_method1("on_hover", (&ctx,))?;
        return Ok(none(py));
    }
    if tool.getattr("start_point")?.is_none() {
        tool.call_method1("_track_grips", (&ctx,))?;
    }
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
fn value_is_unitless<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok({
        let value = tool.getattr("_turn")?.getattr("takes_angle")?;
        if !value.is_truthy()? {
            value
        } else {
            (tool.getattr("start_point")?.is_none()).into_bound_py_any(py)?
        }
    })
}
fn on_value<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let value = a.get_item(2)?;
    let viewport = a.get_item(1)?;
    let turn = tool
        .getattr("_turn")?
        .call_method1("value", (&tool, &viewport, &value))?;
    if !turn.is_none() {
        return Ok(turn.clone());
    }
    let consumed = tool
        .call_method1(
            "_send",
            (
                &viewport,
                fields!(py; "type" => "value", "value" => &value).into_any(),
            ),
        )?
        .get_item("consumed")?;
    tool.call_method1("_after", (&viewport,))?;
    Ok(consumed.clone())
}
fn array_pending<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let window = tool.call_method1("_state", ("window",))?;
    if window.is_none() || !tool.getattr("start_point")?.is_none() {
        return false.into_bound_py_any(py);
    }
    let _host = imported(py, "tools.rust_tool_base")?.getattr("_Host")?;
    (window
        .get_item("token")?
        .eq(_host.call1((&viewport,))?.call_method0("revision")?)?)
    .into_bound_py_any(py)
}
fn last<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("window",))
}
fn close_array_window<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    tool.call_method1(
        "_send",
        (
            py.None(),
            fields!(py; "type" => "key", "key" => "CloseArray").into_any(),
        ),
    )?;
    tool.getattr("_move_gesture")?.call_method0("close_array")?;
    Ok(none(py))
}
fn on_array_value<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let count = a.get_item(2)?;
    let mode = a.get_item(3)?;
    let tool = a.get_item(0)?;
    let step = a.get_item(4)?;
    let viewport = a.get_item(1)?;
    let done = tool.call_method1("_send", (&viewport, fields!(py; "type" => "array", "count" => py.get_type::<pyo3::types::PyInt>().call1((&count,))?, "mode" => &mode, "step" => if step.is_none() { none(py) } else { py.get_type::<pyo3::types::PyFloat>().call1((&step,))? }).into_any(),))?.get_item("consumed")?;
    tool.call_method1("_after", (&viewport,))?;
    Ok(done.clone())
}
fn on_cancel<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if tool
        .getattr("_turn")?
        .call_method1("cancel", (&viewport,))?
        .is_truthy()?
    {
        tool.call_method0("_clear_grips")?;
        viewport.call_method0("update")?;
        return Ok(none(py));
    }
    tool.call_method1(
        "_send",
        (&viewport, fields!(py; "type" => "cancel").into_any()),
    )?;
    tool.call_method1("_after", (&viewport,))?;
    Ok(none(py))
}
fn rubber_band_lines<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    if !tool.getattr("_grip_rot")?.is_none() {
        return tool.getattr("_grip_rot")?.call_method0("rubber_band_lines");
    }
    tool.getattr("_base_segments")?.call_method1(
        "move_lines",
        (
            tool.getattr("grab")?,
            tool.getattr("hover_point")?,
            tool.getattr("_copy")?,
        ),
    )
}
fn value_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.move_host")?;
    let grab: Obj<'py>;
    let hover: Obj<'py>;
    let tool = a.get_item(0)?;
    if !tool.getattr("_grip_rot")?.is_none() {
        return tool.getattr("_grip_rot")?.call_method0("value_label");
    }
    [grab, hover] =
        unpack::<2>(&(tool.getattr("grab")?, tool.getattr("hover_point")?).into_bound_py_any(py)?)?;
    if grab.is_none() || hover.is_none() {
        return Ok(none(py));
    }
    let delta = hover.sub(&grab)?;
    let mid = grab.add(&hover)?.mul(0.5)?;
    (
        module.call_method1("fmt_len", (delta.call_method0("length")?,))?,
        &mid,
    )
        .into_bound_py_any(py)
}
fn snap_excluded<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.move_host")?;
    let tool = a.get_item(0)?;
    if !tool.getattr("_grip_rot")?.is_none() {
        return tool.getattr("_grip_rot")?.call_method0("snap_excluded");
    }
    if tool.getattr("grab")?.is_none() || tool.getattr("_copy")?.is_truthy()? {
        return Ok(none(py));
    }
    module.call_method1("snap_exclusions", (&tool, false))
}
fn moving_face_ids<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.move_host")?;
    let tool = a.get_item(0)?;
    if tool.getattr("grab")?.is_none() || tool.getattr("_copy")?.is_truthy()? {
        return py.get_type::<pyo3::types::PySet>().call0();
    }
    module.call_method1("moving_faces", (&tool,))
}
fn drag<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("_grip_rot")
}
fn wireframe_color<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok(if !tool.getattr("_grip_rot")?.is_none() {
        tool.getattr("_grip_rot")?.getattr("wireframe_color")?
    } else {
        none(py)
    })
}
fn drag_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.move_host")?;
    let tool = a.get_item(0)?;
    if !tool.getattr("_grip_rot")?.is_none() {
        return (
            module.call_method1(
                "QVector3D",
                (tool.getattr("_grip_rot")?.getattr("start_point")?,),
            )?,
            module.call_method1(
                "QVector3D",
                (tool.getattr("_grip_rot")?.call_method0("_axis")?,),
            )?,
        )
            .into_bound_py_any(py);
    }
    Ok(none(py))
}
fn clear_grips<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    tool.setattr("_grip_group", none(py))?;
    tool.setattr("_grips", PyList::empty(py).into_any())?;
    tool.setattr("_hot_grip", none(py))?;
    Ok(none(py))
}
fn track_grips<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.move_host")?;
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    module.call_method1(
        "track_move_grips",
        (&tool, &ctx, module.getattr("GRIP_PX")?),
    )?;
    Ok(none(py))
}
fn grip_under<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.move_host")?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let x = a.get_item(2)?;
    let y = a.get_item(3)?;
    let move_grip_hit = imported(py, "built_core")?.getattr("move_grip_hit")?;
    Ok(if tool.getattr("_grips")?.is_truthy()? {
        move_grip_hit.call1((
            &viewport,
            tool.getattr("_grips")?,
            (&x, &y).into_bound_py_any(py)?,
            module.getattr("GRIP_PX")?,
        ))?
    } else {
        none(py)
    })
}
fn hovered_vertex<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(0)?;
    let move_endpoint = imported(py, "built_core")?.getattr("move_endpoint")?;
    move_endpoint.call1((&ctx,))
}
fn make_copy_builder<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let scene = a.get_item(1)?;
    let tool = a.get_item(0)?;
    let copy_builder = imported(py, "built_core")?.getattr("MoveCopies")?;
    copy_builder.call1((
        &scene,
        tool.getattr("_groups")?,
        tool.getattr("_sel_faces")?,
        tool.getattr("_sel_edges")?,
        tool.getattr("_moved_guides")?,
    ))
}
fn shift<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.move_host")?;
    let tool = a.get_item(0)?;
    let step = a.get_item(2)?;
    let viewport = a.get_item(1)?;
    module
        .call_method1(
            "TransformPose",
            (&tool, viewport.getattr("scene")?, "move", false),
        )?
        .call_method1("translate", (viewport.getattr("scene")?, &step))?;
    Ok(none(py))
}
fn commit<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let delta = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if tool.getattr("grab")?.is_none() {
        tool.getattr("_machine")?
            .call_method1("set", ("grab", (0.0, 0.0, 0.0).into_bound_py_any(py)?))?;
    }
    let d = if delta.hasattr("toTuple")? {
        delta.call_method0("toTuple")?
    } else {
        positional(&delta)?.into_any()
    };
    tool.call_method1("_send", (&viewport, fields!(py; "type" => "value", "value" => py.get_type::<pyo3::types::PyTuple>().call1((imported(py, "builtins")?.getattr("map")?.call1((py.get_type::<pyo3::types::PyFloat>().into_any(), &d,))?,))?).into_any(),))?;
    tool.call_method1("_after", (&viewport,))?;
    Ok(none(py))
}
fn reset<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    tool.getattr("_machine")?
        .call_method1("set", ("reset", true))?;
    tool.call_method0("_clear_sets")?;
    Ok(none(py))
}

fn on_click<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    let viewport = ctx.getattr("viewport")?;
    if tool
        .getattr("_turn")?
        .call_method1("click", (&tool, &ctx))?
        .is_truthy()?
    {
        return Ok(none(py));
    }
    tool.setattr("_ctx", &ctx)?;
    let action = (|| -> Reply<'py> {
        tool.call_method1(
            "_send",
            (&viewport, tool.call_method1("_pointer", ("click", &ctx))?),
        )?;
        Ok(none(py))
    })();
    tool.setattr("_ctx", none(py))?;
    action?;
    tool.call_method1("_after", (&viewport,))?;
    Ok(none(py))
}
fn moving_guide_ids<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let mut g: Obj<'py>;
    let tool = a.get_item(0)?;
    if tool.getattr("grab")?.is_none() || tool.getattr("_copy")?.is_truthy()? {
        return Ok(py.get_type::<pyo3::types::PySet>().call0()?);
    }
    return Ok({
        let collected = PySet::empty(py)?;
        for item in tool.getattr("_moved_guides")?.try_iter()? {
            g = item?;
            collected.add(imported(py, "builtins")?.getattr("id")?.call1((&g,))?)?;
        }
        collected.into_any()
    });
}
fn tell_join<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.move_host")?;
    let mut c: Obj<'py>;
    let commands = a.get_item(1)?;
    let viewport = a.get_item(0)?;
    let flash = optional(&viewport, "flash_status")?;
    if flash.is_none() {
        return Ok(none(py));
    }
    let refused = optional(&viewport.getattr("history")?, "last_rejected")?;
    if refused.is_truthy()? {
        flash.call1((&refused, 8000))?;
    } else {
        if {
            let mut found = false;
            for item in commands.try_iter()? {
                c = item?;
                if c.getattr_opt("welded")?
                    .unwrap_or(false.into_bound_py_any(py)?)
                    .is_truthy()?
                {
                    found = true;
                    break;
                }
            }
            found.into_bound_py_any(py)?
        }
        .is_truthy()?
        {
            flash.call1((module.call_method1("tr", ("Vertices joined",))?,))?;
        }
    }
    Ok(none(py))
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("_clear_sets", Mode::Method, clear_sets, &["self"], "None", ""),
    ("_copy", Mode::Property, copy, &["self"], "bool", ""),
    ("cursor_plus", Mode::Property, cursor_plus, &["self"], "bool", "The little + beside the cursor while Ctrl has it making a COPY."),
    ("_after", Mode::Method, after, &["self", "viewport"], "", ""),
    ("on_activate", Mode::Method, on_activate, &["self", "viewport"], "None", ""),
    ("on_deactivate", Mode::Method, on_deactivate, &["self", "viewport"], "None", ""),
    ("on_key", Mode::Method, on_key, &["self", "viewport", "key:int", "modifiers"], "bool", ""),
    ("_copy_changed", Mode::Method, copy_changed, &["self", "viewport"], "None", ""),
    ("_toggle_copy", Mode::Method, toggle_copy, &["self", "viewport", "say:bool=True"], "None", "Ctrl pressed with the click (the Metal view's own check)."),
    ("on_key_release", Mode::Method, on_key_release, &["self", "viewport", "key:int"], "bool", ""),
    ("on_click", Mode::Method, on_click, &["self", "ctx:ToolContext"], "None", ""),
    ("on_hover", Mode::Method, on_hover, &["self", "ctx:ToolContext"], "None", ""),
    ("value_is_unitless", Mode::Method, value_is_unitless, &["self"], "bool", "A rotation grip held (or just used) takes an ANGLE (#176);\notherwise the value is a distance."),
    ("on_value", Mode::Method, on_value, &["self", "viewport", "value"], "bool", ""),
    ("array_pending", Mode::Method, array_pending, &["self", "viewport"], "bool", "Whether a copy was just made, so «3x» / «/3» would lay an array:\nthe viewport then lets a bare «x» open the value box (#542)."),
    ("_last", Mode::Property, last, &["self"], "", ""),
    ("close_array_window", Mode::Method, close_array_window, &["self"], "None", "Esc right after a copy: «3x» / «/3» stop applying and a bare X\nis the Text shortcut again (#542)."),
    ("on_array_value", Mode::Method, on_array_value, &["self", "viewport", "count:int", "mode:str", "step:float | None=None"], "bool", "Arrays, typed right after a Move-copy (``3x``, ``/3``, ``5x10m``,\n#111): the machine decides, the copy builder stamps."),
    ("on_cancel", Mode::Method, on_cancel, &["self", "viewport"], "None", ""),
    ("rubber_band_lines", Mode::Method, rubber_band_lines, &["self"], "", ""),
    ("value_label", Mode::Method, value_label, &["self"], "", ""),
    ("snap_excluded", Mode::Method, snap_excluded, &["self"], "", "What is in motion, for the snap engine to leave out of its\ncandidates: ``(edge ids, group ids)`` or ``None``. The entities\nbeing moved are excluded from inference — otherwise the tool snaps\nto the very geometry it is dragging (issue #19, @pacaeiro). In copy\nmode nothing moves (the ghost is a wireframe), so nothing is left\nout."),
    ("moving_face_ids", Mode::Method, moving_face_ids, &["self"], "set", "The loose faces the drag bends — any face with a corner on a\ngrabbed vertex. The viewport keeps the cursor from landing on\nthem (it chased its own face and shook)."),
    ("moving_guide_ids", Mode::Method, moving_guide_ids, &["self"], "set", "The guides the drag carries (#627). They pass through the\ncursor all the way, so the viewport keeps them out of the snap\ncandidates — the cursor would otherwise lock onto its own guide."),
    ("_drag", Mode::Property, drag, &["self"], "", "Busy while a grip is held — the viewport's Esc asks this."),
    ("wireframe_color", Mode::Property, wireframe_color, &["self"], "", ""),
    ("drag_plane", Mode::Method, drag_plane, &["self", "viewport"], "", "While a grip turns the object the cursor is read on the plane of\nthe turn — the face the grip sits on."),
    ("_clear_grips", Mode::Method, clear_grips, &["self"], "None", ""),
    ("_track_grips", Mode::Method, track_grips, &["self", "ctx"], "", ""),
    ("_grip_under", Mode::Method, grip_under, &["self", "viewport", "x", "y"], "", ""),
    ("_hovered_vertex", Mode::Static, hovered_vertex, &["ctx"], "", ""),
    ("_make_copy_builder", Mode::Method, make_copy_builder, &["self", "scene=None"], "", "The copy command for a list of offsets, kept by the array window\nso ``3x`` / ``/3`` re-lay the copies (``built_core.move_ops.copy_builder``,\nthe implementation the protocol's ``move_selection`` runs too)."),
    ("_shift", Mode::Method, shift, &["self", "viewport", "step:QVector3D"], "None", ""),
    ("_commit", Mode::Method, commit, &["self", "viewport", "delta"], "None", "Commit what is grabbed by ``delta`` (as a typed delta would)."),
    ("_tell_join", Mode::Static, tell_join, &["viewport", "commands"], "None", "Say what became of a landing on other vertices: joined, or\nrefused (and the move undone) because the join would break a\nface."),
    ("_reset", Mode::Method, reset, &["self"], "None", ""),
    ("_copy", Mode::Setter, set_copy, &["self", "on"], "None", ""),
];
