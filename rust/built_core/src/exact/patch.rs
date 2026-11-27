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
    edge: Option<u64>,
    pole: Option<u64>,
    #[serde(default)]
    projected: bool,
    #[serde(default)]
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
fn uv_at(a: DVec2, b: DVec2, t: f64) -> DVec2 {
    DVec2::new(lerp([a.x, b.x], t), lerp([a.y, b.y], t))
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
            let uv = uv_at(a, b, t);
            let p = surface.point(&rows.curves, uv.x, uv.y)?;
            let q = curve.point(lerp(e.interval, if c.reversed { 1. - t } else { t }))?;
            check(
                p.distance(q) <= allowance,
                "edge and lifted pcurve disagree",
            )?;
        }
        Ok(())
    }

    /// A planar cap can use an orthogonal projection of a full circle or
    /// ellipse, with one closed edge per loop. Its extrema along the plane
    /// normal certify the complete pcurve. Conservative inscribed and
    /// circumscribed radii certify separation without sampling intersections.
    fn certify_conic_loops(
        &self,
        rows: &Rows,
        face: &Face,
        surface: &Surface,
        edges: &HashMap<u64, &Edge>,
    ) -> Result<()> {
        let Surface::Plane { frame } = surface else {
            return Err("projected conic needs a plane");
        };
        let mut loops: Vec<(DVec2, f64, f64)> = Vec::new();
        for ring in &face.loops {
            let [c] = ring.as_slice() else {
                return Err("expected a single conic use per loop");
            };
            check(
                c.projected
                    && c.pole.is_none()
                    && c.tolerance > 0.
                    && c.tolerance <= self.tolerance,
                "invalid projected use",
            )?;
            let e = edges
                .get(&c.edge.ok_or("missing conic edge")?)
                .ok_or("missing conic edge")?;
            let span = e.interval[1] - e.interval[0];
            check(
                e.vertices[0] == e.vertices[1]
                    && (span - std::f64::consts::TAU).abs() <= 8. * f64::EPSILON,
                "conic cap needs a closed full turn",
            )?;
            let curve = &rows.curves[e.curve as usize];
            let (base, major, minor) = match curve {
                Curve::Circle { frame, radius } => (frame, *radius, *radius),
                Curve::Ellipse {
                    frame,
                    major,
                    minor,
                } => (frame, *major, *minor),
                _ => return Err("expected circle or ellipse for planar cap"),
            };
            let normal_deviation = (base.origin - frame.origin).dot(frame.axis).abs()
                + (major * base.reference.dot(frame.axis)).hypot(minor * base.y().dot(frame.axis));
            check(normal_deviation <= c.tolerance, "conic misses cap plane")?;
            let project = |p: DVec3| {
                let d = p - frame.origin;
                DVec2::new(d.dot(frame.reference), d.dot(frame.y()))
            };
            let anchor = project(curve.point(e.interval[0])?);
            check(
                c.uv[0] == c.uv[1] && anchor.distance(c.uv[0]) <= c.tolerance,
                "incorrect projected trim anchor",
            )?;
            let center = project(base.origin);
            let inner = minor * base.axis.dot(frame.axis).abs();
            check(
                inner > 0. && center.is_finite(),
                "collapsed projected conic",
            )?;
            if let Some(&(outer, radius, _)) = loops.first() {
                check(
                    outer.distance(center) + major < radius,
                    "uncertified conic hole containment",
                )?;
                for &(other, _, r) in &loops[1..] {
                    check(
                        other.distance(center) > r + major,
                        "uncertified conic hole separation",
                    )?;
                }
            }
            loops.push((center, inner, major));
        }
        Ok(())
    }

    pub(super) fn boundary(&self, id: u64) -> Result<String> {
        let face = self
            .faces
            .iter()
            .find(|f| f.id == id)
            .ok_or("unknown logical face")?;
        serde_json::to_string(&face.loops).map_err(|_| "invalid boundary")
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
            if f.loops.iter().flatten().any(|c| c.projected) {
                self.certify_conic_loops(rows, f, s, &index)?;
                continue;
            }
            for ring in &f.loops {
                check(ring.len() >= 3, "insufficient UV corners")?;
                for (i, c) in ring.iter().enumerate() {
                    let next = &ring[(i + 1) % ring.len()];
                    let endpoints = |u: &Coedge| -> Result<[u64; 2]> {
                        match (u.edge, u.pole) {
                            (Some(id), None) => {
                                let e = index.get(&id).ok_or("missing coedge edge")?;
                                let mut ends = e.vertices;
                                if u.reversed {
                                    ends.swap(0, 1);
                                }
                                Ok(ends)
                            }
                            (None, Some(id)) if self.vertices.contains_key(&id) => Ok([id, id]),
                            _ => Err("invalid edge or singular vertex use"),
                        }
                    };
                    check(
                        endpoints(c)?[1] == endpoints(next)?[0] && c.uv[1] == next.uv[0],
                        "open trim loop",
                    )?;
                    if let Some(id) = c.edge {
                        self.certify(rows, s, c, index[&id])?;
                    } else {
                        let [a, b] = c.uv;
                        check(
                            a.is_finite()
                                && b.is_finite()
                                && a != b
                                && a.y == b.y
                                && c.tolerance > 0.
                                && c.tolerance <= self.tolerance,
                            "invalid pole use",
                        )?;
                        let radius = match s {
                            Surface::Sphere { radius, .. } if a.y.abs() == FRAC_PI_2 => {
                                radius * a.y.cos().abs()
                            }
                            Surface::Cone {
                                radius, half_angle, ..
                            } => (radius + a.y * half_angle.sin()).abs(),
                            _ => return Err("singular use on regular support"),
                        };
                        let v = self.vertices[&c.pole.unwrap()];
                        let p = s.point(&rows.curves, a.x, a.y)?;
                        check(
                            2. * radius + p.distance(v) <= c.tolerance,
                            "pole vertex misses support",
                        )?;
                    }
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
fn inside(segments: impl Iterator<Item = [DVec2; 2]>, p: DVec2) -> bool {
    segments
        .filter(|&[a, b]| (a.y > p.y) != (b.y > p.y) && ((b.y > a.y) == (orient(a, b, p) > 0.)))
        .count()
        % 2
        == 1
}
fn contains(ring: &[Coedge], p: DVec2) -> bool {
    inside(ring.iter().map(|c| c.uv), p)
}
pub(crate) fn validate_polygon(rings: &[Vec<DVec2>]) -> Result<()> {
    check(!rings.is_empty(), "missing outer boundary")?;
    let mut loops = Vec::new();
    for ring in rings {
        check(ring.len() >= 3, "degenerate trim ring")?;
        let mut uses = Vec::new();
        for (i, &a) in ring.iter().enumerate() {
            let b = ring[(i + 1) % ring.len()];
            check(a.is_finite() && a != b, "invalid trim segment")?;
            uses.push(Coedge {
                edge: Some(0),
                pole: None,
                projected: false,
                reversed: false,
                uv: [a, b],
                tolerance: 0.,
            });
        }
        loops.push(uses);
    }
    check_loops(&loops)
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
        let mut face_bounds = HashMap::<u64, f64>::new();
        for face in &self.faces {
            let surface = &rows.surfaces[face.surface as usize];
            for c in face.loops.iter().flatten() {
                if let Some(id) = c.edge {
                    let a = surface.acceleration(&rows.curves, c.uv[0], c.uv[1])?;
                    let bound = face_bounds.entry(id).or_default();
                    *bound = bound.max(a);
                }
            }
        }
        for e in &self.edges {
            let c = &rows.curves[e.curve as usize];
            let n = divisions(
                c.acceleration(e.interval[1] - e.interval[0])?
                    .max(face_bounds.get(&e.id).copied().unwrap_or(0.)),
                self.tolerance / 4.,
            )?;
            check(
                mesh.points.len().saturating_add(n) <= 4_000_000,
                "edge sample budget exceeded",
            )?;
            let n = if e.vertices[0] == e.vertices[1] {
                n.max(8)
            } else {
                n
            };
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
            mesh.face(rows, f, &edges, &vertices, self.tolerance)?;
        }
        Ok(mesh)
    }
}

#[derive(Clone, Copy)]
struct Corner {
    vertex: usize,
    uv: DVec2,
    residual: f64,
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
        vertices: &HashMap<u64, usize>,
        tol: f64,
    ) -> Result<()> {
        use spade::{ConstrainedDelaunayTriangulation, Triangulation};
        let surface = &rows.surfaces[f.surface as usize];
        let mut corners = Vec::new();
        let mut constraints = Vec::new();
        let mut sampled_loops = Vec::new();
        for ring in &f.loops {
            let first = corners.len();
            for c in ring {
                let Some(edge) = c.edge else {
                    let vertex = vertices[&c.pole.unwrap()];
                    // A pole covers an entire UV side. Repeated UV samples
                    // share its single model vertex.
                    let n = divisions(
                        surface.acceleration(&rows.curves, c.uv[0], c.uv[1])?,
                        tol / 4.,
                    )?;
                    for i in 0..n {
                        let uv = uv_at(c.uv[0], c.uv[1], i as f64 / n as f64);
                        let residual = surface
                            .point(&rows.curves, uv.x, uv.y)?
                            .distance(self.points[vertex].into());
                        corners.push(Corner {
                            vertex,
                            uv,
                            residual,
                        });
                    }
                    continue;
                };
                let sample = &edges[&edge];
                let n = sample.len() - 1;
                for i in 0..n {
                    let vertex = sample[if c.reversed { n - i } else { i }];
                    let uv = if c.projected {
                        let Surface::Plane { frame } = surface else {
                            return Err("projected conic needs a plane");
                        };
                        let d = DVec3::from(self.points[vertex]) - frame.origin;
                        DVec2::new(d.dot(frame.reference), d.dot(frame.y()))
                    } else {
                        uv_at(c.uv[0], c.uv[1], i as f64 / n as f64)
                    };
                    let residual = surface
                        .point(&rows.curves, uv.x, uv.y)?
                        .distance(self.points[vertex].into());
                    corners.push(Corner {
                        vertex,
                        uv,
                        residual,
                    });
                }
            }
            constraints.extend((first..corners.len() - 1).map(|i| [i, i + 1]));
            constraints.push([corners.len() - 1, first]);
            if ring[0].projected {
                sampled_loops.push(corners[first..].iter().map(|c| c.uv).collect::<Vec<_>>());
            }
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
                let in_loop = |k: usize| {
                    if sampled_loops.is_empty() {
                        return contains(&f.loops[k], uv);
                    }
                    let r = &sampled_loops[k];
                    inside(
                        r.iter()
                            .zip(r.iter().cycle().skip(1))
                            .map(|(&a, &b)| [a, b]),
                        uv,
                    )
                };
                if !in_loop(0) || (1..f.loops.len()).any(in_loop) {
                    continue;
                }
                let lo = c[0].uv.min(c[1].uv).min(c[2].uv);
                let hi = c[0].uv.max(c[1].uv).max(c[2].uv);
                let residual = c.iter().map(|c| c.residual).fold(0., f64::max);
                check(residual <= tol, "boundary sample outside model tolerance")?;
                // Taylor expand at an arbitrary UV point inside the triangle.
                // Its barycentric linear terms cancel. The parameter box bounds
                // every direction to a corner, also across a cone or revolution.
                let valid = 0.5 * surface.acceleration(&rows.curves, lo, hi)? + residual <= tol;
                if !valid {
                    let mut best = None;
                    for edge in triangle.adjacent_edges() {
                        let [a, b] = edge.vertices().map(|v| v.data().uv);
                        if !cdt.is_constraint_edge(edge.as_undirected().fix()) {
                            let score = surface.acceleration(&rows.curves, a, b)?;
                            if best.is_none_or(|(old, _, _)| score > old) {
                                best = Some((score, edge.as_undirected().index(), (a + b) * 0.5));
                            }
                        }
                    }
                    if let Some((_, index, mid)) = best {
                        mids.insert(index, mid);
                    } else {
                        return Err("boundary sampling insufficient for support");
                    }
                }
                if valid {
                    let mut indices = c.map(|v| v.vertex);
                    if f.reversed {
                        indices.swap(1, 2);
                    }
                    if indices[0] == indices[1]
                        || indices[1] == indices[2]
                        || indices[0] == indices[2]
                    {
                        let duplicate = if indices[0] == indices[1] {
                            indices[0]
                        } else {
                            indices[2]
                        };
                        check(
                            f.loops
                                .iter()
                                .flatten()
                                .any(|u| u.pole.is_some_and(|id| vertices[&id] == duplicate)),
                            "degenerate display triangle",
                        )?;
                        continue;
                    }
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
                let previous = cdt.num_vertices();
                let h = cdt
                    .insert(Corner {
                        vertex,
                        uv,
                        residual: 0.,
                    })
                    .map_err(|_| "invalid refinement point")?;
                check(
                    previous + 1 == cdt.num_vertices() && cdt.vertex(h).data().vertex == vertex,
                    "tessellation precision exhausted",
                )?;
                self.points
                    .push(surface.point(&rows.curves, uv.x, uv.y)?.to_array());
            }
        }
        Err("tessellation depth limit exceeded")
    }
}
