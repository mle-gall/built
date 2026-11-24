// SPDX-License-Identifier: GPL-3.0-or-later
//! DimensionTool callbacks on the live document.
use super::*;

fn magnet_on<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    (tool.getattr("b")?.is_none()).into_bound_py_any(py)
}
fn owns_shift_lock<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    (!tool.getattr("b")?.is_none()).into_bound_py_any(py)
}
fn shift_held<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let qt = imported(py, "core.input_types")?.getattr("Qt")?;
    let input_state = imported(py, "views.input_state")?.into_any();
    (input_state
        .call_method1("modifiers", (tool.getattr("_vp")?,))?
        .bitand(qt.getattr("ShiftModifier")?)?
        .is_truthy()?)
    .into_bound_py_any(py)
}
fn host_dimension_edge<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.annotation_host")?;
    let ends: Obj<'py>;
    let hit: Obj<'py>;
    let owner: Obj<'py>;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let picked_edge = imported(py, "core.annotation_anchor")?.getattr("picked_edge")?;
    let attachment = imported(py, "core.annotation_anchor")?.getattr("Attachment")?;
    let snap = optional(&tool.getattr("_ctx")?, "snap")?;
    let edge = optional(&snap, "edge")?;
    if optional(&snap, "kind")?.eq("on_edge")?
        && !edge.is_none()
        && !edge.getattr("a")?.is_none()
        && !edge.getattr("b")?.is_none()
    {
        [hit, owner] = unpack::<2>(&picked_edge.call1((vp.getattr("scene")?, &edge))?)?;
        ends = {
            let items = PyList::empty(py);
            for item in (edge.getattr("a")?, edge.getattr("b")?)
                .into_bound_py_any(py)?
                .try_iter()?
            {
                let p = item?;
                items.append(if hit.is_truthy()? {
                    attachment.call_method1("capture", (vp.getattr("scene")?, &hit, &owner, &p))?
                } else {
                    none(py)
                })?;
            }
            items.into_any()
        };
        return Ok((
            module.call_method1("_tuple", (edge.getattr("a")?,))?,
            module.call_method1("_tuple", (edge.getattr("b")?,))?,
            ({
                let items = PyList::empty(py);
                for item in ends.try_iter()? {
                    let e = item?;
                    items.append(if e.is_truthy()? {
                        e.call_method0("archive")?
                    } else {
                        none(py)
                    })?;
                }
                items.into_any()
            }),
        )
            .into_bound_py_any(py)?);
    }
    Ok(none(py))
}
fn proposed<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.annotation_host")?;
    let axis: Obj<'py>;
    let cursor = a.get_item(1)?;
    let offset: Obj<'py>;
    let tool = a.get_item(0)?;
    let dimension = imported(py, "core.dimension")?.getattr("Dimension")?;
    tool.getattr("_machine")?.call_method1(
        "set",
        (
            "propose",
            (
                module.call_method1("_tuple", (&cursor,))?,
                optional(&tool.getattr("_vp")?, "axis_lock")?,
                tool.call_method0("_shift_held")?,
            )
                .into_bound_py_any(py)?,
        ),
    )?;
    [offset, axis] = unpack::<2>(&tool.call_method1("_state", ("proposal",))?)?;
    Ok(dimension.call(
        (
            tool.getattr("a")?,
            tool.getattr("b")?,
            module.call_method1("Vec3", positional(&offset)?)?,
        ),
        Some(&fields!(py; "axis" => &axis)),
    )?)
}
fn rubber_band_lines<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let p: Obj<'py>;
    let q: Obj<'py>;
    let tool = a.get_item(0)?;
    if tool.getattr("a")?.is_none() || tool.getattr("hover_point")?.is_none() {
        return Ok(PyList::empty(py).into_any());
    }
    if tool.getattr("b")?.is_none() {
        return Ok(PyList::new(
            py,
            [(tool.getattr("a")?, tool.getattr("hover_point")?).into_bound_py_any(py)?],
        )?
        .into_any());
    }
    [p, q] = unpack::<2>(
        &tool
            .call_method1("_proposed", (tool.getattr("hover_point")?,))?
            .call_method0("line_points")?,
    )?;
    Ok(PyList::new(
        py,
        [
            (tool.getattr("a")?, &p).into_bound_py_any(py)?,
            (tool.getattr("b")?, &q).into_bound_py_any(py)?,
            (&p, &q).into_bound_py_any(py)?,
        ],
    )?
    .into_any())
}
fn value_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let fmt_len = imported(py, "core.units")?.getattr("fmt_len")?;
    if tool.getattr("a")?.is_none() || tool.getattr("hover_point")?.is_none() {
        return Ok(none(py));
    }
    if tool.getattr("b")?.is_none() {
        return (
            fmt_len.call1((tool
                .getattr("a")?
                .sub(tool.getattr("hover_point")?)?
                .call_method0("length")?,))?,
            tool.getattr("a")?
                .add(tool.getattr("hover_point")?)?
                .mul(0.5)?,
        )
            .into_bound_py_any(py);
    }
    let dim = tool.call_method1("_proposed", (tool.getattr("hover_point")?,))?;
    (
        fmt_len.call1((dim.call_method0("value")?,))?,
        dim.call_method0("midpoint")?,
    )
        .into_bound_py_any(py)
}

fn pointer<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let [tool, kind, ctx, t] = unpack::<4>(a.as_any())?;
    let cls = imported(py, "tools.annotation_host")?.getattr("DimensionTool")?;
    let parent = imported(py, "builtins")?.call_method1("super", (cls, &tool))?;
    let event = parent.call_method1("_pointer", (&kind, &ctx, t))?;
    let camera = optional(&ctx.getattr("viewport")?, "camera")?;
    let origin = tool.getattr("a")?;
    if !origin.is_none()
        && camera.is_truthy()?
        && (kind.eq("hover")? || tool.getattr("b")?.is_none())
    {
        let normal = camera.getattr("target")?.sub(camera.call_method0("eye")?)?;
        let world = imported(py, "core.snap")?.call_method1(
            "project_to_view_plane",
            (ctx.getattr("world")?, origin, normal),
        )?;
        event.set_item("world", world.call_method0("toTuple")?)?;
    }
    if tool.call_method0("_shift_held")?.is_truthy()? {
        let shift = imported(py, "core.input_types")?
            .getattr("Qt")?
            .getattr("ShiftModifier")?
            .getattr("value")?;
        event.set_item("mods", event.get_item("mods")?.bitor(shift)?)?;
    }
    Ok(event)
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("magnet_on", Mode::Method, magnet_on, &["self"], "", ""),
    ("owns_shift_lock", Mode::Method, owns_shift_lock, &["self"], "", ""),
    ("_shift_held", Mode::Method, shift_held, &["self"], "", ""),
    ("_pointer", Mode::Method, pointer, &["self", "kind", "ctx", "t=0.0"], "", ""),
    ("host_dimension_edge", Mode::Method, host_dimension_edge, &["self", "vp", "_"], "", ""),
    ("_proposed", Mode::Method, proposed, &["self", "cursor"], "", ""),
    ("rubber_band_lines", Mode::Method, rubber_band_lines, &["self"], "", ""),
    ("value_label", Mode::Method, value_label, &["self"], "", ""),
];
