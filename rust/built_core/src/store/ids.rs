// SPDX-License-Identifier: GPL-3.0-or-later
//! Persistent element ids per slot (docs/persistent-ids.md §3.3).
//!
//! One column per kind, part of the snapshot `State`: undo, redo and every
//! restore bring ids back exactly, and a tombstone keeps its id for a
//! relink. While the ids are `first + slot` (a block built in one piece,
//! a fresh mesh drawn without interruption) nothing is stored; the first
//! allocation that breaks the sequence makes the column dense, in the same
//! 4,096-entry `Arc` chunks as every other column.

use super::cow::CowVec;
use super::keys::Kind;
use super::Store;
use crate::element_ids;
use std::collections::HashMap;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub enum IdColumn {
    /// `id(slot) = first + slot`. `None`: no slot yet, the first
    /// allocation fixes `first`.
    Implicit { first: Option<u64> },
    Dense(CowVec<u64>),
}

impl Default for IdColumn {
    fn default() -> Self {
        IdColumn::Implicit { first: None }
    }
}

impl IdColumn {
    pub fn implicit(first: u64) -> Self {
        IdColumn::Implicit { first: Some(first) }
    }

    #[inline]
    pub fn get(&self, slot: u32) -> u64 {
        match self {
            IdColumn::Implicit { first } => first.unwrap_or(0).wrapping_add(slot as u64),
            IdColumn::Dense(v) => *v.get(slot as usize),
        }
    }

    /// The id of a new or reused `slot` of a kind with `len` slots before
    /// this allocation (`slot == len` appends).
    pub fn put(&mut self, slot: u32, len: usize, id: u64) {
        if let IdColumn::Implicit { first } = self {
            match *first {
                None if slot == 0 && len == 0 => {
                    *first = Some(id);
                    return;
                }
                Some(f) if slot as usize == len && f.wrapping_add(slot as u64) == id => return,
                _ => {}
            }
            let f = first.unwrap_or(0);
            let dense: Vec<u64> = (0..len as u64).map(|i| f.wrapping_add(i)).collect();
            *self = IdColumn::Dense(CowVec::from_vec(dense));
        }
        let IdColumn::Dense(v) = self else { unreachable!() };
        while v.len() < slot as usize {
            v.push(0);
        }
        if v.len() == slot as usize {
            v.push(id);
        } else {
            v.set(slot as usize, id);
        }
    }

    pub fn is_implicit(&self) -> bool {
        matches!(self, IdColumn::Implicit { .. })
    }

    pub fn heap_bytes(&self) -> usize {
        match self {
            IdColumn::Implicit { .. } => 0,
            IdColumn::Dense(v) => v.heap_bytes(),
        }
    }

    pub fn same_chunk(&self, other: &Self, c: usize) -> bool {
        match (self, other) {
            (IdColumn::Implicit { first: a }, IdColumn::Implicit { first: b }) => a == b,
            (IdColumn::Dense(a), IdColumn::Dense(b)) => c < a.n_chunks() && c < b.n_chunks() && a.same_chunk(b, c),
            _ => false,
        }
    }
}

/// Where a block's ids come from when a store is built from it.
#[derive(Clone, Copy, Debug)]
pub enum IdSource<'a> {
    /// New elements: one minted range per kind.
    Fresh,
    /// A file without ids: the migration origin numbers elements in the
    /// order the load builds them (docs/persistent-ids.md §5.3).
    Legacy,
    /// `first + source index` (migration: first = 0, origin 0).
    First(u64),
    /// One packed id per source element, already in process space.
    Array(&'a [u64]),
}

impl Default for IdSource<'_> {
    fn default() -> Self {
        IdSource::Fresh
    }
}

impl IdSource<'_> {
    fn get(&self, i: usize) -> u64 {
        match self {
            IdSource::Fresh | IdSource::Legacy => unreachable!(),
            IdSource::First(f) => f.wrapping_add(i as u64),
            IdSource::Array(a) => a[i],
        }
    }

    pub fn len_ok(&self, n: usize) -> bool {
        match self {
            IdSource::Array(a) => a.len() == n,
            _ => true,
        }
    }
}

/// The column of `n` built elements whose source indices are `src`
/// (`None`: an element the source does not list, such as a boundary edge
/// only a ring names). `n_src` is the source's element count.
pub fn build_column(kind: usize, source: IdSource, src: &[Option<usize>], n_src: usize) -> IdColumn {
    let source = match source {
        IdSource::Fresh => IdSource::First(element_ids::mint_range(kind, n_src as u64)),
        IdSource::Legacy => return IdColumn::implicit(0),
        s => s,
    };
    let identity = src.iter().enumerate().all(|(i, s)| *s == Some(i));
    if identity {
        if let IdSource::First(f) = source {
            return IdColumn::implicit(f);
        }
    }
    // Elements the source does not list: deterministic under the
    // migration origin (past every source index), minted otherwise.
    let migration = matches!(source, IdSource::First(f) if element_ids::origin_index(f) == 0);
    let base = if let IdSource::First(f) = source { f.wrapping_add(n_src as u64) } else { 0 };
    let mut extra = 0u64;
    let mut out: Vec<u64> = src
        .iter()
        .map(|s| match s {
            Some(i) => source.get(*i),
            None if migration => {
                extra += 1;
                base.wrapping_add(extra - 1)
            }
            None => element_ids::mint(kind),
        })
        .collect();
    if matches!(source, IdSource::Array(_)) {
        dedupe(kind, &mut out);
    }
    if out.iter().enumerate().all(|(i, &id)| out[0].wrapping_add(i as u64) == id) && !out.is_empty() {
        return IdColumn::implicit(out[0]);
    }
    IdColumn::Dense(CowVec::from_vec(out))
}

/// A damaged file can list one id twice: the later ones get fresh ids.
/// Returns how many were renewed.
pub fn dedupe(kind: usize, ids: &mut [u64]) -> usize {
    // Written in list order, ids usually ascend: then none repeats.
    if ids.windows(2).all(|w| w[0] < w[1]) {
        return 0;
    }
    let mut seen = std::collections::HashSet::with_capacity(ids.len());
    let mut renewed = 0;
    for id in ids.iter_mut() {
        if !seen.insert(*id) {
            *id = element_ids::mint(kind);
            seen.insert(*id);
            renewed += 1;
        }
    }
    renewed
}

impl Store {
    /// The persistent id of a slot (live or a tombstone).
    #[inline]
    pub fn pid(&self, kind: Kind, slot: u32) -> u64 {
        self.state.ids[kind as usize].get(slot)
    }

    /// Give `slot` another element's id (succession, keep-if-free).
    pub fn set_pid(&mut self, kind: Kind, slot: u32, id: u64) {
        let len = self.state.slots[kind as usize].len();
        let col = &mut self.state.ids[kind as usize];
        if col.get(slot) != id {
            col.put(slot, len, id);
        }
    }

    /// Live slot of a persistent id. A linear scan for a dense column:
    /// callers resolving many ids build [`Store::pid_index`].
    pub fn slot_of_pid(&self, kind: Kind, id: u64) -> Option<u32> {
        let k = kind as usize;
        let slots = &self.state.slots[k];
        match &self.state.ids[k] {
            IdColumn::Implicit { first: Some(f) } => {
                let s = id.wrapping_sub(*f);
                (s < slots.len() as u64 && slots.alive(s as u32)).then_some(s as u32)
            }
            IdColumn::Implicit { first: None } => None,
            IdColumn::Dense(v) => (0..v.len().min(slots.len()) as u32)
                .find(|&s| *v.get(s as usize) == id && slots.alive(s)),
        }
    }

    /// id → live slot for every live element of a kind.
    pub fn pid_index(&self, kind: Kind) -> HashMap<u64, u32> {
        let k = kind as usize;
        let slots = &self.state.slots[k];
        slots.live_slots().map(|s| (self.state.ids[k].get(s), s)).collect()
    }

    /// Number every element under the migration origin in list order: a
    /// file without ids read through a path that builds element by element
    /// (small JSON meshes). Deterministic for a given file.
    pub fn number_legacy(&mut self) {
        for kind in Kind::ALL {
            let k = kind as usize;
            let st = &mut self.state;
            let n = st.slots[k].len();
            let order = st.order[k].live(&st.slots[k]);
            if order.len() == n && order.iter().enumerate().all(|(i, &s)| s as usize == i) {
                st.ids[k] = IdColumn::implicit(0);
                continue;
            }
            let mut v = vec![u64::MAX; n];
            for (i, &s) in order.iter().enumerate() {
                v[s as usize] = i as u64;
            }
            let mut next = order.len() as u64;
            for x in v.iter_mut().filter(|x| **x == u64::MAX) {
                *x = next;
                next += 1;
            }
            st.ids[k] = IdColumn::Dense(CowVec::from_vec(v));
        }
    }

    /// Live ids of a kind in list order (the writer's element order).
    pub fn live_pids(&self, kind: Kind) -> Vec<u64> {
        let k = kind as usize;
        let st = &self.state;
        st.order[k].live(&st.slots[k]).into_iter().map(|s| st.ids[k].get(s)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implicit_until_the_sequence_breaks() {
        let mut c = IdColumn::default();
        c.put(0, 0, 100);
        c.put(1, 1, 101);
        assert!(c.is_implicit());
        assert_eq!(c.get(1), 101);
        c.put(2, 2, 7);
        assert!(!c.is_implicit());
        assert_eq!([c.get(0), c.get(1), c.get(2)], [100, 101, 7]);
        c.put(1, 3, 9);
        assert_eq!(c.get(1), 9);
    }

    #[test]
    fn build_column_identity_welds_and_extras() {
        let c = build_column(2, IdSource::First(0), &[Some(0), Some(1), Some(2)], 3);
        assert!(c.is_implicit());
        // A welded duplicate and a ring-only edge under the migration origin.
        let c = build_column(1, IdSource::First(0), &[Some(0), Some(2), None, None], 3);
        assert_eq!([c.get(0), c.get(1), c.get(2), c.get(3)], [0, 2, 3, 4]);
        let ids = [5u64, 9, 5];
        let c = build_column(2, IdSource::Array(&ids), &[Some(0), Some(1), Some(2)], 3);
        assert_eq!([c.get(0), c.get(1)], [5, 9]);
        assert_ne!(c.get(2), 5);
    }
}
