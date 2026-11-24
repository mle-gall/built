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
