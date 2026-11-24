// SPDX-License-Identifier: GPL-3.0-or-later
//! Extrusion preview geometry and inference for the Push/Pull machine.
use super::{host_geometry::Projection, view_queries::hit};
use crate::math::{xyz, Vec3};
use glam::DVec3;
use pyo3::prelude::*;

type Loop = Vec<Vec3>;
type Wire = (Vec3, Vec3);

/// One translated cap with holes, wall quads, and the edges that stay hard
/// after the store's finish pass. The base outline is already resident.
#[pyfunction]
pub fn push_sweep_preview(
    face: &Bound<'_, PyAny>,
    offset: &Bound<'_, PyAny>,
    facet_cos: f64,
) -> PyResult<(Vec<Loop>, Vec<Loop>, Vec<Wire>)> {
    let offset = xyz(offset)?;
    let mut loops = vec![face.getattr("vertices")?];
    for hole in face.getattr("holes")?.try_iter()? {
        loops.push(hole?);
    }
    let mut caps = Vec::new();
    let mut walls = Vec::new();
    let mut wires = Vec::new();
    for ring in loops {
        let low = ring
            .try_iter()?
            .map(|p| xyz(&p?))
            .collect::<PyResult<Vec<_>>>()?;
        let high: Vec<_> = low.iter().map(|p| *p + offset).collect();
        let count = low.len();
        let pairs = (0..count).map(|i| (i, (i + 1) % count));
        let normals: Vec<_> = pairs
            .clone()
            .map(|(i, j)| {
                let n = (low[j] - low[i]).cross(offset);
                (n.length() > 1e-12).then(|| n.normalize())
            })
            .collect();
        for (i, j) in pairs {
            walls.push(vec![
                low[i].into(),
                low[j].into(),
                high[j].into(),
                high[i].into(),
            ]);
            if count < 2 {
                continue;
            }
            wires.push((high[i].into(), high[j].into()));
            let smooth = normals[(i + count - 1) % count]
                .zip(normals[i])
                .is_some_and(|(a, b)| {
                    let dot = a.dot(b);
                    facet_cos < dot && dot < 0.99995
                });
            if !smooth {
                wires.push((low[i].into(), high[i].into()));
            }
        }
        caps.push(high.into_iter().map(Vec3::from).collect());
    }
    Ok((caps, walls, wires))
}

/// Closest projected point of a guide line to the cursor ray. Guides parallel
/// to the extrusion axis cannot supply a target height.
#[pyfunction]
pub fn push_guide_reference(
    view: &Bound<'_, PyAny>,
    guides: &Bound<'_, PyAny>,
    px: [f64; 2],
    radius: f64,
    anchor: &Bound<'_, PyAny>,
    normal: &Bound<'_, PyAny>,
    min_distance: f64,
) -> PyResult<Option<(Vec3, f64)>> {
    let mut lines = Vec::new();
    for g in guides.try_iter()? {
        let g = g?;
        if let Some(d) = g.getattr_opt("direction")?.filter(|d| !d.is_none()) {
            lines.push((xyz(&g.getattr("point")?)?, xyz(&d)?));
        }
    }
    if lines.is_empty() {
        return Ok(None);
    }
    let Some(ray) = hit(view, "_pixel_to_ray", (px[0], px[1]))? else {
        return Ok(None);
    };
    let (origin, dir) = (ray.get_item(0)?, ray.get_item(1)?);
    if origin.is_none() || dir.is_none() {
        return Ok(None);
    }
    let (origin, dir) = (xyz(&origin)?, xyz(&dir)?);
    let camera = Projection::from_view(view)?;
    let (anchor, normal) = (xyz(anchor)?, xyz(normal)?);
    let mut best: Option<(f64, DVec3, f64)> = None;
    for (point, axis) in lines {
        if axis.dot(normal).abs() > 0.999 {
            continue;
        }
        let (a, b, c) = (axis.length_squared(), axis.dot(dir), dir.length_squared());
        let den = a * c - b * b;
        if den.abs() < 1e-12 {
            continue;
        }
        let w = point - origin;
        let on = point + axis * ((b * dir.dot(w) - c * axis.dot(w)) / den);
        let dist = (on - anchor).dot(normal);
        if dist.abs() < min_distance {
            continue;
        }
        if let Some([x, y]) = camera.pixel(on)? {
            let dd = (x - px[0]).powi(2) + (y - px[1]).powi(2);
            if dd <= radius * radius && best.as_ref().is_none_or(|v| dd < v.0) {
                best = Some((dd, on, dist));
            }
        }
    }
    Ok(best.map(|(_, p, d)| (p.into(), d)))
}
