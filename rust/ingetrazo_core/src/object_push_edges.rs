// SPDX-License-Identifier: GPL-3.0-or-later
//! Ordered push-edge queries on materialised handles and detached preview faces.
use crate::math::xyz;
use crate::store::{
    commands::{cell, Cell},
    drawing::StrokeSegment,
};
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyDict, PyFrozenSet, PySet},
};
use std::collections::HashSet;

type Obj<'py> = Bound<'py, PyAny>;
type Pair = [Cell; 2];

fn key(a: DVec3, b: DVec3) -> Pair {
    let mut pair = [cell(a.to_array()), cell(b.to_array())];
    pair.sort_unstable();
    pair
}

fn walk(ring: &Obj<'_>, mut visit: impl FnMut(DVec3, DVec3) -> PyResult<()>) -> PyResult<()> {
    let mut it = ring.try_iter()?;
    let Some(p) = it.next() else { return Ok(()) };
    let first = xyz(&p?)?;
    let mut last = first;
    for p in it {
        let next = xyz(&p?)?;
        visit(last, next)?;
        last = next;
    }
    visit(last, first)
}

struct Boundary {
    segments: Vec<StrokeSegment>,
    exact: HashSet<Pair>,
}
impl Boundary {
    fn read(face: &Obj<'_>) -> PyResult<Self> {
        let mut out = Self {
            segments: Vec::new(),
            exact: HashSet::new(),
        };
        for ring in
            std::iter::once(face.getattr("vertices")).chain(face.getattr("holes")?.try_iter()?)
        {
            walk(&ring?, |a, b| {
                out.exact.insert(key(a, b));
                out.segments.push(StrokeSegment::new(a, b));
                Ok(())
            })?;
        }
        Ok(out)
    }
    fn carries(&self, a: DVec3, b: DVec3) -> bool {
        self.segments
            .iter()
            .any(|s| s.contains(a, 1e-4) && s.contains(b, 1e-4))
    }
}

#[pyfunction]
pub(crate) fn boundary_carries(a: &Obj<'_>, b: &Obj<'_>, face: &Obj<'_>) -> PyResult<bool> {
    Ok(Boundary::read(face)?.carries(xyz(a)?, xyz(b)?))
}

fn frozen(py: Python<'_>, pair: Pair) -> PyResult<Bound<'_, PyFrozenSet>> {
    let points = pair.map(|c| {
        let [x, y, z] = c.map(f64::from_bits);
        (x, y, z)
    });
    PyFrozenSet::new(py, points)
}
#[pyfunction]
pub(crate) fn loop_edges<'py>(ring: &Obj<'py>) -> PyResult<Vec<Bound<'py, PyFrozenSet>>> {
    let mut out = Vec::new();
    walk(ring, |a, b| {
        out.push(frozen(ring.py(), key(a, b))?);
        Ok(())
    })?;
    Ok(out)
}
#[pyfunction]
pub(crate) fn face_edges<'py>(face: &Obj<'py>) -> PyResult<Bound<'py, PySet>> {
    let out = PySet::empty(face.py());
    for pair in Boundary::read(face)?.exact {
        out.add(frozen(face.py(), pair)?)?;
    }
    Ok(out)
}

struct Candidate {
    face: Py<PyAny>,
    boundary: Option<Boundary>,
}
#[pyclass]
struct PushEdges {
    subject: Py<PyAny>,
    // Search all coplanar candidates before any side wall, keeping input order.
    coplanar: Vec<Candidate>,
    other: Vec<Candidate>,
}

impl PushEdges {
    fn read(subject: &Obj<'_>, faces: &Obj<'_>) -> PyResult<Self> {
        let mut out = Self {
            subject: subject.clone().unbind(),
            coplanar: Vec::new(),
            other: Vec::new(),
        };
        let normal = xyz(&subject.call_method0("normal")?)?.normalize_or_zero();
        for f in faces.try_iter()? {
            let f = f?;
            if f.eq(subject)? {
                continue;
            }
            let n = xyz(&f.call_method0("normal")?)?.normalize_or_zero();
            let group = if normal.dot(n).abs() > 0.999 {
                &mut out.coplanar
            } else {
                &mut out.other
            };
            group.push(Candidate {
                face: f.unbind(),
                boundary: None,
            });
        }
        Ok(out)
    }
    fn query<'py>(
        &mut self,
        py: Python<'py>,
        a: DVec3,
        b: DVec3,
    ) -> PyResult<(&'static str, Option<Obj<'py>>)> {
        let pair = key(a, b);
        for (kind, faces) in [("coplanar", &mut self.coplanar), ("perp", &mut self.other)] {
            for g in faces {
                let face = g.face.bind(py);
                if g.boundary.is_none() {
                    g.boundary = Some(Boundary::read(face)?);
                }
                let r = g.boundary.as_ref().unwrap();
                if (kind == "coplanar" && r.exact.contains(&pair)) || r.carries(a, b) {
                    return Ok((kind, Some(face.clone())));
                }
            }
        }
        Ok(("free", None))
    }
}
#[pyfunction]
#[pyo3(signature=(face,a,b,faces,memo=None))]
pub(crate) fn classify_object_edge<'py>(
    face: &Obj<'py>,
    a: &Obj<'py>,
    b: &Obj<'py>,
    faces: &Obj<'py>,
    memo: Option<&Bound<'py, PyDict>>,
) -> PyResult<(&'static str, Option<Obj<'py>>)> {
    let (py, a, b) = (face.py(), xyz(a)?, xyz(b)?);
    let Some(memo) = memo else {
        return PushEdges::read(face, faces)?.query(py, a, b);
    };
    let slot = pyo3::intern!(py, "_push_edges");
    if let Some(value) = memo.get_item(slot)? {
        let mut cache = value.extract::<PyRefMut<'_, PushEdges>>()?;
        if cache.subject.bind(py).eq(face)? {
            return cache.query(py, a, b);
        }
    }
    let mut cache = PushEdges::read(face, faces)?;
    let result = cache.query(py, a, b)?;
    memo.set_item(slot, Py::new(py, cache)?)?;
    Ok(result)
}
