// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! Port step 3 (`docs/meshstore-design.md` §8 items 3–4, §16): the splits
//! and the stitch primitives of `core/mesh.py` as store ops —
//! `split_edge`, `split_edge_at`, `_insert_at_every_use`,
//! `_insert_into_face_loops`, `interior_vertex_on`, `weld_coincident`,
//! `_weld_into`, `collapsible_vertex`, `collapse_vertex` (and the phase-2
//! loop of `history.run_stitch` around them), `dedupe_faces`, the four
//! dissolves with `_shared_edge_index`, `_splice_loops`, `_trace_loops`,
//! `_loop_area` and `_orient_union_loops`, and the ring reversal of
//! `turn_face_over`.
//!
//! Each is its Python method step for step: the same list order (§4), the
//! same weld lookups, the key rules of `docs/element-keys-implementation.md`
//! (a split's new first half and a weld's redirected edge take the old
//! edge's key, i.e. its SLOT: a key is a slot and a generation), and the
//! same model-unit tolerances. Geometric predicates use doubles.
//!
//! Where Python iterates a `set` (`collapse_vertex`'s two edges,
//! `_weld_into`'s edges of the welded vertex, `_trace_loops`' next edge),
//! the order was the interpreter's; `core/mesh.py` now fixes it (design
//! decision 5) and this file follows the same rule.
//!
//! The ops count Python's dirty bumps (`_chunk_dirty = True; _mut_serial
//! += 1` per primitive call) and return the count, so the facade's
//! revision counters move exactly as the Python mesh's.
//!
//! **Refusals** (`OpError::Fallback`, before anything changes) follow
//! step 2: a result the store keeps no invariant for — an end of a split
//! that is the splitting vertex, a self-loop on the vertex welded away or
//! collapsed, a collapse that would leave its vertex in a face — or a
//! dissolve whose hole filter (Python's `_maximal_holes`) raised.

use super::cow::Cell;
use super::edit::Change;
use super::keys::{Kind, NONE};
use super::topo::OpError;
use super::{Store, FLAT, SOFT};
use crate::store::geometry::{self as facegeom, Hypot, V3};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::sync::Arc;

/// `core.mesh._STITCH_TOL`.
pub const STITCH_TOL: f64 = 1e-4;

#[inline]
fn v3(p: &[f64; 3]) -> V3 {
    V3(p[0], p[1], p[2])
}

#[inline]
fn add(a: V3, b: V3) -> V3 {
    V3(a.0 + b.0, a.1 + b.1, a.2 + b.2)
}

#[inline]
fn scale(a: V3, s: f64) -> V3 {
    V3(a.0 * s, a.1 * s, a.2 * s)
}

/// `Mesh._uses`: segments of the rings between `a` and `b`, either way.
fn uses(rings: &[Vec<u32>], a: u32, b: u32) -> usize {
    let mut n = 0;
    for r in rings {
        let m = r.len();
        for i in 0..m {
            let (p, q) = (r[i], r[(i + 1) % m]);
            if (p == a && q == b) || (p == b && q == a) {
                n += 1;
            }
        }
    }
    n
}

/// `Mesh._insert_at_every_use`: `mid` into every `v0`/`v1` segment of
/// every ring, as many times as a ring walks it.
fn insert_at_every_use(rings: &mut [Vec<u32>], v0: u32, v1: u32, mid: u32) -> bool {
    let mut changed = false;
    for r in rings.iter_mut() {
        let mut i = 0;
        while i < r.len() {
            let j = (i + 1) % r.len();
            if (r[i] == v0 && r[j] == v1) || (r[i] == v1 && r[j] == v0) {
                r.insert(i + 1, mid); // i+1 == len appends, as list.insert
                changed = true;
                i += 2;
            } else {
                i += 1;
            }
        }
    }
    changed
}

/// `Mesh._insert_into_face_loops`: `mid` into the FIRST `v0`/`v1` segment
/// (outer ring, then the holes).
fn insert_into_face_loops(rings: &mut [Vec<u32>], v0: u32, v1: u32, mid: u32) -> bool {
    for r in rings.iter_mut() {
        let n = r.len();
        for i in 0..n {
            let j = (i + 1) % n;
            if (r[i] == v0 && r[j] == v1) || (r[i] == v1 && r[j] == v0) {
                r.insert(i + 1, mid);
                return true;
            }
        }
    }
    false
}

/// `core.mesh._shared_edge_index`.
pub fn shared_edge_index(lp: &[u32], v0: u32, v1: u32) -> Option<usize> {
    let n = lp.len();
    (0..n).find(|&i| {
        let (a, b) = (lp[i], lp[(i + 1) % n]);
        (a == v0 && b == v1) || (a == v1 && b == v0)
    })
}

/// `core.mesh._splice_loops`.
pub fn splice_loops(a: &[u32], b: &[u32], v0: u32, v1: u32) -> Option<Vec<u32>> {
    let ia = shared_edge_index(a, v0, v1)?;
    let ib = shared_edge_index(b, v0, v1)?;
    let (na, nb) = (a.len(), b.len());
    let (u, w) = (a[ia], a[(ia + 1) % na]);
    if !(b[ib] == w && b[(ib + 1) % nb] == u) {
        return None;
    }
    let sa = (ia + 1) % na;
    let a_path: Vec<u32> = a[sa..].iter().chain(a[..sa].iter()).copied().collect();
    let sb = (ib + 1) % nb;
    let b_path: Vec<u32> = b[sb..].iter().chain(b[..sb].iter()).copied().collect();
    let mut merged: Vec<u32> = a_path[..a_path.len() - 1].to_vec();
    merged.extend_from_slice(&b_path[..b_path.len() - 1]);
    let distinct: HashSet<u32> = merged.iter().copied().collect();
    if distinct.len() != merged.len() {
        return None;
    }
    Some(merged)
}

/// `core.mesh._trace_loops`: undirected boundary segments into closed
/// loops, each started at the first segment (in outline order) no loop
/// has walked yet, in that segment's direction. None when a vertex has
/// valence other than 2 or a segment runs onto itself.
pub fn trace_loops(outline: &[(u32, u32)]) -> Option<Vec<Vec<u32>>> {
    let mut adj: HashMap<u32, Vec<u32>> = HashMap::new();
    // Insertion-ordered "set" of undirected segments.
    let mut order: Vec<(u32, u32)> = Vec::new();
    let mut left: HashSet<(u32, u32)> = HashSet::new();
    let und = |a: u32, b: u32| (a.min(b), a.max(b));
    for &(u, w) in outline {
        if u == w {
            return None;
        }
        adj.entry(u).or_default().push(w);
        adj.entry(w).or_default().push(u);
        if left.insert(und(u, w)) {
            order.push((u, w));
        }
    }
    if adj.values().any(|n| n.len() != 2) {
        return None;
    }
    let mut loops = Vec::new();
    let mut at = 0;
    while !left.is_empty() {
        while !left.contains(&und(order[at].0, order[at].1)) {
            at += 1;
        }
        let (a, b) = order[at];
        left.remove(&und(a, b));
        let mut lp = vec![a, b];
        let (mut prev, mut cur) = (a, b);
        loop {
            let nb = &adj[&cur];
            let nxt = if nb[0] != prev { nb[0] } else { nb[1] };
            left.remove(&und(cur, nxt));
            if nxt == lp[0] {
                break;
            }
            lp.push(nxt);
            prev = cur;
            cur = nxt;
        }
        loops.push(lp);
    }
    Some(loops)
}

/// A merge decided, before anything changes: what the dissolve removes
/// and the face it adds (vertex slots; the face goes in by positions, as
/// Python's `add_face` takes them).
#[derive(Debug, Clone)]
pub struct Merge {
    pub faces: Vec<u32>,
    pub edges: Vec<u32>,
    /// Remove an edge only when no face is left on it
    /// (`dissolve_coplanar_region`); else always.
    pub only_bare: bool,
    pub outer: Vec<u32>,
    pub holes: Vec<Vec<u32>>,
}

#[derive(Debug, Clone)]
pub enum Plan {
    /// The dissolve declines (Python returns None, nothing changed).
    No,
    /// `dissolve_coplanar_region` on faces stacked on one cycle: keep the
    /// first, drop the rest.
    Stacked(Vec<u32>),
    Merge(Merge),
}

/// What `split_edge_at` did, for its record.
#[derive(Debug, Clone)]
pub struct SplitAt {
    pub e0: u32,
    pub e1: u32,
    pub e0_new: bool,
    pub e1_new: bool,
    pub faces: Vec<u32>,
}

/// What `collapse_vertex` did, for its record.
#[derive(Debug, Clone)]
pub struct Collapse {
    pub e1: u32,
    pub e2: u32,
    pub n1: u32,
    pub n2: u32,
    pub edge: u32,
    pub edge_new: bool,
    pub faces: Vec<u32>,
}

impl Store {
    // ---- reads --------------------------------------------------------------

    #[inline]
    pub(super) fn position_vec(&self, v: u32) -> V3 {
        v3(self.state.vpos.get(v as usize))
    }

    #[inline]
    pub(super) fn other_end(&self, e: u32, v: u32) -> u32 {
        // Edge.other: `v1 if v == v0 else v0`.
        let [a, b] = *self.state.ends.get(e as usize);
        if v == a {
            b
        } else {
            a
        }
    }

    /// A face's outer ring (`face.loop`).
    pub fn outer_ring(&self, f: u32) -> Vec<u32> {
        let st = &self.state;
        let r = *st.frings.get(f as usize);
        if r.n_rings == 0 {
            return Vec::new();
        }
        let n = *st.ring_sizes.get(r.ring_start as usize) as usize;
        (0..n).map(|j| *st.corners.get(r.corner_start as usize + j)).collect()
    }

    /// Unit Newell vector, (0, 0, 1) for fewer than three corners or a
    /// length under 1e-9.
    pub fn ring_normal(&self, ring: &[u32], h: Hypot) -> V3 {
        let m = ring.len();
        if m < 3 {
            return V3(0.0, 0.0, 1.0);
        }
        let (mut nx, mut ny, mut nz) = (0.0f64, 0.0f64, 0.0f64);
        for k in 0..m {
            let c = self.state.vpos.get(ring[k] as usize);
            let n = self.state.vpos.get(ring[(k + 1) % m] as usize);
            nx += (c[1] - n[1]) * (c[2] + n[2]);
            ny += (c[2] - n[2]) * (c[0] + n[0]);
            nz += (c[0] - n[0]) * (c[1] + n[1]);
        }
        let n = V3(nx, ny, nz);
        let len = facegeom::length(n, h);
        if (len as f64) < 1e-9 {
            return V3(0.0, 0.0, 1.0);
        }
        V3(n.0 / len, n.1 / len, n.2 / len)
    }

    pub fn face_normal(&self, f: u32, h: Hypot) -> V3 {
        self.ring_normal(&self.outer_ring(f), h)
    }

    /// `core.mesh._loop_area`: the Newell terms summed in a QVector3D.
    fn loop_area(&self, ring: &[u32], h: Hypot) -> f64 {
        let mut sum = [0.; 3];
        for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            let p = self.state.vpos.get(a as usize);
            let q = self.state.vpos.get(b as usize);
            let term = self.quantize([
                (p[1] - q[1]) * (p[2] + q[2]),
                (p[2] - q[2]) * (p[0] + q[0]),
                (p[0] - q[0]) * (p[1] + q[1]),
            ]);
            sum = self.quantize(std::array::from_fn(|k| sum[k] + term[k]));
        }
        if self.f32_parity {
            0.5 * crate::facegeom::hypot3(sum.map(|x| x as f32), h) as f64
        } else {
            0.5 * facegeom::length(v3(&sum), h)
        }
    }

    /// `core.mesh._orient_union_loops`.
    fn orient_union_loops(&self, outer: &mut [u32], holes: &mut [Vec<u32>], n0: V3, h: Hypot) {
        if (self.ring_normal(outer, h).dot(n0, h) as f64) < 0.0 {
            outer.reverse();
        }
        for hole in holes.iter_mut() {
            if (self.ring_normal(hole, h).dot(n0, h) as f64) > 0.0 {
                hole.reverse();
            }
        }
    }

    /// The live slots of a kind in list order, and each one's place in it
    /// (NONE for a slot not listed).
    fn order_index(&self, kind: Kind) -> (Vec<u32>, Vec<u32>) {
        let live = self.live_slots(kind);
        let mut at = vec![NONE; self.state.slots[kind as usize].len()];
        for (i, &s) in live.iter().enumerate() {
            at[s as usize] = i as u32;
        }
        (live, at)
    }

    /// `Mesh.interior_vertex_on`: the first listed vertex strictly inside
    /// the edge (within `_STITCH_TOL`), QVector3D arithmetic throughout.
    pub fn interior_vertex_on(&self, e: u32, h: Hypot) -> Option<u32> {
        let [v0, v1] = *self.state.ends.get(e as usize);
        let a = self.position_vec(v0);
        let b = self.position_vec(v1);
        let ab = b.sub(a);
        let length = facegeom::length(ab, h) as f64;
        if length < STITCH_TOL {
            return None;
        }
        let lo = STITCH_TOL / length;
        let hi = 1.0 - STITCH_TOL / length;
        for v in self.live_slots(Kind::Vertex) {
            if v == v0 || v == v1 {
                continue;
            }
            let p = self.position_vec(v);
            let t = p.sub(a).dot(ab, h) as f64 / (length * length);
            if t <= lo || t >= hi {
                continue;
            }
            let foot = add(a, scale(ab, t));
            if (facegeom::length(p.sub(foot), h) as f64) < STITCH_TOL {
                return Some(v);
            }
        }
        None
    }

    /// `Mesh.collapsible_vertex`: two collinear edges (within 0.999, each
    /// longer than the tolerance) bordering the same set of faces.
    pub fn collapsible(&self, v: u32, h: Hypot) -> bool {
        if !self.alive(Kind::Vertex, v) {
            return false;
        }
        let mut it = self.edges_at(v);
        let (Some(e1), Some(e2), None) = (it.next(), it.next(), it.next()) else {
            return false;
        };
        let pv = self.position_vec(v);
        let d1 = pv.sub(self.position_vec(self.other_end(e1, v)));
        let d2 = self.position_vec(self.other_end(e2, v)).sub(pv);
        if (facegeom::length(d1, h) as f64) < STITCH_TOL || (facegeom::length(d2, h) as f64) < STITCH_TOL {
            return false;
        }
        let d = facegeom::normalized(d1, h).dot(facegeom::normalized(d2, h), h) as f64;
        if d < 0.999 {
            return false;
        }
        let a: HashSet<u32> = self.edge_faces(e1).iter().copied().collect();
        let b: HashSet<u32> = self.edge_faces(e2).iter().copied().collect();
        if a != b {
            return false;
        }
        // A loop of three keeps its corners, however thin (Mesh.collapsible_vertex:
        // a sliver hole of an STL import became a hole of two).
        !a.iter().any(|&f| self.face_rings(f).iter().any(|r| r.len() <= 3 && r.contains(&v)))
    }

    /// How many times each vertex slot is a corner of a live face (all
    /// rings): what tells a weld or a collapse whether its vertex would
    /// stay in a face it cannot reach through its edges (a face whose
    /// boundary edge was erased). One pass per top-level op, kept up to
    /// date by the welds and collapses inside it.
    pub fn corner_counts(&self) -> Vec<u32> {
        let mut n = vec![0u32; self.state.slots[0].len()];
        for f in self.live_slots(Kind::Face) {
            for r in self.face_rings(f) {
                for v in r {
                    n[v as usize] += 1;
                }
            }
        }
        n
    }

    // ---- writes -------------------------------------------------------------

    /// A face's rings replaced when they changed.
    fn put_rings(&mut self, f: u32, rings: &[Vec<u32>], ch: &mut Change) {
        self.set_rings(f, rings, true);
        ch.modified[2].push(self.key(Kind::Face, f));
    }

    fn ef_push_rec(&mut self, e: u32, f: u32, ch: &mut Change) {
        self.ef_push(e, f);
        ch.modified[1].push(self.key(Kind::Edge, e));
    }

    /// A just-removed edge's slot back as a new edge `a`–`b` with no
    /// faces: the key rules' "the new edge takes the old edge's key". Its
    /// flags are the caller's (a new Python edge's defaults, or what
    /// `split_edge_at` copies).
    fn revive_edge(&mut self, e: u32, a: u32, b: u32, flags: u8, curve: i128, layer: u32, ch: &mut Change) {
        debug_assert!(!self.alive(Kind::Edge, e));
        self.state.ends.set(e as usize, [a, b]);
        if *self.state.eflags.get(e as usize) != flags {
            self.state.eflags.set(e as usize, flags);
        }
        if *self.state.ecurve.get(e as usize) != curve {
            self.state.ecurve.set(e as usize, curve);
        }
        if *self.state.elayer.get(e as usize) != layer {
            self.state.elayer.set(e as usize, layer);
        }
        self.ef_write(e, Vec::new());
        self.wake(Kind::Edge, e);
        self.disk_link(e);
        ch.created[1].push(self.key(Kind::Edge, e));
    }

    /// `Mesh.split_edge(edge, position)`: returns `(e0, e1, bumps)`; e0 ==
    /// e1 == edge with no bump when the position welds onto an end.
    pub fn split_edge(&mut self, e: u32, p: [f64; 3], ch: &mut Change) -> Result<(u32, u32, usize), OpError> {
        if !self.alive(Kind::Edge, e) {
            return Err(OpError::Fallback("split_edge of a gone edge"));
        }
        let [v0, v1] = *self.state.ends.get(e as usize);
        let mid = self.vertex(p, ch)?;
        if mid == v0 || mid == v1 {
            return Ok((e, e, 0));
        }
        // Each incident face once, first occurrence first.
        let mut incident = Vec::new();
        for &f in self.edge_faces(e) {
            if !incident.contains(&f) {
                incident.push(f);
            }
        }
        self.remove_edge(e, ch);
        let e0 = match self.find_edge(v0, mid) {
            Some(x) => x,
            None => {
                // A new Python edge: default flags, the split edge's key.
                self.revive_edge(e, v0, mid, 0, -1, NONE, ch);
                e
            }
        };
        let e1 = self.link_edge(mid, v1, ch)?;
        for &f in &incident {
            let mut rings = self.face_rings(f);
            if insert_at_every_use(&mut rings, v0, v1, mid) {
                self.put_rings(f, &rings, ch);
            }
            for sub in [e0, e1] {
                let [a, b] = *self.state.ends.get(sub as usize);
                let have = self.edge_faces(sub).iter().filter(|&&x| x == f).count();
                for _ in have..uses(&rings, a, b) {
                    self.ef_push_rec(sub, f, ch);
                }
            }
        }
        Ok((e0, e1, 3))
    }

    /// `Mesh.split_edge_at(edge, mid)` at an existing vertex: new halves
    /// carry the edge's soft (and flat) / layer / curve (not hidden), e0
    /// its key.
    pub fn split_edge_at(&mut self, e: u32, mid: u32, ch: &mut Change) -> Result<SplitAt, OpError> {
        if !self.alive(Kind::Edge, e) || !self.alive(Kind::Vertex, mid) {
            return Err(OpError::Fallback("split_edge_at with a gone element"));
        }
        let [v0, v1] = *self.state.ends.get(e as usize);
        if mid == v0 || mid == v1 {
            return Err(OpError::Fallback("split_edge_at at an end of the edge"));
        }
        let incident = self.edge_faces(e).to_vec();
        let soft = *self.state.eflags.get(e as usize) & (SOFT | FLAT);
        let curve = *self.state.ecurve.get(e as usize);
        let layer = *self.state.elayer.get(e as usize);
        self.remove_edge(e, ch);
        let (e0, e0_new) = match self.find_edge(v0, mid) {
            Some(x) => (x, false),
            None => {
                self.revive_edge(e, v0, mid, soft, curve, layer, ch);
                (e, true)
            }
        };
        let e1_new = self.find_edge(mid, v1).is_none();
        let e1 = self.link_edge(mid, v1, ch)?;
        if e1_new {
            self.state.eflags.set(e1 as usize, soft);
            self.state.ecurve.set(e1 as usize, curve);
            self.state.elayer.set(e1 as usize, layer);
        }
        for &f in &incident {
            let mut rings = self.face_rings(f);
            if insert_into_face_loops(&mut rings, v0, v1, mid) {
                self.put_rings(f, &rings, ch);
            }
            for sub in [e0, e1] {
                if !self.edge_faces(sub).contains(&f) {
                    self.ef_push_rec(sub, f, ch);
                }
            }
        }
        Ok(SplitAt { e0, e1, e0_new, e1_new, faces: incident })
    }

    /// `_weld_into`'s order over the welded vertex's edges: by the other
    /// end's position, ties (two ends on one point) in list order.
    fn weld_order(&self, dup: u32) -> Vec<u32> {
        let mut es: Vec<u32> = self.edges_at(dup).collect();
        if es.len() < 2 {
            return es;
        }
        let pos = |e: u32| *self.state.vpos.get(self.other_end(e, dup) as usize);
        es.sort_by(|&a, &b| pos(a).partial_cmp(&pos(b)).unwrap_or(std::cmp::Ordering::Equal));
        if es.windows(2).any(|w| pos(w[0]) == pos(w[1])) {
            let (_, at) = self.order_index(Kind::Edge);
            es.sort_by(|&a, &b| {
                pos(a)
                    .partial_cmp(&pos(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(at[a as usize].cmp(&at[b as usize]))
            });
        }
        es
    }

    /// `Mesh._weld_into(keep, dup)`: `dup`'s edges move onto `keep` (a
    /// redirected edge is a NEW Python edge `keep`–`other` with default
    /// flags that takes the old key; an existing one keeps its own),
    /// zero-length edges vanish, `dup` leaves every face loop that its
    /// edges carry (repeats squeezed out), faces left with fewer than three
    /// distinct corners go, `dup` leaves the list and hands its registry
    /// cell to `keep`. Returns the bumps.
    pub fn weld_into(&mut self, keep: u32, dup: u32, occ: &mut [u32], ch: &mut Change) -> Result<usize, OpError> {
        if keep == dup || !self.alive(Kind::Vertex, keep) || !self.alive(Kind::Vertex, dup) {
            return Err(OpError::Fallback("_weld_into with a gone or the same vertex"));
        }
        if self.edges_at(dup).any(|e| self.state.ends.get(e as usize)[0] == self.state.ends.get(e as usize)[1]) {
            return Err(OpError::Fallback("_weld_into of a vertex with a self-loop"));
        }
        let mut bumps = 0;
        let mut affected: Vec<u32> = Vec::new();
        let mut seen = HashSet::new();
        for e in self.edges_at(dup) {
            for &f in self.edge_faces(e) {
                if seen.insert(f) {
                    affected.push(f);
                }
            }
        }
        let reach: u32 =
            affected.iter().map(|&f| self.face_rings(f).iter().flatten().filter(|&&x| x == dup).count() as u32).sum();
        if occ[dup as usize] != reach {
            return Err(OpError::Fallback("_weld_into would leave its vertex in a face"));
        }
        for e in self.weld_order(dup) {
            let other = self.other_end(e, dup);
            let incident = self.edge_faces(e).to_vec();
            self.remove_edge(e, ch);
            bumps += 1;
            if other == keep {
                continue;
            }
            let kept = match self.find_edge(keep, other) {
                Some(x) => x,
                None => {
                    self.revive_edge(e, keep, other, 0, -1, NONE, ch);
                    e
                }
            };
            bumps += 1;
            for f in incident {
                if !self.edge_faces(kept).contains(&f) {
                    self.ef_push_rec(kept, f, ch);
                }
            }
        }
        for f in affected {
            let mut rings = self.face_rings(f);
            for &x in rings.iter().flatten() {
                occ[x as usize] -= 1;
            }
            let mut changed = false;
            for r in rings.iter_mut() {
                for x in r.iter_mut() {
                    if *x == dup {
                        *x = keep;
                        changed = true;
                    }
                }
                let mut i = 0;
                while r.len() > 1 && i < r.len() {
                    if r[i] == r[(i + 1) % r.len()] {
                        let at = if i + 1 < r.len() { i } else { i + 1 - r.len() };
                        r.remove(at);
                        changed = true;
                    } else {
                        i += 1;
                    }
                }
            }
            if changed {
                self.put_rings(f, &rings, ch);
            }
            let distinct: HashSet<u32> = rings.first().map(|r| r.iter().copied().collect()).unwrap_or_default();
            if distinct.len() < 3 {
                self.remove_face(f, ch);
                bumps += 1;
            } else {
                for &x in rings.iter().flatten() {
                    occ[x as usize] += 1;
                }
            }
        }
        // `self.vertices.remove(dup)`; `registry[_key(dup.position)] = keep`
        // when dup held that cell.
        self.kill(Kind::Vertex, dup);
        ch.removed[0].push(self.key(Kind::Vertex, dup));
        let c = self.cell(self.state.vpos.get(dup as usize));
        if self.state.registry.get(&c) == Some(dup) {
            self.state.registry.insert(c, keep);
            if self.cell(self.state.vpos.get(keep as usize)) != c {
                Arc::make_mut(&mut self.state.extra).entry(keep).or_default().push(c);
            }
        }
        self.unfile_extra(dup);
        Ok(bumps)
    }

    /// `Mesh.weld_coincident`: returns `(changed, bumps)`.
    ///
    /// In-cell pass: per cell (first-seen order), the first vertex in list
    /// order keeps, the others weld into it. Cross-cell pass: Python scans
    /// the list for the first vertex holding its own cell with a holder of
    /// a neighbouring cell within weld distance (26 probes in dx, dy, dz
    /// order), welds the pair into the lexicographically smaller cell's
    /// vertex and starts the scan over. Here one scan does the same: a
    /// weld changes only the holder of the welded vertex's cell, so only
    /// vertices whose own cell neighbours it can start (or stop) matching;
    /// those already passed are queued and taken first when they now
    /// match, which is the vertex the restarted scan would reach first.
    pub fn weld_coincident(&mut self, ch: &mut Change) -> Result<(bool, usize), OpError> {
        if self.nothing_to_weld() {
            // What the passes below find on this state: no two vertices
            // in a cell, no holder with a partner across a cell border.
            return Ok((false, 1));
        }
        let mut bumps = 1;
        let mut changed = false;
        let mut occ = self.corner_counts();
        let (live, _) = self.order_index(Kind::Vertex);
        let mut groups: HashMap<Cell, usize> = HashMap::new();
        let mut lists: Vec<Vec<u32>> = Vec::new();
        for &v in &live {
            let c = self.cell(self.state.vpos.get(v as usize));
            let g = *groups.entry(c).or_insert_with(|| {
                lists.push(Vec::new());
                lists.len() - 1
            });
            lists[g].push(v);
        }
        for vs in &lists {
            if vs.len() < 2 {
                continue;
            }
            for &dup in &vs[1..] {
                bumps += self.weld_into(vs[0], dup, &mut occ, ch)?;
            }
            changed = true;
        }
        // Cross-cell pass.
        let (live, at) = self.order_index(Kind::Vertex);
        let mut by_cell: HashMap<Cell, Vec<u32>> = HashMap::new();
        for &v in &live {
            by_cell.entry(self.cell(self.state.vpos.get(v as usize))).or_default().push(v);
        }
        let mut heap: BinaryHeap<Reverse<u32>> = BinaryHeap::new();
        let mut i = 0usize;
        loop {
            // The next scanner: a queued earlier vertex, else the scan's.
            let (idx, from_heap) = match heap.peek() {
                Some(&Reverse(j)) if (j as usize) < i => (j as usize, true),
                _ => {
                    if i >= live.len() {
                        break;
                    }
                    (i, false)
                }
            };
            if from_heap {
                heap.pop();
            }
            let v = live[idx];
            let hit = self.cross_partner(v);
            match hit {
                None => {
                    if !from_heap {
                        i += 1;
                    }
                }
                Some((k, nk, w)) => {
                    let (kp, dp) = if k < nk { (v, w) } else { (w, v) };
                    let mut changed_cells = vec![self.cell(self.state.vpos.get(dp as usize))];
                    if let Some(cs) = self.state.extra.get(&dp) {
                        changed_cells.extend(cs.iter().copied());
                    }
                    bumps += self.weld_into(kp, dp, &mut occ, ch)?;
                    changed = true;
                    // Scanners whose own cell neighbours the cell that
                    // changed holder, already passed: look again.
                    for dc in changed_cells {
                        for dx in -1..=1i64 {
                            for dy in -1..=1i64 {
                                for dz in -1..=1i64 {
                                    if let Some(us) = by_cell.get(&(dc.0 + dx, dc.1 + dy, dc.2 + dz)) {
                                        for &u in us {
                                            let j = at[u as usize];
                                            if j != NONE && (j as usize) < i && self.alive(Kind::Vertex, u) {
                                                heap.push(Reverse(j));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // The scanner itself is looked at again where it is.
                    if from_heap && self.alive(Kind::Vertex, v) {
                        heap.push(Reverse(idx as u32));
                    }
                }
            }
        }
        Ok((changed, bumps))
    }

    /// Whether `weld_coincident` would weld nothing here: no cell holds
    /// two live vertices (its first pass), and no vertex has a partner
    /// across a cell border (`cross_partner`, which only welds change: with
    /// nothing welded, the second pass asks the state as it is). Read
    /// only, in parallel; a push on a 267k-vertex mesh spent 80 ms per
    /// stitch walking both passes to find nothing.
    fn nothing_to_weld(&self) -> bool {
        use rayon::prelude::*;
        let live = self.live_slots(Kind::Vertex);
        let mut cells: Vec<Cell> = live.par_iter().map(|&v| self.cell(self.state.vpos.get(v as usize))).collect();
        cells.par_sort_unstable();
        if cells.windows(2).any(|w| w[0] == w[1]) {
            return false;
        }
        !live.par_iter().any(|&v| self.cross_partner(v).is_some())
    }

    /// Whether `dedupe_faces` would drop nothing: no two live faces with
    /// the same outer-ring edge signature. Read only, in parallel.
    fn nothing_to_dedupe(&self) -> bool {
        use rayon::prelude::*;
        let sigs: Vec<Vec<(u32, u32)>> = self
            .live_slots(Kind::Face)
            .par_iter()
            .map(|&f| {
                let lp = self.outer_ring(f);
                let n = lp.len();
                let mut sig: Vec<(u32, u32)> = (0..n)
                    .map(|i| {
                        let (a, b) = (lp[i], lp[(i + 1) % n]);
                        (a.min(b), a.max(b))
                    })
                    .collect();
                sig.sort_unstable();
                sig.dedup();
                sig
            })
            .collect();
        let mut sigs = sigs;
        sigs.par_sort_unstable();
        !sigs.windows(2).any(|w| w[0] == w[1])
    }

    /// The cross-cell pass's test of one vertex: `(its cell, the
    /// neighbour cell, that cell's holder)` for the first match.
    fn cross_partner(&self, v: u32) -> Option<(Cell, Cell, u32)> {
        if !self.alive(Kind::Vertex, v) {
            return None;
        }
        let p = *self.state.vpos.get(v as usize);
        let k = self.cell(&p);
        if self.state.registry.get(&k) != Some(v) {
            return None;
        }
        for dx in -1..=1i64 {
            for dy in -1..=1i64 {
                for dz in -1..=1i64 {
                    let nk = (k.0 + dx, k.1 + dy, k.2 + dz);
                    if nk == k {
                        continue;
                    }
                    let Some(w) = self.state.registry.get(&nk) else { continue };
                    if w == v || !self.within(self.state.vpos.get(w as usize), &p) {
                        continue;
                    }
                    return Some((k, nk, w));
                }
            }
        }
        None
    }

    /// `Mesh.collapse_vertex(v)`: v's two edges become one (`n1` → `n2`,
    /// `n1` the end with the smaller position; an existing edge keeps its
    /// key, a new one gets a fresh key), v leaves the rings of the faces
    /// of its first edge (first occurrence per ring), the list and its
    /// cell. Refused when v is not a valence-2 vertex without self-loops,
    /// or would stay a corner of one of its faces.
    pub fn collapse_vertex(&mut self, v: u32, occ: &mut [u32], ch: &mut Change) -> Result<Collapse, OpError> {
        if !self.alive(Kind::Vertex, v) {
            return Err(OpError::Fallback("collapse_vertex of a gone vertex"));
        }
        let es: Vec<u32> = self.edges_at(v).collect();
        if es.len() != 2 {
            return Err(OpError::Fallback("collapse_vertex needs exactly two edges"));
        }
        let (mut e1, mut e2) = (es[0], es[1]);
        let (mut n1, mut n2) = (self.other_end(e1, v), self.other_end(e2, v));
        if n1 == v || n2 == v {
            return Err(OpError::Fallback("collapse_vertex of a vertex with a self-loop"));
        }
        if self.state.vpos.get(n2 as usize) < self.state.vpos.get(n1 as usize) {
            std::mem::swap(&mut e1, &mut e2);
            std::mem::swap(&mut n1, &mut n2);
        }
        let mut faces = Vec::new();
        for &f in self.edge_faces(e1) {
            if !faces.contains(&f) {
                faces.push(f);
            }
        }
        if self.edge_faces(e2).iter().any(|f| !faces.contains(f)) {
            return Err(OpError::Fallback("collapse_vertex would leave its vertex in a face"));
        }
        let mut new_rings = Vec::with_capacity(faces.len());
        for &f in &faces {
            let mut rings = self.face_rings(f);
            let mut changed = false;
            for r in rings.iter_mut() {
                if let Some(i) = r.iter().position(|&x| x == v) {
                    r.remove(i);
                    changed = true;
                }
            }
            if rings.iter().flatten().any(|&x| x == v) {
                return Err(OpError::Fallback("collapse_vertex would leave its vertex in a face"));
            }
            new_rings.push((f, rings, changed));
        }
        let reach: u32 =
            faces.iter().map(|&f| self.face_rings(f).iter().flatten().filter(|&&x| x == v).count() as u32).sum();
        if occ[v as usize] != reach {
            return Err(OpError::Fallback("collapse_vertex would leave its vertex in a face"));
        }
        self.remove_edge(e1, ch);
        self.remove_edge(e2, ch);
        let edge_new = self.find_edge(n1, n2).is_none();
        let edge = self.link_edge(n1, n2, ch)?;
        for (f, rings, changed) in new_rings {
            if changed {
                self.put_rings(f, &rings, ch);
            }
            if !self.edge_faces(edge).contains(&f) {
                self.ef_push_rec(edge, f, ch);
            }
        }
        occ[v as usize] = 0;
        self.kill(Kind::Vertex, v);
        ch.removed[0].push(self.key(Kind::Vertex, v));
        self.unfile(v);
        Ok(Collapse { e1, e2, n1, n2, edge, edge_new, faces })
    }

    /// Phase 2 of `history.run_stitch`: `while` a listed vertex is
    /// collapsible, collapse the first one in list order. One scan does
    /// it: a collapse changes the collapsibility of its two neighbours
    /// only, so a neighbour already passed is queued and taken first when
    /// it now qualifies. Returns `(collapses, bumps, refused)`: `refused`
    /// is the vertex the store would not collapse (the caller finishes the
    /// loop in Python from there; everything before it is done).
    pub fn collapse_collinear(&mut self, h: Hypot, ch: &mut Change) -> (usize, usize, Option<u32>) {
        let (live, at) = self.order_index(Kind::Vertex);
        let mut occ: Option<Vec<u32>> = None;
        let mut heap: BinaryHeap<Reverse<u32>> = BinaryHeap::new();
        let (mut i, mut n) = (0usize, 0usize);
        loop {
            let (idx, from_heap) = match heap.peek() {
                Some(&Reverse(j)) if (j as usize) < i => (j as usize, true),
                _ => {
                    if i >= live.len() {
                        break;
                    }
                    (i, false)
                }
            };
            if from_heap {
                heap.pop();
            } else {
                i += 1;
            }
            let v = live[idx];
            if !self.collapsible(v, h) {
                continue;
            }
            let occ = occ.get_or_insert_with(|| self.corner_counts());
            match self.collapse_vertex(v, occ, ch) {
                Ok(c) => {
                    n += 1;
                    for u in [c.n1, c.n2] {
                        let j = at[u as usize];
                        if j != NONE && (j as usize) < i {
                            heap.push(Reverse(j));
                        }
                    }
                }
                Err(_) => return (n, 3 * n, Some(v)),
            }
        }
        (n, 3 * n, None)
    }

    /// `Mesh.dedupe_faces`: faces over the same outer edge cycle (as a set
    /// of undirected vertex pairs) — the one with more holes survives,
    /// else the first in list order. Returns the `(dropped, kept)` pairs
    /// in order (the facade carries attrs across) and the bumps.
    pub fn dedupe_faces(&mut self, ch: &mut Change) -> (Vec<(u32, u32)>, usize) {
        if self.nothing_to_dedupe() {
            return (Vec::new(), 1);
        }
        let mut bumps = 1;
        let mut seen: HashMap<Vec<(u32, u32)>, u32> = HashMap::new();
        let mut pairs = Vec::new();
        for f in self.live_slots(Kind::Face) {
            let lp = self.outer_ring(f);
            let n = lp.len();
            let mut sig: Vec<(u32, u32)> = (0..n)
                .map(|i| {
                    let (a, b) = (lp[i], lp[(i + 1) % n]);
                    (a.min(b), a.max(b))
                })
                .collect();
            sig.sort_unstable();
            sig.dedup();
            let holes = |s: &Store, x: u32| s.state.frings.get(x as usize).n_rings.saturating_sub(1);
            match seen.get(&sig).copied() {
                None => {
                    seen.insert(sig, f);
                }
                Some(other) => {
                    let (drop, keep) = if holes(self, f) > holes(self, other) { (other, f) } else { (f, other) };
                    self.remove_face(drop, ch);
                    bumps += 1;
                    pairs.push((drop, keep));
                    seen.insert(sig, keep);
                }
            }
        }
        (pairs, bumps)
    }

    // ---- dissolves: decided first, then applied --------------------------

    fn rings_of(&self, f: u32) -> Vec<Vec<u32>> {
        self.face_rings(f)
    }

    /// `Mesh.dissolve_edge` (`abs` = True: winding-tolerant, B reversed when
    /// opposite) and `dissolve_edge_recorded` (`abs` = False).
    pub fn plan_dissolve_edge(&self, e: u32, abs: bool, h: Hypot) -> Plan {
        let fs = self.edge_faces(e);
        if fs.len() != 2 {
            return Plan::No;
        }
        let (a, b) = (fs[0], fs[1]);
        if a == b {
            return Plan::No;
        }
        let dot = self.face_normal(a, h).dot(self.face_normal(b, h), h) as f64;
        if (if abs { dot.abs() } else { dot }) < 0.999 {
            return Plan::No;
        }
        let la = self.outer_ring(a);
        let mut lb = self.outer_ring(b);
        if abs && dot <= 0.0 {
            lb.reverse();
        }
        let [v0, v1] = *self.state.ends.get(e as usize);
        let Some(outer) = splice_loops(&la, &lb, v0, v1) else {
            return Plan::No;
        };
        let mut holes: Vec<Vec<u32>> = self.rings_of(a).into_iter().skip(1).collect();
        holes.extend(self.rings_of(b).into_iter().skip(1));
        Plan::Merge(Merge { faces: vec![a, b], edges: vec![e], only_bare: false, outer, holes })
    }

    /// Segment counts over the faces' rings (unordered vertex pairs, first
    /// seen first) and the outline: the segments seen once, in order.
    fn union_outline(&self, faces: &[u32]) -> (Vec<((u32, u32), usize)>, Vec<(u32, u32)>) {
        let mut idx: HashMap<(u32, u32), usize> = HashMap::new();
        let mut counts: Vec<((u32, u32), usize)> = Vec::new();
        let mut segs: Vec<(u32, u32, usize)> = Vec::new();
        for &f in faces {
            for r in self.face_rings(f) {
                let n = r.len();
                for i in 0..n {
                    let (u, w) = (r[i], r[(i + 1) % n]);
                    let k = (u.min(w), u.max(w));
                    let j = *idx.entry(k).or_insert_with(|| {
                        counts.push((k, 0));
                        counts.len() - 1
                    });
                    counts[j].1 += 1;
                    segs.push((u, w, j));
                }
            }
        }
        let outline = segs.into_iter().filter(|&(_, _, j)| counts[j].1 == 1).map(|(u, w, _)| (u, w)).collect();
        (counts, outline)
    }

    /// The traced union split into outer (largest `_loop_area`, a stable
    /// sort) and holes, wound to `n0`.
    fn union_rings(&self, mut loops: Vec<Vec<u32>>, n0: V3, h: Hypot) -> (Vec<u32>, Vec<Vec<u32>>) {
        let areas: Vec<f64> = loops.iter().map(|l| self.loop_area(l, h)).collect();
        let mut ix: Vec<usize> = (0..loops.len()).collect();
        // `sort(key=area, reverse=True)` keeps equal areas in order.
        ix.sort_by(|&a, &b| areas[b].partial_cmp(&areas[a]).unwrap_or(std::cmp::Ordering::Equal));
        let mut sorted: Vec<Vec<u32>> = ix.into_iter().map(|i| std::mem::take(&mut loops[i])).collect();
        let mut holes = sorted.split_off(1);
        let mut outer = sorted.pop().unwrap_or_default();
        self.orient_union_loops(&mut outer, &mut holes, n0, h);
        (outer, holes)
    }

    /// `Mesh.dissolve_coplanar_pair`. Fallback where Python raises (a
    /// union with no outline: `loops_out[0]` of an empty list).
    pub fn plan_dissolve_pair(&self, a: u32, b: u32, h: Hypot) -> Result<Plan, OpError> {
        if a == b {
            return Ok(Plan::No);
        }
        let na = self.face_normal(a, h);
        if (na.dot(self.face_normal(b, h), h) as f64) < 0.999 {
            return Ok(Plan::No);
        }
        let (counts, outline) = self.union_outline(&[a, b]);
        let shared: Vec<(u32, u32)> = counts.iter().filter(|c| c.1 >= 2).map(|c| c.0).collect();
        if shared.is_empty() {
            return Ok(Plan::No);
        }
        let Some(loops) = trace_loops(&outline) else {
            return Ok(Plan::No);
        };
        if loops.is_empty() {
            return Err(OpError::Fallback("dissolve_coplanar_pair of faces with no outline"));
        }
        let (outer, holes) = self.union_rings(loops, na, h);
        let edges = shared.iter().filter_map(|&(u, w)| self.find_edge(u, w)).collect();
        Ok(Plan::Merge(Merge { faces: vec![a, b], edges, only_bare: false, outer, holes }))
    }

    /// `Mesh.dissolve_coplanar_region(faces)`, faces in the caller's order.
    pub fn plan_dissolve_region(&self, faces: &[u32], h: Hypot) -> Plan {
        if faces.len() < 2 {
            return Plan::No;
        }
        let n0 = self.face_normal(faces[0], h);
        if faces[1..].iter().any(|&f| (n0.dot(self.face_normal(f, h), h) as f64).abs() < 0.999) {
            return Plan::No;
        }
        let (counts, outline) = self.union_outline(faces);
        if !counts.iter().any(|c| c.1 >= 2) {
            return Plan::No;
        }
        let Some(loops) = trace_loops(&outline) else {
            return Plan::No;
        };
        if loops.is_empty() {
            return Plan::Stacked(faces.to_vec());
        }
        let (outer, holes) = self.union_rings(loops, n0, h);
        let edges = counts.iter().filter(|c| c.1 >= 2).filter_map(|c| self.find_edge(c.0 .0, c.0 .1)).collect();
        Plan::Merge(Merge { faces: faces.to_vec(), edges, only_bare: true, outer, holes })
    }

    /// Carry out a merge: faces removed, then edges, then the merged face
    /// added from positions with the holes `keep` lets through (Python's
    /// `add_face` filter, run by the caller when there are several).
    /// Returns the new face and the bumps.
    pub fn apply_merge(&mut self, m: &Merge, keep: &[bool], ch: &mut Change) -> Result<(u32, usize), OpError> {
        let mut bumps = 0;
        for &f in &m.faces {
            self.remove_face(f, ch);
            bumps += 1;
        }
        for &e in &m.edges {
            if m.only_bare && !self.edge_faces(e).is_empty() {
                continue;
            }
            self.remove_edge(e, ch);
            bumps += 1;
        }
        let pos = |s: &Store, r: &[u32]| r.iter().map(|&v| *s.state.vpos.get(v as usize)).collect::<Vec<_>>();
        let mut rings = vec![pos(self, &m.outer)];
        for (i, hole) in m.holes.iter().enumerate() {
            if keep.get(i).copied().unwrap_or(true) {
                rings.push(pos(self, hole));
            }
        }
        bumps += 1 + rings.iter().map(|r| r.len()).sum::<usize>();
        let f = self.add_face(&rings, ch)?;
        Ok((f, bumps))
    }

    pub fn reverse_ring(&mut self, face: u32, ring: usize, ch: &mut Change) -> bool {
        let mut rings = self.face_rings(face);
        let Some(vertices) = rings.get_mut(ring) else { return false };
        vertices.reverse();
        self.put_rings(face, &rings, ch);
        true
    }

    /// `turn_face_over`'s `loop.reverse()` and `h.reverse()` (the attrs
    /// side stays in Python). A removed face is turned too, as its held
    /// object would be.
    pub fn reverse_face(&mut self, f: u32, ch: &mut Change) {
        let mut rings = self.face_rings(f);
        for r in rings.iter_mut() {
            r.reverse();
        }
        self.put_rings(f, &rings, ch);
    }
}
