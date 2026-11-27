// SPDX-License-Identifier: GPL-3.0-or-later
//! Reserved: exact geometry per element (docs/persistent-ids.md §11).
//!
//! A face may later carry an exact surface (plane, cylinder, cone, sphere,
//! torus, NURBS) and an edge an exact curve (line, arc, ellipse, NURBS), as
//! an index into the document's geometry pools (`ext["built.exact_geometry"]`,
//! docs/exact-geometry.md §2). Nothing makes them yet: the
//! columns stay `None` (every element faceted, no memory). They live in the
//! snapshot `State` beside the ids, so undo, redo and restores carry them,
//! and a successor that inherits an id inherits its shape too
//! (`succession.rs`).

use super::cow::CowVec;
use super::keys::{Kind, NONE};

/// Per-slot index into the document's shape table, `NONE` for faceted.
#[derive(Clone, Debug, Default)]
pub struct ExactColumns {
    pub face_surface: Option<CowVec<u32>>,
    pub edge_exact_curve: Option<CowVec<u32>>,
}

impl ExactColumns {
    fn column(&mut self, kind: Kind) -> Option<&mut CowVec<u32>> {
        match kind {
            Kind::Face => self.face_surface.as_mut(),
            Kind::Edge => self.edge_exact_curve.as_mut(),
            Kind::Vertex => None,
        }
    }

    /// A new or reused slot is faceted.
    pub fn alloc(&mut self, kind: Kind, slot: u32) {
        if let Some(c) = self.column(kind) {
            while c.len() <= slot as usize {
                c.push(NONE);
            }
            c.set(slot as usize, NONE);
        }
    }

    pub fn get(&self, kind: Kind, slot: u32) -> u32 {
        let c = match kind {
            Kind::Face => self.face_surface.as_ref(),
            Kind::Edge => self.edge_exact_curve.as_ref(),
            Kind::Vertex => None,
        };
        c.filter(|c| (slot as usize) < c.len()).map(|c| *c.get(slot as usize)).unwrap_or(NONE)
    }

    /// Give `slot` another element's shape (succession).
    pub fn inherit(&mut self, kind: Kind, slot: u32, shape: u32) {
        if shape == NONE && self.column(kind).is_none() {
            return;
        }
        if let Some(c) = self.column(kind) {
            while c.len() <= slot as usize {
                c.push(NONE);
            }
            c.set(slot as usize, shape);
        }
    }

    pub fn is_faceted(&self) -> bool {
        self.face_surface.is_none() && self.edge_exact_curve.is_none()
    }
}
