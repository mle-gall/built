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

use super::cow::CowVec;
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
#[derive(Clone, Debug, Default)]
pub struct Slots {
    pub state: CowVec<u32>,
    pub free: Arc<Vec<u32>>,
    pub live: usize,
}

/// The highest generation each slot has ever had, NOT part of a snapshot.
#[derive(Clone, Debug, Default)]
pub struct Hwm(pub Vec<u32>);

impl Slots {
    /// `n` live slots, generation 0: a freshly built store.
    pub fn fresh(n: usize) -> (Self, Hwm) {
        (
            Self { state: CowVec::from_vec(vec![1u32; n]), free: Arc::new(Vec::new()), live: n },
            Hwm(vec![0; n]),
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

    /// A slot for a new element and its generation. Free slots of this
    /// state first, then slots past this state's end that a later state
    /// (since undone) used, then a brand new slot; each reused slot gets a
    /// generation above any it had, so no key is ever handed out twice.
    pub fn alloc(&mut self, hwm: &mut Hwm) -> Option<(u32, u32)> {
        while !self.free.is_empty() {
            let s = Arc::make_mut(&mut self.free).pop().unwrap();
            let g = hwm.0[s as usize];
            if g < GEN_MAX {
                hwm.0[s as usize] = g + 1;
                self.state.set(s as usize, (g + 1) << 1 | 1);
                self.live += 1;
                return Some((s, g + 1));
            }
        }
        loop {
            let s = self.len();
            if s >= MAX_SLOTS as usize {
                return None;
            }
            if s < hwm.0.len() {
                let g = hwm.0[s];
                if g >= GEN_MAX {
                    self.state.push(g << 1); // retired: dead forever
                    continue;
                }
                hwm.0[s] = g + 1;
                self.state.push((g + 1) << 1 | 1);
                self.live += 1;
                return Some((s as u32, g + 1));
            }
            hwm.0.push(0);
            self.state.push(1);
            self.live += 1;
            return Some((s as u32, 0));
        }
    }

    /// Retire a live slot's element (its key stays dead forever).
    pub fn release(&mut self, slot: u32, hwm: &Hwm) {
        let st = self.state.get_mut(slot as usize);
        debug_assert!(*st & 1 == 1);
        *st &= !1;
        self.live -= 1;
        if hwm.0[slot as usize] < GEN_MAX {
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
}

impl Hwm {
    /// After adopting another lineage's state: never below what it issued.
    pub fn raise_to(&mut self, slots: &Slots) {
        if self.0.len() < slots.len() {
            self.0.resize(slots.len(), 0);
        }
        for (s, st) in slots.state.iter().enumerate() {
            let g = st >> 1;
            if self.0[s] < g {
                self.0[s] = g;
            }
        }
    }
}

/// The `mesh.vertices` / `mesh.edges` / `mesh.faces` list order.
///
/// Python semantics hang on it: "the first vertex in mesh order" survives
/// a weld, dedupe keeps the first face, the plan is index for index, and
/// a relinked element goes to the END of its list. A slot reused by a new
/// element must therefore not inherit its old place. `list` holds slots in
/// list order with stale entries left in place; `at[slot]` is the index of
/// the slot's current entry, so removal is O(1) (the entry turns stale) and
/// relinking appends a fresh entry.
#[derive(Clone, Debug, Default)]
pub struct Order {
    pub list: CowVec<u32>,
    pub at: CowVec<u32>,
    pub stale: usize,
}

impl Order {
    pub fn identity(n: usize) -> Self {
        let ids: Vec<u32> = (0..n as u32).collect();
        Self { list: CowVec::from_vec(ids.clone()), at: CowVec::from_vec(ids), stale: 0 }
    }

    pub fn append(&mut self, slot: u32) {
        while self.at.len() <= slot as usize {
            self.at.push(NONE);
        }
        let i = self.list.len() as u32;
        self.list.push(slot);
        if *self.at.get(slot as usize) != NONE {
            self.stale += 1; // its old entry turns stale
        }
        self.at.set(slot as usize, i);
    }

    pub fn remove(&mut self, slot: u32) {
        if *self.at.get(slot as usize) != NONE {
            self.at.set(slot as usize, NONE);
            self.stale += 1;
        }
    }

    /// Live slots in list order.
    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.list
            .iter()
            .enumerate()
            .filter(move |(i, &s)| *self.at.get(s as usize) == *i as u32)
            .map(|(_, &s)| s)
    }

    /// Drop stale entries (rewrites `at`): worth it when they outnumber
    /// live ones; never needed for correctness.
    pub fn compact(&mut self) {
        let live: Vec<u32> = self.iter().collect();
        let mut at = vec![NONE; self.at.len()];
        for (i, &s) in live.iter().enumerate() {
            at[s as usize] = i as u32;
        }
        self.list = CowVec::from_vec(live);
        self.at = CowVec::from_vec(at);
        self.stale = 0;
    }

    /// slot → dense index (list position among live entries), NONE for
    /// slots not listed. The readback index space.
    pub fn dense_of_slot(&self) -> Vec<u32> {
        let mut out = vec![NONE; self.at.len()];
        for (i, s) in self.iter().enumerate() {
            out[s as usize] = i as u32;
        }
        out
    }

    pub fn is_identity(&self) -> bool {
        self.stale == 0 && self.list.iter().enumerate().all(|(i, &s)| s as usize == i)
    }
}
