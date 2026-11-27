// SPDX-License-Identifier: GPL-3.0-or-later
//! Plane selection and the position-keyed graph used by the drawing planner.
use super::{keys::Kind, Store};
use crate::store::geometry::{length, normalized, Hypot, V3};
use std::collections::{HashMap, HashSet, VecDeque};

type Point = [f64; 3];
type Node = [u64; 3];
type Links = HashMap<Node, Vec<(usize, Point)>>;

pub(super) fn vector(p: Point) -> V3 {
    V3(p[0], p[1], p[2])
}
pub(super) fn point(v: V3) -> Point {
    [v.0, v.1, v.2]
}

pub(crate) fn drawing_plane(
    points: impl Iterator<Item = V3> + Clone,
    h: Hypot,
) -> Option<[Point; 2]> {
    drawing_plane_where(points, h, |distance| distance <= 1e-4)
}

pub(crate) fn drawing_plane_where(
    points: impl Iterator<Item = V3> + Clone,
    h: Hypot,
    accepts_distance: impl Fn(f64) -> bool,
) -> Option<[Point; 2]> {
    let o = points.clone().next().unwrap_or(V3(0., 0., 0.));
    let offsets = || points.clone().map(|p| p.sub(o));
    let horizontal = Some([point(o), [0., 0., 1.]]);
    let Some(dir) = offsets().find(|&d| length(d, h) > 1e-6) else {
        return horizontal;
    };
    let dir = normalized(dir, h);
    let Some(cross) = offsets()
        .map(|d| dir.cross(d, h))
        .find(|&d| length(d, h) > 1e-6)
    else {
        return horizontal;
    };
    let n = normalized(cross, h);
    offsets()
        .all(|d| accepts_distance(d.dot(n, h).abs()))
        .then(|| [point(o), point(n)])
}

pub(crate) fn rebuild_plane(
    planes: impl Iterator<Item = (V3, V3)> + Clone,
    points: &[Point],
    cap: usize,
    h: Hypot,
) -> Option<[Point; 2]> {
    let (o, n) = planes.clone().find(|&(o, n)| {
        n.dot(n, h) >= 0.5
            && points
                .iter()
                .all(|&p| vector(p).sub(o).dot(n, h).abs() < 1e-4)
    })?;
    let count = planes
        .filter(|&(p, m)| p.sub(o).dot(n, h).abs() < 1e-4 && m.dot(n, h).abs() > 0.999)
        .take(cap.saturating_add(1))
        .count();
    (count <= cap).then(|| [point(o), point(n)])
}

pub(crate) struct StrokeSegment {
    start: glam::DVec3,
    delta: glam::DVec3,
    length: f64,
}

impl StrokeSegment {
    pub(crate) fn new(start: glam::DVec3, end: glam::DVec3) -> Self {
        let delta = end - start;
        Self { start, delta, length: delta.length() }
    }

    pub(crate) fn contains(&self, point: glam::DVec3, tol: f64) -> bool {
        if self.length < tol {
            return super::commands::cell(point.to_array()) == super::commands::cell(self.start.to_array());
        }
        let t = (point - self.start).dot(self.delta) / (self.length * self.length);
        (-tol..=1.0 + tol).contains(&t) && point.distance(self.start + self.delta * t) < tol
    }
}

pub(crate) fn path_segments(path: &[Point], closed: bool) -> Vec<StrokeSegment> {
    path.iter().zip(path.iter().cycle().skip(1))
        .take(path.len().saturating_sub(usize::from(!closed)))
        .map(|(&a, &b)| StrokeSegment::new(a.into(), b.into())).collect()
}

impl Store {
    pub fn drawing_plane(&self, extra: &[Point], h: Hypot) -> Option<[Point; 2]> {
        let vertices = self.state.order[0].live(&self.state.slots[0]);
        let points = || {
            vertices
                .iter()
                .map(|&v| *self.state.vpos.get(v as usize))
                .chain(extra.iter().copied())
                .map(vector)
        };
        drawing_plane(points(), h)
    }

    fn face_plane(&self, face: u32, h: Hypot) -> Option<(V3, V3)> {
        let ring = self.outer_ring(face);
        let &first = ring.first()?;
        Some((vector(*self.state.vpos.get(first as usize)), self.ring_normal(&ring, h)))
    }

    pub(super) fn has_empty_outer(&self, faces: &[u32]) -> bool {
        faces.iter().any(|&f| {
            let r = self.state.frings.get(f as usize);
            r.n_rings == 0 || *self.state.ring_sizes.get(r.ring_start as usize) == 0
        })
    }

    /// The common-plane test used by Push/Pull and overlap healing. Walking
    /// the outer rings directly avoids triangulating a solid to test flatness.
    pub fn faces_are_flat(&self, tolerance: f64, h: Hypot) -> Option<bool> {
        let faces = self.live_slots(Kind::Face);
        if faces.len() < 2 {
            return Some(true);
        }
        let (origin, normal) = self.face_plane(faces[0], h)?;
        let unit = normalized(normal, h);
        let distance = normal.dot(origin, h);
        // A new cap or wall is usually at the end of a large sheet. If
        // every outer ring exists, a counterexample anywhere is sufficient.
        if faces.len() >= 1024 && !self.has_empty_outer(&faces) {
            for &face in faces.iter().rev().take(16) {
                let (point, other) = self.face_plane(face, h)?;
                if !(unit.dot(normalized(other, h), h).abs() > 0.999)
                    || (normal.dot(point, h) - distance).abs() > tolerance {
                    return Some(false);
                }
            }
        }
        for &face in &faces[1..] {
            let (point, other) = self.face_plane(face, h)?;
            if !(unit.dot(normalized(other, h), h).abs() > 0.999)
                || (normal.dot(point, h) - distance).abs() > tolerance
            {
                return Some(false);
            }
        }
        Some(true)
    }

    pub fn rebuild_plane(&self, points: &[Point], cap: usize, h: Hypot) -> Option<[Point; 2]> {
        let faces = self.state.order[2].live(&self.state.slots[2]);
        rebuild_plane(faces.iter().filter_map(|&f| self.face_plane(f, h)), points, cap, h)
    }

    /// Stamp untagged edges whose two ends lie on one path segment. This
    /// includes sub-edges created when another stroke crossed the path.
    pub fn tag_curve_path(&mut self, path: &[Point], closed: bool, id: i64) -> bool {
        if path.len() < 2 { return false; }
        let segments = path_segments(path, closed);
        let mut tagged = false;
        for edge in self.live_slots(Kind::Edge) {
            if *self.state.ecurve.get(edge as usize) >= 0 { continue; }
            let ends = self.state.ends.get(edge as usize)
                .map(|v| glam::DVec3::from_array(*self.state.vpos.get(v as usize)));
            if segments.iter().any(|s| ends.iter().all(|&p| s.contains(p, 1e-4))) {
                self.set_edge_curve(edge, id.into());
                tagged = true;
            }
        }
        tagged
    }

    pub fn curve_keys(&self, curve: i128) -> Vec<u128> {
        self.state.order[1]
            .live(&self.state.slots[1])
            .into_iter()
            .filter(|&e| *self.state.ecurve.get(e as usize) == curve)
            .map(|e| self.key(Kind::Edge, e))
            .collect()
    }
}

// f64 coordinates times 10,000 are exact in f64. Key by rounded integer
// bits so large coordinates cannot saturate an integer cell index.
pub(super) fn key(p: Point) -> Node {
    super::commands::cell(p)
}

#[derive(Default)]
pub(crate) struct DrawingGraph(Links);
impl DrawingGraph {
    pub fn append(&mut self, i: usize, [a, b]: [Point; 2]) {
        let (u, v) = (key(a), key(b));
        if u != v {
            self.0.entry(u).or_default().push((i, b));
            self.0.entry(v).or_default().push((i, a));
        }
    }

    /// Edge `i` (ending at `a` and `b`) out of the graph.
    pub fn remove(&mut self, i: usize, [a, b]: [Point; 2]) {
        let (u, v) = (key(a), key(b));
        if u == v {
            return;
        }
        for n in [u, v] {
            if let Some(list) = self.0.get_mut(&n) {
                list.retain(|e| e.0 != i);
                if list.is_empty() {
                    self.0.remove(&n);
                }
            }
        }
    }

    /// Edge `i` into the graph, each end's list kept in `rank` order: the
    /// order `append` meets edges in when the graph is built afresh, so a
    /// search walks a patched graph as it would a fresh one.
    pub fn insert(&mut self, i: usize, [a, b]: [Point; 2], rank: impl Fn(usize) -> u64) {
        let (u, v) = (key(a), key(b));
        if u == v {
            return;
        }
        let r = rank(i);
        for (n, p) in [(u, b), (v, a)] {
            let list = self.0.entry(n).or_default();
            let at = list.partition_point(|e| rank(e.0) < r);
            list.insert(at, (i, p));
        }
    }

    #[cfg(test)]
    pub fn sorted(&self) -> Vec<(Node, Vec<(usize, Point)>)> {
        let mut out: Vec<_> = self.0.iter().map(|(k, v)| (*k, v.clone())).collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    pub fn cycles(
        &self,
        a: Point,
        b: Point,
        dead: HashSet<usize>,
        extras: Vec<[Point; 2]>,
        limit: usize,
        both: bool,
    ) -> Vec<Vec<Point>> {
        Self::search_cycles(a, b, dead, extras, limit, both, |node, links| {
            if let Some(row) = self.0.get(&node) { links.extend_from_slice(row); }
        })
    }

    fn search_cycles(
        a: Point, b: Point, dead: HashSet<usize>, extras: Vec<[Point; 2]>,
        limit: usize, both: bool, mut neighbors: impl FnMut(Node, &mut Vec<(usize, Point)>),
    ) -> Vec<Vec<Point>> {
        let (start, end) = (key(b), key(a));
        if start == end || limit < 3 {
            return vec![];
        }
        let mut added = DrawingGraph::default();
        for edge in extras {
            added.append(usize::MAX, edge);
        }
        let mut links = Vec::new();
        let mut search = |skip: &HashSet<Node>| -> Option<Vec<Point>> {
            let mut parents = HashMap::from([(start, (start, b))]);
            let mut queue = VecDeque::from([(start, 1usize)]);
            while let Some((u, depth)) = queue.pop_front() {
                if depth >= limit {
                    continue;
                }
                links.clear();
                neighbors(u, &mut links);
                for &(i, p) in links.iter().chain(added.0.get(&u).into_iter().flatten()) {
                    let v = key(p);
                    if dead.contains(&i)
                        || skip.contains(&v)
                        || (u == start && v == end)
                        || parents.contains_key(&v)
                    {
                        continue;
                    }
                    parents.insert(v, (u, p));
                    if v == end {
                        let mut path = vec![p];
                        let mut parent = u;
                        while parent != start {
                            let (prev, pos) = parents[&parent];
                            path.push(pos);
                            parent = prev;
                        }
                        path.push(b);
                        return Some(path);
                    }
                    queue.push_back((v, depth + 1));
                }
            }
            None
        };
        let Some(first) = search(&HashSet::new()) else {
            return vec![];
        };
        let second = if both {
            search(&first[1..first.len() - 1].iter().copied().map(key).collect())
        } else {
            None
        };
        std::iter::once(first).chain(second).collect()
    }
}

/// A temporary drawing frame can change between neighboring strokes. Index
/// each vertex once and visit only the edge disks the cycle search reaches.
/// Coincident rounded coordinates share a bucket but keep all their edges.
pub(crate) struct FrameVertices {
    slots: Vec<(Node, u32)>,
}

impl FrameVertices {
    pub(crate) fn new(state: &super::State) -> Self {
        let mut slots: Vec<_> = state.slots[0].live_slots()
            .map(|v| (key(*state.vpos.get(v as usize)), v)).collect();
        slots.sort_unstable();
        Self { slots }
    }

    pub(crate) fn cycles(&self, state: &super::State, a: Point, b: Point,
                         dead: HashSet<usize>, extras: Vec<[Point; 2]>,
                         limit: usize, both: bool) -> Vec<Vec<Point>> {
        DrawingGraph::search_cycles(a, b, dead, extras, limit, both, |node, links| {
            let first = self.slots.partition_point(|&(cell, _)| cell < node);
            for &(_, v) in self.slots[first..].iter().take_while(|(cell, _)| *cell == node) {
                for e in state.edges_at(v) {
                    let ends = *state.ends.get(e as usize);
                    let other = ends[usize::from(ends[0] == v)];
                    let point = *state.vpos.get(other as usize);
                    if key(point) != node { links.push((e as usize, point)); }
                }
            }
            links.sort_unstable_by_key(|&(e, _)| state.list_rank(Kind::Edge, e as u32));
        })
    }
}

/// Closest points in units of distance along each line.
pub(crate) fn intersection(
    [a, b]: [Point; 2],
    [c, d]: [Point; 2],
    h: Hypot,
    compensated: bool,
) -> Option<Point> {
    intersection_at([a,b],[c,d],h,compensated,1e-4)
}

pub(crate) fn intersection_at(
    [a,b]: [Point;2], [c,d]: [Point;2], h: Hypot, compensated: bool, tolerance: f64,
) -> Option<Point> {
    use crate::store::geometry::py_sum;
    let delta = |a, b| vector(b).sub(vector(a));
    let (ab, cd) = (delta(a, b), delta(c, d));
    let (l, m) = (length(ab, h), length(cd, h));
    if l < tolerance || m < tolerance {
        return None;
    }
    let (u, v) = (point(ab).map(|x| x / l), point(cd).map(|x| x / m));
    let w = point(delta(c, a));
    let dot = |u: Point, v: Point| py_sum((0..3).map(|i| u[i] * v[i]), compensated);
    let (aa, bb, cc, dd, ee) = (dot(u, u), dot(u, v), dot(v, v), dot(u, w), dot(v, w));
    let denom = aa * cc - bb * bb;
    if denom < 1e-12 {
        return None;
    }
    let (s, t) = ((bb * ee - cc * dd) / denom, (aa * ee - bb * dd) / denom);
    if !(-tolerance / l..=1. + tolerance / l).contains(&(s / l))
        || !(-tolerance / m..=1. + tolerance / m).contains(&(t / m))
    {
        return None;
    }
    let p: Point = std::array::from_fn(|i| a[i] + s * u[i]);
    let q: Point = std::array::from_fn(|i| c[i] + t * v[i]);
    let gap = std::array::from_fn(|i| p[i] - q[i]);
    if dot(gap, gap).sqrt() > tolerance.max(1e-7 * l.max(m)) {
        return None;
    }
    Some(point(vector(std::array::from_fn(|i| (p[i] + q[i]) * 0.5))))
}
pub(crate) fn split_plan(
    a: Point,
    b: Point,
    edges: impl Iterator<Item = [Point; 2]>,
    h: Hypot,
    compensated: bool,
) -> (Vec<Point>, Vec<(usize, Point)>) {
    let mut hits = Vec::new();
    let mut cuts = Vec::new();
    let close = |p, q| length(vector(p).sub(vector(q)), h) <= 1e-4;
    for (i, [x, y]) in edges.enumerate() {
        let Some(hit) = intersection([a, b], [x, y], h, compensated) else {
            continue;
        };
        let p = [x, y, a, b]
            .into_iter()
            .find(|&q| close(q, hit))
            .unwrap_or(hit);
        if key(p) != key(x) && key(p) != key(y) {
            cuts.push((i, p))
        }
        if key(p) != key(a)
            && key(p) != key(b)
            && !close(p, a)
            && !close(p, b)
            && !hits.iter().any(|&q| close(p, q))
        {
            hits.push(p)
        }
    }
    let d = vector(b).sub(vector(a));
    let order = |p| vector(p).sub(vector(a)).dot(d, h);
    hits.sort_by(|&p, &q| {
        order(p)
            .partial_cmp(&order(q))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    (hits, cuts)
}

#[cfg(test)]
mod frame_graph_tests {
    use super::*;
    use crate::store::edit::Change;

    #[test]
    fn framed_cycles_visit_edges_in_the_original_order() {
        let mut store = Store::empty(16, Default::default());
        let mut change = Change::default();
        for x in 0..8 {
            for y in 0..5 {
                let x = x as f64;
                let y = y as f64;
                store.add_face(&[vec![[x,y,0.],[x+1.,y,0.],[x+1.,y+1.,0.],[x,y+1.,0.]]], &mut change).unwrap();
            }
        }
        // Two distinct slots now round to the same drawing cell.
        store.place_vertex(0, [1.00001, 0., 0.]);
        let state = &store.state;
        let mut full = DrawingGraph::default();
        for e in store.live_slots(Kind::Edge) {
            full.append(e as usize, state.ends.get(e as usize).map(|v| *state.vpos.get(v as usize)));
        }
        let indexed = FrameVertices::new(state);
        for x in 0..7 {
            let (a, b) = ([x as f64, 0., 0.], [1. + x as f64, 1., 0.]);
            for both in [false, true] {
                let dead = HashSet::from([1, 9, 21]);
                let extra = vec![[a, [x as f64, -2., 0.]], [[x as f64, -2., 0.], b]];
                assert_eq!(indexed.cycles(state, a, b, dead.clone(), extra.clone(), 7, both),
                           full.cycles(a, b, dead, extra, 7, both));
            }
        }
    }
}
