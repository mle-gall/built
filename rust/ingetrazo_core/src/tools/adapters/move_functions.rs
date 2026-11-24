// SPDX-License-Identifier: GPL-3.0-or-later
//! Functions exported for legacy tool callers.
use super::*;

fn gather_dimensions<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.move_host")?;
    let ctx = a.get_item(0)?;
    let groups = a.get_item(1)?;
    let vertices = a.get_item(2)?;
    let dimension_modes = imported(py, "core.move_ops")?.getattr("dimension_modes")?;
    dimension_modes.call1((
        module.call_method1("transform_records", (&ctx, "dimension"))?,
        &groups,
        &vertices,
    ))
}

fn gather_images<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.move_host")?;
    let ctx = a.get_item(0)?;
    module.call_method1("transform_records", (&ctx, "image_plane"))
}

#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("gather_images", Mode::Function, gather_images, &["ctx"], "", ""),
    ("gather_dimensions", Mode::Function, gather_dimensions, &["ctx", "groups", "vertices"], "", ""),
];
