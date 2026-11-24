// SPDX-License-Identifier: GPL-3.0-or-later
//! Ordered runs of drawn and inferred arcs on the outer and hole boundaries.
use crate::{
    math::{xyz, Vec3},
    snap_curve::snap_circle,
    store::Store,
};
use glam::DVec3;
use pyo3::prelude::*;

type Obj<'py> = Bound<'py, PyAny>;
pub(crate) type Centers = Vec<(Vec3, f64, (usize, usize))>;

#[derive(Clone, Copy, PartialEq)]
enum Mark {
    None,
    Soft,
    Shape,
    Drawn,
}
struct Ring {
    positions: Vec<DVec3>,
    marks: Vec<Mark>,
    same_tag: Vec<bool>,
}
impl Ring {
    fn runs(&self) -> Vec<(Vec<usize>, bool)> {
        let n = self.positions.len();
        if n < 3 {
            return vec![];
        }
        let mut marks = self.marks.clone();
        if marks.contains(&Mark::None) {
            let sides: Vec<_> = (0..n)
                .map(|i| self.positions[(i + 1) % n] - self.positions[i])
                .collect();
            let lengths: Vec<_> = sides.iter().map(|u| u.length()).collect();
            let turns: Vec<_> = (0..n)
                .map(|i| {
                    let prev = (i + n - 1) % n;
                    if lengths[prev] < 1e-9 || lengths[i] < 1e-9 {
                        return f64::NAN;
                    }
                    (sides[prev].dot(sides[i]) / (lengths[prev] * lengths[i]))
                        .clamp(-1., 1.)
                        .acos()
                        .to_degrees()
                })
                .collect();
            for (i, mark) in marks.iter_mut().enumerate() {
                // The incoming length at the next corner is this same segment.
                if *mark == Mark::None
                    && turns[i] > 2.
                    && turns[i] < 60.
                    && (turns[i] - turns[(i + 1) % n]).abs() < 1.5
                {
                    *mark = Mark::Shape;
                }
            }
        }
        let continues = |i: usize| {
            marks[i] == marks[(i + n - 1) % n] && (marks[i] != Mark::Drawn || self.same_tag[i])
        };
        let Some(start) = (0..n).find(|&i| !continues(i)) else {
            return if marks[0] == Mark::None {
                vec![]
            } else {
                vec![((0..n).collect(), marks[0] == Mark::Drawn)]
            };
        };
        let mut result = Vec::new();
        let mut begin = 0;
        for end in 1..=n {
            if end < n && continues((start + end) % n) {
                continue;
            }
            let kind = marks[(start + begin) % n];
            if kind != Mark::None && end - begin >= 2 {
                result.push((
                    (begin..=end).map(|j| (start + j) % n).collect(),
                    kind == Mark::Drawn,
                ));
            }
            begin = end;
        }
        result
    }
}

fn object_ring(vertices: &[Obj<'_>]) -> PyResult<Ring> {
    let n = vertices.len();
    let mut ring = Ring {
        positions: Vec::with_capacity(n),
        marks: vec![Mark::None; n],
        same_tag: vec![false; n],
    };
    let mut ids = Vec::with_capacity(n);
    for (i, v) in vertices.iter().enumerate() {
        ring.positions.push(xyz(&v.getattr("position")?)?);
        let mut id = None;
        if n >= 3 {
            if let Some(edges) = v.getattr_opt("edges")? {
                for edge in edges.try_iter()? {
                    let edge = edge?;
                    if !edge
                        .call_method1("other", (v,))?
                        .eq(&vertices[(i + 1) % n])?
                    {
                        continue;
                    }
                    id = edge.getattr_opt("curve")?.filter(|x| !x.is_none());
                    if id.is_some() {
                        ring.marks[i] = Mark::Drawn;
                    } else if let Some(soft) = edge.getattr_opt("soft")? {
                        if soft.is_truthy()? {
                            ring.marks[i] = Mark::Soft;
                        }
                    }
                    break;
                }
            }
        }
        ids.push(id);
    }
    for i in 0..n {
        if let (Some(a), Some(b)) = (&ids[i], &ids[(i + n - 1) % n]) {
            ring.same_tag[i] = a.is(b) || a.eq(b)?;
        }
    }
    Ok(ring)
}

#[pyfunction]
pub(crate) fn boundary_runs<'py>(vertices: &Obj<'py>) -> PyResult<Vec<(Vec<Obj<'py>>, bool)>> {
    let vertices = vertices.try_iter()?.collect::<PyResult<Vec<_>>>()?;
    if vertices.len() < 3 {
        return Ok(vec![]);
    }
    Ok(object_ring(&vertices)?
        .runs()
        .into_iter()
        .map(|(r, drawn)| (r.into_iter().map(|i| vertices[i].clone()).collect(), drawn))
        .collect())
}

struct Placement<'py> {
    matrix: Option<&'py Obj<'py>>,
    translation: Option<DVec3>,
}
impl<'py> Placement<'py> {
    fn new(matrix: Option<&'py Obj<'py>>) -> PyResult<Self> {
        let translation = if let Some(m) = matrix {
            if m.call_method0("isAffine")?.is_truthy()? {
                Some(xyz(&m.call_method1("map", (Vec3::from(DVec3::ZERO),))?)?)
            } else {
                None
            }
        } else {
            None
        };
        Ok(Self {
            matrix,
            translation,
        })
    }
    fn add(&self, py: Python<'_>, ring: Ring, li: usize, centers: &mut Centers) -> PyResult<()> {
        for (ri, (indices, drawn)) in ring.runs().into_iter().enumerate() {
            if indices.len() < if drawn { 3 } else { 5 } {
                continue;
            }
            let mut points = Vec::with_capacity(indices.len());
            for i in indices {
                let p = ring.positions[i];
                let p = if let Some(m) = self.matrix {
                    let method = if self.translation.is_some() {
                        "mapVector"
                    } else {
                        "map"
                    };
                    xyz(&m.call_method1(method, (Vec3::from(p),))?)?
                } else {
                    p
                };
                points.push(p.to_array());
            }
            if let Some((c, r)) = snap_circle(py, points, 0.01) {
                let c = DVec3::from_array(c) + self.translation.unwrap_or(DVec3::ZERO);
                centers.push((c.into(), r, (li, ri)));
            }
        }
        Ok(())
    }
}

#[pyfunction]
#[pyo3(signature=(face,xform=None))]
pub(crate) fn curve_centers(face: &Obj<'_>, xform: Option<&Obj<'_>>) -> PyResult<Centers> {
    let mut loops = vec![face.getattr("loop")?];
    if let Some(holes) = face.getattr_opt("hole_loops")?.filter(|h| !h.is_none()) {
        for h in holes.try_iter()? {
            loops.push(h?);
        }
    }
    let placement = Placement::new(xform)?;
    let mut out = Vec::new();
    for (li, r) in loops.iter().enumerate() {
        let vs = r.try_iter()?.collect::<PyResult<Vec<_>>>()?;
        if vs.len() >= 3 {
            placement.add(face.py(), object_ring(&vs)?, li, &mut out)?;
        }
    }
    Ok(out)
}

pub(crate) fn store_centers(
    py: Python<'_>,
    store: &Store,
    face: u32,
    xform: Option<&Obj<'_>>,
) -> PyResult<Centers> {
    let mut out = Vec::new();
    let placement = Placement::new(xform)?;
    for (li, vertices) in store.face_rings(face).into_iter().enumerate() {
        let n = vertices.len();
        if n < 3 {
            continue;
        }
        let mut ring = Ring {
            positions: vec![],
            marks: vec![],
            same_tag: vec![],
        };
        let mut tags = Vec::new();
        for (i, &v) in vertices.iter().enumerate() {
            ring.positions
                .push((*store.state.vpos.get(v as usize)).into());
            let e = store
                .find_edge(v, vertices[(i + 1) % n])
                .map(|e| e as usize);
            let id = e.map_or(-1, |e| *store.state.ecurve.get(e));
            let soft = e.is_some_and(|e| *store.state.eflags.get(e) & crate::store::SOFT != 0);
            ring.marks.push(if id >= 0 {
                Mark::Drawn
            } else if soft {
                Mark::Soft
            } else {
                Mark::None
            });
            tags.push(id);
        }
        ring.same_tag = (0..n).map(|i| tags[i] == tags[(i + n - 1) % n]).collect();
        placement.add(py, ring, li, &mut out)?;
    }
    Ok(out)
}
