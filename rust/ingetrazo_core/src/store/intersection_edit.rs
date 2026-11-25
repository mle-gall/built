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
    IntoPyObjectExt,
};

/// Keep the same facade on both sides of a split: the store can materialise
/// while splitting a detached handle, and the half edges inherit all flags.
#[pyfunction]
pub(crate) fn _partition_faces<'py>(
    mesh: &Bound<'py, PyAny>,
    segments: &Bound<'py, PyAny>,
) -> PyResult<()> {
    let py = mesh.py();
    let scan = py.import("core.scan_index")?.getattr("scan")?;
    let mesh_api = py.import("core.mesh")?;
    let read_flags = mesh_api.getattr("edge_flags")?;
    let stamp_flags = mesh_api.getattr("stamp_edge_flags")?;
    let lines = PyDict::new(py);
    for seg in segments.try_iter()? {
        let seg = seg?;
        let (a, b) = (seg.get_item(0)?, seg.get_item(1)?);
        let index = scan.call1((mesh,))?;
        let plan = if index.is_none() {
            py.import("core.topology")?
                .call_method1("plan_edge_split", (mesh.getattr("edges")?, a, b))?
        } else {
            py.import("core.edits")?
                .call_method1("_ScanEdges", (index,))?
                .call_method1("split_plan", (a, b))?
        };
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
                let poly = match super::face_intersections::world_polys(&face, None)? {
                    Some(p) => p.into_any(),
                    None => py.None().into_bound(py),
                };
                polys.set_item(&key, &poly)?;
                poly
            };
            if poly.is_none() {
                continue;
            }
            let clipped = poly
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
        let polygon = polys
            .get_item(face.getattr("key")?)?
            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("intersection polygon missing"))?
            .cast_into::<PyPolygon>()?;
        let polygon = polygon.borrow();
        let segments = PyList::empty(py);
        for ring in &polygon.boundary {
            for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                segments.append((
                    Vec3::from(glam::DVec3::from(*a)),
                    Vec3::from(glam::DVec3::from(*b)),
                ))?;
            }
        }
        segments.call_method1("extend", (cuts,))?;
        let origin = Vec3::from(glam::DVec3::from(polygon.boundary[0][0])).into_bound_py_any(py)?;
        let n = polygon.face_normal();
        let normal = Vec3::from(glam::DVec3::new(n.0, n.1, n.2)).into_bound_py_any(py)?;
        let regions = super::planar_api::planar_arrangement(segments.as_any(), &origin, &normal)?
            .into_bound_py_any(py)?;
        replace_regions(
            &polygon,
            mesh,
            &face,
            &regions,
            &py.import("copy")?.getattr("deepcopy")?,
        )?;
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
