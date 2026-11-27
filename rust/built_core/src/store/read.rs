// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! Reading a store back as dense arrays, in `mesh.vertices` /
//! `mesh.edges` / `mesh.faces` order — the `lazy_mesh.Plan` layout, so
//! code that reads a plan today reads a store with the same indices.
//!
//! Dense index = position among the live entries of the list order. A
//! store fresh from a block is the identity (slot = index); after edits
//! the remap is built here, once per call.

use super::keys::{Kind, NONE};
use super::{State, Store};

/// The `Plan` arrays.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dense {
    pub pos: Vec<[f64; 3]>,
    pub edges: Vec<[i64; 2]>,
    pub flags: Vec<u8>,
    pub curve: Vec<i128>,
    pub layer: Vec<i64>,
    pub corners: Vec<i64>,
    pub ring_sizes: Vec<i64>,
    pub ring_counts: Vec<i64>,
    pub ef_start: Vec<i64>,
    pub ef_face: Vec<i64>,
    pub face_attr: Vec<i64>,
    pub face_flags: Vec<u8>,
}

impl Store {
    pub fn dense_maps(&self) -> [Vec<u32>; 3] {
        let (o, s) = (&self.state.order, &self.state.slots);
        [o[0].dense_of_slot(&s[0]), o[1].dense_of_slot(&s[1]), o[2].dense_of_slot(&s[2])]
    }

    pub fn dense(&self) -> Dense {
        self.dense_parts(true, true)
    }

    /// `dense`, leaving the edges' faces (`incidence`) and the faces'
    /// arrays (`faces`) empty when not asked for: a reader of positions and
    /// edges alone skips most of the remap.
    pub fn dense_parts(&self, incidence: bool, faces: bool) -> Dense {
        let st = &self.state;
        let dv = st.order[0].dense_of_slot(&st.slots[0]);
        let df = if incidence { st.order[2].dense_of_slot(&st.slots[2]) } else { Vec::new() };
        let mut d = Dense::default();
        for v in st.order[Kind::Vertex as usize].live(&st.slots[Kind::Vertex as usize]) {
            d.pos.push(*st.vpos.get(v as usize));
        }
        d.ef_start.push(0);
        for e in st.order[Kind::Edge as usize].live(&st.slots[Kind::Edge as usize]) {
            let [a, b] = *st.ends.get(e as usize);
            d.edges.push([dv[a as usize] as i64, dv[b as usize] as i64]);
            d.flags.push(*st.eflags.get(e as usize));
            d.curve.push(*st.ecurve.get(e as usize));
            let l = *st.elayer.get(e as usize);
            d.layer.push(if l == NONE { -1 } else { l as i64 });
            if incidence {
                for &f in self.edge_faces(e) {
                    d.ef_face.push(df[f as usize] as i64);
                }
                d.ef_start.push(d.ef_face.len() as i64);
            }
        }
        if !incidence {
            d.ef_start.clear();
        }
        if !faces {
            return d;
        }
        for f in st.order[Kind::Face as usize].live(&st.slots[Kind::Face as usize]) {
            let r = *st.frings.get(f as usize);
            d.ring_counts.push(r.n_rings as i64);
            let mut c = r.corner_start as usize;
            for k in 0..r.n_rings as usize {
                let size = *st.ring_sizes.get(r.ring_start as usize + k) as usize;
                d.ring_sizes.push(size as i64);
                for j in 0..size {
                    d.corners.push(dv[*st.corners.get(c + j) as usize] as i64);
                }
                c += size;
            }
            d.face_attr.push(*st.fattr.get(f as usize) as i64 - 1);
            d.face_flags.push(*st.fflags.get(f as usize));
        }
        d
    }

    /// Live keys of a kind, in list order (local 40-bit parts).
    pub fn local_keys(&self, kind: Kind) -> Vec<u64> {
        let slots = &self.state.slots[kind as usize];
        self.state.order[kind as usize]
            .live(slots)
            .into_iter()
            .map(|s| super::keys::local_key(kind, slots.gen(s), s))
            .collect()
    }

    /// Allocated bytes per column (what this store holds, counting chunks
    /// shared with snapshots once, here).
    pub fn memory(&self) -> Vec<(&'static str, Kind, usize)> {
        let st = &self.state;
        let o = |k: Kind| st.order[k as usize].heap_bytes();
        let hwm = |k: usize| self.hwm[k].over.capacity() * 9;
        let spill: usize = st.ef_spill.iter().map(|v| v.capacity() * 4).sum();
        vec![
            ("vpos", Kind::Vertex, st.vpos.heap_bytes()),
            ("vedge", Kind::Vertex, st.vedge.heap_bytes()),
            ("vslots", Kind::Vertex, st.slots[0].state.heap_bytes() + hwm(0)),
            ("vorder", Kind::Vertex, o(Kind::Vertex)),
            ("registry", Kind::Vertex, st.registry.heap_bytes()),
            ("ends", Kind::Edge, st.ends.heap_bytes()),
            ("enext", Kind::Edge, st.enext.heap_bytes()),
            ("eflags", Kind::Edge, st.eflags.heap_bytes()),
            ("ecurve", Kind::Edge, st.ecurve.heap_bytes()),
            ("elayer", Kind::Edge, st.elayer.heap_bytes()),
            ("efaces", Kind::Edge, st.efaces.heap_bytes() + st.ef_spill.heap_bytes() + spill),
            ("eslots", Kind::Edge, st.slots[1].state.heap_bytes() + hwm(1)),
            ("eorder", Kind::Edge, o(Kind::Edge)),
            ("frings", Kind::Face, st.frings.heap_bytes()),
            ("fattr", Kind::Face, st.fattr.heap_bytes()),
            ("fflags", Kind::Face, st.fflags.heap_bytes()),
            ("corners", Kind::Face, st.corners.heap_bytes()),
            ("ring_sizes", Kind::Face, st.ring_sizes.heap_bytes()),
            ("fslots", Kind::Face, st.slots[2].state.heap_bytes() + hwm(2)),
            ("forder", Kind::Face, o(Kind::Face)),
            // The store struct and the per-column headers (a small mesh's
            // fixed cost), counted with the vertices.
            ("fixed", Kind::Vertex, std::mem::size_of::<Store>() + st.displaced.capacity() * 40
                + st.layers.iter().map(|l| l.capacity() + 24).sum::<usize>()),
        ]
    }
}

impl State {
    /// `capture_state() == other` for two states of one store: the same
    /// elements (keys) in the same list order with the same fields, the
    /// same registry. `displaced` is not compared: Python's snapshot does
    /// not hold it. Chunks shared by both are skipped unread — a state
    /// undone back to a snapshot compares in O(chunks).
    pub fn same_as(&self, o: &State) -> bool {
        if self.ns != o.ns {
            return false;
        }
        for k in 0..3 {
            let (a, b) = (&self.slots[k], &o.slots[k]);
            if a.live != b.live {
                return false;
            }
            // Live slots and their generations; tombstones (removed
            // elements kept for relinking) are not part of what Python's
            // snapshot compares, so slot counts may differ.
            if !live_gens_eq(a, b) {
                return false;
            }
            let (oa, ob) = (&self.order[k], &o.order[k]);
            let shared = match (&oa.explicit, &ob.explicit) {
                (None, None) => true, // both ascending live slots, compared above
                (Some(x), Some(y)) => cols_eq_ptr(&x.list, &y.list) && cols_eq_ptr(&x.at, &y.at),
                _ => false,
            };
            if !shared && oa.live(a) != ob.live(b) {
                return false;
            }
        }
        let live = |k: usize, s: usize| self.slots[k].state.get(s) & 1 == 1;
        let ok = cols_eq_live(&self.vpos, &o.vpos, |s| live(0, s))
            && cols_eq_live(&self.ends, &o.ends, |s| live(1, s))
            && cols_eq_live(&self.eflags, &o.eflags, |s| live(1, s))
            && cols_eq_live(&self.ecurve, &o.ecurve, |s| live(1, s))
            && cols_eq_live(&self.elayer, &o.elayer, |s| live(1, s))
            && cols_eq_live(&self.fattr, &o.fattr, |s| live(2, s))
            && cols_eq_live(&self.fflags, &o.fflags, |s| live(2, s))
            && [Kind::Edge, Kind::Face].into_iter().all(|k| self.exact.same_live(&o.exact, k, |s| self.slots[k as usize].alive(s)))
            && self.registry.same_as(&o.registry)
            && layers_agree(&self.layers, &o.layers);
        if !ok {
            return false;
        }
        // Incidence, rings and vertex adjacency by content (pool layout may
        // differ), a whole chunk skipped when both still share it — so an
        // unchanged state compares in O(chunks).
        let spill_shared = cols_eq_ptr(&self.ef_spill, &o.ef_spill);
        for (c, range) in chunk_ranges(self.slots[1].len()) {
            if spill_shared && self.efaces.same_chunk(&o.efaces, c) {
                continue;
            }
            if range.filter(|&e| live(1, e)).any(|e| edge_faces(self, e as u32) != edge_faces(o, e as u32)) {
                return false;
            }
        }
        let pools_shared =
            cols_eq_ptr(&self.corners, &o.corners) && cols_eq_ptr(&self.ring_sizes, &o.ring_sizes);
        for (c, range) in chunk_ranges(self.slots[2].len()) {
            if pools_shared && self.frings.same_chunk(&o.frings, c) {
                continue;
            }
            if range.filter(|&f| live(2, f)).any(|f| rings(self, f as u32) != rings(o, f as u32)) {
                return false;
            }
        }
        // Vertex adjacency as sets (Python compares `Vertex.edges` sets).
        let links_shared = cols_eq_ptr(&self.enext, &o.enext) && cols_eq_ptr(&self.ends, &o.ends);
        for (c, range) in chunk_ranges(self.slots[0].len()) {
            if links_shared && self.vedge.same_chunk(&o.vedge, c) {
                continue;
            }
            for v in range.filter(|&v| live(0, v)) {
                let mut a = disk(self, v as u32);
                let mut b = disk(o, v as u32);
                a.sort_unstable();
                b.sort_unstable();
                if a != b {
                    return false;
                }
            }
        }
        true
    }
}

fn chunk_ranges(n: usize) -> impl Iterator<Item = (usize, std::ops::Range<usize>)> {
    let c = super::cow::CHUNK;
    (0..n.div_ceil(c)).map(move |i| (i, i * c..((i + 1) * c).min(n)))
}

fn cols_eq_ptr<T: Clone>(a: &super::cow::CowVec<T>, b: &super::cow::CowVec<T>) -> bool {
    a.len() == b.len() && (0..a.n_chunks()).all(|c| a.same_chunk(b, c))
}

/// The same slots live, under the same generations (chunks both share
/// skipped).
fn live_gens_eq(a: &super::keys::Slots, b: &super::keys::Slots) -> bool {
    let (sa, sb) = (&a.state, &b.state);
    let n = sa.len().max(sb.len());
    let at = |s: &super::cow::CowVec<u32>, i: usize| if i < s.len() { *s.get(i) } else { 0 };
    for (c, range) in chunk_ranges(n) {
        if sa.same_chunk(sb, c) && c < sa.n_chunks() {
            continue;
        }
        for i in range {
            let (x, y) = (at(sa, i), at(sb, i));
            if (x & 1 == 1 || y & 1 == 1) && x != y {
                return false;
            }
        }
    }
    true
}

fn cols_eq_live<T: Clone + PartialEq>(
    a: &super::cow::CowVec<T>,
    b: &super::cow::CowVec<T>,
    live: impl Fn(usize) -> bool,
) -> bool {
    if a.len() != b.len() {
        // Columns grow with slots; slots past the shorter one are dead
        // in both (slot counts were compared already).
        let n = a.len().min(b.len());
        return (0..n).all(|i| !live(i) || a.get(i) == b.get(i));
    }
    for c in 0..a.n_chunks() {
        if a.same_chunk(b, c) {
            continue;
        }
        let lo = c << super::cow::CHUNK_BITS;
        let hi = (lo + super::cow::CHUNK).min(a.len());
        if !(lo..hi).all(|i| !live(i) || a.get(i) == b.get(i)) {
            return false;
        }
    }
    true
}

fn edge_faces(st: &State, e: u32) -> Vec<u32> {
    let inc = st.efaces.get(e as usize);
    if inc.len <= 2 {
        inc.a[..inc.len as usize].to_vec()
    } else {
        st.ef_spill.get(inc.a[0] as usize).clone()
    }
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

fn disk(st: &State, v: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut e = *st.vedge.get(v as usize);
    while e != NONE {
        out.push(e);
        let ends = st.ends.get(e as usize);
        let nx = st.enext.get(e as usize);
        e = if ends[0] == v { nx[0] } else { nx[1] };
    }
    out
}

/// Interned tag keys of two states of one lineage: one list a prefix of
/// the other (interning only appends; a restore may append a tombstone's
/// tag, restore_tail.rs). Live edges compared their indices already.
fn layers_agree(a: &[String], b: &[String]) -> bool {
    let n = a.len().min(b.len());
    a[..n] == b[..n]
}
