// SPDX-License-Identifier: GPL-3.0-or-later
//! Divide and join the selected edges, preserving directed walks and edge flags.
use super::{
    drawing::{point, vector},
    edit::Change,
    keys::Kind,
    topo::OpError,
    Store,
};
use crate::store::geometry::{length, normalized, py_sum, Hypot};
use std::collections::{HashMap, HashSet};

impl Store {
    fn drawing_flags(&self, e: u32) -> (u8, i128, u32) {
        let i = e as usize;
        (
            *self.state.eflags.get(i),
            *self.state.ecurve.get(i),
            *self.state.elayer.get(i),
        )
    }
    fn stamp(&mut self, e: u32, (flags, curve, layer): (u8, i128, u32), ch: &mut Change) {
        let i = e as usize;
        self.state.eflags.set(i, flags);
        self.state.ecurve.set(i, curve);
        self.state.elayer.set(i, layer);
        ch.modified[1].push(self.key(Kind::Edge, e));
    }
    fn curve_walk(&self, seed: u32, curve: i128) -> Vec<(u32, u32, u32)> {
        let [a, b] = *self.state.ends.get(seed as usize);
        if curve == -1 {
            return vec![(seed, a, b)];
        }
        let mut incident: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut order = vec![];
        for e in self.state.order[1].live(&self.state.slots[1]) {
            if *self.state.ecurve.get(e as usize) != curve {
                continue;
            }
            for &v in self.state.ends.get(e as usize) {
                let row = incident.entry(v).or_default();
                if row.is_empty() {
                    order.push(v)
                }
                row.push(e);
            }
        }
        let mut v = order
            .into_iter()
            .find(|v| incident[v].len() == 1)
            .unwrap_or(a);
        let mut walk = vec![];
        let mut visited = HashSet::new();
        while let Some(&e) = incident[&v].iter().find(|e| !visited.contains(*e)) {
            let w = self.other_end(e, v);
            walk.push((e, v, w));
            visited.insert(e);
            v = w;
        }
        walk
    }
    pub fn divide_drawing_edges(
        &mut self,
        edges: &[u32],
        n: usize,
        mut next: i64,
        h: Hypot,
        compensated: bool,
        ch: &mut Change,
    ) -> Result<(usize, i64), OpError> {
        let mut done = HashSet::new();
        let mut cuts = 0;
        if n < 2 {
            return Ok((0, next));
        }
        for &seed in edges {
            if done.contains(&seed) {
                continue;
            }
            let curve = *self.state.ecurve.get(seed as usize);
            let walk = self.curve_walk(seed, curve);
            let lengths: Vec<_> = walk
                .iter()
                .map(|&(e, a, b)| {
                    done.insert(e);
                    length(
                        vector(*self.state.vpos.get(b as usize))
                            .sub(vector(*self.state.vpos.get(a as usize))),
                        h,
                    ) as f64
                })
                .collect();
            let total = py_sum(lengths.iter().copied(), compensated);
            if total < 1e-9 {
                continue;
            }
            let step = total / n as f64;
            let mut walked = 0.;
            let mut pieces = std::collections::BTreeMap::<usize, Vec<u32>>::new();
            for ((e, mut from, b), len) in walk.into_iter().zip(lengths) {
                if len < 1e-12 {
                    continue;
                }
                let a = vector(*self.state.vpos.get(from as usize));
                let dir = vector(*self.state.vpos.get(b as usize)).sub(a);
                let flags = self.drawing_flags(e);
                let mut rest = e;
                let mut start = walked;
                // Python's float // (including 1.0 // 0.1).
                let mut piece = |lo: f64, hi: f64, e| {
                    if curve != -1 {
                        let mid = (lo + hi) * 0.5;
                        let k = ((mid - mid % step) / step).round() as usize;
                        pieces.entry(k.min(n - 1)).or_default().push(e);
                    }
                };
                for k in 1..n {
                    let at = step * k as f64;
                    if at <= walked + 1e-9 || at >= walked + len - 1e-9 {
                        continue;
                    }
                    let pos = point(a.add(dir.mul((at - walked) / len)));
                    let (mut first, mut second, _) = self.split_edge(rest, pos, ch)?;
                    if first == second {
                        break;
                    }
                    if !self.state.ends.get(first as usize).contains(&from) {
                        std::mem::swap(&mut first, &mut second)
                    }
                    self.stamp(first, flags, ch);
                    self.stamp(second, flags, ch);
                    piece(start, at, first);
                    from = self.other_end(first, from);
                    rest = second;
                    start = at;
                    cuts += 1;
                }
                piece(start, walked + len, rest);
                walked += len;
            }
            for es in pieces.values() {
                for &e in es {
                    self.state.ecurve.set(e as usize, next.into());
                    ch.modified[1].push(self.key(Kind::Edge, e));
                }
                next = next.checked_add(1).ok_or(OpError::Full)?;
            }
        }
        Ok((cuts, next))
    }

    fn join_geometry(&self, v: u32, a: u32, b: u32, h: Hypot, compensated: bool) -> (bool, f64) {
        let [p, a, b] = [v, a, b].map(|v| *self.state.vpos.get(v as usize));
        join_geometry([p,a,b],h,compensated)
    }
    pub fn join_drawing_edges(
        &mut self,
        mut live: Vec<u32>,
        h: Hypot,
        compensated: bool,
        ch: &mut Change,
    ) -> Result<(Vec<u128>, Vec<(u128, &'static str)>, usize), OpError> {
        let mut picked: HashSet<_> = live.iter().copied().collect();
        let mut seen = HashSet::new();
        let candidates: Vec<_> = live
            .iter()
            .flat_map(|&e| *self.state.ends.get(e as usize))
            .filter(|&v| seen.insert(v))
            .collect();
        let mut reasons = vec![];
        let mut joins = 0;
        let mut occ = None;
        for v in candidates {
            let es: Vec<_> = self.edges_at(v).filter(|e| picked.contains(e)).collect();
            if es.len() < 2 {
                continue;
            }
            let reason = if self.edges_at(v).count() > 2 {
                if !es.iter().enumerate().any(|(i, &e)| {
                    es[i + 1..].iter().any(|&f| {
                        let (straight, angle) = self.join_geometry(
                            v,
                            self.other_end(e, v),
                            self.other_end(f, v),
                            h,
                            compensated,
                        );
                        straight && angle < 90.
                    })
                }) {
                    continue;
                }
                "branch"
            } else {
                let (e, f) = (es[0], es[1]);
                let (a, b) = (self.other_end(e, v), self.other_end(f, v));
                let (straight, angle) = self.join_geometry(v, a, b, h, compensated);
                let flags = self.drawing_flags(e);
                if !straight {
                    if angle >= 1. {
                        continue;
                    }
                    "kink"
                } else if flags != self.drawing_flags(f) {
                    "flags"
                } else if flags.1 != -1 {
                    "curve"
                } else {
                    if !self.collapsible(v, h) || self.find_edge(a, b).is_some() {
                        continue;
                    }
                    let counts = occ.get_or_insert_with(|| self.corner_counts());
                    let joined = self.collapse_vertex(v, counts, ch)?.edge;
                    self.stamp(joined, flags, ch);
                    picked.remove(&e);
                    picked.remove(&f);
                    picked.insert(joined);
                    live.push(joined);
                    joins += 1;
                    "join"
                }
            };
            reasons.push((self.key(Kind::Vertex, v), reason));
        }
        Ok((
            live.into_iter()
                .filter(|e| picked.contains(e))
                .map(|e| self.key(Kind::Edge, e))
                .collect(),
            reasons,
            joins,
        ))
    }
}

pub(crate) fn join_geometry([p,a,b]:[[f64;3];3], h:Hypot, compensated:bool)->(bool,f64) {
        let ab: [f64; 3] = std::array::from_fn(|i| b[i] - a[i]);
        let ap: [f64; 3] = std::array::from_fn(|i| p[i] - a[i]);
        let cross: [f64; 3] = std::array::from_fn(|i| {
            ap[(i + 1) % 3] * ab[(i + 2) % 3] - ap[(i + 2) % 3] * ab[(i + 1) % 3]
        });
        let norm = |x: [f64; 3]| py_sum(x.into_iter().map(|c| c * c), compensated).sqrt();
        let off = norm(cross) / norm(ab);
        let size = p
            .into_iter()
            .chain(a)
            .chain(b)
            .map(f64::abs)
            .fold(0., f64::max);
        let d1 = vector(p).sub(vector(a));
        let d2 = vector(b).sub(vector(p));
        let angle = if length(d1, h) == 0. || length(d2, h) == 0. {
            180.
        } else {
            normalized(d1, h).dot(normalized(d2, h), h)
                .clamp(-1., 1.)
                .acos()
                .to_degrees()
        };
        (off <= (1e-6 + 1e-6 * size).min(1e-4), angle)
}
