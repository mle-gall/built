// SPDX-License-Identifier: GPL-3.0-or-later
//! Position-addressed deletion for the History drawing commands.
use super::{edit::Change, keys::Kind, Store};
use std::collections::HashSet;

type Point = [f64; 3];
type Cell = [u64; 3];

/// Bits of Python's round(x, 4), canonicalising signed zero. When the
/// multiplication itself can change a rounding decision, round the original
/// binary64 value directly to four decimal places.
fn cell(point: Point) -> Cell {
    point.map(|x| {
        let scaled = x * 10000.;
        let rounded = if scaled.abs() >= (1u64 << 52) as f64 || scaled.fract().abs() == 0.5 {
            format!("{x:.4}").parse::<f64>().unwrap()
        } else {
            scaled.round_ties_even() / 10000.
        };
        if rounded == 0. {
            0
        } else {
            rounded.to_bits()
        }
    })
}

fn segment(a: Cell, b: Cell) -> [Cell; 2] {
    if a < b {
        [a, b]
    } else {
        [b, a]
    }
}

impl Store {
    fn command_remove_face(&mut self, face: u32) {
        let mut change = Change::default();
        self.remove_face(face, &mut change);
        self.finish(change);
    }

    /// Outer loop matching ignores winding, repeated corners and holes.
    /// None requests the materialising facade for unusual incidence.
    pub fn delete_command_face(
        &mut self,
        points: &[Point],
        candidates: &[u32],
    ) -> Option<Option<u128>> {
        let target: HashSet<_> = points.iter().copied().map(cell).collect();
        let found = candidates.iter().copied().find(|&f| {
            let corners: HashSet<_> = self
                .outer_ring(f)
                .into_iter()
                .map(|v| cell(*self.state.vpos.get(v as usize)))
                .collect();
            corners == target
        });
        match found {
            None => Some(None),
            Some(f) if self.remove_face_blocked(f) => None,
            Some(f) => {
                let key = self.key(Kind::Face, f);
                self.command_remove_face(f);
                Some(Some(key))
            }
        }
    }

    pub fn delete_command_edges(
        &mut self,
        endpoints: &[[Point; 2]],
        candidates: &[u32],
    ) -> Option<(Vec<u128>, Vec<u128>)> {
        let edges: Vec<_> = endpoints
            .iter()
            .filter_map(|&[a, b]| self.find_edge(self.lookup(a)?, self.lookup(b)?))
            .collect();
        let gone: HashSet<_> = endpoints
            .iter()
            .map(|&[a, b]| segment(cell(a), cell(b)))
            .collect();
        let mut faces = Vec::new();
        for &f in candidates {
            let corners: Vec<_> = self
                .outer_ring(f)
                .into_iter()
                .map(|v| cell(*self.state.vpos.get(v as usize)))
                .collect();
            if corners
                .iter()
                .zip(corners.iter().cycle().skip(1))
                .any(|(&a, &b)| gone.contains(&segment(a, b)))
            {
                if self.remove_face_blocked(f) {
                    return None;
                }
                faces.push(f);
            }
        }
        let face_keys = faces.iter().map(|&f| self.key(Kind::Face, f)).collect();
        let edge_keys = edges.iter().map(|&e| self.key(Kind::Edge, e)).collect();
        for f in faces {
            self.command_remove_face(f);
        }
        for e in edges {
            let mut change = Change::default();
            self.remove_edge(e, &mut change);
            self.finish(change);
        }
        Some((edge_keys, face_keys))
    }
}
