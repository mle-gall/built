// SPDX-License-Identifier: GPL-3.0-or-later
//! _AnnotationTool callbacks on the live document.
use super::*;

fn host_annotation_pick<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.annotation_host")?;
    let binding: Obj<'py>;
    let point = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let picked_attachment = imported(py, "ingetrazo_core.annotation_anchor")?.getattr("picked_attachment")?;
    if tool
        .getattr("_ctx")?
        .getattr("world")?
        .eq(module.call_method1("Vec3", positional(&point)?)?)?
    {
        binding = picked_attachment.call1((tool.getattr("_ctx")?,))?;
        return Ok(if !binding.is_none() {
            binding.call_method0("archive")?
        } else {
            none(py)
        });
    }
    Ok(none(py))
}

fn pointer<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.annotation_host")?;
    let ctx = a.get_item(2)?;
    let kind = a.get_item(1)?;
    let tool = a.get_item(0)?;
    let t = a.get_item(3)?;
    {
        let [part0, part1] = unpack::<2>(&(&ctx, ctx.getattr("viewport")?).into_bound_py_any(py)?)?;
        tool.setattr("_ctx", part0)?;
        tool.setattr("_vp", part1)?;
    }
    imported(py, "builtins")?
        .getattr("super")?
        .call1((module.getattr("_AnnotationTool")?, &tool))?
        .call_method1("_pointer", (&kind, &ctx, &t))
}
fn send<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.annotation_host")?;
    let event = a.get_item(2)?;
    let tool = a.get_item(0)?;
    let vp = a.get_item(1)?;
    let result = imported(py, "builtins")?
        .getattr("super")?
        .call1((module.getattr("_AnnotationTool")?, &tool))?
        .call_method1("_send", (&vp, &event))?;
    if result.get_item("committed")?.is_truthy()?
        || ("activate", "cancel", "deactivate")
            .into_bound_py_any(py)?
            .contains(event.get_item("type")?)?
    {
        tool.setattr("work_plane", none(py))?;
    }
    Ok(result.clone())
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("_pointer", Mode::Method, pointer, &["self", "kind", "ctx", "t=0.0"], "", ""),
    ("host_annotation_pick", Mode::Method, host_annotation_pick, &["self", "vp", "point"], "", ""),
    ("_send", Mode::Method, send, &["self", "vp", "event"], "", ""),
];
