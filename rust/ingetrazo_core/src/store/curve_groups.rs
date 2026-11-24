// SPDX-License-Identifier: GPL-3.0-or-later
//! Split trimmed curves into connected chains in edge-list order.
use super::{
    drawing::vector,
    geometry::{length, normalized, Hypot},
    keys::Kind,
    Store,
};
use std::collections::{HashMap, HashSet};

impl Store {
    fn curve_break(&self, v: u32, group: &HashSet<u32>, h: Hypot) -> bool {
        let edges: Vec<_> = self.edges_at(v).collect();
        let mine: Vec<_> = edges
            .iter()
            .copied()
            .filter(|e| group.contains(e))
            .collect();
        let [a, b] = mine.as_slice() else { return true };
        if edges.len() == 2 {
            return false;
        }
        let p = vector(*self.state.vpos.get(v as usize));
        let direction = |e| vector(*self.state.vpos.get(self.other_end(e, v) as usize)) - p;
        let (a, b) = (direction(*a), direction(*b));
        let cross = a.cross(b, h);
        if length(cross, h) <= 1e-6 * length(a, h) * length(b, h) {
            return true;
        }
        let n = normalized(cross, h);
        edges.into_iter().filter(|e| !group.contains(e)).any(|e| {
            let u = direction(e);
            let len = length(u, h);
            len <= 1e-12 || u.dot(n, h).abs() < 0.05 * len
        })
    }

    pub(super) fn resplit_curves(&mut self, next: &mut i64, h: Hypot) {
        let mut groups: Vec<(i64, Vec<u32>)> = Vec::new();
        let mut index = HashMap::new();
        for e in self.live_slots(Kind::Edge) {
            let id = *self.state.ecurve.get(e as usize);
            if id >= 0 {
                let i = *index.entry(id).or_insert_with(|| {
                    groups.push((id, Vec::new()));
                    groups.len() - 1
                });
                groups[i].1.push(e);
            }
        }
        for (id, edges) in groups {
            let group: HashSet<_> = edges.iter().copied().collect();
            let mut unvisited = group.clone();
            let mut breaks = HashSet::new();
            for &e in &edges {
                for &v in self.state.ends.get(e as usize) {
                    if self.curve_break(v, &group, h) {
                        breaks.insert(v);
                    }
                }
            }
            let mut seeds: Vec<_> = edges
                .iter()
                .copied()
                .filter(|&e| {
                    self.state
                        .ends
                        .get(e as usize)
                        .iter()
                        .any(|v| breaks.contains(v))
                })
                .collect();
            seeds.extend(edges);
            let mut chains = Vec::new();
            for seed in seeds {
                if !unvisited.contains(&seed) {
                    continue;
                }
                let [a, b] = *self.state.ends.get(seed as usize);
                let mut v = if !breaks.contains(&a) && breaks.contains(&b) {
                    b
                } else {
                    a
                };
                let mut e = seed;
                let mut chain = Vec::new();
                while unvisited.remove(&e) {
                    chain.push(e);
                    v = self.other_end(e, v);
                    if breaks.contains(&v) {
                        break;
                    }
                    let Some(next) = self.edges_at(v).find(|k| unvisited.contains(k)) else {
                        break;
                    };
                    e = next;
                }
                chains.push(chain);
            }
            if chains.len() > 1 {
                for (i, chain) in chains.into_iter().enumerate() {
                    let tag = if i == 0 {
                        id
                    } else {
                        let tag = *next;
                        *next += 1;
                        tag
                    };
                    for e in chain {
                        self.set_edge_curve(e, tag);
                    }
                }
            }
        }
    }
}
