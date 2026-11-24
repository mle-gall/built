//! The arcs (R2.5 slice 8): 2-point, 3-point, centre and pie.
//!
//! From `tools/arc.py`, issue numbers kept with the rules:
//! - Every arc lives on the drawing plane (the machine's own `PlaneLock`);
//!   a point snapped off it turns the arc to the plane its points ask for.
//! - A number typed before the first click is the segment count
//!   (unitless); «Ns» sets it at any moment, and right after a 2-point arc
//!   rebuilds that arc in place, one undo step.
//! - 2-point: start, end (the chord), then the bulge. The bulge snaps to
//!   the arc tangent to the curve or edge the start sits on (cyan), and to
//!   a half circle. Started on an edge's interior the preview is tangent to
//!   that edge; at the point of an adjacent edge as far from their corner
//!   it turns magenta, the fillet (#43): a typed number is then its radius,
//!   a click or double-click draws it and trims the corner when exactly the
//!   two edges meet there on a face (Alt keeps it). A double-click near
//!   another such corner repeats the last fillet; a corner already rounded
//!   is measured from where its lines meet (#1354). «2r» types a radius.
//! - 3-point: the circle through the three clicks.
//! - Centre arc: centre, the 0° arm (radius), the sweep; sampled every 15°
//!   (the circle's lattice) unless a count was typed. Pie closes the wedge.
//!
//! The topology the 2-point arc reads (the edge under the start, a
//! corner's edges, curves' tangents, two-edge corners) and the projection
//! to pixels are the host's (`ToolHost::call`); the geometry is here. The
//! commit is the protocol's `add_arc`.

use std::f64::consts::PI;
use std::sync::atomic::{AtomicU64, Ordering};

use glam::DVec3;
use serde_json::{json, Value as J};

use super::plane::{plane_axes, Axes, Plane, PlaneLock};
use super::preview::ValueAnchor;
use super::{Command, Preview, Reply, Tool, ToolEvent, ToolHost, ToolInfo, Value};

const SEGMENTS: usize = 16;
const MIN_SEGMENTS: i64 = 2;
/// Within this many pixels of the tangent bulge the arc snaps to it.
const TANGENT_PX: f64 = 8.0;
/// Within this many pixels of the equidistant point: the fillet.
const FILLET_PX: f64 = 10.0;
/// A double-click this close to a two-edge corner repeats the last fillet.
const CORNER_PX: f64 = 20.0;
const PITCH_DEG: f64 = 15.0;

/// The corner distance of the last fillet, shared by every arc (a
/// double-click near another corner repeats it). NaN: none yet.
static LAST_FILLET_D: AtomicU64 = AtomicU64::new(0x7ff8_0000_0000_0000);

pub fn last_fillet_d() -> Option<f64> {
    let d = f64::from_bits(LAST_FILLET_D.load(Ordering::Relaxed));
    (!d.is_nan()).then_some(d)
}

pub fn set_last_fillet_d(d: Option<f64>) {
    LAST_FILLET_D.store(d.unwrap_or(f64::NAN).to_bits(), Ordering::Relaxed);
}

/// `ArcTool.last_fillet_d` in Python: the distance a double-click repeats.
#[pyo3::pyfunction]
#[pyo3(name = "arc_last_fillet_d", signature = (value=None, set=false))]
pub fn py_last_fillet_d(value: Option<f64>, set: bool) -> Option<f64> {
    if set {
        set_last_fillet_d(value);
    }
    last_fillet_d()
}

fn info(kind: &'static str, name: &'static str, shortcut: &'static str, description: &'static str,
        vcb: &'static str) -> ToolInfo {
    ToolInfo {
        kind, name, shortcut, description, uses_snap: true, box_select: false, vcb_label: Some(vcb),
        // The axis magnet is `tools.base.AxisMagnet`'s, read per hover.
        magnetic_axis_deg: None, screen_axis_px: None, axis_source: true, axis_source_empty_only: true,
    }
}

static ARC: std::sync::LazyLock<ToolInfo> = std::sync::LazyLock::new(|| {
    info("arc", "Arc", "A", "Draw an arc from its two ends, then pull out its bulge.", "Bulge")
});
static ARC3: std::sync::LazyLock<ToolInfo> = std::sync::LazyLock::new(|| {
    info("arc3", "3-Point Arc", "J", "Draw an arc that passes through three points.", "Segments")
});
static CENTER_ARC: std::sync::LazyLock<ToolInfo> = std::sync::LazyLock::new(|| {
    // Shift+O: plain O is Orbit (tests/test_shortcuts.py).
    info("center_arc", "Center Arc", "Shift+O",
         "Draw an arc from its centre: the centre, where the arc starts, then the angle it sweeps.", "Angle")
});
static PIE: std::sync::LazyLock<ToolInfo> = std::sync::LazyLock::new(|| {
    info("pie", "Pie", "", "Draw an arc from its centre whose two radii close it into a slice-shaped face.", "Angle")
});

fn d3(p: [f64; 3]) -> DVec3 { DVec3::from_array(p) }
fn j3(v: DVec3) -> J { json!([v.x, v.y, v.z]) }
fn p3(v: DVec3) -> [f64; 3] { [v.x, v.y, v.z] }
fn jp(v: &J) -> Option<DVec3> { super::json_p3(v).map(d3) }

// ---- Geometry -------------------------------------------------------------

fn circumcenter2(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> Option<(f64, f64)> {
    let (ax, ay) = a;
    let (bx, by) = b;
    let (cx, cy) = c;
    let d = 2.0 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by));
    if d.abs() < 1e-12 {
        return None;
    }
    let (a2, b2, c2) = (ax * ax + ay * ay, bx * bx + by * by, cx * cx + cy * cy);
    let ux = (a2 * (by - cy) + b2 * (cy - ay) + c2 * (ay - by)) / d;
    let uy = (a2 * (cx - bx) + b2 * (ax - cx) + c2 * (bx - ax)) / d;
    Some((ux, uy))
}

/// Where the line `a0`–`a1` meets `b0`–`b1`, or None (parallel, skew).
pub fn lines_meet(a0: DVec3, a1: DVec3, b0: DVec3, b1: DVec3, tol: f64) -> Option<DVec3> {
    let (u, w, r) = (a1 - a0, b1 - b0, a0 - b0);
    let (uu, ww, uw) = (u.dot(u), w.dot(w), u.dot(w));
    let den = uu * ww - uw * uw;
    if uu < 1e-18 || ww < 1e-18 || den.abs() <= 1e-12 * uu * ww {
        return None;
    }
    let s = (uw * w.dot(r) - ww * u.dot(r)) / den;
    let t = (uu * w.dot(r) - uw * u.dot(r)) / den;
    let (p, q) = (a0 + u * s, b0 + w * t);
    let size = uu.sqrt().max(ww.sqrt()).max(1.0);
    if (p - q).length() > tol * size {
        return None;
    }
    Some((p + q) * 0.5)
}

fn wrap(mut a: f64) -> f64 {
    while a <= -PI {
        a += 2.0 * PI;
    }
    while a > PI {
        a -= 2.0 * PI;
    }
    a
}

/// The arc from `s` to `e` through `m` (their circumcircle), sampled into
/// `segments` spans; the chord when the three are in a line.
pub fn arc_3pts_2d(s: (f64, f64), m: (f64, f64), e: (f64, f64), segments: usize) -> Vec<(f64, f64)> {
    let Some((cx, cy)) = circumcenter2(s, m, e) else { return vec![s, e] };
    let r = (s.0 - cx).hypot(s.1 - cy);
    let a0 = (s.1 - cy).atan2(s.0 - cx);
    let a1 = (e.1 - cy).atan2(e.0 - cx);
    let am = (m.1 - cy).atan2(m.0 - cx);
    let mut d = wrap(a1 - a0);
    let da = wrap(am - a0);
    if d >= 0.0 && !(0.0 <= da && da <= d) {
        d -= 2.0 * PI;
    } else if d < 0.0 && !(d <= da && da <= 0.0) {
        d += 2.0 * PI;
    }
    (0..=segments)
        .map(|k| {
            let t = a0 + d * (k as f64 / segments as f64);
            (cx + r * t.cos(), cy + r * t.sin())
        })
        .collect()
}

/// Python's `round()`: halves to even.
fn py_round(x: f64) -> f64 { x.round_ties_even() }

fn polyline(pts: &[DVec3]) -> Vec<([f64; 3], [f64; 3])> {
    pts.windows(2).map(|w| (p3(w[0]), p3(w[1]))).collect()
}

// ---- What the planar arcs share ------------------------------------------

#[derive(Default)]
struct Planar {
    work_plane: Option<Plane>,
    lock: PlaneLock,
    axes: Option<Axes>,
    names_point: Option<bool>,
}

impl Planar {
    fn ax(&self) -> Axes { self.axes.unwrap_or_else(Axes::world) }
    fn drawing(&self, points: &[DVec3]) -> Plane {
        self.lock.drawing(self.work_plane, points, self.names_point, &self.ax())
    }
    fn uv(&self, points: &[DVec3]) -> (DVec3, DVec3) { plane_axes(self.drawing(points).1, &self.ax()) }
    /// A point that is not the last was clicked: the plane its points ask
    /// for becomes the captured one.
    fn adopt_snapped(&mut self, points: &[DVec3]) {
        let ax = self.ax();
        let own = self.lock.own_plane(self.work_plane, &ax);
        if let Some(p) = self.lock.snapped(own, points, self.names_point, &ax) {
            self.work_plane = Some(p);
        }
    }
    fn first_click(&mut self, world: DVec3) {
        if self.work_plane.is_none() {
            self.work_plane = self.lock.locked(world, &self.ax());
        }
    }
    fn reset(&mut self) {
        self.work_plane = None;
        self.lock.hover = None;
        self.lock.clear();
    }
    fn event(&mut self, ev: &ToolEvent) -> bool {
        match ev {
            ToolEvent::WorkPlane(plane) => self.work_plane = plane.map(|(p, n)| (d3(p), d3(n))),
            ToolEvent::Frame { axes, names_point } => {
                self.axes = Some(Axes { o: d3(axes[0]), x: d3(axes[1]), y: d3(axes[2]), z: d3(axes[3]) });
                self.names_point = *names_point;
            }
            _ => return false,
        }
        true
    }
    fn set(&mut self, key: &str, value: &J) -> bool {
        match key {
            "work_plane" => {
                self.work_plane = value.as_array().and_then(|a| Some((jp(a.first()?)?, jp(a.get(1)?)?)));
                true
            }
            _ => false,
        }
    }
}

fn plane_json(p: Option<Plane>) -> J { p.map(|(a, n)| json!([p3(a), p3(n)])).unwrap_or(J::Null) }
fn pt_json(p: Option<DVec3>) -> J { p.map(j3).unwrap_or(J::Null) }

/// «Ns» or a number before the first click: the count, said.
fn set_segments(n: f64, host: &mut dyn ToolHost) -> Option<usize> {
    let n = py_round(n);
    if !(n >= MIN_SEGMENTS as f64) {
        return None;
    }
    host.message("{n} segments", &[("n", format!("{}", n as i64))]);
    Some(n as usize)
}

/// Pixels from the cursor to each point (None: the view cannot project).
fn pixel_distances(host: &mut dyn ToolHost, px: [f64; 2], points: &[DVec3]) -> Vec<Option<f64>> {
    let answer = host.call("to_px", json!({"points": points.iter().map(|p| j3(*p)).collect::<Vec<_>>()}));
    let rows = answer.as_array().cloned().unwrap_or_default();
    (0..points.len())
        .map(|i| {
            let q = rows.get(i)?.as_array()?;
            Some((q.first()?.as_f64()? - px[0]).hypot(q.get(1)?.as_f64()? - px[1]))
        })
        .collect()
}

/// Metres per pixel at `point` along `along` (None: no projection).
fn world_per_pixel(host: &mut dyn ToolHost, point: DVec3, along: DVec3) -> Option<f64> {
    let b = point + along.normalize_or_zero() * 0.25;
    let answer = host.call("to_px", json!({"points": [j3(point), j3(b)]}));
    let rows = answer.as_array()?;
    let q = |i: usize| -> Option<(f64, f64)> {
        let r = rows.get(i)?.as_array()?;
        Some((r.first()?.as_f64()?, r.get(1)?.as_f64()?))
    };
    let (a, b) = (q(0)?, q(1)?);
    let d = (b.0 - a.0).hypot(b.1 - a.1);
    (d > 1e-6).then(|| 0.25 / d)
}

/// An edge as the host names it.
#[derive(Clone, Debug, PartialEq)]
struct Edge {
    id: u64,
    a: DVec3,
    b: DVec3,
    faces: bool,
}

impl Edge {
    fn of(v: &J) -> Option<Edge> {
        Some(Edge { id: v.get("id")?.as_u64()?, a: jp(v.get("a")?)?, b: jp(v.get("b")?)?,
                    faces: v.get("faces").and_then(J::as_bool).unwrap_or(false) })
    }
}

/// What a corner of the start edge offers the fillet: an adjacent edge `b`
/// leaving the corner `v` toward `far`.
#[derive(Clone, Debug)]
struct Adjacent {
    v: DVec3,
    b: Edge,
    far: DVec3,
}

/// A corner an arc already rounds (#1354): `x` where the lines meet, `b`
/// the straight edge after the curve, `near`/`far` its ends.
#[derive(Clone, Debug)]
struct Rounded {
    x: DVec3,
    b: Edge,
    near: DVec3,
    far: DVec3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind { Tangent, Fillet, Half }

impl Kind {
    fn name(self) -> &'static str {
        match self { Kind::Tangent => "tangent", Kind::Fillet => "fillet", Kind::Half => "half" }
    }
}

/// The 2-point arc just drawn, for «Ns»: its apex and chord in the plane
/// `(u, v)` at `origin`, the trim, and the retype token.
#[derive(Clone, Debug)]
struct LastArc {
    apex: (f64, f64),
    e2: (f64, f64),
    u: DVec3,
    v: DVec3,
    origin: DVec3,
    trim: Option<Vec<(DVec3, DVec3)>>,
    token: Option<u64>,
}

type Apex = ((f64, f64), (f64, f64), (DVec3, DVec3, DVec3));

// ---- 2-point arc ------------------------------------------------------------

pub struct ArcTool {
    p: Planar,
    segments: usize,
    start: Option<DVec3>,
    end: Option<DVec3>,
    hover: Option<DVec3>,
    tangent_dir: Option<DVec3>,
    snap_bulge: Option<f64>,
    kind: Option<Kind>,
    /// The cue the rubber band shows (cyan, magenta, the half circle).
    cue: Option<Kind>,
    /// The edge the start sits on, and the start.
    start_edge: Option<(Edge, DVec3)>,
    /// That edge's corners: adjacent edges and corners already rounded.
    adjacent: Vec<Adjacent>,
    rounded: Vec<Rounded>,
    /// End phase: the equidistant point the cursor is on, `(V, P2, B)`.
    equidistant: Option<(DVec3, DVec3, Edge)>,
    /// After the end click: the corner this arc rounds, `(V, P1, P2)`.
    fillet: Option<(DVec3, DVec3, DVec3)>,
    fillet_edge_b: Option<Edge>,
    last: Option<LastArc>,
}

impl Default for ArcTool {
    fn default() -> Self {
        ArcTool {
            p: Planar::default(), segments: SEGMENTS, start: None, end: None, hover: None, tangent_dir: None,
            snap_bulge: None, kind: None, cue: None, start_edge: None, adjacent: vec![], rounded: vec![],
            equidistant: None, fillet: None, fillet_edge_b: None, last: None,
        }
    }
}

impl ArcTool {
    fn plane_points(&self) -> Vec<DVec3> {
        let Some(s) = self.start else { return vec![] };
        std::iter::once(s).chain(self.end.or(self.hover)).collect()
    }

    fn uv(&self) -> (DVec3, DVec3) { self.p.uv(&self.plane_points()) }

    fn to2(&self, p: DVec3, u: DVec3, v: DVec3) -> (f64, f64) {
        let d = p - self.start.unwrap_or(DVec3::ZERO);
        (d.dot(u), d.dot(v))
    }

    /// Signed distance from the chord's midpoint to the cursor.
    fn bulge_for(&self, cursor: DVec3) -> f64 {
        let Some(end) = self.end else { return 0.0 };
        let (u, v) = self.uv();
        let e2 = self.to2(end, u, v);
        let length = e2.0.hypot(e2.1);
        if length < 1e-9 {
            return 0.0;
        }
        let (px, py) = (-e2.1 / length, e2.0 / length);
        let mid = (e2.0 / 2.0, e2.1 / 2.0);
        let c2 = self.to2(cursor, u, v);
        (c2.0 - mid.0) * px + (c2.1 - mid.1) * py
    }

    /// The arc's points for a bulge (given, snapped, else the cursor's)
    /// to `end` (default the end point), and its apex.
    fn points(&self, cursor: Option<DVec3>, bulge: Option<f64>, end: Option<DVec3>) -> (Vec<DVec3>, Option<Apex>) {
        let (Some(start), Some(end)) = (self.start, end.or(self.end)) else { return (vec![], None) };
        let (u, v) = self.uv();
        let e2 = self.to2(end, u, v);
        let length = e2.0.hypot(e2.1);
        if length < 1e-9 {
            return (vec![], None);
        }
        let h = bulge.or(self.snap_bulge).unwrap_or_else(|| cursor.map(|c| self.bulge_for(c)).unwrap_or(0.0));
        if h.abs() < 1e-4 {
            return (vec![start, end], None); // flat: the chord
        }
        let (px, py) = (-e2.1 / length, e2.0 / length);
        let mid = (e2.0 / 2.0, e2.1 / 2.0);
        let apex = (mid.0 + px * h, mid.1 + py * h);
        let pts = arc_3pts_2d((0.0, 0.0), apex, e2, self.segments)
            .into_iter()
            .map(|(x, y)| start + u * x + v * y)
            .collect();
        (pts, Some((apex, e2, (u, v, start))))
    }

    /// The bulge that leaves the start along `tangent` toward `end`: the
    /// sagitta (L/2)·tan(a/2), on the tangent's side.
    fn tangent_bulge(&self, end: Option<DVec3>, tangent: Option<DVec3>) -> Option<f64> {
        let tangent = tangent.or(self.tangent_dir)?;
        let end = end.or(self.end)?;
        let (u, v) = self.uv();
        let e2 = self.to2(end, u, v);
        let length = e2.0.hypot(e2.1);
        if length < 1e-9 {
            return None;
        }
        let (cx, cy) = (e2.0 / length, e2.1 / length);
        let (tx, ty) = (tangent.dot(u), tangent.dot(v));
        let tl = tx.hypot(ty);
        if tl < 1e-6 {
            return None; // the tangent leaves the plane
        }
        let (tx, ty) = (tx / tl, ty / tl);
        let angle = (cx * ty - cy * tx).atan2(cx * tx + cy * ty);
        if (angle.abs() - PI).abs() < 1e-3 {
            return None; // doubling back
        }
        Some((length / 2.0) * (angle / 2.0).tan())
    }

    /// The direction the arc leaves the start along its edge, toward `toward`.
    fn edge_tangent(&self, toward: DVec3) -> Option<DVec3> {
        let (edge, p1) = self.start_edge.as_ref()?;
        let d = edge.b - edge.a;
        if d.length() < 1e-9 {
            return None;
        }
        let d = d.normalize();
        Some(if d.dot(toward - *p1) < 0.0 { -d } else { d })
    }

    fn load_corners(&mut self, host: &mut dyn ToolHost) {
        self.adjacent.clear();
        self.rounded.clear();
        let Some((edge, _)) = self.start_edge.clone() else { return };
        let answer = host.call("corner_edges", json!({"edge": edge.id}));
        for row in answer.as_array().into_iter().flatten() {
            if let (Some(v), Some(b), Some(far)) = (row.get("v").and_then(jp), row.get("b").and_then(Edge::of),
                                                     row.get("far").and_then(jp)) {
                self.adjacent.push(Adjacent { v, b, far });
            }
        }
        let answer = host.call("rounded_corners", json!({"edge": edge.id}));
        for row in answer.as_array().into_iter().flatten() {
            let get = |k: &str| row.get(k).and_then(jp);
            let (Some(v), Some(other), Some(v2), Some(far), Some(b)) =
                (get("v"), get("edge_far"), get("v2"), get("far"), row.get("b").and_then(Edge::of)) else { continue };
            let Some(x) = lines_meet(v, other, v2, far, 1e-6) else { continue };
            // The corner lies past the curve's ends, on the lines'
            // continuations away from the edges.
            if (x - v).dot(v - other) < -1e-9 || (x - v2).dot(v2 - far) < -1e-9 {
                continue;
            }
            self.rounded.push(Rounded { x, b, near: v2, far });
        }
    }

    /// `(V, P2, B)`: the point of an edge next to the start edge as far
    /// from their corner as the start, within `FILLET_PX` of the cursor.
    fn equidistant_near(&self, px: [f64; 2], host: &mut dyn ToolHost) -> Option<(DVec3, DVec3, Edge)> {
        let (edge, p1) = self.start_edge.as_ref()?;
        let mut cands: Vec<(DVec3, DVec3, Edge)> = vec![];
        for v in [edge.a, edge.b] {
            let d = (*p1 - v).length();
            if d < 1e-6 {
                continue;
            }
            for adj in self.adjacent.iter().filter(|a| a.v == v) {
                let span = (adj.far - v).length();
                if span < 1e-9 || d > span + 1e-6 {
                    continue;
                }
                cands.push((v, v + (adj.far - v) * (d / span), adj.b.clone()));
            }
        }
        for r in &self.rounded {
            let d = (*p1 - r.x).length();
            let span = (r.far - r.x).length();
            if span < 1e-9 || d < (r.near - r.x).length() - 1e-6 || d > span + 1e-6 {
                continue;
            }
            cands.push((r.x, r.x + (r.far - r.x) * (d / span), r.b.clone()));
        }
        if cands.is_empty() {
            return None;
        }
        let dists = pixel_distances(host, px, &cands.iter().map(|c| c.1).collect::<Vec<_>>());
        let mut best: Option<(f64, usize)> = None;
        for (i, dist) in dists.into_iter().enumerate() {
            let Some(dist) = dist.filter(|d| *d <= FILLET_PX) else { continue };
            if best.is_none_or(|(b, _)| dist < b) {
                best = Some((dist, i));
            }
        }
        best.map(|(_, i)| cands.swap_remove(i))
    }

    /// The two corner stubs a fillet leaves, to erase: only for the fillet
    /// itself, on a face, where exactly two edges meet; never with Alt.
    fn corner_trim(&self, alt: bool, host: &mut dyn ToolHost) -> Option<Vec<(DVec3, DVec3)>> {
        if self.kind != Some(Kind::Fillet) || alt {
            return None;
        }
        let (v, p1, p2) = self.fillet?;
        let (edge, _) = self.start_edge.as_ref()?;
        if !edge.faces {
            return None;
        }
        if host.call("vertex_degree", json!({"point": j3(v)})).as_u64() != Some(2) {
            return None;
        }
        Some(vec![(v, p1), (v, p2)])
    }

    fn reset(&mut self) {
        self.start = None;
        self.end = None;
        self.p.reset();
        self.tangent_dir = None;
        self.snap_bulge = None;
        self.kind = None;
        self.cue = None;
        self.start_edge = None;
        self.adjacent.clear();
        self.rounded.clear();
        self.equidistant = None;
        self.fillet = None;
        self.fillet_edge_b = None;
    }

    fn commit(&mut self, pts: Vec<DVec3>, apex: Option<Apex>, trim: Option<Vec<(DVec3, DVec3)>>,
              host: &mut dyn ToolHost) -> Reply {
        if trim.is_some() {
            if let Some((v, p1, _)) = self.fillet {
                set_last_fillet_d(Some((p1 - v).length()));
            }
        }
        let answer = run_arc(host, &pts, None, trim.as_deref());
        self.last = match apex {
            Some((apex, e2, (u, v, origin))) if pts.len() >= 3 => {
                Some(LastArc { apex, e2, u, v, origin, trim, token: answer.ok.then_some(answer.revision) })
            }
            _ => None,
        };
        self.reset();
        Reply::committed()
    }

    fn click(&mut self, world: DVec3, alt: bool, host: &mut dyn ToolHost) -> Reply {
        let Some(start) = self.start else {
            self.last = None;
            self.start = Some(world);
            self.p.first_click(world);
            let edge = host.call("edge_under", json!({"point": j3(world)}));
            self.start_edge = Edge::of(&edge).map(|e| (e, world));
            self.load_corners(host);
            return Reply::redraw();
        };
        if self.end.is_none() {
            let end = self.equidistant.as_ref().map(|e| e.1).unwrap_or(world);
            if (end - start).length() < 1e-6 {
                return Reply::ignored();
            }
            self.end = Some(end);
            let pts = self.plane_points();
            self.p.adopt_snapped(&pts); // the bulge is read on the arc's plane
            self.fillet = None;
            self.fillet_edge_b = None;
            if let Some((v, p2, b)) = self.equidistant.take() {
                self.fillet = Some((v, start, p2));
                self.fillet_edge_b = Some(b);
            }
            let tangent = host.call("tangent_at", json!({"point": j3(start)}));
            self.tangent_dir = jp(&tangent).or_else(|| self.edge_tangent(end));
            return Reply::redraw();
        }
        let (pts, apex) = self.points(Some(world), None, None);
        if pts.len() < 2 {
            return Reply::redraw();
        }
        let trim = self.corner_trim(alt, host);
        self.commit(pts, apex, trim, host)
    }

    fn double_click(&mut self, world: DVec3, px: [f64; 2], alt: bool, host: &mut dyn ToolHost) -> Reply {
        if self.start.is_some() && self.end.is_some() && self.fillet.is_some() {
            if let Some(h) = self.tangent_bulge(None, None) {
                self.snap_bulge = Some(h);
                self.kind = Some(Kind::Fillet);
                let (pts, apex) = self.points(None, Some(h), None);
                if pts.len() >= 2 {
                    let trim = self.corner_trim(alt, host);
                    return self.commit(pts, apex, trim, host);
                }
            }
        }
        if self.start.is_some() && self.end.is_none() {
            if let Some(d) = last_fillet_d().filter(|d| *d != 0.0) {
                if let Some(corner) = self.corner_near(px, d, host) {
                    self.reset();
                    return self.round_corner(corner, d, alt, host);
                }
            }
        }
        self.click(world, alt, host)
    }

    /// `(V, A, B, A's far end, B's far end)`: a two-edge corner on a face
    /// within `CORNER_PX`, both edges long enough for the fillet.
    fn corner_near(&self, px: [f64; 2], d: f64, host: &mut dyn ToolHost) -> Option<(DVec3, Edge, Edge, DVec3, DVec3)> {
        let answer = host.call("two_edge_corners", json!({"min_length": d - 1e-6}));
        let rows: Vec<_> = answer.as_array().into_iter().flatten().filter_map(|r| {
            Some((jp(r.get("v")?)?, Edge::of(r.get("a")?)?, Edge::of(r.get("b")?)?,
                  jp(r.get("a_far")?)?, jp(r.get("b_far")?)?))
        }).collect();
        if rows.is_empty() {
            return None;
        }
        let dists = pixel_distances(host, px, &rows.iter().map(|r| r.0).collect::<Vec<_>>());
        let mut best: Option<(f64, usize)> = None;
        for (i, dist) in dists.into_iter().enumerate() {
            let Some(dist) = dist.filter(|d| *d <= CORNER_PX) else { continue };
            if best.is_none_or(|(b, _)| dist < b) {
                best = Some((dist, i));
            }
        }
        best.map(|(_, i)| rows[i].clone())
    }

    fn round_corner(&mut self, corner: (DVec3, Edge, Edge, DVec3, DVec3), d: f64, alt: bool,
                    host: &mut dyn ToolHost) -> Reply {
        let (v, a, _b, a_far, b_far) = corner;
        let p1 = v + (a_far - v).normalize_or_zero() * d;
        let p2 = v + (b_far - v).normalize_or_zero() * d;
        self.start = Some(p1);
        self.end = Some(p2);
        self.start_edge = Some((a, p1));
        self.fillet = Some((v, p1, p2));
        let n = (a_far - v).cross(b_far - v);
        self.p.lock.hover = (n.length() >= 1e-9).then(|| (v, n.normalize()));
        self.tangent_dir = Some((v - p1).normalize_or_zero());
        let Some(h) = self.tangent_bulge(None, None) else {
            self.reset();
            return Reply::redraw();
        };
        self.snap_bulge = Some(h);
        self.kind = Some(Kind::Fillet);
        let (pts, apex) = self.points(None, Some(h), None);
        if pts.len() < 2 {
            self.reset();
            return Reply::redraw();
        }
        let trim = self.corner_trim(alt, host);
        self.commit(pts, apex, trim, host)
    }

    fn hover(&mut self, world: DVec3, px: [f64; 2], host: &mut dyn ToolHost) -> Reply {
        self.hover = Some(world);
        self.snap_bulge = None;
        self.kind = None;
        self.equidistant = None;
        self.cue = None;
        if self.start.is_some() && self.end.is_none() {
            // From an edge the preview is tangent to it (cyan); at the
            // equidistant point of the next edge, magenta.
            if self.start_edge.is_some() {
                self.cue = Some(Kind::Tangent);
                self.equidistant = self.equidistant_near(px, host);
                if let Some((_, p2, _)) = &self.equidistant {
                    self.hover = Some(*p2);
                    self.cue = Some(Kind::Fillet);
                }
            }
        } else if self.end.is_some() {
            self.snap_bulge_for(world, host);
        }
        Reply::redraw()
    }

    /// The bulge's inferences: tangent to what the start sits on (cyan;
    /// magenta for the fillet), and the half circle.
    fn snap_bulge_for(&mut self, world: DVec3, host: &mut dyn ToolHost) {
        let (Some(start), Some(end)) = (self.start, self.end) else { return };
        let (u, v) = self.uv();
        let e2 = self.to2(end, u, v);
        let length = e2.0.hypot(e2.1);
        if length < 1e-9 {
            return;
        }
        let perp = u * (-e2.1 / length) + v * (e2.0 / length);
        let Some(scale) = world_per_pixel(host, (start + end) * 0.5, perp) else { return };
        let tol = scale * TANGENT_PX;
        let b = self.bulge_for(world);
        if self.tangent_dir.is_some() {
            if let Some(h) = self.tangent_bulge(None, None).filter(|h| (b - h).abs() <= tol) {
                self.snap_bulge = Some(h);
                let k = if self.fillet.is_some() { Kind::Fillet } else { Kind::Tangent };
                self.kind = Some(k);
                self.cue = Some(k);
                return;
            }
        }
        let half = length / 2.0;
        if (b.abs() - half).abs() <= tol {
            self.snap_bulge = Some(half.copysign(if b != 0.0 { b } else { 1.0 }));
            self.kind = Some(Kind::Half);
            self.cue = Some(Kind::Half);
        }
    }

    fn value(&mut self, value: &Value, host: &mut dyn ToolHost) -> Reply {
        let Value::Length(x) = *value else { return Reply::ignored() };
        if self.start.is_none() {
            return match set_segments(x, host) {
                Some(n) => { self.segments = n; Reply::redraw() }
                None => Reply::ignored(),
            };
        }
        let (Some(_), Some(hover)) = (self.end, self.hover) else { return Reply::ignored() };
        if self.fillet.is_some() {
            // Both tangencies fixed: the number is the fillet's radius (#43).
            return self.fillet_with_radius(x, host);
        }
        let sign = if self.bulge_for(hover) < 0.0 { -1.0 } else { 1.0 };
        self.kind = None;
        let (pts, apex) = self.points(None, Some(sign * x), None);
        if pts.len() >= 2 {
            return self.commit(pts, apex, None, host);
        }
        Reply::redraw()
    }

    /// The armed fillet with `radius`: tangent points `r / tan(θ/2)` from
    /// the corner on each edge, the corner trimmed. A radius the edges
    /// cannot hold is refused, and said.
    fn fillet_with_radius(&mut self, radius: f64, host: &mut dyn ToolHost) -> Reply {
        let Some((v, p1, p2)) = self.fillet else { return Reply::ignored() };
        let (a, b) = (p1 - v, p2 - v);
        if radius <= 0.0 || a.length() < 1e-9 || b.length() < 1e-9 {
            return Reply::ignored();
        }
        let (ua, ub) = (a.normalize(), b.normalize());
        let theta = ua.dot(ub).clamp(-1.0, 1.0).acos();
        if theta < 1e-3 || (theta - PI).abs() < 1e-3 {
            return Reply::ignored(); // no corner to round
        }
        let d = radius / (theta / 2.0).tan();
        let (mut reach, mut start) = (vec![], vec![]);
        for e in [self.start_edge.as_ref().map(|s| &s.0), self.fillet_edge_b.as_ref()].into_iter().flatten() {
            let (x, y) = ((e.a - v).length(), (e.b - v).length());
            start.push(x.min(y));
            reach.push(x.max(y));
        }
        if !start.is_empty() && d < start.iter().cloned().fold(f64::MIN, f64::max) - 1e-6 {
            return Reply::ignored(); // inside the arc already there
        }
        if !reach.is_empty() {
            let shortest = reach.iter().cloned().fold(f64::MAX, f64::min);
            if d > shortest + 1e-6 {
                host.message("Radius {r} needs {d} of edge on each side of the corner; the shorter one is {e}",
                             &[("r", format!("len:{radius}")), ("d", format!("len:{d}")), ("e", format!("len:{shortest}"))]);
                return Reply::redraw();
            }
        }
        let (s, e) = (v + ua * d, v + ub * d);
        self.start = Some(s);
        self.end = Some(e);
        if let Some((edge, _)) = self.start_edge.take() {
            self.start_edge = Some((edge, s));
        }
        self.fillet = Some((v, s, e));
        self.tangent_dir = Some((v - s).normalize_or_zero());
        let Some(h) = self.tangent_bulge(None, None) else { return Reply::ignored() };
        self.snap_bulge = Some(h);
        self.kind = Some(Kind::Fillet);
        let (pts, apex) = self.points(None, Some(h), None);
        if pts.len() < 2 {
            return Reply::ignored();
        }
        let trim = self.corner_trim(false, host);
        self.commit(pts, apex, trim, host)
    }

    /// «2r»: the radius instead of the bulge; the minor arc.
    fn radius(&mut self, radius: f64, host: &mut dyn ToolHost) -> Reply {
        let (Some(start), Some(end)) = (self.start, self.end) else { return Reply::ignored() };
        if radius <= 0.0 {
            return Reply::ignored();
        }
        let half = (end - start).length() * 0.5;
        if half < 1e-9 {
            return Reply::ignored();
        }
        if radius < half - 1e-9 {
            host.message("Radius {r} is smaller than half the chord ({h})",
                         &[("r", format!("len:{radius}")), ("h", format!("len:{half}"))]);
            return Reply::redraw();
        }
        let radius = radius.max(half);
        let h = radius - (radius * radius - half * half).max(0.0).sqrt();
        let mut sign = if self.hover.is_some_and(|c| self.bulge_for(c) < 0.0) { -1.0 } else { 1.0 };
        // While magenta, a typed radius keeps the trim only when it IS the
        // fillet's.
        let mut trim = None;
        if self.fillet.is_some() {
            if let Some(ht) = self.tangent_bulge(None, None) {
                if (ht.abs() - h).abs() <= 1e-6 * radius.max(1.0) {
                    sign = 1f64.copysign(ht);
                    self.kind = Some(Kind::Fillet);
                    trim = self.corner_trim(false, host);
                }
            }
        }
        let (pts, apex) = self.points(None, Some(sign * h), None);
        if pts.len() >= 2 {
            return self.commit(pts, apex, trim, host);
        }
        Reply::redraw()
    }

    fn segments(&mut self, n: i64, host: &mut dyn ToolHost) -> Reply {
        let Some(n) = set_segments(n as f64, host) else { return Reply::ignored() };
        self.segments = n;
        if let Some(last) = self.last.clone() {
            if last.token.is_some_and(|t| t == host.revision()) {
                host.run(&Command { name: "undo", args: json!({}) });
                let pts: Vec<DVec3> = arc_3pts_2d((0.0, 0.0), last.apex, last.e2, n)
                    .into_iter()
                    .map(|(x, y)| last.origin + last.u * x + last.v * y)
                    .collect();
                let answer = run_arc(host, &pts, None, last.trim.as_deref());
                self.last = Some(LastArc { token: answer.ok.then_some(answer.revision), ..last });
            }
        }
        Reply::redraw()
    }

    fn lines(&self) -> Vec<([f64; 3], [f64; 3])> {
        let (Some(start), Some(hover)) = (self.start, self.hover) else { return vec![] };
        if self.end.is_none() {
            if self.start_edge.is_some() {
                // Tangent to the start edge, through the cursor.
                let tangent = self.edge_tangent(hover);
                if let Some(h) = self.tangent_bulge(Some(hover), tangent) {
                    let (pts, _) = self.points(None, Some(h), Some(hover));
                    if pts.len() >= 2 {
                        return polyline(&pts);
                    }
                }
            }
            return vec![(p3(start), p3(hover))]; // the chord
        }
        polyline(&self.points(Some(hover), None, None).0)
    }

    /// `[kind, value | null, mid]`: "tangent_edge" (from an edge), "half",
    /// "fillet", "tangent", or "bulge".
    fn label(&self) -> J {
        let (Some(start), Some(hover)) = (self.start, self.hover) else { return J::Null };
        let Some(end) = self.end else {
            return if self.start_edge.is_some() {
                json!(["tangent_edge", J::Null, p3((start + hover) * 0.5)])
            } else {
                J::Null
            };
        };
        let mid = p3((start + end) * 0.5);
        if let Some(h) = self.snap_bulge {
            let k = match self.kind { Some(Kind::Half) => "half", Some(Kind::Fillet) => "fillet", _ => "tangent" };
            return json!([k, h.abs(), mid]);
        }
        json!(["bulge", self.bulge_for(hover).abs(), mid])
    }
}

fn run_arc(host: &mut dyn ToolHost, pts: &[DVec3], center: Option<DVec3>, trim: Option<&[(DVec3, DVec3)]>) -> super::Answer {
    let mut args = json!({"points": pts.iter().map(|p| j3(*p)).collect::<Vec<_>>()});
    if let Some(c) = center {
        args["center"] = j3(c);
    }
    if let Some(t) = trim {
        args["trim"] = J::Array(t.iter().map(|(a, b)| json!([j3(*a), j3(*b)])).collect());
    }
    host.run(&Command { name: "add_arc", args })
}

impl Tool for ArcTool {
    fn info(&self) -> &'static ToolInfo { &ARC }

    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        if self.p.event(ev) {
            return Reply::ignored();
        }
        match ev {
            ToolEvent::Activate => { self.reset(); self.last = None; Reply::redraw() }
            ToolEvent::Deactivate => { self.reset(); self.hover = None; self.last = None; Reply::redraw() }
            ToolEvent::Cancel => { self.reset(); Reply::redraw() }
            ToolEvent::Hover(p) => match p.world {
                Some(w) => self.hover(d3(w), p.px, host),
                None => Reply::ignored(),
            },
            ToolEvent::Click(p) | ToolEvent::TripleClick(p) => match p.world {
                Some(w) => self.click(d3(w), p.mods.alt, host),
                None => Reply::ignored(),
            },
            ToolEvent::DoubleClick(p) => match p.world {
                Some(w) => self.double_click(d3(w), p.px, p.mods.alt, host),
                None => Reply::ignored(),
            },
            ToolEvent::Radius(r) => self.radius(*r, host),
            ToolEvent::Value(v) => self.value(v, host),
            ToolEvent::Segments(n) => self.segments(*n, host),
            _ => Reply::ignored(),
        }
    }

    fn preview(&self) -> Preview {
        let mut out = Preview::default();
        for (a, b) in self.lines() {
            out.lines.push(Preview::rubber(a, b));
        }
        if let (Some(s), Some(h)) = (self.start, self.hover) {
            out.value = Some(ValueAnchor { at: p3((s + self.end.unwrap_or(h)) * 0.5), label: "Bulge" });
        }
        out
    }

    fn busy(&self) -> bool { self.start.is_some() }

    fn state(&self) -> J {
        json!({
            "start_point": pt_json(self.start),
            "end_point": pt_json(self.end),
            "hover_point": pt_json(self.hover),
            "work_plane": plane_json(self.p.work_plane),
            "drawing_plane": plane_json(Some(self.p.drawing(&self.plane_points()))),
            "segments": self.segments,
            "cue": self.cue.map(Kind::name),
            "kind": self.kind.map(Kind::name),
            "snap_bulge": self.snap_bulge,
            "tangent_dir": pt_json(self.tangent_dir),
            "fillet": self.fillet.map(|(v, a, b)| json!([p3(v), p3(a), p3(b)])),
            "start_edge": self.start_edge.as_ref().map(|(e, p)| json!([e.id, p3(*p)])),
            "equidistant": self.equidistant.as_ref().map(|(v, p, e)| json!([p3(*v), p3(*p), e.id])),
            "label": self.label(),
            "last_fillet_d": last_fillet_d(),
        })
    }

    fn set(&mut self, key: &str, value: &J) -> bool {
        if self.p.set(key, value) {
            return true;
        }
        match key {
            "start_point" => self.start = jp(value),
            "end_point" => self.end = jp(value),
            "hover_point" => self.hover = jp(value),
            "segments" => match value.as_u64() { Some(n) if n >= 1 => self.segments = n as usize, _ => return false },
            "last_fillet_d" => set_last_fillet_d(value.as_f64()),
            "reset" => self.reset(),
            _ => return false,
        }
        true
    }

    fn plane_lock(&mut self) -> Option<&mut PlaneLock> { Some(&mut self.p.lock) }

    fn query(&self, what: &str, arg: &J) -> J {
        match what {
            "axes" => { let (u, v) = self.uv(); json!([p3(u), p3(v)]) }
            "points" => {
                let cursor = arg.get(0).and_then(jp);
                let bulge = arg.get(1).and_then(J::as_f64);
                J::Array(self.points(cursor, bulge, None).0.into_iter().map(j3).collect())
            }
            "bulge_for" => arg.get(0).and_then(jp).map(|c| json!(self.bulge_for(c))).unwrap_or_default(),
            "tangent_bulge" => json!(self.tangent_bulge(None, None)),
            _ => J::Null,
        }
    }
}

// ---- 3-point arc ------------------------------------------------------------

pub struct ThreePointArcTool {
    p: Planar,
    segments: usize,
    start: Option<DVec3>,
    mid: Option<DVec3>,
    hover: Option<DVec3>,
}

impl Default for ThreePointArcTool {
    fn default() -> Self { ThreePointArcTool { p: Planar::default(), segments: SEGMENTS, start: None, mid: None, hover: None } }
}

impl ThreePointArcTool {
    fn plane_points(&self) -> Vec<DVec3> {
        let Some(s) = self.start else { return vec![] };
        match self.mid {
            None => std::iter::once(s).chain(self.hover).collect(),
            Some(m) => [Some(s), Some(m), self.hover].into_iter().flatten().collect(),
        }
    }

    fn points(&self, end: DVec3) -> Vec<DVec3> {
        let (Some(s), Some(m)) = (self.start, self.mid) else { return vec![] };
        let (u, v) = self.p.uv(&self.plane_points());
        let to2 = |p: DVec3| { let d = p - s; (d.dot(u), d.dot(v)) };
        let (m2, e2) = (to2(m), to2(end));
        if e2.0.hypot(e2.1) < 1e-9 || m2.0.hypot(m2.1) < 1e-9 {
            return vec![];
        }
        arc_3pts_2d((0.0, 0.0), m2, e2, self.segments).into_iter().map(|(x, y)| s + u * x + v * y).collect()
    }

    fn reset(&mut self) {
        self.start = None;
        self.mid = None;
        self.p.reset();
    }
}

impl Tool for ThreePointArcTool {
    fn info(&self) -> &'static ToolInfo { &ARC3 }

    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        if self.p.event(ev) {
            return Reply::ignored();
        }
        match ev {
            ToolEvent::Activate | ToolEvent::Cancel => { self.reset(); Reply::redraw() }
            ToolEvent::Deactivate => { self.reset(); self.hover = None; Reply::redraw() }
            ToolEvent::Hover(p) => {
                if let Some(w) = p.world {
                    self.hover = Some(d3(w));
                }
                Reply::redraw()
            }
            ToolEvent::Click(p) | ToolEvent::DoubleClick(p) | ToolEvent::TripleClick(p) => {
                let Some(w) = p.world.map(d3) else { return Reply::ignored() };
                let Some(s) = self.start else {
                    self.start = Some(w);
                    self.p.first_click(w);
                    return Reply::redraw();
                };
                if self.mid.is_none() {
                    if (w - s).length() < 1e-6 {
                        return Reply::ignored();
                    }
                    self.mid = Some(w);
                    let pts = self.plane_points();
                    self.p.adopt_snapped(&pts);
                    return Reply::redraw();
                }
                let pts = self.points(w);
                if pts.len() < 2 {
                    return Reply::redraw();
                }
                run_arc(host, &pts, None, None);
                self.reset();
                Reply::committed()
            }
            ToolEvent::Value(Value::Length(x)) if self.start.is_none() => match set_segments(*x, host) {
                Some(n) => { self.segments = n; Reply::redraw() }
                None => Reply::ignored(),
            },
            ToolEvent::Segments(n) => match set_segments(*n as f64, host) {
                Some(n) => { self.segments = n; Reply::redraw() }
                None => Reply::ignored(),
            },
            _ => Reply::ignored(),
        }
    }

    fn preview(&self) -> Preview {
        let mut out = Preview::default();
        let (Some(s), Some(h)) = (self.start, self.hover) else { return out };
        let lines = if self.mid.is_none() { vec![(p3(s), p3(h))] } else { polyline(&self.points(h)) };
        out.lines = lines.into_iter().map(|(a, b)| Preview::rubber(a, b)).collect();
        out
    }

    fn busy(&self) -> bool { self.start.is_some() }

    fn state(&self) -> J {
        json!({
            "start_point": pt_json(self.start),
            "mid_point": pt_json(self.mid),
            "hover_point": pt_json(self.hover),
            "work_plane": plane_json(self.p.work_plane),
            "drawing_plane": plane_json(Some(self.p.drawing(&self.plane_points()))),
            "segments": self.segments,
        })
    }

    fn set(&mut self, key: &str, value: &J) -> bool {
        if self.p.set(key, value) {
            return true;
        }
        match key {
            "start_point" => self.start = jp(value),
            "mid_point" => self.mid = jp(value),
            "hover_point" => self.hover = jp(value),
            "segments" => match value.as_u64() { Some(n) if n >= 1 => self.segments = n as usize, _ => return false },
            "reset" => self.reset(),
            _ => return false,
        }
        true
    }

    fn plane_lock(&mut self) -> Option<&mut PlaneLock> { Some(&mut self.p.lock) }

    fn query(&self, what: &str, arg: &J) -> J {
        match what {
            "axes" => { let (u, v) = self.p.uv(&self.plane_points()); json!([p3(u), p3(v)]) }
            "points" => arg.get(0).and_then(jp).map(|e| J::Array(self.points(e).into_iter().map(j3).collect()))
                .unwrap_or_default(),
            _ => J::Null,
        }
    }
}

// ---- Centre arc and pie -------------------------------------------------------

pub struct CenterArcTool {
    p: Planar,
    pie: bool,
    /// None: sampled every 15°, the circle's lattice.
    segments: Option<usize>,
    start: Option<DVec3>,
    arm: Option<DVec3>,
    hover: Option<DVec3>,
}

impl CenterArcTool {
    pub fn center() -> Self { CenterArcTool { p: Planar::default(), pie: false, segments: None, start: None, arm: None, hover: None } }
    pub fn pie() -> Self { CenterArcTool { pie: true, ..Self::center() } }

    fn plane_points(&self) -> Vec<DVec3> {
        let Some(s) = self.start else { return vec![] };
        std::iter::once(s).chain(self.arm.or(self.hover)).collect()
    }

    fn uv(&self) -> (DVec3, DVec3) { self.p.uv(&self.plane_points()) }

    /// Signed sweep in degrees from the 0° arm to the cursor.
    fn sweep_to(&self, cursor: DVec3) -> f64 {
        let (Some(s), Some(arm)) = (self.start, self.arm) else { return 0.0 };
        let (u, v) = self.uv();
        let (a, b) = (arm - s, cursor - s);
        let mut deg = (b.dot(v).atan2(b.dot(u)) - a.dot(v).atan2(a.dot(u))).to_degrees();
        while deg <= -180.0 {
            deg += 360.0;
        }
        while deg > 180.0 {
            deg -= 360.0;
        }
        deg
    }

    fn points(&self, sweep_deg: f64) -> Vec<DVec3> {
        let (Some(s), Some(arm)) = (self.start, self.arm) else { return vec![] };
        if sweep_deg.abs() < 1e-6 {
            return vec![];
        }
        let (u, v) = self.uv();
        let a = arm - s;
        let r = a.dot(u).hypot(a.dot(v));
        if r < 1e-6 {
            return vec![];
        }
        let a0 = a.dot(v).atan2(a.dot(u));
        let steps = match self.segments {
            Some(n) if n > 0 => n,
            _ => (py_round(sweep_deg.abs() / PITCH_DEG) as usize).max(1),
        };
        (0..=steps)
            .map(|k| {
                let t = a0 + sweep_deg.to_radians() * k as f64 / steps as f64;
                s + (u * t.cos() + v * t.sin()) * r
            })
            .collect()
    }

    fn commit(&mut self, pts: Vec<DVec3>, host: &mut dyn ToolHost) -> Reply {
        run_arc(host, &pts, self.pie.then_some(self.start).flatten(), None);
        self.reset();
        Reply::committed()
    }

    fn reset(&mut self) {
        self.start = None;
        self.arm = None;
        self.p.reset();
    }

    fn label(&self) -> J {
        let (Some(s), Some(h)) = (self.start, self.hover) else { return J::Null };
        if self.arm.is_none() {
            let (u, v) = self.uv();
            let d = h - s;
            return json!(["radius", d.dot(u).hypot(d.dot(v)), p3(h)]);
        }
        json!(["sweep", self.sweep_to(h), p3(h)])
    }
}

impl Tool for CenterArcTool {
    fn info(&self) -> &'static ToolInfo { if self.pie { &PIE } else { &CENTER_ARC } }

    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        if self.p.event(ev) {
            return Reply::ignored();
        }
        match ev {
            ToolEvent::Activate | ToolEvent::Cancel => { self.reset(); Reply::redraw() }
            ToolEvent::Deactivate => { self.reset(); self.hover = None; Reply::redraw() }
            ToolEvent::Hover(p) => {
                if let Some(w) = p.world {
                    self.hover = Some(d3(w));
                }
                Reply::redraw()
            }
            ToolEvent::Click(p) | ToolEvent::DoubleClick(p) | ToolEvent::TripleClick(p) => {
                let Some(w) = p.world.map(d3) else { return Reply::ignored() };
                let Some(s) = self.start else {
                    self.start = Some(w);
                    self.p.first_click(w);
                    return Reply::redraw();
                };
                if self.arm.is_none() {
                    if (w - s).length() < 1e-6 {
                        return Reply::ignored();
                    }
                    self.arm = Some(w);
                    let pts = self.plane_points();
                    self.p.adopt_snapped(&pts); // the sweep is read on the arc's plane
                    return Reply::redraw();
                }
                let pts = self.points(self.sweep_to(w));
                if pts.len() < 2 {
                    return Reply::redraw();
                }
                self.commit(pts, host)
            }
            ToolEvent::Value(Value::Length(x)) => {
                if self.start.is_none() {
                    return match set_segments(*x, host) {
                        Some(n) => { self.segments = Some(n); Reply::redraw() }
                        None => Reply::ignored(),
                    };
                }
                if self.arm.is_none() {
                    return Reply::ignored();
                }
                let sign = match self.hover {
                    Some(h) if self.sweep_to(h) >= 0.0 => 1.0,
                    _ => -1.0,
                };
                let pts = self.points(sign * x.abs());
                if pts.len() < 2 {
                    return Reply::redraw();
                }
                self.commit(pts, host)
            }
            ToolEvent::Segments(n) => match set_segments(*n as f64, host) {
                Some(n) => { self.segments = Some(n); Reply::redraw() }
                None => Reply::ignored(),
            },
            _ => Reply::ignored(),
        }
    }

    fn preview(&self) -> Preview {
        let mut out = Preview::default();
        let (Some(s), Some(h)) = (self.start, self.hover) else { return out };
        let mut lines = vec![];
        match self.arm {
            None => lines.push((p3(s), p3(h))), // the radius arm
            Some(arm) => {
                lines.push((p3(s), p3(arm)));
                lines.push((p3(s), p3(h)));
                lines.extend(polyline(&self.points(self.sweep_to(h))));
            }
        }
        out.lines = lines.into_iter().map(|(a, b)| Preview::rubber(a, b)).collect();
        out
    }

    fn busy(&self) -> bool { self.start.is_some() }

    fn state(&self) -> J {
        json!({
            "start_point": pt_json(self.start),
            "arm_point": pt_json(self.arm),
            "hover_point": pt_json(self.hover),
            "work_plane": plane_json(self.p.work_plane),
            "drawing_plane": plane_json(Some(self.p.drawing(&self.plane_points()))),
            "segments": self.segments,
            "label": self.label(),
        })
    }

    fn set(&mut self, key: &str, value: &J) -> bool {
        if self.p.set(key, value) {
            return true;
        }
        match key {
            "start_point" => self.start = jp(value),
            "arm_point" => self.arm = jp(value),
            "hover_point" => self.hover = jp(value),
            "segments" => self.segments = value.as_u64().filter(|n| *n >= 1).map(|n| n as usize),
            "reset" => self.reset(),
            _ => return false,
        }
        true
    }

    fn plane_lock(&mut self) -> Option<&mut PlaneLock> { Some(&mut self.p.lock) }

    fn query(&self, what: &str, arg: &J) -> J {
        match what {
            "axes" => { let (u, v) = self.uv(); json!([p3(u), p3(v)]) }
            "sweep_to" => arg.get(0).and_then(jp).map(|c| json!(self.sweep_to(c))).unwrap_or_default(),
            "points" => arg.get(0).and_then(J::as_f64).map(|s| J::Array(self.points(s).into_iter().map(j3).collect()))
                .unwrap_or_default(),
            _ => J::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_points_and_lines() {
        let pts = arc_3pts_2d((0.0, 0.0), (1.0, 1.0), (2.0, 0.0), 4);
        assert_eq!(pts.len(), 5);
        assert!((pts[2].0 - 1.0).abs() < 1e-12 && (pts[2].1 - 1.0).abs() < 1e-12);
        assert_eq!(arc_3pts_2d((0.0, 0.0), (1.0, 0.0), (2.0, 0.0), 4), vec![(0.0, 0.0), (2.0, 0.0)]);
        let x = lines_meet(DVec3::new(1., 0., 0.), DVec3::new(2., 0., 0.), DVec3::new(0., 1., 0.), DVec3::new(0., 2., 0.), 1e-6);
        assert!(x.unwrap().length() < 1e-12);
        assert!(lines_meet(DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::Y + DVec3::X, 1e-6).is_none());
    }
}
