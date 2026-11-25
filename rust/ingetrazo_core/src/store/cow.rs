// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! Copy-on-write containers for the store: a chunked column and a sharded
//! cell map.
//!
//! Undo takes a snapshot of a whole mesh before every command
//! (`History._execute`) and the push/pull preview takes and restores one
//! per frame. In Python that is an O(n) walk building a dozen dicts. Here a
//! snapshot clones a vector of `Arc`s: a 1M-vertex column is 245 refcount
//! bumps, and only the chunks an edit then writes are copied (4096
//! elements each). The same holds for the weld registry, split into shards
//! so that moving one vertex copies one shard, not the table.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::Arc;

/// Elements per chunk. 4096 keeps a copied chunk small (≤ 96 KB for
/// positions) while the per-chunk `Arc` overhead stays below 0.5 %.
pub const CHUNK_BITS: u32 = 12;
pub const CHUNK: usize = 1 << CHUNK_BITS;
const MASK: usize = CHUNK - 1;

/// A growable column split into `Arc`-shared chunks.
///
/// `clone()` is the snapshot: it shares every chunk. A write goes through
/// `Arc::make_mut`, which copies the chunk only when a snapshot still holds
/// it, so the snapshot never sees later edits.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct CowVec<T: Clone> {
    chunks: Vec<Arc<Vec<T>>>,
    len: usize,
}

impl<T: Clone> Default for CowVec<T> {
    fn default() -> Self {
        Self { chunks: Vec::new(), len: 0 }
    }
}

impl<T: Clone> CowVec<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from a plain vector (one copy into chunks; the bulk build
    /// fills plain vectors first because pushing through `make_mut` would
    /// check the refcount per element).
    pub fn from_vec(v: Vec<T>) -> Self {
        let len = v.len();
        // Exact capacity: most meshes of a big model are a few dozen
        // elements (16,733 of the ride's 18,482), so a chunk must not
        // round them up to 4096. A chunk grows when pushed to.
        let chunks = v.chunks(CHUNK).map(|c| Arc::new(c.to_vec())).collect();
        Self { chunks, len }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn get(&self, i: usize) -> &T {
        debug_assert!(i < self.len);
        &self.chunks[i >> CHUNK_BITS][i & MASK]
    }

    /// Mutable access; copies the chunk if a snapshot shares it.
    #[inline]
    pub fn get_mut(&mut self, i: usize) -> &mut T {
        debug_assert!(i < self.len);
        let chunk = Arc::make_mut(&mut self.chunks[i >> CHUNK_BITS]);
        &mut chunk[i & MASK]
    }

    #[inline]
    pub fn set(&mut self, i: usize, v: T) {
        *self.get_mut(i) = v;
    }

    pub fn push(&mut self, v: T) {
        if self.len & MASK == 0 {
            // The first chunk grows by doubling (a small mesh stays
            // small); later ones are a big mesh's: full size at once.
            let chunk = if self.chunks.is_empty() { vec![v] } else {
                let mut c = Vec::with_capacity(CHUNK);
                c.push(v);
                c
            };
            self.chunks.push(Arc::new(chunk));
        } else {
            let last = self.chunks.last_mut().expect("a partial chunk exists");
            Arc::make_mut(last).push(v);
        }
        self.len += 1;
    }

    /// Drop the last element (copies the last chunk if a snapshot shares
    /// it, frees it once empty).
    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let last = self.chunks.last_mut().expect("a chunk holds the last element");
        let v = Arc::make_mut(last).pop();
        if last.is_empty() {
            self.chunks.pop();
        }
        self.len -= 1;
        v
    }

    pub fn last(&self) -> Option<&T> {
        if self.len == 0 { None } else { Some(self.get(self.len - 1)) }
    }

    /// The contiguous runs, in order: how native consumers (pack, snap
    /// index) read a column without assembling one buffer.
    pub fn chunks(&self) -> impl Iterator<Item = &[T]> {
        self.chunks.iter().map(|c| c.as_slice())
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.chunks.iter().flat_map(|c| c.iter())
    }

    pub fn to_vec(&self) -> Vec<T> {
        let mut out = Vec::with_capacity(self.len);
        for c in self.chunks() {
            out.extend_from_slice(c);
        }
        out
    }

    /// Bytes this column has allocated (chunk capacities, not counting
    /// sharing with snapshots).
    pub fn heap_bytes(&self) -> usize {
        self.chunks
            .iter()
            // Arc allocation: two counts and the Vec header, then the data.
            .map(|c| c.capacity() * std::mem::size_of::<T>() + 2 * std::mem::size_of::<usize>()
                + std::mem::size_of::<Vec<T>>())
            .sum::<usize>()
            + self.chunks.capacity() * std::mem::size_of::<Arc<Vec<T>>>()
    }

    /// `(chunks shared with other, total chunks)`: how much of a snapshot
    /// is still the same memory.
    pub fn shared_with(&self, other: &Self) -> (usize, usize) {
        let shared = self
            .chunks
            .iter()
            .zip(other.chunks.iter())
            .filter(|(a, b)| Arc::ptr_eq(a, b))
            .count();
        (shared, self.chunks.len())
    }

    /// Whether chunk `c` of both columns is the same allocation (the cheap
    /// first test of snapshot equality).
    pub fn same_chunk(&self, other: &Self, c: usize) -> bool {
        match (self.chunks.get(c), other.chunks.get(c)) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    pub fn n_chunks(&self) -> usize {
        self.chunks.len()
    }
}

/// FxHash (rustc's): the weld registry is hit once per point on load, and
/// SipHash cost 2–3× more there for no benefit (keys are not adversarial).
#[derive(Default, Clone, Copy)]
pub struct Fx(u64);

const FX_K: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl Hasher for Fx {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(5) ^ b as u64).wrapping_mul(FX_K);
        }
    }
    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.0 = (self.0.rotate_left(5) ^ i as u64).wrapping_mul(FX_K);
    }
    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(FX_K);
    }
    #[inline]
    fn write_i64(&mut self, i: i64) {
        self.write_u64(i as u64);
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
}

pub type FxBuild = BuildHasherDefault<Fx>;
pub type FxHashMap<K, V> = HashMap<K, V, FxBuild>;

/// A weld cell: the position × 10⁴, rounded half to even (`Mesh._key`).
pub type Cell = (i64, i64, i64);

/// Cell → vertex slot, in `Arc`-shared shards.
///
/// The registry is mutable state that cannot be derived from positions
/// (the first vertex into a cell holds it; a later one waits), so undo has
/// to snapshot it. Sharding makes that snapshot O(shards) and a later
/// single-vertex move copy one shard (≈ n / shards entries).
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct CellMap {
    shards: Vec<Arc<FxHashMap<Cell, u32>>>,
    mask: u64,
    len: usize,
}

/// Target entries per shard: a shard copy stays ~100 KB.
const SHARD_TARGET: usize = 4096;

impl Default for CellMap {
    fn default() -> Self {
        Self::with_expected(0)
    }
}

impl CellMap {
    /// A map sized for `n` cells: a power-of-two shard count in [1, 4096]
    /// (one shard for a small mesh: its whole map is a shard's size).
    pub fn with_expected(n: usize) -> Self {
        let want = (n / SHARD_TARGET).next_power_of_two().clamp(1, 4096);
        let per = n / want + 1;
        let shards = (0..want)
            .map(|_| Arc::new(FxHashMap::with_capacity_and_hasher(per, FxBuild::default())))
            .collect();
        Self { shards, mask: (want - 1) as u64, len: 0 }
    }

    #[inline]
    fn shard(&self, c: &Cell) -> usize {
        // A different mix from the shard's own hasher, so a shard's keys
        // still spread over its buckets.
        let h = (c.0 as u64)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (c.1 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
            ^ (c.2 as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
        ((h >> 32) & self.mask) as usize
    }

    #[inline]
    pub fn get(&self, c: &Cell) -> Option<u32> {
        self.shards[self.shard(c)].get(c).copied()
    }

    #[inline]
    pub fn contains(&self, c: &Cell) -> bool {
        self.shards[self.shard(c)].contains_key(c)
    }

    pub fn insert(&mut self, c: Cell, v: u32) -> Option<u32> {
        let s = self.shard(&c);
        let old = Arc::make_mut(&mut self.shards[s]).insert(c, v);
        if old.is_none() {
            self.len += 1;
            if self.len > 4 * SHARD_TARGET * self.shards.len() && self.shards.len() < 4096 {
                self.reshard();
            }
        }
        old
    }

    /// A mesh drawn up from nothing outgrew its shards: redistribute, so a
    /// snapshot's first write keeps copying ~SHARD_TARGET entries.
    fn reshard(&mut self) {
        let mut next = CellMap::with_expected(self.len);
        if next.shards.len() <= self.shards.len() {
            next = CellMap::with_expected(self.len * 8);
        }
        for (c, &v) in self.iter() {
            let s = next.shard(c);
            Arc::make_mut(&mut next.shards[s]).insert(*c, v);
        }
        next.len = self.len;
        *self = next;
    }

    pub fn remove(&mut self, c: &Cell) -> Option<u32> {
        let s = self.shard(c);
        // Look before make_mut: a miss must not copy a shared shard.
        if !self.shards[s].contains_key(c) {
            return None;
        }
        let old = Arc::make_mut(&mut self.shards[s]).remove(c);
        if old.is_some() {
            self.len -= 1;
        }
        old
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Cell, &u32)> {
        self.shards.iter().flat_map(|s| s.iter())
    }

    /// Allocated bytes: hashbrown stores (key, value) buckets plus one
    /// control byte each, at its capacity.
    pub fn heap_bytes(&self) -> usize {
        let bucket = std::mem::size_of::<(Cell, u32)>() + 1;
        self.shards.iter().map(|s| s.capacity() * bucket).sum::<usize>()
            + self.shards.len() * std::mem::size_of::<Arc<FxHashMap<Cell, u32>>>()
    }

    pub fn shared_with(&self, other: &Self) -> (usize, usize) {
        let shared = self
            .shards
            .iter()
            .zip(other.shards.iter())
            .filter(|(a, b)| Arc::ptr_eq(a, b))
            .count();
        (shared, self.shards.len())
    }

    /// Same cells to the same slots (shard pointers first, then content).
    pub fn same_as(&self, other: &Self) -> bool {
        if self.len != other.len {
            return false;
        }
        if self.shards.len() == other.shards.len()
            && self.shards.iter().zip(other.shards.iter()).all(|(a, b)| Arc::ptr_eq(a, b))
        {
            return true;
        }
        self.iter().all(|(c, v)| other.get(c) == Some(*v))
    }
}
