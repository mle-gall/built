// SPDX-License-Identifier: GPL-3.0-or-later
//! Find the first wall reached by a push and project the filled islands of a ring.
use super::{
    drawing::point,
    geometry::{py_sum, Hypot},
    keys::Kind,
    Store,
};

use glam::DVec3;

type Ring = Vec<[f64; 3]>;

fn onto_rim(p: DVec3, rim: &[DVec3]) -> DVec3 {
    const SNAP: f64 = 5e-4;
    let mut near = rim[0];
    for &v in &rim[1..] {
        if v.distance_squared(p) < near.distance_squared(p) {
            near = v;
        }
    }
    if near.distance(p) <= SNAP {
        return near;
    }
    let (mut best, mut distance) = (p, SNAP);
    for (&a, &b) in rim.iter().zip(rim.iter().cycle().skip(1)) {
        let ab = b - a;
        if ab.length_squared() < 1e-18 {
            continue;
        }
        let t = (p - a).dot(ab) / ab.length_squared();
        if t <= 0. || t >= 1. {
            continue;
        }
        let q = a + ab * t;
        let d = q.distance(p);
        if d < distance {
            (best, distance) = (q, d);
        }
    }
    best
}

pub(super) fn centroid(r: &[[f64; 3]], compensated: bool) -> DVec3 {
    DVec3::from_array(std::array::from_fn(|i| {
        py_sum(r.iter().map(|p| p[i]), compensated) / r.len() as f64
    }))
}

impl Store {
    pub(super) fn push_opening(
        &self,
        base: u32,
        distance: f64,
        h: Hypot,
        compensated: bool,
        index: Option<&super::face_index::FaceIndex>,
    ) -> Option<(u32, Ring, Option<Ring>)> {
        let rings = self.position_rings(base);
        let outer = rings.first()?;
        let origin = centroid(outer, compensated);
        let n = DVec3::from_array(point(self.face_normal(base, h))).normalize_or_zero();
        let push = if distance > 0. { n } else { -n };
        let travel = distance.abs();
        let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
        for &v in outer {
            let v = DVec3::from_array(v);
            let end = v + push * travel;
            lo = lo.min(v).min(end);
            hi = hi.max(v).max(end);
        }
        let margin = 0.01 + 1e-6 * lo.abs().max(hi.abs()).max_element();
        lo -= DVec3::splat(margin);
        hi += DVec3::splat(margin);
        let mut best = None;
        let mut best_depth = f64::INFINITY;
        let candidates = match index {
            Some(tree) => tree.faces(self, [lo.x, lo.y, lo.z, hi.x, hi.y, hi.z], super::face_index::Query::Meeting),
            None => self.live_slots(Kind::Face),
        };
        for far in candidates {
            if far == base {
                continue;
            }
            let ids = self.outer_ring(far);
            if ids.is_empty() {
                continue;
            }
            let ring: Ring = ids
                .iter()
                .map(|&v| *self.state.vpos.get(v as usize))
                .collect();
            if (0..3)
                .any(|k| ring.iter().all(|p| p[k] < lo[k]) || ring.iter().all(|p| p[k] > hi[k]))
            {
                continue;
            }
            let n = DVec3::from_array(point(self.face_normal(far, h))).normalize_or_zero();
            let toward = push.dot(n);
            if toward.abs() < 0.999 {
                continue;
            }
            let centre = centroid(&ring, compensated);
            let depth = (centre - origin).dot(push);
            if depth <= 1e-4 || depth >= best_depth {
                continue;
            }
            let mut first = f64::INFINITY;
            let mut last = f64::NEG_INFINITY;
            for &v in outer {
                let reach = (centre - DVec3::from_array(v)).dot(n) / toward;
                first = first.min(reach);
                last = last.max(reach);
            }
            if last - first > 2e-3 {
                (first, last) = (depth, depth);
            }
            if first > travel + 1e-4 {
                continue;
            }
            let rim: Vec<_> = ring.into_iter().map(DVec3::from_array).collect();
            let back: Ring = outer
                .iter()
                .map(|&v| {
                    let p = DVec3::from_array(v) + push * depth;
                    onto_rim(p - n * (p - centre).dot(n), &rim).to_array()
                })
                .collect();
            let wall = self.outline(far, h);
            let rest = if wall.contains(&back, false) {
                None
            } else {
                if toward <= 0. && (travel < first - 1e-4 || travel > last + 1e-4) {
                    continue;
                }
                let Some(rest) = wall.subtract_chain(&back) else {
                    continue;
                };
                Some(rest)
            };
            best_depth = depth;
            best = Some((far, back, rest));
        }
        best
    }

    pub(super) fn push_islands(
        &self,
        face: u32,
        holes: &[Ring],
        far: u32,
        back: &[[f64; 3]],
        h: Hypot,
        compensated: bool,
    ) -> Option<Vec<Ring>> {
        if holes.is_empty() || back.len() < 3 {
            return None;
        }
        let wall = self.outline(far, h);
        let n = DVec3::from_array(point(wall.normal)).normalize_or_zero();
        let center = centroid(wall.rings.first()?, compensated);
        let v = *self.outer_ring(face).first()?;
        let lead = DVec3::from_array(back[0]) - DVec3::from_array(*self.state.vpos.get(v as usize));
        if lead.length() < 1e-9 {
            return None;
        }
        let push = lead.normalize_or_zero();
        let den = push.dot(n);
        if den < 1e-6 {
            return None;
        }
        for &p in back {
            let p = DVec3::from_array(p);
            for r in &wall.rings {
                for (&a, &b) in r.iter().zip(r.iter().cycle().skip(1)) {
                    let a = DVec3::from_array(a);
                    let ab = DVec3::from_array(b) - a;
                    let len = ab.length_squared();
                    let t = if len < 1e-18 {
                        0.
                    } else {
                        ((p - a).dot(ab) / len).clamp(0., 1.)
                    };
                    if (a + ab * t - p).length() < 1e-4 {
                        return None;
                    }
                }
            }
        }
        let faces = self.live_slots(Kind::Face);
        let mut projected = Vec::with_capacity(holes.len());
        for hole in holes {
            if hole.len() < 3 {
                return None;
            }
            let mut fill = None;
            for (&a, &b) in hole.iter().zip(hole.iter().cycle().skip(1)) {
                let ("coplanar", Some(key)) = self.classify_segment(face, a, b, &faces, h) else {
                    return None;
                };
                if fill.is_some_and(|f| f != key) {
                    return None;
                }
                fill = Some(key);
            }
            let filler = self.face_rings(self.slot(Kind::Face, fill?)?);
            if filler.len() != 1 || filler[0].len() != hole.len() {
                return None;
            }
            let cap: Ring = hole
                .iter()
                .map(|&p| {
                    let p = DVec3::from_array(p);
                    (p + push * ((center - p).dot(n) / den)).to_array()
                })
                .collect();
            if !wall.contains(&cap, false) {
                return None;
            }
            projected.push(cap);
        }
        Some(projected)
    }
}
