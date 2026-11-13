// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! Per-element reads and the writes that change no topology, for the
//! Python facade (`core/store_mesh.py`, port step 1).
//!
//! The facade hands out small view objects (`(mesh, key)`) instead of
//! Python `Vertex` / `Edge` / `Face` objects; their attributes read through
//! these. The writes here are the ones a tool makes on an element without
//! touching the incidence (`e.soft = True`, `e.layer = tag`, `f.interior`,
//! a face's attribute-table index): they stay in the store, so a mesh does
//! not have to become Python objects for a paint or a tag. Everything else
//! still materialises the mesh (see the design, §8 and §14).

use super::keys::{Kind, NONE};
use super::{Store, HIDDEN, SOFT};
use std::sync::Arc;

impl Store {
    /// The edge between two vertices (either direction), walking the
    /// smaller disk cycle, like `Mesh.find_edge`.
    pub fn find_edge(&self, a: u32, b: u32) -> Option<u32> {
        let (small, other) = if self.degree(a) <= self.degree(b) { (a, b) } else { (b, a) };
        self.edges_at(small).find(|&e| {
            let [p, q] = *self.state.ends.get(e as usize);
            (p == small && q == other) || (q == small && p == other)
        })
    }

    /// A face's rings as vertex slots (outer first, then holes).
    pub fn face_rings(&self, f: u32) -> Vec<Vec<u32>> {
        let st = &self.state;
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

    /// `Edge.soft` / `Edge.hidden` (the other format-3 bits are not kept).
    pub fn set_edge_flags(&mut self, e: u32, flags: u8) -> u64 {
        let f = flags & (SOFT | HIDDEN);
        if *self.state.eflags.get(e as usize) != f {
            self.state.eflags.set(e as usize, f);
        }
        self.bump_rev()
    }

    /// `Edge.curve` (−1: none).
    pub fn set_edge_curve(&mut self, e: u32, curve: i64) -> u64 {
        if *self.state.ecurve.get(e as usize) != curve {
            self.state.ecurve.set(e as usize, curve);
        }
        self.bump_rev()
    }

    /// `Edge.layer`: the tag key, interned in the store's name table.
    pub fn set_edge_layer(&mut self, e: u32, name: Option<&str>) -> u64 {
        let idx = match name {
            None => NONE,
            Some(n) => match self.state.layers.iter().position(|l| l == n) {
                Some(i) => i as u32,
                None => {
                    Arc::make_mut(&mut self.state.layers).push(n.to_string());
                    (self.state.layers.len() - 1) as u32
                }
            },
        };
        if *self.state.elayer.get(e as usize) != idx {
            self.state.elayer.set(e as usize, idx);
        }
        self.bump_rev()
    }

    /// A face's attribute-table index + 1 (0: none). The table is Python's.
    pub fn set_face_attr(&mut self, f: u32, attr: u32) -> u64 {
        if *self.state.fattr.get(f as usize) != attr {
            self.state.fattr.set(f as usize, attr);
        }
        self.bump_rev()
    }

    /// `Face.interior` and future face bits.
    pub fn set_face_flags(&mut self, f: u32, flags: u8) -> u64 {
        if *self.state.fflags.get(f as usize) != flags {
            self.state.fflags.set(f as usize, flags);
        }
        self.bump_rev()
    }

    fn bump_rev(&mut self) -> u64 {
        self.revision += 1;
        self.revision
    }

    /// Live slots of a kind, in list order.
    pub fn live_slots(&self, kind: Kind) -> Vec<u32> {
        let st = &self.state;
        st.order[kind as usize].live(&st.slots[kind as usize])
    }
}
