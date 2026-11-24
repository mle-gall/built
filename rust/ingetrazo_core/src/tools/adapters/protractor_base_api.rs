// SPDX-License-Identifier: GPL-3.0-or-later
//! ProtractorBase callbacks on the live document.
use super::*;

fn value_is_unitless<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    true.into_bound_py_any(py)
}
fn on_key<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.protractor_host")?;
    let key = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let axis = fields!(py; module.getattr("Qt")?.getattr("Key_Right")? => "x", module.getattr("Qt")?.getattr("Key_Left")? => "y", module.getattr("Qt")?.getattr("Key_Up")? => "z").into_any().call_method1("get", (&key,))?;
    Ok(if axis.is_truthy()? {
        tool.call_method1("lock_disc", (&viewport, &axis))?
    } else {
        false.into_bound_py_any(py)?
    })
}
fn wireframe_color<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.protractor_host")?;
    let mut axis: Obj<'py>;
    let tool = a.get_item(0)?;
    let mut v: Obj<'py>;
    let n = tool.call_method0("_axis")?;
    for item in module.getattr("_AXES")?.call_method0("items")?.try_iter()? {
        [axis, v] = unpack::<2>(&item?)?;
        if module
            .getattr("QVector3D")?
            .call_method1("dotProduct", (&n, &v))?
            .abs()?
            .gt(0.999)?
        {
            return module.getattr("_AXIS_RGBA")?.get_item(&axis);
        }
    }
    module.getattr("_OFF_AXIS_RGBA")
}
fn disc_lines<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.protractor_host")?;
    let tool = a.get_item(0)?;
    let sweep = a.get_item(1)?;
    let centre = if !tool.getattr("start_point")?.is_none() {
        tool.getattr("start_point")?
    } else {
        tool.getattr("hover_point")?
    };
    if centre.is_none() {
        return Ok(PyList::empty(py).into_any());
    }
    let tilt = if tool.getattr("_axis_drag_armed")?.is_truthy()? {
        tool.getattr("_axis_drag_live")?
    } else {
        none(py)
    };
    let basis = module.call_method1(
        "plane_axes",
        ((if !tilt.is_none() {
            tilt.clone()
        } else {
            tool.call_method0("_axis")?
        }),),
    )?;
    return Ok(tool.getattr("_machine")?.call_method1(
        "cursor_lines",
        (
            &centre,
            ({
                let items = PyList::empty(py);
                for item in basis.try_iter()? {
                    let p = item?;
                    items.append(p.call_method0("toTuple")?)?;
                }
                items.into_any()
            }),
            tool.getattr("_disc_r")?,
            (if !tool.getattr("hover_point")?.is_none() {
                tool.getattr("hover_point")?.call_method0("toTuple")?
            } else {
                none(py)
            }),
            &sweep,
        ),
    )?);
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("value_is_unitless", Mode::Method, value_is_unitless, &["self"], "bool", "The typed value is an ANGLE: the document's length unit must\nnot scale it («45» was 0.045° in a millimetre model, #176)."),
    ("on_key", Mode::Method, on_key, &["self", "viewport", "key", "modifiers"], "", ""),
    ("wireframe_color", Mode::Property, wireframe_color, &["self"], "", ""),
    ("_disc_lines", Mode::Method, disc_lines, &["self", "sweep=None"], "", ""),
];
