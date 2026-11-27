// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! Explode as a store op: a group's geometry copied into the mesh it sits
//! in, through its placement (`built_core.history.ExplodeGroupCommand`), plus
//! the broad phase of the coplanar merge that follows
//! (`_merge_overlapping_planes`).
//!
//! `paste_from` is the command's geometry loop step for step:
//! 1. every face of the source, in list order, through `Mesh.add_face`
//!    with its corners mapped (outer ring, then the holes the nested-hole
//!    filter keeps): corners weld onto what is there, the face is new;
//! 2. every edge of the source, in list order: when either mapped end
//!    finds no vertex, or the two have no edge between them, `add_edge`;
//! 3. every source edge with flags (soft, hidden, flat, curve, tag) stamps
//!    them, all five, on the edge now between its mapped ends.
//!
//! Keys: everything the destination had keeps its key (a corner landing on
//! one of its vertices IS that vertex, a segment along one of its edges IS
//! that edge, which then lists the new face: `modified`). The source's
//! elements get new keys in the destination. The source is read, never
//! changed. Positions come mapped from Python (`QMatrix4x4.map`, Qt's own
//! float arithmetic), one per source vertex in list order.

use super::edit::Change;
use super::keys::{Kind, NONE};
use super::topo::OpError;
use super::{Store, FLAT, HIDDEN, SOFT};
use crate::store::geometry::{self as facegeom, Hypot, V3};
use std::collections::HashMap;
use std::sync::Arc;

/// What `paste_from` made, per source element in list order: the face
/// made from each face; the vertex and edge now at each source vertex's
/// and edge's mapped place (NONE: none, an edge welded shut).
pub struct Pasted {
    pub faces: Vec<u32>,
    pub vertices: Vec<u32>,
    pub edges: Vec<u32>,
}

/// One face's outer-ring box and whether its Newell normal has length.
#[derive(Clone, Copy)]
struct FaceBox {
    lo: [f64; 3],
    hi: [f64; 3],
}

impl Store {
    fn intern_layer(&mut self, name: &str) -> u32 {
        match self.state.layers.iter().position(|l| l == name) {
            Some(i) => i as u32,
            None => {
                Arc::make_mut(&mut self.state.layers).push(name.to_string());
                (self.state.layers.len() - 1) as u32
            }
        }
    }

    /// The command's geometry (see the module doc). `mapped[i]`: source
    /// vertex `i` (list order) through the placement; `keep[f]`: which
    /// holes of source face slot `f` the nested-hole filter keeps (absent:
    /// all); `face_attr[i]`: the attribute value (table index + 1, 0 for
    /// none) of the face made from the `i`-th source face.
    pub fn paste_from(
        &mut self,
        src: &Store,
        mapped: &[[f64; 3]],
        keep: &HashMap<u32, Vec<bool>>,
        face_attr: &[u32],
        ch: &mut Change,
    ) -> Result<Pasted, OpError> {
        let vl = src.live_slots(Kind::Vertex);
        let fl = src.live_slots(Kind::Face);
        let el = src.live_slots(Kind::Edge);
        if mapped.len() != vl.len() || face_attr.len() != fl.len() {
            return Err(OpError::Fallback("paste_from: one position per vertex, one attr per face"));
        }
        let mut dense = vec![NONE; src.state.slots[Kind::Vertex as usize].len()];
        for (i, &v) in vl.iter().enumerate() {
            dense[v as usize] = i as u32;
        }
        // Explode keeps each element's id where the destination has none
        // like it (docs/persistent-ids.md §4: keep if free).
        let mut taken: [std::collections::HashSet<u64>; 3] = Default::default();
        for kind in Kind::ALL {
            taken[kind as usize] = self.live_slots(kind).into_iter().map(|s| self.pid(kind, s)).collect();
        }
        let made_before: [usize; 3] = [ch.created[0].len(), ch.created[1].len(), ch.created[2].len()];
        let at = |v: u32| mapped[dense[v as usize] as usize];
        let mut faces = Vec::with_capacity(fl.len());
        for (i, &f) in fl.iter().enumerate() {
            let mask = keep.get(&f);
            let rings: Vec<Vec<[f64; 3]>> = src
                .face_rings(f)
                .into_iter()
                .enumerate()
                .filter(|(k, _)| *k == 0 || mask.is_none_or(|m| m.get(k - 1).copied().unwrap_or(true)))
                .map(|(_, r)| r.into_iter().map(at).collect())
                .collect();
            let nf = self.add_face(&rings, ch)?;
            if face_attr[i] != 0 {
                self.state.fattr.set(nf as usize, face_attr[i]);
            }
            faces.push(nf);
        }
        let ends = |e: u32| {
            let [a, b] = *src.state.ends.get(e as usize);
            (at(a), at(b))
        };
        for &e in &el {
            let (a, b) = ends(e);
            let found = match (self.lookup(a), self.lookup(b)) {
                (Some(v0), Some(v1)) => self.find_edge(v0, v1).is_some(),
                _ => false,
            };
            if !found {
                self.add_edge(a, b, ch)?;
            }
        }
        for &e in &el {
            let flags = *src.state.eflags.get(e as usize) & (SOFT | HIDDEN | FLAT);
            let curve = *src.state.ecurve.get(e as usize);
            let layer = *src.state.elayer.get(e as usize);
            if flags == 0 && curve < 0 && layer == NONE {
                continue; // edge_is_plain
            }
            let (a, b) = ends(e);
            let (Some(v0), Some(v1)) = (self.lookup(a), self.lookup(b)) else { continue };
            let Some(k) = self.find_edge(v0, v1) else { continue };
            let layer = if layer == NONE { NONE } else { self.intern_layer(&src.state.layers[layer as usize].clone()) };
            let st = &mut self.state;
            if *st.eflags.get(k as usize) != flags || *st.ecurve.get(k as usize) != curve || *st.elayer.get(k as usize) != layer {
                st.eflags.set(k as usize, flags);
                st.ecurve.set(k as usize, curve);
                st.elayer.set(k as usize, layer);
                ch.modified[1].push(self.key(Kind::Edge, k));
            }
        }
        let vertices: Vec<u32> = mapped.iter().map(|&p| self.lookup(p).unwrap_or(NONE)).collect();
        let edges: Vec<u32> = el
            .iter()
            .map(|&e| {
                let (a, b) = ends(e);
                match (self.lookup(a), self.lookup(b)) {
                    (Some(v0), Some(v1)) => self.find_edge(v0, v1).unwrap_or(NONE),
                    _ => NONE,
                }
            })
            .collect();
        // Ids: an element this paste made takes its source's id if free.
        let made: [std::collections::HashSet<u32>; 3] = std::array::from_fn(|k| {
            ch.created[k][made_before[k]..].iter().filter_map(|&key| super::keys::split_key(key).map(|t| t.3)).collect()
        });
        let pairs: [Vec<(u32, u32)>; 3] = [
            vl.iter().copied().zip(vertices.iter().copied()).collect(),
            el.iter().copied().zip(edges.iter().copied()).collect(),
            fl.iter().copied().zip(faces.iter().copied()).collect(),
        ];
        for kind in Kind::ALL {
            let k = kind as usize;
            for &(from, to) in &pairs[k] {
                if to == NONE || !made[k].contains(&to) {
                    continue;
                }
                let id = src.pid(kind, from);
                if taken[k].insert(id) {
                    self.set_pid(kind, to, id);
                }
            }
        }
        // Exact bindings come along through the placement (§3 Explode).
        self.adopt_curved(src, &pairs);
        Ok(Pasted { faces, vertices, edges })
    }

    fn outer_box(&self, f: u32) -> FaceBox {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for v in self.outer_ring(f) {
            let p = self.state.vpos.get(v as usize);
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        FaceBox { lo, hi }
    }

    /// `unit_normal(f) is not None` in `_merge_overlapping_planes`: the
    /// outer ring's Newell vector (sums in double, the vector in float32)
    /// longer than 1e-9.
    fn has_normal(&self, f: u32, h: Hypot) -> bool {
        let ring = self.outer_ring(f);
        let (mut nx, mut ny, mut nz) = (0.0f64, 0.0f64, 0.0f64);
        let m = ring.len();
        for i in 0..m {
            let p = self.state.vpos.get(ring[i] as usize);
            let q = self.state.vpos.get(ring[(i + 1) % m] as usize);
            nx += (p[1] - q[1]) * (p[2] + q[2]);
            ny += (p[2] - q[2]) * (p[0] + q[0]);
            nz += (p[0] - q[0]) * (p[1] + q[1]);
        }
        facegeom::length(V3(nx, ny, nz), h) > 1e-9
    }

    /// The broad phase of `_merge_overlapping_planes(scene, before, added)`
    /// when `before` is every face the mesh had: the faces of `added`
    /// (slots, in order) with a normal, and for every other live face in
    /// list order whose box passes the test against all of them, the
    /// indices into that list of the faces its box overlaps (the Python
    /// `overlap`, ascending). Python then runs its plane tests on exactly
    /// those pairs, in the order its own double loop meets them.
    pub fn plane_merge_candidates(&self, added: &[u32], tol: f64, h: Hypot) -> (Vec<u32>, Vec<(u32, Vec<u32>)>) {
        let new: Vec<u32> = added.iter().copied().filter(|&f| self.alive(Kind::Face, f) && self.has_normal(f, h)).collect();
        if new.is_empty() {
            return (new, Vec::new());
        }
        let boxes: Vec<FaceBox> = new.iter().map(|&f| self.outer_box(f)).collect();
        let mut glo = [f64::INFINITY; 3];
        let mut ghi = [f64::NEG_INFINITY; 3];
        for b in &boxes {
            for k in 0..3 {
                glo[k] = glo[k].min(b.lo[k]);
                ghi[k] = ghi[k].max(b.hi[k]);
            }
        }
        let flat = |b: &FaceBox, k: usize| (b.hi[k] - b.lo[k]).abs() < tol;
        let overlap = |a: &FaceBox, b: &FaceBox| {
            (0..3).all(|k| (a.lo[k] < b.hi[k] - tol && b.lo[k] < a.hi[k] - tol) || (flat(a, k) && flat(b, k)))
        };
        // A bucket grid along the axis where the fewest new boxes are flat
        // (a flat old box also meets every new box flat on that axis).
        let s = (0..3).min_by_key(|&k| boxes.iter().filter(|b| flat(b, k)).count()).unwrap();
        let flat_s: Vec<u32> = (0..boxes.len() as u32).filter(|&i| flat(&boxes[i as usize], s)).collect();
        let n = boxes.len();
        let nb = n.clamp(1, 1 << 20);
        let (lo0, span) = (glo[s], (ghi[s] - glo[s]).max(1e-12));
        let width = span / nb as f64;
        let bucket = |x: f64| (((x - lo0) / width).floor().max(0.0) as usize).min(nb - 1);
        let mut grid: Vec<Vec<u32>> = vec![Vec::new(); nb];
        let mut long: Vec<u32> = Vec::new();
        for (i, b) in boxes.iter().enumerate() {
            let (b0, b1) = (bucket(b.lo[s]), bucket(b.hi[s]));
            if b1 - b0 > 64 {
                long.push(i as u32);
            } else {
                for cell in &mut grid[b0..=b1] {
                    cell.push(i as u32);
                }
            }
        }
        let mut pairs = Vec::new();
        let mut is_new = vec![false; self.state.slots[Kind::Face as usize].len()];
        for &f in added {
            if (f as usize) < is_new.len() {
                is_new[f as usize] = true;
            }
        }
        let mut cand: Vec<u32> = Vec::new();
        for f in self.live_slots(Kind::Face) {
            if is_new[f as usize] {
                continue;
            }
            let a = self.outer_box(f);
            if !(0..3).all(|k| a.lo[k] <= ghi[k] + tol && glo[k] <= a.hi[k] + tol) {
                continue;
            }
            cand.clear();
            let (b0, b1) = (bucket(a.lo[s] - tol), bucket(a.hi[s] + tol));
            if b1 - b0 >= nb / 2 {
                cand.extend(0..n as u32);
            } else {
                for cell in &grid[b0..=b1] {
                    cand.extend(cell);
                }
                cand.extend(&long);
                if flat(&a, s) {
                    cand.extend(&flat_s);
                }
                cand.sort_unstable();
                cand.dedup();
            }
            let hits: Vec<u32> = cand.iter().copied().filter(|&i| overlap(&a, &boxes[i as usize])).collect();
            if !hits.is_empty() {
                pairs.push((f, hits));
            }
        }
        (new, pairs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::pieces::tests::cubes;

    #[test]
    fn paste_welds_onto_a_shared_wall_and_finds_the_coplanar_pair() {
        let mut dst = cubes(&[[0.0, 0.0, 0.0]]);
        let src = cubes(&[[0.0, 0.0, 0.0]]);
        let before: Vec<u128> = dst.live_slots(Kind::Vertex).iter().map(|&v| dst.key(Kind::Vertex, v)).collect();
        // The source cube moved one along x: its wall x=0 lands on x=1.
        let mapped: Vec<[f64; 3]> = src
            .live_slots(Kind::Vertex)
            .iter()
            .map(|&v| {
                let p = src.state.vpos.get(v as usize);
                [p[0] + 1.0, p[1], p[2]]
            })
            .collect();
        let mut ch = Change::default();
        let made = dst.paste_from(&src, &mapped, &HashMap::new(), &[0; 6], &mut ch).unwrap();
        let ch = dst.finish(ch);
        assert_eq!((dst.n(Kind::Vertex), dst.n(Kind::Face)), (12, 12));
        assert_eq!(ch.created[0].len(), 4); // the far corners only
        assert_eq!(ch.created[2].len(), 6);
        assert!(before.iter().all(|&k| dst.slot(Kind::Vertex, k).is_some()));
        // Shared wall edges list the new face: modified, not created.
        assert_eq!(ch.modified[1].len(), 4);
        assert_eq!(made.vertices.iter().filter(|&&v| v != NONE).count(), 8);
        assert_eq!(made.edges.iter().filter(|&&e| e != NONE).count(), 12);
        let (new, pairs) = dst.plane_merge_candidates(&made.faces, 1e-4, Hypot::LibStdCxx);
        assert_eq!(new.len(), 6);
        // The grid finds exactly what the Python double loop does.
        let tol = 1e-4;
        let flat = |b: &FaceBox, k: usize| (b.hi[k] - b.lo[k]).abs() < tol;
        let (mut glo, mut ghi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for &f in &new {
            let b = dst.outer_box(f);
            for k in 0..3 {
                glo[k] = glo[k].min(b.lo[k]);
                ghi[k] = ghi[k].max(b.hi[k]);
            }
        }
        let mut brute = Vec::new();
        for f in dst.live_slots(Kind::Face) {
            if made.faces.contains(&f) {
                continue;
            }
            let a = dst.outer_box(f);
            if !(0..3).all(|k| a.lo[k] <= ghi[k] + tol && glo[k] <= a.hi[k] + tol) {
                continue;
            }
            let hits: Vec<u32> = (0..new.len() as u32)
                .filter(|&j| {
                    let b = dst.outer_box(new[j as usize]);
                    (0..3).all(|k| (a.lo[k] < b.hi[k] - tol && b.lo[k] < a.hi[k] - tol) || (flat(&a, k) && flat(&b, k)))
                })
                .collect();
            if !hits.is_empty() {
                brute.push((f, hits));
            }
        }
        assert_eq!(pairs, brute);
        // The old wall at x = 1 meets the new one there.
        let wall = |f: u32| {
            let b = dst.outer_box(f);
            b.lo[0] == 1.0 && b.hi[0] == 1.0
        };
        assert!(pairs.iter().any(|(f, hits)| wall(*f) && hits.iter().any(|&j| wall(new[j as usize]))));
        dst.check().unwrap();
    }
}
