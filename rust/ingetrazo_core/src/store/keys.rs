// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! Element keys, slots and list order.
//!
//! Phase 2.1 gave every vertex, edge and face an opaque int key:
//! `namespace << 40 | counter`, one counter per mesh for all three kinds.
//! The store keeps that int shape and fills the low 40 bits with
//! `kind << 38 | generation << 26 | slot`, so
//!
//! - a key finds its element by indexing (no key → slot map),
//! - kinds never collide inside a mesh (selections mix edges and faces),
//! - keys of store-backed and Python meshes share one space during the
//!   port (distinct namespaces from the same `_KEY_NAMESPACES` counter).
//!
//! Keys are never reused, as in 2.1 ("discarded edits never free keys"):
//! a freed slot comes back with a higher generation, and the highest
//! generation each slot ever had (`Hwm`) is kept OUTSIDE the snapshots, so
//! restoring an old state cannot rewind allocation.

use super::cow::{CowVec, FxHashMap};
use std::sync::Arc;

pub const NONE: u32 = u32::MAX;

pub const SLOT_BITS: u32 = 26;
pub const GEN_BITS: u32 = 12;
pub const KIND_SHIFT: u32 = SLOT_BITS + GEN_BITS; // 38
pub const LOCAL_BITS: u32 = 40;
pub const MAX_SLOTS: u32 = 1 << SLOT_BITS;
/// A slot that reached this generation is retired, never reused.
pub const GEN_MAX: u32 = (1 << GEN_BITS) - 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Vertex = 0,
    Edge = 1,
    Face = 2,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Vertex, Kind::Edge, Kind::Face];

    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "vertex" | "v" => Some(Kind::Vertex),
            "edge" | "e" => Some(Kind::Edge),
            "face" | "f" => Some(Kind::Face),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Vertex => "vertex",
            Kind::Edge => "edge",
            Kind::Face => "face",
        }
    }
}

/// The 40 low bits of a key.
#[inline]
pub fn local_key(kind: Kind, gen: u32, slot: u32) -> u64 {
    ((kind as u64) << KIND_SHIFT) | ((gen as u64) << SLOT_BITS) | slot as u64
}

/// The Python int: `namespace << 40 | local`. `u128` so the namespace
/// counter never has to be bounded.
#[inline]
pub fn full_key(ns: u64, local: u64) -> u128 {
    ((ns as u128) << LOCAL_BITS) | local as u128
}

/// `(namespace, kind, generation, slot)`, or None for bits no store makes.
pub fn split_key(key: u128) -> Option<(u64, Kind, u32, u32)> {
    let ns = (key >> LOCAL_BITS) as u64;
    let local = (key & ((1u128 << LOCAL_BITS) - 1)) as u64;
    let kind = match local >> KIND_SHIFT {
        0 => Kind::Vertex,
        1 => Kind::Edge,
        2 => Kind::Face,
        _ => return None,
    };
    let gen = ((local >> SLOT_BITS) & GEN_MAX as u64) as u32;
    let slot = (local & (MAX_SLOTS as u64 - 1)) as u32;
    Some((ns, kind, gen, slot))
}

/// Per-kind slot table, part of the snapshot: generation and liveness per
/// slot (`gen << 1 | alive`) and the free slots.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
pub struct Slots {
    pub state: CowVec<u32>,
    pub free: Arc<Vec<u32>>,
    pub live: usize,
}

/// Generations issued in states since discarded by a restore — NOT part
/// of a snapshot. Sparse: only slots whose highest generation ever
/// exceeds what the current state shows (or that lie past its end) are
/// listed, so a store that was never restored carries nothing here.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
pub struct Hwm {
    pub over: FxHashMap<u32, u32>,
    /// Slots ever used (a state may be shorter after a restore).
    pub len: usize,
}

impl Hwm {
    pub fn new(len: usize) -> Self {
        Self { over: FxHashMap::default(), len }
    }

    /// Before `cur` is replaced by `next`: remember every generation of
    /// `cur` that `next` does not show. Chunks both share are skipped, so
    /// an undo costs O(chunks the undone edits wrote).
    pub fn fold(&mut self, cur: &Slots, next: &Slots) {
        self.len = self.len.max(cur.len());
        for c in 0..cur.state.n_chunks() {
            if cur.state.same_chunk(&next.state, c) {
                continue;
            }
            let lo = c << super::cow::CHUNK_BITS;
            let hi = (lo + super::cow::CHUNK).min(cur.len());
            for i in lo..hi {
                let g = cur.state.get(i) >> 1;
                let shown = if i < next.len() { Some(next.state.get(i) >> 1) } else { None };
                if shown.is_none_or(|n| n < g) {
                    let e = self.over.entry(i as u32).or_insert(g);
                    *e = (*e).max(g);
                }
            }
        }
    }

    /// The generation a reuse of `slot` must exceed.
    #[inline]
    fn floor(&self, slots: &Slots, slot: u32) -> u32 {
        let shown = if (slot as usize) < slots.len() { slots.gen(slot) } else { 0 };
        self.over.get(&slot).copied().unwrap_or(0).max(shown)
    }
}

impl Slots {
    /// `n` live slots, generation 0: a freshly built store.
    pub fn fresh(n: usize) -> (Self, Hwm) {
        (
            Self { state: CowVec::from_vec(vec![1u32; n]), free: Arc::new(Vec::new()), live: n },
            Hwm::new(n),
        )
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.state.len()
    }

    #[inline]
    pub fn alive(&self, slot: u32) -> bool {
        (slot as usize) < self.len() && self.state.get(slot as usize) & 1 == 1
    }

    #[inline]
    pub fn gen(&self, slot: u32) -> u32 {
        self.state.get(slot as usize) >> 1
    }

    /// A slot for a new element and its generation, and whether the slot
    /// is past every slot in use (then list order can stay implicit).
    /// Free slots of this state first, then slots past this state's end
    /// that a discarded state used, then a brand new slot; a reused slot
    /// gets a generation above any it ever had, so no key is handed out
    /// twice.
    pub fn alloc(&mut self, hwm: &mut Hwm) -> Option<(u32, u32, bool)> {
        while !self.free.is_empty() {
            let s = Arc::make_mut(&mut self.free).pop().unwrap();
            let g = hwm.floor(self, s);
            if g < GEN_MAX {
                hwm.over.remove(&s);
                self.state.set(s as usize, (g + 1) << 1 | 1);
                self.live += 1;
                return Some((s, g + 1, false));
            }
        }
        loop {
            let s = self.len();
            if s >= MAX_SLOTS as usize {
                return None;
            }
            if s < hwm.len {
                let g = hwm.floor(self, s as u32);
                hwm.over.remove(&(s as u32));
                if g >= GEN_MAX {
                    self.state.push(g << 1); // retired: dead forever
                    continue;
                }
                self.state.push((g + 1) << 1 | 1);
                self.live += 1;
                return Some((s as u32, g + 1, true));
            }
            hwm.len = s + 1;
            self.state.push(1);
            self.live += 1;
            return Some((s as u32, 0, true));
        }
    }

    /// Retire a live slot's element (its key stays dead forever).
    pub fn release(&mut self, slot: u32) {
        let st = self.state.get_mut(slot as usize);
        debug_assert!(*st & 1 == 1);
        *st &= !1;
        let g = *st >> 1;
        self.live -= 1;
        if g < GEN_MAX {
            Arc::make_mut(&mut self.free).push(slot);
        }
    }

    /// Bring a retired element back with its own key — the store's
    /// `relink_edge` / `relink_face` (redo re-attaching the very element).
    /// Fails when the slot was reused meanwhile.
    pub fn revive(&mut self, slot: u32, gen: u32) -> bool {
        if (slot as usize) >= self.len() || *self.state.get(slot as usize) != gen << 1 {
            return false;
        }
        let free = Arc::make_mut(&mut self.free);
        match free.iter().position(|&s| s == slot) {
            Some(i) => {
                free.swap_remove(i);
            }
            None => return false,
        }
        self.state.set(slot as usize, gen << 1 | 1);
        self.live += 1;
        true
    }

    /// Retire a live element and keep its slot as a TOMBSTONE: dead, its
    /// data left in the columns, never handed out again in this state.
    /// Editing ops remove this way (not [`Slots::release`]) because a
    /// removed element is often brought back by undo or redo with its own
    /// key (`relink_edge` / `relink_face` / `relink_vertices`), and a
    /// reused slot would make that impossible after LIFO undo: delete e,
    /// add e2 (reusing e's slot), undo the add, undo the delete.
    pub fn kill(&mut self, slot: u32) {
        let st = self.state.get_mut(slot as usize);
        debug_assert!(*st & 1 == 1);
        *st &= !1;
        self.live -= 1;
    }

    /// A tombstone back to life under its own generation (see `kill`).
    pub fn wake(&mut self, slot: u32) -> bool {
        if (slot as usize) >= self.len() {
            return false;
        }
        let st = self.state.get_mut(slot as usize);
        if *st & 1 == 1 {
            return false;
        }
        *st |= 1;
        self.live += 1;
        true
    }

    /// Whether no live slot lies above `slot` (then a relisted element
    /// at `slot` keeps an implicit list order ascending).
    pub fn none_live_above(&self, slot: u32) -> bool {
        ((slot as usize + 1)..self.len()).rev().all(|i| self.state.get(i) & 1 == 0)
    }

    /// Live slots, ascending.
    pub fn live_slots(&self) -> impl Iterator<Item = u32> + '_ {
        self.state.iter().enumerate().filter(|(_, &st)| st & 1 == 1).map(|(i, _)| i as u32)
    }
}

/// The `mesh.vertices` / `mesh.edges` / `mesh.faces` list order.
///
/// Python semantics hang on it: "the first vertex in mesh order" survives
/// a weld, dedupe keeps the first face, the plan is index for index, and
/// a relinked element goes to the END of its list. A slot reused by a new
/// element must therefore not inherit its old place.
///
/// While list order is ascending slot order (a fresh store, and any store
/// that only appended and removed) nothing is stored: the order is the
/// live slots. The first out-of-order listing (a freed slot reused, an
/// element relinked below the top) makes it explicit: `list` holds slots
/// in list order with stale entries left in place and `at[slot]` the index
/// of its current entry — O(1) removal, appending relinks.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
pub struct Order {
    pub explicit: Option<Explicit>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
pub struct Explicit {
    pub list: CowVec<u32>,
    pub at: CowVec<u32>,
    pub stale: usize,
}

impl Order {
    pub fn identity() -> Self {
        Self { explicit: None }
    }

    /// `slot` (now live in `slots`) goes to the end of the list.
    /// `on_top`: no live slot is above it.
    pub fn append(&mut self, slot: u32, slots: &Slots, on_top: bool) {
        if self.explicit.is_none() {
            if on_top {
                return;
            }
            let mut list: Vec<u32> = slots.live_slots().filter(|&s| s != slot).collect();
            let mut at = vec![NONE; slots.len()];
            for (i, &s) in list.iter().enumerate() {
                at[s as usize] = i as u32;
            }
            list.push(slot);
            at[slot as usize] = (list.len() - 1) as u32;
            self.explicit = Some(Explicit { list: CowVec::from_vec(list), at: CowVec::from_vec(at), stale: 0 });
            return;
        }
        let x = self.explicit.as_mut().unwrap();
        while x.at.len() <= slot as usize {
            x.at.push(NONE);
        }
        let i = x.list.len() as u32;
        x.list.push(slot);
        if *x.at.get(slot as usize) != NONE {
            x.stale += 1; // its old entry turns stale
        }
        x.at.set(slot as usize, i);
    }

    /// `slot` leaves the list (call before or after releasing it). The
    /// last entry is popped instead of left stale, with any stale entries
    /// it uncovers: undo removes what the command appended last, so LIFO
    /// edits keep the list free of stale entries (indexing stays O(1)).
    pub fn remove(&mut self, slot: u32) {
        if let Some(x) = self.explicit.as_mut() {
            if (slot as usize) < x.at.len() && *x.at.get(slot as usize) != NONE {
                let i = *x.at.get(slot as usize) as usize;
                x.at.set(slot as usize, NONE);
                if i + 1 == x.list.len() {
                    x.list.pop();
                    while let Some(&last) = x.list.last() {
                        let here = (x.list.len() - 1) as u32;
                        if (last as usize) < x.at.len() && *x.at.get(last as usize) == here {
                            break;
                        }
                        x.list.pop();
                        x.stale -= 1;
                    }
                } else {
                    x.stale += 1;
                }
            }
        }
    }

    /// The `index`-th live slot in list order without building the list
    /// when that is free: implicit order with no dead slot, or an explicit
    /// list with no stale entry.
    pub fn nth_fast(&self, slots: &Slots, index: usize) -> Option<Option<u32>> {
        match &self.explicit {
            None if slots.live == slots.len() => Some((index < slots.len()).then_some(index as u32)),
            Some(x) if x.stale == 0 => Some((index < x.list.len()).then(|| *x.list.get(index))),
            _ => None,
        }
    }

    /// The list index of a live `slot` when that is free (see `nth_fast`).
    pub fn index_fast(&self, slots: &Slots, slot: u32) -> Option<usize> {
        match &self.explicit {
            None if slots.live == slots.len() => Some(slot as usize),
            Some(x) if x.stale == 0 => Some(*x.at.get(slot as usize) as usize),
            _ => None,
        }
    }

    /// Live slots in list order.
    pub fn live(&self, slots: &Slots) -> Vec<u32> {
        match &self.explicit {
            None => slots.live_slots().collect(),
            Some(x) => x
                .list
                .iter()
                .enumerate()
                .filter(|(i, &s)| *x.at.get(s as usize) == *i as u32)
                .map(|(_, &s)| s)
                .collect(),
        }
    }

    /// Drop stale entries; back to implicit when the order is ascending.
    pub fn compact(&mut self, slots: &Slots) {
        if self.explicit.is_none() {
            return;
        }
        let live = self.live(slots);
        if live.windows(2).all(|w| w[0] < w[1]) {
            self.explicit = None;
            return;
        }
        let mut at = vec![NONE; slots.len()];
        for (i, &s) in live.iter().enumerate() {
            at[s as usize] = i as u32;
        }
        self.explicit = Some(Explicit { list: CowVec::from_vec(live), at: CowVec::from_vec(at), stale: 0 });
    }

    /// slot → dense index (list position among live entries), NONE for
    /// slots not listed. The readback index space.
    pub fn dense_of_slot(&self, slots: &Slots) -> Vec<u32> {
        let mut out = vec![NONE; slots.len()];
        for (i, s) in self.live(slots).into_iter().enumerate() {
            out[s as usize] = i as u32;
        }
        out
    }

    pub fn stale(&self) -> usize {
        self.explicit.as_ref().map_or(0, |x| x.stale)
    }

    pub fn heap_bytes(&self) -> usize {
        self.explicit.as_ref().map_or(0, |x| x.list.heap_bytes() + x.at.heap_bytes())
    }
}
