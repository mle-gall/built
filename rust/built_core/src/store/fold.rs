// SPDX-License-Identifier: GPL-3.0-or-later
//! Split a warped face, then reunite adjacent planar pieces of that face.
use super::{
    access::is_planar,
    commands::{cell, Cell},
    drawing::vector,
    edit::Change,
    geometry::{length, normalized, Hypot},
    keys::Kind,
    stitch::Plan,
    topo::OpError,
    Store, SOFT,
};
use std::collections::{HashMap, HashSet};

type Point = [f64; 3];
type Triangle = [Point; 3];

pub(super) struct Fold {
    pub triangles: Vec<Triangle>,
    soft: Vec<(Point, Point)>,
    tolerance: Option<f64>,
}

/// The two ordered triangle fans and the rim points whose edges may be softened.
pub(crate) fn strip_geometry(
    points: &[Point],
    moved: &HashSet<Cell>,
    normal: super::geometry::V3,
    h: Hypot,
) -> Option<(Vec<Triangle>, Point, Vec<Point>)> {
    let n = points.len();
    if n < 4 || moved.is_empty() {
        return None;
    }
    let flags: Vec<_> = points.iter().map(|&p| moved.contains(&cell(p))).collect();
    let mut starts = (0..n).filter(|&i| flags[i] && !flags[(i + n - 1) % n]);
    let first = starts.next()?;
    if starts.next().is_some() {
        return None;
    }
    let order: Vec<_> = (0..n).map(|i| points[(first + i) % n]).collect();
    let count = flags.into_iter().filter(|&v| v).count();
    let (moving, fixed) = order.split_at(count);
    let apex = fixed[0];
    let mut triangles: Vec<_> = fixed
        .windows(2)
        .rev()
        .map(|pair| [moving[0], pair[0], pair[1]])
        .collect();
    triangles.extend(moving.windows(2).map(|pair| [pair[0], pair[1], apex]));
    for tri in &triangles {
        let [a, b, c] = tri.map(vector);
        let cross = b.sub(a).cross(c.sub(a), h);
        if length(cross, h) < 1e-12 || cross.dot(normal, h) <= 0. {
            return None;
        }
    }
    let rim = moving
        .iter()
        .skip(1)
        .take(count.saturating_sub(2))
        .copied()
        .collect();
    Some((triangles, apex, rim))
}

pub(crate) fn exact_geometry<K: std::hash::Hash + Eq>(
    points: &[Point],
    triangles: &[Triangle],
    adjacent: &HashMap<[u64; 3], HashSet<K>>,
    normal: super::geometry::V3,
    h: Hypot,
) -> Option<Vec<Triangle>> {
    let n = points.len();
    if !(3..=96).contains(&n) {
        return None;
    }
    let doubles_neighbour = |tri: Triangle| {
        let [Some(a), Some(b), Some(c)] = tri.map(|p| adjacent.get(&p.map(f64::to_bits))) else {
            return false;
        };
        a.iter().any(|f| b.contains(f) && c.contains(f))
    };
    if !triangles.iter().any(|&t| doubles_neighbour(t)) {
        return None;
    }
    // Least total area triangulation with no inverted piece or piece
    // lying on a neighbour. Equal costs retain the first split.
    let mut costs = vec![0.; n * n];
    let mut splits = vec![0; n * n];
    for gap in 2..n {
        for i in 0..n - gap {
            let j = i + gap;
            let mut best = f64::INFINITY;
            for k in i + 1..j {
                let sub = costs[i * n + k] + costs[k * n + j];
                if sub >= best {
                    continue;
                }
                let tri = [points[i], points[k], points[j]];
                let [a, b, c] = tri.map(vector);
                let cross = b.sub(a).cross(c.sub(a), h);
                let area = length(cross, h) * 0.5;
                if cross.dot(normal, h) > 0.
                    && area >= 1e-12
                    && !doubles_neighbour(tri)
                    && sub + area < best
                {
                    best = sub + area;
                    splits[i * n + j] = k;
                }
            }
            costs[i * n + j] = best;
        }
    }
    if !costs[n - 1].is_finite() {
        return None;
    }
    let mut out = Vec::new();
    let mut stack = vec![(0, n - 1)];
    while let Some((i, j)) = stack.pop() {
        if j - i < 2 {
            continue;
        }
        let k = splits[i * n + j];
        out.push([points[i], points[k], points[j]]);
        stack.extend([(i, k), (k, j)]);
    }
    Some(out)
}

impl Store {
    fn corner_positions(&self, ring: &[u32]) -> Vec<Point> {
        ring.iter()
            .map(|&v| *self.state.vpos.get(v as usize))
            .collect()
    }

    fn strip_fold(
        &self,
        face: u32,
        moved: &HashSet<Cell>,
        tolerance: f64,
        h: Hypot,
    ) -> Option<Fold> {
        let rings = self.face_rings(face);
        if rings.len() != 1 || moved.is_empty() {
            return None;
        }
        let ring = &rings[0];
        let n = ring.len();
        if n < 4 {
            return None;
        }
        let points = self.corner_positions(ring);
        let (triangles, apex, rim) = strip_geometry(&points, moved, self.face_normal(face, h), h)?;
        // The facade indexes a repeated corner by its last loop occurrence.
        let index: HashMap<_, _> = points
            .iter()
            .enumerate()
            .map(|(i, &p)| (cell(p), i))
            .collect();
        let mut soft = Vec::new();
        for p in rim {
            let i = index[&cell(p)];
            let curved = |a, b| {
                self.find_edge(a, b)
                    .is_some_and(|e| *self.state.ecurve.get(e as usize) >= 0)
            };
            if curved(ring[(i + n - 1) % n], ring[i]) && curved(ring[i], ring[(i + 1) % n]) {
                soft.push((apex, p));
            }
        }
        Some(Fold {
            triangles,
            soft,
            tolerance: Some(tolerance),
        })
    }

    fn fold_neighbours(&self, face: u32, p: Point) -> HashSet<u32> {
        let Some(v) = self.lookup(p) else {
            return HashSet::new();
        };
        self.edges_at(v)
            .flat_map(|e| self.edge_faces(e).iter().copied())
            .filter(|&f| f != face)
            .collect()
    }

    fn exact_fold(&self, face: u32, triangles: &[Triangle], h: Hypot) -> Option<Vec<Triangle>> {
        let rings = self.face_rings(face);
        let n = rings[0].len();
        if rings.len() != 1 || !(3..=96).contains(&n) {
            return None;
        }
        let points = self.corner_positions(&rings[0]);
        let adjacent: HashMap<_, _> = points
            .iter()
            .map(|&p| (p.map(f64::to_bits), self.fold_neighbours(face, p)))
            .collect();
        exact_geometry(&points, triangles, &adjacent, self.face_normal(face, h), h)
    }

    pub(super) fn plan_fold(
        &self,
        face: u32,
        moved: &HashSet<Cell>,
        tolerance: f64,
        h: Hypot,
    ) -> Fold {
        if let Some(plan) = self.strip_fold(face, moved, tolerance, h) {
            return plan;
        }
        let mut triangles = self.face_triangles(face, None, h);
        let mut merge_tolerance = None;
        if triangles.len() >= 2 {
            if let Some(better) = self.exact_fold(face, &triangles, h) {
                triangles = better;
                merge_tolerance = Some(tolerance * 0.5);
            }
        }
        Fold {
            triangles,
            soft: vec![],
            tolerance: merge_tolerance,
        }
    }

    pub(super) fn split_fold(&mut self, face: u32, fold: &Fold) -> Result<Vec<u128>, OpError> {
        if self.remove_face_blocked(face) {
            return Err(OpError::Fallback("fold incidence"));
        }
        self.command_remove_face(face);
        let mut keys = Vec::new();
        for tri in &fold.triangles {
            let mut change = Change::default();
            let f = self.add_face(&[tri.to_vec()], &mut change)?;
            self.finish(change);
            keys.push(self.key(Kind::Face, f));
        }
        Ok(keys)
    }

    /// Merge at most one pair in the editor's piece order. Both operands
    /// must be descendants of the same original face.
    pub(super) fn merge_fold(
        &mut self,
        order: &[u128],
        fold: &Fold,
        h: Hypot,
    ) -> Result<Option<(u128, u128, u128, usize)>, OpError> {
        let pieces: HashSet<_> = order
            .iter()
            .filter_map(|&k| self.slot(Kind::Face, k))
            .collect();
        for &key in order {
            let Some(f) = self.slot(Kind::Face, key) else {
                continue;
            };
            let ring = self.outer_ring(f);
            for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                let Some(e) = self.find_edge(a, b) else {
                    continue;
                };
                let &[x, y] = self.edge_faces(e) else {
                    continue;
                };
                let g = if y == f { x } else { y };
                if f == g || !pieces.contains(&g) {
                    continue;
                }
                let normal = |f| normalized(self.face_normal(f, h), h);
                if normal(f).dot(normal(g), h) <= 0.999 {
                    continue;
                }
                if let Some(tol) = fold.tolerance {
                    let mut corners = self.corner_positions(&ring);
                    corners.extend(self.corner_positions(&self.outer_ring(g)));
                    if !is_planar(&corners, tol) {
                        continue;
                    }
                }
                let Plan::Merge(m) = self.plan_dissolve_edge(e, true, h) else {
                    self.finish(Change::default());
                    continue;
                };
                // All pieces start as triangles without openings.
                let mut change = Change::default();
                let (new, dirty) = self.apply_merge(&m, &[], &mut change)?;
                if self.dangling(&m.faces) {
                    return Err(OpError::Fallback("fold merge incidence"));
                }
                self.finish(change);
                return Ok(Some((
                    key,
                    self.key(Kind::Face, g),
                    self.key(Kind::Face, new),
                    dirty,
                )));
            }
        }
        Ok(None)
    }

    pub(super) fn soften_fold(&mut self, fold: &Fold) {
        for &(a, b) in &fold.soft {
            if let Some(e) = self
                .lookup(a)
                .zip(self.lookup(b))
                .and_then(|(a, b)| self.find_edge(a, b))
            {
                if self.edge_faces(e).len() == 2 {
                    self.set_edge_flags(e, *self.state.eflags.get(e as usize) | SOFT);
                }
            }
        }
    }
}
