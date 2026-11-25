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
use super::{Store, FLAT, HIDDEN, SOFT};
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

    /// `Edge.soft` / `Edge.hidden` / `Edge.flat` (the other format-3 bits
    /// are not kept).
    pub fn set_edge_flags(&mut self, e: u32, flags: u8) -> u64 {
        let f = flags & (SOFT | HIDDEN | FLAT);
        if *self.state.eflags.get(e as usize) != f {
            self.state.eflags.set(e as usize, f);
        }
        self.bump_rev()
    }

    /// `Edge.curve` (−1: none).
    pub fn set_edge_curve(&mut self, e: u32, curve: i128) -> u64 {
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

    /// The faces `core.topology.fold_nonplanar_faces` folds: more than 3
    /// corners over all rings and not `is_planar` at `tolerance`, in list
    /// order. `is_planar`'s double arithmetic, step for step (plane through
    /// the first corner, the farthest one and the one spanning the widest
    /// triangle), so the verdicts are Python's bit for bit.
    pub fn nonplanar_faces(&self, tolerance: f64) -> Vec<u32> {
        let st = &self.state;
        let mut out = Vec::new();
        let mut pts: Vec<[f64; 3]> = Vec::new();
        for f in self.live_slots(Kind::Face) {
            pts.clear();
            for ring in self.face_rings(f) {
                pts.extend(ring.iter().map(|&v| *st.vpos.get(v as usize)));
            }
            if pts.len() <= 3 {
                continue;
            }
            if !is_planar(&pts, tolerance) {
                out.push(f);
            }
        }
        out
    }

    /// Live slots of a kind, in list order.
    pub fn live_slots(&self, kind: Kind) -> Vec<u32> {
        let st = &self.state;
        st.order[kind as usize].live(&st.slots[kind as usize])
    }
}

/// `core.topology.is_planar` for more than three points.
pub(crate) fn is_planar(pts: &[[f64; 3]], tolerance: f64) -> bool {
    let a = pts[0];
    let rel: Vec<[f64; 3]> = pts.iter().map(|p| [p[0] - a[0], p[1] - a[1], p[2] - a[2]]).collect();
    // Python's max(): the first of the largest.
    let mut u = rel[0];
    let mut um = u[0] * u[0] + u[1] * u[1] + u[2] * u[2];
    for r in &rel[1..] {
        let m = r[0] * r[0] + r[1] * r[1] + r[2] * r[2];
        if m > um {
            um = m;
            u = *r;
        }
    }
    let (mut nx, mut ny, mut nz, mut best) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for v in &rel {
        let cx = u[1] * v[2] - u[2] * v[1];
        let cy = u[2] * v[0] - u[0] * v[2];
        let cz = u[0] * v[1] - u[1] * v[0];
        let size = cx * cx + cy * cy + cz * cz;
        if size > best {
            best = size;
            nx = cx;
            ny = cy;
            nz = cz;
        }
    }
    let length = best.sqrt();
    if length <= 1e-6 && length <= 1e-5 * um.sqrt() {
        return false;
    }
    let (nx, ny, nz) = (nx / length, ny / length, nz / length);
    rel.iter().all(|r| (nx * r[0] + ny * r[1] + nz * r[2]).abs() <= tolerance)
}
