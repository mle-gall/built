// SPDX-License-Identifier: GPL-3.0-or-later
//! Edge and vertex rounding share the replacement transaction and preview data.
use super::{edit::Change, keys::Kind, topo::OpError, Store, SOFT};
use crate::store::geometry::{length, Hypot, V3};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::{Add, Div, Mul, Sub};

impl Add for V3 {
    type Output = Self;
    fn add(self, b: Self) -> Self {
        Self(self.0 + b.0, self.1 + b.1, self.2 + b.2)
    }
}
impl Sub for V3 {
    type Output = Self;
    fn sub(self, b: Self) -> Self {
        self.sub(b)
    }
}
impl Mul<f64> for V3 {
    type Output = Self;
    fn mul(self, s: f64) -> Self {
        let k = s;
        Self(self.0 * k, self.1 * k, self.2 * k)
    }
}
impl Div<f64> for V3 {
    type Output = Self;
    fn div(self, s: f64) -> Self {
        let k = s;
        Self(self.0 / k, self.1 / k, self.2 / k)
    }
}
pub(super) const ZERO: V3 = V3(0., 0., 0.);
pub(super) fn pos(s: &Store, v: u32) -> V3 {
    let p = s.state.vpos.get(v as usize);
    V3(p[0], p[1], p[2])
}
pub(super) fn xyz(p: V3) -> [f64; 3] {
    [p.0, p.1, p.2]
}
pub(super) fn unit(a: V3, h: Hypot) -> Option<V3> {
    let n = length(a, h);
    (n >= 1e-12).then(|| a / n)
}
pub(super) fn ends(s: &Store, e: u32) -> [u32; 2] {
    *s.state.ends.get(e as usize)
}
pub(super) fn other(s: &Store, e: u32, v: u32) -> u32 {
    let [a, b] = ends(s, e);
    if a == v {
        b
    } else {
        a
    }
}
pub(super) fn mean(p: &[V3]) -> V3 {
    p.iter().copied().fold(ZERO, |a, b| a + b) / p.len() as f64
}
pub(super) fn newell(p: &[V3]) -> V3 {
    let mut n = ZERO;
    for (&a, &b) in p.iter().zip(p.iter().cycle().skip(1)) {
        let ([x, y, z], [u, v, w]) = (xyz(a), xyz(b));
        n = n + V3((y - v) * (z + w), (z - w) * (x + u), (x - u) * (y + v));
    }
    n
}
pub(super) fn facing(mut ring: Vec<V3>, target: V3, sign: f64, h: Hypot) -> Vec<V3> {
    if newell(&ring).dot(target - mean(&ring), h) * sign < 0. {
        ring.reverse()
    }
    ring
}
pub(super) type Refusal = (&'static str, Option<V3>);
pub(super) type Result<T> = std::result::Result<T, Refusal>;
pub(super) fn at(msg: &'static str, p: V3) -> Refusal {
    (msg, Some(p))
}
pub(super) fn check(size: f64, profile: f64) -> Result<()> {
    if !size.is_finite() || size <= 0. {
        return Err(("The radius must be positive.", None));
    }
    if !(profile > 0. && profile < 1.) {
        return Err(("The profile must be between 0 and 1.", None));
    }
    Ok(())
}

#[derive(Default)]
pub struct RoundPlan {
    pub rebuilt: Vec<(u32, Vec<Vec<V3>>)>,
    pub edges: BTreeSet<u32>,
    pub strips: Vec<Vec<V3>>,
    pub patches: Vec<Vec<V3>>,
    pub soft: Vec<(V3, V3)>,
    pub curves: Vec<Vec<V3>>,
    pub flags: Vec<(u32, V3, V3)>,
    pub materials: Vec<u32>,
}
impl RoundPlan {
    pub(super) fn rebuild(
        &mut self,
        s: &impl FilletMesh,
        f: u32,
        h: Hypot,
        mut replace: impl FnMut(&[u32], usize) -> Result<Vec<V3>>,
    ) -> Result<()> {
        let mut rings = Vec::new();
        for ring in s.rings(f) {
            let mut out = Vec::new();
            for i in 0..ring.len() {
                for p in replace(&ring, i)? {
                    if out.last().is_none_or(|&q| length(p - q, h) > 1e-7) {
                        out.push(p)
                    }
                }
            }
            if out.len() > 1 && length(out[0] - out[out.len() - 1], h) <= 1e-7 {
                out.pop();
            }
            if out.len() < 3 {
                return Err(("A face would collapse — the radius is too large.", None));
            }
            rings.push(out);
        }
        self.rebuilt.push((f, rings));
        Ok(())
    }
    /// The geometry used to choose soft seams sees the original rebuilt
    /// rings. Apply add_face's hole filter after those decisions.
    pub fn filter_nested_holes(&mut self, h: Hypot) {
        for (_, rings) in &mut self.rebuilt {
            if rings.len() > 2 {
                let mask = maximal_holes(&rings[1..], h);
                let mut keep = std::iter::once(true).chain(mask);
                rings.retain(|_| keep.next().unwrap());
            }
        }
    }
    pub fn apply(
        &self,
        s: &mut Store,
        ids: &[i64],
        material: u32,
        h: Hypot,
        ch: &mut Change,
    ) -> std::result::Result<(), OpError> {
        for (f, _) in &self.rebuilt {
            s.remove_face(*f, ch)
        }
        for &e in &self.edges {
            if s.edge_faces(e).is_empty() {
                s.remove_edge(e, ch)
            }
        }
        for (old, rings) in &self.rebuilt {
            let rings = rings
                .iter()
                .map(|r| r.iter().copied().map(xyz).collect())
                .collect::<Vec<_>>();
            let f = s.add_face(&rings, ch)?;
            let attr = *s.state.fattr.get(*old as usize);
            let flags = *s.state.fflags.get(*old as usize);
            s.state.fattr.set(f as usize, attr);
            s.state.fflags.set(f as usize, flags);
        }
        for r in self.strips.iter().chain(&self.patches) {
            let face = s.add_face(&[r.iter().copied().map(xyz).collect()], ch)?;
            s.state.fattr.set(face as usize, material);
        }
        let edge = |s: &Store, a, b| s.find_edge(s.lookup(xyz(a))?, s.lookup(xyz(b))?);
        for &(old, a, b) in &self.flags {
            if let Some(e) = edge(s, a, b) {
                let (i, j) = (e as usize, old as usize);
                s.state.eflags.set(i, *s.state.eflags.get(j));
                s.state.elayer.set(i, *s.state.elayer.get(j));
                s.state.ecurve.set(i, *s.state.ecurve.get(j));
                ch.modified[1].push(s.key(Kind::Edge, e));
            }
        }
        for &(a, b) in &self.soft {
            if let Some(e) = edge(s, a, b) {
                let i = e as usize;
                s.state.eflags.set(i, *s.state.eflags.get(i) | SOFT);
                ch.modified[1].push(s.key(Kind::Edge, e));
            }
        }
        if !ids.is_empty() {
            for e in s.live_slots(Kind::Edge) {
                if *s.state.ecurve.get(e as usize) != -1 {
                    continue;
                }
                let [a, b] = ends(s, e).map(|v| pos(s, v));
                for (curve, &id) in self.curves.iter().zip(ids) {
                    if curve
                        .windows(2)
                        .any(|w| on_segment(a, w[0], w[1], h) && on_segment(b, w[0], w[1], h))
                    {
                        s.state.ecurve.set(e as usize, id.into());
                        ch.modified[1].push(s.key(Kind::Edge, e));
                        break;
                    }
                }
            }
        }
        // A section can weld onto a previously isolated vertex. Its position
        // survives, but the new disk incidence belongs in the change feed.
        for &key in &ch.created[1] {
            if let Some(e) = s.slot(Kind::Edge, key) {
                ch.modified[0].extend(ends(s, e).map(|v| s.key(Kind::Vertex, v)));
            }
        }
        // Removing incident faces marked the subsequently removed edges modified.
        ch.modified[1].retain(|&k| s.slot(Kind::Edge, k).is_some());
        Ok(())
    }
}
fn on_segment(q: V3, a: V3, b: V3, h: Hypot) -> bool {
    let d = b - a;
    let l = length(d, h);
    if l < 1e-4 {
        return xyz(q).map(|v| (v * 1e4).round_ties_even())
            == xyz(a).map(|v| (v * 1e4).round_ties_even());
    }
    let t = (q - a).dot(d, h) / (l * l);
    (-1e-4..=1.0001).contains(&t) && length(q - (a + d * t), h) < 1e-4
}

pub(super) fn arc(curves: &[Vec<V3>], a: V3, b: V3, h: Hypot) -> Option<Vec<V3>> {
    for r in curves {
        let (p, q) = (r[0], r[r.len() - 1]);
        if length(p - a, h) < 1e-5 && length(q - b, h) < 1e-5 {
            return Some(r.clone());
        }
        if length(p - b, h) < 1e-5 && length(q - a, h) < 1e-5 {
            return Some(r.iter().rev().copied().collect());
        }
    }
    None
}
// Triangular patch with exact boundary samples. Interior seam edges are soft;
// the fillet patch also smooths its joins to the adjacent strips.
pub(super) fn grid_cap(
    plan: &mut RoundPlan,
    arcs: [Vec<V3>; 3],
    target: V3,
    sign: f64,
    soft_rim: bool,
    h: Hypot,
    interior: impl Fn(usize, usize, usize) -> V3,
) {
    let [ab, bc, ac] = arcs;
    let n = ab.len() - 1;
    let grid: Vec<Vec<_>> = (0..=n)
        .map(|i| {
            (0..=n - i)
                .map(|j| {
                    let k = n - i - j;
                    if k == 0 {
                        ab[j]
                    } else if i == 0 {
                        bc[k]
                    } else if j == 0 {
                        ac[k]
                    } else {
                        interior(i, j, k)
                    }
                })
                .collect()
        })
        .collect();
    for i in 0..=n {
        for j in 0..=n - i {
            let p = grid[i][j];
            let k = n - i - j;
            if k > 0 {
                let a = grid[i + 1][j];
                let b = grid[i][j + 1];
                plan.patches.push(facing(vec![p, a, b], target, sign, h));
                if k > 1 {
                    plan.patches
                        .push(facing(vec![a, grid[i + 1][j + 1], b], target, sign, h));
                }
                if soft_rim || j > 0 {
                    plan.soft.push((p, a))
                }
                if soft_rim || i > 0 {
                    plan.soft.push((p, b))
                }
            }
            if i > 0 && (soft_rim || k > 0) {
                plan.soft.push((p, grid[i - 1][j + 1]))
            }
        }
    }
}

// The editor's control points (core/bevel_profiles.py); sampled in the kernel.
const CORNICE: &[(f64, f64, bool)] = &[
    (1.0, 0.0, false),
    (1.0, 0.125, false),
    (0.92, 0.16, true),
    (0.875, 0.25, false),
    (0.8, 0.25, false),
    (0.733, 0.433, true),
    (0.582, 0.522, true),
    (0.4, 0.6, true),
    (0.289, 0.727, true),
    (0.25, 0.925, false),
    (0.175, 0.925, false),
    (0.175, 1.0, false),
    (0.0, 1.0, false),
];
const CROWN: &[(f64, f64, bool)] = &[
    (1.0, 0.0, false),
    (1.0, 0.25, false),
    (0.75, 0.25, false),
    (0.75, 0.325, false),
    (0.925, 0.4, true),
    (0.975, 0.5, true),
    (0.94, 0.65, true),
    (0.85, 0.75, true),
    (0.75, 0.875, true),
    (0.7, 1.0, false),
    (0.0, 1.0, false),
];

pub(super) fn sample_profile(
    shape: &str,
    drawn: &[(f64, f64, bool)],
    n: usize,
) -> Result<Vec<(f64, f64)>> {
    use std::f64::consts::FRAC_PI_2;
    let controls = match shape {
        "steps" => {
            let k = n.div_ceil(2);
            let mut pts = vec![(1., 0.)];
            for i in 0..k {
                let x = 1. - (i + 1) as f64 / k as f64;
                pts.extend([(x, i as f64 / k as f64), (x, (i + 1) as f64 / k as f64)]);
            }
            return Ok(pts);
        }
        "supports" => {
            let k = n.saturating_sub(2).max(1);
            let mut pts = vec![(1., 0.)];
            for i in 0..=k {
                let a = FRAC_PI_2 * i as f64 / k as f64;
                pts.push((0.5 + 0.5 * a.cos(), 0.5 + 0.5 * a.sin()))
            }
            pts.push((0., 1.));
            return Ok(pts);
        }
        "cornice" => CORNICE,
        "crown" => CROWN,
        "custom" => drawn,
        _ => return Ok(vec![]),
    };
    if controls.len() < 2
        || controls.iter().any(|&(x, y, _)| {
            !(-1e-9..=1. + 1e-9).contains(&x) || !(-1e-9..=1. + 1e-9).contains(&y)
        })
        || (controls[0].0 - 1.).abs() > 1e-9
        || controls[0].1.abs() > 1e-9
        || controls.last().unwrap().0.abs() > 1e-9
        || (controls.last().unwrap().1 - 1.).abs() > 1e-9
    {
        return Err((
            "The drawn profile must run from one face to the other.",
            None,
        ));
    }
    let mut straight = 0;
    let mut total = 0.;
    for pair in controls.windows(2) {
        let [a, b] = [pair[0], pair[1]];
        if a.2 || b.2 {
            total += (a.0 - b.0).hypot(a.1 - b.1)
        } else {
            straight += 1
        }
    }
    if total == 0. {
        total = 1.
    }
    let spare = n.saturating_sub(straight);
    let mut pts = vec![(controls[0].0, controls[0].1)];
    for (i, pair) in controls.windows(2).enumerate() {
        let (a, b) = (pair[0], pair[1]);
        if !a.2 && !b.2 {
            pts.push((b.0, b.1));
            continue;
        }
        let pieces = ((spare as f64 * (a.0 - b.0).hypot(a.1 - b.1) / total).round_ties_even()
            as usize)
            .max(1);
        let prev = if i > 0 && a.2 { controls[i - 1] } else { a };
        let next = if i + 2 < controls.len() && b.2 {
            controls[i + 2]
        } else {
            b
        };
        for j in 1..=pieces {
            let t = j as f64 / pieces as f64;
            let catmull = |a: f64, b: f64, c: f64, d: f64| {
                0.5 * (2. * b
                    + (-a + c) * t
                    + (2. * a - 5. * b + 4. * c - d) * (t * t)
                    + (-a + 3. * b - 3. * c + d) * (t * t * t))
            };
            pts.push((
                catmull(prev.0, a.0, b.0, next.0),
                catmull(prev.1, a.1, b.1, next.1),
            ));
        }
    }
    Ok(pts)
}
struct Round {
    edge: u32,
    faces: [u32; 2],
    d: V3,
    w: [V3; 2],
    t: f64,
    radius: f64,
    centre: V3,
    sign: f64,
    offsets: Vec<V3>,
    // Two ends, in first-visited order (the order in which arcs get curve IDs).
    sections: Vec<(u32, Vec<V3>)>,
}
/// The planner only reads incidence. Object meshes supply an indexed neighbourhood
/// in their existing traversal order, including vertices at coincident positions.
pub(super) trait FilletMesh {
    const OBJECT: bool;
    fn point(&self, v: u32) -> V3;
    fn ends(&self, e: u32) -> [u32; 2];
    fn compensated_sum(&self) -> bool {
        false
    }
    fn other_end(&self, e: u32, v: u32) -> u32 {
        let [a, b] = self.ends(e);
        if a == v {
            b
        } else {
            a
        }
    }
    fn radial(&self, e: u32) -> &[u32];
    fn rings(&self, f: u32) -> Vec<Vec<u32>>;
    fn normal(&self, f: u32, h: Hypot) -> V3;
    fn plane_normal(&self, f: u32, h: Hypot) -> V3 {
        let n = self.normal(f, h);
        if Self::OBJECT && n.dot(n, h) > 1e-18 {
            super::geometry::normalized(n, h)
        } else {
            n
        }
    }
    fn adjacent(&self, v: u32) -> Vec<u32>;
    fn disk(&self, v: u32) -> Vec<u32>;
}
impl FilletMesh for Store {
    const OBJECT: bool = false;
    fn point(&self, v: u32) -> V3 {
        pos(self, v)
    }
    fn ends(&self, e: u32) -> [u32; 2] {
        ends(self, e)
    }
    fn radial(&self, e: u32) -> &[u32] {
        self.edge_faces(e)
    }
    fn rings(&self, f: u32) -> Vec<Vec<u32>> {
        self.face_rings(f)
    }
    fn normal(&self, f: u32, h: Hypot) -> V3 {
        self.face_normal(f, h)
    }
    fn adjacent(&self, v: u32) -> Vec<u32> {
        self.faces_around(&[v])
    }
    fn disk(&self, v: u32) -> Vec<u32> {
        self.edges_at(v).collect()
    }
}

fn profile_power(profile: f64, object: bool) -> f64 {
    if object {
        2. / (4f64.ln() / (1. / profile).ln())
    } else {
        2. * (1. / profile).ln() / 4f64.ln()
    }
}

fn inward(s: &impl FilletMesh, f: u32, [a, b]: [u32; 2], h: Hypot) -> Option<V3> {
    for ring in s.rings(f) {
        for (&i, &j) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            if (i == a && j == b) || (i == b && j == a) {
                return unit(s.normal(f, h).cross(s.point(j) - s.point(i), h), h);
            }
        }
    }
    None
}
/// Conservative plane-distance bound used to reject separated hole boxes.
pub(crate) struct HoleBoxes {
    boxes: Vec<([f64; 3], [f64; 3])>,
    world: ([f64; 3], [f64; 3]),
    margin: f64,
}
impl HoleBoxes {
    pub(crate) fn new(holes: &[Vec<V3>]) -> Self {
        let mut world = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        let boxes = holes
            .iter()
            .map(|ring| {
                let mut lo = [f64::INFINITY; 3];
                let mut hi = [f64::NEG_INFINITY; 3];
                for &p in ring {
                    let point = xyz(p);
                    for k in 0..3 {
                        lo[k] = lo[k].min(point[k]);
                        hi[k] = hi[k].max(point[k]);
                    }
                }
                for k in 0..3 {
                    world.0[k] = world.0[k].min(lo[k]);
                    world.1[k] = world.1[k].max(hi[k]);
                }
                (lo, hi)
            })
            .collect();
        let extent = world
            .0
            .into_iter()
            .chain(world.1)
            .map(f64::abs)
            .fold(0., f64::max);
        Self {
            boxes,
            world,
            margin: 1e-3 + 1e-5 * extent,
        }
    }
    pub(crate) fn gap(&self, origin: V3, normal: V3) -> f64 {
        let p = xyz(origin);
        let n = xyz(normal);
        let (mut low, mut high) = (0., 0.);
        for k in 0..3 {
            let a = (self.world.0[k] - p[k]) * n[k];
            let b = (self.world.1[k] - p[k]) * n[k];
            low += a.min(b);
            high += a.max(b);
        }
        2. * low.abs().max(high.abs()) + self.margin
    }
    pub(crate) fn near(&self, i: usize, j: usize, gap: f64) -> bool {
        let (a, b) = self.boxes[i];
        let (c, d) = self.boxes[j];
        (0..3).all(|k| c[k] <= b[k] + gap && d[k] >= a[k] - gap)
    }
}

// Mesh.add_face drops holes nested inside a larger hole. Area is measured
// in XY, while containment uses the inner hole's plane, as in topology.py.
pub(crate) fn maximal_holes(holes: &[Vec<V3>], h: Hypot) -> Vec<bool> {
    maximal_holes_with_sum(holes,h,true)
}

pub(crate) fn maximal_holes_with_sum(holes: &[Vec<V3>], h: Hypot, compensated:bool) -> Vec<bool> {
    let areas: Vec<f64> = holes
        .iter()
        .map(|r| {
            sides(r)
                .fold(0., |sum, (a, b)| sum + a.0 * b.1 - b.0 * a.1)
                .abs()
                / 2.
        })
        .collect();
    let boxes = HoleBoxes::new(holes);
    let mut keep = Vec::with_capacity(holes.len());
    for (i, inner) in holes.iter().enumerate() {
        let normal = (inner[1] - inner[0]).cross(inner[2] - inner[0], h);
        if length(normal, h) < 1e-9 {
            keep.push(true);
            continue;
        }
        let (u, v) =
            crate::store::geometry::plane_axes(crate::store::geometry::normalized(normal, h), h);
        let project = |p: V3| (p.dot(u, h), p.dot(v, h));
        let average = |axis: usize| {
            crate::store::geometry::py_sum(inner.iter().map(|&p| xyz(p)[axis]), compensated)
                / inner.len() as f64
        };
        let centre = V3(average(0), average(1), average(2));
        let gap = boxes.gap(inner[0], crate::store::geometry::normalized(normal, h));
        let mut interior = None;
        let nested = holes.iter().enumerate().any(|(j, outer)| {
            if areas[j] <= areas[i] + 1e-6 || !boxes.near(i, j, gap) {
                return false;
            }
            let poly: Vec<_> = outer.iter().map(|&p| project(p - outer[0])).collect();
            let contains = |p: V3| inside_2d(&poly, project(p - outer[0]));
            if contains(centre) {
                return true;
            }
            interior
                .get_or_insert_with(|| {
                    let points: Vec<_> = inner.iter().map(|&p| project(p - inner[0])).collect();
                    let tris = if crate::store::geometry::is_convex(&points) {
                        (1..inner.len() - 1).map(|i| [0, i, i + 1]).collect()
                    } else {
                        crate::store::geometry::earcut(&points, &[])
                    };
                    let mut best = None;
                    let mut area = 0.;
                    for tri in tris {
                        let [a, b, c] = tri.map(|k| inner[k]);
                        let cross = (b - a).cross(c - a, h);
                        let value = cross.dot(cross, h);
                        if value > area {
                            area = value;
                            best = Some((a + b + c) / 3.)
                        }
                    }
                    best
                })
                .is_some_and(contains)
        });
        keep.push(!nested);
    }
    keep
}
fn inside_2d(poly: &[(f64, f64)], (x, y): (f64, f64)) -> bool {
    let mut inside = false;
    for (&(bx, by), &(ax, ay)) in poly.iter().zip(poly.iter().cycle().skip(1)) {
        if (ay > y) != (by > y) && x < (bx - ax) * (y - ay) / (by - ay) + ax {
            inside = !inside
        }
    }
    inside
}
fn solid(s: &impl FilletMesh, f: u32, p: V3, h: Hypot) -> bool {
    let rings = s.rings(f);
    let Some(&anchor) = rings.first().and_then(|ring| ring.first()) else {
        return false;
    };
    let o = s.point(anchor);
    let (u, v) = crate::store::geometry::plane_axes(s.normal(f, h), h);
    let project = |p: V3| ((p - o).dot(u, h), (p - o).dot(v, h));
    let inside = |r: &Vec<u32>| {
        let polygon: Vec<_> = r.iter().map(|&v| project(s.point(v))).collect();
        inside_2d(&polygon, project(p))
    };
    inside(&rings[0]) && !rings.iter().skip(1).any(inside)
}
impl Round {
    fn end(&self, vertex: u32) -> &Vec<V3> {
        &self
            .sections
            .iter()
            .find(|(v, _)| *v == vertex)
            .expect("both ends resolved before building strips")
            .1
    }

    fn section<M: FilletMesh>(
        s: &M,
        e: u32,
        size: f64,
        n: usize,
        profile: f64,
        measure: &str,
        outline: &[(f64, f64)],
        mirror: bool,
        h: Hypot,
    ) -> Result<Self> {
        let [f, g] = s.radial(e) else {
            return Err((
                "Only an edge between exactly two faces can be rounded.",
                None,
            ));
        };
        let vs = s.ends(e);
        let d = unit(s.point(vs[1]) - s.point(vs[0]), h)
            .ok_or(("A zero-length edge cannot be rounded.", None))?;
        let read =
            |f| inward(s, f, vs, h).ok_or(("Could not read the faces around the edge.", None));
        let (a, b) = (read(*f)?, read(*g)?);
        let angle = a.dot(b, h).clamp(-1., 1.).acos();
        let flat = ("The faces meet too flat or too sharp to round.", None);
        if !(2f64.to_radians()..=178f64.to_radians()).contains(&angle) {
            return Err(flat);
        }
        let half = angle / 2.;
        let t = match measure {
            "offset" => size,
            "width" => size / (2. * half.sin()),
            "depth" => size / half.cos(),
            _ => size / half.tan(),
        };
        let radius = t * half.tan();
        let t = radius / half.tan();
        let mut centre = (a + b) * (radius / angle.sin());
        let ta = a * t;
        let tb = b * t;
        let sign = if s.normal(*f, h).dot(ta - centre, h) >= 0. {
            1.
        } else {
            -1.
        };
        let offsets = if !outline.is_empty() || (profile - 0.5).abs() >= 1e-9 {
            centre = ta + tb;
            let point = |(x, y)| centre + (ta - centre) * x + (tb - centre) * y;
            if outline.is_empty() {
                let pow = profile_power(profile, M::OBJECT);
                (0..=n)
                    .map(|k| {
                        let theta = std::f64::consts::FRAC_PI_2 * k as f64 / n as f64;
                        point(if k == 0 {
                            (1., 0.)
                        } else if k == n {
                            (0., 1.)
                        } else {
                            (theta.cos().powf(pow), theta.sin().powf(pow))
                        })
                    })
                    .collect()
            } else if mirror && s.normal(*g, h).2 > s.normal(*f, h).2 + 1e-6 {
                outline.iter().rev().map(|&(x, y)| point((y, x))).collect()
            } else {
                outline.iter().copied().map(point).collect()
            }
        } else {
            let u = unit(ta - centre, h).ok_or(flat)?;
            let rb = tb - centre;
            let v = unit(rb - u * rb.dot(u, h), h).ok_or(flat)?;
            let phi = u.dot(unit(rb, h).ok_or(flat)?, h).clamp(-1., 1.).acos();
            (0..=n)
                .map(|i| {
                    let x = phi * i as f64 / n as f64;
                    centre + (u * x.cos() + v * x.sin()) * radius
                })
                .collect()
        };
        Ok(Self {
            edge: e,
            faces: [*f, *g],
            d,
            w: [a, b],
            t,
            radius,
            centre,
            sign,
            offsets,
            sections: Vec::with_capacity(2),
        })
    }
    fn cut(&self, p: V3, q: V3, m: V3, h: Hypot) -> Option<Vec<V3>> {
        let den = self.d.dot(m, h);
        if den.abs() < 0.05 {
            return None;
        }
        Some(
            self.offsets
                .iter()
                .map(|&off| {
                    let base = p + off;
                    base + self.d * ((q - base).dot(m, h) / den)
                })
                .collect(),
        )
    }
}

pub(super) fn plan<M: FilletMesh>(
    s: &M,
    edges: Vec<u32>,
    size: f64,
    n: usize,
    profile: f64,
    measure: &str,
    cut_corner: bool,
    outline: &[(f64, f64)],
    mirror: bool,
    steps: bool,
    h: Hypot,
) -> Result<RoundPlan> {
    if edges.is_empty() {
        return Err(("Pick an edge to round.", None));
    }
    check(size, profile)?;
    let mut plan = RoundPlan::default();
    let mut rounds = Vec::new();
    let mut groups: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    let mut vertex_order = Vec::new();
    for e in edges {
        if !plan.edges.insert(e) {
            continue;
        }
        let r = Round::section(s, e, size, n, profile, measure, outline, mirror, h)?;
        if rounds.is_empty() {
            plan.materials.extend(r.faces)
        }
        for v in s.ends(e) {
            let incident = groups.entry(v).or_default();
            if incident.is_empty() {
                vertex_order.push(v);
            }
            incident.push(rounds.len())
        }
        rounds.push(r);
    }
    // Resolve both tangent ends and the incident face replacements before editing.
    let mut replacements: BTreeMap<(u32, u32), Vec<V3>> = BTreeMap::new();
    let mut patches = Vec::new();
    for &v in &vertex_order {
        let group = &groups[&v];
        let p = s.point(v);
        let here = s.adjacent(v);
        let allowed: BTreeSet<_> = group.iter().flat_map(|&i| rounds[i].faces).collect();
        let caps: Vec<_> = here.iter().filter(|f| !allowed.contains(f)).collect();
        let err = |msg| at(msg, p);
        if group.len() > 3 {
            return Err(err("More than three rounded edges meet at {p}."));
        }
        if caps.len() > usize::from(group.len() == 1) {
            return Err(err("The corner at {p} has too many faces to round."));
        }
        let first = &rounds[group[0]];
        let mut sections = Vec::new();
        match group.len() {
            1 => {
                let (q, m) = if let Some(&&f) = caps.first() {
                    let anchor = s.rings(f)[0]
                        .first()
                        .copied()
                        .ok_or(("Could not read the faces around the edge.", None))?;
                    (s.point(anchor), s.plane_normal(f, h))
                } else {
                    (p, first.d)
                };
                let sec = first
                    .cut(p, q, m, h)
                    .ok_or(err("The end face at {p} is too oblique to the edge."))?;
                if let Some(&&f) = caps.first() {
                    replacements.insert((f, v), sec.clone());
                }
                sections.push(sec);
            }
            2 => {
                let rb = &rounds[group[1]];
                let dir = |r: &Round| unit(s.point(s.other_end(r.edge, v)) - p, h).unwrap();
                let m = unit(dir(first) - dir(rb), h)
                    .ok_or(err("The two edges at {p} fold back on each other."))?;
                let sec = first
                    .cut(p, p, m, h)
                    .ok_or(err("The edges at {p} meet too sharply to round."))?;
                let sec_b = if first.faces[0] == rb.faces[0] || first.faces[1] == rb.faces[1] {
                    sec.clone()
                } else if first.faces[0] == rb.faces[1] || first.faces[1] == rb.faces[0] {
                    sec.iter().rev().copied().collect()
                } else {
                    return Err(err("The edges at {p} share no face."));
                };
                sections.extend([sec, sec_b]);
            }
            _ => {
                for &i in group {
                    let r = &rounds[i];
                    let mut along = [0.; 2];
                    for side in 0..2 {
                        let f = r.faces[side];
                        let mut others = group
                            .iter()
                            .filter(|&&j| j != i && rounds[j].faces.contains(&f));
                        let simple = err("The corner at {p} is not a simple corner.");
                        let q = &rounds[*others.next().ok_or(simple)?];
                        if others.next().is_some() {
                            return Err(simple);
                        }
                        let normal = s.plane_normal(f, h);
                        let a = p + r.w[side] * r.t;
                        let b = p + q.w[usize::from(q.faces[1] == f)] * q.t;
                        let den = r.d.cross(q.d, h).dot(normal, h);
                        if den.abs() < 1e-12 {
                            return Err(simple);
                        }
                        let hit = a + r.d * ((b - a).cross(q.d, h).dot(normal, h) / den);
                        along[side] = (hit - p).dot(r.d, h);
                    }
                    if (along[0] - along[1]).abs() > 1e-3 * r.t.max(1.) {
                        return Err(err("The corner at {p} is too skewed to round."));
                    }
                    if along[0].abs() > length(s.point(s.other_end(r.edge, v)) - p, h) * 0.5 + 1e-9
                    {
                        return Err(err("The radius is too large for the edge at {p}."));
                    }
                    let base = p + r.d * along[0];
                    sections.push(r.offsets.iter().map(|&o| base + o).collect());
                }
                patches.push((v, group.clone()));
            }
        }
        for (&i, sec) in group.iter().zip(sections) {
            let r = &mut rounds[i];
            let [a, b] = s.ends(r.edge);
            let mid = (s.point(a) + s.point(b)) * 0.5;
            for side in 0..2 {
                let f = r.faces[side];
                let q = sec[if side == 0 { 0 } else { sec.len() - 1 }];
                if !solid(s, f, q + ((mid - q) - r.w[side] * r.t) * 1e-3, h) {
                    return Err(err("The radius is too large for the face at {p}."));
                }
                if let Some(cur) = replacements.get(&(f, v)) {
                    if !cur.iter().any(|&x| length(x - q, h) < 1e-5) {
                        return Err(err("Conflicting cuts at {p}."));
                    }
                } else {
                    replacements.insert((f, v), vec![q]);
                }
            }
            r.sections.push((v, sec));
        }
    }
    // Splice the cut sequences into each outer and hole ring in winding order.
    let touched: BTreeSet<_> = groups.keys().flat_map(|&v| s.adjacent(v)).collect();
    for f in touched {
        plan.rebuild(s, f, h, |ring, i| {
            let v = ring[i];
            if !groups.contains_key(&v) {
                return Ok(vec![s.point(v)]);
            }
            let mut seq = replacements
                .get(&(f, v))
                .ok_or(("A face would collapse — the radius is too large.", None))?
                .clone();
            if seq.len() > 1 {
                let prev = s.point(ring[(i + ring.len() - 1) % ring.len()]);
                let next = s.point(ring[(i + 1) % ring.len()]);
                let (a, b) = (seq[0], seq[seq.len() - 1]);
                let cost = |a, b| length(prev - a, h) + length(next - b, h);
                if cost(b, a) < cost(a, b) {
                    seq.reverse()
                }
            }
            Ok(seq)
        })?;
        let ring = &plan.rebuilt.last().unwrap().1[0];
        let normal = s.plane_normal(f, h);
        let o = s.point(s.rings(f)[0][0]);
        let span = ring.iter().map(|&p| length(p - o, h)).fold(1., f64::max);
        for &p in ring {
            if (p - o).dot(normal, h).abs() > 1e-4 * span {
                return Err(at("Rounding would bend the face at {p}.", p));
            }
        }
    }
    // Extrude each section into strips, then close the three-way junctions.
    for r in &rounds {
        let [va, vb] = s.ends(r.edge);
        let a = r.end(va);
        let b = r.end(vb);
        let last = a.len() - 1;
        if (b[0] - a[0]).dot(r.d, h) <= 1e-6 {
            return Err(at(
                "The radius is too large for the edge at {p}.",
                s.point(va),
            ));
        }
        let centre = (a[0] + b[0]) * 0.5 - r.w[0] * r.t + r.centre;
        let chord = [a[0], a[last], b[last], b[0]];
        let flip = newell(&chord).dot(mean(&chord) - centre, h) * r.sign < 0.;
        for k in 0..last {
            let mut quad = vec![a[k], a[k + 1], b[k + 1], b[k]];
            if flip {
                quad.reverse()
            }
            plan.strips.push(quad)
        }
        if outline.is_empty() {
            if M::OBJECT {
                plan.soft
                    .extend((1..last).chain([0, last]).map(|i| (a[i], b[i])));
            } else {
                plan.soft.extend(a.iter().copied().zip(b.iter().copied()));
            }
        }
        for (v, sec) in &r.sections {
            let degree = groups[v].len();
            if degree == 1 || (degree == 3 && cut_corner && outline.is_empty()) {
                plan.curves.push(sec.clone())
            } else if outline.is_empty() {
                plan.soft.extend(sec.windows(2).map(|w| (w[0], w[1])))
            }
        }
    }
    for (v, g) in patches {
        let rs = [&rounds[g[0]], &rounds[g[1]], &rounds[g[2]]];
        corner(&mut plan, s, v, rs, profile, cut_corner, outline, steps, h)?;
    }
    if !outline.is_empty() {
        smooth(&mut plan, h, M::OBJECT)
    }
    for &v in groups.keys() {
        plan.edges
            .extend(s.disk(v).into_iter().filter(|&e| !s.radial(e).is_empty()))
    }
    Ok(plan)
}

type PointKey = [i64; 3];
type Side = (PointKey, PointKey);
fn key(p: V3) -> PointKey {
    xyz(p).map(|v| (v * 1e5).round_ties_even() as i64)
}
fn side(a: V3, b: V3) -> Side {
    let (a, b) = (key(a), key(b));
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}
fn sides(r: &[V3]) -> impl Iterator<Item = (V3, V3)> + '_ {
    r.iter().copied().zip(r.iter().copied().cycle().skip(1))
}
fn smooth(plan: &mut RoundPlan, h: Hypot, insertion_order: bool) {
    let mut normals: BTreeMap<Side, Vec<(V3, V3, V3)>> = BTreeMap::new();
    let mut order = Vec::new();
    for ring in plan
        .rebuilt
        .iter()
        .map(|(_, r)| &r[0])
        .chain(&plan.strips)
        .chain(&plan.patches)
    {
        if let Some(n) = unit(newell(ring), h) {
            for (a, b) in sides(ring) {
                let key = side(a, b);
                let entries = normals.entry(key).or_default();
                if insertion_order && entries.is_empty() {
                    order.push(key);
                }
                entries.push((n, a, b));
            }
        }
    }
    if !insertion_order {
        order.extend(normals.keys().copied());
    }
    for key in order {
        let ns = &normals[&key];
        if let [(n, a, b), (m, _, _)] = ns.as_slice() {
            if n.dot(*m, h) > 30f64.to_radians().cos() {
                plan.soft.push((*a, *b))
            }
        }
    }
}

fn cut_faces(
    rs: [&Round; 3],
    secs: &[Vec<V3>],
    corners: [V3; 3],
    out: V3,
    tip: V3,
    h: Hypot,
    object_order: bool,
) -> Option<Vec<Vec<V3>>> {
    // Corner i lies on sections i and (i+1)%3.
    let entries = if object_order {
        // Each incident strip visits its first corner and then its last. The
        // first such point also supplies the common bottom on a square box.
        let mut pts: Vec<(V3, Vec<usize>)> = Vec::new();
        for (i, sec) in secs.iter().enumerate() {
            for p in [sec[0], *sec.last()?] {
                if let Some((_, sides)) = pts.iter_mut().find(|(q, _)| length(*q - p, h) < 1e-5) {
                    sides.push(i);
                } else {
                    pts.push((p, vec![i]));
                }
            }
        }
        if pts.len() != 3 {
            return None;
        }
        pts.into_iter()
            .map(|(p, sides)| match sides.as_slice() {
                &[a, b] => Some((p, a, b)),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?
    } else {
        (0..3).map(|i| (corners[i], i, (i + 1) % 3)).collect()
    };
    let mut bottoms = Vec::new();
    for &(p, i, j) in &entries {
        let (a, b) = (rs[i], rs[j]);
        let mut d = unit(a.d.cross(b.d, h), h)?;
        if d.dot(out, h) > 0. {
            d = d * (-1.)
        }
        let depth = (length(a.centre, h) + length(b.centre, h)) / 2. / 2f64.sqrt();
        bottoms.push(p + d * depth);
    }
    let centre = bottoms.iter().any(|&p| length(p - bottoms[0], h) >= 1e-5);
    let bottom = |p| {
        entries
            .iter()
            .position(|&(c, _, _)| length(c - p, h) < 1e-5)
            .map(|i| bottoms[i])
    };
    let mut faces = Vec::new();
    for sec in secs {
        let mut ring = sec.clone();
        if centre {
            ring.extend([bottom(sec[sec.len() - 1])?, bottom(sec[0])?])
        } else {
            ring.push(bottoms[0])
        }
        faces.push(facing(ring, tip, rs[0].sign, h))
    }
    if centre {
        faces.push(facing(bottoms, tip, rs[0].sign, h))
    }
    Some(faces)
}

fn corner<M: FilletMesh>(
    plan: &mut RoundPlan,
    s: &M,
    v: u32,
    rs: [&Round; 3],
    profile: f64,
    cut: bool,
    outline: &[(f64, f64)],
    steps: bool,
    h: Hypot,
) -> Result<()> {
    let secs = rs.map(|r| r.end(v).clone());
    let tip = s.point(v);
    let err = at("Could not close the corner at {p}.", tip);
    let meet = |a: usize, b: usize| {
        let (x, y) = (&secs[a], &secs[b]);
        [x[0], x[x.len() - 1]]
            .into_iter()
            .find(|&p| {
                [y[0], y[y.len() - 1]]
                    .iter()
                    .any(|&q| length(p - q, h) < 1e-5)
            })
            .ok_or(err)
    };
    let corners = [meet(0, 1)?, meet(1, 2)?, meet(2, 0)?];
    let out = s.adjacent(v).iter().fold(ZERO, |a, &f| a + s.normal(f, h));
    let cutoff = || cut_faces(rs, &secs, corners, out, tip, h, M::OBJECT);
    if cut {
        if let Some(faces) = cutoff() {
            if outline.is_empty() || closes(&faces, &secs, &BTreeMap::new()) {
                plan.patches.extend(faces);
                return Ok(());
            }
        }
        if outline.is_empty() {
            return Err(err);
        }
    }
    let centre = rs
        .iter()
        .zip(&secs)
        .fold(ZERO, |p, (r, sec)| p + (sec[0] - r.w[0] * r.t + r.centre))
        / 3.;
    if !outline.is_empty() {
        let opposite = [corners[1], corners[2], corners[0]];
        let faces = if steps {
            Some(staircase(
                opposite,
                centre,
                (outline.len() - 1) / 2,
                rs[0].sign,
                h,
            ))
        } else {
            product(opposite, centre, &secs, outline, &plan.strips, h)
        };
        if let Some(faces) = faces {
            if closes(&faces, &secs, &BTreeMap::new()) {
                plan.patches.extend(faces);
                return Ok(());
            }
            let (faces, splits) = split_sides(&faces, &secs, h);
            if closes(&faces, &secs, &splits) {
                for ring in &mut plan.strips {
                    *ring = insert_splits(ring, &splits)
                }
                plan.patches.extend(faces);
                return Ok(());
            }
        }
    }
    let ab = arc(&secs, corners[0], corners[1], h).ok_or(err)?;
    let bc = arc(&secs, corners[1], corners[2], h).ok_or(err)?;
    let ac = arc(&secs, corners[0], corners[2], h).ok_or(err)?;
    let n = ab.len() - 1;
    let dirs = corners.map(|p| unit(p - centre, h));
    let [Some(a), Some(b), Some(c)] = dirs else {
        return Err(err);
    };
    let radius = super::geometry::py_sum(rs.iter().map(|r| r.radius), s.compensated_sum()) / 3.;
    let power = profile_power(profile, M::OBJECT);
    let mut patch = RoundPlan::default();
    grid_cap(
        &mut patch,
        [ab.clone(), bc.clone(), ac.clone()],
        centre,
        -rs[0].sign,
        true,
        h,
        |i, j, k| {
            if !outline.is_empty() {
                let row = |p, q, t| p + (q - p) * t;
                let t = |a: usize, b: usize| a as f64 / b as f64;
                return (row(ac[n - i], ab[n - i], t(j, n - i))
                    + row(bc[n - j], ab[j], t(i, n - j))
                    + row(bc[k], ac[k], t(i, n - k)))
                    / 3.;
            }
            let [wa, wb, wc] =
                [i, j, k].map(|l| (l as f64 * std::f64::consts::FRAC_PI_2 / n as f64).sin());
            if (profile - 0.5).abs() < 1e-9 {
                centre + unit(a * wa + b * wb + c * wc, h).unwrap_or(ZERO) * radius
            } else {
                let norm = (wa * wa + wb * wb + wc * wc).sqrt();
                centre
                    + (corners[0] - centre) * (wa / norm).powf(power)
                    + (corners[1] - centre) * (wb / norm).powf(power)
                    + (corners[2] - centre) * (wc / norm).powf(power)
            }
        },
    );
    if outline.is_empty() {
        plan.soft.extend(patch.soft)
    } else {
        wind_like(&mut patch.patches, &plan.strips);
        if !closes(&patch.patches, &secs, &BTreeMap::new()) {
            patch.patches = cutoff().ok_or(err)?
        }
    }
    plan.patches.extend(patch.patches);
    Ok(())
}
fn wind_like(faces: &mut [Vec<V3>], strips: &[Vec<V3>]) {
    let directed: BTreeSet<_> = strips
        .iter()
        .flat_map(|r| sides(r).map(|(a, b)| (key(a), key(b))))
        .collect();
    let flip = faces.iter().flat_map(|r| sides(r)).find_map(|(a, b)| {
        if directed.contains(&(key(a), key(b))) {
            Some(true)
        } else if directed.contains(&(key(b), key(a))) {
            Some(false)
        } else {
            None
        }
    });
    if flip == Some(true) {
        for r in faces {
            r.reverse()
        }
    }
}
type Splits = BTreeMap<Side, Vec<V3>>;
fn closes(faces: &[Vec<V3>], secs: &[Vec<V3>], splits: &Splits) -> bool {
    let mut counts = BTreeMap::new();
    let mut uses = BTreeMap::new();
    for r in faces {
        for (a, b) in sides(r) {
            if key(a) == key(b) {
                return false;
            }
            *counts.entry(side(a, b)).or_insert(0) += 1
        }
    }
    for r in secs {
        for w in r.windows(2) {
            let mut a = w[0];
            for &b in splits
                .get(&(key(a), key(w[1])))
                .into_iter()
                .flatten()
                .chain(std::iter::once(&w[1]))
            {
                if key(a) != key(b) {
                    *uses.entry(side(a, b)).or_insert(0) += 1
                }
                a = b;
            }
        }
    }
    counts
        .iter()
        .all(|(k, &n)| n <= 2 && (n == 1) == (uses.get(k) == Some(&1)))
        && uses.iter().all(|(k, &n)| {
            if n == 1 {
                counts.get(k) == Some(&1)
            } else {
                !counts.contains_key(k)
            }
        })
}
fn insert_splits(ring: &[V3], splits: &Splits) -> Vec<V3> {
    let mut out = Vec::new();
    for (a, b) in sides(ring) {
        out.push(a);
        if let Some(mid) = splits.get(&(key(a), key(b))) {
            out.extend(mid)
        }
    }
    out
}
fn split_sides(faces: &[Vec<V3>], secs: &[Vec<V3>], h: Hypot) -> (Vec<Vec<V3>>, Splits) {
    let mut pts = BTreeMap::new();
    for &p in faces.iter().flatten() {
        pts.entry(key(p)).or_insert(p);
    }
    let inner = |a: V3, b: V3| {
        let d = b - a;
        let den = d.dot(d, h);
        let mut hits = Vec::new();
        if den >= 1e-18 {
            for &p in pts.values() {
                let t = (p - a).dot(d, h) / den;
                if t > 1e-6 && t < 1. - 1e-6 && length(a + d * t - p, h) < 1e-7 {
                    hits.push((t, p))
                }
            }
        }
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        hits.into_iter().map(|(_, p)| p).collect::<Vec<_>>()
    };
    let faces = faces
        .iter()
        .map(|r| {
            let mut ring = Vec::new();
            for (a, b) in sides(r) {
                ring.push(a);
                ring.extend(inner(a, b))
            }
            ring
        })
        .collect();
    let mut splits = Splits::new();
    for r in secs {
        for w in r.windows(2) {
            let mut mid = inner(w[0], w[1]);
            if !mid.is_empty() {
                splits.insert((key(w[0]), key(w[1])), mid.clone());
                mid.reverse();
                splits.insert((key(w[1]), key(w[0])), mid);
            }
        }
    }
    (faces, splits)
}
fn product(
    c: [V3; 3],
    o: V3,
    secs: &[Vec<V3>],
    outline: &[(f64, f64)],
    strips: &[Vec<V3>],
    h: Hypot,
) -> Option<Vec<Vec<V3>>> {
    let m = outline.len() - 1;
    let mirrored: Vec<_> = outline.iter().rev().map(|&(x, y)| (y, x)).collect();
    let matches = |sec: &[V3], r: &[V3]| {
        sec.iter().zip(r).all(|(&a, &b)| length(a - b, h) < 1e-5)
            || sec
                .iter()
                .rev()
                .zip(r)
                .all(|(&a, &b)| length(a - b, h) < 1e-5)
    };
    for read in [outline, &mirrored] {
        for pole in 0..3 {
            let sides: Vec<_> = (0..3).filter(|&i| i != pole).collect();
            for (ix, iy) in [(sides[0], sides[1]), (sides[1], sides[0])] {
                let at = |u: usize, v: usize| {
                    let ((x, y), (a, b)) = (read[u], read[v]);
                    o + (c[ix] - o) * (x * a) + (c[iy] - o) * (x * b) + (c[pole] - o) * y
                };
                let row = |axis| {
                    (0..=m)
                        .map(|i| match axis {
                            0 => at(0, i),
                            1 => at(i, 0),
                            _ => at(i, m),
                        })
                        .collect::<Vec<_>>()
                };
                if !matches(&secs[pole], &row(0))
                    || !matches(&secs[iy], &row(1))
                    || !matches(&secs[ix], &row(2))
                {
                    continue;
                }
                let mut faces = Vec::new();
                for u in 0..m {
                    for v in 0..m {
                        let mut pts = Vec::new();
                        for p in [at(u, v), at(u + 1, v), at(u + 1, v + 1), at(u, v + 1)] {
                            if !pts.iter().any(|&q| length(p - q, h) < 1e-5) {
                                pts.push(p)
                            }
                        }
                        if pts.len() < 3 {
                            continue;
                        }
                        if pts.len() == 4
                            && !unit(newell(&pts), h).is_some_and(|n| {
                                pts.iter().all(|&p| (p - pts[0]).dot(n, h).abs() < 1e-7)
                            })
                        {
                            for tri in [vec![pts[0], pts[1], pts[2]], vec![pts[0], pts[2], pts[3]]]
                            {
                                if length(newell(&tri), h) > 1e-12 {
                                    faces.push(tri)
                                }
                            }
                        } else if length(newell(&pts), h) > 1e-12 {
                            faces.push(pts)
                        }
                    }
                }
                wind_like(&mut faces, strips);
                return Some(faces);
            }
        }
    }
    None
}
fn staircase(c: [V3; 3], o: V3, k: usize, sign: f64, h: Hypot) -> Vec<Vec<V3>> {
    let mut faces = Vec::new();
    let at = |q: [f64; 3]| {
        o + (c[0] - o) * (q[0] / k as f64)
            + (c[1] - o) * (q[1] / k as f64)
            + (c[2] - o) * (q[2] / k as f64)
    };
    let kept = |[a, b, c]: [usize; 3]| a + b + 1 < k && a + c + 1 < k && b + c + 1 < k;
    for a in 0..k {
        for b in 0..k {
            for c in 0..k {
                let cell = [a, b, c];
                if !kept(cell) {
                    continue;
                }
                for axis in 0..3 {
                    let mut next = cell;
                    next[axis] += 1;
                    if kept(next) {
                        continue;
                    }
                    let dims: Vec<_> = (0..3).filter(|&i| i != axis).collect();
                    let ring = [(0, 0), (1, 0), (1, 1), (0, 1)].map(|(u, v)| {
                        let mut q = next;
                        q[dims[0]] += u;
                        q[dims[1]] += v;
                        at(q.map(|n| n as f64))
                    });
                    faces.push(facing(
                        ring.to_vec(),
                        at(next.map(|n| n as f64 + 0.5)),
                        sign,
                        h,
                    ))
                }
            }
        }
    }
    faces
}
