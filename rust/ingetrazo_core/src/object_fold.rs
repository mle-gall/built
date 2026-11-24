// SPDX-License-Identifier: GPL-3.0-or-later
//! Use the store's fold geometry while committing changes to existing handles.
use crate::{
    math::{xyz, Vec3},
    store::{
        access::is_planar,
        commands::{cell, supplied_cell, Cell},
        fold::{exact_geometry, strip_geometry},
        geometry::{Hypot, V3},
    },
};
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet},
};
use std::collections::{HashMap, HashSet};
type Obj<'py> = Bound<'py, PyAny>;
type P = [f64; 3];
type Tri = [P; 3];
fn positions(items: &Obj<'_>) -> PyResult<Vec<P>> {
    items
        .try_iter()?
        .map(|p| Ok(xyz(&p?)?.to_array()))
        .collect()
}
fn vector(p: P) -> Vec3 {
    Vec3::from(DVec3::from_array(p))
}
fn face_key(f: &Obj<'_>) -> PyResult<i128> {
    f.getattr("key")?.extract()
}
fn neighbours(
    mesh: &Obj<'_>,
    face: &Obj<'_>,
    ring: &[P],
) -> PyResult<HashMap<[u64; 3], HashSet<i128>>> {
    let key = face_key(face)?;
    let mut out = HashMap::new();
    for &p in ring {
        let v = mesh.call_method1("vertex_at", (vector(p),))?;
        let mut around = HashSet::new();
        if !v.is_none() {
            for f in mesh.call_method1("faces_at", (v,))?.try_iter()? {
                let k = face_key(&f?)?;
                if k != key {
                    around.insert(k);
                }
            }
        }
        out.insert(p.map(f64::to_bits), around);
    }
    Ok(out)
}

struct Fold {
    triangles: Vec<Tri>,
    soft: Vec<(P, P)>,
    merge_tolerance: Option<f64>,
}
fn plan(
    mesh: &Obj<'_>,
    face: &Obj<'_>,
    moved: &HashSet<Cell>,
    tolerance: f64,
) -> PyResult<Option<Fold>> {
    let ring = positions(&face.getattr("vertices")?)?;
    let holes = face.getattr("holes")?;
    let mut all = ring.clone();
    for hole in holes.try_iter()? {
        all.extend(positions(&hole?)?);
    }
    if all.len() <= 3 || is_planar(&all, tolerance) {
        return Ok(None);
    }
    let h = Hypot::LibStdCxx;
    let [x, y, z] = xyz(&face.call_method0("normal")?)?.to_array();
    let normal = V3(x, y, z);
    if holes.len()? == 0 {
        if let Some((triangles, apex, rim)) = strip_geometry(&ring, moved, normal, h) {
            let corners = face.getattr("loop")?;
            let index: HashMap<_, _> = ring
                .iter()
                .enumerate()
                .map(|(i, &p)| (cell(p), i))
                .collect();
            let mut soft = vec![];
            let n = ring.len();
            for p in rim {
                let i = index[&cell(p)];
                let mut curved = true;
                for (a, b) in [((i + n - 1) % n, i), (i, (i + 1) % n)] {
                    let e = mesh
                        .call_method1("find_edge", (corners.get_item(a)?, corners.get_item(b)?))?;
                    if e.is_none() || e.getattr("curve")?.is_none() {
                        curved = false;
                        break;
                    }
                }
                if curved {
                    soft.push((apex, p));
                }
            }
            return Ok(Some(Fold {
                triangles,
                soft,
                merge_tolerance: Some(tolerance),
            }));
        }
    }
    let mut triangles = vec![];
    for tri in face.call_method0("triangulate")?.try_iter()? {
        let tri = tri?;
        let p = |i| Ok::<_, PyErr>(xyz(&tri.get_item(i)?)?.to_array());
        triangles.push([p(0)?, p(1)?, p(2)?]);
    }
    if triangles.len() < 2 {
        return Ok(None);
    }
    let mut merge_tolerance = None;
    if holes.len()? == 0 && ring.len() <= 96 {
        if let Some(better) = exact_geometry(
            &ring,
            &triangles,
            &neighbours(mesh, face, &ring)?,
            normal,
            h,
        ) {
            triangles = better;
            merge_tolerance = Some(tolerance * 0.5);
        }
    }
    Ok(Some(Fold {
        triangles,
        merge_tolerance,
        soft: vec![],
    }))
}
fn merge(
    mesh: &Obj<'_>,
    pieces: &Bound<'_, PySet>,
    attrs: &Obj<'_>,
    tolerance: Option<f64>,
) -> PyResult<bool> {
    for f in mesh
        .py()
        .get_type::<PyList>()
        .call1((pieces,))?
        .try_iter()?
    {
        let f = f?;
        if !mesh.getattr("faces")?.contains(&f)? {
            pieces.discard(&f)?;
            continue;
        }
        let ring = f.getattr("loop")?;
        let n = ring.len()?;
        for i in 0..n {
            let e = mesh.call_method1(
                "find_edge",
                (ring.get_item(i)?, ring.get_item((i + 1) % n)?),
            )?;
            if e.is_none() {
                continue;
            }
            let radial = e.getattr("faces")?;
            if radial.len()? != 2 {
                continue;
            }
            let g = radial.get_item(if radial.get_item(1)?.eq(&f)? { 0 } else { 1 })?;
            if g.eq(&f)? || !pieces.contains(&g)? {
                continue;
            }
            let unit = |f: &Obj<'_>| xyz(&f.call_method0("normal")?.call_method0("normalized")?);
            if unit(&f)?.dot(unit(&g)?) <= 0.999 {
                continue;
            }
            if let Some(t) = tolerance {
                let mut points = positions(&f.getattr("vertices")?)?;
                points.extend(positions(&g.getattr("vertices")?)?);
                if !is_planar(&points, t) {
                    continue;
                }
            }
            let region = mesh.call_method1("dissolve_edge", (e,))?;
            if !region.is_none() {
                region.setattr("attrs", mesh.py().get_type::<PyDict>().call1((attrs,))?)?;
                pieces.discard(&f)?;
                pieces.discard(&g)?;
                pieces.add(region)?;
                return Ok(true);
            }
        }
    }
    Ok(false)
}
#[pyfunction]
pub(crate) fn fold_objects<'py>(
    mesh: &Obj<'py>,
    faces: &Obj<'py>,
    tolerance: f64,
    moved: Vec<P>,
) -> PyResult<Vec<Obj<'py>>> {
    let moved = moved.into_iter().map(supplied_cell).collect();
    let mut folded = vec![];
    let faces = faces.try_iter()?.collect::<PyResult<Vec<_>>>()?;
    let py = mesh.py();
    for face in faces {
        let Some(fold) = plan(mesh, &face, &moved, tolerance)? else {
            continue;
        };
        let attrs = face.getattr("attrs")?;
        mesh.call_method1("remove_face", (&face,))?;
        let pieces = PySet::empty(py)?;
        for tri in fold.triangles {
            let verts = PyList::new(py, tri.map(vector))?;
            let piece = mesh.call_method1("add_face", (verts,))?;
            piece.setattr("attrs", py.get_type::<PyDict>().call1((&attrs,))?)?;
            pieces.add(piece)?;
        }
        while merge(mesh, &pieces, &attrs, fold.merge_tolerance)? {}
        for (a, b) in fold.soft {
            let vertex = |p| mesh.call_method1("vertex_at", (vector(p),));
            let e = mesh.call_method1("find_edge", (vertex(a)?, vertex(b)?))?;
            if !e.is_none() && e.getattr("faces")?.len()? == 2 {
                e.setattr("soft", true)?;
            }
        }
        folded.push(face);
    }
    Ok(folded)
}
