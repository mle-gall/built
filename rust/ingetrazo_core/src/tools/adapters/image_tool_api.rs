// SPDX-License-Identifier: GPL-3.0-or-later
//! ImageTool callbacks on the live document.
use super::*;

fn work_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let tool = a.get_item(0)?;
    tool.getattr("_plane")
}
fn set_work_plane<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let value = a.get_item(1)?;
    let plane_axes = py.import("core.axes")?.getattr("plane_axes")?;
    tool.setattr("_plane", &value)?;
    tool.getattr("_machine")?.call_method1(
        "set",
        (
            "axes",
            (if value.is_truthy()? {
                {
                    let items = PyList::empty(py);
                    for item in plane_axes
                        .call1((value.get_item(1)?.call_method0("normalized")?,))?
                        .try_iter()?
                    {
                        let p = item?;
                        items.append(p.call_method0("toTuple")?)?;
                    }
                    items.into_any()
                }
            } else {
                none(py)
            }),
        ),
    )?;
    Ok(none(py))
}
fn load<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let aspect = a.get_item(2)?;
    let label = a.get_item(3)?;
    let path = a.get_item(1)?;
    let tool = a.get_item(0)?;
    tool.setattr("_plane", none(py))?;
    tool.getattr("_machine")?.call_method1(
        "set",
        (
            "load",
            (
                py.get_type::<pyo3::types::PyString>().call1((&path,))?,
                py.get_type::<pyo3::types::PyFloat>().call1((({
                    let value = aspect.clone();
                    if value.is_truthy()? {
                        value
                    } else {
                        1.into_bound_py_any(py)?
                    }
                }),))?,
                ({
                    let value = label.clone();
                    if value.is_truthy()? {
                        value
                    } else {
                        ("").into_bound_py_any(py)?
                    }
                }),
            )
                .into_bound_py_any(py)?,
        ),
    )?;
    Ok(none(py))
}
fn value_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let u: Obj<'py>;
    let v: Obj<'py>;
    let fmt_pair = py.import("core.units")?.getattr("fmt_pair")?;
    if tool.getattr("armed")?.is_truthy()?
        && !tool.getattr("start_point")?.is_none()
        && !tool.getattr("hover_point")?.is_none()
    {
        [u, v] = unpack::<2>(&tool.call_method1(
            "_frame",
            (tool.getattr("start_point")?, tool.getattr("hover_point")?),
        )?)?;
        return (
            fmt_pair
                .call1((u.call_method0("length")?, v.call_method0("length")?))?
                .add(if tool.getattr("_free_aspect")?.is_truthy()? {
                    ("  (libre)").into_bound_py_any(py)?
                } else {
                    ("").into_bound_py_any(py)?
                })?,
            tool.getattr("start_point")?.add(u.add(&v)?.mul(0.5)?)?,
        )
            .into_bound_py_any(py);
    }
    Ok(none(py))
}

fn send<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.image_host")?;
    let event = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let reply = py
        .import("builtins")?
        .getattr("super")?
        .call1((module.getattr("ImageTool")?, &tool))?
        .call_method1("_send", (&vp, &event))?;
    if reply.get_item("committed")?.is_truthy()?
        || ("activate", "deactivate", "cancel")
            .into_bound_py_any(py)?
            .contains(event.get_item("type")?)?
    {
        tool.setattr("_plane", none(py))?;
    }
    if reply.get_item("committed")?.is_truthy()? && vp.hasattr("finish_image_placement")? {
        vp.call_method0("finish_image_placement")?;
    }
    Ok(reply.clone())
}
fn frame<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.image_host")?;
    let first = a.get_item(1)?;
    let b = a.get_item(2)?;
    let mut p: Obj<'py>;
    let tool = a.get_item(0)?;
    return Ok(py.get_type::<pyo3::types::PyTuple>().call1(({
        let collected = PyList::empty(py);
        for item in tool
            .getattr("_machine")?
            .call_method1(
                "query",
                (
                    "frame",
                    PyList::new(
                        py,
                        [
                            module.call_method1("_tuple", (&first,))?,
                            module.call_method1("_tuple", (&b,))?,
                        ],
                    )?
                    .into_any(),
                ),
            )?
            .try_iter()?
        {
            p = item?;
            collected.append(module.call_method1("Vec3", positional(&p)?)?)?;
        }
        collected.into_any()
    },))?);
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("work_plane", Mode::Property, work_plane, &["self"], "", ""),
    ("load", Mode::Method, load, &["self", "path", "aspect", "label=''"], "", ""),
    ("_send", Mode::Method, send, &["self", "vp", "event"], "", ""),
    ("_frame", Mode::Method, frame, &["self", "a", "b"], "", ""),
    ("value_label", Mode::Method, value_label, &["self"], "", ""),
    ("work_plane", Mode::Setter, set_work_plane, &["self", "value"], "", ""),
];
