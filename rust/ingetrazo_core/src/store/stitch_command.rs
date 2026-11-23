// SPDX-License-Identifier: GPL-3.0-or-later
//! Ordered T-junction splits and seeded coplanar regions for a stitch.
use super::{
    commands::{cell, supplied_cell},
    geometry::{normalized, Hypot},
    keys::Kind,
    Store,
};
use std::collections::{HashMap, HashSet};
type Point = [f64; 3];

pub(super) fn junctions(
    points: &[Point],
    edges: &[[i64; 2]],
    bounds: Option<[Point; 2]>,
    tol: f64,
) -> Result<Vec<(usize, usize)>, &'static str> {
    let mut order: Vec<_> = (0..points.len()).collect();
    order.sort_unstable_by(|&a, &b| points[a][0].total_cmp(&points[b][0]));
    let mut hits = Vec::new();
    for (edge, &[a, b]) in edges.iter().enumerate() {
        let (&p, &q) = points
            .get(a as usize)
            .zip(points.get(b as usize))
            .ok_or("invalid stitch edge index")?;
        let lo: Point = std::array::from_fn(|i| p[i].min(q[i]));
        let hi: Point = std::array::from_fn(|i| p[i].max(q[i]));
        if bounds.is_some_and(|[low, high]| (0..3).any(|i| hi[i] < low[i] || lo[i] > high[i])) {
            continue;
        }
        let seg: Point = std::array::from_fn(|i| q[i] - p[i]);
        let dot = |a: Point, b: Point| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let length2 = dot(seg, seg);
        if length2 <= tol * tol {
            continue;
        }
        let margin = tol / length2.sqrt();
        let start = order.partition_point(|&v| points[v][0] < lo[0] - tol);
        let end = order.partition_point(|&v| points[v][0] <= hi[0] + tol);
        let mut best = None;
        for &v in &order[start..end] {
            if v == a as usize || v == b as usize || best.is_some_and(|index| v >= index) {
                continue;
            }
            let r = points[v];
            if (1..3).any(|i| r[i] < lo[i] - tol || r[i] > hi[i] + tol) {
                continue;
            }
            let rel: Point = std::array::from_fn(|i| r[i] - p[i]);
            let t = dot(rel, seg) / length2;
            if !(t > margin && t < 1. - margin) {
                continue;
            }
            let d: Point = std::array::from_fn(|i| rel[i] - seg[i] * t);
            if dot(d, d) < tol * tol {
                best = Some(v);
            }
        }
        if let Some(v) = best {
            hits.push((edge, v));
        }
    }
    Ok(hits)
}

impl Store {
    /// Ordered admission, including the discovery order used to populate the
    /// editor's face set. A failed dissolve resumes at the next starting face.
    pub(super) fn stitch_region(
        &self,
        seeds: Vec<Point>,
        fresh: Option<HashSet<u128>>,
        start: usize,
        h: Hypot,
    ) -> Option<(usize, Vec<u128>)> {
        let seeds: HashSet<_> = seeds.into_iter().map(supplied_cell).collect();
        let mut touches = HashMap::new();
        let mut seeded = |f| {
            *touches.entry(f).or_insert_with(|| {
                self.outer_ring(f)
                    .iter()
                    .any(|&v| seeds.contains(&cell(*self.state.vpos.get(v as usize))))
            })
        };
        let mut normals = HashMap::new();
        let mut normal = |f| *normals.entry(f).or_insert_with(|| self.face_normal(f, h));
        for (index, first) in self
            .live_slots(Kind::Face)
            .into_iter()
            .enumerate()
            .skip(start)
        {
            if !seeded(first) {
                continue;
            }
            let n = normalized(normal(first), h);
            let mut seen = HashSet::from([first]);
            let mut result = vec![self.key(Kind::Face, first)];
            let mut stack = vec![first];
            while let Some(f) = stack.pop() {
                for ring in self.face_rings(f) {
                    for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                        let Some(e) = self.find_edge(a, b) else {
                            continue;
                        };
                        let near = self.edge_faces(e);
                        if near
                            .iter()
                            .any(|&g| n.dot(normalized(normal(g), h), h).abs() < 0.999)
                        {
                            continue;
                        }
                        for &g in near {
                            if !seen.contains(&g) && seeded(g) && n.dot(normal(g), h).abs() > 0.999
                            {
                                seen.insert(g);
                                stack.push(g);
                                result.push(self.key(Kind::Face, g));
                            }
                        }
                    }
                }
            }
            if result.len() >= 2
                && fresh
                    .as_ref()
                    .is_none_or(|keys| result.iter().any(|k| keys.contains(k)))
            {
                return Some((index + 1, result));
            }
        }
        None
    }
}
