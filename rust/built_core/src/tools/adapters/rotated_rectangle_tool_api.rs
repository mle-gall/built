// SPDX-License-Identifier: GPL-3.0-or-later
//! RotatedRectangleTool callbacks on the live document.
use super::*;

fn angle<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    py.get_type::<pyo3::types::PyFloat>()
        .call1((tool.call_method1("_state", ("angle",))?,))
}
fn set_angle<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let value = a.get_item(1)?;
    tool.getattr("_machine")?.call_method1(
        "set",
        (
            "angle",
            py.get_type::<pyo3::types::PyFloat>().call1((&value,))?,
        ),
    )?;
    Ok(none(py))
}
fn vcb_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok(if tool.getattr("base_point")?.is_none() {
        ("Length; angle").into_bound_py_any(py)?
    } else {
        ("Width; angle").into_bound_py_any(py)?
    })
}
fn magnet_on<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    (tool.getattr("base_point")?.is_none()).into_bound_py_any(py)
}
fn diagonal_span<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    (!tool.getattr("base_point")?.is_none()).into_bound_py_any(py)
}
fn view<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let mut _v: Obj<'py>;
    let centre: Obj<'py>;
    let ctx = a.get_item(2)?;
    let d: Obj<'py>;
    let disc: Obj<'py>;
    let kind: Obj<'py>;
    let mut near: Obj<'py>;
    let o: Obj<'py>;
    let mut p0: Obj<'py>;
    let mut p1: Obj<'py>;
    let px: Obj<'py>;
    let mut radius: Obj<'py>;
    let ray: Obj<'py>;
    let screen: Obj<'py>;
    let tool = a.get_item(0)?;
    let u: Obj<'py>;
    let viewport = a.get_item(1)?;
    let w2p: Obj<'py>;
    let math = imported(py, "math")?.into_any();
    let a_x_e_s = imported(py, "built_core.axes")?.getattr("AXES")?;
    let plane_axes = imported(py, "built_core.axes")?.getattr("plane_axes")?;
    let lock = optional(&viewport, "axis_lock")?;
    let reading = fields!(py; "type" => "view", "axis_lock" => if lock.is_truthy()? && a_x_e_s.contains(&lock)? { module.call_method1("_tuple", (a_x_e_s.get_item(&lock)?,))? } else { none(py) }).into_any();
    let normal = optional(&viewport, "_work_plane_normal")?;
    if tool.getattr("work_plane")?.is_none() && normal.is_callable() {
        reading.set_item(
            "plane_normal",
            module.call_method1("_tuple", (normal.call0()?,))?,
        )?;
    }
    if !ctx.is_none() {
        screen = ctx.getattr("screen")?;
        ray = optional(&viewport, "_pixel_to_ray")?;
        kind = optional(&optional(&ctx, "snap")?, "kind")?;
        if !ray.is_none()
            && !tool.getattr("base_point")?.is_none()
            && !(tool
                .getattr("_machine")?
                .call_method1("query", ("point_snap", &kind))?
                .is_truthy()?)
        {
            [o, d] =
                unpack::<2>(&ray.call1((screen.call_method0("x")?, screen.call_method0("y")?))?)?;
            if !o.is_none() && !d.is_none() {
                reading.set_item(
                    "ray",
                    (
                        module.call_method1("_tuple", (&o,))?,
                        module.call_method1("_tuple", (&d,))?,
                    )
                        .into_bound_py_any(py)?,
                )?;
            }
        }
        disc = tool.call_method1("_state", ("disc",))?;
        w2p = optional(&viewport, "_world_to_pixel")?;
        if !disc.is_none() {
            [radius, near] = unpack::<2>(&(py.None(), false).into_bound_py_any(py)?)?;
            centre = module.call_method1("Vec3", positional(&disc.get_item(0)?)?)?;
            [u, _v] = unpack::<2>(
                &plane_axes
                    .call1((module.call_method1("Vec3", positional(&disc.get_item(1)?)?)?,))?,
            )?;
            {
                let value = none(py);
                p0 = value.clone();
                p1 = value.clone();
            }
            if !w2p.is_none() {
                [p0, p1] = unpack::<2>(
                    &(w2p.call1((&centre,))?, w2p.call1((centre.add(&u)?,))?)
                        .into_bound_py_any(py)?,
                )?;
            }
            if !p0.is_none() && !p1.is_none() {
                let disc_px = imported(py, "tools.protractor")?.getattr("DISC_PX")?;
                px = math.call_method1(
                    "hypot",
                    (
                        p1.get_item(0)?.sub(p0.get_item(0)?)?,
                        p1.get_item(1)?.sub(p0.get_item(1)?)?,
                    ),
                )?;
                if px.gt(1e-06)? {
                    radius = disc_px.div(&px)?;
                }
                near = (math
                    .call_method1(
                        "hypot",
                        (
                            screen.call_method0("x")?.sub(p0.get_item(0)?)?,
                            screen.call_method0("y")?.sub(p0.get_item(1)?)?,
                        ),
                    )?
                    .le(disc_px.mul(1.25)?)?)
                .into_bound_py_any(py)?;
            }
            reading.set_item("protractor", (&radius, &near).into_bound_py_any(py)?)?;
        }
    }
    tool.call_method1("_send", (&viewport, &reading))?;
    Ok(none(py))
}
fn face_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let mut _point: Obj<'py>;
    let ctx = a.get_item(1)?;
    let face: Obj<'py>;
    let group: Obj<'py>;
    let normal: Obj<'py>;
    let pick = {
        let value = optional(&ctx.getattr("viewport")?, "pick_face_placement")?;
        if value.is_truthy()? {
            value
        } else {
            optional(&ctx.getattr("viewport")?, "pick_face_any")?
        }
    };
    if pick.is_none() {
        return Ok(none(py));
    }
    [face, group] = unpack::<2>(&pick.call1((
        ctx.getattr("screen")?.call_method0("x")?,
        ctx.getattr("screen")?.call_method0("y")?,
    ))?)?;
    if face.is_none() {
        return Ok(none(py));
    }
    let face_plane_world = imported(py, "core.snap")?.getattr("face_plane_world")?;
    [_point, normal] = unpack::<2>(&face_plane_world.call1((&face, optional(&group, "xform")?))?)?;
    if normal.is_none() || normal.call_method0("lengthSquared")?.lt(0.5)? {
        return Ok(none(py));
    }
    (
        module.call_method1("_tuple", (ctx.getattr("world")?,))?,
        module.call_method1("_tuple", (&normal,))?,
    )
        .into_bound_py_any(py)
}
fn framed<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(3)?;
    let event = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let axes = imported(py, "built_core.axes")?.into_any();
    tool.call_method1(
        "_send",
        (
            &viewport,
            fields!(py; "type" => "frame", "axes" => axes.call_method0("frame")?).into_any(),
        ),
    )?;
    tool.call_method1("_view", (&viewport, &ctx))?;
    tool.call_method1("_send", (&viewport, &event))
}
fn on_activate<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1(
        "_framed",
        (&viewport, fields!(py; "type" => "activate").into_any()),
    )?;
    Ok(none(py))
}
fn on_deactivate<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1(
        "_framed",
        (&viewport, fields!(py; "type" => "deactivate").into_any()),
    )?;
    Ok(none(py))
}
fn on_cancel<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1(
        "_framed",
        (&viewport, fields!(py; "type" => "cancel").into_any()),
    )?;
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn on_click<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.setattr("_viewport", ctx.getattr("viewport")?)?;
    if tool.getattr("start_point")?.is_none() {
        tool.call_method1("_send", (ctx.getattr("viewport")?, fields!(py; "type" => "work_plane", "plane" => tool.call_method1("_face_plane", (&ctx,))?).into_any(),))?;
    }
    tool.call_method1(
        "_framed",
        (
            ctx.getattr("viewport")?,
            tool.call_method1("_pointer", ("click", &ctx))?,
            &ctx,
        ),
    )?;
    ctx.getattr("viewport")?.call_method0("update")?;
    Ok(none(py))
}
fn on_hover<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.setattr("_viewport", ctx.getattr("viewport")?)?;
    tool.call_method1(
        "_framed",
        (
            ctx.getattr("viewport")?,
            tool.call_method1("_pointer", ("hover", &ctx))?,
            &ctx,
        ),
    )?;
    ctx.getattr("viewport")?.call_method0("update")?;
    Ok(none(py))
}
fn on_value<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let tool = a.get_item(0)?;
    let mut value = a.get_item(2)?;
    let viewport = a.get_item(1)?;
    if value.is_instance(&py.get_type::<pyo3::types::PyTuple>().into_any())? && value.len()? == 2 {
        value = (
            py.get_type::<pyo3::types::PyFloat>()
                .call1((value.get_item(0)?,))?,
            module.call_method1("_degrees", (value.get_item(1)?,))?,
        )
            .into_bound_py_any(py)?;
    }
    let reply = tool.call_method1(
        "_framed",
        (
            &viewport,
            fields!(py; "type" => "value", "value" => &value).into_any(),
        ),
    )?;
    if reply.get_item("consumed")?.is_truthy()? {
        viewport.call_method0("update")?;
    }
    reply.get_item("consumed")
}
fn on_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("plane_lock_key", (&viewport, &key))
}
fn locked<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let tool = a.get_item(0)?;
    let d = tool.call_method1("_state", ("locked",))?;
    Ok(if d.is_none() {
        none(py)
    } else {
        module.call_method1("Vec3", positional(&d)?)?
    })
}
fn locked_dir<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    tool.call_method1("_view", (&viewport,))?;
    let d = tool
        .getattr("_machine")?
        .call_method1("query", ("locked_dir", py.None()))?;
    Ok(if d.is_none() {
        none(py)
    } else {
        module.call_method1("Vec3", positional(&d)?)?
    })
}
fn corners<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let angle = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let width = a.get_item(1)?;
    return Ok({
        let items = PyList::empty(py);
        for item in tool
            .getattr("_machine")?
            .call_method1(
                "query",
                (
                    "corners",
                    (
                        py.get_type::<pyo3::types::PyFloat>().call1((&width,))?,
                        &angle,
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
fn width_and_angle<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let first: Obj<'py>;
    let cursor = a.get_item(1)?;
    let forced = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let w: Obj<'py>;
    [w, first] = unpack::<2>(
        &tool.getattr("_machine")?.call_method1(
            "query",
            (
                "width_and_angle",
                (
                    module.call_method1("_tuple", (&cursor,))?,
                    module.call_method1("_tuple", (&forced,))?,
                )
                    .into_bound_py_any(py)?,
            ),
        )?,
    )?;
    (&w, &first).into_bound_py_any(py)
}
fn perp<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let tool = a.get_item(0)?;
    module.call_method1(
        "Vec3",
        positional(
            &tool
                .getattr("_machine")?
                .call_method1("query", ("perp", py.None()))?,
        )?,
    )
}
fn normal<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let tool = a.get_item(0)?;
    module.call_method1(
        "Vec3",
        positional(
            &tool
                .getattr("_machine")?
                .call_method1("query", ("normal", py.None()))?,
        )?,
    )
}
fn reset<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    tool.getattr("_machine")?
        .call_method1("set", ("reset", true))?;
    Ok(none(py))
}

fn value_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let mut _unused: Obj<'py>;
    let angle: Obj<'py>;
    let length: Obj<'py>;
    let mid: Obj<'py>;
    let tool = a.get_item(0)?;
    let text: Obj<'py>;
    let width: Obj<'py>;
    let label = tool.call_method1("_state", ("label",))?;
    if label.is_none() {
        return Ok(none(py));
    }
    let fmt_len = imported(py, "core.units")?.getattr("fmt_len")?;
    let fmt_pair = imported(py, "core.units")?.getattr("fmt_pair")?;
    if label.get_item(0)?.eq("edge")? {
        {
            let [part0, part1, part2, part3] = unpack::<4>(&label)?;
            _unused = part0;
            length = part1;
            angle = part2;
            mid = part3;
        }
        text = fmt_len.call1((&length,))?.add(if angle.is_none() {
            ("").into_bound_py_any(py)?
        } else {
            [
                "   ".to_owned(),
                angle
                    .call_method1("__format__", (".0f",))?
                    .extract::<String>()?,
                "°".to_owned(),
            ]
            .concat()
            .into_bound_py_any(py)?
        })?;
        return (&text, module.call_method1("Vec3", positional(&mid)?)?).into_bound_py_any(py);
    }
    {
        let [part0, part1, part2, part3, part4] = unpack::<5>(&label)?;
        _unused = part0;
        length = part1;
        width = part2;
        angle = part3;
        mid = part4;
    }
    text = fmt_pair
        .call1((&length, &width))?
        .add(if angle.abs()?.gt(0.05)? {
            [
                "   ".to_owned(),
                angle
                    .call_method1("__format__", (".0f",))?
                    .extract::<String>()?,
                "°".to_owned(),
            ]
            .concat()
            .into_bound_py_any(py)?
        } else {
            ("").into_bound_py_any(py)?
        })?;
    (&text, module.call_method1("Vec3", positional(&mid)?)?).into_bound_py_any(py)
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("angle", Mode::Property, angle, &["self"], "", ""),
    ("vcb_label", Mode::Property, vcb_label, &["self"], "str", ""),
    ("magnet_on", Mode::Method, magnet_on, &["self"], "bool", ""),
    ("diagonal_span", Mode::Property, diagonal_span, &["self"], "bool", ""),
    ("_view", Mode::Method, view, &["self", "viewport", "ctx=None"], "", "``ViewReading`` for the event about to go: the cursor's ray, the\naxis lock, the next protractor's size and nearness, the view's\ndrawing plane."),
    ("_face_plane", Mode::Method, face_plane, &["self", "ctx"], "", "The face under the first click, as the plane to draw on."),
    ("_framed", Mode::Method, framed, &["self", "viewport", "event", "ctx=None"], "", ""),
    ("on_activate", Mode::Method, on_activate, &["self", "viewport"], "None", ""),
    ("on_deactivate", Mode::Method, on_deactivate, &["self", "viewport"], "None", ""),
    ("on_cancel", Mode::Method, on_cancel, &["self", "viewport"], "None", ""),
    ("on_click", Mode::Method, on_click, &["self", "ctx"], "None", ""),
    ("on_hover", Mode::Method, on_hover, &["self", "ctx"], "None", ""),
    ("on_value", Mode::Method, on_value, &["self", "viewport", "value"], "bool", "``3`` = 3 m keeping the angle shown; ``3;90`` = 3 m standing\nsquare to the plane. The angle field reads without the document's\nunit (``_degrees``)."),
    ("on_key", Mode::Method, on_key, &["self", "viewport", "key", "modifiers"], "bool", ""),
    ("value_label", Mode::Method, value_label, &["self"], "", ""),
    ("_locked", Mode::Property, locked, &["self"], "", "The width's direction the axis lock (or Shift) imposes, as of\nthe last hover."),
    ("locked_dir", Mode::Method, locked_dir, &["self", "viewport"], "", "The direction the view's axis lock imposes on the width, or None\n(no lock, or the lock is the base edge's own axis)."),
    ("_corners", Mode::Method, corners, &["self", "width", "angle=None"], "", ""),
    ("_width_and_angle", Mode::Method, width_and_angle, &["self", "cursor", "forced=None"], "", ""),
    ("_perp", Mode::Method, perp, &["self"], "", ""),
    ("_normal", Mode::Method, normal, &["self"], "", ""),
    ("_reset", Mode::Method, reset, &["self"], "", ""),
    ("angle", Mode::Setter, set_angle, &["self", "value"], "", ""),
];
