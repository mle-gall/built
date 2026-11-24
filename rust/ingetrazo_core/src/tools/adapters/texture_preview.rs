// SPDX-License-Identifier: GPL-3.0-or-later
//! Placement preview uploads.
use super::*;

fn update<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = py.import("tools.texture_position_metal")?;
    let _tool = a.get_item(0)?;
    let tool = a.get_item(1)?;
    if !(tool.getattr("active")?.is_truthy()?) {
        tool.call_method0("restore")?;
        return Ok(none(py));
    }
    let view = tool.getattr("view")?;
    if view.getattr("renderer")?.is_none() || optional(&view, "_instance_updates")?.is_none() {
        return Ok(none(py));
    }
    let target =
        (tool.getattr("face")?.getattr("key")?, tool.getattr("side")?).into_bound_py_any(py)?;
    if !tool.getattr("_packed")?.is(&view.getattr("packed")?)
        || tool.getattr("_target")?.ne(&target)?
    {
        tool.call_method0("restore")?;
        tool.setattr(
            "_rows",
            module.getattr("TextureToolRows")?.call_method1(
                "prepare",
                (&view, tool.getattr("face")?, tool.getattr("side")?),
            )?,
        )?;
        {
            let [part0, part1] =
                unpack::<2>(&(view.getattr("packed")?, &target).into_bound_py_any(py)?)?;
            tool.setattr("_packed", part0)?;
            tool.setattr("_target", part1)?;
        }
    }
    let uvw = tool.getattr("map")?.call_method0("uvw")?;
    if uvw.is_none()
        || uvw.eq(tool.getattr("signature")?)?
        || (tool.getattr("overlay_only")?.is_truthy()?
            && tool.getattr("gesture_active")?.is_truthy()?)
    {
        return Ok(none(py));
    }
    let start = module.call_method0("perf_counter")?;
    tool.getattr("_rows")?
        .call_method1("upload", (view.getattr("renderer")?, &uvw))?;
    tool.getattr("samples_ms")?.call_method1(
        "append",
        (module
            .call_method0("perf_counter")?
            .sub(&start)?
            .mul(1000)?,),
    )?;
    tool.setattr("signature", &uvw)?;
    Ok(none(py))
}
fn restore<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let tool = a.get_item(0)?;
    if !tool.getattr("_rows")?.is_none()
        && !tool.getattr("view")?.getattr("renderer")?.is_none()
        && tool
            .getattr("_packed")?
            .is(&tool.getattr("view")?.getattr("packed")?)
    {
        tool.getattr("_rows")?.call_method1(
            "upload",
            (tool.getattr("view")?.getattr("renderer")?, py.None()),
        )?;
    }
    {
        let value = none(py);
        tool.setattr("_packed", &value)?;
        tool.setattr("_rows", &value)?;
        tool.setattr("_target", &value)?;
        tool.setattr("signature", &value)?;
    }
    tool.setattr("overlay_only", false)?;
    Ok(none(py))
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("update", Mode::Method, update, &["self", "tool"], "", ""),
    ("restore", Mode::Method, restore, &["self"], "", ""),
];
