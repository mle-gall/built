// SPDX-License-Identifier: GPL-3.0-or-later
//! ThreePointArcTool callbacks on the live document.
use super::*;

fn on_value<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.rust_tools")?;
    let tool = a.get_item(0)?;
    let value = a.get_item(2)?;
    let viewport = a.get_item(1)?;
    if value.is_instance(&py.get_type::<pyo3::types::PyTuple>().into_any())?
        || !tool.getattr("start_point")?.is_none()
    {
        return false.into_bound_py_any(py);
    }
    module
        .getattr("_ArcBase")?
        .call_method1("on_value", (&tool, &viewport, &value))
}
fn plane_points<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if tool.getattr("start_point")?.is_none() {
        return Ok(PyList::empty(py).into_any());
    }
    if tool.getattr("mid_point")?.is_none() {
        return Ok(PyList::new(
            py,
            [tool.getattr("start_point")?, tool.getattr("hover_point")?],
        )?
        .into_any());
    }
    Ok(PyList::new(
        py,
        [
            tool.getattr("start_point")?,
            tool.getattr("mid_point")?,
            tool.getattr("hover_point")?,
        ],
    )?
    .into_any())
}
fn points<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.rust_tools")?;
    let end = a.get_item(1)?;
    let tool = a.get_item(0)?;
    return Ok({
        let items = PyList::empty(py);
        for item in tool
            .call_method1(
                "_query",
                (
                    "points",
                    (module.call_method1("_tuple", (&end,))?,).into_bound_py_any(py)?,
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
    ("on_value", Mode::Method, on_value, &["self", "viewport", "value"], "bool", ""),
    ("plane_points", Mode::Method, plane_points, &["self"], "", ""),
    ("_points", Mode::Method, points, &["self", "end"], "", ""),
];
