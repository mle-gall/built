// SPDX-License-Identifier: GPL-3.0-or-later
//! Which new element inherits a removed element's persistent id
//! (docs/persistent-ids.md §4, Rules 1 and 2).
//!
//! A History step can remove an element and create its replacements: a
//! line across a face, a plane rebuilt after drawing, a merge after an
//! erase, a push/pull that rebuilds its cap. Keys follow the kernel, but
//! the persistent id should follow the user's element. After the step,
//! between the state before and the state after:
//!
//! - faces: a created face that lies inside a removed one (or holds it) on
//!   the same plane overlaps it by the smaller one's area; the pairs are
//!   matched greedily, largest overlap first, so a split's larger half and
//!   a merge's larger original keep the id;
//! - faces again: a removed face whose exact copy, translated along its
//!   normal, was created (a rebuilt push/pull cap) passes its id on; a kept
//!   face that turned over while a translated copy of it appeared (a lone
//!   face pushed into a box: the original becomes the bottom) hands its id
//!   to that copy, the moved face;
//! - edges: collinear overlap, longest first (a split keeps its first
//!   half by key already);
//! - vertices: a created vertex at a removed one's position.
//!
//! Everything else keeps its fresh id. Pure function of the two states,
//! so a redo that restores the step's state and runs this again gets the
//! same ids.

use super::keys::{split_key, Kind};
use super::{State, Store};
use glam::DVec3;
use std::collections::{HashMap, HashSet};

/// What a step derived: `(kind, new id, parent id)` for created elements
/// that came from a removed one without inheriting it, `(kind, absorbed,
/// into)` for removed ones another inherited over.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lineage {
    pub derived: Vec<(Kind, u64, u64)>,
    pub merged: Vec<(Kind, u64, u64)>,
}

/// Pairs above this are skipped (a whole-mesh rebuild): ids stay fresh.
const MAX_PAIRS: usize = 4_000_000;

struct Poly {
    slot: u32,
    pts: Vec<DVec3>,
    normal: DVec3,
    d: f64,
    area: f64,
}

fn rings(st: &State, f: u32) -> Vec<Vec<u32>> {
    let r = *st.frings.get(f as usize);
    let mut c = r.corner_start as usize;
    let mut out = Vec::with_capacity(r.n_rings as usize);
    for k in 0..r.n_rings as usize {
        let size = *st.ring_sizes.get(r.ring_start as usize + k) as usize;
        out.push((0..size).map(|j| *st.corners.get(c + j)).collect());
        c += size;
    }
    out
}

fn newell(pts: &[DVec3]) -> DVec3 {
    let mut n = DVec3::ZERO;
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        n += DVec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
    }
    n * 0.5
}

fn poly(st: &State, f: u32) -> Option<Poly> {
    let rs = rings(st, f);
    let outer = rs.first()?;
    if outer.len() < 3 {
        return None;
    }
    let p = |v: u32| DVec3::from_array(*st.vpos.get(v as usize));
    let pts: Vec<DVec3> = outer.iter().map(|&v| p(v)).collect();
    let n2 = newell(&pts);
    let mut area = n2.length();
    if area <= 0.0 {
        return None;
    }
    let normal = n2 / area;
    for h in &rs[1..] {
        let hp: Vec<DVec3> = h.iter().map(|&v| p(v)).collect();
        area -= newell(&hp).length();
    }
    Some(Poly { slot: f, d: normal.dot(pts[0]), pts, normal, area: area.max(0.0) })
}

/// Point in (or on) the polygon, in its plane.
fn inside(poly: &Poly, q: DVec3, tol: f64) -> bool {
    let n = poly.normal;
    let (u, v) = {
        let a = if n.x.abs() < 0.9 { DVec3::X } else { DVec3::Y };
        let u = n.cross(a).normalize();
        (u, n.cross(u))
    };
    let p2 = |p: DVec3| (p.dot(u), p.dot(v));
    let (qx, qy) = p2(q);
    let pts: Vec<(f64, f64)> = poly.pts.iter().map(|&p| p2(p)).collect();
    let mut odd = false;
    for i in 0..pts.len() {
        let (ax, ay) = pts[i];
        let (bx, by) = pts[(i + 1) % pts.len()];
        // On the boundary counts as inside.
        let (dx, dy) = (bx - ax, by - ay);
        let len2 = dx * dx + dy * dy;
        if len2 > 0.0 {
            let t = (((qx - ax) * dx + (qy - ay) * dy) / len2).clamp(0.0, 1.0);
            let (cx, cy) = (ax + t * dx - qx, ay + t * dy - qy);
            if cx * cx + cy * cy <= tol * tol {
                return true;
            }
        }
        if (ay > qy) != (by > qy) && qx < ax + (qy - ay) * (bx - ax) / (by - ay) {
            odd = !odd;
        }
    }
    odd
}

fn coplanar(a: &Poly, b: &Poly, tol: f64) -> bool {
    a.normal.dot(b.normal).abs() > 1.0 - 1e-9 && (a.d - a.normal.dot(b.pts[0])).abs() <= tol
}

/// `b` is `a` moved by one vector along `a`'s normal (same corners, any
/// rotation of the ring): that vector, or None.
fn translated(a: &Poly, b: &Poly, tol: f64) -> Option<DVec3> {
    if a.pts.len() != b.pts.len() || a.normal.dot(b.normal) < 1.0 - 1e-9 || (a.area - b.area).abs() > tol * (1.0 + a.area) {
        return None;
    }
    let n = a.pts.len();
    for shift in 0..n {
        let off = b.pts[shift] - a.pts[0];
        if off.length() <= tol || off.cross(a.normal).length() > tol {
            continue;
        }
        if (0..n).all(|i| (b.pts[(i + shift) % n] - a.pts[i] - off).length() <= tol) {
            return Some(off);
        }
    }
    None
}

impl Store {
    /// Rules 1 and 2 between `before` and the current state (module docs).
    pub fn succeed(&mut self, before: &State) -> Lineage {
        let mut out = Lineage::default();
        if before.ns != self.state.ns {
            return out;
        }
        let ch = self.state.changes_since(before);
        let tol = 1e-6;
        let slot = |k: u128| split_key(k).map(|(_, _, _, s)| s);
        let removed = |kind: Kind| -> Vec<u32> { ch.removed[kind as usize].iter().filter_map(|&k| slot(k)).collect() };
        let created = |kind: Kind| -> Vec<u32> { ch.created[kind as usize].iter().filter_map(|&k| slot(k)).collect() };
        let mut assign: Vec<(Kind, u32, u64)> = Vec::new();

        // ---- faces
        let rf: Vec<Poly> = removed(Kind::Face).into_iter().filter_map(|f| poly(before, f)).collect();
        let nf: Vec<Poly> = created(Kind::Face).into_iter().filter_map(|f| poly(&self.state, f)).collect();
        let mut taken_r = HashSet::new();
        let mut taken_n = HashSet::new();
        if !rf.is_empty() && !nf.is_empty() && rf.len() * nf.len() <= MAX_PAIRS {
            let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
            for (i, r) in rf.iter().enumerate() {
                for (j, n) in nf.iter().enumerate() {
                    if !coplanar(r, n, tol) {
                        continue;
                    }
                    let (small, big) = if n.area <= r.area { (n, r) } else { (r, n) };
                    if small.pts.iter().all(|&q| inside(big, q, tol)) {
                        pairs.push((small.area, i, j));
                    }
                }
            }
            pairs.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
            let mut parent_of: std::collections::BTreeMap<usize, usize> = Default::default();
            let mut winner_of_r: std::collections::BTreeMap<usize, usize> = Default::default();
            for &(_, i, j) in &pairs {
                if !taken_r.contains(&i) && !taken_n.contains(&j) {
                    taken_r.insert(i);
                    taken_n.insert(j);
                    winner_of_r.insert(i, j);
                    assign.push((Kind::Face, nf[j].slot, before.ids[2].get(rf[i].slot)));
                }
                parent_of.entry(j).or_insert(i);
            }
            let winners: HashSet<usize> = winner_of_r.values().copied().collect();
            for (&j, &i) in &parent_of {
                if !winners.contains(&j) {
                    out.derived.push((Kind::Face, self.state.ids[2].get(nf[j].slot), before.ids[2].get(rf[i].slot)));
                }
            }
            // Removed faces another face inherited over: merged into it.
            for &(_, i, j) in &pairs {
                if !winner_of_r.contains_key(&i) {
                    if let Some((wi, _)) = winner_of_r.iter().find(|(_, &w)| w == j) {
                        let into = before.ids[2].get(rf[*wi].slot);
                        let absorbed = before.ids[2].get(rf[i].slot);
                        if !out.merged.contains(&(Kind::Face, absorbed, into)) {
                            out.merged.push((Kind::Face, absorbed, into));
                        }
                    }
                }
            }
            // A rebuilt cap: a removed face's translated copy.
            for (i, r) in rf.iter().enumerate() {
                if taken_r.contains(&i) {
                    continue;
                }
                if let Some(j) = (0..nf.len()).find(|j| !taken_n.contains(j) && translated(r, &nf[*j], tol).is_some()) {
                    taken_r.insert(i);
                    taken_n.insert(j);
                    assign.push((Kind::Face, nf[j].slot, before.ids[2].get(r.slot)));
                }
            }
        }
        // A lone face pushed into a box: the original stays as the bottom,
        // turned over; the moved top takes its id.
        if !nf.is_empty() && nf.len() <= 4096 {
            let kept: Vec<u32> = ch.modified[2].iter().filter_map(|&k| slot(k)).collect();
            for b in kept {
                let (Some(old), Some(now)) = (poly(before, b), poly(&self.state, b)) else { continue };
                if old.normal.dot(now.normal) > -0.5 {
                    continue;
                }
                if let Some(j) = (0..nf.len()).find(|j| !taken_n.contains(j) && translated(&old, &nf[*j], tol).is_some()) {
                    taken_n.insert(j);
                    let top = self.state.ids[2].get(nf[j].slot);
                    let id = before.ids[2].get(b);
                    assign.push((Kind::Face, nf[j].slot, id));
                    assign.push((Kind::Face, b, top));
                    out.derived.push((Kind::Face, top, id));
                }
            }
        }

        // ---- edges: collinear overlap
        let re = removed(Kind::Edge);
        let ne = created(Kind::Edge);
        if !re.is_empty() && !ne.is_empty() && re.len() * ne.len() <= MAX_PAIRS {
            let seg = |st: &State, e: u32| {
                let [a, b] = *st.ends.get(e as usize);
                (DVec3::from_array(*st.vpos.get(a as usize)), DVec3::from_array(*st.vpos.get(b as usize)))
            };
            let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
            for (i, &r) in re.iter().enumerate() {
                let (a, b) = seg(before, r);
                let dir = b - a;
                let len = dir.length();
                if len <= tol {
                    continue;
                }
                let u = dir / len;
                for (j, &n) in ne.iter().enumerate() {
                    let (c, d) = seg(&self.state, n);
                    if (c - a).cross(u).length() > tol || (d - a).cross(u).length() > tol {
                        continue;
                    }
                    let (t0, t1) = ((c - a).dot(u), (d - a).dot(u));
                    let (lo, hi) = (t0.min(t1).max(0.0), t0.max(t1).min(len));
                    if hi - lo > tol {
                        pairs.push((hi - lo, i, j));
                    }
                }
            }
            pairs.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
            let (mut tr, mut tn) = (HashSet::new(), HashSet::new());
            for &(_, i, j) in &pairs {
                if tr.insert(i) {
                    if tn.insert(j) {
                        assign.push((Kind::Edge, ne[j], before.ids[1].get(re[i])));
                    } else {
                        tr.remove(&i);
                    }
                }
            }
            for &(_, i, j) in &pairs {
                if !tn.contains(&j) {
                    tn.insert(j);
                    out.derived.push((Kind::Edge, self.state.ids[1].get(ne[j]), before.ids[1].get(re[i])));
                }
            }
        }

        // ---- vertices: same position
        let rv = removed(Kind::Vertex);
        let nv = created(Kind::Vertex);
        if !rv.is_empty() && !nv.is_empty() {
            let cell = |p: [f64; 3]| (p.map(|c| (c / tol).round() as i64), ());
            let mut at: HashMap<[i64; 3], u32> = HashMap::new();
            for &v in &rv {
                at.entry(cell(*before.vpos.get(v as usize)).0).or_insert(v);
            }
            for &n in &nv {
                if let Some(v) = at.remove(&cell(*self.state.vpos.get(n as usize)).0) {
                    assign.push((Kind::Vertex, n, before.ids[0].get(v)));
                }
            }
        }

        // The successor takes the id and, once exact geometry exists, the
        // exact surface or curve of the element it came from (§11).
        let shapes: Vec<(Kind, u32, u32)> = if before.exact.is_faceted() {
            Vec::new()
        } else {
            let by_id: HashMap<(usize, u64), u32> = [Kind::Edge, Kind::Face]
                .iter()
                .flat_map(|&k| removed(k).into_iter().map(move |s| (k, s)))
                .map(|(k, s)| ((k as usize, before.ids[k as usize].get(s)), before.exact.get(k, s)))
                .collect();
            assign.iter().filter_map(|&(k, s, id)| by_id.get(&(k as usize, id)).map(|&shape| (k, s, shape))).collect()
        };
        for (kind, s, id) in assign {
            self.set_pid(kind, s, id);
        }
        for (kind, s, shape) in shapes {
            self.state.exact.inherit(kind, s, shape);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::super::edit::Change;
    use super::super::keys::Kind;
    use super::super::{Store, Weld};

    fn square(st: &mut Store, x0: f64, x1: f64) -> u32 {
        let mut ch = Change::default();
        st.add_face(&[vec![[x0, 0.0, 0.0], [x1, 0.0, 0.0], [x1, 2.0, 0.0], [x0, 2.0, 0.0]]], &mut ch).unwrap()
    }

    #[test]
    fn the_larger_half_of_a_split_face_keeps_its_id() {
        let mut st = Store::empty(5, Weld::default());
        let f = square(&mut st, 0.0, 4.0);
        let id = st.pid(Kind::Face, f);
        let before = st.state.clone();
        let mut ch = Change::default();
        st.remove_face(f, &mut ch);
        let small = square(&mut st, 0.0, 1.0);
        let large = square(&mut st, 1.0, 4.0);
        let lineage = st.succeed(&before);
        assert_eq!(st.pid(Kind::Face, large), id);
        assert_ne!(st.pid(Kind::Face, small), id);
        assert_eq!(lineage.derived, vec![(Kind::Face, st.pid(Kind::Face, small), id)]);
    }

    #[test]
    fn a_merge_keeps_the_larger_original() {
        let mut st = Store::empty(6, Weld::default());
        let a = square(&mut st, 0.0, 1.0);
        let b = square(&mut st, 1.0, 4.0);
        let (ia, ib) = (st.pid(Kind::Face, a), st.pid(Kind::Face, b));
        let before = st.state.clone();
        let mut ch = Change::default();
        st.remove_face(a, &mut ch);
        st.remove_face(b, &mut ch);
        let m = square(&mut st, 0.0, 4.0);
        let lineage = st.succeed(&before);
        assert_eq!(st.pid(Kind::Face, m), ib);
        assert_eq!(lineage.merged, vec![(Kind::Face, ia, ib)]);
    }

    #[test]
    fn the_inheriting_half_also_inherits_an_exact_surface() {
        let mut st = Store::empty(8, Weld::default());
        let f = square(&mut st, 0.0, 4.0);
        st.state.exact.face_surface = Some(super::super::cow::CowVec::from_vec(vec![super::super::keys::NONE; 1]));
        st.state.exact.inherit(Kind::Face, f, 7);
        let before = st.state.clone();
        let mut ch = Change::default();
        st.remove_face(f, &mut ch);
        let small = square(&mut st, 0.0, 1.0);
        let large = square(&mut st, 1.0, 4.0);
        st.succeed(&before);
        assert_eq!(st.state.exact.get(Kind::Face, large), 7);
        assert_eq!(st.state.exact.get(Kind::Face, small), super::super::keys::NONE);
    }

    #[test]
    fn a_rebuilt_translated_cap_keeps_its_id() {
        let mut st = Store::empty(7, Weld::default());
        let f = square(&mut st, 0.0, 1.0);
        let id = st.pid(Kind::Face, f);
        let before = st.state.clone();
        let mut ch = Change::default();
        st.remove_face(f, &mut ch);
        let cap = st.add_face(&[vec![[0.0, 0.0, 1.0], [1.0, 0.0, 1.0], [1.0, 2.0, 1.0], [0.0, 2.0, 1.0]]], &mut ch).unwrap();
        st.succeed(&before);
        assert_eq!(st.pid(Kind::Face, cap), id);
    }
}
