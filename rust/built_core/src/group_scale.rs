// SPDX-License-Identifier: GPL-3.0-or-later
//! Read and change scale along a placement's own axes.
use crate::math::{xyz, Vec3};
use glam::DVec3;
use pyo3::{prelude::*, types::PyTuple};
type Obj<'p> = Bound<'p, PyAny>;
fn scale(frame: &Obj<'_>) -> PyResult<[f64; 3]> {
    if frame.is_none() {
        return Ok([1.; 3]);
    }
    let mut size = [0.; 3];
    for (i, unit) in [DVec3::X, DVec3::Y, DVec3::Z].into_iter().enumerate() {
        size[i] = xyz(&frame.call_method1("mapVector", (Vec3::from(unit),))?)?.length();
    }
    Ok(size)
}
#[pyfunction]
pub(crate) fn group_scale(group: &Obj<'_>) -> PyResult<(f64, f64, f64)> {
    let frame = group
        .py()
        .import("core.group")?
        .call_method1("group_frame", (group,))?;
    Ok(scale(&frame)?.into())
}
#[pyfunction]
pub(crate) fn group_scale_command<'p>(
    group: &Obj<'p>,
    target: [f64; 3],
) -> PyResult<Option<Obj<'p>>> {
    if target.iter().any(|&v| v <= 1e-9) {
        return Ok(None);
    }
    let py = group.py();
    let api = py.import("core.group")?;
    let frame = api.call_method1("group_frame", (group,))?;
    let now = scale(&frame)?;
    let ratios = std::array::from_fn::<_, 3, _>(|i| {
        if now[i] > 1e-9 {
            target[i] / now[i]
        } else {
            1.
        }
    });
    if ratios.iter().all(|r| (r - 1.).abs() < 1e-6) {
        return Ok(None);
    }
    let axes = api.call_method1("frame_axes", (frame,))?;
    let center = if !group.getattr("xform")?.is_none() {
        axes.get_item(0)?
    } else {
        let mut min = DVec3::splat(f64::INFINITY);
        let mut max = -min;
        for v in group.getattr("mesh")?.getattr("vertices")?.try_iter()? {
            let p = xyz(&v?.getattr("position")?)?;
            min = min.min(p);
            max = max.max(p);
        }
        let p = if min.x.is_infinite() {
            DVec3::ZERO
        } else {
            (min + max) * 0.5
        };
        Bound::new(py, Vec3::from(p))?.into_any()
    };
    let basis = PyTuple::new(
        py,
        [axes.get_item(1)?, axes.get_item(2)?, axes.get_item(3)?],
    )?;
    Ok(Some(py.import("built_core.history")?.call_method1(
        "ScaleGroupCommand",
        (group, center, PyTuple::new(py, ratios)?, basis),
    )?))
}
