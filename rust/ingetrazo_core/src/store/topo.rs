// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! The incidence primitives of `core/mesh.py` as store ops (port step 2,
//! `docs/meshstore-design.md` §8 item 2 and §15): `vertex`, `_link_edge` /
//! `add_edge`, `add_face`, `remove_edge`, `remove_face`, `relink_edge`,
//! `relink_face`, `add_hole`, `remove_hole`, `purge_debris`,
//! `drop_orphan_vertices`, `relink_vertices`. Each one is its Python
//! method line for line (list order, weld lookups, multiplicity of
//! `edge.faces`, what is left behind) and returns a [`Change`].
//!
//! **Tombstones.** A removed element keeps its slot, dead, with its data
//! in the columns (`Slots::kill`): history holds removed elements and
//! brings them back by key (`DeleteEdgesCommand.undo` → `relink_edge`),
//! and reads them while they are gone (`AddFaceCommand.undo` walks the
//! removed face's loop). Slots are therefore not reused inside one state;
//! a restore still rewinds them (the generation floor keeps keys unique).
//!
//! **What the store refuses** (`OpError::Fallback`, raised BEFORE anything
//! changes): an op whose Python result is a state the store does not
//! represent — a second edge between two vertices (`find_edge` would then
//! answer by set order), a live element pointing at a gone one (an edge
//! relinked onto a dropped vertex, a face listed on an edge after it was
//! removed, a dropped vertex still a face corner), or an element of
//! another mesh. The facade then turns the mesh into Python objects and
//! runs the Python kernel, so the result is exact either way. Invariants
//! kept: no duplicate live edges; live elements reference live elements
//! only; a dead edge is in no disk cycle; a dead vertex has no edges.

use super::cow::{CowVec, FxHashMap};
use super::edit::Change;
use super::keys::{self, Kind, NONE};
use super::{FaceRings, Inc, Store};
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpError {
    /// A NaN coordinate (Python's `round` raises ValueError).
    Nan,
    /// An infinite one (OverflowError), including a double beyond float32.
    Inf,
    /// `add_edge` whose ends weld to one vertex (Python's ValueError).
    Degenerate,
    /// More elements than a store has slots.
    Full,
    /// Not representable in the store: run the Python kernel instead.
    Fallback(&'static str),
}

fn put<T: Clone>(col: &mut CowVec<T>, i: u32, v: T, fill: T) {
    while col.len() < i as usize {
        col.push(fill.clone());
    }
    if col.len() == i as usize {
        col.push(v);
    } else {
        col.set(i as usize, v);
    }
}

#[inline]
fn side(ends: [u32; 2], v: u32) -> usize {
    if ends[0] == v {
        0
    } else {
        1
    }
}

impl Change {
    /// Each list once, first occurrence first.
    pub fn dedup(&mut self) {
        for lists in [&mut self.created, &mut self.removed, &mut self.modified] {
            for l in lists.iter_mut() {
                if l.len() > 1 {
                    let mut seen = HashSet::with_capacity(l.len());
                    l.retain(|k| seen.insert(*k));
                }
            }
        }
    }
}

impl Store {
    // ---- keys -------------------------------------------------------------

    /// The slot a key names, live or a tombstone of this state (a removed
    /// element history may still read or relink). None for another
    /// store's key, a stale generation, or a slot this state never had.
    pub fn slot_any(&self, kind: Kind, key: u128) -> Option<u32> {
        let (ns, k, gen, slot) = keys::split_key(key)?;
        let slots = &self.state.slots[kind as usize];
        if ns != self.state.ns || k != kind || (slot as usize) >= slots.len() || slots.gen(slot) != gen {
            return None;
        }
        Some(slot)
    }

    pub fn alive(&self, kind: Kind, slot: u32) -> bool {
        self.state.slots[kind as usize].alive(slot)
    }

    // ---- slots ------------------------------------------------------------

    /// A new element's slot, its columns at their defaults, listed last.
    fn new_slot(&mut self, kind: Kind) -> Result<u32, OpError> {
        let k = kind as usize;
        let st = &mut self.state;
        let (slot, _gen, on_top) = st.slots[k].alloc(&mut self.hwm[k]).ok_or(OpError::Full)?;
        match kind {
            Kind::Vertex => {
                put(&mut st.vpos, slot, [0.0; 3], [0.0; 3]);
                put(&mut st.vedge, slot, NONE, NONE);
            }
            Kind::Edge => {
                put(&mut st.ends, slot, [NONE, NONE], [NONE, NONE]);
                put(&mut st.enext, slot, [NONE, NONE], [NONE, NONE]);
                put(&mut st.eflags, slot, 0, 0);
                put(&mut st.ecurve, slot, -1, -1);
                put(&mut st.elayer, slot, NONE, NONE);
                put(&mut st.efaces, slot, Inc::default(), Inc::default());
            }
            Kind::Face => {
                put(&mut st.frings, slot, FaceRings::default(), FaceRings::default());
                put(&mut st.fattr, slot, 0, 0);
                put(&mut st.fflags, slot, 0, 0);
            }
        }
        st.order[k].append(slot, &st.slots[k], on_top);
        Ok(slot)
    }

    /// Unlist and retire a live element, keeping it as a tombstone.
    fn kill(&mut self, kind: Kind, slot: u32) {
        let k = kind as usize;
        self.state.slots[k].kill(slot);
        self.state.order[k].remove(slot);
        self.tidy_order(kind);
    }

    /// A tombstone back, listed LAST (`list.append` of the very object).
    fn wake(&mut self, kind: Kind, slot: u32) {
        let k = kind as usize;
        let st = &mut self.state;
        let ok = st.slots[k].wake(slot);
        debug_assert!(ok);
        let on_top = st.slots[k].none_live_above(slot);
        st.order[k].append(slot, &st.slots[k], on_top);
    }

    /// Compact an explicit order once stale entries outnumber live ones.
    fn tidy_order(&mut self, kind: Kind) {
        let k = kind as usize;
        let st = &mut self.state;
        let stale = st.order[k].stale();
        if stale > 1024 && stale > st.slots[k].live {
            st.order[k].compact(&st.slots[k]);
        }
    }

    // ---- disk cycles ------------------------------------------------------

    /// Put a (live) edge at the head of its ends' disk cycles.
    fn disk_link(&mut self, e: u32) {
        let st = &mut self.state;
        let [a, b] = *st.ends.get(e as usize);
        let mut nx = [NONE, NONE];
        nx[0] = *st.vedge.get(a as usize);
        st.vedge.set(a as usize, e);
        if b != a {
            nx[1] = *st.vedge.get(b as usize);
            st.vedge.set(b as usize, e);
        }
        st.enext.set(e as usize, nx);
    }

    fn disk_unlink(&mut self, e: u32) {
        let [a, b] = *self.state.ends.get(e as usize);
        self.unlink_at(a, e);
        if b != a {
            self.unlink_at(b, e);
        }
    }

    fn unlink_at(&mut self, v: u32, e: u32) {
        let st = &mut self.state;
        let after = st.enext.get(e as usize)[side(*st.ends.get(e as usize), v)];
        let (mut prev, mut cur) = (NONE, *st.vedge.get(v as usize));
        while cur != NONE && cur != e {
            prev = cur;
            cur = st.enext.get(cur as usize)[side(*st.ends.get(cur as usize), v)];
        }
        if cur == NONE {
            return;
        }
        if prev == NONE {
            st.vedge.set(v as usize, after);
        } else {
            let s = side(*st.ends.get(prev as usize), v);
            st.enext.get_mut(prev as usize)[s] = after;
        }
    }

    // ---- edge.faces -------------------------------------------------------

    fn ef_write(&mut self, e: u32, fs: Vec<u32>) {
        let st = &mut self.state;
        let inc = *st.efaces.get(e as usize);
        if fs.len() <= 2 {
            if inc.len > 2 {
                st.ef_spill.set(inc.a[0] as usize, Vec::new());
            }
            let mut a = [NONE; 2];
            a[..fs.len()].copy_from_slice(&fs);
            st.efaces.set(e as usize, Inc { len: fs.len() as u32, a });
        } else if inc.len > 2 {
            let n = fs.len() as u32;
            st.ef_spill.set(inc.a[0] as usize, fs);
            st.efaces.get_mut(e as usize).len = n;
        } else {
            let n = fs.len() as u32;
            st.ef_spill.push(fs);
            let idx = (st.ef_spill.len() - 1) as u32;
            st.efaces.set(e as usize, Inc { len: n, a: [idx, NONE] });
        }
    }

    /// `edge.faces.append(f)`.
    fn ef_push(&mut self, e: u32, f: u32) {
        let inc = *self.state.efaces.get(e as usize);
        if inc.len < 2 {
            let st = &mut self.state;
            let x = st.efaces.get_mut(e as usize);
            x.a[x.len as usize] = f;
            x.len += 1;
            return;
        }
        let mut fs = self.edge_faces(e).to_vec();
        fs.push(f);
        self.ef_write(e, fs);
    }

    /// `if f in edge.faces: edge.faces.remove(f)` (the first occurrence).
    fn ef_remove(&mut self, e: u32, f: u32) -> bool {
        let fs = self.edge_faces(e);
        match fs.iter().position(|&x| x == f) {
            None => false,
            Some(i) => {
                let mut fs = fs.to_vec();
                fs.remove(i);
                self.ef_write(e, fs);
                true
            }
        }
    }

    // ---- face rings -------------------------------------------------------

    /// Give a face new rings: appended at the pools' ends, the old range
    /// left as garbage (compacted, tombstones included, when it outgrows
    /// the live pool).
    fn set_rings(&mut self, f: u32, rings: &[Vec<u32>], had: bool) {
        let st = &mut self.state;
        if had {
            let r = *st.frings.get(f as usize);
            let mut n = 0usize;
            for k in 0..r.n_rings as usize {
                n += *st.ring_sizes.get(r.ring_start as usize + k) as usize;
            }
            st.pool_garbage += n + r.n_rings as usize;
        }
        let fr = FaceRings {
            corner_start: st.corners.len() as u32,
            ring_start: st.ring_sizes.len() as u32,
            n_rings: rings.len() as u32,
        };
        for r in rings {
            st.ring_sizes.push(r.len() as u32);
            for &v in r {
                st.corners.push(v);
            }
        }
        st.frings.set(f as usize, fr);
        if st.pool_garbage > 65_536 && st.pool_garbage > (st.corners.len() + st.ring_sizes.len()) / 2 {
            self.compact_pools();
        }
    }

    /// Rewrite the pools in slot order, live faces and tombstones alike
    /// (a removed face's loop is still read and relinked).
    fn compact_pools(&mut self) {
        let n = self.state.slots[Kind::Face as usize].len();
        let mut corners = Vec::with_capacity(self.state.corners.len() - self.state.pool_garbage);
        let mut sizes = Vec::new();
        let mut fr = Vec::with_capacity(n);
        for f in 0..n as u32 {
            let rings = self.face_rings(f);
            fr.push(FaceRings {
                corner_start: corners.len() as u32,
                ring_start: sizes.len() as u32,
                n_rings: rings.len() as u32,
            });
            for r in rings {
                sizes.push(r.len() as u32);
                corners.extend(r);
            }
        }
        let st = &mut self.state;
        for (f, r) in fr.into_iter().enumerate() {
            if *st.frings.get(f) != r {
                st.frings.set(f, r);
            }
        }
        st.corners = CowVec::from_vec(corners);
        st.ring_sizes = CowVec::from_vec(sizes);
        st.pool_garbage = 0;
    }

    // ---- the ops ----------------------------------------------------------

    fn finite(&self, p: [f64; 3]) -> Result<[f64; 3], OpError> {
        let q = self.quantize(p);
        if q.iter().any(|c| c.is_nan()) {
            return Err(OpError::Nan);
        }
        if q.iter().any(|c| c.is_infinite()) {
            return Err(OpError::Inf);
        }
        Ok(q)
    }

    /// `Mesh.vertex`: the cell's holder, else a probe within weld
    /// distance, else a new vertex that claims the cell.
    pub fn vertex(&mut self, p: [f64; 3], ch: &mut Change) -> Result<u32, OpError> {
        let q = self.finite(p)?;
        let k = self.cell(&q);
        if let Some(v) = self.state.registry.get(&k) {
            return Ok(v);
        }
        if let Some(v) = self.probe(k, &q) {
            return Ok(v);
        }
        let v = self.new_slot(Kind::Vertex)?;
        self.state.vpos.set(v as usize, q);
        self.state.registry.insert(k, v);
        ch.created[0].push(self.key(Kind::Vertex, v));
        Ok(v)
    }

    /// `Mesh._link_edge`: the edge between two live vertices, made when
    /// missing (in this direction, listed last, at the head of both disk
    /// cycles; a vertex onto itself makes a self-loop, as in Python).
    pub fn link_edge(&mut self, a: u32, b: u32, ch: &mut Change) -> Result<u32, OpError> {
        if let Some(e) = self.find_edge(a, b) {
            return Ok(e);
        }
        let e = self.new_slot(Kind::Edge)?;
        self.state.ends.set(e as usize, [a, b]);
        self.disk_link(e);
        ch.created[1].push(self.key(Kind::Edge, e));
        Ok(e)
    }

    /// `Mesh.add_edge`: both ends welded, then the edge. The first end's
    /// vertex is made even when the second welds onto it (the error comes
    /// after, as in Python).
    pub fn add_edge(&mut self, a: [f64; 3], b: [f64; 3], ch: &mut Change) -> Result<u32, OpError> {
        let v0 = self.vertex(a, ch)?;
        let v1 = self.vertex(b, ch)?;
        if v0 == v1 {
            return Err(OpError::Degenerate);
        }
        self.link_edge(v0, v1, ch)
    }

    /// `Mesh.add_face` after the nested-hole filter (Python side): every
    /// corner welded (outer ring, then the holes in order), then every
    /// segment's edge linked and the face listed on it once PER USE (a
    /// slit or jamb lists it twice).
    pub fn add_face(&mut self, rings: &[Vec<[f64; 3]>], ch: &mut Change) -> Result<u32, OpError> {
        let mut vr: Vec<Vec<u32>> = Vec::with_capacity(rings.len());
        for r in rings {
            let mut ring = Vec::with_capacity(r.len());
            for &p in r {
                ring.push(self.vertex(p, ch)?);
            }
            vr.push(ring);
        }
        let f = self.new_slot(Kind::Face)?;
        self.set_rings(f, &vr, false);
        for ring in &vr {
            let n = ring.len();
            for i in 0..n {
                let made = ch.created[1].len();
                let e = self.link_edge(ring[i], ring[(i + 1) % n], ch)?;
                if ch.created[1].len() == made {
                    ch.modified[1].push(self.key(Kind::Edge, e));
                }
                self.ef_push(e, f);
            }
        }
        ch.created[2].push(self.key(Kind::Face, f));
        Ok(f)
    }

    /// `Mesh.remove_edge`: unlinked from its ends and unlisted; its faces
    /// list is left as it is (callers remove faces first), as are its
    /// vertices. A gone edge: nothing.
    pub fn remove_edge(&mut self, e: u32, ch: &mut Change) {
        if !self.alive(Kind::Edge, e) {
            return;
        }
        self.disk_unlink(e);
        self.kill(Kind::Edge, e);
        let [a, b] = *self.state.ends.get(e as usize);
        ch.removed[1].push(self.key(Kind::Edge, e));
        ch.modified[0].push(self.key(Kind::Vertex, a));
        ch.modified[0].push(self.key(Kind::Vertex, b));
    }

    /// `Mesh.remove_face`: detached from the edges `find_edge` gives for
    /// each segment of its rings (one occurrence per segment), unlisted.
    /// Its edges stay. A gone face is still detached (Python walks its
    /// loops whether or not it is listed).
    pub fn remove_face(&mut self, f: u32, ch: &mut Change) {
        for ring in self.face_rings(f) {
            let n = ring.len();
            for i in 0..n {
                if let Some(e) = self.find_edge(ring[i], ring[(i + 1) % n]) {
                    if self.ef_remove(e, f) {
                        ch.modified[1].push(self.key(Kind::Edge, e));
                    }
                }
            }
        }
        if self.alive(Kind::Face, f) {
            self.kill(Kind::Face, f);
            ch.removed[2].push(self.key(Kind::Face, f));
        }
    }

    /// What `relink_edge` would refuse (see the module docs).
    pub fn relink_edge_blocked(&self, e: u32) -> Option<&'static str> {
        if self.alive(Kind::Edge, e) {
            return None;
        }
        let [a, b] = *self.state.ends.get(e as usize);
        if !self.alive(Kind::Vertex, a) || !self.alive(Kind::Vertex, b) {
            return Some("relink_edge onto a gone vertex");
        }
        if self.find_edge(a, b).is_some() {
            return Some("relink_edge beside another edge between its ends");
        }
        if self.edge_faces(e).iter().any(|&f| !self.alive(Kind::Face, f)) {
            return Some("relink_edge listing a gone face");
        }
        None
    }

    /// `Mesh.relink_edge`: a removed edge back with its key, at the end of
    /// the list, its faces list as it was; each end claims its cell when
    /// the cell is free (`registry.setdefault`). A live edge: nothing.
    pub fn relink_edge(&mut self, e: u32, ch: &mut Change) -> Result<bool, OpError> {
        if self.alive(Kind::Edge, e) {
            return Ok(false);
        }
        if let Some(why) = self.relink_edge_blocked(e) {
            return Err(OpError::Fallback(why));
        }
        self.wake(Kind::Edge, e);
        self.disk_link(e);
        let [a, b] = *self.state.ends.get(e as usize);
        for v in [a, b] {
            let c = self.cell(self.state.vpos.get(v as usize));
            if !self.state.registry.contains(&c) {
                self.state.registry.insert(c, v);
            }
        }
        ch.created[1].push(self.key(Kind::Edge, e));
        Ok(true)
    }

    /// `Mesh.relink_face`: a removed face back with its key, at the end of
    /// the list; every segment's edge linked (made when missing) and the
    /// face listed on it when it is not already (once, unlike add_face).
    pub fn relink_face(&mut self, f: u32, ch: &mut Change) -> Result<bool, OpError> {
        if self.alive(Kind::Face, f) {
            return Ok(false);
        }
        let rings = self.face_rings(f);
        if rings.iter().flatten().any(|&v| !self.alive(Kind::Vertex, v)) {
            return Err(OpError::Fallback("relink_face onto a gone vertex"));
        }
        for ring in &rings {
            let n = ring.len();
            for i in 0..n {
                let e = self.link_edge(ring[i], ring[(i + 1) % n], ch)?;
                if !self.edge_faces(e).contains(&f) {
                    self.ef_push(e, f);
                    ch.modified[1].push(self.key(Kind::Edge, e));
                }
            }
        }
        self.wake(Kind::Face, f);
        ch.created[2].push(self.key(Kind::Face, f));
        Ok(true)
    }

    /// `Mesh.add_hole` on a live face: the loop welded, appended to the
    /// face's holes, its edges linked and the face listed on each when
    /// missing. Returns the loop.
    pub fn add_hole(&mut self, f: u32, ring: &[[f64; 3]], ch: &mut Change) -> Result<Vec<u32>, OpError> {
        if !self.alive(Kind::Face, f) {
            return Err(OpError::Fallback("add_hole on a gone face"));
        }
        let mut loop_ = Vec::with_capacity(ring.len());
        for &p in ring {
            loop_.push(self.vertex(p, ch)?);
        }
        let mut rings = self.face_rings(f);
        rings.push(loop_.clone());
        self.set_rings(f, &rings, true);
        let n = loop_.len();
        for i in 0..n {
            let e = self.link_edge(loop_[i], loop_[(i + 1) % n], ch)?;
            if !self.edge_faces(e).contains(&f) {
                self.ef_push(e, f);
                ch.modified[1].push(self.key(Kind::Edge, e));
            }
        }
        ch.modified[2].push(self.key(Kind::Face, f));
        Ok(loop_)
    }

    /// `Mesh.remove_hole`: the first hole equal to `ring` (vertex for
    /// vertex) leaves the face, and the face leaves the edges of `ring`'s
    /// segments (one occurrence each) — whether or not it was a hole.
    pub fn remove_hole(&mut self, f: u32, ring: &[u32], ch: &mut Change) {
        let mut rings = self.face_rings(f);
        if let Some(i) = rings.iter().skip(1).position(|h| h.as_slice() == ring) {
            rings.remove(i + 1);
            self.set_rings(f, &rings, true);
            ch.modified[2].push(self.key(Kind::Face, f));
        }
        let n = ring.len();
        for i in 0..n {
            if let Some(e) = self.find_edge(ring[i], ring[(i + 1) % n]) {
                if self.ef_remove(e, f) {
                    ch.modified[1].push(self.key(Kind::Edge, e));
                }
            }
        }
    }

    /// Mark the vertex slots that are corners of live faces.
    fn face_corners(&self) -> Vec<bool> {
        let st = &self.state;
        let mut used = vec![false; st.slots[0].len()];
        for f in st.slots[2].live_slots() {
            let r = *st.frings.get(f as usize);
            let mut n = 0usize;
            for k in 0..r.n_rings as usize {
                n += *st.ring_sizes.get(r.ring_start as usize + k) as usize;
            }
            for c in r.corner_start as usize..r.corner_start as usize + n {
                used[*st.corners.get(c) as usize] = true;
            }
        }
        used
    }

    fn unfile(&mut self, v: u32) -> bool {
        let c = self.cell(self.state.vpos.get(v as usize));
        if self.state.registry.get(&c) == Some(v) {
            self.state.registry.remove(&c);
            return true;
        }
        false
    }

    /// `Mesh.purge_debris`: drop the listed vertices with no edges that
    /// no live face uses, and their registry entries; the displaced queues
    /// keep only listed survivors (done even when nothing goes, as in
    /// Python). Returns `(vertices dropped, entries dropped)` — a store
    /// has no ghost entries (design decision 4), so the second counts the
    /// dropped vertices' own cells.
    pub fn purge_debris(&mut self, ch: &mut Change) -> (usize, usize) {
        let used = self.face_corners();
        let doomed: Vec<u32> = self
            .live_slots(Kind::Vertex)
            .into_iter()
            .filter(|&v| *self.state.vedge.get(v as usize) == NONE && !used[v as usize])
            .collect();
        let gone: HashSet<u32> = doomed.iter().copied().collect();
        let keep = |s: &Store, w: u32| s.alive(Kind::Vertex, w) && !gone.contains(&w);
        if self.state.displaced.iter().any(|(_, q)| q.iter().any(|&w| !keep(self, w))) {
            let mut d: FxHashMap<_, Vec<u32>> = (*self.state.displaced).clone();
            d.retain(|_, q| {
                q.retain(|&w| keep(self, w));
                !q.is_empty()
            });
            self.state.displaced = Arc::new(d);
        }
        let mut ghosts = 0;
        for &v in &doomed {
            if self.unfile(v) {
                ghosts += 1;
            }
            self.kill(Kind::Vertex, v);
            ch.removed[0].push(self.key(Kind::Vertex, v));
        }
        (doomed.len(), ghosts)
    }

    /// `Mesh.drop_orphan_vertices`: of `vs`, the listed ones with no edges
    /// leave the list and their own cell. Refused (before any change) when
    /// one of them is still a live face's corner — a face whose boundary
    /// edge was erased — since a live face would then name a gone vertex.
    pub fn drop_orphan_vertices(&mut self, vs: &[u32], ch: &mut Change) -> Result<usize, OpError> {
        let mut doomed = Vec::new();
        let mut seen = HashSet::new();
        for &v in vs {
            if self.alive(Kind::Vertex, v) && *self.state.vedge.get(v as usize) == NONE && seen.insert(v) {
                doomed.push(v);
            }
        }
        if doomed.is_empty() {
            return Ok(0);
        }
        let used = self.face_corners();
        if doomed.iter().any(|&v| used[v as usize]) {
            return Err(OpError::Fallback("drop_orphan_vertices on a face corner"));
        }
        // Python filters the list in list order.
        let mut ordered = doomed.clone();
        let dense = self.state.order[0].dense_of_slot(&self.state.slots[0]);
        ordered.sort_by_key(|&v| dense[v as usize]);
        for v in ordered {
            self.unfile(v);
            self.kill(Kind::Vertex, v);
            ch.removed[0].push(self.key(Kind::Vertex, v));
        }
        Ok(doomed.len())
    }

    /// `Mesh.relink_vertices`: each gone vertex of `vs` back with its key,
    /// listed last, claiming its cell if free; listed ones are skipped.
    pub fn relink_vertices(&mut self, vs: &[u32], ch: &mut Change) {
        for &v in vs {
            if self.alive(Kind::Vertex, v) {
                continue;
            }
            self.wake(Kind::Vertex, v);
            let c = self.cell(self.state.vpos.get(v as usize));
            if !self.state.registry.contains(&c) {
                self.state.registry.insert(c, v);
            }
            ch.created[0].push(self.key(Kind::Vertex, v));
        }
    }

    /// Bump the revision and stamp the record with it.
    pub fn finish(&mut self, mut ch: Change) -> Change {
        self.revision += 1;
        ch.revision = self.revision;
        ch.dedup();
        ch
    }

    /// The invariants above, for tests: `Err` names the first one broken.
    pub fn check(&self) -> Result<(), String> {
        let st = &self.state;
        let mut pairs = HashSet::new();
        for e in st.slots[1].live_slots() {
            let [a, b] = *st.ends.get(e as usize);
            if !self.alive(Kind::Vertex, a) || !self.alive(Kind::Vertex, b) {
                return Err(format!("edge {e} on a gone vertex"));
            }
            if !pairs.insert((a.min(b), a.max(b))) {
                return Err(format!("duplicate edge {e}"));
            }
            for &f in self.edge_faces(e) {
                if !self.alive(Kind::Face, f) {
                    return Err(format!("edge {e} lists gone face {f}"));
                }
            }
            for v in [a, b] {
                if !self.edges_at(v).any(|x| x == e) {
                    return Err(format!("edge {e} missing from vertex {v}'s cycle"));
                }
            }
        }
        for v in 0..st.slots[0].len() as u32 {
            let n = self.edges_at(v).count();
            if !self.alive(Kind::Vertex, v) && n > 0 {
                return Err(format!("gone vertex {v} has edges"));
            }
            for e in self.edges_at(v) {
                if !self.alive(Kind::Edge, e) {
                    return Err(format!("gone edge {e} in vertex {v}'s cycle"));
                }
            }
        }
        for f in st.slots[2].live_slots() {
            if self.face_rings(f).iter().flatten().any(|&v| !self.alive(Kind::Vertex, v)) {
                return Err(format!("face {f} on a gone vertex"));
            }
        }
        for (c, v) in st.registry.iter() {
            if !self.alive(Kind::Vertex, *v) {
                return Err(format!("registry cell {c:?} holds gone vertex {v}"));
            }
        }
        for k in Kind::ALL {
            let o = &st.order[k as usize];
            if o.live(&st.slots[k as usize]).len() != st.slots[k as usize].live {
                return Err(format!("{} order out of step", k.name()));
            }
        }
        Ok(())
    }
}
