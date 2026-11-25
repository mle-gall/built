// SPDX-License-Identifier: GPL-3.0-or-later
//! Plane/plane clipping in the coordinate system of one editing context.
use super::{keys::Kind, Store};
use crate::boxes::{touching, Box3};
use crate::store::geometry::{length, normalized, plane_axes, Hypot, V3};
use std::collections::HashSet;

pub type Segment = ([f64; 3], [f64; 3]);

pub type WorldRings = Vec<Vec<[f64; 3]>>;

pub enum Input {
    Face(u32),
    Placement(WorldRings),
}

pub(super) struct Polygon {
    origin: V3,
    bounds: Box3,
    normal: V3,
    distance: f64,
    axes: (V3, V3),
    rings: Vec<Vec<[f64; 2]>>,
}

impl Polygon {
    fn from_face(store: &Store, face: u32, h: Hypot) -> Option<Self> {
        let rings = store.face_rings(face);
        Self::from_rings(&rings, |v| *store.state.vpos.get(v as usize), h)
    }

    fn from_rings<T: Copy>(
        rings: &[Vec<T>],
        pos: impl Fn(T) -> [f64; 3],
        h: Hypot,
    ) -> Option<Self> {
        let outer = rings.first()?;
        if outer.len() < 3 {
            return None;
        }
        let mut bounds = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        let mut n = [0.; 3];
        for (i, &v) in outer.iter().enumerate() {
            let a = pos(v);
            let b = pos(outer[(i + 1) % outer.len()]);
            for k in 0..3 {
                bounds.0[k] = bounds.0[k].min(a[k]);
                bounds.1[k] = bounds.1[k].max(a[k]);
                let (j, l) = ((k + 1) % 3, (k + 2) % 3);
                n[k] += (a[j] - b[j]) * (a[l] + b[l]);
            }
        }
        let v3 = |p: [f64; 3]| V3(p[0], p[1], p[2]);
        let n = v3(n);
        if length(n, h) < 1e-12 {
            return None;
        }
        let normal = normalized(n, h);
        let axes = plane_axes(normal, h);
        let distance = normal.dot(v3(pos(outer[0])), h);
        let rings = rings
            .iter()
            .map(|r| {
                r.iter()
                    .map(|&v| {
                        let p = v3(pos(v));
                        [p.dot(axes.0, h), p.dot(axes.1, h)]
                    })
                    .collect()
            })
            .collect();
        Some(Self {
            origin: v3(pos(outer[0])),
            bounds,
            normal,
            distance,
            axes,
            rings,
        })
    }

    fn intervals(&self, origin: V3, dir: V3, h: Hypot) -> Vec<f64> {
        let project = |p: V3| (p.dot(self.axes.0, h), p.dot(self.axes.1, h));
        let (ox, oy) = project(origin);
        let (dx, dy) = project(dir);
        let mut hits = Vec::new();
        for ring in &self.rings {
            for (i, &[ax, ay]) in ring.iter().enumerate() {
                let [bx, by] = ring[(i + 1) % ring.len()];
                let sa = (ax - ox) * dy - (ay - oy) * dx;
                let sb = (bx - ox) * dy - (by - oy) * dx;
                if (sa >= 0.) != (sb >= 0.) {
                    let k = sa / (sa - sb);
                    hits.push(
                        ((ax + (bx - ax) * k - ox) * dx + (ay + (by - ay) * k - oy) * dy)
                            / (dx * dx + dy * dy),
                    );
                }
            }
        }
        hits.sort_by(f64::total_cmp);
        hits
    }

    fn cross(&self, b: &Self, h: Hypot, out: &mut Vec<Segment>) {
        let dir = self.normal.cross(b.normal, h);
        let c = self.normal.dot(b.normal, h);
        let den = 1. - c * c;
        if length(dir, h) < 1e-9 || den < 1e-12 {
            return;
        }
        let dir = normalized(dir, h);
        let add = |a: V3, b: V3| V3(a.0 + b.0, a.1 + b.1, a.2 + b.2);
        let origin = add(
            self.normal.mul((self.distance - b.distance * c) / den),
            b.normal.mul((b.distance - self.distance * c) / den),
        );
        let point = |t: f64| add(origin, dir.mul(t));
        let a = self.intervals(origin, dir, h);
        let b = b.intervals(origin, dir, h);
        let (mut i, mut j) = (0, 0);
        while i + 1 < a.len() && j + 1 < b.len() {
            let (lo, hi) = (a[i].max(b[j]), a[i + 1].min(b[j + 1]));
            if hi > lo {
                let (p, q) = (point(lo), point(hi));
                if length(q.sub(p), h) > 1e-5 {
                    out.push(([p.0, p.1, p.2], [q.0, q.1, q.2]));
                }
            }
            if a[i + 1] < b[j + 1] {
                i += 2;
            } else {
                j += 2;
            }
        }
    }
}

/// A projected materialised face reused for all crossing lines in one edit.
#[pyo3::pyclass(module = "built_core", name = "IntersectionPolygon", frozen)]
pub(super) struct PyPolygon {
    poly: Polygon,
    pub(super) boundary: WorldRings,
    pub(super) h: Hypot,
}

impl PyPolygon {
    pub(super) fn face_normal(&self) -> V3 { self.poly.normal }
    pub(super) fn has_holes(&self) -> bool { self.poly.rings.len() > 1 }
}

#[pyo3::pymethods]
impl PyPolygon {
    #[staticmethod]
    pub(super) fn from_rings(rings: WorldRings, hypot: &str) -> pyo3::PyResult<Option<Self>> {
        let h = crate::hypot_of(hypot)?;
        Ok(Polygon::from_rings(&rings, |p| p, h).map(|poly| Self { poly, h, boundary: rings }))
    }

    #[getter]
    pub(super) fn bounds(&self) -> Box3 {
        self.poly.bounds
    }

    #[getter]
    fn normal(&self) -> [f64; 3] {
        self.poly.normal.into()
    }

    pub(super) fn cross(&self, other: &Self) -> Vec<Segment> {
        let mut out = Vec::new();
        self.poly.cross(&other.poly, self.h, &mut out);
        out
    }

    pub(super) fn in_hole(&self, point: [f64; 3]) -> bool {
        let p = V3(point[0], point[1], point[2]);
        let xy = [
            p.dot(self.poly.axes.0, self.h),
            p.dot(self.poly.axes.1, self.h),
        ];
        self.poly
            .rings
            .iter()
            .skip(1)
            .any(|ring| super::drawing_face::inside(xy, ring, true))
    }

    pub(super) fn clip(&self, a: [f64; 3], b: [f64; 3]) -> Vec<Segment> {
        let (p, q) = (V3(a[0], a[1], a[2]), V3(b[0], b[1], b[2]));
        let poly = &self.poly;
        let h = self.h;
        if [p, q]
            .iter()
            .any(|&v| (v - poly.origin).dot(poly.normal, h).abs() > 1e-4)
        {
            return vec![];
        }
        let delta = q - p;
        let len = length(delta, h);
        if len < 1e-5 {
            return vec![];
        }
        let dir = normalized(delta, h);
        let mut cuts = Vec::new();
        for span in poly.intervals(p, dir, h).chunks_exact(2) {
            let lo = span[0].max(0.);
            let hi = span[1].min(len);
            if hi - lo < 1e-5 {
                continue;
            }
            let mid = p + dir * ((lo + hi) / 2.);
            let xy = [mid.dot(poly.axes.0, h), mid.dot(poly.axes.1, h)];
            if super::drawing_face::inside(xy, &poly.rings[0], true) {
                cuts.push((
                    if lo == 0. { a } else { (p + dir * lo).into() },
                    if hi == len { b } else { (p + dir * hi).into() },
                ));
            }
        }
        cuts
    }
}

pub fn segments(
    store: &Store,
    selected: &[Input],
    other_world: &[WorldRings],
    mode: &str,
    h: Hypot,
) -> Vec<Segment> {
    let mine: Vec<_> = selected
        .iter()
        .filter_map(|source| match source {
            Input::Face(f) => Polygon::from_face(store, *f, h),
            Input::Placement(rings) => Polygon::from_rings(rings, |p| p, h),
        })
        .collect();
    let bounds = |p: &[Polygon]| p.iter().map(|p| p.bounds).collect::<Vec<_>>();
    let boxes = bounds(&mine);
    let mut out = Vec::new();
    let mut cross = |other: &[Polygon], bb: &[Box3], same: bool| {
        let (ii, jj) = touching(&boxes, bb, same, 1e-6);
        for (i, j) in ii.into_iter().zip(jj) {
            mine[i as usize].cross(&other[j as usize], h, &mut out);
        }
    };
    if mode != "selection" {
        let selected: HashSet<_> = selected
            .iter()
            .filter_map(|source| match source {
                Input::Face(f) => Some(*f),
                _ => None,
            })
            .collect();
        let kind = Kind::Face as usize;
        let mut rest = store.state.order[kind].live(&store.state.slots[kind]);
        rest.retain(|f| !selected.contains(f));
        let others: Vec<_> = rest
            .into_iter()
            .filter_map(|f| Polygon::from_face(store, f, h))
            .chain(
                other_world
                    .iter()
                    .filter_map(|r| Polygon::from_rings(r, |p| p, h)),
            )
            .collect();
        cross(&others, &bounds(&others), false);
    }
    if mode != "context" {
        cross(&mine, &boxes, true);
    }
    let key = |p: [f64; 3]| p.map(|v| (v / 1e-6).round_ties_even() as i128);
    let mut seen = HashSet::new();
    out.retain(|&(p, q)| {
        let (a, b) = (key(p), key(q));
        seen.insert(if a <= b { (a, b) } else { (b, a) })
    });
    out
}
