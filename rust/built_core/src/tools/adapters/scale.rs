// SPDX-License-Identifier: GPL-3.0-or-later
//! ScaleHost callbacks on the live document.
use super::*;

fn uses_snap<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    (!tool.getattr("_grip")?.is_none()).into_bound_py_any(py)
}
fn host_activate_box<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method0("_reset")?;
    tool.setattr("about_center", false)?;
    tool.setattr("uniform", false)?;
    tool.call_method1("_refresh_box", (&viewport,))?;
    Ok(none(py))
}
fn host_deactivate_box<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_revert_preview", (&viewport,))?;
    tool.call_method0("_reset")?;
    Ok(none(py))
}
fn refresh_box<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.scale_host")?;
    let mut mask: Obj<'py>;
    let mut p: Obj<'py>;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if !tool.getattr("_grip")?.is_none() {
        return Ok(none(py));
    }
    if viewport
        .getattr("scene")?
        .getattr("version")?
        .eq(tool.getattr("_box_version")?)?
    {
        return Ok(none(py));
    }
    tool.setattr(
        "_box_version",
        viewport.getattr("scene")?.getattr("version")?,
    )?;
    {
        let [part0, part1] = unpack::<2>(&tool.call_method1("_selection_bounds", (&viewport,))?)?;
        tool.setattr("_lo", part0)?;
        tool.setattr("_hi", part1)?;
    }
    tool.setattr("_grips", PyList::empty(py).into_any())?;
    tool.setattr("_hover_grip", none(py))?;
    if tool.getattr("_lo")?.is_none() {
        return Ok(none(py));
    }
    let grips = tool.getattr("_box")?.call_method0("scale_grips")?;
    tool.setattr("_grips", {
        let items = PyList::empty(py);
        for item in grips.try_iter()? {
            [p, mask] = unpack::<2>(&item?)?;
            items.append(module.call_method1("_Grip", (positional(&p)?, positional(&mask)?))?)?;
        }
        items.into_any()
    })?;
    if !grips.is_truthy()? {
        {
            let value = none(py);
            tool.setattr("_lo", &value)?;
            tool.setattr("_hi", &value)?;
        }
    }
    Ok(none(py))
}
fn extents<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    (
        tool.getattr("_hi")?
            .call_method0("x")?
            .sub(tool.getattr("_lo")?.call_method0("x")?)?,
        tool.getattr("_hi")?
            .call_method0("y")?
            .sub(tool.getattr("_lo")?.call_method0("y")?)?,
        tool.getattr("_hi")?
            .call_method0("z")?
            .sub(tool.getattr("_lo")?.call_method0("z")?)?,
    )
        .into_bound_py_any(py)
}
fn grip_pos<'py>(a: &Args<'py>) -> Reply<'py> {
    let grip = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.getattr("_box")?
        .call_method1("grip", (grip.getattr("params")?,))
}
fn grip_under<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.scale_host")?;
    let tool = a.get_item(0)?;
    let sx = a.get_item(2)?;
    let sy = a.get_item(3)?;
    let viewport = a.get_item(1)?;
    let threshold = py.get_type::<pyo3::types::PyFloat>().call1((viewport
        .getattr_opt("pick_threshold_px")?
        .unwrap_or(10.0.into_bound_py_any(py)?),))?;
    let cam = optional(&viewport, "camera")?;
    let eye = if !cam.is_none() {
        cam.call_method0("eye")?
    } else {
        module.call_method0("QVector3D")?
    };
    let fwd = if !cam.is_none() {
        cam.getattr("target")?.sub(&eye)?
    } else {
        module.call_method0("QVector3D")?
    };
    let i = tool.getattr("_box")?.call_method1(
        "pick_scale",
        (
            &viewport,
            (&sx, &sy).into_bound_py_any(py)?,
            &threshold,
            &eye,
            &fwd,
        ),
    )?;
    Ok(if !i.is_none() {
        tool.getattr("_grips")?.get_item(&i)?
    } else {
        none(py)
    })
}
fn ray<'py>(a: &Args<'py>) -> Reply<'py> {
    let sx = a.get_item(2)?;
    let sy = a.get_item(3)?;
    let viewport = a.get_item(1)?;
    viewport.call_method1("_pixel_to_ray", (&sx, &sy))
}
fn snap_excluded<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if tool.getattr("_grip")?.is_none() {
        return Ok(none(py));
    }
    let snap_exclusions = imported(py, "built_core")?.getattr("snap_exclusions")?;
    snap_exclusions.call1((&tool, true))
}
fn snapped_point<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.scale_host")?;
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    let snap = if !ctx.is_none() {
        optional(&ctx, "snap")?
    } else {
        none(py)
    };
    if snap.is_none()
        || !tool
            .getattr("_TARGET_KINDS")?
            .contains(snap.getattr("kind")?)?
    {
        return Ok(none(py));
    }
    module.call_method1("QVector3D", (snap.getattr("point")?,))
}
fn grip_spec<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.scale_host")?;
    let tool = a.get_item(0)?;
    let grip = tool.getattr("_grip")?;
    Ok(module.call_method1(
        "ScaleGripSpec",
        (
            tool.call_method0("_extents")?,
            grip.getattr("mask")?,
            ({
                let value = tool.getattr("uniform")?;
                if value.is_truthy()? {
                    value
                } else {
                    (grip
                        .call_method1("kind", (tool.call_method0("_active_axes")?,))?
                        .eq("corner")?)
                    .into_bound_py_any(py)?
                }
            }),
        ),
    )?)
}
fn line_grip<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let grip = tool.getattr("_grip")?;
    Ok({
        let value = tool.getattr("uniform")?;
        if value.is_truthy()? {
            value
        } else {
            {
                let value = (grip.getattr("mask")?.len()? == 1).into_bound_py_any(py)?;
                if value.is_truthy()? {
                    value
                } else {
                    (grip
                        .call_method1("kind", (tool.call_method0("_active_axes")?,))?
                        .eq("corner")?)
                    .into_bound_py_any(py)?
                }
            }
        }
    })
}
fn relative<'py>(a: &Args<'py>) -> Reply<'py> {
    let point = a.get_item(1)?;
    let tool = a.get_item(0)?;
    if tool.getattr("_axes")?.is_none() || tool.call_method0("_line_grip")?.is_truthy()? {
        return point.sub(tool.getattr("_anchor")?)?.call_method0("toTuple");
    }
    tool.getattr("_box")?
        .call_method1("local", (&point,))?
        .sub(
            tool.getattr("_box")?
                .call_method1("local", (tool.getattr("_anchor")?,))?,
        )?
        .call_method0("toTuple")
}
fn factors_from_point<'py>(a: &Args<'py>) -> Reply<'py> {
    let p = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.call_method0("_grip_spec")?.call_method1(
        "point",
        (
            tool.call_method1(
                "_relative",
                (tool.call_method1("_grip_pos", (tool.getattr("_grip")?,))?,),
            )?,
            tool.call_method1("_relative", (&p,))?,
        ),
    )
}
fn factors_for<'py>(a: &Args<'py>) -> Reply<'py> {
    let ctx = a.get_item(1)?;
    let got: Obj<'py>;
    let tool = a.get_item(0)?;
    let sx = a.get_item(3)?;
    let sy = a.get_item(4)?;
    let viewport = a.get_item(2)?;
    let p = tool.call_method1("_snapped_point", (&ctx,))?;
    if !p.is_none() {
        got = tool.call_method1("_factors_from_point", (&p,))?;
        if !got.is_none() {
            return Ok(got.clone());
        }
    }
    tool.call_method1("_factors_from_cursor", (&viewport, &sx, &sy))
}
fn on_click<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.call_method1("_pointer_event", ("click", &ctx))?;
    Ok(none(py))
}
fn on_hover<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.call_method1("_pointer_event", ("hover", &ctx))?;
    Ok(none(py))
}
fn host_pick_grip<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.scale_host")?;
    let edge: Obj<'py>;
    let face: Obj<'py>;
    let grip: Obj<'py>;
    let groups: Obj<'py>;
    let images: Obj<'py>;
    let mut picked: Obj<'py>;
    let positions: Obj<'py>;
    let tool = a.get_item(0)?;
    let sx: Obj<'py>;
    let sy: Obj<'py>;
    let viewport = a.get_item(1)?;
    let xy = a.get_item(2)?;
    let gather_images = imported(py, "tools.move_host")?.getattr("gather_images")?;
    let gather_targets = imported(py, "tools.move_host")?.getattr("gather_targets")?;
    let ctx = tool.getattr("_ctx")?;
    tool.call_method1("_refresh_box", (&viewport,))?;
    [sx, sy] = unpack::<2>(&xy)?;
    if !tool.getattr("_lo")?.is_none() {
        grip = tool.call_method1("_grip_under", (&viewport, &sx, &sy))?;
        if !grip.is_none() {
            tool.call_method1(
                "_capture_grip",
                (&viewport, &grip, (&sx, &sy).into_bound_py_any(py)?),
            )?;
            return true.into_bound_py_any(py);
        }
    }
    if !viewport
        .getattr("scene")?
        .getattr("selection")?
        .is_truthy()?
    {
        [groups, positions] = unpack::<2>(&gather_targets.call1((&ctx,))?)?;
        images = gather_images.call1((&ctx,))?;
        picked = py
            .get_type::<pyo3::types::PyList>()
            .call1((&groups,))?
            .add(py.get_type::<pyo3::types::PyList>().call1((&images,))?)?;
        if !(picked.is_truthy()?) && positions.is_truthy()? {
            edge = viewport.call_method1("pick_edge", (&sx, &sy))?;
            face = if edge.is_none() {
                viewport.call_method1("pick_face", (&sx, &sy))?
            } else {
                none(py)
            };
            picked = {
                let items = PyList::empty(py);
                for item in (&edge, &face).into_bound_py_any(py)?.try_iter()? {
                    let e = item?;
                    if !!e.is_none() {
                        continue;
                    }
                    items.append(&e)?;
                }
                items.into_any()
            };
        }
        if picked.is_truthy()? {
            viewport
                .getattr("scene")?
                .call_method1("select", (&picked,))?;
            tool.call_method1("_refresh_box", (&viewport,))?;
            viewport.call_method0("update")?;
            return Ok(none(py));
        }
        viewport.call_method1(
            "flash_status",
            (module.call_method1("tr", ("Select (or click) the geometry to scale first",))?,),
        )?;
    }
    Ok(none(py))
}
fn grab<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let grip = a.get_item(2)?;
    let screen_xy = a.get_item(3)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_capture_grip", (&viewport, &grip, &screen_xy))?;
    tool.getattr("_machine")?
        .call_method1("set", ("grab", &screen_xy))?;
    Ok(none(py))
}
fn capture_grip<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.scale_host")?;
    let grip = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    module.call_method1("scale_grab", (viewport.getattr("scene")?, &tool))?;
    tool.setattr("_grip", &grip)?;
    tool.setattr("_anchor", tool.call_method1("_anchor_for", (&grip,))?)?;
    tool.call_method1("_capture_originals", (&viewport,))?;
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn host_scale_preview<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let factors = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_apply_preview", (&viewport, &factors))?;
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn host_preview_factors<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("_factors")
}
fn on_release<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1(
        "_send",
        (&viewport, fields!(py; "type" => "release").into_any()),
    )?;
    Ok(none(py))
}
fn host_toggle_center<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.scale_host")?;
    let say = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.setattr(
        "about_center",
        (!(tool.getattr("about_center")?.is_truthy()?)).into_bound_py_any(py)?,
    )?;
    tool.call_method1("_reanchor", (&viewport,))?;
    if say.is_truthy()? {
        viewport.call_method1(
            "flash_status",
            ((if tool.getattr("about_center")?.is_truthy()? {
                module.call_method1("tr", ("About Center: on",))?
            } else {
                module.call_method1("tr", ("About Center: off",))?
            }),),
        )?;
    }
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn host_toggle_uniform<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.scale_host")?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.setattr(
        "uniform",
        (!(tool.getattr("uniform")?.is_truthy()?)).into_bound_py_any(py)?,
    )?;
    viewport.call_method1(
        "flash_status",
        ((if tool.getattr("uniform")?.is_truthy()? {
            module.call_method1("tr", ("Scale Uniformly: on",))?
        } else {
            module.call_method1("tr", ("Scale Uniformly: off",))?
        }),),
    )?;
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn on_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.scale_host")?;
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let name = fields!(py; module.getattr("Qt")?.getattr("Key_Control")? => "Control", module.getattr("Qt")?.getattr("Key_Shift")? => "Shift").into_any().call_method1("get", (&key,))?;
    Ok((name.is_truthy()?
        && tool
            .call_method1(
                "_send",
                (
                    &viewport,
                    fields!(py; "type" => "key", "key" => &name).into_any(),
                ),
            )?
            .get_item("consumed")?
            .is_truthy()?)
    .into_bound_py_any(py)?)
}
fn reanchor<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if tool.getattr("_grip")?.is_none() {
        return Ok(none(py));
    }
    let current = tool.getattr("_factors")?;
    tool.call_method1(
        "_apply_preview",
        (&viewport, (1.0, 1.0, 1.0).into_bound_py_any(py)?),
    )?;
    tool.setattr(
        "_anchor",
        tool.call_method1("_anchor_for", (tool.getattr("_grip")?,))?,
    )?;
    tool.call_method1("_apply_preview", (&viewport, &current))?;
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn value_is_unitless<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    true.into_bound_py_any(py)
}
fn host_cancel_box<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_revert_preview", (&viewport,))?;
    tool.setattr("_grip", none(py))?;
    tool.setattr("_anchor", none(py))?;
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn capture_originals<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.getattr("_gesture")?
        .call_method1("begin", (&tool, &viewport, true))?;
    Ok(none(py))
}
fn apply_preview<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let target = a.get_item(2)?;
    let viewport = a.get_item(1)?;
    tool.getattr("_gesture")?
        .call_method1("scale", (&tool, &viewport, &target))?;
    Ok(none(py))
}
fn revert_preview<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.getattr("_gesture")?
        .call_method1("revert", (&viewport, false))?;
    Ok(none(py))
}
fn commit<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let factors = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1(
        "_send",
        (
            &viewport,
            fields!(py; "type" => "scale_factors", "value" => &factors).into_any(),
        ),
    )?;
    Ok(none(py))
}
fn host_revert<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_revert_preview", (&viewport,))?;
    Ok(none(py))
}
fn host_grip_spec<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let grip = tool.getattr("_grip")?;
    return Ok(fields!(py; "extents" => tool.call_method0("_extents")?, "mask" => {let items=PyList::empty(py); for item in imported(py, "builtins")?.getattr("range")?.call1((3,))?.try_iter()? { let i = item?;
items.append((grip.getattr("mask")?.contains(&i)?).into_bound_py_any(py)?)?;
} items.into_any()}, "uniform" => { let value = tool.getattr("uniform")?; if value.is_truthy()? {value} else {(grip.call_method1("kind", (tool.call_method0("_active_axes")?,))?.eq("corner")?).into_bound_py_any(py)?} }).into_any());
}
fn host_scale_targets<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let scene = viewport.getattr("scene")?;
    return Ok(fields!(py; "anchor" => tool.getattr("_anchor")?.call_method0("toTuple")?, "vertices" => {let items=PyList::empty(py); for item in tool.getattr("_verts")?.try_iter()? { let v = item?;
items.append(py.get_type::<pyo3::types::PyString>().call1((v.getattr("key")?,))?)?;
} items.into_any()}, "placements" => {let items=PyList::empty(py); for item in tool.getattr("_groups")?.try_iter()? { let g = item?;
items.append({ let value = scene.call_method1("occurrence_path", (&g,))?; if value.is_truthy()? {value} else {g.getattr("uid")?} })?;
} items.into_any()}, "images" => {let items=PyList::empty(py); for item in tool.getattr("_images")?.try_iter()? { let im = item?;
items.append(scene.getattr("image_planes")?.call_method1("index", (&im,))?)?;
} items.into_any()}, "axes" => if tool.getattr("_axes")?.is_truthy()? { {let items=PyList::empty(py); for item in tool.getattr("_axes")?.try_iter()? { let first = item?;
items.append(first.call_method0("toTuple")?)?;
} items.into_any()} } else { none(py) }).into_any());
}
fn host_end_grip<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.setattr("_grip", none(py))?;
    tool.setattr("_anchor", none(py))?;
    tool.setattr("_box_version", 1.into_bound_py_any(py)?.neg()?)?;
    tool.call_method1("_refresh_box", (&viewport,))?;
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn reset<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    {
        let value = none(py);
        tool.setattr("_lo", &value)?;
        tool.setattr("_hi", &value)?;
    }
    tool.setattr("_grips", PyList::empty(py).into_any())?;
    tool.setattr("_grip", none(py))?;
    tool.setattr("_hover_grip", none(py))?;
    tool.setattr("_anchor", none(py))?;
    tool.setattr("_groups", PyList::empty(py).into_any())?;
    tool.setattr("_positions", PyList::empty(py).into_any())?;
    tool.setattr("_verts", PyList::empty(py).into_any())?;
    tool.setattr("_images", PyList::empty(py).into_any())?;
    tool.setattr("_box_version", 1.into_bound_py_any(py)?.neg()?)?;
    Ok(none(py))
}
fn scale_box_state<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if tool.getattr("_lo")?.is_none() {
        return Ok(none(py));
    }
    tool.getattr("_box")?.call_method1(
        "scale_preview",
        (
            tool.getattr("_factors")?,
            (if tool.getattr("_grip")?.is_truthy()? {
                tool.getattr("_anchor")?
            } else {
                none(py)
            }),
            (if tool.getattr("_grip")?.is_truthy()? {
                tool.getattr("_grip")?.getattr("params")?
            } else {
                none(py)
            }),
            (if tool.getattr("_hover_grip")?.is_truthy()? {
                tool.getattr("_hover_grip")?.getattr("params")?
            } else {
                none(py)
            }),
        ),
    )
}
fn selection_bounds<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let [tool, viewport] = unpack::<2>(a.as_any())?;
    tool.call_method1("_pick_frame", (&viewport,))?;
    let axes = tool.getattr("_axes")?;
    let frame = if axes.is_truthy()? {
        let points = PyList::new(py, [tool.getattr("_orig")?.call_method0("toTuple")?])?;
        for axis in axes.try_iter()? {
            points.append(axis?.call_method0("toTuple")?)?;
        }
        points.into_any()
    } else {
        none(py)
    };
    let bounds = imported(py, "tools.rust_tool_base")?
        .getattr("selection_bounds")?
        .call(
            (viewport.getattr("scene")?, frame),
            Some(&fields!(py; "images"=>true)),
        )?;
    tool.setattr("_box", &bounds)?;
    bounds.call_method0("limits")
}

fn active_axes<'py>(a: &Args<'py>) -> Reply<'py> {
    let ext: [f64; 3] = a.get_item(0)?.call_method0("_extents")?.extract()?;
    let flat: f64 = a
        .py()
        .import("tools.scale_host")?
        .getattr("_FLAT")?
        .extract()?;
    ext.iter()
        .filter(|&&x| x > flat)
        .count()
        .into_bound_py_any(a.py())
}

fn pick_frame<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let [tool, viewport] = unpack::<2>(a.as_any())?;
    let group = imported(py, "built_core.group")?;
    let axes = imported(py, "built_core.axes")?;
    let selection = viewport
        .getattr("scene")?
        .getattr("selection")?
        .try_iter()?
        .collect::<PyResult<Vec<_>>>()?;
    let mut frame = none(py);
    if let [only] = selection.as_slice() {
        if only.is_instance(&group.getattr("Group")?)? {
            frame = group.call_method1("group_frame", (only,))?;
        }
    }
    if frame.is_none() && !axes.call_method0("is_world")?.is_truthy()? {
        let directions = axes.getattr("AXES")?;
        frame = axes.call_method1(
            "frame_matrix",
            (
                axes.getattr("ORIGIN")?,
                directions.get_item("x")?,
                directions.get_item("y")?,
                directions.get_item("z")?,
            ),
        )?;
    }
    if !frame.is_none() {
        let [origin, x, y, z] = unpack::<4>(&group.call_method1("frame_axes", (frame,))?)?;
        use glam::DVec3;
        if crate::math::xyz(&x)? != DVec3::X
            || crate::math::xyz(&y)? != DVec3::Y
            || crate::math::xyz(&z)? != DVec3::Z
        {
            tool.setattr("_axes", (x, y, z))?;
            tool.setattr("_orig", origin)?;
            return Ok(none(py));
        }
    }
    tool.setattr("_axes", py.None())?;
    tool.setattr("_orig", py.None())?;
    Ok(none(py))
}

fn anchor_for<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let [tool, grip] = unpack::<2>(a.as_any())?;
    if tool.getattr("about_center")?.is_truthy()? {
        return tool
            .getattr("_box")?
            .call_method1("grip", ((0.5, 0.5, 0.5),));
    }
    let mask = grip.getattr("mask")?;
    let mut params: [f64; 3] = grip.getattr("params")?.extract()?;
    for (i, value) in params.iter_mut().enumerate() {
        *value = if mask.contains(i)? { 1.0 - *value } else { 0.5 };
    }
    let anchor = imported(py, "tools.scale_host")?
        .call_method1("_Grip", (PyTuple::new(py, params)?, mask))?;
    tool.call_method1("_grip_pos", (anchor,))
}

fn factors_from_cursor<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let [tool, vp, x, y] = unpack::<4>(a.as_any())?;
    let [origin, direction] = unpack::<2>(&tool.call_method1("_ray", (vp, x, y))?)?;
    if origin.is_none() {
        return Ok(none(py));
    }
    let axes = tool.getattr("_axes")?;
    let ray = if axes.is_none() || tool.call_method0("_line_grip")?.is_truthy()? {
        direction.call_method0("toTuple")?
    } else {
        let direction = crate::math::xyz(&direction)?;
        let [x, y, z] = unpack::<3>(&axes)?;
        (
            direction.dot(crate::math::xyz(&x)?),
            direction.dot(crate::math::xyz(&y)?),
            direction.dot(crate::math::xyz(&z)?),
        )
            .into_bound_py_any(py)?
    };
    let grip = tool.call_method1("_grip_pos", (tool.getattr("_grip")?,))?;
    tool.call_method0("_grip_spec")?.call_method1(
        "ray",
        (
            tool.call_method1("_relative", (grip,))?,
            tool.call_method1("_relative", (origin,))?,
            ray,
        ),
    )
}

fn pointer_event<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let [tool, kind, ctx] = unpack::<3>(a.as_any())?;
    tool.setattr("_ctx", &ctx)?;
    let sent = (|| {
        tool.call_method1(
            "_send",
            (
                ctx.getattr("viewport")?,
                tool.call_method1("_pointer", (kind, ctx))?,
            ),
        )
    })();
    tool.setattr("_ctx", py.None())?;
    sent?;
    Ok(none(py))
}

fn host_factors<'py>(a: &Args<'py>) -> Reply<'py> {
    let [tool, vp, xy] = unpack::<3>(a.as_any())?;
    let [x, y] = unpack::<2>(&xy)?;
    tool.call_method1("_factors_for", (tool.getattr("_ctx")?, vp, x, y))
}

fn host_hover_grip<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let [tool, vp, xy] = unpack::<3>(a.as_any())?;
    tool.call_method1("_refresh_box", (&vp,))?;
    let hover = if tool.getattr("_lo")?.is_none() {
        none(py)
    } else {
        let [x, y] = unpack::<2>(&xy)?;
        tool.call_method1("_grip_under", (&vp, x, y))?
    };
    if !hover.is(&tool.getattr("_hover_grip")?) {
        tool.setattr("_hover_grip", hover)?;
        vp.call_method0("update")?;
    }
    Ok(none(py))
}

fn float_values<'py>(v: &Obj<'py>) -> PyResult<Option<Bound<'py, PyTuple>>> {
    let values = if let Ok(parts) = v.cast::<PyTuple>() {
        parts.clone()
    } else {
        PyTuple::new(v.py(), [v])?
    };
    Ok(values
        .iter()
        .all(|x| x.is_instance_of::<pyo3::types::PyFloat>())
        .then_some(values))
}
fn typed_factors<'py>(a: &Args<'py>) -> Reply<'py> {
    let [tool, value, absolute, mut spec] = unpack::<4>(a.as_any())?;
    let Some(values) = float_values(&value)? else {
        return Ok(none(a.py()));
    };
    if !spec.is_truthy()? {
        spec = tool.call_method0("_grip_spec")?;
    }
    spec.call_method1("typed", (values, absolute))
}
fn on_value<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let [tool, vp, mut value] = unpack::<3>(a.as_any())?;
    let absolute = value.is_instance_of::<PyTuple>()
        && value.len()? > 0
        && value.get_item(0)?.eq("abs_len")?;
    if absolute {
        value = value.get_item(1)?;
    }
    if float_values(&value)?.is_none() {
        return false.into_bound_py_any(py);
    }
    let payload =
        fields!(py; "type"=>if absolute {"absolute_value"} else {"value"}, "value"=>value);
    let result = tool.call_method1("_send", (&vp, payload))?;
    if result.get_item("redraw")?.is_truthy()? {
        vp.call_method0("update")?;
    }
    result.get_item("consumed")
}
fn vcb_caption<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let mut grip = tool.getattr("_grip")?;
    if !grip.is_truthy()? {
        grip = tool.getattr("_hover_grip")?;
    }
    if grip.is_none()
        || tool.getattr("uniform")?.is_truthy()?
        || grip
            .call_method1("kind", (tool.call_method0("_active_axes")?,))?
            .eq("corner")?
    {
        return "Scale".into_bound_py_any(py);
    }
    let names = imported(py, "tools.scale_host")?.getattr("_AXIS_NAMES")?;
    let labels = grip
        .getattr("mask")?
        .try_iter()?
        .map(|i| names.get_item(i?)?.extract::<String>())
        .collect::<PyResult<Vec<_>>>()?;
    format!("{} Scale", labels.join(", ")).into_bound_py_any(py)
}
fn value_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let grip = tool.getattr("_grip")?;
    if grip.is_none() {
        return Ok(none(py));
    }
    let mask = grip.getattr("mask")?;
    let factors = tool.getattr("_factors")?;
    let format = |i: Obj<'py>| {
        factors
            .get_item(i)?
            .call_method1("__format__", (".2f",))?
            .extract::<String>()
    };
    let text = if tool.getattr("uniform")?.is_truthy()?
        || grip
            .call_method1("kind", (tool.call_method0("_active_axes")?,))?
            .eq("corner")?
    {
        format(mask.get_item(0)?)?
    } else {
        mask.try_iter()?
            .map(|i| format(i?))
            .collect::<PyResult<Vec<_>>>()?
            .join("; ")
    };
    let pos = if tool.getattr("_lo")?.is_none() {
        tool.getattr("_anchor")?
    } else {
        tool.call_method1("_grip_pos", (grip,))?
    };
    (text, pos).into_bound_py_any(py)
}

fn grip_kind<'py>(a: &Args<'py>) -> Reply<'py> {
    let count = a.get_item(0)?.getattr("mask")?.len()?;
    let active = a.get_item(1)?.extract::<i64>()?;
    (if count as i64 >= active {
        "corner"
    } else if count == 2 {
        "edge"
    } else {
        "face"
    })
    .into_bound_py_any(a.py())
}

pub(super) const GRIP_METHODS: &[Method] = &[(
    "kind",
    Mode::Method,
    grip_kind,
    &["self", "active_axes:int"],
    "str",
    "",
)];

fn on_key_release<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let [tool, viewport, key] = unpack::<3>(a.as_any())?;
    if key.ne(imported(py, "tools.scale_host")?
        .getattr("Qt")?
        .getattr("Key_Control")?)?
    {
        return false.into_bound_py_any(py);
    }
    let tapped = match viewport.getattr_opt("ctrl_tapped")? {
        Some(call) => call.call0()?,
        None => true.into_bound_py_any(py)?,
    };
    let event = fields!(py;"type"=>"key_release","key"=>"Control","tapped"=>tapped);
    tool.call_method1("_send", (viewport, event))?
        .get_item("consumed")
}

#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("on_key_release", Mode::Method, on_key_release, &["self","viewport","key"], "", ""),
    ("uses_snap", Mode::Property, uses_snap, &["self"], "bool", "Only while a grip is held: then a corner, a midpoint or an edge\nof ANOTHER object sets the size — scale «to» it (issue #233,\n@ales-limon). Before the grab, the grips take the click, not the\ngeometry."),
    ("host_activate_box", Mode::Method, host_activate_box, &["self", "viewport", "_"], "None", ""),
    ("host_deactivate_box", Mode::Method, host_deactivate_box, &["self", "viewport", "_"], "None", ""),
    ("_selection_bounds", Mode::Method, selection_bounds, &["self", "viewport"], "", ""),
    ("_refresh_box", Mode::Method, refresh_box, &["self", "viewport"], "None", "(Re)build the grip box from the selection — cheap to call: keyed on\n``scene.version``, which selection changes already bump."),
    ("_extents", Mode::Method, extents, &["self"], "tuple", ""),
    ("_active_axes", Mode::Method, active_axes, &["self"], "int", ""),
    ("_pick_frame", Mode::Method, pick_frame, &["self", "viewport"], "None", "One group or component selected: its own axes. Anything else:\nthe drawing axes. The world frame is kept as ``None`` so every\nnumber is exactly what it was before the box could turn."),
    ("_grip_pos", Mode::Method, grip_pos, &["self", "grip:_Grip"], "QVector3D", ""),
    ("_anchor_for", Mode::Method, anchor_for, &["self", "grip:_Grip"], "QVector3D", "The point straight opposite the grip — or the box centre\nwhile About Center is toggled on."),
    ("_grip_under", Mode::Method, grip_under, &["self", "viewport", "sx:float", "sy:float"], "_Grip | None", "The grip under the cursor. Where two grips overlap on screen the\none in front wins — it is the one drawn on top (#833) — otherwise\nthe nearest on screen."),
    ("_ray", Mode::Method, ray, &["self", "viewport", "sx:float", "sy:float"], "", ""),
    ("snap_excluded", Mode::Method, snap_excluded, &["self"], "", "What is being scaled stays out of the snap engine, or the grip\nwould land on the very geometry it resizes. ``(edge ids, group\nids)`` as Move gives it."),
    ("_snapped_point", Mode::Method, snapped_point, &["self", "ctx"], "", ""),
    ("_grip_spec", Mode::Method, grip_spec, &["self"], "", ""),
    ("_line_grip", Mode::Method, line_grip, &["self"], "", ""),
    ("_relative", Mode::Method, relative, &["self", "point"], "", ""),
    ("_factors_from_point", Mode::Method, factors_from_point, &["self", "p"], "", ""),
    ("_factors_for", Mode::Method, factors_for, &["self", "ctx", "viewport", "sx:float", "sy:float"], "", ""),
    ("_factors_from_cursor", Mode::Method, factors_from_cursor, &["self", "viewport", "sx:float", "sy:float"], "", ""),
    ("_pointer_event", Mode::Method, pointer_event, &["self", "kind", "ctx"], "", ""),
    ("on_click", Mode::Method, on_click, &["self", "ctx"], "", ""),
    ("on_hover", Mode::Method, on_hover, &["self", "ctx"], "", ""),
    ("host_factors", Mode::Method, host_factors, &["self", "viewport", "xy"], "", ""),
    ("host_pick_grip", Mode::Method, host_pick_grip, &["self", "viewport", "xy"], "", ""),
    ("_grab", Mode::Method, grab, &["self", "viewport", "grip:_Grip", "screen_xy"], "None", ""),
    ("_capture_grip", Mode::Method, capture_grip, &["self", "viewport", "grip", "screen_xy"], "", ""),
    ("host_hover_grip", Mode::Method, host_hover_grip, &["self", "viewport", "xy"], "", ""),
    ("host_scale_preview", Mode::Method, host_scale_preview, &["self", "viewport", "factors"], "", ""),
    ("host_preview_factors", Mode::Method, host_preview_factors, &["self", "viewport", "_"], "", ""),
    ("on_release", Mode::Method, on_release, &["self", "viewport"], "", ""),
    ("host_toggle_center", Mode::Method, host_toggle_center, &["self", "viewport", "say"], "", ""),
    ("host_toggle_uniform", Mode::Method, host_toggle_uniform, &["self", "viewport", "_"], "", ""),
    ("on_key", Mode::Method, on_key, &["self", "viewport", "key", "modifiers"], "", ""),
    ("_reanchor", Mode::Method, reanchor, &["self", "viewport"], "None", "Ctrl mid-operation moves the anchor between centre and opposite\nside without losing the factors already dragged."),
    ("value_is_unitless", Mode::Method, value_is_unitless, &["self"], "bool", "A bare number is a FACTOR, never a length (#176); one typed\nwith a unit is the new size and still converts."),
    ("on_value", Mode::Method, on_value, &["self", "viewport", "value"], "bool", ""),
    ("_typed_factors", Mode::Method, typed_factors, &["self", "value", "absolute:bool", "spec=None"], "", ""),
    ("host_cancel_box", Mode::Method, host_cancel_box, &["self", "viewport", "_"], "None", ""),
    ("_capture_originals", Mode::Method, capture_originals, &["self", "viewport"], "", ""),
    ("_apply_preview", Mode::Method, apply_preview, &["self", "viewport", "target"], "", ""),
    ("_revert_preview", Mode::Method, revert_preview, &["self", "viewport"], "", ""),
    ("_commit", Mode::Method, commit, &["self", "viewport", "factors:tuple"], "None", ""),
    ("host_revert", Mode::Method, host_revert, &["self", "viewport", "_"], "", ""),
    ("host_grip_spec", Mode::Method, host_grip_spec, &["self", "viewport", "_"], "", ""),
    ("host_scale_targets", Mode::Method, host_scale_targets, &["self", "viewport", "_"], "", ""),
    ("host_end_grip", Mode::Method, host_end_grip, &["self", "viewport", "_"], "", ""),
    ("_reset", Mode::Method, reset, &["self"], "None", ""),
    ("vcb_caption", Mode::Method, vcb_caption, &["self"], "str", "Caption the Measurements box by the axes in play."),
    ("value_label", Mode::Method, value_label, &["self"], "", "The live factor readout beside the box: one number\nfor a uniform scale, one per axis otherwise."),
    ("scale_box_state", Mode::Method, scale_box_state, &["self"], "", "Everything the overlay needs: the (live-scaled) box segments and\neach grip with its role — ``idle`` green, ``hover``/``active``/\n``anchor`` red, as the official doc describes them."),
];
