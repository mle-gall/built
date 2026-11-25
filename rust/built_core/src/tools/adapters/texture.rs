// SPDX-License-Identifier: GPL-3.0-or-later
//! TexturePositionTool callbacks on the live document.
use super::*;

fn map<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok(if tool.getattr("active")?.is_truthy()? {
        tool.getattr("_placement")?.getattr("map")?
    } else {
        none(py)
    })
}
fn pin_uv<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok(if tool.getattr("active")?.is_truthy()? {
        tool.getattr("_placement")?.getattr("pin_uv")?
    } else {
        PyList::empty(py).into_any()
    })
}
fn gesture_active<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    Ok({
        let value = tool.getattr("active")?;
        if !value.is_truthy()? {
            value
        } else {
            tool.getattr("_placement")?.getattr("gesture_active")?
        }
    })
}
fn undo_depth<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok(if tool.getattr("active")?.is_truthy()? {
        tool.getattr("_placement")?.getattr("undo_depth")?
    } else {
        0.into_bound_py_any(py)?
    })
}
fn lifted<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok(if tool.getattr("active")?.is_truthy()? {
        tool.getattr("_placement")?.getattr("lifted")?
    } else {
        none(py)
    })
}
fn lifted_pos<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok(if tool.getattr("active")?.is_truthy()? {
        tool.getattr("_placement")?.getattr("lifted_pos")?
    } else {
        none(py)
    })
}
fn held<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok(if tool.getattr("active")?.is_truthy()? {
        tool.getattr("_placement")?.getattr("held")?
    } else {
        none(py)
    })
}
fn sweep_deg<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok(if tool.getattr("active")?.is_truthy()? {
        tool.getattr("_placement")?.getattr("sweep_deg")?
    } else {
        none(py)
    })
}
fn on_tick<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    Ok({
        let value = tool.getattr("active")?;
        if !value.is_truthy()? {
            value
        } else {
            tool.getattr("_placement")?.getattr("on_tick")?
        }
    })
}
fn side_texture<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let back: Obj<'py>;
    let face = a.get_item(0)?;
    let side = a.get_item(1)?;
    let tex: Obj<'py>;
    let attrs = {
        let value = optional(&face, "attrs")?;
        if value.is_truthy()? {
            value
        } else {
            PyDict::new(py).into_any()
        }
    };
    if side.eq("back")? {
        back = attrs.call_method1("get", ("back",))?;
        tex = if back.is_instance(&py.get_type::<pyo3::types::PyDict>().into_any())? {
            back.call_method1("get", ("texture",))?
        } else {
            none(py)
        };
    } else {
        tex = attrs.call_method1("get", ("texture",))?;
    }
    if !(tex.is_truthy()?) || !(tex.call_method1("get", ("path",))?.is_truthy()?) {
        return Ok(none(py));
    }
    Ok(tex.clone())
}
fn begin<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.texture_position")?;
    let at = a.get_item(3)?;
    let face = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let side = a.get_item(4)?;
    let viewport = a.get_item(1)?;
    let tex = tool.call_method1("side_texture", (&face, &side))?;
    if tex.is_none() {
        return false.into_bound_py_any(py);
    }
    tool.setattr("side", &side)?;
    let m = module
        .getattr("TextureMap")?
        .call_method1("from_face", (&face, &tex, &at))?;
    if m.is_none() {
        return false.into_bound_py_any(py);
    }
    tool.setattr("face", &face)?;
    tool.setattr(
        "_tex0",
        py.get_type::<pyo3::types::PyDict>().call1((&tex,))?,
    )?;
    let normal = face.call_method0("normal")?.call_method0("normalized")?;
    tool.setattr(
        "_plane",
        (
            module.call_method1("QVector3D", (face.getattr("vertices")?.get_item(0)?,))?,
            &normal,
        )
            .into_bound_py_any(py)?,
    )?;
    tool.setattr(
        "_placement",
        module.call_method1("TexturePlacement", (&m, &normal))?,
    )?;
    let suppress = optional(&viewport, "set_suppressed_faces")?;
    if !suppress.is_none() {
        suppress.call1((PySet::new(py, [face.clone()])?.into_any(),))?;
    }
    let flash = optional(&viewport, "flash_status")?;
    if !flash.is_none() {
        flash.call1((module.call_method1("tr", ("Drag the texture or a pin — red moves, green scales and rotates, blue scales and shears. Enter finishes.",))?, 6000,))?;
    }
    let update = optional(&viewport, "update")?;
    if !update.is_none() {
        update.call0()?;
    }
    true.into_bound_py_any(py)
}
fn active<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    (!tool.getattr("_placement")?.is_none()).into_bound_py_any(py)
}
fn dragging<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("active")
}
fn on_activate<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    Ok(none(py))
}
fn on_deactivate<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_leave", (&viewport,))?;
    Ok(none(py))
}
fn leave<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let suppress = optional(&viewport, "set_suppressed_faces")?;
    if !suppress.is_none() {
        suppress.call1((py.get_type::<pyo3::types::PySet>().call0()?,))?;
    }
    tool.setattr("face", none(py))?;
    tool.setattr("_placement", none(py))?;
    tool.setattr("_tex0", none(py))?;
    tool.setattr("_plane", none(py))?;
    Ok(none(py))
}
fn exit<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let set_tool: Obj<'py>;
    let viewport = a.get_item(1)?;
    tool.call_method1("_leave", (&viewport,))?;
    let win = if viewport.hasattr("window")? {
        viewport.call_method0("window")?
    } else {
        none(py)
    };
    let activate = optional(&win, "_activate_tool")?;
    if !activate.is_none() {
        activate.call1(("select",))?;
    } else {
        set_tool = optional(&viewport, "set_active_tool")?;
        if !set_tool.is_none() {
            set_tool.call1((py.None(),))?;
        }
    }
    let update = optional(&viewport, "update")?;
    if !update.is_none() {
        update.call0()?;
    }
    Ok(none(py))
}
fn pins<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok(if tool.getattr("active")?.is_truthy()? {
        tool.getattr("_placement")?.call_method0("pins")?
    } else {
        PyList::empty(py).into_any()
    })
}
fn on_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let direction: Obj<'py>;
    let origin: Obj<'py>;
    let screen = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if tool.getattr("_plane")?.is_none() || screen.is_none() {
        return Ok(none(py));
    }
    let ray = optional(&viewport, "_pixel_to_ray")?;
    let hit = optional(&viewport, "_ray_plane")?;
    if ray.is_none() || hit.is_none() {
        return Ok(none(py));
    }
    [origin, direction] =
        unpack::<2>(&ray.call1((screen.call_method0("x")?, screen.call_method0("y")?))?)?;
    if origin.is_none() {
        return Ok(none(py));
    }
    hit.call1((
        &origin,
        &direction,
        tool.getattr("_plane")?.get_item(0)?,
        tool.getattr("_plane")?.get_item(1)?,
    ))
}
fn on_face<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let screen = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let pick = optional(&viewport, "pick_face")?;
    if pick.is_none() || screen.is_none() {
        return true.into_bound_py_any(py);
    }
    (pick
        .call1((screen.call_method0("x")?, screen.call_method0("y")?))?
        .eq(tool.getattr("face")?)?)
    .into_bound_py_any(py)
}
fn on_click<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.texture_position")?;
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    if !tool.getattr("active")?.is_truthy()? {
        return Ok(none(py));
    }
    let vp = ctx.getattr("viewport")?;
    let project = optional(&vp, "_world_to_pixel")?;
    let action = tool.getattr("_placement")?.call_method1(
        "click",
        (
            (if !ctx.getattr("screen")?.is_none() {
                (
                    ctx.getattr("screen")?.call_method0("x")?,
                    ctx.getattr("screen")?.call_method0("y")?,
                )
                    .into_bound_py_any(py)?
            } else {
                none(py)
            }),
            tool.call_method1("_on_plane", (&vp, ctx.getattr("screen")?))?,
            (ctx.getattr("modifiers")?
                .bitand(module.getattr("Qt")?.getattr("ControlModifier")?)?
                .is_truthy()?)
            .into_bound_py_any(py)?,
            &project,
            tool.getattr("face")?,
            tool.getattr("GRAB_PX")?,
            tool.getattr("SNAP_PX")?,
        ),
    )?;
    if action.eq("update")? {
        vp.call_method0("update")?;
    } else {
        if action.eq("distort")? {
            vp.call_method1("flash_status", (module.call_method1("tr", ("Distort (perspective) is not available yet — use the red, green and blue pins.",))?, 4000,))?;
        } else if action.eq("pick_face")?
            && !(tool
                .call_method1("_on_face", (&vp, ctx.getattr("screen")?))?
                .is_truthy()?)
        {
            tool.call_method1("commit", (&vp,))?;
        }
    }
    Ok(none(py))
}
fn on_hover<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.texture_position")?;
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    let vp: Obj<'py>;
    if tool.getattr("active")?.is_truthy()? {
        vp = ctx.getattr("viewport")?;
        tool.getattr("_placement")?.call_method1(
            "hover",
            (
                (if !ctx.getattr("screen")?.is_none() {
                    (
                        ctx.getattr("screen")?.call_method0("x")?,
                        ctx.getattr("screen")?.call_method0("y")?,
                    )
                        .into_bound_py_any(py)?
                } else {
                    none(py)
                }),
                tool.call_method1("_on_plane", (&vp, ctx.getattr("screen")?))?,
                (ctx.getattr("modifiers")?
                    .bitand(module.getattr("Qt")?.getattr("ControlModifier")?)?
                    .is_truthy()?)
                .into_bound_py_any(py)?,
                optional(&vp, "_world_to_pixel")?,
                tool.getattr("face")?,
                tool.getattr("GRAB_PX")?,
                tool.getattr("SNAP_PX")?,
                tool.getattr("CLICK_PX")?,
                tool.getattr("TICK_DEG")?,
            ),
        )?;
        vp.call_method0("update")?;
    }
    Ok(none(py))
}
fn on_release<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.texture_position")?;
    let action: Obj<'py>;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if tool.getattr("active")?.is_truthy()? {
        action = tool.getattr("_placement")?.call_method0("release")?;
        if action.eq("lifted")? {
            viewport.call_method1(
                "flash_status",
                (
                    module.call_method1("tr", ("Pin lifted — click where it should sit.",))?,
                    3000,
                ),
            )?;
        } else if action.eq("update")? {
            viewport.call_method0("update")?;
        }
    }
    Ok(none(py))
}
fn value_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.texture_position")?;
    let tool = a.get_item(0)?;
    if tool.getattr("_sweep_deg")?.is_none() || !(tool.getattr("active")?.is_truthy()?) {
        return Ok(none(py));
    }
    let fmt_angle_deg = imported(py, "core.units")?.getattr("fmt_angle_deg")?;
    Ok((
        fmt_angle_deg.call(
            (tool.getattr("_sweep_deg")?,),
            Some(&fields!(py; "signed" => true)),
        )?,
        tool.getattr("map")?.call_method1(
            "world",
            positional(
                &tool
                    .getattr("pin_uv")?
                    .get_item(module.getattr("PIN_SCALE_ROTATE")?)?,
            )?,
        )?,
    )
        .into_bound_py_any(py)?)
}
fn protractor_segments<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if tool.getattr("active")?.is_truthy()? {
        return tool.getattr("_placement")?.call_method1(
            "protractor",
            (
                optional(&viewport, "_world_to_pixel")?,
                tool.getattr("DISC_PX")?,
            ),
        );
    }
    Ok(none(py))
}
fn rotate<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let degrees = a.get_item(1)?;
    let tool = a.get_item(0)?;
    if tool.getattr("active")?.is_truthy()? {
        tool.getattr("_placement")?
            .call_method1("rotate", (&degrees,))?;
    }
    Ok(none(py))
}
fn flip<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let horizontal = a.get_item(1)?;
    let tool = a.get_item(0)?;
    if tool.getattr("active")?.is_truthy()? {
        tool.getattr("_placement")?
            .call_method1("flip", (&horizontal,))?;
    }
    Ok(none(py))
}
fn undo_step<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    Ok({
        let value = tool.getattr("active")?;
        if !value.is_truthy()? {
            value
        } else {
            tool.getattr("_placement")?.call_method0("undo_step")?
        }
    })
}
fn reset<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if tool.getattr("active")?.is_truthy()? {
        tool.getattr("_placement")?.call_method0("reset")?;
    }
    Ok(none(py))
}
fn changed<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    Ok({
        let value = tool.getattr("active")?;
        if !value.is_truthy()? {
            value
        } else {
            tool.getattr("_placement")?.call_method0("changed")?
        }
    })
}
fn commit<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let tex: Obj<'py>;
    let viewport = a.get_item(1)?;
    if !tool.getattr("active")?.is_truthy()? {
        return Ok(none(py));
    }
    let preview = optional(&optional(&viewport, "view")?, "_texture_preview")?;
    if !preview.is_none() {
        preview.call_method0("restore")?;
    }
    if tool.call_method0("changed")?.is_truthy()? {
        tex = tool.call_method0("result_texture")?;
        if !tex.is_none() {
            viewport.getattr("history")?.call_method1(
                "execute",
                (tool.call_method1(
                    "side_command",
                    (tool.getattr("face")?, tool.getattr("side")?, &tex),
                )?,),
            )?;
        }
    }
    tool.call_method1("_exit", (&viewport,))?;
    Ok(none(py))
}
fn side_command<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let back: Obj<'py>;
    let face = a.get_item(0)?;
    let side = a.get_item(1)?;
    let tex = a.get_item(2)?;
    if side.eq("back")? {
        let set_face_back_command = imported(py, "built_core.history")?.getattr("SetFaceBackCommand")?;
        back = py.get_type::<pyo3::types::PyDict>().call1((({
            let value = face.getattr("attrs")?.call_method1("get", ("back",))?;
            if value.is_truthy()? {
                value
            } else {
                PyDict::new(py).into_any()
            }
        }),))?;
        if tex.is_none() {
            back.call_method1("pop", ("texture", py.None()))?;
        } else {
            back.set_item(
                "texture",
                py.get_type::<pyo3::types::PyDict>().call1((&tex,))?,
            )?;
        }
        return set_face_back_command.call1((
            PyList::new(py, [face.clone()])?.into_any(),
            PyList::new(py, [back.clone()])?.into_any(),
        ));
    }
    let set_face_texture_command =
        imported(py, "built_core.history")?.getattr("SetFaceTextureCommand")?;
    set_face_texture_command.call1((PyList::new(py, [face.clone()])?.into_any(), &tex))
}
fn cancel<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_exit", (&viewport,))?;
    Ok(none(py))
}
fn on_cancel<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.texture_position")?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if !tool.getattr("active")?.is_truthy()? {
        return Ok(none(py));
    }
    let action = tool.getattr("_placement")?.call_method0("escape")?;
    if action.eq("exit")? {
        tool.call_method1("cancel", (&viewport,))?;
    } else {
        if action.eq("restored")? {
            viewport.call_method1(
                "flash_status",
                (
                    module.call_method1("tr", ("Position restored — Esc again to leave.",))?,
                    3000,
                ),
            )?;
        }
        viewport.call_method0("update")?;
    }
    Ok(none(py))
}
fn on_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.texture_position")?;
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if !tool.getattr("active")?.is_truthy()? {
        return false.into_bound_py_any(py);
    }
    if (
        module.getattr("Qt")?.getattr("Key_Return")?,
        module.getattr("Qt")?.getattr("Key_Enter")?,
    )
        .into_bound_py_any(py)?
        .contains(&key)?
    {
        tool.call_method1("commit", (&viewport,))?;
        return true.into_bound_py_any(py);
    }
    false.into_bound_py_any(py)
}
fn menu_action<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if !tool.getattr("active")?.is_truthy()? {
        return false.into_bound_py_any(py);
    }
    if key.eq("done")? {
        tool.call_method1("commit", (&viewport,))?;
    } else if !(tool
        .getattr("_placement")?
        .call_method1("menu_action", (&key,))?
        .is_truthy()?)
    {
        return false.into_bound_py_any(py);
    }
    viewport.call_method0("update")?;
    true.into_bound_py_any(py)
}
fn preview_faces<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if !tool.getattr("active")?.is_truthy()? {
        return Ok(PyList::empty(py).into_any());
    }
    let tex = tool.call_method0("result_texture")?;
    if tex.is_none() {
        return Ok(PyList::empty(py).into_any());
    }
    let preview_face = imported(py, "built_core.geometry")?.getattr("Face")?;
    return Ok(PyList::new(
        py,
        [preview_face.call1((
            py.get_type::<pyo3::types::PyList>()
                .call1((tool.getattr("face")?.getattr("vertices")?,))?,
            ({
                let items = PyList::empty(py);
                for item in tool.getattr("face")?.getattr("holes")?.try_iter()? {
                    let h = item?;
                    items.append(py.get_type::<pyo3::types::PyList>().call1((&h,))?)?;
                }
                items.into_any()
            }),
            fields!(py; "texture" => &tex).into_any(),
        ))?],
    )?
    .into_any());
}
fn grid_lines<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok(if tool.getattr("active")?.is_truthy()? {
        tool.getattr("_placement")?
            .call_method1("grid_lines", (tool.getattr("face")?.getattr("vertices")?,))?
    } else {
        PyList::empty(py).into_any()
    })
}
fn result_texture<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let map = tool.getattr("map")?;
    let original = tool.getattr("_tex0")?;
    if map.is_none() || original.is_none() {
        return Ok(none(py));
    }
    let uvw = map.call_method0("uvw")?;
    if uvw.is_none() {
        return Ok(none(py));
    }
    let texture = PyDict::new(py);
    for entry in original.call_method0("items")?.try_iter()? {
        let [key, value] = unpack::<2>(&entry?)?;
        if !key.eq("rot")? {
            texture.set_item(key, value)?;
        }
    }
    texture.set_item("uvw", uvw)?;
    Ok(texture.into_any())
}

#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("map", Mode::Property, map, &["self"], "", ""),
    ("pin_uv", Mode::Property, pin_uv, &["self"], "", ""),
    ("gesture_active", Mode::Property, gesture_active, &["self"], "", ""),
    ("undo_depth", Mode::Property, undo_depth, &["self"], "", ""),
    ("_lifted", Mode::Property, lifted, &["self"], "", ""),
    ("_lifted_pos", Mode::Property, lifted_pos, &["self"], "", ""),
    ("_held", Mode::Property, held, &["self"], "", ""),
    ("_sweep_deg", Mode::Property, sweep_deg, &["self"], "", ""),
    ("_on_tick", Mode::Property, on_tick, &["self"], "", ""),
    ("side_texture", Mode::Static, side_texture, &["face", "side:str"], "", "The image texture dict of ``face``'s ``side`` (\"front\"/\"back\"),\nor ``None``. A back painted as a mirror of the front (``True``)\nhas no texture of its own."),
    ("begin", Mode::Method, begin, &["self", "viewport", "face", "at:QVector3D | None=None", "side:str='front'"], "bool", "Start positioning the texture of ``face``'s ``side`` with the\npins on the tile under ``at``. Returns False when that side has no\nimage texture. The back side projects with the same plane basis as\nthe front (the renderer builds both from the face normal), so one\nmap serves either; only where it is stored differs."),
    ("active", Mode::Property, active, &["self"], "bool", ""),
    ("dragging", Mode::Property, dragging, &["self"], "bool", ""),
    ("on_activate", Mode::Method, on_activate, &["self", "viewport"], "None", "The picked face enters placement through begin()."),
    ("on_deactivate", Mode::Method, on_deactivate, &["self", "viewport"], "None", ""),
    ("_leave", Mode::Method, leave, &["self", "viewport"], "None", ""),
    ("_exit", Mode::Method, exit, &["self", "viewport"], "None", "Back to Select (the classic tool returns to the previous one)."),
    ("pins", Mode::Method, pins, &["self"], "list[QVector3D]", ""),
    ("_on_plane", Mode::Method, on_plane, &["self", "viewport", "screen"], "QVector3D | None", "The cursor's point on the face plane."),
    ("_on_face", Mode::Method, on_face, &["self", "viewport", "screen"], "bool", ""),
    ("on_click", Mode::Method, on_click, &["self", "ctx:ToolContext"], "None", ""),
    ("on_hover", Mode::Method, on_hover, &["self", "ctx:ToolContext"], "None", ""),
    ("on_release", Mode::Method, on_release, &["self", "viewport"], "None", ""),
    ("value_label", Mode::Method, value_label, &["self"], "", "``(text, anchor)`` while the green pin rotates — the angle swept,\nas the Measurements box shows it; ``None`` otherwise."),
    ("_protractor_segments", Mode::Method, protractor_segments, &["self", "viewport"], "", ""),
    ("rotate", Mode::Method, rotate, &["self", "degrees:float"], "None", ""),
    ("flip", Mode::Method, flip, &["self", "horizontal:bool"], "None", ""),
    ("undo_step", Mode::Method, undo_step, &["self"], "bool", ""),
    ("reset", Mode::Method, reset, &["self"], "None", ""),
    ("changed", Mode::Method, changed, &["self"], "bool", ""),
    ("result_texture", Mode::Method, result_texture, &["self"], "dict | None", "The texture dict the face gets on Done."),
    ("commit", Mode::Method, commit, &["self", "viewport"], "None", "Done: write the map to the face as one undoable step."),
    ("side_command", Mode::Static, side_command, &["face", "side:str", "tex:dict | None"], "", "The undoable command that puts ``tex`` on ``face``'s ``side``\n(``None`` clears the texture of that side)."),
    ("cancel", Mode::Method, cancel, &["self", "viewport"], "None", ""),
    ("on_cancel", Mode::Method, on_cancel, &["self", "viewport"], "None", "Esc: the first press restores the entry placement and stays; a\nsecond one (or one with nothing to restore) leaves."),
    ("on_key", Mode::Method, on_key, &["self", "viewport", "key:int", "modifiers"], "bool", ""),
    ("menu_action", Mode::Method, menu_action, &["self", "viewport", "key"], "", ""),
    ("preview_faces", Mode::Method, preview_faces, &["self"], "", ""),
    ("grid_lines", Mode::Method, grid_lines, &["self"], "list[tuple[QVector3D, QVector3D]]", ""),
];
