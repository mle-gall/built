// SPDX-License-Identifier: GPL-3.0-or-later
//! TextTool callbacks on the live document.
use super::*;

fn host_text_default<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.call_method1("_describe", (tool.getattr("_ctx")?,))
}
fn value_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if !tool.getattr("anchor")?.is_none() && !tool.getattr("hover_point")?.is_none() {
        return Ok((
            ({
                let value = tool.call_method1("_state", ("description",))?;
                if value.is_truthy()? {
                    value
                } else {
                    ("…").into_bound_py_any(py)?
                }
            }),
            tool.getattr("hover_point")?,
        )
            .into_bound_py_any(py)?);
    }
    Ok(none(py))
}

fn describe<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let vp = ctx.getattr("viewport")?;
    let (x, y) = screen(&ctx)?;
    let units = imported(py, "built_core.units")?;
    if let Ok(edge) = vp.call_method1("pick_edge", (&x, &y)) {
        if !edge.is_none() {
            let len = crate::math::xyz(&edge.getattr("b")?)?
                .distance(crate::math::xyz(&edge.getattr("a")?)?);
            return units.call_method1("fmt_len", (len,));
        }
    }
    if let Ok(hit) = vp.call_method1("pick_face_any", (x, y)) {
        if !hit.is_none() {
            let face = if hit.is_instance_of::<PyTuple>() {
                hit.get_item(0)?
            } else {
                hit
            };
            for name in ["net_area", "area"] {
                if let Some(area) = face.getattr_opt(name)?.filter(|f| f.is_callable()) {
                    return units.call_method1("fmt_area", (area.call0()?,));
                }
            }
        }
    }
    let world = ctx.getattr("world")?;
    let coord = |axis: &str| {
        world
            .call_method0(axis)?
            .call_method1("__format__", (".2f",))?
            .extract::<String>()
    };
    format!("({}, {}, {})", coord("x")?, coord("y")?, coord("z")?).into_bound_py_any(py)
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("host_text_default", Mode::Method, host_text_default, &["self", "vp", "_"], "", ""),
    ("value_label", Mode::Method, value_label, &["self"], "", ""),
    ("_describe", Mode::Method, describe, &["self", "ctx"], "str", "The default label: what did the anchor click land on?"),
];
