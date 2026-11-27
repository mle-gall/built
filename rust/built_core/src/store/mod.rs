// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! `MeshStore`: the geometry of one mesh as columns instead of Python
//! objects.
//!
//! Same model as `core/mesh.py` — shared vertices, edges with a radial
//! (non-manifold, ordered, with multiplicity) face list, faces as an outer
//! ring plus holes — and the same semantics wherever Python code can
//! observe them: list order, weld cells and probes, the displaced queue,
//! snapshot contents. What changes is the storage: positions, edges and
//! face rings in `Arc`-chunked columns so a snapshot is O(chunks), and
//! elements known by `(slot, generation)` keys so nothing outside needs an
//! object to name one.
//!
//! Wired into the app by `core/store_mesh.py` (port step 1): deferred and
//! bulk-built meshes live here, snapshots are this store's; every op not
//! ported yet turns the mesh into Python objects first. Step 3 ports the
//! kernel onto this, op by op, with the Python suite as the oracle.

pub mod access;
pub mod bevel;
pub mod fillet;
pub(crate) mod intersection_edit;
pub(crate) mod plane_repair;
pub(crate) mod plane_regions;
pub mod build;
mod changes;
pub mod cow;
pub mod diff;
pub(crate) mod drawing;
pub(crate) mod command_facade;
pub(crate) mod commands;
pub(crate) mod fold;
pub(crate) mod landing;
pub(crate) mod push_band;
mod soften;
pub(crate) mod stitch_command;
mod push_rebuild;
mod push_region;
mod push_extrude;
mod push_shell;
mod push_through;
mod push_translate;
mod push_finish;
mod overlap;
pub(crate) mod curve_groups;
pub(crate) mod orphan_overlap;
pub use push_rebuild::{Fresh, Paint};
mod erase;
pub(crate) mod drawing_face;
pub(crate) use drawing_face::Outline;
pub(crate) mod drawing_plane;
pub(crate) mod drawing_edges;
pub mod edit;
pub(crate) mod edge_index;
pub mod face_index;
pub(crate) mod geometry;
pub(crate) mod frame;
pub mod explode;
pub mod keys;
pub mod ids;
pub(crate) mod ids_py;
pub mod succession;
pub mod exact;
pub(crate) mod source_arcs;
pub(crate) mod exact_bound;
pub(crate) mod exact_display;
pub(crate) mod native;
pub mod intersect;
pub mod offset;
pub mod pieces;
pub mod py;
pub mod read;
mod restore_tail;
mod revolution;
pub mod stitch;
pub mod sweep;
pub mod tally;
pub mod topo;
mod hole;
mod solid;
mod caps;
mod cap_outline;
pub(crate) mod planar;
pub(crate) mod planar_api;
pub(crate) mod arrangement;
mod orient_seed;
mod winding;
#[cfg(test)]
mod tests;

use cow::{Cell, CellMap, CowVec, FxHashMap};
use keys::{Hwm, Kind, Order, Slots, NONE};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Edge flag bits, as `.igz` format 3 stores them (`edge_flags`).
pub const SOFT: u8 = 1;
pub const HIDDEN: u8 = 2;
/// A soft edge the shading does not smooth across (`Edge.flat`: Soften
/// Edges with «Smooth normals» off). Without it a soft edge smooths.
pub const FLAT: u8 = 4;
/// Face flag bits (`Face.interior`; read and written from step 3 on).
#[allow(dead_code)]
pub const INTERIOR: u8 = 1;

/// An edge's incident faces, `edge.faces` in order with multiplicity.
/// Two inline (almost every edge); more spill into `ef_spill`, and then
/// `a[0]` is the spill index.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inc {
    pub len: u32,
    pub a: [u32; 2],
}

/// Where a face's rings live in the pools: `n_rings` sizes from
/// `ring_start` in `ring_sizes`, their corners from `corner_start` in
/// `corners` (outer ring first, then holes — `add_faces_welded`'s layout).
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FaceRings {
    pub corner_start: u32,
    pub ring_start: u32,
    pub n_rings: u32,
}

/// The weld constants of `core/mesh.py`, passed in from Python so the
/// float bits are Python's (`10.0 ** 4`, `(10.0 ** -4) ** 2`).
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
pub struct Weld {
    pub scale: f64,
    pub tol2: f64,
    /// Whether this Qt fuses `QVector3D::lengthSquared()` into
    /// `fma(z, z, fma(x, x, y * y))` (Apple clang on arm64:
    /// `core._native.qt_hypot() == "libc++-fma"`). The weld distance test
    /// is that call, so a probe at exactly the tolerance follows it.
    pub fused: bool,
}

impl Default for Weld {
    fn default() -> Self {
        Self { scale: 10_000.0, tol2: 0.0001 * 0.0001, fused: false }
    }
}

/// Everything a snapshot holds (`Mesh.capture_state`) — cloning it IS the
/// snapshot. Indices inside (`ends`, `corners`, incidence, disk links) are
/// slots, never keys: generations only matter at the API boundary.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
pub struct State {
    pub frame: frame::Frame,
    /// Key namespace: `key = ns << 40 | local`.
    pub ns: u64,
    pub slots: [Slots; 3],
    pub order: [Order; 3],
    // Vertices.
    pub vpos: CowVec<[f64; 3]>,
    /// Head of the vertex's disk cycle of edges (`Vertex.edges`).
    pub vedge: CowVec<u32>,
    // Edges.
    pub ends: CowVec<[u32; 2]>,
    /// Next edge around `ends[0]` / `ends[1]` (a self-loop uses `[0]`).
    pub enext: CowVec<[u32; 2]>,
    pub eflags: CowVec<u8>,
    /// Curve id, −1 for none (`Edge.curve`).
    pub ecurve: CowVec<i128>,
    /// Index into `layers`, NONE for none (`Edge.layer`).
    pub elayer: CowVec<u32>,
    pub efaces: CowVec<Inc>,
    pub ef_spill: CowVec<Vec<u32>>,
    // Faces.
    pub frings: CowVec<FaceRings>,
    /// Attribute-table index + 1 (0: no attrs). The table itself — Python
    /// dicts, immutable once interned — stays with the Python facade.
    pub fattr: CowVec<u32>,
    pub fflags: CowVec<u8>,
    pub corners: CowVec<u32>,
    pub ring_sizes: CowVec<u32>,
    /// Pool entries no face points at any more (rewritten rings).
    pub pool_garbage: usize,
    // Weld.
    pub registry: CellMap,
    /// `Mesh._displaced`: vertices waiting for a cell another one holds.
    pub displaced: Arc<FxHashMap<Cell, Vec<u32>>>,
    /// Cells a vertex holds besides its own: `_weld_into` hands the welded
    /// vertex's cell to the survivor (`registry[_key(dup.position)] =
    /// keep`), which across a cell border is another cell. Kept so the
    /// survivor's death takes those entries too (no ghost entries, design
    /// decision 4); entries are checked against the registry before use.
    pub extra: Arc<FxHashMap<u32, Vec<Cell>>>,
    /// The two curve runs made by a branch landing inside a segment.
    /// Runtime curve ids are remapped by file readers.
    #[serde(skip)]
    pub(crate) split_runs: Arc<FxHashMap<u128, curve_groups::CurveJunction>>,
    /// Edge tag keys (`Edge.layer` strings), interned per store.
    pub layers: Arc<Vec<String>>,
    /// Persistent element ids per kind (`ids.rs`). Kept out of the
    /// serialized state (its layout predates them: recorded pickles read
    /// on); the mesh pickle carries them after it.
    #[serde(skip)]
    pub ids: [ids::IdColumn; 3],
    /// Reserved: exact surfaces and curves (`exact.rs`), `None` = faceted.
    #[serde(skip)]
    pub exact: exact::ExactColumns,
}

/// A frozen state: what history keeps for undo and what native readers
/// (pack, snap index) borrow off the GIL while the live store moves on.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub revision: u64,
    pub lineage: u64,
    pub state: State,
}

static LINEAGE: AtomicU64 = AtomicU64::new(1);

/// The store: the state plus what must survive restores.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct Store {
    pub state: State,
    /// Highest generation per slot ever issued, per kind (never restored).
    pub hwm: [Hwm; 3],
    /// Bumped by every mutation and every restore; never rewinds
    /// (`Mesh.revision` / `_mut_serial`).
    pub revision: u64,
    /// Which snapshots are this store's own (same-lineage restore keeps
    /// keys; a foreign one takes a fresh namespace).
    pub lineage: u64,
    pub weld: Weld,
    /// Diagnostic mode for legacy float32 vertex edits and weld lookup.
    pub f32_parity: bool,
    /// What the last succession did to exact bindings (not persisted).
    #[serde(skip)]
    pub exact_report: exact_bound::Report,
}

impl Store {
    pub fn empty(ns: u64, weld: Weld) -> Self {
        let mut state = State { ns, ..State::default() };
        state.registry = CellMap::with_expected(0);
        Self {
            state,
            hwm: Default::default(),
            revision: 0,
            lineage: LINEAGE.fetch_add(1, Ordering::Relaxed),
            weld,
            f32_parity: false,
            exact_report: Default::default(),
        }
    }

    pub fn n(&self, kind: Kind) -> usize {
        self.state.slots[kind as usize].live
    }

    #[inline]
    pub fn cell(&self, p: &[f64; 3]) -> Cell {
        let s = self.weld.scale;
        let p = self.state.frame.definition_point(*p);
        (
            (p[0] * s).round_ties_even() as i64,
            (p[1] * s).round_ties_even() as i64,
            (p[2] * s).round_ties_even() as i64,
        )
    }

    /// The live key of a slot.
    pub fn key(&self, kind: Kind, slot: u32) -> u128 {
        let gen = self.state.slots[kind as usize].gen(slot);
        keys::full_key(self.state.ns, keys::local_key(kind, gen, slot))
    }

    /// The slot a key names, if it names a live element of this store.
    pub fn slot(&self, kind: Kind, key: u128) -> Option<u32> {
        let (ns, k, gen, slot) = keys::split_key(key)?;
        let slots = &self.state.slots[kind as usize];
        if ns != self.state.ns || k != kind || !slots.alive(slot) || slots.gen(slot) != gen {
            return None;
        }
        Some(slot)
    }

    /// `capture_state`: O(chunks).
    pub fn snapshot(&self) -> Snapshot {
        Snapshot { revision: self.revision, lineage: self.lineage, state: self.state.clone() }
    }

    /// `restore_state`. Python clears `_displaced` on restore (it is not in
    /// the snapshot), so this does too. A snapshot of another store (the
    /// share-back of an edited instance: `proto.restore_state(local
    /// .capture_state())`) is adopted under `fresh_ns`: two live stores
    /// must never allocate in one namespace.
    pub fn restore(&mut self, snap: &Snapshot, fresh_ns: Option<u64>) -> Result<(), String> {
        let mut state = snap.state.in_frame(self.state.frame.clone(), Some(&self.state));
        if snap.lineage != self.lineage {
            let ns = fresh_ns.ok_or("a snapshot of another store needs a fresh namespace")?;
            state.ns = ns;
            // A fresh namespace has issued nothing: no floor to keep.
            for k in Kind::ALL {
                self.hwm[k as usize] = Hwm::new(state.slots[k as usize].len());
            }
        } else {
            for k in Kind::ALL {
                self.hwm[k as usize].fold(&self.state.slots[k as usize], &state.slots[k as usize]);
            }
            // Elements made after the snapshot stay tombstones a redo can
            // relink (restore_tail.rs).
            self.carry_tombstones(&mut state);
        }
        if !state.displaced.is_empty() {
            state.displaced = Arc::new(FxHashMap::default());
        }
        self.state = state;
        self.revision += 1;
        Ok(())
    }

    /// `copy.deepcopy(mesh)`: a new owner sharing every chunk, its keys in
    /// a new namespace (2.1: copies are new elements).
    pub fn copy_as(&self, ns: u64) -> Store {
        let mut out = self.clone();
        out.state.ns = ns;
        out.lineage = LINEAGE.fetch_add(1, Ordering::Relaxed);
        out
    }

    /// Edges around a vertex (its disk cycle).
    pub fn edges_at(&self, v: u32) -> DiskIter<'_> {
        DiskIter { st: &self.state, v, e: *self.state.vedge.get(v as usize) }
    }

    pub fn degree(&self, v: u32) -> usize {
        self.edges_at(v).count()
    }

    /// `edge.faces` as face slots, in order.
    pub fn edge_faces(&self, e: u32) -> &[u32] {
        let inc = self.state.efaces.get(e as usize);
        if inc.len <= 2 {
            &inc.a[..inc.len as usize]
        } else {
            self.state.ef_spill.get(inc.a[0] as usize)
        }
    }

    /// Faces touching vertices (via their edges), each once, first seen
    /// first: what an incremental rebake of a move needs.
    pub fn faces_around(&self, vs: &[u32]) -> Vec<u32> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for &v in vs {
            for e in self.edges_at(v) {
                for &f in self.edge_faces(e) {
                    if seen.insert(f) {
                        out.push(f);
                    }
                }
            }
        }
        out
    }
}

impl State {
    /// `Store::edges_at` of this state.
    pub(crate) fn edges_at(&self, v: u32) -> DiskIter<'_> {
        DiskIter { st: self, v, e: *self.vedge.get(v as usize) }
    }

    /// An element's place in its list (`Order`): ascending in list order.
    pub(crate) fn list_rank(&self, kind: keys::Kind, slot: u32) -> u64 {
        match &self.order[kind as usize].explicit {
            None => u64::from(slot),
            Some(x) => u64::from(*x.at.get(slot as usize)),
        }
    }

    /// `Store::edge_faces` of this state.
    pub(crate) fn edge_faces(&self, e: u32) -> &[u32] {
        let inc = self.efaces.get(e as usize);
        if inc.len <= 2 {
            &inc.a[..inc.len as usize]
        } else {
            self.ef_spill.get(inc.a[0] as usize)
        }
    }
}

pub struct DiskIter<'a> {
    st: &'a State,
    v: u32,
    e: u32,
}

impl Iterator for DiskIter<'_> {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        if self.e == NONE {
            return None;
        }
        let e = self.e;
        let ends = self.st.ends.get(e as usize);
        let nx = self.st.enext.get(e as usize);
        self.e = if ends[0] == self.v { nx[0] } else { nx[1] };
        Some(e)
    }
}

pub(crate) mod face_edit;
pub(crate) mod face_intersections;
pub(crate) mod piece_command;
