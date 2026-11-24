// SPDX-License-Identifier: GPL-3.0-or-later
//! OffsetHost callbacks on the live document.
use super::*;

fn uses_snap<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("dragging")
}
fn chain<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_state", ("chain",))
}
fn set_chain<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let chain = a.get_item(1)?;
    let tool = a.get_item(0)?;
    if chain.is_truthy()? {
        tool.setattr("base_face", none(py))?;
    }
    Ok(none(py))
}
fn base_face<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("_base_face")
}
fn set_base_face<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let face = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.setattr("_base_face", &face)?;
    tool.getattr("_machine")?.call_method1(
        "set",
        (
            "face",
            (if !face.is_none() {
                py.get_type::<pyo3::types::PyString>()
                    .call1((face.getattr("key")?,))?
            } else {
                none(py)
            }),
        ),
    )?;
    Ok(none(py))
}
fn loop_points<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.offset_host")?;
    let tool = a.get_item(0)?;
    return Ok({
        let items = PyList::empty(py);
        for item in tool
            .getattr("_machine")?
            .call_method1("query", ("points", py.None()))?
            .try_iter()?
        {
            let p = item?;
            items.append(module.call_method1("QVector3D", positional(&p)?)?)?;
        }
        items.into_any()
    });
}
fn set_loop<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let points = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.getattr("_machine")?.call_method1(
        "set",
        (
            "points",
            ({
                let items = PyList::empty(py);
                for item in points.try_iter()? {
                    let p = item?;
                    items.append(p.call_method0("toTuple")?)?;
                }
                items.into_any()
            }),
        ),
    )?;
    Ok(none(py))
}
fn normal<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.offset_host")?;
    let tool = a.get_item(0)?;
    module.call_method1(
        "QVector3D",
        positional(
            &tool
                .getattr("_machine")?
                .call_method1("query", ("normal", py.None()))?,
        )?,
    )
}
fn set_normal<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let n = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.getattr("_machine")?
        .call_method1("set", ("normal", n.call_method0("toTuple")?))?;
    Ok(none(py))
}
fn on_value<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let value = a.get_item(2)?;
    let viewport = a.get_item(1)?;
    if !(py
        .import("builtins")?
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
    let answer = tool.call_method1(
        "_send",
        (
            &viewport,
            fields!(py; "type" => "value", "value" => &value).into_any(),
        ),
    )?;
    if answer.get_item("redraw")?.is_truthy()? {
        viewport.call_method0("update")?;
    }
    answer.get_item("consumed")
}
fn commit<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1(
        "_send",
        (
            &viewport,
            fields!(py; "type" => "key", "key" => "CommitOffset").into_any(),
        ),
    )?;
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn value_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.offset_host")?;
    let tool = a.get_item(0)?;
    let label = if tool.getattr("_during")?.is_none() {
        tool.getattr("_machine")?
            .call_method1("query", ("label", py.None()))?
    } else {
        none(py)
    };
    if !label.is_none() {
        return (
            module.call_method1("fmt_len", (label.get_item(0)?,))?,
            module.call_method1("QVector3D", positional(&label.get_item(1)?)?)?,
        )
            .into_bound_py_any(py);
    }
    Ok(none(py))
}
fn host_clear_offset_picks<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    {
        let value = none(py);
        tool.setattr("hovered_face", &value)?;
        tool.setattr("hovered_edge", &value)?;
        tool.setattr("_base_face", &value)?;
    }
    Ok(none(py))
}
fn host_clear_offset_hover<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let viewport = a.get_item(1)?;
    viewport.call_method1("set_hover", (py.None(),))?;
    Ok(none(py))
}
fn host_hover_outline<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let pick: Obj<'py>;
    let px = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.setattr(
        "hovered_face",
        viewport.call_method1("pick_face", positional(&px)?)?,
    )?;
    tool.setattr("hovered_edge", none(py))?;
    if tool.getattr("hovered_face")?.is_none() {
        pick = optional(&viewport, "pick_edge")?;
        tool.setattr(
            "hovered_edge",
            if pick.is_truthy()? {
                pick.call1(positional(&px)?)?
            } else {
                none(py)
            },
        )?;
    }
    viewport.call_method1(
        "set_hover",
        (({
            let value = tool.getattr("hovered_face")?;
            if value.is_truthy()? {
                value
            } else {
                tool.getattr("hovered_edge")?
            }
        }),),
    )?;
    tool.call_method1("_picked", (&viewport, false))
}
fn host_picked_outline<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.setattr("_base_face", tool.getattr("hovered_face")?)?;
    tool.call_method1("_picked", (&viewport, true))
}
fn picked<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.offset_host")?;
    let report = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let f = tool.getattr("hovered_face")?;
    if !f.is_none() {
        if f.getattr("vertices")?.len()? >= 3 {
            return Ok(fields!(py; "points" => f.getattr("vertices")?, "normal" => f.call_method0("normal")?, "origin" => f.call_method0("centroid")?, "face" => py.get_type::<pyo3::types::PyString>().call1((f.getattr("key")?,))?, "closed" => true).into_any());
        }
    } else if !tool.getattr("hovered_edge")?.is_none() {
        return tool.call_method1(
            "_run_payload",
            (
                &viewport,
                module.call_method1("_run_from_edge", (tool.getattr("hovered_edge")?,))?,
                &report,
            ),
        );
    }
    Ok(none(py))
}
fn run_payload<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.offset_host")?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let run = a.get_item(2)?;
    let report = a.get_item(3)?.is_truthy()?;
    let (message, duration) = if run.is_none() {
        (
            "Offset needs one connected run of edges — a polyline, or a closed loop.",
            5000,
        )
    } else {
        let [points, closed] = unpack::<2>(&run)?;
        let normal = module.call_method1("_newell", (&points,))?;
        if !normal.is_none() {
            return Ok(
                fields!(py; "points" => points, "closed" => closed, "normal" => normal).into_any(),
            );
        }
        if report && points.len()? == 2 && !closed.is_truthy()? {
            let edge = tool.getattr("hovered_edge")?;
            if !edge.is_none() {
                viewport
                    .getattr("scene")?
                    .call_method1("select", (PyList::new(py, [edge])?,))?;
            }
            ("This line is on its own — nothing is joined to its ends, so there is no chain to offset. Draw the missing line, or select the edges to offset.", 6000)
        } else {
            ("Offset needs at least two edges that are not in line — a straight run has no plane to offset in.", 5000)
        }
    };
    if report {
        viewport.call_method1(
            "flash_status",
            (module.call_method1("tr", (message,))?, duration),
        )?;
    }
    Ok(none(py))
}
fn host_offset_refusal<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.offset_host")?;
    let arguments = a.get_item(2)?;
    let distance: Obj<'py>;
    let error: Obj<'py>;
    let message: Obj<'py>;
    let normal: Obj<'py>;
    let points: Obj<'py>;
    let room: Obj<'py>;
    let side: Obj<'py>;
    let sign: Obj<'py>;
    let viewport = a.get_item(1)?;
    let max_offset_distance = py.import("core.topology")?.getattr("max_offset_distance")?;
    [error, distance, points, normal] = unpack::<4>(&arguments)?;
    let d = module.call_method1("fmt_len", (distance.abs()?,))?;
    if error.eq("hole")? {
        message = module.call_method(
            "tr",
            ("{d} runs into a hole of this face — try a smaller offset",),
            Some(&fields!(py; "d" => &d)),
        )?;
    } else {
        if error.eq("collapsed_chain")? {
            message = module.call_method(
                "tr",
                ("{d} collapses this run — try a smaller offset",),
                Some(&fields!(py; "d" => &d)),
            )?;
        } else {
            if error.eq("collapsed_face")? {
                sign = if distance.lt(0)? {
                    1.into_bound_py_any(py)?.neg()?
                } else {
                    1.into_bound_py_any(py)?
                };
                room = max_offset_distance.call1((
                    ({
                        let items = PyList::empty(py);
                        for item in points.try_iter()? {
                            let p = item?;
                            items.append(module.call_method1("QVector3D", positional(&p)?)?)?;
                        }
                        items.into_any()
                    }),
                    module.call_method1("QVector3D", positional(&normal)?)?,
                    &sign,
                ))?;
                side = if sign.gt(0)? {
                    module.call_method1("tr", ("inward",))?
                } else {
                    module.call_method1("tr", ("outward",))?
                };
                if room.le(0.0001)? {
                    message = module.call_method(
                        "tr",
                        ("{d} closes this face — it takes no offset {side}",),
                        Some(&fields!(py; "d" => &d, "side" => &side)),
                    )?;
                } else {
                    message = module.call_method("tr", ("{d} closes this face — {side} it takes at most {max}",), Some(&fields!(py; "d" => &d, "side" => &side, "max" => module.call_method1("fmt_len", (&room,))?)))?;
                }
            } else {
                message = error.clone();
            }
        }
    }
    if message.is_truthy()? {
        viewport.call_method1("flash_status", (&message, 5000))?;
    }
    Ok(none(py))
}

fn offset_reference<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.offset_host")?;
    let mut p: Obj<'py>;
    let tool = a.get_item(0)?;
    let points = tool.call_method1("_state", ("reference",))?;
    return Ok(if points.is_none() {
        none(py)
    } else {
        py.get_type::<pyo3::types::PyTuple>().call1(({
            let collected = PyList::empty(py);
            for item in points.try_iter()? {
                p = item?;
                collected.append(module.call_method1("QVector3D", positional(&p)?)?)?;
            }
            collected.into_any()
        },))?
    });
}
fn pointer<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.offset_host")?;
    let ctx = a.get_item(2)?;
    let kind = a.get_item(1)?;
    let tool = a.get_item(0)?;
    let t = a.get_item(3)?;
    let event = py
        .import("builtins")?
        .getattr("super")?
        .call1((module.getattr("OffsetHost")?, &tool))?
        .call_method1("_pointer", (&kind, &ctx, &t))?;
    if tool
        .getattr("allowed_snap_kinds")?
        .contains(event.get_item("snap_kind")?)?
    {
        event.set_item(
            "world",
            ctx.getattr("snap")?
                .getattr("point")?
                .call_method0("toTuple")?,
        )?;
    } else {
        event.set_item("snap_kind", none(py))?;
    }
    Ok(event.clone())
}
fn host_selected_outline<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.offset_host")?;
    let mut e: Obj<'py>;
    let tool = a.get_item(0)?;
    let source: Obj<'py>;
    let viewport = a.get_item(1)?;
    let selection = py
        .get_type::<pyo3::types::PyList>()
        .call1((optional(&viewport, "scene")?
            .getattr_opt("selection")?
            .unwrap_or(PyTuple::empty(py).into_any()),))?;
    if selection.len()? < 2
        || !({
            let mut found = true;
            for item in selection.try_iter()? {
                e = item?;
                if !(e.is_instance(&module.getattr("Edge")?)?) {
                    found = false;
                    break;
                }
            }
            found.into_bound_py_any(py)?
        }
        .is_truthy()?)
    {
        return Ok(none(py));
    }
    let run = module.call_method1("_run_from_edges", (&selection,))?;
    if !run.is_none() {
        source = tool.call_method1("_run_payload", (&viewport, &run, true))?;
        if source.is_truthy()? {
            viewport.call_method1(
                "flash_status",
                (
                    module.call_method(
                        "tr",
                        ("{n} edges selected — click to start the offset",),
                        Some(&fields!(py; "n" => selection.len()?.into_bound_py_any(py)?)),
                    )?,
                    4000,
                ),
            )?;
        }
        return Ok(source.clone());
    }
    Ok(none(py))
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("uses_snap", Mode::Property, uses_snap, &["self"], "bool", "Only during the drag, whose distance snaps to named model points.\nBefore it the click picks a face or a run of edges, not a point."),
    ("_chain", Mode::Property, chain, &["self"], "", ""),
    ("base_face", Mode::Property, base_face, &["self"], "", ""),
    ("_normal", Mode::Property, normal, &["self"], "", ""),
    ("_offset_reference", Mode::Property, offset_reference, &["self"], "", ""),
    ("_pointer", Mode::Method, pointer, &["self", "kind", "ctx", "t=0.0"], "", ""),
    ("on_value", Mode::Method, on_value, &["self", "viewport", "value"], "", ""),
    ("_commit", Mode::Method, commit, &["self", "viewport"], "", ""),
    ("value_label", Mode::Method, value_label, &["self"], "", ""),
    ("host_clear_offset_picks", Mode::Method, host_clear_offset_picks, &["self", "viewport", "_"], "", ""),
    ("host_clear_offset_hover", Mode::Method, host_clear_offset_hover, &["self", "viewport", "_"], "", ""),
    ("host_hover_outline", Mode::Method, host_hover_outline, &["self", "viewport", "px"], "", ""),
    ("host_picked_outline", Mode::Method, host_picked_outline, &["self", "viewport", "_"], "", ""),
    ("_picked", Mode::Method, picked, &["self", "viewport", "report"], "", ""),
    ("host_selected_outline", Mode::Method, host_selected_outline, &["self", "viewport", "_"], "", ""),
    ("_run_payload", Mode::Method, run_payload, &["self", "viewport", "run", "report"], "", ""),
    ("host_offset_refusal", Mode::Method, host_offset_refusal, &["self", "viewport", "args"], "", ""),
    ("_loop", Mode::Property, loop_points, &["self"], "", ""),
    ("_chain", Mode::Setter, set_chain, &["self", "chain"], "", ""),
    ("base_face", Mode::Setter, set_base_face, &["self", "face"], "", ""),
    ("_loop", Mode::Setter, set_loop, &["self", "points"], "", ""),
    ("_normal", Mode::Setter, set_normal, &["self", "n"], "", ""),
];
