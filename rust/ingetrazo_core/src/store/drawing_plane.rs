// SPDX-License-Identifier: GPL-3.0-or-later
//! The coverage and winding of the regions returned by planar_rebuild.
use super::{
    drawing::{key, point, vector},
    edit::Change,
    keys::Kind,
    Store,
};
use crate::facegeom::{length, triangles, Hypot, V3};
use std::collections::{HashMap, HashSet};
type P = [f64; 3];
type Ring = Vec<P>;
type Signature = Vec<[u64; 3]>;
fn signature(r: &[P]) -> Signature {
    let mut s: Vec<_> = r.iter().copied().map(key).collect();
    s.sort_unstable();
    s.dedup();
    s
}
pub(super) fn tris(r: &[Ring], n: V3, h: Hypot) -> Vec<[V3; 3]> {
    if r.first().is_none_or(|x| x.len() < 3) {
        return vec![];
    }
    let mut p = vec![];
    let indices: Vec<Vec<_>> = r
        .iter()
        .map(|ring| {
            let a = p.len();
            p.extend(ring.iter().copied().map(vector));
            (a as u32..p.len() as u32).collect()
        })
        .collect();
    triangles(
        &p,
        &indices.iter().map(Vec::as_slice).collect::<Vec<_>>(),
        n,
        h,
    )
    .into_iter()
    .map(|t| t.map(|i| p[i as usize]))
    .collect()
}
fn covers(t: [V3; 3], p: V3, h: Hypot) -> bool {
    let [a, b, c] = t;
    let n = b.sub(a).cross(c.sub(a), h);
    let nl = length(n, h) as f64;
    let sides = [b.sub(a), c.sub(b), a.sub(c)];
    let longest = sides.iter().map(|v| v.dot(*v, h)).fold(0., f32::max);
    if nl < 1e-12 || nl < 1e-9 * longest as f64 {
        return false;
    }
    (0..3).all(|i| {
        let side = length(sides[i], h) as f64;
        side == 0. || (sides[i].cross(p.sub(t[i]), h).dot(n, h) as f64) / (nl * side) >= -1e-7
    })
}
pub(super) fn bounds(r: &[P]) -> (P, P) {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in r {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    (lo, hi)
}
impl Store {
    pub fn drawing_clear(&mut self, ch: &mut Change) {
        for kind in Kind::ALL {
            for s in self.live_slots(kind) {
                ch.removed[kind as usize].push(self.key(kind, s));
                self.kill(kind, s);
            }
        }
        self.state.registry = Default::default();
        self.state.displaced = Default::default();
        self.state.extra = Default::default();
    }
    fn position_rings(&self, f: u32) -> Vec<Ring> {
        self.face_rings(f)
            .iter()
            .map(|r| {
                r.iter()
                    .map(|&v| *self.state.vpos.get(v as usize))
                    .collect()
            })
            .collect()
    }
    /// A rebuilt curved drawing inherits paint from the first old painted
    /// face covering the centroid of its first triangle (mesh order).
    pub fn paint_owners(
        &self,
        old: &Store,
        own: &HashMap<u128, bool>,
        rows: &HashSet<u32>,
        h: Hypot,
    ) -> Vec<(u128, u128, i64)> {
        let mut painted = vec![];
        for f in old.live_slots(Kind::Face) {
            let k = old.key(Kind::Face, f);
            let row = *old.state.fattr.get(f as usize);
            if own.get(&k).copied().unwrap_or_else(|| rows.contains(&row)) {
                painted.push((
                    k,
                    row,
                    tris(&old.position_rings(f), old.face_normal(f, h), h),
                ));
            }
        }
        if painted.is_empty() {
            return vec![];
        }
        let mut result = vec![];
        for f in self.live_slots(Kind::Face) {
            let triangles = tris(&self.position_rings(f), self.face_normal(f, h), h);
            if let Some(&[a, b, c]) = triangles.first() {
                let probe = a.add(b).add(c).div(3.);
                if let Some((old_key, row, _)) = painted
                    .iter()
                    .find(|(_, _, ts)| ts.iter().any(|&t| covers(t, probe, h)))
                {
                    result.push((self.key(Kind::Face, f), *old_key, *row as i64 - 1));
                }
            }
        }
        result
    }
    pub fn drawing_plane_edges(&self, o: P, n: P, h: Hypot, all: bool) -> (Vec<[P; 2]>, Vec<u128>) {
        let mut segs = vec![];
        let mut flagged = vec![];
        for e in self.live_slots(Kind::Edge) {
            let ends = self
                .state
                .ends
                .get(e as usize)
                .map(|v| *self.state.vpos.get(v as usize));
            if !all
                && !ends
                    .iter()
                    .all(|&p| (vector(p).sub(vector(o)).dot(vector(n), h) as f64).abs() < 1e-4)
            {
                continue;
            }
            segs.push(ends);
            let i = e as usize;
            if *self.state.eflags.get(i) != 0
                || *self.state.ecurve.get(i) >= 0
                || *self.state.elayer.get(i) != super::keys::NONE
            {
                flagged.push(self.key(Kind::Edge, e))
            }
        }
        (segs, flagged)
    }
    pub fn drawing_coverage(
        &self,
        o: P,
        n: P,
        regions: &[Vec<Ring>],
        h: Hypot,
    ) -> (Vec<u128>, Vec<(usize, u128, P)>) {
        let mut outlines = HashMap::new();
        let mut holes: HashMap<Signature, Vec<Signature>> = HashMap::new();
        let mut old = vec![];
        let mut keys = vec![];
        for f in self.live_slots(Kind::Face) {
            let rings = self.position_rings(f);
            if !rings
                .iter()
                .flatten()
                .all(|&p| (vector(p).sub(vector(o)).dot(vector(n), h) as f64).abs() < 1e-4)
            {
                continue;
            }
            let k = self.key(Kind::Face, f);
            let normal = self.face_normal(f, h);
            keys.push(k);
            let outer = signature(&rings[0]);
            *outlines.entry(outer.clone()).or_insert(0) += 1;
            for r in rings.iter().skip(1) {
                holes.entry(signature(r)).or_default().push(outer.clone());
            }
            old.push((k, normal, tris(&rings, normal, h)));
        }
        let mut keep = vec![];
        for (i, rings) in regions.iter().enumerate() {
            if rings.is_empty() {
                continue;
            }
            if rings.len() == 1 {
                let sig = signature(&rings[0]);
                if !outlines.contains_key(&sig)
                    && holes
                        .get(&sig)
                        .is_some_and(|owners| !owners.iter().any(|s| outlines[s] > 1))
                {
                    continue;
                }
            }
            let mut largest = -1.;
            let mut probe = None;
            for [a, b, c] in tris(rings, vector(n), h) {
                let cross = b.sub(a).cross(c.sub(a), h);
                let area = cross.dot(cross, h);
                if area > largest {
                    largest = area;
                    probe = Some(a.add(b).add(c).div(3.));
                }
            }
            if let Some(p) = probe {
                if let Some((k, normal, _)) = old
                    .iter()
                    .find(|(_, _, ts)| ts.iter().any(|&t| covers(t, p, h)))
                {
                    keep.push((i, *k, point(*normal)))
                }
            }
        }
        (keys, keep)
    }
}
