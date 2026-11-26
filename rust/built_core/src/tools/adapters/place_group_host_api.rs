// SPDX-License-Identifier: GPL-3.0-or-later
//! PlaceGroupHost callbacks on the live document.
use super::*;

fn on_click<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let ctx = a.get_item(1)?;
    let tool = a.get_item(0)?;
    if !tool.getattr("_group")?.is_none()
        && tool.getattr("_align")?.is_truthy()?
        && tool.getattr("_group")?.getattr("text3d")?.is_truthy()?
    {
        let picked_attachment =
            imported(py, "built_core.annotation_anchor")?.getattr("picked_attachment")?;
        tool.setattr("_attachment", picked_attachment.call1((&ctx,))?)?;
    }
    tool.call_method1(
        "_drop",
        (
            ctx.getattr("viewport")?,
            tool.call_method1("_pointer", ("click", &ctx))?,
        ),
    )?;
    Ok(none(py))
}
fn place_at<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let attachment = a.get_item(3)?;
    let tool = a.get_item(0)?;
    let viewport = a.get_item(1)?;
    let world = a.get_item(2)?;
    tool.setattr("_attachment", &attachment)?;
    tool.call_method1(
        "_drop",
        (
            &viewport,
            fields!(py; "type" => "paste_at", "offset" => world.call_method0("toTuple")?)
                .into_any(),
        ),
    )?;
    Ok(none(py))
}

fn face_frame<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.place_group_host")?;
    let normal = a.get_item(0)?;
    let mut p: Obj<'py>;
    return Ok(py.get_type::<pyo3::types::PyTuple>().call1(({
        let collected = PyList::empty(py);
        for item in module
            .call_method1("ToolMachine", ("place_group",))?
            .call_method1("query", ("basis", normal.call_method0("toTuple")?))?
            .try_iter()?
        {
            p = item?;
            collected.append(module.call_method1("QVector3D", positional(&p)?)?)?;
        }
        collected.into_any()
    },))?);
}
fn drop<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let event = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let window: Obj<'py>;
    let tr = imported(py, "built_core.i18n")?.getattr("tr")?;
    if tool
        .call_method1("_send", (&vp, &event))?
        .get_item("committed")?
        .is_truthy()?
    {
        tool.setattr("_group", none(py))?;
        vp.call_method1(
            "flash_status",
            (tr.call1(("Component placed — Move (M) adjusts it",))?,),
        )?;
        window = vp.call_method0("window")?;
        if window.hasattr("_activate_tool")? {
            window.call_method1("_activate_tool", ("select",))?;
        }
        vp.call_method0("update")?;
    }
    Ok(none(py))
}
fn on_cancel<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.place_group_host")?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    tool.setattr("_group", none(py))?;
    imported(py, "builtins")?
        .getattr("super")?
        .call1((module.getattr("PlaceGroupHost")?, &tool))?
        .call_method1("on_cancel", (&vp,))?;
    Ok(none(py))
}
fn on_deactivate<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.place_group_host")?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    tool.setattr("_group", none(py))?;
    imported(py, "builtins")?
        .getattr("super")?
        .call1((module.getattr("PlaceGroupHost")?, &tool))?
        .call_method1("on_deactivate", (&vp,))?;
    Ok(none(py))
}
fn value_label<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    let values = tool.getattr("_machine")?.call_method1("query", ("wall_values", py.None()))?;
    if values.is_none() { return Ok(none(py)); }
    let units = py.import("built_core.units")?;
    let sill = units.call_method1("fmt_len", (values.get_item(1)?,))?;
    let offset = units.call_method1("fmt_len", (values.get_item(2)?,))?;
    let text = format!("Sill {} · Offset {}", sill.str()?, offset.str()?);
    let at = py.import("built_core")?.getattr("Vec3")?.call(positional(&values.get_item(0)?)?, None)?;
    (text, at).into_bound_py_any(py)
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("value_label", Mode::Method, value_label, &["self"], "", ""),
    ("_face_frame", Mode::Static, face_frame, &["normal"], "", ""),
    ("on_click", Mode::Method, on_click, &["self", "ctx"], "", ""),
    ("place_at", Mode::Method, place_at, &["self", "viewport", "world", "*", "attachment=None"], "", ""),
    ("_drop", Mode::Method, drop, &["self", "vp", "event"], "", ""),
    ("on_cancel", Mode::Method, on_cancel, &["self", "vp"], "", ""),
    ("on_deactivate", Mode::Method, on_deactivate, &["self", "vp"], "", ""),
];
