// SPDX-License-Identifier: GPL-3.0-or-later
//! Plane selection and the position-keyed graph used by the drawing planner.
use super::{keys::Kind, Store};
use crate::facegeom::{length, normalized, Hypot, V3};
use std::collections::{HashMap, HashSet, VecDeque};

type Point = [f64; 3];
type Node = [u64; 3];
type Links = HashMap<Node, Vec<(usize, Point)>>;

pub(super) fn vector(p: Point) -> V3 {
    V3(p[0] as f32, p[1] as f32, p[2] as f32)
}
pub(super) fn point(v: V3) -> Point {
    [v.0 as f64, v.1 as f64, v.2 as f64]
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
        let o = points().next().unwrap_or(V3(0., 0., 0.));
        let offsets = || points().map(|p| p.sub(o));
        let horizontal = Some([point(o), [0., 0., 1.]]);
        let Some(dir) = offsets().find(|&d| length(d, h) as f64 > 1e-6) else {
            return horizontal;
        };
        let dir = normalized(dir, h);
        let Some(cross) = offsets()
            .map(|d| dir.cross(d, h))
            .find(|&d| length(d, h) as f64 > 1e-6)
        else {
            return horizontal;
        };
        let n = normalized(cross, h);
        offsets()
            .all(|d| (d.dot(n, h) as f64).abs() <= 1e-4)
            .then(|| [point(o), point(n)])
    }

    pub fn rebuild_plane(&self, points: &[Point], cap: usize, h: Hypot) -> Option<[Point; 2]> {
        let faces = self.state.order[2].live(&self.state.slots[2]);
        let plane = |f| {
            let ring = self.outer_ring(f);
            ring.first().map(|&v| {
                (
                    vector(*self.state.vpos.get(v as usize)),
                    self.ring_normal(&ring, h),
                )
            })
        };
        let (o, n) = faces.iter().filter_map(|&f| plane(f)).find(|&(o, n)| {
            n.dot(n, h) >= 0.5
                && points
                    .iter()
                    .all(|&p| (vector(p).sub(o).dot(n, h) as f64).abs() < 1e-4)
        })?;
        let count = faces
            .iter()
            .filter_map(|&f| plane(f))
            .filter(|&(p, m)| {
                (p.sub(o).dot(n, h) as f64).abs() < 1e-4 && (m.dot(n, h) as f64).abs() > 0.999
            })
            .take(cap.saturating_add(1))
            .count();
        (count <= cap).then(|| [point(o), point(n)])
    }

    pub fn curve_keys(&self, curve: i64) -> Vec<u128> {
        self.state.order[1]
            .live(&self.state.slots[1])
            .into_iter()
            .filter(|&e| *self.state.ecurve.get(e as usize) == curve)
            .map(|e| self.key(Kind::Edge, e))
            .collect()
    }
}

// f32 coordinates times 10,000 are exact in f64. Key by rounded integer
// bits so large coordinates cannot saturate an integer cell index.
pub(super) fn key(p: Point) -> Node {
    p.map(|x| {
        let q = (x * 10000.).round_ties_even();
        if q == 0. {
            0
        } else {
            q.to_bits()
        }
    })
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

/// Closest points in units of distance along each line. Qt rounds only the
/// initial differences, lengths and returned position to float32.
fn intersection(
    [a, b]: [Point; 2],
    [c, d]: [Point; 2],
    h: Hypot,
    compensated: bool,
) -> Option<Point> {
    use crate::facegeom::py_sum;
    let delta = |a, b| vector(b).sub(vector(a));
    let (ab, cd) = (delta(a, b), delta(c, d));
    let (l, m) = (length(ab, h) as f64, length(cd, h) as f64);
    if l < 1e-4 || m < 1e-4 {
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
    if !(-1e-4 / l..=1. + 1e-4 / l).contains(&(s / l))
        || !(-1e-4 / m..=1. + 1e-4 / m).contains(&(t / m))
    {
        return None;
    }
    let p: Point = std::array::from_fn(|i| a[i] + s * u[i]);
    let q: Point = std::array::from_fn(|i| c[i] + t * v[i]);
    let gap = std::array::from_fn(|i| p[i] - q[i]);
    if dot(gap, gap).sqrt() > 1e-4_f64.max(1e-7 * l.max(m)) {
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
    let close = |p, q| (length(vector(p).sub(vector(q)), h) as f64) <= 1e-4;
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
