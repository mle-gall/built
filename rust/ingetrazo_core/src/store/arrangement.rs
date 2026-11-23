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

struct Graph<'py> {
    index: HashMap<Cell, usize>,
    points: Ring,
    keys: Vec<Bound<'py, PyTuple>>,
    // Set iteration determines persistent edge order and the first corner of
    // each face. Use the running interpreter's tuple-of-integers hash here.
    adjacent: Vec<Bound<'py, PySet>>,
}
impl<'py> Graph<'py> {
    fn vertex(&mut self, py: Python<'py>, p: Point) -> PyResult<usize> {
        let k =
            Cell::at(p, TOL).ok_or_else(|| PyValueError::new_err("non-finite plane coordinate"))?;
        if let Some(&i) = self.index.get(&k) {
            return Ok(i);
        }
        let i = self.points.len();
        self.index.insert(k, i);
        self.points.push(p);
        self.keys.push(k.tuple(py)?);
        self.adjacent.push(PySet::empty(py)?);
        Ok(i)
    }
    fn build(py: Python<'py>, segments: &[Segment]) -> PyResult<Self> {
        let mut g = Self {
            index: HashMap::new(),
            points: Vec::new(),
            keys: Vec::new(),
            adjacent: Vec::new(),
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
                let k = g.vertex(py, p)?;
                if let Some(j) = prev.filter(|&j| j != k) {
                    g.adjacent[k].add(&g.keys[j])?;
                    g.adjacent[j].add(&g.keys[k])?;
                }
                prev = Some(k);
            }
        }
        Ok(g)
    }
    fn ids(&self, set: &Bound<'py, PySet>) -> PyResult<Vec<usize>> {
        set.iter()
            .map(|v| Ok(self.index[&Cell::at(v.extract()?, 1.).unwrap()]))
            .collect()
    }
    fn full_edges(&self) -> PyResult<Vec<(usize, usize)>> {
        let mut edges = Vec::new();
        for (i, set) in self.adjacent.iter().enumerate() {
            for j in self.ids(set)? {
                if j > i {
                    edges.push((i, j));
                }
            }
        }
        Ok(edges)
    }
    fn cycles(&self) -> PyResult<Vec<Ring>> {
        let mut links = self
            .adjacent
            .iter()
            .map(|s| self.ids(s.call_method0("copy")?.cast::<PySet>()?))
            .collect::<PyResult<Vec<_>>>()?;
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
    let g = Graph::build(py, &segments)?;
    let coords = PyDict::new(py);
    let adjacent = PyDict::new(py);
    for (i, k) in g.keys.iter().enumerate() {
        coords.set_item(k, (g.points[i][0], g.points[i][1]))?;
        adjacent.set_item(k, &g.adjacent[i])?;
    }
    Ok((coords, adjacent))
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
    let g = Graph::build(py, &lines)?;
    let mut edges = Vec::new();
    if include_edges {
        for (a, b) in g.full_edges()? {
            edges.push((lift(g.points[a]), lift(g.points[b])));
        }
    }
    let cycles = g.cycles()?;
    let lift_ring = |k: usize| cycles[k].iter().copied().map(lift).collect();
    let faces = nesting(&Metrics::new(py)?, &cycles, true)?
        .into_iter()
        .map(|(i, hs)| (lift_ring(i), hs.into_iter().map(lift_ring).collect()))
        .collect();
    Ok((edges, faces))
}
