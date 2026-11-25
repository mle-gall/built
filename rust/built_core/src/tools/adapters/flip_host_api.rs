// SPDX-License-Identifier: GPL-3.0-or-later
//! FlipHost callbacks on the live document.
use super::*;

fn status_clause<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.flip_host")?;
    let tool = a.get_item(0)?;
    module.call_method1(
        "ctrl_clause",
        (tool.getattr("cursor_plus")?, "copy", "original"),
    )
}
fn host_empty_flip<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.flip_host")?;
    let vp = a.get_item(1)?;
    vp.call_method1(
        "flash_status",
        (module.call_method1("tr", ("Select the geometry to flip first",))?,),
    )?;
    Ok(none(py))
}
fn host_flip_targets<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.flip_host")?;
    let axis = a.get_item(2)?;
    let hi: Obj<'py>;
    let lo: Obj<'py>;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    [lo, hi] = unpack::<2>(&tool.call_method1("_bounds", (&vp,))?)?;
    let sel = vp.getattr("scene")?.getattr("selection")?;
    if !(tool
        .call_method1("host_flip_selected", (&vp, py.None()))?
        .is_truthy()?)
    {
        return Ok(none(py));
    }
    let centre = if !lo.is_none() {
        lo.add(&hi)?.mul(0.5)?
    } else {
        module.call_method0("QVector3D")?
    };
    return Ok(fields!(py; "center" => &centre, "axis" => module.getattr("_AXES")?.get_item(&axis)?, "placements" => {let items=PyList::empty(py); for item in sel.try_iter()? { let g = item?;
if !(g.is_instance(&module.getattr("Group")?)?) { continue; }
items.append(g.getattr("uid")?)?;
} items.into_any()}, "faces" => {let items=PyList::empty(py); for item in sel.try_iter()? { let f = item?;
if !(f.is_instance(&module.getattr("Face")?)?) { continue; }
items.append(py.get_type::<pyo3::types::PyString>().call1((f.getattr("key")?,))?)?;
} items.into_any()}, "edges" => {let items=PyList::empty(py); for item in sel.try_iter()? { let e = item?;
if !(e.is_instance(&module.getattr("Edge")?)?) { continue; }
items.append(py.get_type::<pyo3::types::PyString>().call1((e.getattr("key")?,))?)?;
} items.into_any()}).into_any());
}
fn host_flip_copy<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.flip_host")?;
    let active: Obj<'py>;
    let arguments = a.get_item(2)?;
    let say: Obj<'py>;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    [active, say] = unpack::<2>(&arguments)?;
    tool.getattr("_during")?.set_item("copy", active.clone())?;
    if say.is_truthy()? {
        vp.call_method1(
            "flash_status",
            ((if active.is_truthy()? {
                module.call_method1("tr", ("Flip a copy: on",))?
            } else {
                module.call_method1("tr", ("Flip a copy: off",))?
            }),),
        )?;
    }
    tool.call_method1("host_flip_cursor", (&vp, py.None()))?;
    tool.call_method1("refresh_hint", (&vp,))?;
    vp.call_method0("update")?;
    Ok(none(py))
}
fn host_flip_cursor<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let active = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    if !active.is_none() {
        tool.getattr("_during")?.set_item("copy", active.clone())?;
    }
    let callback = optional(&vp, "_apply_tool_cursor")?;
    if callback.is_truthy()? {
        callback.call0()?;
    }
    Ok(none(py))
}
fn on_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.flip_host")?;
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let name = fields!(py; module.getattr("Qt")?.getattr("Key_Control")? => "Control", module.getattr("Qt")?.getattr("Key_Right")? => "Right", module.getattr("Qt")?.getattr("Key_Left")? => "Left", module.getattr("Qt")?.getattr("Key_Up")? => "Up").into_any().call_method1("get", (&key,))?;
    if name.is_none() {
        return false.into_bound_py_any(py);
    }
    let result = tool.call_method1(
        "_send",
        (&vp, fields!(py; "type" => "key", "key" => &name).into_any()),
    )?;
    vp.call_method0("update")?;
    result.get_item("consumed")
}
fn flip<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let axis = a.get_item(2)?;
    let copy = a.get_item(3)?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let result = tool.call_method1(
        "_send",
        (
            &vp,
            fields!(py; "type" => "flip_along", "axis" => &axis, "copy" => &copy).into_any(),
        ),
    )?;
    if result.get_item("committed")?.is_truthy()? {
        vp.call_method0("update")?;
    }
    result.get_item("committed")
}
fn bounds<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let viewport = a.get_item(1)?;
    let selection_bounds = imported(py, "tools.rust_tool_base")?.getattr("selection_bounds")?;
    selection_bounds
        .call1((viewport.getattr("scene")?,))?
        .call_method0("limits")
}
fn plane_quad<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.flip_host")?;
    let axis = a.get_item(1)?;
    let centre = a.get_item(2)?;
    let half = a.get_item(3)?;
    let margin = a.get_item(4)?;
    return Ok({
        let items = PyList::empty(py);
        for item in module
            .getattr("ToolMachine")?
            .call_method1(
                "flip_quad",
                (
                    &axis,
                    centre.call_method0("toTuple")?,
                    half.call_method0("toTuple")?,
                    &margin,
                ),
            )?
            .try_iter()?
        {
            let p = item?;
            items.append(module.call_method1("QVector3D", positional(&p)?)?)?;
        }
        items.into_any()
    });
}
fn plane_under_cursor<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let sx = a.get_item(2)?;
    let sy = a.get_item(3)?;
    let viewport = a.get_item(1)?;
    let selection_bounds = imported(py, "tools.rust_tool_base")?.getattr("selection_bounds")?;
    let bounds = selection_bounds.call1((viewport.getattr("scene")?,))?;
    if !bounds.call_method0("limits")?.get_item(0)?.is_none()
        && viewport.hasattr("_world_to_pixel")?
    {
        return bounds.call_method1("flip_pick", (&viewport, (&sx, &sy).into_bound_py_any(py)?));
    }
    Ok(none(py))
}

fn host_flip_selected<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.flip_host")?;
    let mut e: Obj<'py>;
    let vp = a.get_item(1)?;
    return Ok({
        let mut found = false;
        for item in vp.getattr("scene")?.getattr("selection")?.try_iter()? {
            e = item?;
            if imported(py, "builtins")?
                .getattr("isinstance")?
                .call1((
                    &e,
                    (
                        module.getattr("Group")?,
                        module.getattr("Face")?,
                        module.getattr("Edge")?,
                    )
                        .into_bound_py_any(py)?,
                ))?
                .is_truthy()?
            {
                found = true;
                break;
            }
        }
        found.into_bound_py_any(py)?
    });
}
fn host_flip_plane_at<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let px = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    Ok(tool.call_method1("_plane_under_cursor", {
        let arguments = PyList::empty(py);
        arguments.append(&vp)?;
        arguments.call_method1("extend", (&px,))?;
        positional(arguments.as_any())?
    })?)
}
fn on_key_release<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.flip_host")?;
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
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("status_clause", Mode::Method, status_clause, &["self"], "", ""),
    ("host_empty_flip", Mode::Method, host_empty_flip, &["self", "vp", "_"], "", ""),
    ("host_flip_selected", Mode::Method, host_flip_selected, &["self", "vp", "_"], "", ""),
    ("host_flip_targets", Mode::Method, host_flip_targets, &["self", "vp", "axis"], "", ""),
    ("host_flip_plane_at", Mode::Method, host_flip_plane_at, &["self", "vp", "px"], "", ""),
    ("host_flip_copy", Mode::Method, host_flip_copy, &["self", "vp", "args"], "", ""),
    ("host_flip_cursor", Mode::Method, host_flip_cursor, &["self", "vp", "active"], "", ""),
    ("on_key_release", Mode::Method, on_key_release, &["self", "vp", "key"], "", ""),
    ("on_key", Mode::Method, on_key, &["self", "vp", "key", "modifiers"], "", ""),
    ("flip", Mode::Method, flip, &["self", "vp", "axis", "copy=False"], "", ""),
    ("_bounds", Mode::Method, bounds, &["self", "viewport"], "", ""),
    ("_plane_quad", Mode::Method, plane_quad, &["self", "axis", "centre", "half", "margin"], "", ""),
    ("_plane_under_cursor", Mode::Method, plane_under_cursor, &["self", "viewport", "sx:float", "sy:float"], "", ""),
];
