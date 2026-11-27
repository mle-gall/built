// SPDX-License-Identifier: GPL-3.0-or-later
//! Exact bindings follow every History step (docs/exact-geometry.md §3).
//!
//! After persistent-id succession, between the state before the step and
//! the current one:
//!
//! 1. a created element whose id was inherited or derived takes the
//!    parent's curve or surface (a split keeps one support on both halves);
//! 2. bound elements whose vertices moved follow the affine map those
//!    vertices moved by (Move, Rotate, Scale, Flip, partial stretches): a
//!    circle stays a circle under a similarity and becomes an ellipse
//!    otherwise, a cylinder stays one while its right section is round;
//! 3. new edges are derived: fitted circles on curve runs drawn this step
//!    (only when the command asked for it), copies of a bound edge at the
//!    same place, sections of a bound cylinder by a plane (intersect, a
//!    line across the face);
//! 4. new faces are derived: a face swept from a bound edge along a line
//!    (push/pull, Follow Me along a straight path) is an extrusion or a
//!    cylinder, one swept about the axis of its bound rails a revolution
//!    (cylinder, cone, plane, sphere, torus);
//! 5. every touched binding is validated against its store element; the
//!    ones that no longer hold are dropped. The facets are already the
//!    conforming fallback, so the step itself never fails here.
use super::{
    edit::Change,
    exact::{ChunkMap, Curved},
    keys::Kind,
    State, Store,
};
type EdgeMap = ChunkMap<EdgeBind>;
type FaceMap = ChunkMap<Support>;
use crate::exact::{
    bound::{self, EdgeBind, Support},
    Curve, Surface,
};
use glam::{DMat3, DQuat, DVec3};
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

/// Fit circles to the curve runs the next step draws.
pub(crate) const FIT_CIRCLES: u8 = 1;
/// The next step's straight edges stay chords (no plane sections).
pub(crate) const CHORDS: u8 = 2;
/// The next step divides edges: new vertices on a bound edge's chord move
/// onto its curve where every incident face stays planar.
pub(crate) const PROJECT: u8 = 4;
/// The next step is a push/pull that keeps its base face.
pub(crate) const KEEP_BASE: u8 = 8;
/// The next step draws arcs: a run of two segments is a circle too.
pub(crate) const ARC: u8 = 16;

thread_local! {
    static INTENT: Cell<u8> = const { Cell::new(0) };
}

/// What the next History step means geometrically, set by the command
/// that runs it and consumed by its succession.
pub(crate) fn set_intent(bits: u8) {
    INTENT.with(|c| c.set(bits));
}
/// Run `f` with an intent for the step it executes; never leaks to later steps.
pub(crate) fn with_intent<T>(bits: u8, f: impl FnOnce() -> T) -> T {
    set_intent(bits);
    let out = f();
    set_intent(0);
    out
}
thread_local! {
    static AXIS: Cell<Option<(DVec3, DVec3)>> = const { Cell::new(None) };
    static JOINTS: std::cell::RefCell<Vec<(DVec3, DVec3)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Follow Me along a polyline (store coordinates): its interior joints'
/// mitre planes, point and normal, for the next step.
pub(crate) fn with_joints<T>(joints: Vec<(DVec3, DVec3)>, f: impl FnOnce() -> T) -> T {
    JOINTS.with(|j| *j.borrow_mut() = joints);
    let out = f();
    JOINTS.with(|j| j.borrow_mut().clear());
    out
}

/// Follow Me about a fixed axis (store coordinates): the next step's new
/// edges that turn about it at one height and radius are arcs about it.
pub(crate) fn with_axis<T>(axis: Option<(DVec3, DVec3)>, f: impl FnOnce() -> T) -> T {
    AXIS.with(|c| c.set(axis));
    let out = f();
    AXIS.with(|c| c.set(None));
    out
}
fn take_axis() -> Option<(DVec3, DVec3)> {
    AXIS.with(|c| c.replace(None))
}
pub(crate) fn peek_intent() -> u8 {
    INTENT.with(|c| c.get())
}
fn take_intent() -> u8 {
    INTENT.with(|c| c.replace(0))
}

/// Counts of the last rebind pass, for tests and the transaction report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub kept: usize,
    pub derived: usize,
    pub transformed: usize,
    pub dropped: usize,
}

const SURFACE_PATH_STEP: f64 = std::f64::consts::PI / 60.;

fn point(st: &State, v: u32) -> DVec3 {
    DVec3::from(st.frame.definition_point(*st.vpos.get(v as usize)))
}

fn debug(text: impl FnOnce() -> String) {
    if std::env::var_os("BUILT_EXACT_DEBUG").is_some() {
        eprintln!("exact: {}", text());
    }
}

fn slot_of(key: u128) -> Option<u32> {
    super::keys::split_key(key).map(|(_, _, _, s)| s)
}

/// One ring side: its two vertices and the edge joining them.
#[derive(Clone, Copy)]
pub(crate) struct Side {
    pub a: u32,
    pub b: u32,
    pub edge: Option<u32>,
}

impl Store {
    pub(crate) fn def_point(&self, v: u32) -> DVec3 {
        point(&self.state, v)
    }

    pub(crate) fn sides(&self, f: u32) -> Vec<Vec<Side>> {
        self.face_rings(f)
            .iter()
            .map(|r| {
                r.iter()
                    .zip(r.iter().cycle().skip(1))
                    .map(|(&a, &b)| Side {
                        a,
                        b,
                        edge: self.find_edge(a, b),
                    })
                    .collect()
            })
            .collect()
    }

    fn tol_for(&self, pts: impl IntoIterator<Item = DVec3>) -> f64 {
        bound::tolerance(pts.into_iter().fold(0., |m, p| m.max(p.abs().max_element())))
    }

    /// The edge's binding with its interval re-derived from its ends, if
    /// both still lie on the curve.
    pub(crate) fn edge_holds(&self, e: u32, b: &EdgeBind) -> Option<EdgeBind> {
        let [p, q] = *self.state.ends.get(e as usize);
        let (a, c) = (self.def_point(p), self.def_point(q));
        let tol = self.tol_for([a, c]);
        if !b.curve.contains(a, tol) || !b.curve.contains(c, tol) {
            return None;
        }
        let mut interval = b.curve.interval(a, c, Some(b.interval))?;
        // Keep the stored interval when it differs by roundoff only.
        if (0..2).all(|i| (interval[i] - b.interval[i]).abs() <= 1e-12 * (1. + b.interval[i].abs())) {
            interval = b.interval;
        }
        Some(EdgeBind {
            curve: b.curve.clone(),
            interval,
        })
    }

    /// Whether the face is a valid trimmed region of `s`: every vertex and
    /// every side on the support (bound sides by their whole curve), and
    /// the rings a simple region in its parameters.
    pub(crate) fn face_holds(&self, f: u32, s: &Support, edges: &EdgeMap) -> bool {
        self.face_holds_with(f, s, edges, &[])
    }

    /// `face_holds` with candidate rails laid over `edges`.
    fn face_holds_with(&self, f: u32, s: &Support, edges: &EdgeMap, extra: &[(u32, EdgeBind)]) -> bool {
        let lookup = |e: u32| extra.iter().find(|r| r.0 == e).map(|r| &r.1).or_else(|| edges.get(&e));
        if !s.validate() || matches!(s.surface, Surface::Plane { .. }) {
            return false;
        }
        let sides = self.sides(f);
        let pts: Vec<Vec<DVec3>> = sides
            .iter()
            .map(|r| r.iter().map(|x| self.def_point(x.a)).collect())
            .collect();
        let tol = self.tol_for(pts.iter().flatten().copied());
        for (ring, p) in sides.iter().zip(&pts) {
            for (i, side) in ring.iter().enumerate() {
                if !s.contains(p[i], tol) {
                    debug(|| format!("face {f}: vertex {i} off support"));
                    return false;
                }
                let q = p[(i + 1) % p.len()];
                let ok = match side.edge.and_then(lookup) {
                    Some(b) => s.holds_curve(&b.curve, b.interval, tol),
                    None => side.edge.is_some() && s.holds_segment(p[i], q, tol),
                };
                if !ok {
                    debug(|| format!("face {f}: side {i} off support"));
                    return false;
                }
            }
        }
        let ok = bound::trim_region(s, &pts, tol).is_some();
        if !ok {
            debug(|| format!("face {f}: no trimmed region"));
        }
        ok
    }

    /// The rebind pass (module docs). Runs at the end of `succeed`.
    pub(crate) fn rebind(
        &mut self,
        before: &State,
        ch: &Change,
        assigned: &[(Kind, u32, u64)],
        derived: &[(Kind, u64, u64)],
    ) -> Report {
        let intent = take_intent();
        let axis = take_axis();
        let mut report = Report::default();
        if before.exact.curved.is_none()
            && self.state.exact.curved.is_none()
            && intent & FIT_CIRCLES == 0
            && axis.is_none()
        {
            return report;
        }
        let empty = Curved::default();
        let old = before.exact.curved.as_deref().unwrap_or(&empty);
        let now = self.state.exact.curved.clone();
        let now = now.as_deref().unwrap_or(&empty);
        // Copy-on-write: only chunks a step edits are copied.
        let mut edges: EdgeMap = now.edges.clone();
        let mut faces: FaceMap = now.faces.clone();
        let created = |k: Kind| -> Vec<u32> {
            ch.created[k as usize].iter().filter_map(|&key| slot_of(key)).collect()
        };
        let (new_edges, new_faces) = (created(Kind::Edge), created(Kind::Face));
        let mut touched_e: BTreeSet<u32> = BTreeSet::new();
        let mut touched_f: BTreeSet<u32> = BTreeSet::new();
        touched_e.extend(ch.modified[1].iter().filter_map(|&k| slot_of(k)));
        touched_f.extend(ch.modified[2].iter().filter_map(|&k| slot_of(k)));
        touched_e.extend(&new_edges);
        touched_f.extend(&new_faces);

        // 1. Inherited and derived ids take their parent's binding. Parents
        // are elements the step removed or rewrote: only those are looked at.
        let parent: HashMap<(usize, u64), u64> =
            derived.iter().map(|&(k, new, p)| ((k as usize, new), p)).collect();
        let _ = assigned;
        let changed_before = |k: Kind| -> Vec<u32> {
            ch.removed[k as usize]
                .iter()
                .chain(&ch.modified[k as usize])
                .filter_map(|&key| slot_of(key))
                .collect()
        };
        if !old.edges.is_empty() && !new_edges.is_empty() {
            let by_id: HashMap<u64, &EdgeBind> = changed_before(Kind::Edge)
                .into_iter()
                .filter_map(|s| Some((before.ids[1].get(s), old.edge(s)?)))
                .collect();
            for &e in &new_edges {
                let id = self.pid(Kind::Edge, e);
                let b = by_id.get(&id).or_else(|| parent.get(&(1, id)).and_then(|p| by_id.get(p)));
                if let Some(b) = b {
                    edges.insert(e, (*b).clone());
                }
            }
        }
        if !old.faces.is_empty() && !new_faces.is_empty() {
            let by_id: HashMap<u64, &Support> = changed_before(Kind::Face)
                .into_iter()
                .filter_map(|s| Some((before.ids[2].get(s), old.face(s)?)))
                .collect();
            for &f in &new_faces {
                let id = self.pid(Kind::Face, f);
                let b = by_id.get(&id).or_else(|| parent.get(&(2, id)).and_then(|p| by_id.get(p)));
                if let Some(b) = b {
                    faces.insert(f, (*b).clone());
                }
            }
        }

        // 2. Bindings follow the map their vertices moved by.
        let moved: HashSet<u32> = ch.moved.iter().filter_map(|&k| slot_of(k)).collect();
        if !moved.is_empty() && (!edges.is_empty() || !faces.is_empty()) {
            report.transformed += self.follow(before, &moved, &mut edges, &mut faces, &mut touched_e, &mut touched_f);
        }

        // 3. Derive curves for new edges.
        if intent & PROJECT != 0 && !old.edges.is_empty() {
            report.derived += self.project(before, old, ch, &new_edges, &mut edges, &mut touched_e);
        }
        if intent & FIT_CIRCLES != 0 {
            report.derived += self.fit_runs(&new_edges, &mut edges, intent & ARC != 0);
        }
        if let Some((o, a)) = axis {
            report.derived += self.turns(o, a, &new_edges, &mut edges);
        }
        if !old.edges.is_empty() {
            report.derived += self.same_place(before, old, ch, &new_edges, &mut edges);
        }
        if intent & CHORDS == 0 && !faces.is_empty() {
            report.derived += self.sections(&new_edges, &faces, &mut edges);
        }

        // 4. Validate touched edges.
        for e in touched_e.iter().copied().collect::<Vec<_>>() {
            if !self.alive(Kind::Edge, e) {
                continue;
            }
            if let Some(b) = edges.get(&e) {
                match self.edge_holds(e, b) {
                    Some(nb) => {
                        edges.insert(e, nb);
                    }
                    None => {
                        edges.remove(&e);
                        report.dropped += 1;
                    }
                }
            }
            touched_f.extend(self.edge_faces(e).iter().copied());
        }

        // 5. Faces: keep a still-valid support, else derive one. A derived
        // face can bind the rail its neighbour needs (a sweep's profile
        // copies), so repeat while that makes progress.
        let mut pending = Vec::new();
        for f in touched_f.iter().copied() {
            if !self.alive(Kind::Face, f) {
                continue;
            }
            if let Some(s) = faces.get(&f) {
                if self.face_holds(f, s, &edges) {
                    report.kept += 1;
                    continue;
                }
            }
            if faces.remove(&f).is_some() {
                report.dropped += 1;
            }
            pending.push(f);
        }
        // A chord the user asked for stays a chord: no rail binds it.
        let frozen: HashSet<u32> = if intent & CHORDS != 0 { new_edges.iter().copied().collect() } else { HashSet::new() };
        for _ in 0..64 {
            let before = pending.len();
            pending.retain(|&f| match self.derive_face(f, &mut edges, &frozen) {
                Some(s) => {
                    faces.insert(f, s);
                    report.derived += 1;
                    false
                }
                None => true,
            });
            if pending.len() == before || pending.is_empty() {
                break;
            }
        }

        // One representation per support and curve: rows equal to
        // roundoff (rails fitted one by one, facets derived from them)
        // take a neighbour's value, so shared edges and groups see one.
        // Only the touched elements and their neighbours are compared.
        let mut reps: Vec<Support> = Vec::new();
        for &f in &touched_f {
            for side in self.sides(f).into_iter().flatten() {
                for &g in side.edge.map(|e| self.edge_faces(e)).unwrap_or(&[]) {
                    if !touched_f.contains(&g) {
                        if let Some(s) = faces.get(&g) {
                            if !reps.contains(s) {
                                reps.push(s.clone());
                            }
                        }
                    }
                }
            }
        }
        for &f in &touched_f {
            let Some(s) = faces.get(&f) else { continue };
            match reps.iter().find(|r| bound::same_support(r, s)) {
                Some(r) if r != s => {
                    let r = r.clone();
                    *faces.get_mut(&f).unwrap() = r;
                }
                Some(_) => (),
                None => reps.push(s.clone()),
            }
        }
        let mut curve_reps: Vec<Curve> = Vec::new();
        let rail_ends: Vec<u32> = touched_e.iter().filter(|e| edges.contains_key(e)).copied().collect();
        for &e in &rail_ends {
            for v in *self.state.ends.get(e as usize) {
                for n in self.edges_at(v) {
                    if !touched_e.contains(&n) {
                        if let Some(b) = edges.get(&n) {
                            if !curve_reps.contains(&b.curve) {
                                curve_reps.push(b.curve.clone());
                            }
                        }
                    }
                }
            }
        }
        for &e in touched_e.iter().chain(&new_edges) {
            let Some(b) = edges.get(&e) else { continue };
            match curve_reps.iter().find(|r| bound::same_curve(r, &b.curve)) {
                Some(r) if *r != b.curve => {
                    let r = r.clone();
                    edges.get_mut(&e).unwrap().curve = r;
                }
                Some(_) => (),
                None => curve_reps.push(b.curve.clone()),
            }
        }
        let next = Curved { edges, faces };
        let unchanged = self.state.exact.curved.as_deref().is_some_and(|c| *c == next)
            || (self.state.exact.curved.is_none() && next.edges.is_empty() && next.faces.is_empty());
        if !unchanged {
            self.state.exact.curved =
                (!next.edges.is_empty() || !next.faces.is_empty()).then(|| Arc::new(next));
        }
        report
    }

    /// Group moved bound elements by support, fit the map their vertices
    /// moved by, and move the support with them.
    fn follow(
        &self,
        before: &State,
        moved: &HashSet<u32>,
        edges: &mut EdgeMap,
        faces: &mut FaceMap,
        touched_e: &mut BTreeSet<u32>,
        touched_f: &mut BTreeSet<u32>,
    ) -> usize {
        let alive_before = |v: u32| before.slots[0].alive(v);
        let mut count = 0;
        // Edges grouped by curve.
        // Every live element on a moved curve belongs to its group, also
        // the ones whose vertices stayed: a partial move then fits no map.
        let mut groups: Vec<(Curve, Vec<u32>)> = Vec::new();
        for (&e, b) in edges.iter() {
            if self.alive(Kind::Edge, e)
                && self.state.ends.get(e as usize).iter().any(|v| moved.contains(v))
                && !groups.iter().any(|g| g.0 == b.curve)
            {
                groups.push((b.curve.clone(), Vec::new()));
            }
        }
        for (&e, b) in edges.iter() {
            if let Some(g) = groups.iter_mut().find(|g| g.0 == b.curve) {
                if self.alive(Kind::Edge, e) {
                    g.1.push(e);
                }
            }
        }
        for (curve, members) in groups {
            let vertices: BTreeSet<u32> = members
                .iter()
                .flat_map(|&e| *self.state.ends.get(e as usize))
                .filter(|&v| alive_before(v))
                .collect();
            let pairs: Vec<_> = vertices.iter().map(|&v| (point(before, v), self.def_point(v))).collect();
            let tol = self.tol_for(pairs.iter().flat_map(|p| [p.0, p.1]));
            let mapped = bound::fit_affine(&pairs, tol).and_then(|(m, t)| curve.mapped(m, t).map(|c| (m, t, c)));
            for e in members {
                touched_e.insert(e);
                if let Some((m, t, c)) = &mapped {
                    let b = edges.get_mut(&e).unwrap();
                    // The old interval stays the branch hint: a similarity
                    // keeps the parameterization, validation re-derives it.
                    b.curve = c.clone();
                    let _ = (m, t);
                    count += 1;
                }
            }
        }
        // Faces grouped by support.
        let mut groups: Vec<(Support, Vec<u32>)> = Vec::new();
        for (&f, s) in faces.iter() {
            if self.alive(Kind::Face, f)
                && self.face_rings(f).iter().flatten().any(|v| moved.contains(v))
                && !groups.iter().any(|g| g.0 == *s)
            {
                groups.push((s.clone(), Vec::new()));
            }
        }
        for (&f, s) in faces.iter() {
            if let Some(g) = groups.iter_mut().find(|g| g.0 == *s) {
                if self.alive(Kind::Face, f) {
                    g.1.push(f);
                }
            }
        }
        for (support, members) in groups {
            let vertices: BTreeSet<u32> = members
                .iter()
                .flat_map(|&f| self.face_rings(f).into_iter().flatten())
                .filter(|&v| alive_before(v))
                .collect();
            let pairs: Vec<_> = vertices.iter().map(|&v| (point(before, v), self.def_point(v))).collect();
            let tol = self.tol_for(pairs.iter().flat_map(|p| [p.0, p.1]));
            let mapped = bound::fit_affine(&pairs, tol).and_then(|(m, t)| support.mapped(m, t));
            for f in members {
                touched_f.insert(f);
                if let Some(s) = &mapped {
                    faces.insert(f, s.clone());
                    count += 1;
                }
            }
        }
        count
    }

    /// Circles through the curve runs drawn this step (Circle, Arc, Follow
    /// Me along an arc, Offset of an arc). A run whose vertices do not lie
    /// on one circle to roundoff stays faceted.
    fn fit_runs(&self, created: &[u32], edges: &mut EdgeMap, arcs: bool) -> usize {
        let mut runs: BTreeMap<i128, Vec<u32>> = BTreeMap::new();
        for &e in created {
            let c = *self.state.ecurve.get(e as usize);
            if c >= 0 && !edges.contains_key(&e) {
                runs.entry(c).or_default().push(e);
            }
        }
        let mut count = 0;
        for (_, run) in runs {
            // Three points lie on some circle: only a drawn arc may be
            // that short. Any other run needs a fourth point to verify.
            if run.len() < if arcs { 2 } else { 3 } {
                continue;
            }
            let chain = self.chain(&run);
            let pts: Vec<DVec3> = chain.iter().map(|&v| self.def_point(v)).collect();
            let tol = self.tol_for(pts.iter().copied());
            let Some(circle) = bound::fit_circle(&pts, tol) else { continue };
            let mut out = Vec::new();
            for &e in &run {
                let [a, b] = *self.state.ends.get(e as usize);
                match circle.interval(self.def_point(a), self.def_point(b), None) {
                    Some(interval) if (interval[1] - interval[0]).abs() < std::f64::consts::PI => {
                        out.push((e, interval))
                    }
                    _ => break,
                }
            }
            if out.len() == run.len() {
                for (e, interval) in out {
                    edges.insert(
                        e,
                        EdgeBind {
                            curve: circle.clone(),
                            interval,
                        },
                    );
                    count += 1;
                }
            }
        }
        count
    }

    /// New edges turning about the Follow Me axis: both ends at one height
    /// and one nonzero radius, less than half a turn apart.
    fn turns(&self, o: DVec3, a: DVec3, created: &[u32], edges: &mut EdgeMap) -> usize {
        let frame = &self.state.frame;
        let origin = DVec3::from(frame.definition_point(o.to_array()));
        let Some(axis) = frame.definition_delta(a).try_normalize() else { return 0 };
        let mut count = 0;
        for &e in created {
            if edges.contains_key(&e) {
                continue;
            }
            let [p, q] = *self.state.ends.get(e as usize);
            let (x, y) = (self.def_point(p), self.def_point(q));
            let tol = self.tol_for([x, y, origin]);
            let (hx, hy) = ((x - origin).dot(axis), (y - origin).dot(axis));
            let (rx, ry) = (x - origin - axis * hx, y - origin - axis * hy);
            if (hx - hy).abs() > tol || (rx.length() - ry.length()).abs() > tol || rx.length() <= tol {
                continue;
            }
            let circle = Curve::Circle {
                frame: crate::exact::Frame {
                    origin: origin + axis * hx,
                    axis,
                    reference: axis.any_orthonormal_vector(),
                },
                radius: 0.5 * (rx.length() + ry.length()),
            };
            if let Some(interval) = circle.interval(x, y, None) {
                edges.insert(e, EdgeBind { curve: circle, interval });
                count += 1;
            }
        }
        count
    }

    /// Divide: each new vertex inside the chord of an edge bound before
    /// the step moves onto that curve (at its radial parameter), when every
    /// incident face stays planar. The pieces take the curve.
    fn project(
        &mut self,
        before: &State,
        old: &Curved,
        ch: &Change,
        created: &[u32],
        edges: &mut EdgeMap,
        touched_e: &mut BTreeSet<u32>,
    ) -> usize {
        let chords: Vec<(DVec3, DVec3, &EdgeBind)> = ch.removed[1]
            .iter()
            .chain(&ch.modified[1])
            .filter_map(|&k| slot_of(k))
            .filter_map(|e| {
                let b = old.edge(e)?;
                let [p, q] = *before.ends.get(e as usize);
                (p != super::keys::NONE).then(|| (point(before, p), point(before, q), b))
            })
            .collect();
        let new_vertices: Vec<u32> = ch.created[0].iter().filter_map(|&k| slot_of(k)).collect();
        let mut sources: HashMap<u32, &EdgeBind> = HashMap::new();
        for v in new_vertices {
            let x = self.def_point(v);
            let Some((a, b, bind)) = chords.iter().find(|(a, b, _)| {
                let tol = self.tol_for([*a, *b, x]) * 16.;
                let d = *b - *a;
                let t = (x - *a).dot(d) / d.length_squared();
                t > 0. && t < 1. && (*a + d * t).distance(x) <= tol
            }) else {
                continue;
            };
            let Some(i) = bind.curve.interval(*a, x, Some(bind.interval)) else { continue };
            let Ok(target) = bind.curve.point(i[1]) else { continue };
            let _ = b;
            // Every incident face must stay planar with the vertex moved.
            let planar = self.edges_at(v).collect::<Vec<_>>().iter().flat_map(|&e| self.edge_faces(e).to_vec()).all(|f| {
                let ring: Vec<DVec3> = self.face_rings(f)[0]
                    .iter()
                    .map(|&w| if w == v { target } else { self.def_point(w) })
                    .collect();
                let mut n = DVec3::ZERO;
                for k in 0..ring.len() {
                    n += ring[k].cross(ring[(k + 1) % ring.len()]);
                }
                let Some(n) = n.try_normalize() else { return false };
                let tol = self.tol_for(ring.iter().copied()) * 16.;
                ring.iter().all(|p| (*p - ring[0]).dot(n).abs() <= tol)
            });
            if planar {
                self.place_vertex(v, self.model_point(target.to_array()));
                sources.insert(v, bind);
            }
        }
        let mut count = 0;
        for &e in created {
            let ends = *self.state.ends.get(e as usize);
            if let Some(b) = ends.iter().find_map(|v| sources.get(v)) {
                if !edges.contains_key(&e) {
                    edges.insert(e, (*b).clone());
                    count += 1;
                }
            }
            touched_e.insert(e);
        }
        // Kept first pieces whose far end moved.
        for (&v, _) in &sources {
            for e in self.edges_at(v).collect::<Vec<_>>() {
                touched_e.insert(e);
            }
        }
        count
    }

    /// The run's vertices in walking order.
    pub(crate) fn weld_chain(&self, run: &[u32]) -> Option<Vec<u32>> {
        if run.len() < 2 { return None; }
        let mut degree = HashMap::new();
        if run.iter().copied().collect::<HashSet<_>>().len() != run.len() { return None; }
        for &e in run {
            for v in *self.state.ends.get(e as usize) {
                let n = degree.entry(v).or_insert(0);
                *n += 1;
                if *n > 2 { return None; }
            }
        }
        let ends = degree.values().filter(|&&n| n == 1).count();
        if ends != 0 && ends != 2 { return None; }
        let chain = self.chain(run);
        (chain.len() == degree.len()).then_some(chain)
    }

    pub(crate) fn weld_edges(&mut self, run: &[u32], id: i128) -> Result<(), &'static str> {
        let chain = self.weld_chain(run).ok_or("Select one connected edge chain")?;
        let points: Vec<_> = chain.into_iter().map(|v| self.def_point(v)).collect();
        let tol = self.tol_for(points.iter().copied());
        let circle = bound::fit_circle(&points, tol).filter(|c| {
            let mut sweep: f64 = 0.;
            for (a,b) in points.iter().zip(points.iter().cycle().skip(1)).take(run.len()) {
                let Some([u,v]) = c.interval(*a,*b,None) else { return false };
                if sweep * (v-u) < 0. { return false; }
                sweep += v-u;
            }
            sweep.abs() <= std::f64::consts::TAU + 1e-10
        });
        if let Some(circle) = circle {
            self.bind_circle(run, circle)?;
        } else if let Some(curved) = self.state.exact.curved.as_mut() {
            let curved = Arc::make_mut(curved);
            for e in run { curved.edges.remove(e); }
        }
        // Changing the boundary can also invalidate a previously bound wall.
        if let Some(current) = self.state.exact.curved.clone() {
            let incident: HashSet<_> = run.iter().flat_map(|&e| self.edge_faces(e).iter().copied()).collect();
            let invalid: Vec<_> = incident.into_iter().filter(|f| {
                current.face(*f).is_some_and(|s| !self.face_holds(*f,s,&current.edges))
            }).collect();
            if !invalid.is_empty() {
                let curved = Arc::make_mut(self.state.exact.curved.as_mut().unwrap());
                for f in invalid { curved.faces.remove(&f); }
            }
        }
        for &e in run { self.set_edge_curve(e, id); }
        Ok(())
    }

    fn chain(&self, run: &[u32]) -> Vec<u32> {
        let mut at: HashMap<u32, Vec<u32>> = HashMap::new();
        for &e in run {
            for v in *self.state.ends.get(e as usize) {
                at.entry(v).or_default().push(e);
            }
        }
        let start = at
            .iter()
            .filter(|(_, es)| es.len() == 1)
            .map(|(&v, _)| v)
            .min()
            .unwrap_or_else(|| self.state.ends.get(run[0] as usize)[0]);
        let mut out = vec![start];
        let mut used = HashSet::new();
        let mut v = start;
        while let Some(&e) = at.get(&v).and_then(|es| es.iter().find(|e| !used.contains(*e))) {
            used.insert(e);
            let [p, q] = *self.state.ends.get(e as usize);
            v = if p == v { q } else { p };
            if v == start {
                break;
            }
            out.push(v);
        }
        out
    }

    /// A new edge where a bound edge was (a rebuilt push/pull base, a face
    /// rebuilt by plane repair) takes its curve.
    fn same_place(&self, before: &State, old: &Curved, ch: &Change, created: &[u32], edges: &mut EdgeMap) -> usize {
        let key = |p: DVec3, tol: f64| (p / tol).round().as_i64vec3().to_array();
        let mut at: HashMap<([i64; 3], [i64; 3]), &EdgeBind> = HashMap::new();
        // Only edges the step removed or rewrote can have been rebuilt.
        let gone: Vec<(u32, &EdgeBind)> = ch.removed[1]
            .iter()
            .chain(&ch.modified[1])
            .filter_map(|&k| slot_of(k))
            .filter_map(|e| Some((e, old.edge(e)?)))
            .collect();
        let mut tol = 0f64;
        for &(e, _) in &gone {
            let [p, q] = *before.ends.get(e as usize);
            if p == super::keys::NONE {
                continue;
            }
            tol = tol.max(self.tol_for([point(before, p), point(before, q)]) * 16.);
        }
        if tol == 0. {
            return 0;
        }
        for &(e, b) in &gone {
            let [p, q] = *before.ends.get(e as usize);
            if p == super::keys::NONE {
                continue;
            }
            let (a, c) = (key(point(before, p), tol), key(point(before, q), tol));
            at.insert((a.min(c), a.max(c)), b);
        }
        let mut count = 0;
        for &e in created {
            if edges.contains_key(&e) {
                continue;
            }
            let [p, q] = *self.state.ends.get(e as usize);
            let (a, c) = (key(self.def_point(p), tol), key(self.def_point(q), tol));
            if let Some(b) = at.get(&(a.min(c), a.max(c))) {
                edges.insert(e, (*b).clone());
                count += 1;
            }
        }
        count
    }

    /// New edges on a bound cylinder face that are its section by a plane:
    /// the plane of the other face they bound (intersect, push through), or
    /// the plane normal to the axis through both ends (a line around the
    /// surface at one height).
    pub(crate) fn sections(&self, created: &[u32], faces: &FaceMap, edges: &mut EdgeMap) -> usize {
        let mut count = 0;
        for &e in created {
            if edges.contains_key(&e) {
                continue;
            }
            let incident = self.edge_faces(e).to_vec();
            let [p, q] = *self.state.ends.get(e as usize);
            let (a, b) = (self.def_point(p), self.def_point(q));
            let tol = self.tol_for([a, b]);
            'face: for &f in &incident {
                let Some(s) = faces.get(&f) else { continue };
                let Surface::Cylinder { frame, .. } = &s.surface else { continue };
                if !s.contains(a, tol) || !s.contains(b, tol) || s.holds_segment(a, b, tol) {
                    continue;
                }
                let mut planes = Vec::new();
                for &g in &incident {
                    if g != f && !faces.contains_key(&g) {
                        if let Some(pl) = self.exact_face_plane(g) {
                            planes.push(pl);
                        }
                    }
                }
                if ((a - b).dot(frame.axis)).abs() <= tol {
                    planes.push(crate::exact::Frame {
                        origin: a,
                        axis: frame.axis,
                        reference: frame.reference,
                    });
                }
                for pl in planes {
                    let Some(c) = bound::plane_section(s, &pl) else { continue };
                    if c.contains(a, tol) && c.contains(b, tol) {
                        if let Some(interval) = c.interval(a, b, None) {
                            edges.insert(e, EdgeBind { curve: c, interval });
                            count += 1;
                            break 'face;
                        }
                    }
                }
            }
        }
        count
    }

    /// The plane of a planar store face, in definition coordinates.
    pub(crate) fn exact_face_plane(&self, f: u32) -> Option<crate::exact::Frame> {
        let rings = self.face_rings(f);
        let pts: Vec<DVec3> = rings.first()?.iter().map(|&v| self.def_point(v)).collect();
        let mut n = DVec3::ZERO;
        for i in 0..pts.len() {
            n += pts[i].cross(pts[(i + 1) % pts.len()]);
        }
        let axis = n.try_normalize()?;
        let origin = pts[0];
        let reference = (pts[1] - pts[0]).try_normalize()?;
        let reference = (reference - axis * reference.dot(axis)).try_normalize()?;
        Some(crate::exact::Frame {
            origin,
            axis,
            reference,
        })
    }

    /// A curved support for an unbound face: an extrusion of one of its
    /// bound sides along an adjacent straight side, or the revolution of a
    /// side about the axis of a bound circular side. Opposite rails that
    /// are translated or rotated copies of a bound side are bound with it.
    fn derive_face(&self, f: u32, edges: &mut EdgeMap, frozen: &HashSet<u32>) -> Option<Support> {
        let sides = self.sides(f);
        let ring = sides.first()?;
        if sides.len() != 1 || ring.len() < 3 || ring.iter().any(|s| s.edge.is_none()) {
            return None;
        }
        let bound_sides: Vec<(usize, EdgeBind)> = ring
            .iter()
            .enumerate()
            .filter_map(|(i, s)| edges.get(&s.edge?).map(|b| (i, b.clone())))
            .collect();
        if bound_sides.is_empty() {
            return None;
        }
        let n = ring.len();
        let p = |v: u32| self.def_point(v);
        let mut candidates: Vec<(Support, Vec<(u32, EdgeBind)>)> = Vec::new();
        for (i, b) in &bound_sides {
            // Extrusions along the straight neighbours.
            for (j, from) in [((i + 1) % n, ring[*i].b), ((i + n - 1) % n, ring[*i].a)] {
                if edges.contains_key(&ring[j].edge.unwrap()) {
                    continue;
                }
                let other = if ring[j].a == from { ring[j].b } else { ring[j].a };
                let v = p(other) - p(from);
                if let Some(s) = bound::extrusion(b.curve.clone(), v) {
                    let mut rails = self.copies(ring, *i, b, edges, |x| x + v, |c| c.mapped(DMat3::IDENTITY, v));
                    rails.extend(self.section_rails(&s, ring, edges, &rails));
                    candidates.push((s, rails));
                }
            }
            // Revolutions about a circular side's axis.
            if let Curve::Circle { frame, .. } = &b.curve {
                for j in [(i + 1) % n, (i + n - 1) % n] {
                    let e = ring[j].edge.unwrap();
                    let profile = match edges.get(&e) {
                        Some(pb) => pb.curve.clone(),
                        None => {
                            let (a, c) = (p(ring[j].a), p(ring[j].b));
                            match (c - a).try_normalize() {
                                Some(d) => Curve::Line { origin: a, direction: d },
                                None => continue,
                            }
                        }
                    };
                    let Some(s) = bound::revolve(&profile, frame.origin, frame.axis) else { continue };
                    let (o, axis) = (frame.origin, frame.axis);
                    let rails = match edges.get(&e) {
                        Some(pb) => {
                            // The opposite profile copy, rotated about the axis.
                            // The other side along this rail.
                            let k = if j == (i + 1) % n { (i + n - 1) % n } else { (i + 1) % n };
                            self.rotated_copy(ring, j, k, pb, o, axis, edges)
                        }
                        None => Vec::new(),
                    };
                    candidates.push((s, rails));
                }
            }
        }
        for (s, mut rails) in candidates {
            rails.retain(|r| !frozen.contains(&r.0));
            if self.face_holds_with(f, &s, edges, &rails) {
                edges.extend(rails);
                return Some(s);
            }
        }
        None
    }

    /// Unbound sides of a cylinder candidate that are its section by a
    /// Follow Me joint plane or by the plane of the other face they bound
    /// (a mitre, an end cap).
    fn section_rails(&self, s: &Support, ring: &[Side], edges: &EdgeMap, taken: &[(u32, EdgeBind)]) -> Vec<(u32, EdgeBind)> {
        if !matches!(s.surface, Surface::Cylinder { .. }) {
            return Vec::new();
        }
        let joints = JOINTS.with(|j| j.borrow().clone());
        let frame = &self.state.frame;
        let mut out = Vec::new();
        for side in ring {
            let e = side.edge.unwrap();
            if edges.contains_key(&e) || taken.iter().any(|r| r.0 == e) {
                continue;
            }
            let [p, q] = *self.state.ends.get(e as usize);
            let (a, b) = (self.def_point(p), self.def_point(q));
            let tol = self.tol_for([a, b]);
            if !s.contains(a, tol) || !s.contains(b, tol) || s.holds_segment(a, b, tol) {
                continue;
            }
            let mut planes: Vec<crate::exact::Frame> = joints
                .iter()
                .filter_map(|(o, n)| {
                    let axis = frame.definition_delta(*n).try_normalize()?;
                    Some(crate::exact::Frame {
                        origin: DVec3::from(frame.definition_point(o.to_array())),
                        axis,
                        reference: axis.any_orthonormal_vector(),
                    })
                })
                .collect();
            for &g in self.edge_faces(e) {
                if let Some(pl) = self.exact_face_plane(g) {
                    planes.push(pl);
                }
            }
            for pl in planes {
                let Some(c) = bound::plane_section(s, &pl) else { continue };
                if c.contains(a, tol) && c.contains(b, tol) {
                    if let Some(interval) = c.interval(a, b, None) {
                        out.push((e, EdgeBind { curve: c, interval }));
                        break;
                    }
                }
            }
        }
        out
    }

    /// Unbound sides that are copies of bound side `i` under `map`.
    fn copies(
        &self,
        ring: &[Side],
        i: usize,
        b: &EdgeBind,
        edges: &EdgeMap,
        map: impl Fn(DVec3) -> DVec3,
        curve: impl Fn(&Curve) -> Option<Curve>,
    ) -> Vec<(u32, EdgeBind)> {
        let (a0, b0) = (map(self.def_point(ring[i].a)), map(self.def_point(ring[i].b)));
        let Some(c) = curve(&b.curve) else { return Vec::new() };
        let mut out = Vec::new();
        for s in ring {
            let e = s.edge.unwrap();
            if edges.contains_key(&e) {
                continue;
            }
            let (x, y) = (self.def_point(s.a), self.def_point(s.b));
            let tol = self.tol_for([x, y, a0, b0]);
            let same = (x.distance(a0) <= tol && y.distance(b0) <= tol) || (x.distance(b0) <= tol && y.distance(a0) <= tol);
            if same {
                let [p, q] = *self.state.ends.get(e as usize);
                if let Some(interval) = c.interval(self.def_point(p), self.def_point(q), Some(b.interval)) {
                    out.push((e, EdgeBind { curve: c.clone(), interval }));
                }
            }
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn rotated_copy(
        &self,
        ring: &[Side],
        j: usize,
        k: usize,
        pb: &EdgeBind,
        o: DVec3,
        axis: DVec3,
        edges: &EdgeMap,
    ) -> Vec<(u32, EdgeBind)> {
        let Some(e) = ring.get(k).and_then(|s| s.edge) else { return Vec::new() };
        if edges.contains_key(&e) {
            return Vec::new();
        }
        let radial = |q: DVec3| {
            let d = q - o;
            d - axis * d.dot(axis)
        };
        // The copy maps side j's first end onto side k's last (a quad) or,
        // when that end sits on the axis (a pole), its last onto k's first.
        let pairs = [(ring[j].a, ring[k].b), (ring[j].b, ring[k].a)];
        let Some((a, x)) = pairs
            .iter()
            .map(|&(p, q)| (radial(self.def_point(p)), radial(self.def_point(q))))
            .find(|(a, x)| a.length() > 1e-9 && x.length() > 1e-9)
        else {
            return Vec::new();
        };
        let angle = a.cross(x).dot(axis).atan2(a.dot(x));
        let r = DMat3::from_quat(DQuat::from_axis_angle(axis, angle));
        self.copies(ring, j, pb, edges, |q| o + r * (q - o), |c| c.mapped(r, o - r * o))
    }

    /// Curved bindings of one face for display and queries.
    pub(crate) fn curved_face(&self, f: u32) -> Option<&Support> {
        self.state.exact.curved_face(f)
    }
    pub(crate) fn curved_edge(&self, e: u32) -> Option<&EdgeBind> {
        self.state.exact.curved_edge(e)
    }

    /// Bind a run of edges to one circle (scripted drawing, tests). Every
    /// edge's ends must lie on it; nothing changes otherwise.
    pub(crate) fn bind_circle(&mut self, run: &[u32], circle: Curve) -> Result<(), &'static str> {
        let mut edges: EdgeMap = self
            .state
            .exact
            .curved
            .as_deref()
            .map(|c| c.edges.clone())
            .unwrap_or_default();
        for &e in run {
            if !self.alive(Kind::Edge, e) {
                return Err("missing edge");
            }
            let [a, b] = *self.state.ends.get(e as usize);
            let (a, b) = (self.def_point(a), self.def_point(b));
            let tol = self.tol_for([a, b]);
            if !circle.contains(a, tol) || !circle.contains(b, tol) {
                return Err("edge misses its circle");
            }
            let interval = circle.interval(a, b, None).ok_or("degenerate arc")?;
            edges.insert(e, EdgeBind { curve: circle.clone(), interval });
        }
        let faces = self.state.exact.curved.as_deref().map(|c| c.faces.clone()).unwrap_or_default();
        self.state.exact.curved = Some(Arc::new(Curved { edges, faces }));
        self.revision += 1;
        Ok(())
    }
}

impl Store {
    /// Install bindings read from a file, keyed by persistent id. Each one
    /// is checked against its element; one that does not hold is dropped.
    pub(crate) fn install_curved(&mut self, rows: &crate::exact::file::CurvedRows) {
        let edges: Vec<(u32, EdgeBind)> = rows
            .edges
            .iter()
            .filter_map(|(id, b)| Some((self.slot_of_pid(Kind::Edge, *id)?, b.clone())))
            .collect();
        let faces: Vec<(u32, Support)> = rows
            .faces
            .iter()
            .filter_map(|(id, s)| Some((self.slot_of_pid(Kind::Face, *id)?, s.clone())))
            .collect();
        self.restore_split_runs(&rows.split_runs);
        self.install_curved_slots(edges, faces);
    }

    pub(crate) fn install_curved_slots(&mut self, edges: Vec<(u32, EdgeBind)>, faces: Vec<(u32, Support)>) {
        let mut e: EdgeMap = EdgeMap::default();
        for (slot, b) in edges {
            if self.alive(Kind::Edge, slot) {
                if let Some(b) = self.edge_holds(slot, &b) {
                    e.insert(slot, b);
                }
            }
        }
        let mut f: FaceMap = FaceMap::default();
        for (slot, s) in faces {
            if self.alive(Kind::Face, slot) && self.face_holds(slot, &s, &e) {
                f.insert(slot, s);
            }
        }
        self.state.exact.curved = (!e.is_empty() || !f.is_empty()).then(|| {
            Arc::new(Curved { edges: e, faces: f })
        });
        self.revision += 1;
    }
}

impl Store {
    /// Copied elements take their source's exact bindings, mapped by the
    /// affine map the copy's vertices were placed by (Explode, paste into
    /// a definition, loose copies). Each mapped binding is validated on
    /// its copy; one that does not hold stays faceted.
    pub(crate) fn adopt_curved(&mut self, src: &Store, pairs: &[Vec<(u32, u32)>; 3]) {
        let Some(c) = src.state.exact.curved.as_deref() else { return };
        let vertex_pairs: Vec<(DVec3, DVec3)> = pairs[0]
            .iter()
            .filter(|p| p.1 != super::keys::NONE)
            .map(|&(a, b)| (src.def_point(a), self.def_point(b)))
            .collect();
        let tol = self.tol_for(vertex_pairs.iter().flat_map(|p| [p.0, p.1]));
        let Some((m, t)) = bound::fit_affine(&vertex_pairs, tol * 16.) else { return };
        let mut edges: EdgeMap = self
            .state
            .exact
            .curved
            .as_deref()
            .map(|c| c.edges.clone())
            .unwrap_or_default();
        let mut faces: FaceMap = self
            .state
            .exact
            .curved
            .as_deref()
            .map(|c| c.faces.clone())
            .unwrap_or_default();
        let mut changed = false;
        for &(from, to) in &pairs[1] {
            if to == super::keys::NONE || edges.contains_key(&to) {
                continue;
            }
            let Some(b) = c.edge(from) else { continue };
            let Some(curve) = b.curve.mapped(m, t) else { continue };
            if let Some(nb) = self.edge_holds(to, &EdgeBind { curve, interval: b.interval }) {
                edges.insert(to, nb);
                changed = true;
            }
        }
        for &(from, to) in &pairs[2] {
            if to == super::keys::NONE || faces.contains_key(&to) {
                continue;
            }
            let Some(s) = c.face(from).and_then(|s| s.mapped(m, t)) else { continue };
            if self.face_holds(to, &s, &edges) {
                faces.insert(to, s);
                changed = true;
            }
        }
        if changed {
            self.state.exact.curved = Some(Arc::new(Curved { edges, faces }));
        }
    }
}

impl Store {
    /// A bound curved face holding `p` (definition coordinates): on its
    /// facet, or on its support within the display chord (a pick lands on
    /// the drawn surface), inside the face's parameter region.
    fn curved_face_at(&self, p: DVec3) -> Option<(u32, Support, glam::DVec2)> {
        let c = self.state.exact.curved.as_deref()?;
        let periodic = |s: &Support| s.periodic();
        for (&f, s) in c.faces.iter() {
            if !self.alive(Kind::Face, f) {
                continue;
            }
            let ring: Vec<DVec3> = self.face_rings(f)[0].iter().map(|&v| self.def_point(v)).collect();
            let (lo, hi) = ring.iter().fold((ring[0], ring[0]), |(lo, hi), q| (lo.min(*q), hi.max(*q)));
            let reach = 0.1 * (hi - lo).length() + 1e-9;
            if (p - p.clamp(lo - DVec3::splat(reach), hi + DVec3::splat(reach))).length() > 0. {
                continue;
            }
            let Some(uv) = s.uv(p) else { continue };
            let Some(q) = s.point(uv) else { continue };
            // Within the drawn chord of the support (or on the facet).
            let chord = 4. * s.size_hint() * (1. - (SURFACE_PATH_STEP * 0.5).cos());
            let on_surface = q.distance(p) <= chord.max(2e-3 * (hi - lo).length());
            let on_facet = self
                .exact_face_plane(f)
                .is_some_and(|pl| (p - pl.origin).dot(pl.axis).abs() <= self.tol_for(ring.iter().copied().chain([p])) * 1e3);
            if !on_surface && !on_facet {
                continue;
            }
            // Unwrap the ring along itself, then bring it next to `uv`.
            let per = periodic(s);
            let mut poly: Vec<glam::DVec2> = Vec::new();
            for &v in &ring {
                let Some(mut w) = s.uv(v) else { continue };
                if let Some(prev) = poly.last() {
                    for k in 0..2 {
                        if per[k] {
                            w[k] = prev[k] + wrap(w[k] - prev[k]);
                        }
                    }
                }
                poly.push(w);
            }
            if poly.len() < 3 {
                continue;
            }
            let centre = poly.iter().sum::<glam::DVec2>() / poly.len() as f64;
            for k in 0..2 {
                if per[k] {
                    let shift = uv[k] - centre[k] - wrap(uv[k] - centre[k]);
                    for w in &mut poly {
                        w[k] += shift;
                    }
                }
            }
            let inside = poly
                .iter()
                .zip(poly.iter().cycle().skip(1))
                .filter(|(a, b)| (a.y > uv.y) != (b.y > uv.y) && uv.x < a.x + (uv.y - a.y) * (b.x - a.x) / (b.y - a.y))
                .count()
                % 2
                == 1;
            if inside {
                return Some((f, s.clone(), uv));
            }
        }
        None
    }

    /// The point of face `f`'s facet under the support point at `uv`: along
    /// the surface normal, so the store's planar face can be split there.
    fn on_facet(&self, f: u32, s: &Support, uv: glam::DVec2) -> Option<DVec3> {
        let plane = self.exact_face_plane(f)?;
        let p = s.point(uv)?;
        let n = s.normal(uv)?;
        let den = n.dot(plane.axis);
        if den.abs() < 1e-9 {
            return None;
        }
        Some(p + n * ((plane.origin - p).dot(plane.axis) / den))
    }

    /// The line from `a` to `b` drawn over the curved face under `a`: the
    /// support's straight parameter line between them, as a polyline in
    /// store coordinates. Its ends lie on the facets under them and its
    /// corners where it crosses the facets' generators (on the exact
    /// surface). A path within 10° of a parameter line snaps onto it, so a
    /// line around a cylinder is its circle and one along it a generator,
    /// both exact. None when `a` and `b` are not on one curved support.
    pub(crate) fn surface_path(&self, a: [f64; 3], b: [f64; 3]) -> Option<SurfacePath> {
        let frame = &self.state.frame;
        let (da, db) = (DVec3::from(frame.definition_point(a)), DVec3::from(frame.definition_point(b)));
        let Some((fa, s, ua)) = self.curved_face_at(da) else {
            debug(|| format!("surface path: no curved face at {da}"));
            return None;
        };
        let Some((fb, sb, ub)) = self.curved_face_at(db) else {
            debug(|| format!("surface path: no curved face at {db}"));
            return None;
        };
        if sb != s {
            return None;
        }
        let periodic = s.periodic();
        let lift = |uv: glam::DVec2, near: glam::DVec2| {
            let mut uv = uv;
            for k in 0..2 {
                if periodic[k] {
                    uv[k] = near[k] + wrap(uv[k] - near[k]);
                }
            }
            uv
        };
        let mut ub = lift(ub, ua);
        let (du, dv) = s.derivatives_at((ua + ub) * 0.5)?;
        let (mu, mv) = ((ub.x - ua.x).abs() * du.length(), (ub.y - ua.y).abs() * dv.length());
        let snap = (10f64).to_radians().tan();
        if mv < snap * mu {
            ub.y = ua.y;
        } else if mu < snap * mv {
            ub.x = ua.x;
        }
        let _ = (fb, self.on_facet(fa, &s, ua));
        // Every crossing of the parameter line (extended both ways) with a
        // straight side of a facet of this support: those points lie on the
        // surface and on the facets.
        let c = self.state.exact.curved.as_deref()?;
        let mut hits: Vec<(f64, DVec3, u32)> = Vec::new();
        let mut seen = HashSet::new();
        for (f, fs) in c.faces.iter() {
            if *fs != s || !self.alive(Kind::Face, *f) {
                continue;
            }
            for side in self.sides(*f).into_iter().flatten() {
                let Some(e) = side.edge else { continue };
                if self.curved_edge(e).is_some() {
                    continue;
                }
                let (p, q) = (self.def_point(side.a), self.def_point(side.b));
                let (Some(up), Some(uq)) = (s.uv(p), s.uv(q)) else { continue };
                let (up, uq) = (lift(up, ua), lift(uq, ua));
                let (r, d) = (ub - ua, uq - up);
                let den = r.perp_dot(d);
                if den.abs() <= 1e-15 {
                    continue;
                }
                let t = (up - ua).perp_dot(d) / den;
                let w = (up - ua).perp_dot(r) / den;
                if (-1e-12..=1. + 1e-12).contains(&w) && seen.insert((e, (t * 1e9).round() as i64)) {
                    hits.push((t, p.lerp(q, w.clamp(0., 1.)), e));
                }
            }
        }
        hits.sort_by(|x, y| x.0.total_cmp(&y.0));
        // The ends move to the nearest such crossing of the facet under
        // them: only there can a line on a faceted support be exact.
        let nearest = |target: f64, face: u32| {
            hits.iter()
                .filter(|h| self.edge_faces(h.2).contains(&face))
                .min_by(|x, y| (x.0 - target).abs().total_cmp(&(y.0 - target).abs()))
                .copied()
        };
        let fb_now = self.curved_face_at(s.point(ub)?).map_or(fb, |x| x.0);
        let (Some((t0, start, _)), Some((t1, end, _))) = (nearest(0., fa), nearest(1., fb_now)) else {
            debug(|| format!("surface path: no generator crossing ({} hits)", hits.len()));
            return None;
        };
        if t1 - t0 <= 1e-9 {
            debug(|| "surface path: ends on one generator".into());
            return None;
        }
        let mut path = vec![start];
        for h in &hits {
            if h.0 > t0 + 1e-9 && h.0 < t1 - 1e-9 && path.last().is_none_or(|p| p.distance(h.1) > self.tol_for([h.1])) {
                path.push(h.1);
            }
        }
        path.push(end);
        let to_store = |p: DVec3| self.model_point(p.to_array());
        let n = (((t1 - t0) * (ub - ua).max_element().abs().max((ub - ua).min_element().abs())) / SURFACE_PATH_STEP)
            .ceil()
            .clamp(1., 2048.) as usize;
        let curve: Vec<[f64; 3]> = (0..=n)
            .filter_map(|k| {
                let t = t0 + (t1 - t0) * k as f64 / n as f64;
                s.point(ua.lerp(ub, t)).map(to_store)
            })
            .collect();
        Some(SurfacePath {
            path: path.into_iter().map(to_store).collect(),
            curve,
        })
    }
}

/// A line over a curved face: the polyline to commit and the exact curve
/// to preview.
pub(crate) struct SurfacePath {
    pub path: Vec<[f64; 3]>,
    pub curve: Vec<[f64; 3]>,
}

fn wrap(t: f64) -> f64 {
    let r = t.rem_euclid(std::f64::consts::TAU);
    if r > std::f64::consts::PI {
        r - std::f64::consts::TAU
    } else {
        r
    }
}
