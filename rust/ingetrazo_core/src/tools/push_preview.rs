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

/// The renderer accepts ordinary face records from both tools and extensions.
/// Build the complete surface and wire payload together, including extra caps.
#[pyfunction]
pub fn push_light_faces<'py>(
    tool: &Bound<'py, PyAny>,
    minimum: f64,
) -> PyResult<Bound<'py, pyo3::types::PyList>> {
    use pyo3::types::{PyDict, PyList};
    let py = tool.py();
    let faces = PyList::empty(py);
    let wires = PyList::empty(py);
    tool.setattr("_light_wires", &wires)?;
    let d = tool.getattr("extrusion")?.extract::<f64>()?;
    let base = tool.getattr("base_face")?;
    let normal = tool.getattr("_normal")?;
    if base.is_none() || normal.is_none() || d.abs() < minimum {
        return Ok(faces);
    }
    let facet_cos = tool.getattr("_CURVE_FACET_COS")?.extract()?;
    let face_record = py.import("core.geometry")?.getattr("Face")?;
    let caps = std::iter::once(Ok(base)).chain(tool.getattr("_extra_faces")?.try_iter()?);
    let normals = std::iter::once(Ok(normal)).chain(tool.getattr("_extra_normals")?.try_iter()?);
    for (face, n) in caps.zip(normals) {
        let face = face?;
        let offset = Py::new(py, Vec3::from(xyz(&n?)? * d))?;
        let (cap, walls, edges) = push_sweep_preview(&face, offset.bind(py), facet_cos)?;
        let mut rings = cap.into_iter();
        let outer = rings.next().unwrap_or_default();
        let attrs = face.getattr("attrs")?;
        let material = if attrs.is_truthy()? {
            Some(py.get_type::<PyDict>().call1((attrs,))?)
        } else {
            None
        };
        faces.append(face_record.call1((outer, rings.collect::<Vec<_>>(), material))?)?;
        let paint = crate::push_operation::paint_of(&face)?;
        let paint = paint.is_truthy()?.then_some(paint);
        for wall in walls {
            faces.append(face_record.call1((wall, PyList::empty(py), &paint))?)?;
        }
        wires.call_method1("extend", (edges,))?;
    }
    Ok(faces)
}

pub(super) fn recessed(tool: &Bound<'_, PyAny>, view: &Bound<'_, PyAny>, d: f64) -> PyResult<bool> {
    let cam = view.getattr_opt("camera")?;
    let eye = cam
        .as_ref()
        .map(|c| c.getattr_opt("eye"))
        .transpose()?
        .flatten();
    let anchor = tool.getattr("_anchor")?;
    let normal = tool.getattr("_normal")?;
    if let Some(eye) = eye.filter(|_| !anchor.is_none() && !normal.is_none()) {
        return Ok((xyz(&normal)? * d).dot(xyz(&anchor)? - xyz(&eye.call0()?)?) > 0.);
    }
    Ok(d < 0.)
}
