// SPDX-License-Identifier: GPL-3.0-or-later
//! CenterArcTool callbacks on the live document.
use super::*;

fn value_is_unitless<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok({
        let value = (tool.getattr("start_point")?.is_none()).into_bound_py_any(py)?;
        if value.is_truthy()? {
            value
        } else {
            (!tool.getattr("arm_point")?.is_none()).into_bound_py_any(py)?
        }
    })
}
fn vcb_caption<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if tool.getattr("start_point")?.is_none() {
        return ("Segments").into_bound_py_any(py);
    }
    Ok(if !tool.getattr("arm_point")?.is_none() {
        ("Angle").into_bound_py_any(py)?
    } else {
        ("Radius").into_bound_py_any(py)?
    })
}
fn value_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.rust_tools")?;
    let at: Obj<'py>;
    let kind: Obj<'py>;
    let tool = a.get_item(0)?;
    let value: Obj<'py>;
    let label = tool.call_method1("_state", ("label",))?;
    if label.is_none() {
        return Ok(none(py));
    }
    [kind, value, at] = unpack::<3>(&label)?;
    if kind.eq("radius")? {
        let fmt_len = py.import("core.units")?.getattr("fmt_len")?;
        return (
            ("R ")
                .into_bound_py_any(py)?
                .add(fmt_len.call1((&value,))?)?,
            module.call_method1("Vec3", positional(&at)?)?,
        )
            .into_bound_py_any(py);
    }
    let fmt_angle_deg = py.import("core.units")?.getattr("fmt_angle_deg")?;
    Ok((
        fmt_angle_deg.call((&value,), Some(&fields!(py; "signed" => true)))?,
        module.call_method1("Vec3", positional(&at)?)?,
    )
        .into_bound_py_any(py)?)
}
fn plane_points<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if tool.getattr("start_point")?.is_none() {
        return Ok(PyList::empty(py).into_any());
    }
    Ok(PyList::new(
        py,
        [
            tool.getattr("start_point")?,
            (if !tool.getattr("arm_point")?.is_none() {
                tool.getattr("arm_point")?
            } else {
                tool.getattr("hover_point")?
            }),
        ],
    )?
    .into_any())
}
fn sweep_to<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.rust_tools")?;
    let cursor = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.call_method1(
        "_query",
        (
            "sweep_to",
            (module.call_method1("_tuple", (&cursor,))?,).into_bound_py_any(py)?,
        ),
    )
}
fn points<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.rust_tools")?;
    let tool = a.get_item(0)?;
    let sweep_deg = a.get_item(1)?;
    return Ok({
        let items = PyList::empty(py);
        for item in tool
            .call_method1(
                "_query",
                (
                    "points",
                    (py.get_type::<pyo3::types::PyFloat>().call1((&sweep_deg,))?,)
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
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("value_is_unitless", Mode::Method, value_is_unitless, &["self"], "bool", "A segment count and a sweep angle are not lengths (#176)."),
    ("vcb_caption", Mode::Method, vcb_caption, &["self"], "str", ""),
    ("value_label", Mode::Method, value_label, &["self"], "", ""),
    ("plane_points", Mode::Method, plane_points, &["self"], "", ""),
    ("_sweep_to", Mode::Method, sweep_to, &["self", "cursor"], "", ""),
    ("_points", Mode::Method, points, &["self", "sweep_deg"], "", ""),
];
