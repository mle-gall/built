// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.

use super::edit::Change;
use super::keys::Kind;
use super::Store;

impl Store {
    /// Restore a detached ring of live vertices without welding its corners.
    pub fn relink_hole(&mut self, face: u32, ring: Vec<u32>, change: &mut Change) {
        let mut rings = self.face_rings(face);
        for (i, &vertex) in ring.iter().enumerate() {
            if let Some(edge) = self.find_edge(vertex, ring[(i + 1) % ring.len()]) {
                if !self.edge_faces(edge).contains(&face) {
                    self.ef_push(edge, face);
                    change.modified[1].push(self.key(Kind::Edge, edge));
                }
            }
        }
        rings.push(ring);
        self.set_rings(face, &rings, true);
        change.modified[2].push(self.key(Kind::Face, face));
    }
}
