// SPDX-License-Identifier: GPL-3.0-or-later
//! Ordered T-junction splits and seeded coplanar regions for a stitch.
use super::{
    commands::supplied_cell,
    geometry::{normalized, Hypot},
    keys::Kind,
    Store,
};
use std::collections::{HashMap, HashSet};
type Point = [f64; 3];

pub(crate) fn junctions(
    points: &[Point],
    edges: &[[i64; 2]],
    bounds: Option<[Point; 2]>,
    tol: f64,
) -> Result<Vec<(usize, usize)>, &'static str> {
    if !tol.is_finite() || tol < 0. {
        return Err("stitch tolerance must be finite and nonnegative");
    }
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

/// List-order vertices and edges in a stitch operation's work area.
pub(crate) fn junction_scope(
    live_v: Vec<u32>,
    live_e: Vec<u32>,
    bounds: Option<[Point; 2]>,
    tol: f64,
    pos: impl Fn(u32) -> Point,
    ends: impl Fn(u32) -> [u32; 2],
) -> (Vec<u32>, Vec<u32>) {
    let Some([lo, hi]) = bounds else {
        return (live_v, live_e);
    };
    let edges: Vec<u32> = live_e
        .into_iter()
        .filter(|&e| {
            let [a, b] = ends(e).map(&pos);
            (0..3).all(|k| a[k].max(b[k]) >= lo[k] && a[k].min(b[k]) <= hi[k])
        })
        .collect();
    if edges.is_empty() {
        return (Vec::new(), edges);
    }
    let mut bx = [
        f64::INFINITY,
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for &e in &edges {
        for p in ends(e).map(&pos) {
            for k in 0..3 {
                bx[k] = bx[k].min(p[k]);
                bx[k + 3] = bx[k + 3].max(p[k]);
            }
        }
    }
    let g = tol * (1.0 + 1e-6) + 1e-12;
    let verts = live_v
        .into_iter()
        .filter(|&v| {
            let p = pos(v);
            (0..3).all(|k| p[k] >= bx[k] - g && p[k] <= bx[k + 3] + g)
        })
        .collect();
    (verts, edges)
}

/// What `Store::stitch_connect` did: the dedupe's `(dropped, kept)`
/// pairs (the facade carries attrs across) and the dirty bumps.
pub struct Connected {
    pub welded: bool,
    pub deduped: Vec<(u32, u32)>,
    pub splits: usize,
    pub collapses: usize,
    pub bumps: usize,
}

impl Store {
    /// Phases 0–2 of `history.run_stitch` as one op: weld coincident
    /// vertices, drop faces stacked on one cycle (`dedupe`), split the
    /// edges reaching the seed box at the vertices on them, round after
    /// round (`topology.sweep_tjunctions`, scoped as
    /// `push_scans.tjunction_arrays` scopes it), then collapse collinear
    /// valence-2 vertices. `seeds` are `topology._key` positions (none:
    /// the whole mesh). A step the store refuses is an error: the caller
    /// (`op_all`) puts the state back.
    pub fn stitch_connect(
        &mut self,
        seeds: &[Point],
        dedupe: bool,
        tol: f64,
        h: Hypot,
        ch: &mut super::edit::Change,
    ) -> Result<Connected, super::topo::OpError> {
        let (welded, mut bumps) = self.weld_coincident(ch)?;
        let deduped = if dedupe {
            let (pairs, b) = self.dedupe_faces(ch);
            bumps += b;
            pairs
        } else {
            Vec::new()
        };
        let bounds = (!seeds.is_empty()).then(|| {
            let mut lo = [f64::INFINITY; 3];
            let mut hi = [f64::NEG_INFINITY; 3];
            for p in seeds {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            [lo.map(|x| x - tol), hi.map(|x| x + tol)]
        });
        let splits = self.split_junctions(bounds, tol, ch)?;
        bumps += 3 * splits;
        let (collapses, b, refused) = self.collapse_collinear(h, ch);
        bumps += b;
        if refused.is_some() {
            return Err(super::topo::OpError::Fallback("a collinear vertex the store will not collapse"));
        }
        Ok(Connected { welded, deduped, splits, collapses, bumps })
    }

    pub(super) fn split_junctions(&mut self, bounds: Option<[Point; 2]>, tol: f64,
                                  ch: &mut super::edit::Change) -> Result<usize, super::topo::OpError> {
        let mut splits = 0;
        loop {
            let (verts, edges) = self.junction_scope(bounds, tol);
            if verts.is_empty() || edges.is_empty() {
                break;
            }
            let at: HashMap<u32, i64> = verts.iter().enumerate().map(|(i, &v)| (v, i as i64)).collect();
            let points: Vec<Point> = verts.iter().map(|&v| *self.state.vpos.get(v as usize)).collect();
            let ends: Vec<[i64; 2]> = edges
                .iter()
                .map(|&e| self.state.ends.get(e as usize).map(|v| at[&v]))
                .collect();
            let hits = junctions(&points, &ends, bounds, tol).map_err(super::topo::OpError::Fallback)?;
            if hits.is_empty() {
                break;
            }
            for (e, v) in hits {
                self.split_edge_at(edges[e], verts[v], ch)?;
                splits += 1;
            }
        }
        Ok(splits)
    }

    /// The vertices and edges a T-junction round can touch, in list order:
    /// every one without bounds; with them, the edges whose box meets the
    /// bounds and the vertices inside the union of their boxes grown by
    /// `tol` (`MeshScan::stitch_hits`).
    fn junction_scope(&self, bounds: Option<[Point; 2]>, tol: f64) -> (Vec<u32>, Vec<u32>) {
        let live_e = self.live_slots(Kind::Edge);
        let live_v = self.live_slots(Kind::Vertex);
        junction_scope(live_v, live_e, bounds, tol,
            |v| *self.state.vpos.get(v as usize), |e| *self.state.ends.get(e as usize))
    }
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
        let vertices = self.push_seed_vertices(&seeds);
        let seeded = |f| self.touches_push_vertex(f, &vertices);
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

#[cfg(test)]
mod tests {
    use super::junctions;

    #[test]
    fn split_uses_mesh_order_in_the_seed_box() {
        let points = [
            [0., 0., 0.],
            [4., 0., 0.],
            [3., 0., 0.],
            [1., 0., 0.],
            [0., 2., 0.],
            [4., 2., 0.],
            [2., 2., 0.],
        ];
        let edges = [[0, 1], [4, 5]];
        let all = junctions(&points, &edges, None, 1e-4).unwrap();
        assert_eq!(all, [(0, 2), (1, 6)]);
        let seed = [[0., 1.9, -0.1], [4., 2.1, 0.1]];
        assert_eq!(
            junctions(&points, &edges, Some(seed), 1e-4).unwrap(),
            [(1, 6)]
        );
        assert!(junctions(&points, &edges, None, -1.).is_err());
    }
}
