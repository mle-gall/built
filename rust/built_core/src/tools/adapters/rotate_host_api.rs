// SPDX-License-Identifier: GPL-3.0-or-later
//! RotateHost callbacks on the live document.
use super::*;

/// The grip pick radius, as Move's.
const GRIP_PX: f64 = 9.0;

fn on_activate<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1(
        "_send",
        (&viewport, fields!(py; "type" => "activate").into_any()),
    )?;
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
    tool.setattr("_axis_pick", none(py))?;
    tool.setattr("hover_point", none(py))?;
    crate::tools::move_hover::clear(&tool)?;
    Ok(none(py))
}
fn cursor_plus<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("_copy")
}
fn toggle_copy<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let say = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_send", (&viewport, fields!(py; "type" => "key", "key" => if say.is_truthy()? { ("ToggleCopy").into_bound_py_any(py)? } else { ("ToggleCopyQuiet").into_bound_py_any(py)? }).into_any(),))?;
    Ok(none(py))
}
fn host_copy_changed<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rotate_host")?;
    let arguments = a.get_item(2)?;
    let copy: Obj<'py>;
    let say: Obj<'py>;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    [copy, say] = unpack::<2>(&arguments)?;
    tool.getattr("_during")?.set_item("copy", copy.clone())?;
    if tool.getattr("_copy")?.is_truthy()? {
        tool.call_method1("_revert_preview", (&viewport,))?;
        if say.is_truthy()? {
            viewport.call_method1(
                "flash_status",
                (module.call_method1("tr", ("Rotate a copy: on",))?,),
            )?;
        }
    } else if say.is_truthy()? {
        viewport.call_method1(
            "flash_status",
            (module.call_method1("tr", ("Rotate a copy: off",))?,),
        )?;
    }
    let apply = optional(&viewport, "_apply_tool_cursor")?;
    if !apply.is_none() {
        apply.call0()?;
    }
    viewport.call_method0("update")?;
    Ok(none(py))
}
/// The box grips show until the swing starts: before the centre and
/// before the start point (both are taken on a grip, exactly).
fn grips_wanted(tool: &Obj<'_>) -> PyResult<bool> {
    Ok(tool.getattr("ref_point")?.is_none())
}
fn on_click<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let mut ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    if grips_wanted(&tool)? && !tool.getattr("_hot_box")?.is_none() {
        if tool.getattr("start_point")?.is_none() {
            tool.setattr("_grip_pick", tool.getattr("_grip_groups")?)?;
        }
        ctx = crate::tools::move_hover::grip_context(&tool, &ctx)?;
    }
    let done = tool.call_method1("_pointer_event", ("click", &ctx));
    tool.setattr("_grip_pick", none(py))?;
    done?;
    if !grips_wanted(&tool)? {
        crate::tools::move_hover::clear(&tool)?;
    }
    Ok(none(py))
}
fn host_pick_rotation<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rotate_host")?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let transform_grab = imported(py, "built_core")?.getattr("transform_grab")?;
    let ctx = tool.getattr("_ctx")?;
    if !crate::tools::move_hover::grip_grab(&tool, "rotate")?
        && !(transform_grab.call1((&tool, &ctx, "rotate"))?.is_truthy()?)
    {
        viewport.call_method1(
            "flash_status",
            (module.call_method1("tr", ("Select (or click) the geometry to rotate first",))?,),
        )?;
        return false.into_bound_py_any(py);
    }
    tool.getattr("_gesture")?
        .call_method1("begin", (&tool, &viewport, false))?;
    tool.call_method1("_gather_copy_entities", (&ctx,))?;
    tool.setattr(
        "_centre_plane",
        (
            ctx.getattr("world")?,
            module.call_method1("QVector3D", (tool.call_method0("_axis")?,))?,
        )
            .into_bound_py_any(py)?,
    )?;
    tool.setattr("_axis_drag_armed", true)?;
    true.into_bound_py_any(py)
}
fn host_rotation_angle<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rotate_host")?;
    let point = a.get_item(2)?;
    let tool = a.get_item(0)?;
    tool.call_method1("_note_snap", (tool.getattr("_ctx")?,))?;
    tool.call_method1(
        "_commit_deg",
        (module.call_method1("QVector3D", positional(&point)?)?,),
    )
}
fn on_hover<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let mut ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    if grips_wanted(&tool)? {
        crate::tools::move_hover::track_move_grips(&tool, &ctx, GRIP_PX, Some(false))?;
        ctx = crate::tools::move_hover::grip_context(&tool, &ctx)?;
    }
    tool.call_method1("_pointer_event", ("hover", &ctx))?;
    ctx.getattr("viewport")?.call_method0("update")?;
    Ok(none(py))
}
fn host_protractor_hover<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let _viewport = a.get_item(1)?;
    let ctx = tool.getattr("_ctx")?;
    tool.setattr("hover_point", ctx.getattr("world")?)?;
    tool.call_method1("_track_axis_drag", (ctx.getattr("viewport")?,))?;
    tool.call_method1("_infer_plane", (&ctx,))?;
    tool.call_method1("_update_screen_metrics", (&ctx,))?;
    tool.call_method1("_note_snap", (&ctx,))?;
    Ok(none(py))
}
fn host_rotation_preview<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let angle = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_apply_preview", (&viewport, &angle))?;
    Ok(none(py))
}
fn host_swing_angle<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    if tool.getattr("_copy")?.is_truthy()? && !tool.getattr("hover_point")?.is_none() {
        return tool.call_method1("_angle_to", (tool.getattr("hover_point")?,));
    }
    tool.getattr("_gesture")?.getattr("angle")
}
fn on_release<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rotate_host")?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if tool
        .call_method1("_release_axis_drag", (&viewport,))?
        .is_truthy()?
    {
        viewport.call_method1(
            "flash_status",
            (module.call_method1("tr", ("Rotation axis set along the drag",))?,),
        )?;
    }
    Ok(none(py))
}
fn on_value<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let value = a.get_item(2)?;
    let viewport = a.get_item(1)?;
    if value.is_instance(&py.get_type::<pyo3::types::PyTuple>().into_any())? {
        return false.into_bound_py_any(py);
    }
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
fn on_array_value<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let count = a.get_item(2)?;
    let mode = a.get_item(3)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let result = tool.call_method1(
        "_send",
        (
            &viewport,
            fields!(py; "type" => "array", "count" => &count, "mode" => &mode, "step" => a.get_item(4)?).into_any(),
        ),
    )?;
    if result.get_item("redraw")?.is_truthy()? {
        viewport.call_method0("update")?;
    }
    result.get_item("consumed")
}
fn array_pending<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let _host = imported(py, "tools.rust_tool_base")?.getattr("_Host")?;
    let token = tool.call_method1("_state", ("array_token",))?;
    Ok({
        let value = (!token.is_none()).into_bound_py_any(py)?;
        if !value.is_truthy()? {
            value
        } else {
            (token.eq(_host.call1((&viewport,))?.call_method0("revision")?)?)
                .into_bound_py_any(py)?
        }
    })
}
fn close_array_window<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    tool.getattr("_machine")?
        .call_method1("set", ("close_window", py.None()))?;
    Ok(none(py))
}
fn host_park_rotation<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.getattr("_gesture")?
        .call_method1("revert", (&viewport, false))?;
    Ok(none(py))
}
fn host_end_rotation<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.getattr("_during")?.call_method("update", (), Some(&fields!(py; "start_point" => py.None(), "ref_point" => py.None(), "copy" => false, "array_token" => py.None())))?;
    tool.call_method0("_reset")?;
    let apply = optional(&viewport, "_apply_tool_cursor")?;
    if !apply.is_none() {
        apply.call0()?;
    }
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn snap_excluded<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if tool.getattr("ref_point")?.is_none() || tool.getattr("_copy")?.is_truthy()? {
        return Ok(none(py));
    }
    let snap_exclusions = imported(py, "built_core")?.getattr("snap_exclusions")?;
    snap_exclusions.call1((&tool, false))
}
fn rubber_band_lines<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rotate_host")?;
    let matrix: Obj<'py>;
    let tool = a.get_item(0)?;
    let angle = if !tool.getattr("start_point")?.is_none()
        && !tool.getattr("ref_point")?.is_none()
        && !tool.getattr("hover_point")?.is_none()
    {
        tool.call_method1("_commit_deg", (tool.getattr("hover_point")?,))?
    } else {
        none(py)
    };
    let segments = tool.call_method1("_disc_lines", (&angle,))?;
    if tool.getattr("_copy")?.is_truthy()? && !angle.is_none() {
        tool.call_method0("_fill_groups_ghost")?;
        matrix = module.call_method1(
            "rotation_matrix",
            (
                tool.getattr("start_point")?,
                tool.call_method0("_axis")?,
                &angle,
            ),
        )?;
        segments.call_method1(
            "extend",
            (tool.getattr("_base_segments")?.call_method(
                "lines",
                (),
                Some(&fields!(py; "matrix" => matrix.call_method0("data")?)),
            )?,),
        )?;
    }
    Ok(segments.clone())
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
    let fmt_angle_deg = imported(py, "built_core.units")?.getattr("fmt_angle_deg")?;
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
fn apply_preview<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let target_deg = a.get_item(2)?;
    let viewport = a.get_item(1)?;
    tool.getattr("_gesture")?
        .call_method1("rotate", (&tool, &viewport, &target_deg))?;
    Ok(none(py))
}
fn revert_preview<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.getattr("_gesture")?
        .call_method1("revert", (&viewport, true))?;
    Ok(none(py))
}
fn make_builder<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rotate_host")?;
    let tool = a.get_item(0)?;
    let partial = imported(py, "functools")?.getattr("partial")?;
    let rotation_command = imported(py, "built_core")?.getattr("rotation_command")?;
    partial.call1((
        &rotation_command,
        module.call_method1("QVector3D", (tool.getattr("start_point")?,))?,
        module.call_method1("QVector3D", (tool.call_method0("_axis")?,))?,
        tool.getattr("_copy")?,
        py.get_type::<pyo3::types::PyList>()
            .call1((tool.getattr("_groups")?,))?,
        py.get_type::<pyo3::types::PyList>()
            .call1((tool.getattr("_positions")?,))?,
        py.get_type::<pyo3::types::PyList>()
            .call1((tool.getattr("_sel_faces")?,))?,
        py.get_type::<pyo3::types::PyList>()
            .call1((tool.getattr("_sel_edges")?,))?,
        py.get_type::<pyo3::types::PyList>()
            .call1((tool.getattr("_moved_guides")?,))?,
        py.get_type::<pyo3::types::PyList>()
            .call1((tool.getattr("_splanes")?,))?,
        py.get_type::<pyo3::types::PyList>()
            .call1((tool.getattr("_images")?,))?,
    ))
}
fn host_rotation_args<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let mut g: Obj<'py>;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let scene = viewport.getattr("scene")?;
    return Ok(py.get_type::<pyo3::types::PyDict>().call((), Some(&fields!(py; "center" => tool.getattr("start_point")?.call_method0("toTuple")?, "axis" => tool.call_method0("_axis")?.call_method0("toTuple")?, "copy" => tool.getattr("_copy")?, "placements" => {let items=PyList::empty(py); for item in tool.getattr("_groups")?.try_iter()? { g = item?;
items.append({ let value = scene.call_method1("occurrence_path", (&g,))?; if value.is_truthy()? {value} else {g.getattr("uid")?} })?;
} items.into_any()}, "vertices" => {let items=PyList::empty(py); for item in tool.getattr("_verts")?.try_iter()? { let v = item?;
items.append(py.get_type::<pyo3::types::PyString>().call1((v.getattr("key")?,))?)?;
} items.into_any()}, "faces" => {let items=PyList::empty(py); for item in tool.getattr("_sel_faces")?.try_iter()? { let f = item?;
items.append(py.get_type::<pyo3::types::PyString>().call1((f.getattr("key")?,))?)?;
} items.into_any()}, "edges" => {let items=PyList::empty(py); for item in tool.getattr("_sel_edges")?.try_iter()? { let e = item?;
items.append(py.get_type::<pyo3::types::PyString>().call1((e.getattr("key")?,))?)?;
} items.into_any()}, "images" => {let items=PyList::empty(py); for item in tool.getattr("_images")?.try_iter()? { let im = item?;
items.append(scene.getattr("image_planes")?.call_method1("index", (&im,))?)?;
} items.into_any()}, "guides" => {let items=PyList::empty(py); for item in tool.getattr("_moved_guides")?.try_iter()? { g = item?;
items.append(scene.getattr("guides")?.call_method1("index", (&g,))?)?;
} items.into_any()}, "sections" => {let items=PyList::empty(py); for item in tool.getattr("_splanes")?.try_iter()? { let sp = item?;
items.append(scene.getattr("section_planes")?.call_method1("index", (&sp,))?)?;
} items.into_any()})))?);
}
fn reset<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rotate_host")?;
    let tool = a.get_item(0)?;
    tool.call_method0("_reset_protractor")?;
    tool.setattr("_groups", PyList::empty(py).into_any())?;
    tool.setattr("_splanes", PyList::empty(py).into_any())?;
    tool.setattr("_images", PyList::empty(py).into_any())?;
    tool.setattr("_moved_guides", PyList::empty(py).into_any())?;
    tool.setattr("_positions", PyList::empty(py).into_any())?;
    tool.setattr("_verts", PyList::empty(py).into_any())?;
    tool.setattr("_sel_faces", PyList::empty(py).into_any())?;
    tool.setattr("_sel_edges", PyList::empty(py).into_any())?;
    tool.setattr("_base_segments", module.call_method0("CopyWire")?)?;
    Ok(none(py))
}

fn on_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rotate_host")?;
    let key = a.get_item(2)?;
    let modifiers = a.get_item(3)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if key.eq(module.getattr("Qt")?.getattr("Key_Alt")?)?
        && grips_wanted(&tool)?
        && crate::tools::move_hover::cycle_box_grips(&tool, &viewport)?
    {
        return true.into_bound_py_any(py);
    }
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
        .call1((module.getattr("RotateHost")?, &tool))?
        .call_method1("on_key", (&viewport, &key, &modifiers))
}
fn on_key_release<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rotate_host")?;
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if key.ne(module.getattr("Qt")?.getattr("Key_Control")?)? {
        return false.into_bound_py_any(py);
    }
    Ok(tool.call_method1("_send", (&viewport, fields!(py; "type" => "key_release", "key" => "Control", "tapped" => if let Some(call)=viewport.getattr_opt("ctrl_tapped")? {call.call0()?} else {true.into_bound_py_any(py)?}).into_any(),))?.get_item("consumed")?)
}
fn pointer_event<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(2)?;
    let kind = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.setattr("_ctx", &ctx)?;
    let action = (|| -> Reply<'py> {
        tool.call_method1(
            "_send",
            (
                ctx.getattr("viewport")?,
                tool.call_method1("_pointer", (&kind, &ctx))?,
            ),
        )?;
        Ok(none(py))
    })();
    tool.setattr("_ctx", none(py))?;
    action?;
    Ok(none(py))
}
fn moving_guide_ids<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let mut g: Obj<'py>;
    let tool = a.get_item(0)?;
    if tool.getattr("ref_point")?.is_none() || tool.getattr("_copy")?.is_truthy()? {
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
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("on_activate", Mode::Method, on_activate, &["self", "viewport"], "", ""),
    ("on_deactivate", Mode::Method, on_deactivate, &["self", "viewport"], "", ""),
    ("cursor_plus", Mode::Property, cursor_plus, &["self"], "bool", "The little + beside the cursor while Ctrl has it making a COPY,\nas the Tape shows its guide mode: the status-bar flash alone was\neasy to miss."),
    ("on_key", Mode::Method, on_key, &["self", "viewport", "key:int", "modifiers"], "bool", ""),
    ("_toggle_copy", Mode::Method, toggle_copy, &["self", "viewport", "say=True"], "", ""),
    ("host_copy_changed", Mode::Method, host_copy_changed, &["self", "viewport", "args"], "", ""),
    ("on_key_release", Mode::Method, on_key_release, &["self", "viewport", "key:int"], "bool", ""),
    ("_pointer_event", Mode::Method, pointer_event, &["self", "kind", "ctx"], "", ""),
    ("on_click", Mode::Method, on_click, &["self", "ctx"], "", ""),
    ("host_pick_rotation", Mode::Method, host_pick_rotation, &["self", "viewport", "point"], "", ""),
    ("host_rotation_angle", Mode::Method, host_rotation_angle, &["self", "viewport", "point"], "", ""),
    ("on_hover", Mode::Method, on_hover, &["self", "ctx"], "", ""),
    ("host_protractor_hover", Mode::Method, host_protractor_hover, &["self", "viewport", "_"], "", ""),
    ("host_rotation_preview", Mode::Method, host_rotation_preview, &["self", "viewport", "angle"], "", ""),
    ("host_swing_angle", Mode::Method, host_swing_angle, &["self", "viewport", "_"], "", ""),
    ("on_release", Mode::Method, on_release, &["self", "viewport"], "None", "A real DRAG from the centre fixes the rotation axis along it\n(the classic fold gesture); a plain click keeps the inferred plane.\nThe gesture lives in :class:`ProtractorBase`, shared with the\nProtractor tool."),
    ("on_value", Mode::Method, on_value, &["self", "viewport", "value"], "", ""),
    ("on_array_value", Mode::Method, on_array_value, &["self", "viewport", "count", "mode", "step=None"], "", ""),
    ("array_pending", Mode::Method, array_pending, &["self", "viewport"], "", ""),
    ("close_array_window", Mode::Method, close_array_window, &["self"], "", ""),
    ("host_park_rotation", Mode::Method, host_park_rotation, &["self", "viewport", "_"], "", ""),
    ("host_end_rotation", Mode::Method, host_end_rotation, &["self", "viewport", "_"], "", ""),
    ("snap_excluded", Mode::Method, snap_excluded, &["self"], "", "The geometry swinging live, left out of the snap candidates so\nthe tool never infers against itself (issue #19). Mirrors\nMoveTool.snap_excluded; in copy mode the original stays put."),
    ("moving_guide_ids", Mode::Method, moving_guide_ids, &["self"], "set", "The guides swinging live, kept out of the snap (#627)."),
    ("rubber_band_lines", Mode::Method, rubber_band_lines, &["self"], "", ""),
    ("value_label", Mode::Method, value_label, &["self"], "", ""),
    ("vcb_caption", Mode::Method, vcb_caption, &["self"], "str", ""),
    ("_apply_preview", Mode::Method, apply_preview, &["self", "viewport", "target_deg"], "", ""),
    ("_revert_preview", Mode::Method, revert_preview, &["self", "viewport"], "", ""),
    ("_make_builder", Mode::Method, make_builder, &["self"], "", ""),
    ("host_rotation_args", Mode::Method, host_rotation_args, &["self", "viewport", "_"], "", ""),
    ("_reset", Mode::Method, reset, &["self"], "None", ""),
];
