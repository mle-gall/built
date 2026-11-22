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
fn tris(r: &[Ring], n: V3, h: Hypot) -> Vec<[V3; 3]> {
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
fn bounds(r: &[P]) -> (P, P) {
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
    pub fn painted_candidates(&self, own: &HashSet<u128>) -> Vec<u128> {
        self.live_slots(Kind::Face)
            .into_iter()
            .filter_map(|f| {
                let k = self.key(Kind::Face, f);
                (*self.state.fattr.get(f as usize) != 0 || own.contains(&k)).then_some(k)
            })
            .collect()
    }
    pub fn drawing_plane_edges(&self, o: P, n: P, h: Hypot) -> (Vec<[P; 2]>, Vec<u128>) {
        let mut segs = vec![];
        let mut flagged = vec![];
        for e in self.live_slots(Kind::Edge) {
            let ends = self
                .state
                .ends
                .get(e as usize)
                .map(|v| *self.state.vpos.get(v as usize));
            if !ends
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
            let rings: Vec<Ring> = self
                .face_rings(f)
                .iter()
                .map(|r| {
                    r.iter()
                        .map(|&v| *self.state.vpos.get(v as usize))
                        .collect()
                })
                .collect();
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
    /// Candidate faces for heal passes 2/3, including all possible partners.
    /// The exact polygon tests still run in their original order.
    pub fn heal_candidates(&self, partial: bool, pad: f64, h: Hypot) -> Vec<u128> {
        let faces = self.live_slots(Kind::Face);
        let data: Vec<_> = faces
            .iter()
            .map(|&f| {
                let r = self.outer_ring(f);
                let p: Vec<_> = r
                    .iter()
                    .map(|&v| *self.state.vpos.get(v as usize))
                    .collect();
                let mut raw = [0.; 3];
                for (a, b) in p.iter().zip(p.iter().cycle().skip(1)) {
                    for k in 0..3 {
                        let (i, j) = ((k + 1) % 3, (k + 2) % 3);
                        raw[k] += (a[i] - b[i]) * (a[j] + b[j])
                    }
                }
                (
                    length(vector(raw), h),
                    bounds(&p),
                    point(self.ring_normal(&r, h)),
                )
            })
            .collect();
        let mut use_face = vec![false; faces.len()];
        for (i, (area, (lo, hi), n)) in data.iter().enumerate() {
            for (j, (other, (low, high), m)) in data.iter().enumerate() {
                if other >= area || (n[0] * m[0] + n[1] * m[1] + n[2] * m[2]).abs() < 0.999 - 1e-12
                {
                    continue;
                }
                if (0..3).all(|k| {
                    if partial {
                        low[k] <= hi[k] + pad && high[k] >= lo[k] - pad
                    } else {
                        low[k] >= lo[k] - pad && high[k] <= hi[k] + pad
                    }
                }) {
                    use_face[i] = true;
                    use_face[j] = true;
                }
            }
        }
        faces
            .into_iter()
            .zip(use_face)
            .filter_map(|(f, t)| t.then(|| self.key(Kind::Face, f)))
            .collect()
    }
}
