// SPDX-License-Identifier: GPL-3.0-or-later
//! Depth-tested group edit frame and the projected boundary for leaving it.
use super::move_grips::convex_hull;
use crate::math::{xyz, Vec3};
use glam::{DVec2, DVec3};
use numpy::{IntoPyArray, PyArray1};
use pyo3::prelude::*;

fn corners(frame: [DVec3; 3], lo: DVec3, hi: DVec3) -> [DVec3; 8] {
    std::array::from_fn(|i| {
        let coordinate = |axis: usize| {
            if i & (1 << axis) != 0 {
                hi[axis]
            } else {
                lo[axis]
            }
        };
        frame[0] * coordinate(0) + frame[1] * coordinate(1) + frame[2] * coordinate(2)
    })
}

fn project(view: &Bound<'_, PyAny>, corner: DVec3) -> PyResult<Option<(f64, f64)>> {
    view.call_method1("_world_to_pixel", (Vec3::from(corner),))?
        .extract()
}

#[pyfunction]
pub fn edit_group_box(view: &Bound<'_, PyAny>) -> PyResult<Option<Vec<Vec3>>> {
    let group = view.getattr("scene")?.getattr("edit_group")?;
    if group.is_none()
        || (!group.getattr("mesh")?.getattr("vertices")?.is_truthy()?
            && !group
                .getattr_opt("children")?
                .map_or(Ok(false), |v| v.is_truthy())?)
    {
        return Ok(None);
    }
    let (frame, lo, hi): ([Bound<PyAny>; 3], [f64; 3], [f64; 3]) =
        view.call_method1("_group_obb", (&group,))?.extract()?;
    let frame = [xyz(&frame[0])?, xyz(&frame[1])?, xyz(&frame[2])?];
    let (lo, hi) = (DVec3::from(lo), DVec3::from(hi));
    let tight = corners(frame, lo, hi);
    let mut min = DVec2::splat(f64::INFINITY);
    let mut max = -min;
    for corner in tight {
        if let Some((x, y)) = project(view, corner)? {
            let point = DVec2::new(x, y);
            min = min.min(point);
            max = max.max(point);
        }
    }
    let pad_px = (0.05 * (max - min).max_element()).clamp(8., 32.);
    let camera = view.getattr("camera")?;
    let mut span: f64 = camera.getattr("distance")?.extract()?;
    if camera.getattr("perspective")?.is_truthy()? {
        let centre = tight.into_iter().sum::<DVec3>() / 8.;
        let depth = (centre - xyz(&camera.call_method0("eye")?)?)
            .dot(xyz(&camera.call_method0("forward")?)?);
        if depth.is_finite() && depth > 1e-9 {
            span = depth;
        }
    }
    let fov: f64 = camera.getattr("fov_deg")?.extract()?;
    let height: f64 = view.call_method0("height")?.extract()?;
    let pad = pad_px * (2. * span.max(1e-9) * (fov.to_radians() / 2.).tan() / height.max(1.));
    Ok(Some(
        corners(frame, lo - pad, hi + pad)
            .into_iter()
            .map(Vec3::from)
            .collect(),
    ))
}

#[pyfunction]
pub fn box_edge_vertices<'py>(
    py: Python<'py>,
    corners: [Bound<'py, PyAny>; 8],
) -> PyResult<Bound<'py, PyArray1<f32>>> {
    let mut pts = [DVec3::ZERO; 8];
    for (dst, src) in pts.iter_mut().zip(corners) {
        *dst = xyz(&src)?;
    }
    let mut coords = Vec::with_capacity(72);
    for i in 0..8 {
        for bit in [1, 2, 4] {
            let j = i | bit;
            if i != j && pts[i].distance(pts[j]) >= 1e-9 {
                coords.extend(pts[i].as_vec3().to_array());
                coords.extend(pts[j].as_vec3().to_array());
            }
        }
    }
    Ok(coords.into_pyarray(py))
}

fn contains(points: Vec<DVec2>, p: DVec2, tolerance: f64) -> bool {
    let hull = convex_hull(points);
    match hull.as_slice() {
        [] => false,
        [a] => (p.x - a.x).hypot(p.y - a.y) <= tolerance,
        [a, b] => {
            let edge = *b - *a;
            let l2 = edge.length_squared();
            let t = if l2 <= 1e-12 {
                0.
            } else {
                ((p - *a).dot(edge) / l2).clamp(0., 1.)
            };
            let delta = p - (*a + edge * t);
            delta.x.hypot(delta.y) <= tolerance
        }
        _ => hull
            .iter()
            .zip(hull.iter().cycle().skip(1))
            .all(|(&a, &b)| {
                let edge = b - a;
                edge.perp_dot(p - a) >= -tolerance * edge.x.hypot(edge.y).max(1.)
            }),
    }
}

#[pyfunction(signature=(x, y, points, tolerance_px=1.5))]
pub fn point_in_screen_hull(
    x: f64,
    y: f64,
    points: &Bound<'_, PyAny>,
    tolerance_px: f64,
) -> PyResult<bool> {
    let mut finite = Vec::new();
    for p in points.try_iter()? {
        let point = DVec2::from(p?.extract::<[f64; 2]>()?);
        if point.is_finite() {
            finite.push(point);
        }
    }
    Ok(contains(finite, DVec2::new(x, y), tolerance_px))
}

#[pyfunction]
pub fn edit_box_contains_pixel(view: &Bound<'_, PyAny>, x: f64, y: f64) -> PyResult<bool> {
    let corners = view.call_method0("_edit_group_box_corners")?;
    if corners.is_none() {
        return Ok(false);
    }
    let mut points = Vec::new();
    for p in corners.try_iter()? {
        if let Some((x, y)) = project(view, xyz(&p?)?)? {
            if x.is_finite() && y.is_finite() {
                points.push(DVec2::new(x, y));
            }
        }
    }
    Ok(contains(points, DVec2::new(x, y), 1.5))
}
