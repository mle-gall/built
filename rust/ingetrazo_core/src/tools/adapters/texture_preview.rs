// SPDX-License-Identifier: GPL-3.0-or-later
//! Placement preview uploads.
use super::*;

fn update<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let [preview, tool] = unpack::<2>(a.as_any())?;
    if !tool.getattr("active")?.is_truthy()? {
        preview.call_method0("restore")?;
        return Ok(none(py));
    }
    let view = preview.getattr("view")?;
    let renderer = view.getattr("renderer")?;
    if renderer.is_none() || optional(&view, "_instance_updates")?.is_none() {
        return Ok(none(py));
    }
    let module = imported(py, "tools.texture_position_metal")?;
    let face = tool.getattr("face")?;
    let side = tool.getattr("side")?;
    let target = (face.getattr("key")?, &side).into_bound_py_any(py)?;
    let packed = view.getattr("packed")?;
    if !preview.getattr("_packed")?.is(&packed) || preview.getattr("_target")?.ne(&target)? {
        preview.call_method0("restore")?;
        let rows = module
            .getattr("TextureToolRows")?
            .call_method1("prepare", (&view, face, side))?;
        preview.setattr("_rows", rows)?;
        preview.setattr("_packed", packed)?;
        preview.setattr("_target", target)?;
    }
    let uvw = tool.getattr("map")?.call_method0("uvw")?;
    if uvw.is_none()
        || uvw.eq(preview.getattr("signature")?)?
        || (preview.getattr("overlay_only")?.is_truthy()?
            && tool.getattr("gesture_active")?.is_truthy()?)
    {
        return Ok(none(py));
    }
    let clock = module.getattr("perf_counter")?;
    let start: f64 = clock.call0()?.extract()?;
    preview
        .getattr("_rows")?
        .call_method1("upload", (renderer, &uvw))?;
    let elapsed = clock.call0()?.extract::<f64>()? - start;
    preview
        .getattr("samples_ms")?
        .call_method1("append", (elapsed * 1000.0,))?;
    preview.setattr("signature", uvw)?;
    Ok(none(py))
}
fn restore<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let preview = a.get_item(0)?;
    let rows = preview.getattr("_rows")?;
    if !rows.is_none() {
        let view = preview.getattr("view")?;
        let renderer = view.getattr("renderer")?;
        if !renderer.is_none() && preview.getattr("_packed")?.is(&view.getattr("packed")?) {
            rows.call_method1("upload", (renderer, py.None()))?;
        }
    }
    for name in ["_packed", "_rows", "_target", "signature"] {
        preview.setattr(name, py.None())?;
    }
    preview.setattr("overlay_only", false)?;
    Ok(none(py))
}
fn init<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let [preview, view] = unpack::<2>(a.as_any())?;
    preview.setattr("view", view)?;
    for field in ["_packed", "_rows", "_target", "signature"] {
        preview.setattr(field, py.None())?;
    }
    preview.setattr("overlay_only", false)?;
    preview.setattr("samples_ms", PyList::empty(py))?;
    Ok(none(py))
}

#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("__init__", Mode::Method, init, &["self","view"], "", ""),
    ("update", Mode::Method, update, &["self", "tool"], "", ""),
    ("restore", Mode::Method, restore, &["self"], "", ""),
];
