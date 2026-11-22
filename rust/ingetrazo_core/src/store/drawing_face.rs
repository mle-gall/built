// SPDX-License-Identifier: GPL-3.0-or-later
//! Drawing predicates on a face in the store. Intermediate pieces carry positions.
use super::drawing::{key, point, vector};
use super::Store;
use crate::facegeom::{length, plane_axes, Hypot, V3};
type P = [f64; 3];
type XY = [f64; 2];
type Ring = Vec<P>;

fn side(a: XY, b: XY, p: XY) -> f64 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}
fn segments<T>(ring: &[T]) -> impl Iterator<Item = (&T, &T)> {
    ring.iter().zip(ring.iter().cycle().skip(1))
}
fn inside(p: XY, poly: &[XY], strict: bool) -> bool {
    let mut result = false;
    for (&b, &a) in segments(poly) {
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let d = (p[0] - a[0]) * dx + (p[1] - a[1]) * dy;
        if strict && side(a, b, p).abs() <= 1e-7 && d >= -1e-7 && d <= dx * dx + dy * dy + 1e-7 {
            return false;
        }
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + dx * (p[1] - a[1]) / dy {
            result = !result
        }
    }
    result
}
fn locate(r: &[P], p: P, h: Hypot) -> Option<(usize, bool)> {
    if let Some(i) = r.iter().position(|&q| key(q) == key(p)) {
        return Some((i, false));
    }
    let p = vector(p);
    segments(r)
        .position(|(&a, &b)| {
            let a = vector(a);
            let ab = vector(b).sub(a);
            let len = length(ab, h) as f64;
            if len < 1e-4 {
                return false;
            }
            let t = p.sub(a).dot(ab, h) as f64 / (len * len);
            (1e-4..=1. - 1e-4).contains(&t)
                && (length(p.sub(a.add(ab.mul(t as f32))), h) as f64) < 1e-4
        })
        .map(|i| (i, true))
}
fn split(r: &[P], chain: &[P], h: Hypot, ordered: bool) -> Option<Vec<Ring>> {
    if chain.len() < 2 {
        return None;
    }
    let ends = [chain[0], *chain.last()?];
    let loc = [locate(r, ends[0], h)?, locate(r, ends[1], h)?];
    let mut aug = Vec::new();
    for (i, &p) in r.iter().enumerate() {
        aug.push(p);
        let mut add: Vec<_> = (0..2)
            .filter(|&k| loc[k] == (i, true))
            .map(|k| ends[k])
            .collect();
        add.sort_by(|a, b| {
            length(vector(*a).sub(vector(p)), h).total_cmp(&length(vector(*b).sub(vector(p)), h))
        });
        aug.extend(add);
    }
    let idx = |p| aug.iter().position(|&q| key(q) == key(p));
    let (mut a, mut b) = (idx(ends[0])?, idx(ends[1])?);
    if ordered && a > b {
        std::mem::swap(&mut a, &mut b)
    }
    let arc = |i: usize, j: usize| -> Ring {
        (0..=(j + aug.len() - i) % aug.len())
            .map(|k| aug[(i + k) % aug.len()])
            .collect()
    };
    let mut one = arc(a, b);
    let mut two = arc(b, a);
    one.extend(chain[1..chain.len() - 1].iter().rev());
    two.extend(&chain[1..chain.len() - 1]);
    (one.len() >= 3 && two.len() >= 3).then(|| vec![one, two])
}
pub struct Outline {
    rings: Vec<Ring>,
    normal: V3,
    axes: (V3, V3),
    h: Hypot,
}
impl Store {
    pub fn outline(&self, f: u32, h: Hypot) -> Outline {
        let normal = self.face_normal(f, h);
        let rings = self
            .face_rings(f)
            .iter()
            .map(|r| {
                r.iter()
                    .map(|&v| *self.state.vpos.get(v as usize))
                    .collect()
            })
            .collect();
        Outline {
            rings,
            normal,
            axes: plane_axes(normal, h),
            h,
        }
    }
}
impl Outline {
    fn proj(&self, p: P) -> XY {
        let rel = vector(p).sub(vector(self.rings[0][0]));
        [
            rel.dot(self.axes.0, self.h) as f64,
            rel.dot(self.axes.1, self.h) as f64,
        ]
    }
    fn plane(&self, p: P) -> bool {
        (vector(p)
            .sub(vector(self.rings[0][0]))
            .dot(self.normal, self.h) as f64)
            .abs()
            <= 1e-3
    }
    fn polygon(&self, r: &[P]) -> Vec<XY> {
        r.iter().map(|&p| self.proj(p)).collect()
    }
    pub fn contains(&self, r: &[P], exclude_holes: bool) -> bool {
        if self.rings.first().is_none_or(|x| x.len() < 3) || r.len() < 3 {
            return false;
        }
        let outer = self.polygon(&self.rings[0]);
        let poly = self.polygon(r);
        if !r.iter().all(|&p| self.plane(p)) || !poly.iter().all(|&p| inside(p, &outer, true)) {
            return false;
        }
        if exclude_holes {
            for hole in self.rings.iter().skip(1) {
                let h = self.polygon(hole);
                if h.iter().any(|&p| inside(p, &poly, true))
                    || poly.iter().any(|&p| inside(p, &h, true))
                {
                    return false;
                }
                let opposite =
                    |x: f64, y: f64| (x > 1e-12 && y < -1e-12) || (x < -1e-12 && y > 1e-12);
                if segments(&h).any(|(&a, &b)| {
                    segments(&poly).any(|(&c, &d)| {
                        opposite(side(a, b, c), side(a, b, d))
                            && opposite(side(c, d, a), side(c, d, b))
                    })
                }) {
                    return false;
                }
            }
        }
        true
    }
    pub fn pieces(&self, chain: &[P], chord: bool) -> Option<Vec<Ring>> {
        let outer = self.rings.first()?;
        if chord {
            if self.rings.len() > 1 || chain.len() != 2 || outer.len() < 3 {
                return None;
            }
            let mid = point(vector(chain[0]).add(vector(chain[1])).mul(0.5));
            if !inside(self.proj(mid), &self.polygon(outer), true) {
                return None;
            }
        }
        split(outer, chain, self.h, chord)
    }
    pub fn chain(&self, r: &[P]) -> Option<Ring> {
        let outer = self.rings.first()?;
        if outer.len() < 3 || r.len() < 3 {
            return None;
        }
        let poly = self.polygon(outer);
        let mut bd = Vec::new();
        for &p in r {
            let on = locate(outer, p, self.h).is_some();
            if !on && !(self.plane(p) && inside(self.proj(p), &poly, true)) {
                return None;
            }
            bd.push(on);
        }
        let mut starts = (0..r.len()).filter(|&i| bd[i] && !bd[(i + 1) % r.len()]);
        let i = starts.next()?;
        if starts.next().is_some() {
            return None;
        }
        let mut out = vec![r[i]];
        for k in 1..=r.len() {
            let j = (k + i) % r.len();
            out.push(r[j]);
            if bd[j] {
                break;
            }
        }
        (key(out[0]) != key(*out.last()?)).then_some(out)
    }
}

impl Outline {
    fn interior(&self) -> Option<P> {
        if self.rings.first().is_none_or(|r| r.len() < 3) {
            return None;
        }
        let pts: Vec<_> = self.rings[0].iter().copied().map(vector).collect();
        let indices: Vec<_> = (0..pts.len() as u32).collect();
        let mut largest = 0.;
        let mut probe = None;
        for tri in crate::facegeom::triangles(&pts, &[&indices], self.normal, self.h) {
            let [a, b, c] = tri.map(|i| pts[i as usize]);
            let cross = b.sub(a).cross(c.sub(a), self.h);
            let area = cross.dot(cross, self.h);
            if area > largest {
                largest = area;
                probe = Some(point(a.add(b).add(c).div(3.)))
            }
        }
        probe
    }
    fn covers(&self, p: P, n: V3) -> bool {
        if (self.normal.dot(n, self.h) as f64).abs() < 0.999 || !self.plane(p) {
            return false;
        }
        let q = self.proj(p);
        inside(q, &self.polygon(&self.rings[0]), false)
            && !self
                .rings
                .iter()
                .skip(1)
                .any(|r| inside(q, &self.polygon(r), false))
    }
}

impl Store {
    pub fn winding_flips(&self, old: Option<&Store>, h: Hypot, compensated: bool) -> Vec<u128> {
        use super::keys::Kind;
        use crate::facegeom::py_sum;
        use std::collections::HashMap;
        let faces = |s: &Store| s.state.order[2].live(&s.state.slots[2]);
        let mut old_regions = Vec::new();
        if let Some(original) = old {
            for f in faces(original) {
                let region = original.outline(f, h);
                if region.rings[0].is_empty() {
                    continue;
                }
                let mut lo = [f64::INFINITY; 3];
                let mut hi = [f64::NEG_INFINITY; 3];
                for p in &region.rings[0] {
                    for k in 0..3 {
                        lo[k] = lo[k].min(p[k]);
                        hi[k] = hi[k].max(p[k]);
                    }
                }
                old_regions.push((lo, hi, region));
            }
        }
        let mut groups = Vec::<(V3, Vec<(u32, V3)>)>::new();
        let mut index = HashMap::new();
        for f in faces(self) {
            let r = self.outer_ring(f);
            let n = self.ring_normal(&r, h);
            let positions: Vec<_> = r
                .iter()
                .map(|&v| *self.state.vpos.get(v as usize))
                .collect();
            let centroid = std::array::from_fn(|k| {
                py_sum(positions.iter().map(|p| p[k]), compensated) / r.len().max(1) as f64
            });
            let group = [n.0, n.1, n.2, n.dot(vector(centroid), h)]
                .map(|x| ((x as f64).abs() * 100.).round_ties_even().to_bits());
            let idx = *index.entry(group).or_insert_with(|| {
                groups.push((V3(0., 0., 0.), vec![]));
                groups.len() - 1
            });
            let mut sum = [0.; 3];
            for (&a, &b) in segments(&positions) {
                for k in 0..3 {
                    let (i, j) = ((k + 1) % 3, (k + 2) % 3);
                    sum[k] += (a[i] - b[i]) * (a[j] + b[j]);
                }
            }
            let (ref mut dominant, ref mut rows) = groups[idx];
            *dominant = dominant.add(n.mul(length(vector(sum), h) * 0.5));
            rows.push((f, n));
        }
        let mut flips = vec![];
        for (dominant, rows) in groups {
            if rows.len() < 2 {
                continue;
            }
            for (f, n) in rows {
                let k = self.key(Kind::Face, f);
                if old.is_some_and(|s| s.slot(Kind::Face, k).is_some()) {
                    continue;
                }
                let target = if old.is_some() {
                    self.outline(f, h)
                        .interior()
                        .and_then(|p| {
                            old_regions
                                .iter()
                                .find(|(lo, hi, r)| {
                                    (0..3).all(|k| lo[k] - 1e-3 <= p[k] && p[k] <= hi[k] + 1e-3)
                                        && r.covers(p, n)
                                })
                                .map(|(_, _, r)| r.normal)
                        })
                        .unwrap_or(dominant)
                } else {
                    dominant
                };
                if n.dot(target, h) < 0. {
                    flips.push(k)
                }
            }
        }
        flips
    }
}

fn pairsig(a: P, b: P) -> [[u64; 3]; 2] {
    let mut pair = [key(a), key(b)];
    pair.sort_unstable();
    pair
}
fn on_segment(p: P, a: P, b: P, h: Hypot) -> bool {
    let ab = vector(b).sub(vector(a));
    let len = length(ab, h) as f64;
    if len < 1e-4 {
        return key(p) == key(a);
    }
    let t = vector(p).sub(vector(a)).dot(ab, h) as f64 / (len * len);
    (-1e-4..=1.0001).contains(&t)
        && (length(vector(p).sub(vector(a).add(ab.mul(t as f32))), h) as f64) < 1e-4
}
pub(super) fn refine(r: &[P], points: &[P], tol: f64, h: Hypot) -> Ring {
    let mut out = vec![];
    for (&a, &b) in segments(r) {
        out.push(a);
        let ab = vector(b).sub(vector(a));
        let len = length(ab, h) as f64;
        if len < tol {
            continue;
        }
        let mut on = Vec::new();
        for &p in points {
            if key(a) == key(p) || key(b) == key(p) {
                continue;
            }
            let t = vector(p).sub(vector(a)).dot(ab, h) as f64 / (len * len);
            if t > tol / len
                && t < 1. - tol / len
                && (length(vector(p).sub(vector(a).add(ab.mul(t as f32))), h) as f64) < tol
            {
                on.push((t, p))
            }
        }
        on.sort_by(|a, b| a.0.total_cmp(&b.0));
        out.extend(on.into_iter().map(|(_, p)| p));
    }
    out
}
impl Outline {
    pub fn carve(&self, r: &[P]) -> Option<Vec<Ring>> {
        let outer = self.rings.first()?;
        if outer.len() < 3 || r.len() < 3 || !r.iter().all(|&v| locate(outer, v, self.h).is_some())
        {
            return None;
        }
        let mut pieces = vec![outer.clone()];
        let poly = self.polygon(outer);
        for (&a, &b) in segments(r) {
            let mid = point(vector(a).add(vector(b)).mul(0.5));
            if locate(outer, mid, self.h).is_some() {
                continue;
            }
            let xy = self.proj(mid);
            if !inside(xy, &poly, true) {
                return None;
            }
            let i = pieces
                .iter()
                .position(|p| inside(xy, &self.polygon(p), true))?;
            let sub = split(&pieces[i], &[a, b], self.h, false)?;
            pieces.splice(i..=i, sub);
        }
        if pieces.len() < 2 {
            return None;
        }
        let sig = |piece: &[P]| {
            piece
                .iter()
                .copied()
                .map(key)
                .collect::<std::collections::HashSet<_>>()
        };
        let target = sig(r);
        let count = pieces.len();
        pieces.retain(|p| sig(p) != target);
        (pieces.len() + 1 == count).then_some(pieces)
    }
    pub fn shared_hole(&self, r: &[P]) -> Option<(usize, Ring, Ring)> {
        for (i, hole) in self.rings.iter().skip(1).enumerate() {
            let h = refine(hole, r, 1e-4, self.h);
            let l = refine(r, hole, 1e-4, self.h);
            let segs: std::collections::HashSet<_> =
                segments(&h).map(|(&a, &b)| pairsig(a, b)).collect();
            if segments(&l).any(|(&a, &b)| segs.contains(&pairsig(a, b))) {
                return Some((i, h, l));
            }
        }
        None
    }
}
impl Store {
    pub fn classify_segment(
        &self,
        face: u32,
        a: P,
        b: P,
        faces: &[u32],
        h: Hypot,
    ) -> (&'static str, Option<u128>) {
        use super::keys::Kind;
        let base = crate::facegeom::normalized(self.face_normal(face, h), h);
        let seg = pairsig(a, b);
        let mut other = None;
        for &f in faces {
            if f == face {
                continue;
            }
            let coplanar =
                (base.dot(crate::facegeom::normalized(self.face_normal(f, h), h), h) as f64).abs()
                    > 0.999;
            if !coplanar && other.is_some() {
                continue;
            }
            if self.face_rings(f).iter().any(|r| {
                segments(r).any(|(&x, &y)| {
                    let p = *self.state.vpos.get(x as usize);
                    let q = *self.state.vpos.get(y as usize);
                    (coplanar && pairsig(p, q) == seg)
                        || (on_segment(a, p, q, h) && on_segment(b, p, q, h))
                })
            }) {
                let k = self.key(Kind::Face, f);
                if coplanar {
                    return ("coplanar", Some(k));
                }
                other = Some(k);
            }
        }
        if other.is_some() {
            ("perp", other)
        } else {
            ("free", None)
        }
    }
    pub fn containing_face(
        &self,
        faces: &[u32],
        ring: &[P],
        exclude: Option<u128>,
        h: Hypot,
    ) -> Option<u128> {
        use super::keys::Kind;
        let p = *ring.first()?;
        let pad = 1e-3 + 1e-6 * p.into_iter().map(f64::abs).fold(1., f64::max);
        let mut best = None;
        let mut count = usize::MAX;
        for &f in faces {
            let k = self.key(Kind::Face, f);
            if Some(k) == exclude {
                continue;
            }
            let face = self.outline(f, h);
            let outer = &face.rings[0];
            if outer.len() >= count
                || !(0..3).all(|i| {
                    outer.iter().any(|v| v[i] - pad <= p[i])
                        && outer.iter().any(|v| v[i] + pad >= p[i])
                })
            {
                continue;
            }
            if face.contains(ring, true) {
                best = Some(k);
                count = outer.len()
            }
        }
        best
    }
    pub fn orphan_edges(&self, edges: &[u32], faces: &[u32], pts: &[P]) -> Vec<u128> {
        use super::keys::Kind;
        use std::collections::HashSet;
        let keys: HashSet<_> = pts.iter().copied().map(key).collect();
        let mut used = HashSet::new();
        let pos = |v: u32| *self.state.vpos.get(v as usize);
        for &f in faces {
            for r in self.face_rings(f) {
                for (&a, &b) in segments(&r) {
                    if keys.contains(&key(pos(a))) || keys.contains(&key(pos(b))) {
                        used.insert(pairsig(pos(a), pos(b)));
                    }
                }
            }
        }
        edges
            .iter()
            .copied()
            .filter(|&e| {
                let [a, b] = self.state.ends.get(e as usize).map(pos);
                (keys.contains(&key(a)) || keys.contains(&key(b))) && !used.contains(&pairsig(a, b))
            })
            .map(|e| self.key(Kind::Edge, e))
            .collect()
    }
}
