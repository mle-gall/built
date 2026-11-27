// SPDX-License-Identifier: GPL-3.0-or-later
//! Split trimmed curves into connected chains in edge-list order.
use super::{geometry::Hypot, keys::Kind, Store};
use glam::DVec3;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

/// A cylinder's side seam leaves the rim's plane and does not cut the rim.
pub(crate) fn planar_branch(a: DVec3, b: DVec3, others: &[DVec3]) -> bool {
    if others.is_empty() {
        return false;
    }
    let cross = a.cross(b);
    if cross.length() <= 1e-6 * a.length() * b.length() {
        return true;
    }
    let normal = cross.normalize_or_zero();
    others.iter().any(|u| {
        let length = u.length();
        length <= 1e-12 || u.dot(normal).abs() < 0.05 * length
    })
}

/// Edge indices in contour order. Seed at branch vertices before closed loops.
pub(crate) fn chains<V: Copy + Eq + Hash>(ends: &[[V; 2]], breaks: &HashSet<V>) -> Vec<Vec<usize>> {
    let mut adjacent: HashMap<V, Vec<usize>> = HashMap::new();
    for (i, [a, b]) in ends.iter().copied().enumerate() {
        adjacent.entry(a).or_default().push(i);
        adjacent.entry(b).or_default().push(i);
    }
    let open = (0..ends.len()).filter(|&i| ends[i].iter().any(|v| breaks.contains(v)));
    let mut visited = vec![false; ends.len()];
    let mut result = Vec::new();
    for seed in open.chain(0..ends.len()) {
        if visited[seed] {
            continue;
        }
        let [a, b] = ends[seed];
        let mut v = if !breaks.contains(&a) && breaks.contains(&b) {
            b
        } else {
            a
        };
        let mut e = seed;
        let mut chain = Vec::new();
        loop {
            visited[e] = true;
            chain.push(e);
            let [a, b] = ends[e];
            v = if a == v { b } else { a };
            if breaks.contains(&v) {
                break;
            }
            let Some(&next) = adjacent[&v].iter().find(|&&i| !visited[i]) else {
                break;
            };
            e = next;
        }
        result.push(chain);
    }
    result
}

impl Store {
    fn curve_break(&self, v: u32, group: &HashSet<u32>) -> bool {
        let (mine, other): (Vec<_>, Vec<_>) = self.edges_at(v).partition(|e| group.contains(e));
        let [a, b] = mine.as_slice() else { return true };
        if other.is_empty() {
            return false;
        }
        let p = DVec3::from_array(*self.state.vpos.get(v as usize));
        let direction =
            |e| DVec3::from_array(*self.state.vpos.get(self.other_end(e, v) as usize)) - p;
        planar_branch(
            direction(*a),
            direction(*b),
            &other.into_iter().map(direction).collect::<Vec<_>>(),
        )
    }

    pub(super) fn resplit_curves(&mut self, next: &mut i64, _h: Hypot) {
        let mut groups: Vec<(i128, Vec<u32>)> = Vec::new();
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
            let ends: Vec<_> = edges
                .iter()
                .map(|&e| *self.state.ends.get(e as usize))
                .collect();
            let group = edges.iter().copied().collect();
            let vertices: HashSet<_> = ends.iter().flatten().copied().collect();
            let breaks: HashSet<_> = vertices
                .into_iter()
                .filter(|&v| self.curve_break(v, &group))
                .collect();
            let chains = chains(&ends, &breaks);
            if chains.len() > 1 {
                // Remember only straight subdivisions. Original curve corners
                // remain vertices when their attached branch is erased.
                let splits: Vec<_> = breaks.iter().filter_map(|&v| {
                    let pair: Vec<_> = self.edges_at(v).filter(|e| group.contains(e)).collect();
                    let [a, b] = *pair.as_slice() else { return None };
                    let p = self.def_point(v);
                    let u = self.def_point(self.other_end(a, v)) - p;
                    let w = self.def_point(self.other_end(b, v)) - p;
                    (u.dot(w) < 0. && u.cross(w).length() < 1e-8*u.length()*w.length()).then_some((v,a,b))
                }).collect();
                for (i, chain) in chains.into_iter().enumerate() {
                    let tag = if i == 0 {
                        id
                    } else {
                        let tag = *next;
                        *next += 1;
                        i128::from(tag)
                    };
                    for e in chain {
                        self.set_edge_curve(edges[e], tag);
                    }
                }
                for (v, a, b) in splits {
                    let mut tags = [a,b].map(|e| *self.state.ecurve.get(e as usize));
                    tags.sort();
                    let key = self.key(Kind::Vertex, v);
                    std::sync::Arc::make_mut(&mut self.state.split_runs).insert(key, tags);
                }
            }
        }
    }
}
