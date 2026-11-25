// SPDX-License-Identifier: GPL-3.0-or-later
//! Ordered intersections, half-edge traversal and face extraction on a plane.
use super::planar::{nesting, Cell, Metrics, Point, Ring, TOL};
use crate::math::{xyz, Vec3};
use glam::DVec3;
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PySet, PyTuple},
};
use std::collections::{HashMap, HashSet, VecDeque};

const EPS: f64 = 1e-7;
type Segment = [Point; 2];

#[pyfunction]
pub(crate) fn plane_segment_parameter(p: Point, a: Point, b: Point) -> Option<f64> {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    if l2 < 1e-12 {
        return None;
    }
    let t = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2;
    let (ex, ey) = (p[0] - (a[0] + t * dx), p[1] - (a[1] + t * dy));
    if ex * ex + ey * ey > TOL * TOL || !(-EPS..=1. + EPS).contains(&t) {
        None
    } else {
        Some(t.clamp(0., 1.))
    }
}

fn intersections([a, b]: Segment, [c, d]: Segment, out: &mut Vec<Point>) {
    let (rx, ry) = (b[0] - a[0], b[1] - a[1]);
    let (sx, sy) = (d[0] - c[0], d[1] - c[1]);
    let cross = rx * sy - ry * sx;
    let (qx, qy) = (c[0] - a[0], c[1] - a[1]);
    if cross.abs() > 1e-9 {
        let (t, u) = ((qx * sy - qy * sx) / cross, (qx * ry - qy * rx) / cross);
        if (-EPS..=1. + EPS).contains(&t) && (-EPS..=1. + EPS).contains(&u) {
            out.push([a[0] + t * rx, a[1] + t * ry]);
        }
    } else if (qx * ry - qy * rx).abs() < 1e-9 {
        out.extend(
            [c, d]
                .into_iter()
                .filter(|&p| plane_segment_parameter(p, a, b).is_some()),
        );
        out.extend(
            [a, b]
                .into_iter()
                .filter(|&p| plane_segment_parameter(p, c, d).is_some()),
        );
    }
}

#[pyfunction]
pub(crate) fn plane_intersections(a: Point, b: Point, c: Point, d: Point) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    intersections([a, b], [c, d], &mut out);
    out.into_iter().map(Into::into).collect()
}

fn neighbours(segments: &[Segment]) -> Vec<Vec<usize>> {
    let mut longest: f64 = 0.;
    let boxes: Vec<_> = segments
        .iter()
        .map(|[a, b]| {
            longest = longest.max((a[0] - b[0]).abs()).max((a[1] - b[1]).abs());
            (
                a[0].min(b[0]),
                a[0].max(b[0]),
                a[1].min(b[1]),
                a[1].max(b[1]),
            )
        })
        .collect();
    let reach = 4. * (TOL + 2. * EPS * longest) + 1e-9;
    let mut order: Vec<_> = (0..boxes.len()).collect();
    order.sort_by(|&i, &j| boxes[i].0.total_cmp(&boxes[j].0));
    let mut near = vec![Vec::new(); boxes.len()];
    for (k, &i) in order.iter().enumerate() {
        let (_, x, y0, y1) = boxes[i];
        for &j in &order[k + 1..] {
            let (lo, _, bottom, top) = boxes[j];
            if lo > x + reach {
                break;
            }
            if bottom <= y1 + reach && y0 <= top + reach {
                near[i].push(j);
                near[j].push(i);
            }
        }
    }
    for row in &mut near {
        row.sort_unstable();
    }
    near
}

struct Graph {
    index: HashMap<Cell, usize>,
    points: Ring,
    /// Each vertex's cell as the Python int pair the old tracer keyed by.
    keys: Vec<[i128; 2]>,
    by_key: HashMap<[i128; 2], usize>,
    // Set iteration determines persistent edge order and the first corner of
    // each face: CPython's set of int pairs, replayed (`crate::pyset`).
    adjacent: Vec<PySetOf<[i128; 2]>>,
    /// The order each vertex's neighbours were added in (to build the real
    /// sets `plane_split_graph` hands to Python).
    added: Vec<Vec<usize>>,
}
type PySetOf<K> = crate::pyset::PySet<K>;

pub(super) fn cell_key(k: Cell) -> [i128; 2] {
    k.point().map(|x| x as i128)
}

impl Graph {
    fn vertex(&mut self, p: Point) -> PyResult<usize> {
        let k =
            Cell::at(p, TOL).ok_or_else(|| PyValueError::new_err("non-finite plane coordinate"))?;
        if let Some(&i) = self.index.get(&k) {
            return Ok(i);
        }
        let i = self.points.len();
        self.index.insert(k, i);
        self.points.push(p);
        self.keys.push(cell_key(k));
        self.by_key.insert(cell_key(k), i);
        self.adjacent.push(PySetOf::new());
        self.added.push(Vec::new());
        Ok(i)
    }
    fn build(segments: &[Segment]) -> PyResult<Self> {
        let mut g = Self {
            index: HashMap::new(),
            points: Vec::new(),
            keys: Vec::new(),
            by_key: HashMap::new(),
            adjacent: Vec::new(),
            added: Vec::new(),
        };
        for (i, near) in neighbours(segments).into_iter().enumerate() {
            let [a, b] = segments[i];
            let mut pts = vec![a, b];
            let mut hits = Vec::new();
            for j in near {
                hits.clear();
                intersections([a, b], segments[j], &mut hits);
                pts.extend(
                    hits.iter()
                        .copied()
                        .filter(|&p| plane_segment_parameter(p, a, b).is_some()),
                );
            }
            let mut sorted: Vec<_> = pts
                .into_iter()
                .map(|p| (plane_segment_parameter(p, a, b).unwrap_or(0.), p))
                .collect();
            sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            let mut prev: Option<usize> = None;
            for (_, p) in sorted {
                let k = g.vertex(p)?;
                if let Some(j) = prev.filter(|&j| j != k) {
                    let (kk, kj) = (g.keys[k], g.keys[j]);
                    g.adjacent[k].add(kj);
                    g.adjacent[j].add(kk);
                    g.added[k].push(j);
                    g.added[j].push(k);
                }
                prev = Some(k);
            }
        }
        Ok(g)
    }
    fn ids(&self, set: &PySetOf<[i128; 2]>) -> Vec<usize> {
        set.iter().map(|k| self.by_key[&k]).collect()
    }
    fn full_edges(&self) -> Vec<(usize, usize)> {
        let mut edges = Vec::new();
        for (i, set) in self.adjacent.iter().enumerate() {
            for j in self.ids(set) {
                if j > i {
                    edges.push((i, j));
                }
            }
        }
        edges
    }
    fn cycles(&self) -> PyResult<Vec<Ring>> {
        let mut links: Vec<Vec<usize>> = self.adjacent.iter().map(|s| self.ids(&s.copy())).collect();
        let mut live: Vec<_> = links.iter().map(Vec::len).collect();
        let mut q: VecDeque<_> = (0..links.len()).filter(|&i| live[i] == 1).collect();
        while let Some(i) = q.pop_front() {
            if live[i] == 0 {
                continue;
            }
            live[i] = 0;
            for &j in &links[i] {
                if live[j] > 0 {
                    live[j] -= 1;
                    if live[j] == 1 {
                        q.push_back(j);
                    }
                }
            }
        }
        for (i, ns) in links.iter_mut().enumerate() {
            if live[i] == 0 {
                ns.clear();
            } else {
                ns.retain(|&j| live[j] != 0);
            }
        }
        let mut next = HashMap::new();
        for (i, ns) in links.iter().enumerate() {
            let center = self.points[i];
            let mut turns: Vec<_> = ns
                .iter()
                .map(|&j| {
                    let p = self.points[j];
                    ((p[1] - center[1]).atan2(p[0] - center[0]), j)
                })
                .collect();
            turns.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            for (at, &(_, j)) in turns.iter().enumerate() {
                next.insert((j, i), turns[(at + turns.len() - 1) % turns.len()].1);
            }
        }
        let mut used = HashSet::new();
        let mut loops = Vec::new();
        for (i, ns) in links.iter().enumerate() {
            for &j in ns {
                let mut run = Vec::new();
                let mut side = (i, j);
                while used.insert(side) {
                    run.push(self.points[side.0]);
                    side = (side.1, next[&side]);
                    if run.len() > 100000 {
                        break;
                    }
                }
                if run.len() >= 3 {
                    loops.push(run);
                }
            }
        }
        Ok(loops)
    }
}

#[pyfunction]
pub(crate) fn plane_split_graph<'py>(
    py: Python<'py>,
    segments: Vec<Segment>,
) -> PyResult<(Bound<'py, PyDict>, Bound<'py, PyDict>)> {
    let g = Graph::build(&segments)?;
    let coords = PyDict::new(py);
    let adjacent = PyDict::new(py);
    let tuple = |k: [i128; 2]| PyTuple::new(py, k);
    for (i, &k) in g.keys.iter().enumerate() {
        coords.set_item(tuple(k)?, (g.points[i][0], g.points[i][1]))?;
        // A real set, built by the same adds in the same order.
        let set = PySet::empty(py)?;
        for &j in &g.added[i] {
            set.add(tuple(g.keys[j])?)?;
        }
        adjacent.set_item(tuple(k)?, set)?;
    }
    Ok((coords, adjacent))
}

/// The planar arrangement of `lines` (plane coordinates, already free of
/// zero-length segments): every split edge when `include_edges`, and the
/// minimal faces with their holes (`core.arrangement.planar_rebuild`).
#[allow(clippy::type_complexity)]
pub(super) fn arrange(
    lines: &[Segment],
    include_edges: bool,
    metrics: &Metrics<'_>,
) -> PyResult<(Vec<(Point, Point)>, Vec<(Ring, Vec<Ring>)>)> {
    let g = Graph::build(lines)?;
    let mut edges = Vec::new();
    if include_edges {
        for (a, b) in g.full_edges() {
            edges.push((g.points[a], g.points[b]));
        }
    }
    let cycles = g.cycles()?;
    let faces = nesting(metrics, &cycles, true)?
        .into_iter()
        .map(|(i, hs)| (cycles[i].clone(), hs.into_iter().map(|h| cycles[h].clone()).collect()))
        .collect();
    Ok((edges, faces))
}

#[pyfunction]
pub(crate) fn arrange_plane(
    segments: &Bound<'_, PyAny>,
    origin: &Bound<'_, PyAny>,
    axes: (Bound<'_, PyAny>, Bound<'_, PyAny>),
    include_edges: bool,
) -> PyResult<(Vec<(Vec3, Vec3)>, Vec<(Vec<Vec3>, Vec<Vec<Vec3>>)>)> {
    let py = origin.py();
    let o = xyz(origin)?;
    let u = xyz(&axes.0)?;
    let v = xyz(&axes.1)?;
    let project = |p: DVec3| [(p - o).dot(u), (p - o).dot(v)];
    let lift = |p: Point| Vec3::from(o + u * p[0] + v * p[1]);
    let mut lines = Vec::new();
    for pair in segments.try_iter()? {
        let pair = pair?;
        let a = project(xyz(&pair.get_item(0)?)?);
        let b = project(xyz(&pair.get_item(1)?)?);
        let (dx, dy) = (a[0] - b[0], a[1] - b[1]);
        if dx * dx + dy * dy > TOL * TOL {
            lines.push([a, b]);
        }
    }
    let (edges, faces) = arrange(&lines, include_edges, &Metrics::new(py)?)?;
    Ok((
        edges.into_iter().map(|(a, b)| (lift(a), lift(b))).collect(),
        faces
            .into_iter()
            .map(|(r, hs)| (r.into_iter().map(lift).collect(), hs.into_iter().map(|h| h.into_iter().map(lift).collect()).collect()))
            .collect(),
    ))
}
