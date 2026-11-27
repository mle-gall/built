// SPDX-License-Identifier: GPL-3.0-or-later
//! Trimming with affine pcurves in unwrapped parameter space. Uses identify
//! edges independently of their endpoints, including both uses of a seam.
use super::*;
use glam::DVec2;
use std::collections::{BTreeMap, HashMap};

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub(super) struct Edge {
    id: u64,
    curve: u32,
    vertices: [u64; 2],
    interval: [f64; 2],
}
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
struct Coedge {
    edge: u64,
    reversed: bool,
    uv: [DVec2; 2],
    tolerance: f64,
}
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub(super) struct Face {
    id: u64,
    surface: u32,
    reversed: bool,
    loops: Vec<Vec<Coedge>>,
}
#[derive(Deserialize, Serialize, Clone, Debug, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct Topology {
    pub tolerance: f64,
    vertices: BTreeMap<u64, DVec3>,
    edges: Vec<Edge>,
    faces: Vec<Face>,
}

impl Curve {
    fn acceleration(&self, dt: f64) -> Result<f64> {
        Ok(match self {
            Self::Line { .. } => 0.,
            Self::Circle { radius, .. } => radius * dt * dt,
            Self::Ellipse { major, .. } => major * dt * dt,
            Self::Spline { axis, controls } => {
                spline::bounds(controls, &[(axis, dt)])?.acceleration
            }
        })
    }
}
impl Surface {
    /// Upper bound on the norm of the second derivative along a UV segment,
    /// parameterized from 0 to 1. The cone's radial term varies linearly.
    fn acceleration(&self, curves: &[Curve], a: DVec2, b: DVec2) -> Result<f64> {
        let d = (b - a).abs();
        Ok(match self {
            Self::Plane { .. } => 0.,
            Self::Cylinder { radius, .. } => radius * d.x * d.x,
            Self::Sphere { radius, .. } => radius * (d.x + d.y).powi(2),
            Self::Torus { major, minor, .. } => major * d.x * d.x + minor * (d.x + d.y).powi(2),
            Self::Cone {
                radius, half_angle, ..
            } => {
                let s = half_angle.sin();
                let r = (radius + a.y * s).abs().max((radius + b.y * s).abs());
                r * d.x * d.x + 2. * s.abs() * d.x * d.y
            }
            Self::Spline { u, v, controls, .. } => {
                spline::bounds(controls, &[(u, d.x), (v, d.y)])?.acceleration
            }
            Self::Extrusion { generator, .. } => curves[*generator as usize].acceleration(d.x)?,
            Self::Revolution {
                generator, origin, ..
            } => {
                let curve = &curves[*generator as usize];
                let (speed, radius) = match curve {
                    Curve::Line { .. } => (
                        d.x,
                        curve
                            .point(a.x)?
                            .distance(*origin)
                            .max(curve.point(b.x)?.distance(*origin)),
                    ),
                    Curve::Circle { frame, radius } => {
                        (radius * d.x, frame.origin.distance(*origin) + radius)
                    }
                    Curve::Ellipse { frame, major, .. } => {
                        (major * d.x, frame.origin.distance(*origin) + major)
                    }
                    Curve::Spline { axis, controls } => (
                        spline::bounds(controls, &[(axis, d.x)])?.speed,
                        controls
                            .iter()
                            .map(|p| p.truncate().distance(*origin))
                            .fold(0., f64::max),
                    ),
                };
                curve.acceleration(d.x)? + 2. * speed * d.y + radius * d.y * d.y
            }
        })
    }
}

fn lerp(t: [f64; 2], s: f64) -> f64 {
    if s == 1. {
        t[1]
    } else {
        t[0] + s * (t[1] - t[0])
    }
}
fn divisions(bound: f64, tol: f64) -> Result<usize> {
    let steps = (bound / tol).sqrt().ceil();
    check(
        steps.is_finite() && steps <= 65536.,
        "exact sampling budget exceeded",
    )?;
    Ok((steps as usize).max(1))
}

impl Topology {
    fn certify(&self, rows: &Rows, surface: &Surface, c: &Coedge, e: &Edge) -> Result<()> {
        let curve = &rows.curves[e.curve as usize];
        check(
            c.tolerance > 0. && c.tolerance <= self.tolerance && c.uv.iter().all(|p| p.is_finite()),
            "invalid coedge tolerance or UV line",
        )?;
        let [a, b] = c.uv;
        check(a != b, "zero-length UV use")?;
        let accel = surface.acceleration(&rows.curves, a, b)?
            + curve.acceleration(e.interval[1] - e.interval[0])?;
        // A twice differentiable vector function deviates from its
        // interpolating chord by at most sup|f''| h²/8. Apply to the
        // difference of the 3D edge and lifted pcurve on every interval.
        let n = divisions(accel / 4., c.tolerance)?;
        let allowance = c.tolerance - accel / (8. * (n as f64).powi(2));
        for i in 0..=n {
            let t = i as f64 / n as f64;
            let uv = a.lerp(b, t);
            let p = surface.point(&rows.curves, uv.x, uv.y)?;
            let q = curve.point(lerp(e.interval, if c.reversed { 1. - t } else { t }))?;
            check(
                p.distance(q) <= allowance,
                "edge and lifted pcurve disagree",
            )?;
        }
        Ok(())
    }

    fn edge_index(&self) -> HashMap<u64, &Edge> {
        self.edges.iter().map(|e| (e.id, e)).collect()
    }

    pub fn validate(&self, rows: &Rows) -> Result<()> {
        check(
            self.tolerance.is_finite() && self.tolerance > 0.,
            "invalid model tolerance",
        )?;
        check(
            self.vertices.values().all(|p| p.is_finite()),
            "nonfinite exact vertex",
        )?;
        let index = self.edge_index();
        check(index.len() == self.edges.len(), "duplicate exact edge id")?;
        for e in &self.edges {
            let curve = rows
                .curves
                .get(e.curve as usize)
                .ok_or("missing edge curve")?;
            check(
                e.interval.iter().all(|t| t.is_finite()) && e.interval[1] > e.interval[0],
                "invalid edge interval",
            )?;
            for (&id, &t) in e.vertices.iter().zip(&e.interval) {
                let v = self.vertices.get(&id).ok_or("missing exact vertex")?;
                check(
                    v.distance(curve.point(t)?) <= self.tolerance,
                    "edge endpoint disagrees with vertex",
                )?;
            }
        }
        let mut ids = std::collections::HashSet::new();
        for f in &self.faces {
            check(ids.insert(f.id), "duplicate exact face id")?;
            let s = rows
                .surfaces
                .get(f.surface as usize)
                .ok_or("missing face support")?;
            check(!f.loops.is_empty(), "missing outer loop")?;
            for ring in &f.loops {
                check(ring.len() >= 3, "insufficient UV corners")?;
                for (i, c) in ring.iter().enumerate() {
                    let next = &ring[(i + 1) % ring.len()];
                    let e = index.get(&c.edge).ok_or("missing coedge edge")?;
                    let ne = index.get(&next.edge).ok_or("missing coedge edge")?;
                    check(
                        e.vertices[usize::from(!c.reversed)]
                            == ne.vertices[usize::from(next.reversed)]
                            && c.uv[1] == next.uv[0],
                        "open trim loop",
                    )?;
                    self.certify(rows, s, c, e)?;
                }
            }
            check_loops(&f.loops)?;
        }
        Ok(())
    }
}

fn orient(a: DVec2, b: DVec2, p: DVec2) -> f64 {
    (b - a).perp_dot(p - a)
}
fn on_segment(a: DVec2, b: DVec2, p: DVec2) -> bool {
    orient(a, b, p) == 0. && a.min(b).cmple(p).all() && p.cmple(a.max(b)).all()
}
fn crosses(a: DVec2, b: DVec2, c: DVec2, d: DVec2) -> bool {
    let (x, y, z, w) = (
        orient(a, b, c),
        orient(a, b, d),
        orient(c, d, a),
        orient(c, d, b),
    );
    (x * y < 0. && z * w < 0.)
        || on_segment(a, b, c)
        || on_segment(a, b, d)
        || on_segment(c, d, a)
        || on_segment(c, d, b)
}
fn contains(ring: &[Coedge], p: DVec2) -> bool {
    ring.iter()
        .filter(|c| {
            let [a, b] = c.uv;
            (a.y > p.y) != (b.y > p.y) && ((b.y > a.y) == (orient(a, b, p) > 0.))
        })
        .count()
        % 2
        == 1
}
fn check_loops(loops: &[Vec<Coedge>]) -> Result<()> {
    for (k, ring) in loops.iter().enumerate() {
        let p = ring[0].uv[0];
        let area: f64 = ring.iter().map(|c| orient(p, c.uv[0], c.uv[1])).sum();
        check(area.is_finite() && area != 0., "degenerate trim loop")?;
        for (i, c) in ring.iter().enumerate() {
            for (l, other) in loops[..=k].iter().enumerate() {
                for (j, s) in other.iter().enumerate() {
                    if k == l && (j >= i || j + 1 == i || (j == 0 && i + 1 == ring.len())) {
                        continue;
                    }
                    check(
                        !crosses(c.uv[0], c.uv[1], s.uv[0], s.uv[1]),
                        "intersecting UV boundaries",
                    )?;
                }
            }
        }
        if k > 0 {
            check(contains(&loops[0], p), "hole outside face")?;
            for hole in &loops[1..k] {
                check(
                    !contains(hole, p) && !contains(ring, hole[0].uv[0]),
                    "nested trim holes",
                )?;
            }
        }
    }
    Ok(())
}

#[derive(Default, Serialize)]
pub(super) struct Mesh {
    pub(super) points: Vec<[f64; 3]>,
    pub(super) triangles: Vec<[usize; 3]>,
    face_ids: Vec<u64>,
}

impl Topology {
    /// Build one conforming mesh for all faces. A reversed coedge reads the
    /// same vertex indices backwards. A full periodic edge ends at its start.
    pub fn mesh(&self, rows: &Rows) -> Result<Mesh> {
        let mut mesh = Mesh::default();
        let mut vertices = HashMap::new();
        for (&id, p) in &self.vertices {
            vertices.insert(id, mesh.points.len());
            mesh.points.push(p.to_array());
        }
        let mut edges = HashMap::new();
        for e in &self.edges {
            let c = &rows.curves[e.curve as usize];
            let n = divisions(
                c.acceleration(e.interval[1] - e.interval[0])?,
                self.tolerance / 4.,
            )?;
            let mut sample = vec![vertices[&e.vertices[0]]];
            for i in 1..n {
                let p = c.point(lerp(e.interval, i as f64 / n as f64))?;
                sample.push(mesh.points.len());
                mesh.points.push(p.to_array());
            }
            sample.push(vertices[&e.vertices[1]]);
            edges.insert(e.id, sample);
        }
        for f in &self.faces {
            mesh.face(rows, f, &edges, self.tolerance)?;
        }
        Ok(mesh)
    }
}

#[derive(Clone, Copy)]
struct Corner {
    vertex: usize,
    uv: DVec2,
}
impl spade::HasPosition for Corner {
    type Scalar = f64;
    fn position(&self) -> spade::Point2<f64> {
        spade::Point2::new(self.uv.x, self.uv.y)
    }
}

impl Mesh {
    fn face(
        &mut self,
        rows: &Rows,
        f: &Face,
        edges: &HashMap<u64, Vec<usize>>,
        tol: f64,
    ) -> Result<()> {
        use spade::{ConstrainedDelaunayTriangulation, Triangulation};
        let surface = &rows.surfaces[f.surface as usize];
        let mut corners = Vec::new();
        let mut constraints = Vec::new();
        for ring in &f.loops {
            let first = corners.len();
            for c in ring {
                let sample = &edges[&c.edge];
                let n = sample.len() - 1;
                for i in 0..n {
                    let uv = c.uv[0].lerp(c.uv[1], i as f64 / n as f64);
                    let vertex = sample[if c.reversed { n - i } else { i }];
                    corners.push(Corner { vertex, uv });
                }
            }
            constraints.extend((first..corners.len() - 1).map(|i| [i, i + 1]));
            constraints.push([corners.len() - 1, first]);
        }
        let count = corners.len();
        let mut conflict = false;
        let mut cdt = ConstrainedDelaunayTriangulation::<Corner>::try_bulk_load_cdt(
            corners,
            constraints,
            |_| conflict = true,
        )
        .map_err(|_| "UV coordinates outside triangulation range")?;
        check(
            !conflict && cdt.num_vertices() == count && cdt.num_constraints() == count,
            "ambiguous constrained trim",
        )?;
        for _ in 0..32 {
            let mut mids = BTreeMap::new();
            let mut accepted = Vec::new();
            for triangle in cdt.inner_faces() {
                let c = triangle.vertices().map(|v| *v.data());
                let uv = (c[0].uv + c[1].uv + c[2].uv) / 3.;
                if !contains(&f.loops[0], uv) || f.loops[1..].iter().any(|r| contains(r, uv)) {
                    continue;
                }
                let mut valid = true;
                for edge in triangle.adjacent_edges() {
                    let [a, b] = edge.vertices().map(|v| v.data().uv);
                    if surface.acceleration(&rows.curves, a, b)? > tol {
                        check(
                            !cdt.is_constraint_edge(edge.as_undirected().fix()),
                            "boundary sampling insufficient for support",
                        )?;
                        mids.insert(edge.as_undirected().index(), (a + b) * 0.5);
                        valid = false;
                    }
                }
                if valid {
                    let mut indices = c.map(|v| v.vertex);
                    if f.reversed {
                        indices.swap(1, 2);
                    }
                    check(
                        indices[0] != indices[1]
                            && indices[1] != indices[2]
                            && indices[0] != indices[2],
                        "degenerate display triangle",
                    )?;
                    accepted.push(indices);
                }
            }
            if mids.is_empty() {
                check(!accepted.is_empty(), "empty trimmed face")?;
                self.face_ids
                    .resize(self.face_ids.len() + accepted.len(), f.id);
                self.triangles.extend(accepted);
                return Ok(());
            }
            check(
                self.points.len().saturating_add(mids.len()) <= 4_000_000,
                "tessellation vertex budget exceeded",
            )?;
            for (_, uv) in mids {
                let vertex = self.points.len();
                let h = cdt
                    .insert(Corner { vertex, uv })
                    .map_err(|_| "invalid refinement point")?;
                check(
                    h.index() + 1 == cdt.num_vertices() && cdt.vertex(h).data().vertex == vertex,
                    "tessellation precision exhausted",
                )?;
                self.points
                    .push(surface.point(&rows.curves, uv.x, uv.y)?.to_array());
            }
        }
        Err("tessellation depth limit exceeded")
    }
}
