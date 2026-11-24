// SPDX-License-Identifier: GPL-3.0-or-later
//! LineTool callbacks on the live document.
use super::*;

fn chain_first_point<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if ({
        let value = tool.call_method1("_state", ("chain_vertices",))?;
        if value.is_truthy()? {
            value
        } else {
            PyTuple::empty(py).into_any()
        }
    })
    .len()?
        <= 1
    {
        return tool.getattr("start_point");
    }
    tool.call_method1("_point", ("chain_first_point",))
}
fn set_chain_first_point<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.rust_tools")?;
    let tool = a.get_item(0)?;
    let value = a.get_item(1)?;
    tool.getattr("_machine")?.call_method1(
        "set",
        (
            "chain_first_point",
            module.call_method1("_tuple", (&value,))?,
        ),
    )?;
    Ok(none(py))
}
fn chain_vertices<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.rust_tools")?;
    let tool = a.get_item(0)?;
    return Ok({
        let items = PyList::empty(py);
        for item in tool
            .call_method1("_state", ("chain_vertices",))?
            .try_iter()?
        {
            let p = item?;
            items.append(module.call_method1("Vec3", positional(&p)?)?)?;
        }
        items.into_any()
    });
}
fn set_chain_vertices<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.rust_tools")?;
    let points = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.getattr("_machine")?.call_method1(
        "set",
        (
            "chain_vertices",
            ({
                let items = PyList::empty(py);
                for item in points.try_iter()? {
                    let p = item?;
                    items.append(module.call_method1("_tuple", (&p,))?)?;
                }
                items.into_any()
            }),
        ),
    )?;
    Ok(none(py))
}
fn work_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.rust_tools")?;
    let tool = a.get_item(0)?;
    let plane = tool.call_method1("_state", ("work_plane",))?;
    Ok(if plane.is_none() {
        none(py)
    } else {
        (
            module.call_method1("Vec3", positional(&plane.get_item(0)?)?)?,
            module.call_method1("Vec3", positional(&plane.get_item(1)?)?)?,
        )
            .into_bound_py_any(py)?
    })
}
fn set_work_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.rust_tools")?;
    let plane = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.getattr("_machine")?.call_method1(
        "set",
        (
            "work_plane",
            (if plane.is_none() {
                none(py)
            } else {
                (
                    module.call_method1("_tuple", (plane.get_item(0)?,))?,
                    module.call_method1("_tuple", (plane.get_item(1)?,))?,
                )
                    .into_bound_py_any(py)?
            }),
        ),
    )?;
    Ok(none(py))
}
fn last<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    Ok(if tool.call_method1("_state", ("retype",))?.is_truthy()? {
        true.into_bound_py_any(py)?
    } else {
        none(py)
    })
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("chain_first_point", Mode::Property, chain_first_point, &["self"], "", ""),
    ("chain_vertices", Mode::Property, chain_vertices, &["self"], "", ""),
    ("work_plane", Mode::Property, work_plane, &["self"], "", ""),
    ("_last", Mode::Property, last, &["self"], "", "Truthy while a typed value would retype the segment just drawn."),
    ("chain_first_point", Mode::Setter, set_chain_first_point, &["self", "value"], "", ""),
    ("chain_vertices", Mode::Setter, set_chain_vertices, &["self", "points"], "", ""),
    ("work_plane", Mode::Setter, set_work_plane, &["self", "plane"], "", ""),
];
