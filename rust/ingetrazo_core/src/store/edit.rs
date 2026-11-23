// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! Editing ops and their change records. One op so far, end to end:
//! `move_vertex` / `place_vertex` with the registry re-keyed exactly as
//! `Mesh._rekey` does, plus the weld lookups that observe it
//! (`_lookup` / `vertex_at`, `registers`).

use super::cow::{Cell, FxHashMap};
use super::keys::{Kind, NONE};
use super::Store;
use std::sync::Arc;

/// What an op did, by key, for whoever must follow it: history (an undo
/// record without a full snapshot for cheap ops), the renderer's
/// incremental rebake (faces around `moved`, plus created / removed) and
/// the snap index (re-insert moved vertices' edges). Keys, not slots: a
/// record outlives the slot's current tenant.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Change {
    pub revision: u64,
    pub created: [Vec<u128>; 3],
    pub removed: [Vec<u128>; 3],
    /// Vertices whose position changed (their edges and faces follow).
    pub moved: Vec<u128>,
    /// Edges / faces whose own data changed without a new key (flags,
    /// incidence, rings, attrs). Empty for a move.
    pub modified: [Vec<u128>; 3],
}

impl Store {
    fn bump(&mut self) -> u64 {
        self.revision += 1;
        self.revision
    }

    /// `QVector3D(position)`: float32 components while the store keeps
    /// Python's precision.
    #[inline]
    pub fn quantize(&self, p: [f64; 3]) -> [f64; 3] {
        if self.f32_parity {
            [p[0] as f32 as f64, p[1] as f32 as f64, p[2] as f32 as f64]
        } else {
            p
        }
    }

    /// `(a - b).lengthSquared() < tol2`, in float32 like Qt when in parity.
    #[inline]
    pub(super) fn within(&self, a: &[f64; 3], b: &[f64; 3]) -> bool {
        if self.f32_parity {
            let d = [
                a[0] as f32 - b[0] as f32,
                a[1] as f32 - b[1] as f32,
                a[2] as f32 - b[2] as f32,
            ];
            let l2 = if self.weld.fused {
                d[2].mul_add(d[2], d[0].mul_add(d[0], d[1] * d[1]))
            } else {
                d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
            };
            (l2 as f64) < self.weld.tol2
        } else {
            let d = glam::DVec3::from(*a) - glam::DVec3::from(*b);
            self.state.frame.definition_delta(d).length_squared() < self.weld.tol2
        }
    }

    /// `Mesh._probe_neighbours`: the 26 cells around `k`, in Python's
    /// dx, dy, dz order, the first registered vertex within weld distance.
    pub fn probe(&self, k: Cell, p: &[f64; 3]) -> Option<u32> {
        for dx in -1..=1i64 {
            for dy in -1..=1i64 {
                for dz in -1..=1i64 {
                    if dx == 0 && dy == 0 && dz == 0 {
                        continue;
                    }
                    if let Some(n) = self.state.registry.get(&(k.0 + dx, k.1 + dy, k.2 + dz)) {
                        if self.within(self.state.vpos.get(n as usize), p) {
                            return Some(n);
                        }
                    }
                }
            }
        }
        None
    }

    /// `Mesh._lookup` (`vertex_at`): the cell's holder, else a probe.
    pub fn lookup(&self, p: [f64; 3]) -> Option<u32> {
        let p = self.quantize(p);
        let k = self.cell(&p);
        self.state.registry.get(&k).or_else(|| self.probe(k, &p))
    }

    /// `Mesh.registers`: whether this store files `v` itself — holder of
    /// its cell, waiting in `displaced`, or found by the probe.
    pub fn registers(&self, v: u32) -> bool {
        if !self.state.slots[Kind::Vertex as usize].alive(v) {
            // Python leaves a vertex welded or collapsed away in the queue
            // it waited in, and `registers` answers from that queue.
            if (v as usize) >= self.state.vpos.len() {
                return false;
            }
            let k = self.cell(self.state.vpos.get(v as usize));
            return self.state.displaced.get(&k).is_some_and(|w| w.contains(&v));
        }
        let p = *self.state.vpos.get(v as usize);
        let k = self.cell(&p);
        let holder = self.state.registry.get(&k);
        if holder == Some(v) {
            return true;
        }
        if let Some(w) = self.state.displaced.get(&k) {
            if w.contains(&v) {
                return true;
            }
        }
        holder.is_none() && self.probe(k, &p) == Some(v)
    }

    /// `Mesh.move_vertex`: translate by `delta` (added in float32 in
    /// parity mode, like `QVector3D + QVector3D`).
    pub fn move_vertex(&mut self, v: u32, delta: [f64; 3]) -> Change {
        let p = *self.state.vpos.get(v as usize);
        let np = if self.f32_parity {
            [
                (p[0] as f32 + delta[0] as f32) as f64,
                (p[1] as f32 + delta[1] as f32) as f64,
                (p[2] as f32 + delta[2] as f32) as f64,
            ]
        } else {
            [p[0] + delta[0], p[1] + delta[1], p[2] + delta[2]]
        };
        self.set_position(v, p, np)
    }

    /// `Mesh.place_vertex`: put the vertex AT `position` (assigned, not
    /// reached through a delta — issue #163).
    pub fn place_vertex(&mut self, v: u32, position: [f64; 3]) -> Change {
        let p = *self.state.vpos.get(v as usize);
        let np = self.quantize(position);
        self.set_position(v, p, np)
    }

    fn set_position(&mut self, v: u32, p: [f64; 3], np: [f64; 3]) -> Change {
        // Python bumps the serial before it knows whether anything moved.
        let revision = self.bump();
        self.state.vpos.set(v as usize, np);
        let old = self.cell(&p);
        let new = self.cell(&np);
        if old != new {
            self.rekey(v, old, new);
        }
        Change { revision, moved: vec![self.key(Kind::Vertex, v)], ..Change::default() }
    }

    /// `Mesh._rekey`, line for line. Note the Python pops the WHOLE queue
    /// of the cell left behind and promotes the first waiter still
    /// standing there with edges; the other waiters are dropped from the
    /// queue (they stay listed and keep their keys).
    fn rekey(&mut self, v: u32, old: Cell, new: Cell) {
        let st = &mut self.state;
        if st.registry.get(&old) == Some(v) {
            st.registry.remove(&old);
            let waiting =
                if st.displaced.contains_key(&old) { Arc::make_mut(&mut st.displaced).remove(&old) } else { None };
            if let Some(waiting) = waiting {
                for w in waiting {
                    if w != v && *self.state.vedge.get(w as usize) != NONE {
                        let wp = *self.state.vpos.get(w as usize);
                        if self.cell(&wp) == old {
                            self.state.registry.insert(old, w);
                            break;
                        }
                    }
                }
            }
        } else if st.displaced.contains_key(&old) {
            let d: &mut FxHashMap<Cell, Vec<u32>> = Arc::make_mut(&mut st.displaced);
            let q = d.get_mut(&old).unwrap();
            q.retain(|&w| w != v);
            if q.is_empty() {
                d.remove(&old);
            }
        }
        let st = &mut self.state;
        match st.registry.get(&new) {
            None => {
                st.registry.insert(new, v);
            }
            Some(h) if h != v => {
                Arc::make_mut(&mut st.displaced).entry(new).or_default().push(v);
            }
            Some(_) => {}
        }
    }
}
