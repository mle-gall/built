// SPDX-License-Identifier: GPL-3.0-or-later
//! Boundary edits retain original point handles for the unchanged corners.
use crate::math::{xyz, Vec3};
use crate::object_drawing::outline_key;
use crate::store::{
    commands::{cell, supplied_cell, Cell},
    drawing_face::locate,
    geometry::Hypot,
};
use glam::DVec3;
use pyo3::prelude::*;

type Obj<'py> = Bound<'py, PyAny>;
type Ring<'py> = Vec<Obj<'py>>;

#[pyfunction]
fn face_exists(faces: &Obj<'_>, cycle: &Obj<'_>) -> PyResult<bool> {
    let target = outline_key(cycle)?;
    for face in faces.try_iter()? {
        if outline_key(&face?.getattr("vertices")?)? == target {
            return Ok(true);
        }
    }
    Ok(false)
}

#[pyfunction]
fn same_cycle(a: &Obj<'_>, b: &Obj<'_>) -> PyResult<bool> {
    Ok(outline_key(a)? == outline_key(b)?)
}

struct Boundary<'py> {
    corners: Ring<'py>,
    points: Vec<[f64; 3]>,
}

impl<'py> Boundary<'py> {
    fn read(ring: &Obj<'py>) -> PyResult<Self> {
        let corners = ring.try_iter()?.collect::<PyResult<Ring>>()?;
        let points = corners
            .iter()
            .map(|p| Ok(xyz(p)?.to_array()))
            .collect::<PyResult<_>>()?;
        Ok(Self { corners, points })
    }

    fn insert(
        mut self,
        a: Cell,
        b: Cell,
        p: DVec3,
        py: Python<'py>,
    ) -> PyResult<Option<Ring<'py>>> {
        let k = cell(p.to_array());
        if k == a || k == b {
            return Ok(None);
        }
        let mut ka = match self.points.first() {
            Some(v) => cell(*v),
            None => return Ok(None),
        };
        // Inspect the closing edge last, as the public boundary walk does.
        for (i, pair) in self
            .points
            .iter()
            .zip(self.points.iter().cycle().skip(1))
            .enumerate()
        {
            let kb = cell(*pair.1);
            if (ka == a && kb == b) || (ka == b && kb == a) {
                self.corners
                    .insert(i + 1, Vec3::from(p).into_pyobject(py)?.into_any());
                return Ok(Some(self.corners));
            }
            ka = kb;
        }
        Ok(None)
    }
}

#[pyfunction]
fn loop_with_point<'py>(
    ring: &Obj<'py>,
    ka: [f64; 3],
    kb: [f64; 3],
    point: &Obj<'py>,
) -> PyResult<Option<Ring<'py>>> {
    Boundary::read(ring)?.insert(supplied_cell(ka), supplied_cell(kb), xyz(point)?, ring.py())
}

#[pyfunction]
fn split_boundaries<'py>(
    faces: &Obj<'py>,
    a: &Obj<'py>,
    b: &Obj<'py>,
    point: &Obj<'py>,
    skip: &Obj<'py>,
) -> PyResult<Vec<(Obj<'py>, Ring<'py>)>> {
    let (ka, kb) = (cell(xyz(a)?.to_array()), cell(xyz(b)?.to_array()));
    let p = xyz(point)?;
    let skip = skip
        .try_iter()?
        .map(|p| Ok(xyz(&p?)?.to_array()))
        .collect::<PyResult<Vec<_>>>()?;
    let mut out = Vec::new();
    for f in faces.try_iter()? {
        let f = f?;
        let r = Boundary::read(&f.getattr("vertices")?)?;
        if !skip.is_empty()
            && skip
                .iter()
                .all(|p| locate(&r.points, *p, Hypot::LibStdCxx).is_some())
        {
            continue;
        }
        if let Some(corners) = r.insert(ka, kb, p, faces.py())? {
            out.push((f, corners))
        }
    }
    Ok(out)
}

#[pyfunction]
fn segment_contains(p: [f64; 2], a: [f64; 2], b: [f64; 2], tol: f64) -> bool {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let (x, y) = (p[0] - a[0], p[1] - a[1]);
    let proj = x * dx + y * dy;
    (dx * y - dy * x).abs() <= tol && proj >= -tol && proj <= dx * dx + dy * dy + tol
}

#[pyfunction]
fn face_winding<'py>(ring: &Obj<'py>, eye: Option<&Obj<'py>>) -> PyResult<Ring<'py>> {
    let mut corners = ring.try_iter()?.collect::<PyResult<Ring>>()?;
    if corners.len() >= 3 {
        let points = corners.iter().map(xyz).collect::<PyResult<Vec<_>>>()?;
        let n = crate::polygon::raw_newell(&points);
        let l = n.length();
        if l >= 1e-12 {
            let turn = if n.z.abs() > 0.5 * l {
                n.z <= 0.
            } else if let Some(eye) = eye {
                let center = points.iter().sum::<DVec3>() / (points.len() as f64);
                n.dot(xyz(eye)? - center) <= 0.
            } else {
                false
            };
            if turn {
                corners[1..].reverse()
            }
        }
    }
    Ok(corners)
}
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(face_exists, m)?)?;
    m.add_function(wrap_pyfunction!(same_cycle, m)?)?;
    m.add_function(wrap_pyfunction!(split_boundaries, m)?)?;
    m.add_function(wrap_pyfunction!(loop_with_point, m)?)?;
    m.add_function(wrap_pyfunction!(segment_contains, m)?)?;
    m.add_function(wrap_pyfunction!(face_winding, m)?)?;
    Ok(())
}
