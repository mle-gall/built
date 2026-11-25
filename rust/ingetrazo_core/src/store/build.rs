// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! A store from a format-3 block: what `Mesh.bulk_weld` +
//! `add_edges_welded` + `add_faces_welded` make on an EMPTY mesh
//! (`formats/igz.py::_build_geometry`), equal index for index to
//! `core/lazy_mesh.plan` — so a lazy mesh can become a store directly and
//! readers that know the plan read the same numbers.
//!
//! The order rules, from the Python:
//! - vertices: the weld's (`weld::weld`, shared with `lazy_mesh.weld`);
//!   `sequential` reorders them as `Mesh.vertex` point by point would;
//! - edges: input order of first occurrence of each unordered pair, in
//!   the first occurrence's direction; pairs welded shut are skipped;
//!   soft/hidden/flat accumulate (any), curve/layer: the last one given wins;
//! - then every ring segment, in global order, appends the boundary edges
//!   still missing (first occurrence's direction, a ring corner welded
//!   onto the next gives a self-loop edge, as Python does) and lists its
//!   face on the edge — multiplicity kept.

use super::cow::{CellMap, CowVec, FxBuild, FxHashMap};
use super::keys::{Order, Slots, NONE};
use super::{FaceRings, Inc, State, Store, Weld};
use crate::weld;
use std::sync::Arc;

/// Probe neighbouring cells in the source precision. At the tolerance
/// boundary a binary32 squared distance can select a different vertex.
pub enum Points<'a> {
    Single(&'a [[f32; 3]]),
    Double(&'a [[f64; 3]]),
}

impl Points<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Single(p) => p.len(),
            Self::Double(p) => p.len(),
        }
    }

    fn weld(&self, w: Weld) -> (Vec<[f64; 3]>, Vec<i64>, Vec<i64>) {
        match self {
            Self::Single(p) => {
                let (points, ids, origins) = weld::weld(p, w.scale, w.tol2);
                (points.into_iter().map(|p| p.map(f64::from)).collect(), ids, origins)
            }
            Self::Double(p) => weld::weld(p, w.scale, w.tol2),
        }
    }
}

/// Borrowed arrays of one block, as `formats/igz.py` holds them.
pub struct Block<'a> {
    pub pos: Points<'a>,
    pub edges: &'a [[i64; 2]],
    pub flags: Option<&'a [u8]>,
    pub curves: Option<&'a [i128]>,
    pub layers: Option<&'a [i64]>,
    pub layer_names: Vec<String>,
    pub rings: &'a [i64],
    pub sizes: &'a [i64],
    pub counts: &'a [i64],
    pub face_attr: Option<&'a [i64]>,
    pub sequential: bool,
}

impl Default for Block<'_> {
    fn default() -> Self {
        Self { pos: Points::Double(&[]), edges: &[], flags: None, curves: None, layers: None,
            layer_names: vec![], rings: &[], sizes: &[], counts: &[], face_attr: None, sequential: false }
    }
}

fn check(b: &Block, points: usize) -> Result<(), String> {
    let np = points as i64;
    let ne = b.edges.len();
    for (name, len) in [
        ("flags", b.flags.map(|a| a.len())),
        ("curves", b.curves.map(|a| a.len())),
        ("layers", b.layers.map(|a| a.len())),
    ] {
        if let Some(len) = len {
            if len != ne {
                return Err(format!("{name}: {len} entries for {ne} edges"));
            }
        }
    }
    if b.edges.iter().any(|e| e[0] < 0 || e[1] < 0 || e[0] >= np || e[1] >= np) {
        return Err("edge index out of range".into());
    }
    if let Some(l) = b.layers {
        let nl = b.layer_names.len() as i64;
        if l.iter().any(|&x| x >= nl) {
            return Err("edge layer index out of range".into());
        }
    }
    if b.rings.iter().any(|&c| c < 0 || c >= np) {
        return Err("ring corner out of range".into());
    }
    if b.sizes.iter().any(|&s| s < 1) {
        // Python would wrap a size-0 ring onto its neighbour's corners.
        return Err("ring of size < 1".into());
    }
    if b.sizes.iter().sum::<i64>() != b.rings.len() as i64 {
        return Err("ring sizes do not add up to the corners".into());
    }
    if b.counts.iter().any(|&c| c < 1) || b.counts.iter().sum::<i64>() != b.sizes.len() as i64 {
        return Err("ring counts do not add up to the rings".into());
    }
    if let Some(fa) = b.face_attr {
        if fa.len() != b.counts.len() {
            return Err("face_attr: one entry per face".into());
        }
    }
    if points >= super::keys::MAX_SLOTS as usize {
        return Err("more points than a store has vertex slots".into());
    }
    Ok(())
}

#[inline]
fn pair(a: u32, b: u32) -> u64 {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    (lo as u64) << 32 | hi as u64
}

/// Edge columns while building (plain vectors, chunked at the end).
#[derive(Default)]
struct Edges {
    ends: Vec<[u32; 2]>,
    flags: Vec<u8>,
    curve: Vec<i128>,
    layer: Vec<u32>,
    faces: Vec<Vec<u32>>,
}

impl Edges {
    fn push(&mut self, a: u32, b: u32) -> u32 {
        self.ends.push([a, b]);
        self.flags.push(0);
        self.curve.push(-1);
        self.layer.push(NONE);
        self.faces.push(Vec::new());
        (self.ends.len() - 1) as u32
    }
}

impl Store {
    pub fn from_block(ns: u64, b: &Block, weld_c: Weld) -> Result<Store, String> {
        Self::from_block_indices(ns, b, weld_c).map(|(store, _)| store)
    }

    pub(super) fn from_block_indices(ns: u64, b: &Block, weld_c: Weld) -> Result<(Store, Vec<i64>), String> {
        check(b, b.pos.len())?;
        let (mut vpos, mut ids, origin) = b.pos.weld(weld_c);
        if b.sequential && !vpos.is_empty() {
            // `lazy_mesh.weld(sequential=True)`: vertices in the order of
            // the point each was made at.
            let mut order: Vec<usize> = (0..vpos.len()).collect();
            order.sort_by_key(|&i| origin[i]); // stable
            let mut rank = vec![0i64; order.len()];
            for (r, &i) in order.iter().enumerate() {
                rank[i] = r as i64;
            }
            vpos = order.iter().map(|&i| vpos[i]).collect();
            for id in ids.iter_mut() {
                *id = rank[*id as usize];
            }
        }
        let store = Self::assemble(ns, b, weld_c, CowVec::from_vec(vpos), None, |i| ids[i as usize] as u32)?;
        Ok((store, ids))
    }

    /// A staged import already knows the vertex slots. Snapshots share the
    /// position chunks and registry while the next topology columns are built.
    pub(super) fn staged_block(&self, b: &Block) -> Result<Store, String> {
        check(b, self.state.vpos.len())?;
        Self::assemble(self.state.ns, b, self.weld, self.state.vpos.clone(),
                       Some(self.state.registry.clone()), |i| i as u32)
    }

    fn assemble(ns: u64, b: &Block, weld_c: Weld, vpos: CowVec<[f64;3]>,
                registry: Option<CellMap>, slot: impl Fn(i64) -> u32) -> Result<Store, String> {
        let nv = vpos.len();
        let mut emap: FxHashMap<u64, u32> =
            FxHashMap::with_capacity_and_hasher(b.edges.len() + b.rings.len() / 2, FxBuild::default());
        let mut ed = Edges::default();
        for (i, e) in b.edges.iter().enumerate() {
            let a = slot(e[0]);
            let c = slot(e[1]);
            if a == c {
                continue; // welded shut: add_edge's ValueError case
            }
            let ei = *emap.entry(pair(a, c)).or_insert_with(|| ed.push(a, c)) as usize;
            if let Some(f) = b.flags {
                ed.flags[ei] |= f[i] & (super::SOFT | super::HIDDEN | super::FLAT);
            }
            if let Some(cv) = b.curves {
                if cv[i] >= 0 {
                    ed.curve[ei] = cv[i];
                }
            }
            if let Some(lv) = b.layers {
                if lv[i] >= 0 {
                    ed.layer[ei] = lv[i] as u32;
                }
            }
        }
        // Faces: corners welded, then every ring segment in global order.
        let corners: Vec<u32> = b.rings.iter().map(|&c| slot(c)).collect();
        let nf = b.counts.len();
        let mut frings = Vec::with_capacity(nf);
        let mut fattr = Vec::with_capacity(nf);
        let (mut ri, mut ci) = (0usize, 0usize);
        for f in 0..nf {
            let nr = b.counts[f] as usize;
            frings.push(FaceRings { corner_start: ci as u32, ring_start: ri as u32, n_rings: nr as u32 });
            fattr.push(match b.face_attr {
                Some(fa) if fa[f] >= 0 => (fa[f] + 1) as u32,
                _ => 0,
            });
            for _ in 0..nr {
                let size = b.sizes[ri] as usize;
                let ring = &corners[ci..ci + size];
                for k in 0..size {
                    let a = ring[k];
                    let c = ring[(k + 1) % size];
                    let ei = *emap.entry(pair(a, c)).or_insert_with(|| ed.push(a, c)) as usize;
                    ed.faces[ei].push(f as u32);
                }
                ci += size;
                ri += 1;
            }
        }
        if ed.ends.len() >= super::keys::MAX_SLOTS as usize || nf >= super::keys::MAX_SLOTS as usize {
            return Err("more elements than a store has slots".into());
        }
        let ne = ed.ends.len();
        // Disk cycles: each edge prepended at both ends (once for a loop).
        let mut vedge = vec![NONE; nv];
        let mut enext = vec![[NONE, NONE]; ne];
        for (e, &[a, c]) in ed.ends.iter().enumerate() {
            enext[e][0] = vedge[a as usize];
            vedge[a as usize] = e as u32;
            if c != a {
                enext[e][1] = vedge[c as usize];
                vedge[c as usize] = e as u32;
            }
        }
        let mut spill: Vec<Vec<u32>> = Vec::new();
        let efaces: Vec<Inc> = ed
            .faces
            .into_iter()
            .map(|fs| {
                if fs.len() <= 2 {
                    let mut a = [NONE; 2];
                    a[..fs.len()].copy_from_slice(&fs);
                    Inc { len: fs.len() as u32, a }
                } else {
                    spill.push(fs);
                    Inc { len: spill.last().unwrap().len() as u32, a: [(spill.len() - 1) as u32, NONE] }
                }
            })
            .collect();
        let mut store = Store::empty(ns, weld_c);
        // One vertex per cell, at the cell of its own position: bulk_weld
        // registers exactly that (a probe-welded point never claims a cell).
        let registry = registry.unwrap_or_else(|| {
            let mut registry = CellMap::with_expected(nv);
            for (v, p) in vpos.iter().enumerate() {
                let prev = registry.insert(store.cell(p), v as u32);
                debug_assert!(prev.is_none(), "two vertices in one weld cell");
            }
            registry
        });
        let (sv, hv) = Slots::fresh(nv);
        let (se, he) = Slots::fresh(ne);
        let (sf, hf) = Slots::fresh(nf);
        store.hwm = [hv, he, hf];
        store.state = State {
            frame: Default::default(),
            ns,
            slots: [sv, se, sf],
            order: [Order::identity(), Order::identity(), Order::identity()],
            vpos,
            vedge: CowVec::from_vec(vedge),
            ends: CowVec::from_vec(ed.ends),
            enext: CowVec::from_vec(enext),
            eflags: CowVec::from_vec(ed.flags),
            ecurve: CowVec::from_vec(ed.curve),
            elayer: CowVec::from_vec(ed.layer),
            efaces: CowVec::from_vec(efaces),
            ef_spill: CowVec::from_vec(spill),
            frings: CowVec::from_vec(frings),
            fattr: CowVec::from_vec(fattr),
            fflags: CowVec::from_vec(vec![0u8; nf]),
            corners: CowVec::from_vec(corners),
            ring_sizes: CowVec::from_vec(b.sizes.iter().map(|&s| s as u32).collect()),
            pool_garbage: 0,
            registry,
            displaced: Arc::new(FxHashMap::default()),
            extra: Arc::new(FxHashMap::default()),
            layers: Arc::new(b.layer_names.clone()),
        };
        Ok(store)
    }
}
