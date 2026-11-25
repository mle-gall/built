// SPDX-License-Identifier: GPL-3.0-or-later
//! FilletHost callbacks on the live document.
use super::*;

fn corners_mode<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    (tool.getattr("affect")?.eq("vertices")?).into_bound_py_any(py)
}
fn host_reset_bevel<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.fillet_host")?;
    let activate = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.setattr("_viewport", &viewport)?;
    tool.call_method0("_reset")?;
    if !activate.is_truthy()? {
        if viewport.hasattr("set_hover")? {
            viewport.call_method1("set_hover", (py.None(),))?;
        }
        return Ok(none(py));
    }
    let scene = optional(&viewport, "scene")?;
    let sel = {
        let value = optional(&scene, "selection")?;
        if value.is_truthy()? {
            value
        } else {
            PyTuple::empty(py).into_any()
        }
    };
    let mesh = optional(&scene, "mesh")?;
    let picked = module.call_method1(
        "bevel_selection",
        (&sel, &mesh, tool.call_method0("_corners_mode")?),
    )?;
    if picked.is_truthy()? && tool.call_method0("_corners_mode")?.is_truthy()? {
        tool.setattr("edges", &picked)?;
        viewport.call_method1("flash_status", (module.call_method("tr", ("{n} corner(s) selected — type the size, or click and move the cursor to set it.",), Some(&fields!(py; "n" => picked.len()?.into_bound_py_any(py)?)))?, 5000,))?;
    } else {
        if picked.is_truthy()? {
            tool.setattr("edges", &picked)?;
            viewport.call_method1("flash_status", (module.call_method("tr", ("{n} edge(s) selected — type the size, or click and move the cursor to set it.",), Some(&fields!(py; "n" => picked.len()?.into_bound_py_any(py)?)))?, 5000,))?;
        } else {
            viewport.call_method1(
                "flash_status",
                (
                    module.call_method1(
                        "tr",
                        ("Click an edge to round or bevel it; type the size and Enter.",),
                    )?,
                    4000,
                ),
            )?;
        }
    }
    tool.call_method0("_replan")?;
    Ok(none(py))
}
fn reset<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    tool.setattr("edges", PyList::empty(py).into_any())?;
    tool.setattr("hover_edge", none(py))?;
    tool.setattr("hover_vertex", none(py))?;
    tool.setattr("hover_point", none(py))?;
    tool.setattr("_plan", none(py))?;
    tool.setattr("_plan_key", none(py))?;
    tool.setattr("_message", none(py))?;
    Ok(none(py))
}
fn start_point<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.fillet_host")?;
    let tool = a.get_item(0)?;
    if !tool.getattr("edges")?.is_truthy()? {
        return Ok(none(py));
    }
    let first = tool.getattr("edges")?.get_item(0)?;
    Ok(if first.is_instance(&module.getattr("Edge")?)? {
        first.getattr("a")?
    } else {
        first.getattr("position")?
    })
}
fn host_pick_bevel<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.fillet_host")?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let world = a.get_item(2)?;
    tool.setattr("_viewport", &vp)?;
    let point = if !world.is_none() {
        module.call_method1("QVector3D", positional(&world)?)?
    } else {
        none(py)
    };
    if !tool.getattr("edges")?.is_truthy()? {
        tool.setattr(
            "edges",
            module.call_method1(
                "bevel_targets",
                (&vp, tool.call_method0("_corners_mode")?, &point),
            )?,
        )?;
    }
    if tool.getattr("edges")?.is_truthy()? {
        tool.setattr("hover_point", &point)?;
    }
    (tool.getattr("edges")?.is_truthy()?).into_bound_py_any(py)
}
fn host_hover_bevel<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.fillet_host")?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    if tool.getattr("edges")?.is_truthy()? {
        return Ok(none(py));
    }
    let edge = optional(&vp, "_hover_edge")?;
    tool.setattr(
        "hover_edge",
        if edge.is_instance(&module.getattr("Edge")?)? {
            edge.clone()
        } else {
            none(py)
        },
    )?;
    tool.setattr(
        "hover_vertex",
        if !tool.getattr("hover_edge")?.is_none() {
            tool.call_method1(
                "_nearest_end",
                (tool.getattr("hover_edge")?, tool.getattr("hover_point")?),
            )?
        } else {
            none(py)
        },
    )?;
    if vp.hasattr("set_hover")? {
        vp.call_method1("set_hover", (tool.getattr("hover_edge")?,))?;
    }
    tool.call_method0("_replan")?;
    Ok(none(py))
}
fn on_hover<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.setattr("_viewport", ctx.getattr("viewport")?)?;
    tool.setattr("hover_point", ctx.getattr("world")?)?;
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
fn host_bevel_geometry<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.fillet_host")?;
    let tool = a.get_item(0)?;
    return Ok({
        let items = PyList::empty(py);
        for item in tool.getattr("edges")?.try_iter()? {
            let e = item?;
            items.append(if e.is_instance(&module.getattr("Edge")?)? {
                PyList::new(
                    py,
                    [
                        e.getattr("a")?.call_method0("toTuple")?,
                        e.getattr("b")?.call_method0("toTuple")?,
                    ],
                )?
                .into_any()
            } else {
                PyList::new(py, [e.getattr("position")?.call_method0("toTuple")?])?.into_any()
            })?;
        }
        items.into_any()
    });
}
fn host_replan<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    tool.call_method0("_replan")?;
    Ok(none(py))
}
fn host_bevel_picked<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    (tool.getattr("edges")?.is_truthy()?).into_bound_py_any(py)
}
fn on_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.fillet_host")?;
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    Ok({
        let value =
            (key.eq(module.getattr("Qt")?.getattr("Key_Control")?)?).into_bound_py_any(py)?;
        if !value.is_truthy()? {
            value
        } else {
            tool.call_method1(
                "_send",
                (
                    &viewport,
                    fields!(py; "type" => "key", "key" => "Control").into_any(),
                ),
            )?
            .get_item("consumed")?
        }
    })
}
fn set_affect<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.fillet_host")?;
    let affect = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if !("edges", "vertices")
        .into_bound_py_any(py)?
        .contains(&affect)?
        || affect.eq(tool.getattr("affect")?)?
    {
        return Ok(none(py));
    }
    tool.get_type().into_any().setattr("affect", &affect)?;
    let mesh = optional(&optional(&viewport, "scene")?, "mesh")?;
    if affect.eq("vertices")? {
        tool.get_type().into_any().setattr("shape", none(py))?;
    }
    if affect.eq("vertices")? && tool.getattr("edges")?.is_truthy()? {
        tool.setattr(
            "edges",
            module.call_method1("bevel_selection", (tool.getattr("edges")?, &mesh, true))?,
        )?;
    } else if affect.eq("edges")? {
        tool.setattr("edges", PyList::empty(py).into_any())?;
        tool.call_method1(
            "_send",
            (
                &viewport,
                fields!(py; "type" => "key", "key" => "AffectEdges").into_any(),
            ),
        )?;
    }
    tool.setattr("hover_vertex", none(py))?;
    viewport.call_method1("flash_status", (module.call_method("tr", ("Affect: {a}",), Some(&fields!(py; "a" => module.call_method1("tr", ((if affect.eq("vertices")? { ("Vertices").into_bound_py_any(py)? } else { ("Edges").into_bound_py_any(py)? }),))?)))?,))?;
    tool.call_method0("_replan")?;
    tool.call_method1("_refresh_readouts", (&viewport,))?;
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn refresh_readouts<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let viewport = a.get_item(1)?;
    let hint = optional(&viewport, "refresh_status_hint")?;
    if !hint.is_none() {
        hint.call0()?;
    }
    let window = optional(&viewport, "window")?;
    let vcb = if window.is_callable() {
        optional(&window.call0()?, "_refresh_vcb")?
    } else {
        none(py)
    };
    if !vcb.is_none() {
        vcb.call0()?;
    }
    Ok(none(py))
}
fn host_bevel_measure<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok(if tool.call_method0("_corners_mode")?.is_truthy()? {
        none(py)
    } else {
        tool.getattr("measure")?
    })
}
fn host_measure<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let measure = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("set_measure", (&viewport, &measure))?;
    Ok(none(py))
}
fn set_measure<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.fillet_host")?;
    let measure = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if !py
        .get_type::<pyo3::types::PyDict>()
        .call1((tool.getattr("_MEASURES")?,))?
        .contains(&measure)?
    {
        return Ok(none(py));
    }
    tool.get_type().into_any().setattr("measure", &measure)?;
    viewport.call_method1("flash_status", (module.call_method("tr", ("Measure: {m}",), Some(&fields!(py; "m" => module.call_method1("tr", (tool.call_method0("_measure_label")?,))?)))?,))?;
    tool.call_method0("_replan")?;
    tool.call_method1("_refresh_readouts", (&viewport,))?;
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn set_size<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let value = a.get_item(2)?;
    let viewport = a.get_item(1)?;
    tool.get_type().into_any().setattr(
        "radius",
        py.get_type::<pyo3::types::PyFloat>().call1((&value,))?,
    )?;
    tool.call_method0("_replan")?;
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn set_corners<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.fillet_host")?;
    let corners = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if !("round", "cut").into_bound_py_any(py)?.contains(&corners)? {
        return Ok(none(py));
    }
    tool.get_type().into_any().setattr("corners", &corners)?;
    viewport.call_method1("flash_status", (module.call_method("tr", ("Corners: {c}",), Some(&fields!(py; "c" => module.call_method1("tr", ((if corners.eq("round")? { ("Fill (triangle / cap)").into_bound_py_any(py)? } else { ("No fill (notch)").into_bound_py_any(py)? }),))?)))?,))?;
    tool.call_method0("_replan")?;
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn apply_from_panel<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.fillet_host")?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    if !tool.call_method0("_current_edges")?.is_truthy()? {
        viewport.call_method1(
            "flash_status",
            (
                module.call_method1(
                    "tr",
                    ("Click an edge to round or bevel it; type the size and Enter.",),
                )?,
                4000,
            ),
        )?;
        return Ok(none(py));
    }
    tool.call_method1(
        "_send",
        (
            &viewport,
            fields!(py; "type" => "key", "key" => "Apply").into_any(),
        ),
    )?;
    Ok(none(py))
}
fn measure_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    py.get_type::<pyo3::types::PyDict>()
        .call1((tool.getattr("_MEASURES")?,))?
        .call_method1("get", (tool.getattr("measure")?, "Radius"))
}
fn vcb_caption<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok(if tool.call_method0("_corners_mode")?.is_truthy()? {
        ("Distance").into_bound_py_any(py)?
    } else {
        tool.call_method0("_measure_label")?
    })
}
fn status_clause<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.fillet_host")?;
    let tool = a.get_item(0)?;
    if tool.call_method0("_corners_mode")?.is_truthy()? {
        return module.call_method1("tr", ("Vertices: size along the edges",));
    }
    Ok(module.call_method("tr", ("Ctrl = measure ({m})",), Some(&fields!(py; "m" => module.call_method1("tr", (tool.call_method0("_measure_label")?,))?)))?)
}
fn on_value<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let value = a.get_item(2)?;
    let viewport = a.get_item(1)?;
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
fn on_segments_value<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let n = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let answer = tool.call_method1("_send", (&viewport, fields!(py; "type" => "segments", "count" => py.get_type::<pyo3::types::PyInt>().call1((&n,))?).into_any(),))?;
    if answer.get_item("redraw")?.is_truthy()? {
        viewport.call_method0("update")?;
    }
    answer.get_item("consumed")
}
fn on_profile_value<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let p = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let answer = tool.call_method1(
        "_send",
        (
            &viewport,
            fields!(py; "type" => "profile", "value" => &p).into_any(),
        ),
    )?;
    if answer.get_item("redraw")?.is_truthy()? {
        viewport.call_method0("update")?;
    }
    answer.get_item("consumed")
}
fn host_bad_profile<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.fillet_host")?;
    let viewport = a.get_item(1)?;
    viewport.call_method1(
        "flash_status",
        (module.call_method1("tr", ("The profile must be between 0 and 1.",))?,),
    )?;
    Ok(none(py))
}
fn host_announce_size<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.fillet_host")?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    viewport.call_method1("flash_status", (module.call_method("tr", ("{m} {r} — click an edge.",), Some(&fields!(py; "m" => module.call_method1("tr", (tool.call_method0("_measure_label")?,))?, "r" => module.call_method1("fmt_len_fine", (tool.getattr("radius")?,))?)))?, 3000,))?;
    Ok(none(py))
}
fn current_edges<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if tool.getattr("edges")?.is_truthy()? {
        return tool.getattr("edges");
    }
    if tool.call_method0("_corners_mode")?.is_truthy()? {
        return Ok(if !tool.getattr("hover_vertex")?.is_none() {
            PyList::new(py, [tool.getattr("hover_vertex")?])?.into_any()
        } else {
            PyList::empty(py).into_any()
        });
    }
    Ok(if !tool.getattr("hover_edge")?.is_none() {
        PyList::new(py, [tool.getattr("hover_edge")?])?.into_any()
    } else {
        PyList::empty(py).into_any()
    })
}
fn host_fillet_plan<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.fillet_host")?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method0("_replan")?;
    if tool.getattr("_plan")?.is_none() {
        viewport.call_method1(
            "flash_status",
            (
                ({
                    let value = tool.getattr("_message")?;
                    if value.is_truthy()? {
                        value
                    } else {
                        module.call_method1("tr", ("Nothing to round.",))?
                    }
                }),
                4000,
            ),
        )?;
        return Ok(none(py));
    }
    let to_json_key = imported(py, "core.protocol.keys")?.getattr("to_json_key")?;
    let kind = if tool.call_method0("_corners_mode")?.is_truthy()? {
        ("vertices").into_bound_py_any(py)?
    } else {
        ("edges").into_bound_py_any(py)?
    };
    return Ok(fields!(py; &kind => {let items=PyList::empty(py); for item in tool.call_method0("_current_edges")?.try_iter()? { let e = item?;
items.append(to_json_key.call1((e.getattr("key")?,))?)?;
} items.into_any()}, "radius" => tool.getattr("radius")?, "segments" => tool.getattr("segments")?, "profile" => tool.getattr("profile")?, "measure" => tool.getattr("measure")?, "corners" => tool.getattr("corners")?, "shape" => tool.getattr("shape")?).into_any());
}
fn host_finish_bevel<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.fillet_host")?;
    let n: Obj<'py>;
    let r: Obj<'py>;
    let said: Obj<'py>;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    [n, r] = unpack::<2>(
        &(
            tool.call_method0("_current_edges")?
                .len()?
                .into_bound_py_any(py)?,
            tool.getattr("radius")?,
        )
            .into_bound_py_any(py)?,
    )?;
    viewport.getattr("scene")?.call_method0("clear_selection")?;
    tool.call_method0("_reset")?;
    let set_hover = optional(&viewport, "set_hover")?;
    if !set_hover.is_none() {
        set_hover.call1((py.None(),))?;
    }
    if tool.call_method0("_corners_mode")?.is_truthy()? {
        said = module.call_method(
            "tr",
            ("Bevelled {n} corner(s), distance {r}.",),
            Some(&fields!(py; "n" => &n, "r" => module.call_method1("fmt_len_fine", (&r,))?)),
        )?;
    } else {
        if tool.call_method0("_is_plain_fillet")?.is_truthy()? {
            said = module.call_method(
                "tr",
                ("Rounded {n} edge(s), radius {r}.",),
                Some(&fields!(py; "n" => &n, "r" => module.call_method1("fmt_len_fine", (&r,))?)),
            )?;
        } else {
            said = module.call_method("tr", ("Bevelled {n} edge(s), {m} {r}.",), Some(&fields!(py; "n" => &n, "m" => module.call_method1("tr", (tool.call_method0("_measure_label")?,))?, "r" => module.call_method1("fmt_len_fine", (&r,))?)))?;
        }
    }
    viewport.call_method1("flash_status", (&said, 3000))?;
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn preview_faces<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if tool.getattr("_plan")?.is_none() {
        return Ok(PyList::empty(py).into_any());
    }
    let preview_face = imported(py, "core.geometry")?.getattr("Face")?;
    return Ok({
        let items = PyList::empty(py);
        for item in tool.getattr("_plan")?.getattr("new_faces")?.try_iter()? {
            let ring = item?;
            items.append(
                preview_face.call1((py.get_type::<pyo3::types::PyList>().call1((&ring,))?,))?,
            )?;
        }
        items.into_any()
    });
}
fn shape_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if tool
        .getattr("shape")?
        .is_instance(&py.get_type::<pyo3::types::PyTuple>().into_any())?
    {
        return ("Custom (draw)").into_bound_py_any(py);
    }
    py.get_type::<pyo3::types::PyDict>()
        .call1((tool.getattr("_SHAPES")?,))?
        .call_method1("get", (tool.getattr("shape")?, "Curve (value 0–1)"))
}
fn is_plain_fillet<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok({
        let value = (tool.getattr("measure")?.eq("radius")?).into_bound_py_any(py)?;
        if !value.is_truthy()? {
            value
        } else {
            {
                let value = (tool.getattr("shape")?.is_none()).into_bound_py_any(py)?;
                if !value.is_truthy()? {
                    value
                } else {
                    (tool
                        .getattr("profile")?
                        .sub(0.5)?
                        .abs()?
                        .lt(1e-09)?)
                    .into_bound_py_any(py)?
                }
            }
        }
    })
}

fn on_key_release<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.fillet_host")?;
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    Ok({
        let value =
            (key.eq(module.getattr("Qt")?.getattr("Key_Control")?)?).into_bound_py_any(py)?;
        if !value.is_truthy()? {
            value
        } else {
            tool.call_method1("_send", (&viewport, fields!(py; "type" => "key_release", "key" => "Control", "tapped" => if let Some(call)=viewport.getattr_opt("ctrl_tapped")? {call.call0()?} else {true.into_bound_py_any(py)?}).into_any(),))?.get_item("consumed")?
        }
    })
}
fn set_shape<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.fillet_host")?;
    let tool = a.get_item(0)?;
    let mut shape = a.get_item(2)?;
    let mut sm: Obj<'py>;
    let viewport = a.get_item(1)?;
    let mut x: Obj<'py>;
    let mut y: Obj<'py>;
    let bevel_profiles = imported(py, "ingetrazo_core.bevel_profiles")?.into_any();
    if imported(py, "builtins")?
        .getattr("isinstance")?
        .call1((
            &shape,
            (
                py.get_type::<pyo3::types::PyList>().into_any(),
                py.get_type::<pyo3::types::PyTuple>().into_any(),
            )
                .into_bound_py_any(py)?,
        ))?
        .is_truthy()?
    {
        if !(bevel_profiles
            .call_method1("valid", (&shape,))?
            .is_truthy()?)
        {
            return Ok(none(py));
        }
        shape = py.get_type::<pyo3::types::PyTuple>().call1(({
            let collected = PyList::empty(py);
            for item in shape.try_iter()? {
                {
                    let [part0, part1, part2] = unpack::<3>(&item?)?;
                    x = part0;
                    y = part1;
                    sm = part2;
                }
                collected.append(
                    (
                        py.get_type::<pyo3::types::PyFloat>().call1((&x,))?,
                        py.get_type::<pyo3::types::PyFloat>().call1((&y,))?,
                        (sm.is_truthy()?).into_bound_py_any(py)?,
                    )
                        .into_bound_py_any(py)?,
                )?;
            }
            collected.into_any()
        },))?;
    } else if !shape.is_none()
        && !py
            .get_type::<pyo3::types::PyDict>()
            .call1((tool.getattr("_SHAPES")?,))?
            .contains(&shape)?
    {
        return Ok(none(py));
    }
    if !shape.is_none() && tool.call_method0("_corners_mode")?.is_truthy()? {
        viewport.call_method1(
            "flash_status",
            (module.call_method1("tr", ("Drawn profiles bevel edges only.",))?,),
        )?;
        return Ok(none(py));
    }
    tool.get_type().into_any().setattr("shape", &shape)?;
    if shape.is_instance(&py.get_type::<pyo3::types::PyString>().into_any())? {
        viewport.call_method1("flash_status", (module.call_method("tr", ("Profile {p}",), Some(&fields!(py; "p" => module.call_method1("tr", (tool.call_method0("_shape_label")?,))?)))?,))?;
    }
    tool.call_method0("_replan")?;
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn host_panel_value<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.fillet_host")?;
    let arguments = a.get_item(2)?;
    let field: Obj<'py>;
    let tool = a.get_item(0)?;
    let value: Obj<'py>;
    let viewport = a.get_item(1)?;
    {
        let [part0, part1] = unpack::<2>(&arguments)?;
        field = part0;
        value = part1;
    }
    imported(py, "builtins")?.getattr("setattr")?.call1((
        tool.get_type().into_any(),
        &field,
        &value,
    ))?;
    if field.eq("segments")? {
        viewport.call_method1(
            "flash_status",
            (module.call_method("tr", ("{n} segments",), Some(&fields!(py; "n" => &value)))?,),
        )?;
    } else {
        if field.eq("profile")? {
            tool.get_type().into_any().setattr("shape", none(py))?;
            viewport.call_method1("flash_status", (module.call_method("tr", ("Profile {p}",), Some(&fields!(py; "p" => [value.call_method1("__format__", (".2f",))?.extract::<String>()?].concat().into_bound_py_any(py)?)))?,))?;
        }
    }
    Ok(none(py))
}
fn replan<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.fillet_host")?;
    let mut e: Obj<'py>;
    let plan: Obj<'py>;
    let tool = a.get_item(0)?;
    let edges = tool.call_method0("_current_edges")?;
    let mesh = optional(&optional(&tool.getattr("_viewport")?, "scene")?, "mesh")?;
    if !(edges.is_truthy()?) || mesh.is_none() {
        tool.setattr("_plan", none(py))?;
        tool.setattr("_plan_key", none(py))?;
        tool.setattr("_message", none(py))?;
        return Ok(none(py));
    }
    let key = (
        py.get_type::<pyo3::types::PyTuple>().call1(({
            let collected = PyList::empty(py);
            for item in edges.try_iter()? {
                e = item?;
                collected.append(e.getattr("key")?)?;
            }
            collected.into_any()
        },))?,
        imported(py, "builtins")?
            .getattr("round")?
            .call1((tool.getattr("radius")?, 6))?,
        tool.getattr("segments")?,
        tool.getattr("profile")?,
        tool.getattr("measure")?,
        tool.getattr("corners")?,
        tool.getattr("affect")?,
        tool.getattr("shape")?,
        mesh.getattr_opt("_mut_serial")?
            .unwrap_or(0.into_bound_py_any(py)?),
        optional(&optional(&mesh, "_store")?, "revision")?,
    )
        .into_bound_py_any(py)?;
    if key.eq(tool.getattr("_plan_key")?)? {
        return Ok(none(py));
    }
    tool.setattr("_plan_key", &key)?;
    if tool.call_method0("_corners_mode")?.is_truthy()? {
        plan = module.call_method1(
            "plan_vertex_bevel",
            (
                &mesh,
                &edges,
                tool.getattr("radius")?,
                tool.getattr("segments")?,
                tool.getattr("profile")?,
            ),
        )?;
    } else {
        plan = module.call_method1(
            "plan_fillet",
            (
                &mesh,
                &edges,
                tool.getattr("radius")?,
                tool.getattr("segments")?,
                tool.getattr("profile")?,
                tool.getattr("measure")?,
                tool.getattr("corners")?,
                tool.getattr("shape")?,
            ),
        )?;
    }
    if plan.is_instance(&py.get_type::<pyo3::types::PyString>().into_any())? {
        tool.setattr("_plan", none(py))?;
        tool.setattr("_message", &plan)?;
    } else {
        tool.setattr("_plan", &plan)?;
        tool.setattr("_message", none(py))?;
    }
    Ok(none(py))
}
fn rubber_band_lines<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let mut i: Obj<'py>;
    let mut pts: Obj<'py>;
    let tool = a.get_item(0)?;
    if tool.getattr("_plan")?.is_none() {
        return Ok(PyList::empty(py).into_any());
    }
    let lines = PyList::empty(py).into_any();
    for item in tool.getattr("_plan")?.getattr("curves")?.try_iter()? {
        pts = item?;
        lines.call_method1(
            "extend",
            ({
                let collected = PyList::empty(py);
                for item in imported(py, "builtins")?
                    .getattr("range")?
                    .call1((pts.len()?.into_bound_py_any(py)?.sub(1)?,))?
                    .try_iter()?
                {
                    i = item?;
                    collected.append(
                        (pts.get_item(&i)?, pts.get_item(i.add(1)?)?).into_bound_py_any(py)?,
                    )?;
                }
                collected.into_any()
            },),
        )?;
    }
    Ok(lines.clone())
}
fn value_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.fillet_host")?;
    let head: Obj<'py>;
    let tool = a.get_item(0)?;
    let edges = tool.call_method0("_current_edges")?;
    if !(edges.is_truthy()?) {
        return Ok(none(py));
    }
    let e = edges.get_item(0)?;
    let mid = if e.is_instance(&module.getattr("Edge")?)? {
        e.getattr("a")?.add(e.getattr("b")?)?.mul(0.5)?
    } else {
        module.call_method1("QVector3D", (e.getattr("position")?,))?
    };
    if tool.getattr("_message")?.is_truthy()? {
        return (tool.getattr("_message")?, &mid).into_bound_py_any(py);
    }
    let size = module.call_method1("fmt_len_fine", (tool.getattr("radius")?,))?;
    if tool.call_method0("_corners_mode")?.is_truthy()? {
        head = module
            .call_method1("tr", ("Distance",))?
            .add(" ")?
            .add(&size)?;
    } else {
        if tool.getattr("measure")?.eq("radius")? {
            head = ("R ").into_bound_py_any(py)?.add(&size)?;
        } else {
            head = module
                .call_method1("tr", (tool.call_method0("_measure_label")?,))?
                .add(" ")?
                .add(&size)?;
        }
    }
    let mut extra = module.call_method(
        "tr",
        ("{n} segments",),
        Some(&fields!(py; "n" => tool.getattr("segments")?)),
    )?;
    if !tool.getattr("shape")?.is_none() && !(tool.call_method0("_corners_mode")?.is_truthy()?) {
        extra = extra.add(
            (", ")
                .into_bound_py_any(py)?
                .add(module.call_method1("tr", (tool.call_method0("_shape_label")?,))?)?,
        )?;
    } else {
        if tool
            .getattr("profile")?
            .sub(0.5)?
            .abs()?
            .gt(1e-09)?
        {
            extra = extra.add((", ").into_bound_py_any(py)?.add(module.call_method("tr", ("profile {p}",), Some(&fields!(py; "p" => [tool.getattr("profile")?.call_method1("__format__", (".2f",))?.extract::<String>()?].concat().into_bound_py_any(py)?)))?)?)?;
        }
    }
    (head.add("  (")?.add(&extra)?.add(")")?, &mid).into_bound_py_any(py)
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("_corners_mode", Mode::Method, corners_mode, &["self"], "bool", ""),
    ("host_reset_bevel", Mode::Method, host_reset_bevel, &["self", "viewport", "activate"], "", ""),
    ("_reset", Mode::Method, reset, &["self"], "None", ""),
    ("start_point", Mode::Property, start_point, &["self"], "", "Esc cascade hook: an in-progress rounding to cancel."),
    ("host_pick_bevel", Mode::Method, host_pick_bevel, &["self", "vp", "world"], "", ""),
    ("host_hover_bevel", Mode::Method, host_hover_bevel, &["self", "vp", "world"], "", ""),
    ("on_hover", Mode::Method, on_hover, &["self", "ctx"], "", ""),
    ("host_bevel_geometry", Mode::Method, host_bevel_geometry, &["self", "vp", "_"], "", ""),
    ("host_replan", Mode::Method, host_replan, &["self", "vp", "_"], "", ""),
    ("host_bevel_picked", Mode::Method, host_bevel_picked, &["self", "vp", "_"], "", ""),
    ("on_key", Mode::Method, on_key, &["self", "viewport", "key", "modifiers"], "", ""),
    ("set_affect", Mode::Method, set_affect, &["self", "viewport", "affect:str"], "None", "Edges or vertices. Picked edges carry over as their corners;\npicked corners do not make edges, so they are let go."),
    ("_refresh_readouts", Mode::Method, refresh_readouts, &["self", "viewport"], "None", "The status clause and the VCB caption name the mode: redraw them\nnow, not on the next live value (with nothing picked there is\nnone)."),
    ("on_key_release", Mode::Method, on_key_release, &["self", "viewport", "key:int"], "bool", ""),
    ("host_bevel_measure", Mode::Method, host_bevel_measure, &["self", "viewport", "_"], "", ""),
    ("host_measure", Mode::Method, host_measure, &["self", "viewport", "measure"], "", ""),
    ("set_measure", Mode::Method, set_measure, &["self", "viewport", "measure:str"], "None", ""),
    ("set_size", Mode::Method, set_size, &["self", "viewport", "value:float"], "None", "The panel's Amount: the size, without rounding yet."),
    ("set_shape", Mode::Method, set_shape, &["self", "viewport", "shape"], "None", "A drawn outline (steps, mouldings…), or None for the\nsuperellipse."),
    ("set_corners", Mode::Method, set_corners, &["self", "viewport", "corners:str"], "None", ""),
    ("apply_from_panel", Mode::Method, apply_from_panel, &["self", "viewport"], "None", "The panel's Apply: round what is picked, as Enter would."),
    ("_measure_label", Mode::Method, measure_label, &["self"], "str", "English source label of the current measure."),
    ("vcb_caption", Mode::Method, vcb_caption, &["self"], "str", ""),
    ("status_clause", Mode::Method, status_clause, &["self"], "str", ""),
    ("on_value", Mode::Method, on_value, &["self", "viewport", "value"], "", ""),
    ("on_segments_value", Mode::Method, on_segments_value, &["self", "viewport", "n"], "", ""),
    ("on_profile_value", Mode::Method, on_profile_value, &["self", "viewport", "p"], "", ""),
    ("host_panel_value", Mode::Method, host_panel_value, &["self", "viewport", "args"], "", ""),
    ("host_bad_profile", Mode::Method, host_bad_profile, &["self", "viewport", "_"], "", ""),
    ("host_announce_size", Mode::Method, host_announce_size, &["self", "viewport", "_"], "", ""),
    ("_current_edges", Mode::Method, current_edges, &["self"], "list", "What would be bevelled now: the picked edges (or corners), else\nthe one under the cursor."),
    ("_replan", Mode::Method, replan, &["self"], "None", ""),
    ("host_fillet_plan", Mode::Method, host_fillet_plan, &["self", "viewport", "_"], "", ""),
    ("host_finish_bevel", Mode::Method, host_finish_bevel, &["self", "viewport", "_"], "", ""),
    ("preview_faces", Mode::Method, preview_faces, &["self"], "", ""),
    ("rubber_band_lines", Mode::Method, rubber_band_lines, &["self"], "", ""),
    ("value_label", Mode::Method, value_label, &["self"], "", ""),
    ("_shape_label", Mode::Method, shape_label, &["self"], "str", "English source label of the drawn outline in use."),
    ("_is_plain_fillet", Mode::Method, is_plain_fillet, &["self"], "bool", ""),
];
