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

    pub fn cycles(
        &self,
        a: Point,
        b: Point,
        dead: HashSet<usize>,
        extras: Vec<[Point; 2]>,
        limit: usize,
        both: bool,
    ) -> Vec<Vec<Point>> {
        let (start, end) = (key(b), key(a));
        if start == end || limit < 3 {
            return vec![];
        }
        let mut added = DrawingGraph::default();
        for edge in extras {
            added.append(usize::MAX, edge);
        }
        let search = |skip: &HashSet<Node>| -> Option<Vec<Point>> {
            let mut parents = HashMap::from([(start, (start, b))]);
            let mut queue = VecDeque::from([(start, 1usize)]);
            while let Some((u, depth)) = queue.pop_front() {
                if depth >= limit {
                    continue;
                }
                for &(i, p) in self
                    .0
                    .get(&u)
                    .into_iter()
                    .flatten()
                    .chain(added.0.get(&u).into_iter().flatten())
                {
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
