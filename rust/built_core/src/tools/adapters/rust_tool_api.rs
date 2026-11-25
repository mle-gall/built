// SPDX-License-Identifier: GPL-3.0-or-later
//! RustTool callbacks on the live document.
use super::*;

fn pointer<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tool_base")?;
    let ctx = a.get_item(1)?;
    let kind = a.get_item(0)?;
    let t = a.get_item(2)?;
    let screen = optional(&ctx, "screen")?;
    Ok(fields!(py; "type" => &kind, "px" => if !screen.is_none() { (screen.call_method0("x")?, screen.call_method0("y")?,).into_bound_py_any(py)? } else { (0.0, 0.0,).into_bound_py_any(py)? }, "mods" => module.call_method1("_mods", (ctx.getattr_opt("modifiers")?.unwrap_or(0.into_bound_py_any(py)?),))?, "world" => module.call_method1("_tuple", (optional(&ctx, "world")?,))?, "snap_kind" => optional(&optional(&ctx, "snap")?, "kind")?, "t" => &t).into_any())
}
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
    tool.getattr("_handles")?.call_method0("clear")?;
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
    viewport.call_method0("update")?;
    Ok(none(py))
}
fn on_click<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
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
fn on_value<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let value = a.get_item(2)?;
    let viewport = a.get_item(1)?;
    Ok(tool
        .call_method1(
            "_send",
            (
                &viewport,
                fields!(py; "type" => "value", "value" => &value).into_any(),
            ),
        )?
        .get_item("consumed")?)
}
fn on_undo<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let consumed = tool
        .call_method1(
            "_send",
            (&viewport, fields!(py; "type" => "undo").into_any()),
        )?
        .get_item("consumed")?;
    if consumed.is_truthy()? {
        viewport.call_method0("update")?;
    }
    Ok(consumed.clone())
}
fn busy<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("_machine")?.call_method0("busy")
}
fn point<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tool_base")?;
    let key = a.get_item(1)?;
    let tool = a.get_item(0)?;
    let value = tool.call_method1("_state", (&key,))?;
    if value.is_none() {
        return Ok(none(py));
    }
    let mut held = tool.getattr("_held_points")?.call_method1("get", (&key,))?;
    if held.is_none() || held.get_item(0)?.ne(&value)? {
        {
            let value = (&value, module.call_method1("Vec3", positional(&value)?)?)
                .into_bound_py_any(py)?;
            held = value.clone();
            tool.getattr("_held_points")?
                .set_item(&key, value.clone())?;
        }
    }
    held.get_item(1)
}
fn rubber_band_lines<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("_machine")?.call_method0("preview_lines")
}
fn preview<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("_machine")?.call_method0("preview")
}
fn state<'py>(a: &Args<'py>) -> Reply<'py> {
    let [tool, key] = unpack::<2>(a.as_any())?;
    let during = tool.getattr("_during")?;
    if !during.is_none() {
        return during.call_method1("get", (key,));
    }
    let seen = tool.getattr("_seen")?;
    let machine = tool.getattr("_machine")?;
    let serial = machine.getattr("serial")?;
    let fields = if serial.eq(seen.get_item(0)?)? {
        seen.get_item(1)?
    } else {
        let fields = PyDict::new(a.py());
        tool.setattr("_seen", (serial, &fields))?;
        fields.into_any()
    };
    match fields.get_item(&key) {
        Ok(value) => Ok(value),
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyKeyError>(a.py()) => {
            let value = machine.call_method1("get", (&key,))?;
            fields.set_item(key, &value)?;
            Ok(value)
        }
        Err(e) => Err(e),
    }
}
fn send<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let [tool, viewport, event] = unpack::<3>(a.as_any())?;
    let during = PyDict::new(py);
    for key in tool.getattr("_READ_DURING")?.try_iter()? {
        let key = key?;
        during.set_item(&key, tool.call_method1("_state", (&key,))?)?;
    }
    tool.setattr("_during", during)?;
    // Host callbacks may read a point while the machine is mutably borrowed.
    let dispatch = (|| {
        let host = imported(py, "tools.rust_tool_base")?
            .call_method1("_Host", (viewport, tool.getattr("_handles")?, &tool))?;
        tool.getattr("_machine")?
            .call_method1("event", (event, host))
    })();
    tool.setattr("_during", py.None())?;
    dispatch
}
fn preview_faces<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let faces = PyList::empty(py);
    let ctor = imported(py, "built_core.geometry")?.getattr("Face")?;
    for surface in a
        .get_item(0)?
        .getattr("_machine")?
        .call_method0("preview_surfaces")?
        .try_iter()?
    {
        let mut loops = surface?.try_iter()?;
        let outer = loops
            .next()
            .transpose()?
            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("missing outer loop"))?;
        let holes = loops.collect::<PyResult<Vec<_>>>()?;
        let holes = if holes.is_empty() {
            none(py)
        } else {
            PyList::new(py, holes)?.into_any()
        };
        faces.append(ctor.call1((outer, holes))?)?;
    }
    Ok(faces.into_any())
}

#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("_send", Mode::Method, send, &["self", "viewport", "event"], "", ""),
    ("_pointer", Mode::Static, pointer, &["kind", "ctx", "t=0.0"], "", ""),
    ("on_activate", Mode::Method, on_activate, &["self", "viewport"], "None", ""),
    ("on_deactivate", Mode::Method, on_deactivate, &["self", "viewport"], "None", ""),
    ("on_cancel", Mode::Method, on_cancel, &["self", "viewport"], "None", ""),
    ("on_click", Mode::Method, on_click, &["self", "ctx"], "None", ""),
    ("on_hover", Mode::Method, on_hover, &["self", "ctx"], "None", ""),
    ("on_value", Mode::Method, on_value, &["self", "viewport", "value"], "bool", ""),
    ("on_undo", Mode::Method, on_undo, &["self", "viewport"], "bool", ""),
    ("busy", Mode::Method, busy, &["self"], "bool", ""),
    ("_state", Mode::Method, state, &["self", "key"], "", ""),
    ("_point", Mode::Method, point, &["self", "key"], "", "The point as a Vec3 that stays the same object while the value\ndoes: the snap frame tells the chain's start from its first point\nby identity."),
    ("rubber_band_lines", Mode::Method, rubber_band_lines, &["self"], "", ""),
    ("preview_faces", Mode::Method, preview_faces, &["self"], "", ""),
    ("preview", Mode::Method, preview, &["self"], "", ""),
];
