// SPDX-License-Identifier: GPL-3.0-or-later
//! The store's drawing planners on detached, binary64 face coordinates.
use crate::math::{xyz, Vec3};
use crate::store::geometry::{Hypot, V3};
use crate::store::Outline;
use glam::DVec3;
use pyo3::prelude::*;

type Point = [f64; 3];
type Ring = Vec<Point>;

fn positions(ring: &Bound<'_, PyAny>) -> PyResult<Ring> {
    ring.try_iter()?
        .map(|item| Ok(xyz(&item?)?.to_array()))
        .collect()
}
fn vectors(ring: Ring) -> Vec<Vec3> {
    ring.into_iter()
        .map(|p| Vec3::from(DVec3::from_array(p)))
        .collect()
}

#[pyclass(module = "ingetrazo_core", frozen)]
pub(crate) struct FaceOutline(Outline);

#[pymethods]
impl FaceOutline {
    #[new]
    fn new(face: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut rings = vec![positions(&face.getattr("vertices")?)?];
        for hole in face.getattr("holes")?.try_iter()? {
            rings.push(positions(&hole?)?);
        }
        let [x, y, z] = xyz(&face.call_method0("normal")?)?.to_array();
        // Store model geometry uses binary64 independently of the Qt ABI.
        Ok(Self(Outline::from_rings(
            rings,
            V3(x, y, z),
            Hypot::LibStdCxx,
        )))
    }
    fn contains(&self, ring: &Bound<'_, PyAny>, outside_holes: bool) -> PyResult<bool> {
        Ok(self.0.contains(&positions(ring)?, outside_holes))
    }
    fn split(
        &self,
        chain: &Bound<'_, PyAny>,
        chord: bool,
    ) -> PyResult<Option<(Vec<Vec3>, Vec<Vec3>)>> {
        Ok(self.0.pieces(&positions(chain)?, chord).map(|mut pieces| {
            let second = pieces.pop().unwrap();
            (vectors(pieces.pop().unwrap()), vectors(second))
        }))
    }
    fn chain(&self, ring: &Bound<'_, PyAny>) -> PyResult<Option<Vec<Vec3>>> {
        Ok(self.0.chain(&positions(ring)?).map(vectors))
    }
    fn solid_at(&self, p: &Bound<'_, PyAny>) -> PyResult<bool> {
        Ok(self.0.solid_at(xyz(p)?.to_array()))
    }

    fn carve(&self, ring: &Bound<'_, PyAny>) -> PyResult<Option<Vec<Vec<Vec3>>>> {
        Ok(self
            .0
            .carve(&positions(ring)?)
            .map(|pieces| pieces.into_iter().map(vectors).collect()))
    }
}

#[pyfunction]
pub(crate) fn segment_hit(
    py: Python<'_>,
    a: &Bound<'_, PyAny>,
    b: &Bound<'_, PyAny>,
    c: &Bound<'_, PyAny>,
    d: &Bound<'_, PyAny>,
    tolerance: f64,
) -> PyResult<Option<Vec3>> {
    let p = |v: &Bound<'_, PyAny>| Ok::<_, PyErr>(xyz(v)?.to_array());
    Ok(crate::store::drawing::intersection_at(
        [p(a)?, p(b)?],
        [p(c)?, p(d)?],
        Hypot::LibStdCxx,
        py.version_info().minor >= 12,
        tolerance,
    )
    .map(|v| DVec3::from_array(v).into()))
}

#[pyfunction]
pub(crate) fn split_edges<'py>(
    edges: &Bound<'py, PyAny>,
    a: &Bound<'py, PyAny>,
    b: &Bound<'py, PyAny>,
) -> PyResult<(
    Vec<(Bound<'py, PyAny>, Bound<'py, PyAny>)>,
    Bound<'py, pyo3::types::PyDict>,
)> {
    let py = a.py();
    let objects = edges.try_iter()?.collect::<PyResult<Vec<_>>>()?;
    let mut lines = Vec::with_capacity(objects.len());
    for edge in &objects {
        lines.push([
            xyz(&edge.getattr("a")?)?.to_array(),
            xyz(&edge.getattr("b")?)?.to_array(),
        ]);
    }
    let (hits, cuts) = crate::store::drawing::split_plan(
        xyz(a)?.to_array(),
        xyz(b)?.to_array(),
        lines.into_iter(),
        Hypot::LibStdCxx,
        py.version_info().minor >= 12,
    );
    let mut segments = Vec::with_capacity(hits.len() + 1);
    let mut prev = a.clone();
    for p in vectors(hits) {
        let next = Bound::new(py, p)?.into_any();
        segments.push((prev, next.clone()));
        prev = next;
    }
    segments.push((prev, b.clone()));
    let result = pyo3::types::PyDict::new(py);
    for (i, point) in cuts {
        result.set_item(&objects[i], Vec3::from(DVec3::from_array(point)))?;
    }
    Ok((segments, result))
}

#[pyfunction]
pub(crate) fn refined_ring(
    ring: &Bound<'_, PyAny>,
    points: &Bound<'_, PyAny>,
    tolerance: f64,
) -> PyResult<Vec<Vec3>> {
    Ok(vectors(crate::store::drawing_face::refine(
        &positions(ring)?,
        &positions(points)?,
        tolerance,
        Hypot::LibStdCxx,
    )))
}
#[pyfunction]
pub(crate) fn locate_boundary(
    ring: &Bound<'_, PyAny>,
    p: &Bound<'_, PyAny>,
) -> PyResult<Option<(&'static str, usize)>> {
    Ok(
        crate::store::drawing_face::locate(&positions(ring)?, xyz(p)?.to_array(), Hypot::LibStdCxx)
            .map(|(i, edge)| (if edge { "edge" } else { "vertex" }, i)),
    )
}
#[pyfunction]
pub(crate) fn polygon_contains_2d(p: [f64; 2], ring: Vec<[f64; 2]>, strict: bool) -> bool {
    crate::store::drawing_face::inside(p, &ring, strict)
}

/// Plan changes by list position, so materialised meshes retain their handles and keys.
#[pyfunction]
pub(crate) fn heal_objects(
    faces: &Bound<'_, PyAny>,
    coverage: f64,
    partial: bool,
    pad: f64,
    compensated: bool,
) -> PyResult<(Vec<u128>, Vec<(u128, Vec<u128>)>)> {
    let outlines = faces
        .try_iter()?
        .map(|f| Ok(FaceOutline::new(&f?)?.0))
        .collect::<PyResult<Vec<_>>>()?;
    let keys = (0..outlines.len() as u128).collect::<Vec<_>>();
    Ok(crate::store::drawing_face::outline_heal_plan(
        &outlines,
        &keys,
        coverage,
        partial,
        pad,
        Hypot::LibStdCxx,
        compensated,
    ))
}

#[pyfunction]
pub(crate) fn heal_partners(
    normals: Vec<Point>,
    areas: Vec<f64>,
    lo: Vec<Point>,
    hi: Vec<Point>,
    pad: f64,
) -> PyResult<std::collections::HashMap<usize, Vec<usize>>> {
    if [normals.len(), lo.len(), hi.len()]
        .iter()
        .any(|n| *n != areas.len())
    {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "inconsistent face columns",
        ));
    }
    let mut out = std::collections::HashMap::new();
    for (i, &area) in areas.iter().enumerate() {
        for (j, &other) in areas.iter().enumerate() {
            let parallel = DVec3::from_array(normals[i])
                .dot(DVec3::from_array(normals[j]))
                .abs()
                > 0.999;
            if crate::store::drawing_face::heal_partner(
                area,
                other,
                (lo[i], hi[i]),
                (lo[j], hi[j]),
                parallel,
                pad,
            ) {
                out.entry(i).or_insert_with(Vec::new).push(j);
            }
        }
    }
    Ok(out)
}

// The public footprint keeps the original handles and independent copies of
// their regions: history can move or remove the originals before consulting it.
#[pyfunction]
pub(crate) fn winding_footprint<'py>(
    faces: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, pyo3::types::PyList>> {
    let out = pyo3::types::PyList::empty(faces.py());
    for f in faces.try_iter()? {
        let f = f?;
        let ring = positions(&f.getattr("vertices")?)?;
        if ring.is_empty() {
            continue;
        }
        let mut lo = ring[0];
        let mut hi = lo;
        for p in &ring {
            for j in 0..3 {
                lo[j] = lo[j].min(p[j]);
                hi[j] = hi[j].max(p[j]);
            }
        }
        let holes = f
            .getattr("holes")?
            .try_iter()?
            .map(|r| Ok(vectors(positions(&r?)?)))
            .collect::<PyResult<Vec<_>>>()?;
        out.append((
            &f,
            f.call_method0("normal")?,
            vectors(ring),
            holes,
            (lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]),
        ))?;
    }
    Ok(out)
}

#[pyfunction]
#[pyo3(signature=(faces, footprint=None))]
pub(crate) fn object_winding<'py>(
    faces: &Bound<'py, PyAny>,
    footprint: Option<&Bound<'py, PyAny>>,
) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let py = faces.py();
    let fs = faces.try_iter()?.collect::<PyResult<Vec<_>>>()?;
    let outlines = fs
        .iter()
        .map(|f| Ok(FaceOutline::new(f)?.0))
        .collect::<PyResult<Vec<_>>>()?;
    let keys = fs
        .iter()
        .map(|f| Ok(f.getattr("key")?.extract::<i128>()? as u128))
        .collect::<PyResult<Vec<_>>>()?;
    let mut owned = std::collections::HashSet::new();
    let mut old = vec![];
    let h = Hypot::LibStdCxx;
    if let Some(footprint) = footprint {
        for row in footprint.try_iter()? {
            let row = row?;
            owned.insert(row.get_item(0)?.getattr("key")?.extract::<i128>()? as u128);
            let [x, y, z] = xyz(&row.get_item(1)?)?.to_array();
            let mut rings = vec![positions(&row.get_item(2)?)?];
            for r in row.get_item(3)?.try_iter()? {
                rings.push(positions(&r?)?);
            }
            let [a, b, c, d, e, f] = row.get_item(4)?.extract::<[f64; 6]>()?;
            old.push((
                [a, b, c],
                [d, e, f],
                Outline::from_rings(rings, V3(x, y, z), h),
            ));
        }
    }
    let rounded = |x: f64| {
        format!("{:.2}", x.abs())
            .parse::<f64>()
            .unwrap_or(x.abs())
            .to_bits()
    };
    let flips = crate::store::drawing_face::outline_winding(
        &outlines,
        &keys,
        &owned,
        &old,
        h,
        py.version_info().minor >= 12,
        rounded,
    );
    let by_key: std::collections::HashMap<_, _> = keys.into_iter().zip(fs).collect();
    Ok(flips.iter().map(|k| by_key[k].clone()).collect())
}

/// Hole rejection bound exposed for comparison with an exhaustive containment walk.
#[pyclass(frozen)]
pub(crate) struct HoleBounds {
    boxes: crate::store::fillet::HoleBoxes,
    origins: Option<Vec<V3>>,
}
#[pymethods]
impl HoleBounds {
    #[new]
    fn new(holes: &Bound<'_, PyAny>) -> PyResult<Self> {
        let rings = holes
            .try_iter()?
            .map(|r| {
                Ok(positions(&r?)?
                    .into_iter()
                    .map(|[x, y, z]| V3(x, y, z))
                    .collect::<Vec<_>>())
            })
            .collect::<PyResult<Vec<_>>>()?;
        let origins = rings.iter().map(|r| r.first().copied()).collect();
        Ok(Self {
            boxes: crate::store::fillet::HoleBoxes::new(&rings),
            origins,
        })
    }
    fn maybe_around(
        &self,
        i: usize,
        normal: &Bound<'_, PyAny>,
        others: Vec<usize>,
    ) -> PyResult<Vec<usize>> {
        let Some(ref origins) = self.origins else {
            return Ok(others);
        };
        if i >= origins.len() || others.iter().any(|&j| j >= origins.len()) {
            return Err(pyo3::exceptions::PyIndexError::new_err(
                "hole index out of range",
            ));
        }
        let [x, y, z] = xyz(normal)?.to_array();
        let gap = self.boxes.gap(origins[i], V3(x, y, z));
        Ok(others
            .into_iter()
            .filter(|&j| self.boxes.near(i, j, gap))
            .collect())
    }
}

#[pyfunction]
pub(crate) fn interior_point(
    ring: &Bound<'_, PyAny>,
    normal: &Bound<'_, PyAny>,
) -> PyResult<Option<Vec3>> {
    let [x, y, z] = xyz(normal)?.to_array();
    let outline = Outline::from_rings(vec![positions(ring)?], V3(x, y, z), Hypot::LibStdCxx);
    Ok(outline.interior().map(|p| Vec3::from(DVec3::from_array(p))))
}

#[pyfunction]
pub(crate) fn inside_loop(
    inner: &Bound<'_, PyAny>,
    outer: &Bound<'_, PyAny>,
    normal: &Bound<'_, PyAny>,
    compensated: bool,
) -> PyResult<bool> {
    let [x, y, z] = xyz(normal)?.to_array();
    let outline = |rings| Outline::from_rings(rings, V3(x, y, z), Hypot::LibStdCxx);
    let inner = outline(vec![positions(inner)?]);
    let outer = outline(vec![vec![], positions(outer)?]);
    Ok(outer.hole_contains(&inner, compensated))
}

#[pyfunction]
pub(crate) fn object_hole_mask(holes: &Bound<'_, PyAny>) -> PyResult<Vec<bool>> {
    let rings = holes
        .try_iter()?
        .map(|r| {
            let ring = positions(&r?)?;
            if ring.len() < 3 {
                return Err(pyo3::exceptions::PyIndexError::new_err(
                    "hole has fewer than three corners",
                ));
            }
            Ok(ring.into_iter().map(|[x, y, z]| V3(x, y, z)).collect())
        })
        .collect::<PyResult<Vec<_>>>()?;
    let v = holes.py().version_info();
    Ok(crate::store::fillet::maximal_holes_with_sum(
        &rings,
        Hypot::LibStdCxx,
        (v.major, v.minor) >= (3, 12),
    ))
}

#[pyfunction]
pub(crate) fn flat_faces(faces: &Bound<'_, PyAny>, tol: f64) -> PyResult<bool> {
    if faces.len()? < 2 {
        return Ok(true);
    }
    let plane = |f: &Bound<'_, PyAny>| -> PyResult<_> {
        let n = xyz(&f.call_method0("normal")?)?;
        let p = xyz(&f.getattr("vertices")?.get_item(0)?)?;
        Ok((n, p))
    };
    let (n, p) = plane(&faces.get_item(0)?)?;
    let u = n.normalize_or_zero();
    let offset = n.dot(p);
    for face in faces.try_iter()?.skip(1) {
        let (m, q) = plane(&face?)?;
        if !(u.dot(m.normalize_or_zero()).abs() > 0.999) || (n.dot(q) - offset).abs() > tol {
            return Ok(false);
        }
    }
    Ok(true)
}
