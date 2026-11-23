// SPDX-License-Identifier: GPL-3.0-or-later
//! Install intersection lines, then partition all affected faces in first-cut order.
use super::{
    drawing::{point, vector},
    drawing_plane::tris,
    fillet::newell,
    geometry::length,
    intersect::PyPolygon,
};
use crate::math::{xyz, Vec3};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};

/// Keep the same facade on both sides of a split: the store can materialise
/// while splitting a detached handle, and the half edges inherit all flags.
#[pyfunction]
pub(crate) fn partition_intersections<'py>(
    mesh: &Bound<'py, PyAny>,
    segments: &Bound<'py, PyAny>,
    split_plan: &Bound<'py, PyAny>,
    read_flags: &Bound<'py, PyAny>,
    stamp_flags: &Bound<'py, PyAny>,
    scan: &Bound<'py, PyAny>,
    polygon: &Bound<'py, PyAny>,
    replace: &Bound<'py, PyAny>,
) -> PyResult<()> {
    let py = mesh.py();
    let lines = PyDict::new(py);
    for seg in segments.try_iter()? {
        let seg = seg?;
        let plan = split_plan.call1((seg.get_item(0)?, seg.get_item(1)?))?;
        let cuts = plan.get_item(1)?;
        for (edge, p) in cuts.cast::<PyDict>()? {
            let flags = read_flags.call1((&edge,))?;
            let key = edge.getattr("key")?;
            let cut = lines.contains(&key)?;
            if cut {
                lines.del_item(key)?;
            }
            for half in mesh.call_method1("split_edge", (edge, p))?.try_iter()? {
                let half = half?;
                stamp_flags.call1((&half, &flags))?;
                if cut {
                    lines.set_item(half.getattr("key")?, &half)?;
                }
            }
        }
        for pair in plan.get_item(0)?.try_iter()? {
            let pair = pair?;
            let e = mesh.call_method1("add_edge", (pair.get_item(0)?, pair.get_item(1)?))?;
            lines.set_item(e.getattr("key")?, &e)?;
        }
    }
    // Polygons see the entire split line graph before any face is replaced.
    let index = scan.call1((mesh,))?;
    let polys = PyDict::new(py);
    let by_face = PyDict::new(py);
    for (_, edge) in lines.iter() {
        let a = xyz(&edge.getattr("a")?)?;
        let b = xyz(&edge.getattr("b")?)?;
        let candidates = if index.is_none() {
            mesh.getattr("faces")?
        } else {
            let lo = a.min(b) - 1e-4;
            let hi = a.max(b) + 1e-4;
            let bounds = ((lo.x, lo.y, lo.z), (hi.x, hi.y, hi.z));
            index.call_method1("faces", (index.call_method1("faces_meeting", (bounds,))?,))?
        };
        for face in candidates.try_iter()? {
            let face = face?;
            let key = face.getattr("key")?;
            let poly = if let Some(poly) = polys.get_item(&key)? {
                poly
            } else {
                let poly = polygon.call1((&face,))?;
                polys.set_item(&key, &poly)?;
                poly
            };
            if poly.is_none() {
                continue;
            }
            let clipped = poly
                .getattr("native")?
                .cast::<PyPolygon>()?
                .borrow()
                .clip(a.to_array(), b.to_array());
            if clipped.is_empty() {
                continue;
            }
            let bucket = if let Some(bucket) = by_face.get_item(&face)? {
                bucket.cast_into::<PyList>()?
            } else {
                let bucket = PyList::empty(py);
                by_face.set_item(&face, &bucket)?;
                bucket
            };
            for (p, q) in clipped {
                bucket.append((
                    Vec3::from(glam::DVec3::from(p)),
                    Vec3::from(glam::DVec3::from(q)),
                ))?;
            }
        }
    }
    for (face, cuts) in by_face.iter() {
        replace.call1((mesh, &face, polys.get_item(face.getattr("key")?)?, cuts))?;
    }
    Ok(())
}

pub(super) fn replace_regions(
    poly: &PyPolygon,
    mesh: &Bound<'_, PyAny>,
    face: &Bound<'_, PyAny>,
    regions: &Bound<'_, PyAny>,
    copy: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let mut kept = Vec::new();
    for pair in regions.try_iter()? {
        let pair = pair?;
        let outer = pair.get_item(0)?;
        let holes = pair.get_item(1)?;
        let coordinates = |r: &Bound<'_, PyAny>| -> PyResult<Vec<[f64; 3]>> {
            r.try_iter()?.map(|p| Ok(xyz(&p?)?.to_array())).collect()
        };
        let ring = coordinates(&outer)?;
        let normal = poly.face_normal();
        if poly.has_holes() {
            let mut rings = vec![ring.clone()];
            for h in holes.try_iter()? {
                rings.push(coordinates(&h?)?);
            }
            let mut largest = -1.;
            let mut probe = None;
            for [a, b, c] in tris(&rings, normal, poly.h) {
                let cross = (b - a).cross(c - a, poly.h);
                let area = cross.dot(cross, poly.h);
                if area > largest {
                    largest = area;
                    probe = Some(point((a + b + c) / 3.));
                }
            }
            if probe.is_none_or(|p| poly.in_hole(p)) {
                continue;
            }
        }
        let n = newell(&ring.into_iter().map(vector).collect::<Vec<_>>());
        let n = if length(n, poly.h) < 1e-9 {
            vector([0., 0., 1.])
        } else {
            super::geometry::normalized(n, poly.h)
        };
        if n.dot(normal, poly.h) < 0. {
            outer.call_method0("reverse")?;
            for h in holes.try_iter()? {
                h?.call_method0("reverse")?;
            }
        }
        kept.push((outer, holes));
    }
    if kept.len() > 1 {
        let attrs = face.getattr("attrs")?;
        let interior = face.getattr("interior")?;
        mesh.call_method1("remove_face", (face,))?;
        for (outer, holes) in kept {
            let new = mesh.call_method1("add_face", (outer, holes))?;
            new.setattr("attrs", copy.call1((&attrs,))?)?;
            new.setattr("interior", &interior)?;
        }
    }
    Ok(())
}
