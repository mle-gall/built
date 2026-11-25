// SPDX-License-Identifier: GPL-3.0-or-later
//! Seed-relative winding on two-face edges, in boundary visitation order.
use super::{cow::FxHashMap, keys::Kind, Store};
use std::collections::VecDeque;

impl Store {
    pub fn seed_orientation(&self, seed: u32) -> (Vec<u128>, usize) {
        let mut seen = FxHashMap::default();
        seen.insert(seed, false);
        let mut queue = VecDeque::from([seed]);
        let mut flips = Vec::new();
        while let Some(face) = queue.pop_front() {
            for r in self.face_rings(face) {
                for (&start, &end) in r.iter().zip(r.iter().cycle().skip(1)) {
                    let (a, b) = if seen[&face] {
                        (end, start)
                    } else {
                        (start, end)
                    };
                    let Some(edge) = self.find_edge(a, b) else {
                        continue;
                    };
                    let [left, right] = self.edge_faces(edge) else {
                        continue;
                    };
                    let other = if *left == face { *right } else { *left };
                    if seen.contains_key(&other) {
                        continue;
                    }
                    let flip = self.face_rings(other).iter().any(|r| {
                        r.iter()
                            .zip(r.iter().cycle().skip(1))
                            .any(|(&x, &y)| x == a && y == b)
                    });
                    seen.insert(other, flip);
                    if flip {
                        flips.push(self.key(Kind::Face, other));
                    }
                    queue.push_back(other);
                }
            }
        }
        (flips, seen.len())
    }
}
