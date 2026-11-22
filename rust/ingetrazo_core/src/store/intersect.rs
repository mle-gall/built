// SPDX-License-Identifier: GPL-3.0-or-later
//! Plane/plane clipping in the coordinate system of one editing context.
use super::{keys::Kind, Store};
use crate::boxes::{touching, Box3};
use crate::facegeom::{length, normalized, plane_axes, Hypot, V3};
use std::collections::HashSet;

pub type Segment = ([f32; 3], [f32; 3]);

struct Polygon {
    bounds: Box3,
    normal: V3,
    distance: f64,
    axes: (V3, V3),
    rings: Vec<Vec<[f64; 2]>>,
}

impl Polygon {
    fn from_face(store: &Store, face: u32, h: Hypot) -> Option<Self> {
        let rings = store.face_rings(face);
        let outer = rings.first()?;
        if outer.len() < 3 {
            return None;
        }
        let pos = |v: u32| *store.state.vpos.get(v as usize);
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
        let v3 = |p: [f64; 3]| V3(p[0] as f32, p[1] as f32, p[2] as f32);
        let n = v3(n);
        if length(n, h) < 1e-12 {
            return None;
        }
        let normal = normalized(n, h);
        let axes = plane_axes(normal, h);
        let distance = normal.dot(v3(pos(outer[0])), h) as f64;
        let rings = rings
            .iter()
            .map(|r| {
                r.iter()
                    .map(|&v| {
                        let p = v3(pos(v));
                        [p.dot(axes.0, h) as f64, p.dot(axes.1, h) as f64]
                    })
                    .collect()
            })
            .collect();
        Some(Self {
            bounds,
            normal,
            distance,
            axes,
            rings,
        })
    }

    fn intervals(&self, origin: V3, dir: V3, h: Hypot) -> Vec<f64> {
        let project = |p: V3| (p.dot(self.axes.0, h) as f64, p.dot(self.axes.1, h) as f64);
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
        let c = self.normal.dot(b.normal, h) as f64;
        let den = 1. - c * c;
        if length(dir, h) < 1e-9 || den < 1e-12 {
            return;
        }
        let dir = normalized(dir, h);
        let add = |a: V3, b: V3| V3(a.0 + b.0, a.1 + b.1, a.2 + b.2);
        let origin = add(
            self.normal
                .mul(((self.distance - b.distance * c) / den) as f32),
            b.normal
                .mul(((b.distance - self.distance * c) / den) as f32),
        );
        let point = |t: f64| add(origin, dir.mul(t as f32));
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

pub fn segments(store: &Store, selected: &[u32], mode: &str, h: Hypot) -> Vec<Segment> {
    let polygons = |faces: &[u32]| {
        faces
            .iter()
            .filter_map(|&f| Polygon::from_face(store, f, h))
            .collect::<Vec<_>>()
    };
    let mine = polygons(selected);
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
        let selected: HashSet<_> = selected.iter().copied().collect();
        let kind = Kind::Face as usize;
        let mut rest = store.state.order[kind].live(&store.state.slots[kind]);
        rest.retain(|f| !selected.contains(f));
        let others = polygons(&rest);
        cross(&others, &bounds(&others), false);
    }
    if mode != "context" {
        cross(&mine, &boxes, true);
    }
    let key = |p: [f32; 3]| p.map(|v| (v as f64 / 1e-6).round_ties_even() as i128);
    let mut seen = HashSet::new();
    out.retain(|&(p, q)| {
        let (a, b) = (key(p), key(q));
        seen.insert(if a <= b { (a, b) } else { (b, a) })
    });
    out
}
