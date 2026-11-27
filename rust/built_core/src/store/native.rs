// SPDX-License-Identifier: GPL-3.0-or-later
//! An exact definition owns logical store slots. Trim incidences name those
//! slots directly. The conforming display mesh never allocates store slots.
use super::{edit::Change, frame::Frame, keys::Kind, Store};
use crate::exact::{
    patch::{Coedge, Mesh, Topology},
    Rows,
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, OnceLock},
};

#[derive(Debug)]
pub(crate) struct NativePatch {
    pub rows: Rows,
    samples: OnceLock<Result<Mesh, &'static str>>,
}
impl NativePatch {
    pub fn topology(&self) -> &Topology {
        self.rows.topology.as_ref().unwrap()
    }
    pub fn mesh(&self) -> Result<&Mesh, &'static str> {
        self.samples
            .get_or_init(|| self.topology().mesh(&self.rows))
            .as_ref()
            .map_err(|e| *e)
    }
    /// Translate caller-local identifiers once. Store file ids and occurrence
    /// keys are assigned by the ordinary topology allocator.
    pub fn new(mut rows: Rows) -> Result<Self, &'static str> {
        rows.validate_pool()?;
        rows.topology
            .as_ref()
            .ok_or("missing native topology")?
            .validate(&rows)?;
        let t = rows.topology.as_mut().ok_or("missing native topology")?;
        let vertices: HashMap<_, _> = t
            .vertices
            .keys()
            .enumerate()
            .map(|(i, &v)| (v, i as u64))
            .collect();
        let edges: HashMap<_, _> = t
            .edges
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id, i as u64))
            .collect();
        let mut points = BTreeMap::new();
        for (&id, &point) in &t.vertices {
            points.insert(vertices[&id], point);
        }
        t.vertices = points;
        for (i, e) in t.edges.iter_mut().enumerate() {
            e.id = i as u64;
            e.vertices = e.vertices.map(|v| vertices[&v]);
        }
        for (i, f) in t.faces.iter_mut().enumerate() {
            f.id = i as u64;
            for c in f.loops.iter_mut().flatten() {
                c.edge = c.edge.map(|e| edges[&e]);
                c.pole = c.pole.map(|v| vertices[&v]);
            }
        }
        Ok(Self {
            rows,
            samples: OnceLock::new(),
        })
    }
    pub fn start(&self, c: &Coedge) -> u32 {
        c.pole.unwrap_or_else(|| {
            self.topology().edges[c.edge.unwrap() as usize].vertices[c.reversed as usize]
        }) as u32
    }
}
impl Store {
    pub(crate) fn install_patch(&mut self, patch: Arc<NativePatch>) -> Result<(), String> {
        if !self.state.frame.is_model()
            || Kind::ALL
                .iter()
                .any(|&k| self.state.slots[k as usize].len() != 0)
        {
            return Err("native patch needs an empty definition".into());
        }
        patch.mesh()?;
        let topo = patch.topology();
        let mut stage = self.clone();
        let mut changes = Change::default();
        for (&v, p) in &topo.vertices {
            let slot = stage
                .vertex(p.to_array(), &mut changes)
                .map_err(|e| format!("{e:?}"))?;
            if slot as u64 != v {
                return Err("store weld collapses distinct native vertices".into());
            }
        }
        for e in &topo.edges {
            let slot = stage.new_slot(Kind::Edge).map_err(|e| format!("{e:?}"))?;
            stage
                .state
                .ends
                .set(slot as usize, e.vertices.map(|v| v as u32));
            stage.disk_link(slot);
        }
        for f in &topo.faces {
            let slot = stage.new_slot(Kind::Face).map_err(|e| format!("{e:?}"))?;
            let rings: Vec<Vec<_>> = f
                .loops
                .iter()
                .map(|r| r.iter().map(|u| patch.start(u)).collect())
                .collect();
            stage.set_rings(slot, &rings, false);
            for e in f.loops.iter().flatten().filter_map(|u| u.edge) {
                stage.ef_push(e as u32, slot);
            }
        }
        stage.state.exact.native = Some(patch);
        stage.revision += 1;
        stage.check()?;
        *self = stage;
        Ok(())
    }

    pub fn boundary_edges(&self, face: u32) -> Vec<Vec<Option<(u32, bool)>>> {
        if let Some(patch) = &self.state.exact.native {
            if let Some(f) = patch.topology().faces.get(face as usize) {
                return f
                    .loops
                    .iter()
                    .map(|r| {
                        r.iter()
                            .map(|c| c.edge.map(|e| (e as u32, c.reversed)))
                            .collect()
                    })
                    .collect();
            }
        }
        self.face_rings(face)
            .iter()
            .map(|r| {
                r.iter()
                    .zip(r.iter().cycle().skip(1))
                    .map(|(&a, &b)| {
                        self.find_edge(a, b)
                            .map(|e| (e, self.state.ends.get(e as usize)[0] != a))
                    })
                    .collect()
            })
            .collect()
    }
    pub(crate) fn model_point(&self, p: [f64; 3]) -> [f64; 3] {
        Frame::default().convert(p, &self.state.frame)
    }
    pub(crate) fn native_triangles(&self, f: u32) -> Option<Vec<[[f64; 3]; 3]>> {
        let patch = self.state.exact.native.as_ref()?;
        let mesh = patch.mesh().expect("admitted native display mesh");
        Some(
            mesh.triangles
                .iter()
                .zip(&mesh.face_ids)
                .filter(|(_, id)| **id == u64::from(f))
                .map(|(tri, _)| {
                    let mut t = tri.map(|v| self.model_point(mesh.points[v]));
                    if self.state.frame.mirrored() {
                        t.swap(1, 2);
                    }
                    t
                })
                .collect(),
        )
    }
}

impl Store {
    /// Stage the conforming fallback in an occurrence's private edit view.
    /// The first successful mutation publishes it through the existing
    /// definition transaction. Selection, cancellation and opening the edit
    /// context alone therefore leave the document exact.
    pub(crate) fn facet_view(&self) -> Result<Self, String> {
        let Some(patch) = self.state.exact.native.clone() else {
            return Ok(self.clone());
        };
        let mesh = patch.mesh()?;
        let mut st = self.clone();
        let mut ch = Change::default();
        let mut vertices = Vec::with_capacity(mesh.points.len());
        let mut distinct = std::collections::HashSet::new();
        for &p in &mesh.points {
            let v = st
                .vertex(self.model_point(p), &mut ch)
                .map_err(|e| format!("{e:?}"))?;
            if !distinct.insert(v) {
                return Err("store weld collapses distinct model samples".into());
            }
            vertices.push(v);
        }
        for f in self.live_slots(Kind::Face) {
            st.remove_face(f, &mut ch);
        }
        for e in self.live_slots(Kind::Edge) {
            st.remove_edge(e, &mut ch);
        }
        st.state.exact.native = None;
        let mut largest = HashMap::<u32, (f64, u32)>::new();
        for (triangle, &face) in mesh.triangles.iter().zip(&mesh.face_ids) {
            let mut ring = triangle.map(|i| vertices[i]);
            if self.state.frame.mirrored() {
                ring.swap(1, 2);
            }
            let f = st
                .add_welded_face(&[ring.to_vec()], &mut ch)
                .map_err(|e| format!("{e:?}"))?;
            let original = face as u32;
            st.state
                .fattr
                .set(f as usize, *self.state.fattr.get(original as usize));
            st.state
                .fflags
                .set(f as usize, *self.state.fflags.get(original as usize));
            let [a, b, c] = triangle.map(|i| glam::DVec3::from(mesh.points[i]));
            let area = (b - a).cross(c - a).length_squared();
            if largest.get(&original).is_none_or(|p| p.0 < area) {
                largest.insert(original, (area, f));
            }
        }
        for (source, (_, slot)) in largest {
            st.set_pid(Kind::Face, slot, self.pid(Kind::Face, source));
        }
        for (&edge, points) in &mesh.edge_samples {
            let mut longest = None;
            for segment in points.windows(2) {
                let (a, b) = (segment[0], segment[1]);
                let e = st
                    .link_edge(vertices[a], vertices[b], &mut ch)
                    .map_err(|e| format!("{e:?}"))?;
                st.state
                    .eflags
                    .set(e as usize, *self.state.eflags.get(edge as usize));
                st.state
                    .elayer
                    .set(e as usize, *self.state.elayer.get(edge as usize));
                st.state
                    .ecurve
                    .set(e as usize, *self.state.ecurve.get(edge as usize));
                let length =
                    glam::DVec3::from(mesh.points[a]).distance_squared(mesh.points[b].into());
                if longest.is_none_or(|(l, _)| l < length) {
                    longest = Some((length, e));
                }
            }
            if let Some((_, slot)) = longest {
                st.set_pid(Kind::Edge, slot, self.pid(Kind::Edge, edge as u32));
            }
        }
        st.revision = self.revision;
        Ok(st)
    }
}

impl NativePatch {
    pub(crate) fn edge_point(&self, edge: usize, t: f64) -> Option<glam::DVec3> {
        let e = self.topology().edges.get(edge)?;
        self.rows.curves[e.curve as usize]
            .point(e.interval[0] + (e.interval[1] - e.interval[0]) * t)
            .ok()
    }

    /// Refine the sample interval selected by the screen segment broad phase.
    pub(crate) fn closest_screen(
        &self,
        edge: usize,
        mut lo: f64,
        mut hi: f64,
        distance: impl Fn(glam::DVec3) -> f64,
    ) -> Option<(f64, f64)> {
        let cost = |t| {
            self.edge_point(edge, t)
                .map(&distance)
                .unwrap_or(f64::INFINITY)
        };
        let ends = [lo, hi];
        for _ in 0..48 {
            let step = (hi - lo) / 3.;
            if cost(lo + step) < cost(hi - step) {
                hi -= step;
            } else {
                lo += step;
            }
        }
        [ends[0], ends[1], (lo + hi) * 0.5]
            .into_iter()
            .map(|t| (cost(t), t))
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }
    pub(crate) fn near_edges(
        &self,
        camera: &crate::pick::Projector,
        world: impl Fn(glam::DVec3) -> [f64; 3],
        x: f64,
        y: f64,
        radius: f64,
    ) -> Vec<(usize, f64, f64)> {
        let mesh = self.mesh().expect("admitted exact mesh");
        let mut out = Vec::new();
        for (&id, points) in &mesh.edge_samples {
            let segments = points.windows(2).map(|p| {
                camera.segment(
                    world(mesh.points[p[0]].into()),
                    world(mesh.points[p[1]].into()),
                )
            });
            let n = (points.len() - 1) as f64;
            let hits = crate::pick::nearest_projected(segments, x, y, radius);
            let best = hits
                .into_iter()
                .filter_map(|(i, _, _)| {
                    self.closest_screen(id as usize, i as f64 / n, (i + 1) as f64 / n, |p| {
                        let (a, b, front) = camera.px(world(p));
                        if front {
                            (a - x).hypot(b - y)
                        } else {
                            f64::INFINITY
                        }
                    })
                })
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((d, t)) = best.filter(|h| h.0 <= radius) {
                out.push((id as usize, d, t));
            }
        }
        out.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        out
    }

    /// Intersect the analytic support using a hit on a model-tolerance triangle
    /// as the initial UV estimate. Accept solutions only in the trimmed UV mesh.
    pub(crate) fn ray(
        &self,
        o: glam::DVec3,
        d: glam::DVec3,
        face: Option<usize>,
    ) -> Option<(usize, f64, glam::DVec3)> {
        use glam::{DMat3, DVec2, DVec3};
        let mesh = self.mesh().ok()?;
        let mut best = None;
        for (i, tri) in mesh.triangles.iter().enumerate() {
            let f = mesh.face_ids[i] as usize;
            if face.is_some_and(|wanted| f != wanted) {
                continue;
            }
            let [a, b, c] = tri.map(|v| DVec3::from(mesh.points[v]));
            let t = crate::pick::ray_triangle(
                o.to_array(),
                d.to_array(),
                a.to_array(),
                (b - a).to_array(),
                (c - a).to_array(),
            );
            let Some(mut t) = t else { continue };
            let face = &self.topology().faces[f];
            let surface = &self.rows.surfaces[face.surface as usize];
            let eval = |uv: DVec2| surface.point(&self.rows.curves, uv.x, uv.y).ok();
            let [u, v, w] = mesh.parameters[i];
            let bc = (b - a).cross(c - a);
            let den = bc.length_squared();
            let q = o + t * d - a;
            let mut uv = u
                + (v - u) * q.cross(c - a).dot(bc) / den
                + (w - u) * (b - a).cross(q).dot(bc) / den;
            for _ in 0..12 {
                let Some(p) = eval(uv) else { break };
                let derivative = |axis: DVec2| {
                    let h = 1e-6 * uv.dot(axis).abs().max(1.);
                    Some(match (eval(uv + axis * h), eval(uv - axis * h)) {
                        (Some(a), Some(b)) => (a - b) / (2. * h),
                        (Some(a), None) => (a - p) / h,
                        (None, Some(b)) => (p - b) / h,
                        _ => return None,
                    })
                };
                let (Some(du), Some(dv)) = (derivative(DVec2::X), derivative(DVec2::Y)) else {
                    break;
                };
                let residual = p - o - t * d;
                if residual.length() <= 1e-10 * (1. + p.length()) {
                    let inside =
                        mesh.parameters
                            .iter()
                            .zip(&mesh.face_ids)
                            .any(|(triangle, &owner)| {
                                if owner != f as u64 {
                                    return false;
                                }
                                let [a, b, c] = *triangle;
                                let area = (b - a).perp_dot(c - a);
                                if area == 0. {
                                    return false;
                                }
                                let x = (uv - a).perp_dot(c - a) / area;
                                let y = (b - a).perp_dot(uv - a) / area;
                                x >= -1e-9 && y >= -1e-9 && x + y <= 1. + 1e-9
                            });
                    if inside && t >= 0. && best.is_none_or(|(_, old, _)| t < old) {
                        let normal = du.cross(dv).normalize_or_zero();
                        best = Some((f, t, if face.reversed { -normal } else { normal }));
                    }
                    break;
                }
                let jac = DMat3::from_cols(du, dv, -d);
                if jac.determinant().abs() <= f64::MIN_POSITIVE {
                    break;
                }
                let step = jac.inverse() * residual;
                if !step.is_finite() {
                    break;
                }
                uv -= step.truncate();
                t -= step.z;
            }
        }
        best
    }
}
