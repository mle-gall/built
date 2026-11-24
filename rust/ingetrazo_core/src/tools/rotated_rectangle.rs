//! Rotated Rectangle: a rectangle at any angle in any plane (R2.5 slice 7).
//!
//! Three clicks (from `tools/rotated_rectangle.py`, issue numbers kept):
//! 1. the first corner, which captures the plane: an arrow-key lock, else
//!    the face under the click (the host sends it as the work plane);
//! 2. the second corner: the base edge's direction and length, over a
//!    protractor at the first corner (15° ticks near its rim, Shift holds
//!    the direction); `length` or `length;angle` typed;
//! 3. the width and its angle around the base edge, read on a second
//!    protractor square to the edge at its end (#70); Shift holds the
//!    angle, the view's axis lock imposes the width's direction; `width`
//!    or `width;angle` typed.
//!
//! What only the view knows comes with `ToolEvent::View`: the cursor's
//! ray (the width is read on the protractor's plane, not the drawing
//! plane), the axis lock, the protractor's size at a fixed screen size and
//! whether the cursor is near its rim, and the view's drawing plane for
//! the winding. The commit is the protocol's `add_face`.

use glam::DVec3;

use super::plane::{face_the_plane, plane_axes, Axes, Plane, PlaneLock};
use super::preview::ValueAnchor;
use super::{Command, Preview, Reply, Tool, ToolEvent, ToolHost, ToolInfo, Value, ViewReading};

static INFO: ToolInfo = ToolInfo {
    kind: "rotated_rectangle",
    name: "Rotated Rect",
    // K belongs to Back Edges (#234); no default key.
    shortcut: "",
    description: "Draw a rectangle at any angle and in any plane: a base edge, then the width.",
    uses_snap: true,
    box_select: false,
    vcb_label: Some("Length; angle"),
    magnetic_axis_deg: None,
    screen_axis_px: None,
    axis_source: false,
    axis_source_empty_only: false,
};

/// Below this the width makes no rectangle: the corners merge.
const MIN_WIDTH: f64 = 1e-6;
/// Degrees within which the width's angle sticks to flat or standing.
const ANGLE_SNAP: f64 = 3.0;
/// The protractor's ticks (`tools.protractor.TICK_DEG`).
pub const TICK_DEG: f64 = 15.0;
/// Snaps that name a real point of the model: the width goes there.
const POINT_SNAPS: [&str; 11] = ["endpoint", "midpoint", "intersection", "intersection_edge",
    "intersection_face", "on_edge", "close", "origin", "center", "centre", "on_line"];

pub struct RotatedRectangleTool {
    start: Option<DVec3>,
    base: Option<DVec3>,
    hover: Option<DVec3>,
    work_plane: Option<Plane>,
    lock: PlaneLock,
    /// The width's turn around the base edge, degrees: 0 lying in the
    /// plane, 90 standing square to it.
    angle: f64,
    /// The direction the axis lock imposes on the width, as of the hover.
    locked: Option<DVec3>,
    disc_r: f64,
    near_disc: bool,
    held_angle: Option<f64>,
    held_dir: Option<DVec3>,
    view: ViewReading,
    axes: Option<Axes>,
}

impl Default for RotatedRectangleTool {
    fn default() -> Self {
        RotatedRectangleTool {
            start: None, base: None, hover: None, work_plane: None, lock: PlaneLock::default(),
            angle: 0.0, locked: None, disc_r: 1.0, near_disc: false, held_angle: None,
            held_dir: None, view: ViewReading::default(), axes: None,
        }
    }
}

fn d3(p: [f64; 3]) -> DVec3 { DVec3::from_array(p) }
fn j(v: DVec3) -> [f64; 3] { [v.x, v.y, v.z] }
/// Python's `round()`: halves to even.
fn ticks(deg: f64) -> f64 { (deg / TICK_DEG).round_ties_even() * TICK_DEG }

impl RotatedRectangleTool {
    fn ax(&self) -> Axes { self.axes.unwrap_or_else(Axes::world) }

    fn normal(&self) -> DVec3 {
        self.work_plane.map(|(_, n)| n.normalize_or_zero()).unwrap_or(DVec3::Z)
    }

    fn edge(&self) -> Option<DVec3> { Some(self.base? - self.start?) }

    /// In-plane unit vector square to the base edge (angle 0).
    fn perp(&self) -> DVec3 {
        match self.edge() {
            Some(e) if e.length() >= 1e-9 => self.normal().cross(e.normalize()).normalize_or_zero(),
            _ => DVec3::ZERO,
        }
    }

    fn width_dir(&self, angle: Option<f64>) -> DVec3 {
        let perp = self.perp();
        if perp.length() < 1e-6 {
            return DVec3::ZERO;
        }
        let a = angle.unwrap_or(self.angle).to_radians();
        (perp * a.cos() + self.normal() * a.sin()).normalize_or_zero()
    }

    /// `(width, angle)` the cursor asks for around the base edge; the
    /// part along the edge is dropped.
    fn width_and_angle(&self, cursor: DVec3, forced: Option<DVec3>) -> (f64, f64) {
        let perp = self.perp();
        let (Some(base), Some(e)) = (self.base, self.edge()) else { return (0.0, 0.0) };
        if perp.length() < 1e-6 {
            return (0.0, 0.0);
        }
        let edge = e.normalize_or_zero();
        let mut d = cursor - base;
        d -= edge * d.dot(edge);
        if let Some(f) = forced {
            // Onto the locked direction: the sign survives as the opposite
            // angle, and a snap cannot take the width off its axis.
            d = f * d.dot(f);
        }
        let width = d.length();
        if width < 1e-9 {
            return (0.0, self.angle);
        }
        let mut angle = d.dot(self.normal()).atan2(d.dot(perp)).to_degrees();
        if self.near_disc && forced.is_none() {
            angle = ticks(angle);
        }
        for target in [-180.0, -90.0, 0.0, 90.0, 180.0] {
            if (angle - target).abs() <= ANGLE_SNAP {
                angle = target;
                break;
            }
        }
        (width, angle)
    }

    /// The width's direction the view's axis lock imposes, unless the
    /// axis is the base edge itself.
    fn locked_dir(&self) -> Option<DVec3> {
        let axis = d3(self.view.axis_lock?);
        let e = self.edge()?;
        if e.length() < 1e-9 {
            return None;
        }
        let e = e.normalize();
        let d = axis - e * axis.dot(e);
        (d.length() > 1e-6).then(|| d.normalize())
    }

    fn corners(&self, width: f64, angle: Option<f64>) -> Vec<DVec3> {
        let dir = self.width_dir(angle);
        let (Some(s), Some(b)) = (self.start, self.base) else { return vec![] };
        if dir.length() < 1e-6 {
            return vec![];
        }
        let off = dir * width;
        vec![s, b, b + off, s + off]
    }

    /// Where the cursor is for the width: on the second protractor's
    /// plane, square to the base edge through its end. A snap that names
    /// a point is taken as is; a protractor seen edge-on gives nothing.
    fn width_cursor(&self, world: DVec3, snap: Option<&str>) -> DVec3 {
        if snap.is_some_and(|k| POINT_SNAPS.contains(&k)) {
            return world;
        }
        let (Some((o, dir)), Some(base), Some(e)) = (self.view.ray, self.base, self.edge()) else { return world };
        let (o, dir, n) = (d3(o), d3(dir), e.normalize_or_zero());
        let denom = dir.dot(n);
        if denom.abs() < 0.02 {
            return world;
        }
        let t = (base - o).dot(n) / denom;
        if t <= 0.0 {
            return world;
        }
        o + dir * t
    }

    fn in_plane(&self, d: DVec3) -> DVec3 {
        let n = self.normal();
        d - n * d.dot(n)
    }

    /// The base edge's direction in the plane, degrees from its first
    /// axis (0..360): what the first protractor reads.
    fn edge_angle(&self, d: DVec3) -> f64 {
        let (u, v) = plane_axes(self.normal(), &self.ax());
        let deg = d.dot(v).atan2(d.dot(u)).to_degrees().rem_euclid(360.0);
        if deg == 0.0 { 0.0 } else { deg }
    }

    fn dir_at(&self, deg: f64) -> DVec3 {
        let (u, v) = plane_axes(self.normal(), &self.ax());
        let a = deg.to_radians();
        (u * a.cos() + v * a.sin()).normalize_or_zero()
    }

    /// A protractor disc in the plane `(u, v)`: the rim and a tick every
    /// 15°, long ones every 90°, zero on `u`.
    fn disc(&self, centre: DVec3, u: DVec3, v: DVec3) -> Vec<(DVec3, DVec3)> {
        if u.length() < 1e-9 || v.length() < 1e-9 {
            return vec![];
        }
        let r = self.disc_r;
        let rim = |t: f64, k: f64| centre + (u * t.cos() + v * t.sin()) * (r * k);
        let n = 48;
        let pts: Vec<DVec3> = (0..n).map(|k| rim(2.0 * std::f64::consts::PI * k as f64 / n as f64, 1.0)).collect();
        let mut segs: Vec<(DVec3, DVec3)> = (0..n).map(|k| (pts[k], pts[(k + 1) % n])).collect();
        for k in 0..(360.0 / TICK_DEG) as usize {
            let deg = k as f64 * TICK_DEG;
            let t = deg.to_radians();
            let inner = if deg % 90.0 == 0.0 { 0.75 } else { 0.86 };
            segs.push((rim(t, inner), rim(t, 1.0)));
        }
        segs
    }

    fn reset(&mut self) {
        self.start = None;
        self.base = None;
        self.work_plane = None;
        self.lock.clear();
        self.angle = 0.0;
        self.locked = None;
        self.near_disc = false;
        self.held_angle = None;
        self.held_dir = None;
    }

    fn commit(&mut self, corners: Vec<DVec3>, host: &mut dyn ToolHost) {
        // Wound to face the plane; stood up on its edge (90°) the loop is
        // square to it and is left as it came.
        let normal = self.work_plane.map(|(_, n)| n).or(self.view.plane_normal.map(d3));
        let wound = face_the_plane(&corners, normal);
        let ring: Vec<serde_json::Value> = wound.iter().map(|p| serde_json::json!([p.x, p.y, p.z])).collect();
        host.run(&Command { name: "add_face", args: serde_json::json!({"outer": ring}) });
        self.reset();
    }

    fn click(&mut self, world: DVec3, snap: Option<&str>, host: &mut dyn ToolHost) -> Reply {
        let Some(start) = self.start else {
            // The arrow-key lock wins; else the face under this click.
            if let Some(locked) = self.lock.locked(world, &self.ax()) {
                self.work_plane = Some(locked);
            }
            self.start = Some(world);
            return Reply::redraw();
        };
        if self.base.is_none() {
            if (world - start).length() < 1e-6 {
                return Reply::ignored();
            }
            // With Shift holding the direction the corner is where the
            // preview shows it, on the held line.
            self.base = Some(match (self.held_dir, self.hover) {
                (Some(_), Some(h)) => h,
                _ => world,
            });
            self.held_dir = None;
            if self.perp().length_squared() < 1e-12 {
                self.base = None;
                host.message("That edge is perpendicular to the drawing plane — lock another one with the \
                              arrow keys, or start on the face you want to draw on", &[]);
            }
            return Reply::redraw();
        }
        let locked = self.locked_dir();
        let (mut width, angle) = self.width_and_angle(self.width_cursor(world, snap), locked);
        self.angle = angle;
        if locked.is_some() && width < MIN_WIDTH {
            // A snap can still hand the click a point off the locked axis:
            // what was accepted is what the hover showed.
            if let Some(h) = self.hover {
                let (w, a) = self.width_and_angle(h, locked);
                width = w;
                self.angle = a;
            }
        }
        if width < MIN_WIDTH {
            host.message("The width is zero — move away from the base edge, or type the width (and the angle \
                          after a comma)", &[]);
            return Reply::redraw();
        }
        let corners = self.corners(width, None);
        if corners.is_empty() {
            return Reply::redraw();
        }
        self.commit(corners, host);
        Reply::committed()
    }

    fn hover(&mut self, world: DVec3, shift: bool, snap: Option<&str>) -> Reply {
        let mut hover = if self.base.is_some() { self.width_cursor(world, snap) } else { world };
        self.locked = self.locked_dir();
        if let (Some(start), None) = (self.start, self.base) {
            if self.near_disc {
                // Near the first protractor the edge turns in 15° steps.
                let d = self.in_plane(world - start);
                if d.length() > 1e-9 {
                    hover = start + self.dir_at(ticks(self.edge_angle(d))) * d.length();
                }
            }
            if !shift {
                self.held_dir = None;
            } else {
                let d = self.in_plane(hover - start);
                if self.held_dir.is_none() && d.length() > 1e-9 {
                    self.held_dir = Some(d.normalize());
                }
                if let Some(held) = self.held_dir {
                    // Only the length follows the cursor, along the held line.
                    hover = start + held * (world - start).dot(held);
                }
            }
        }
        self.hover = Some(hover);
        if self.base.is_some() {
            if !shift {
                self.held_angle = None;
            } else if self.held_angle.is_none() {
                self.held_angle = Some(self.angle);
            }
            if let Some(a) = self.held_angle {
                self.locked = Some(self.width_dir(Some(a)));
            }
            // The angle shown is kept: a typed width uses it.
            let (w, a) = self.width_and_angle(hover, self.locked);
            if w > MIN_WIDTH {
                self.angle = a;
            }
        }
        Reply::redraw()
    }

    fn edge_value(&mut self, value: &Value) -> Reply {
        let Some(start) = self.start else { return Reply::ignored() };
        let (mut length, mut dir) = match *value {
            Value::Pair(l, a) => (l, self.dir_at(a)),
            Value::Length(l) => {
                let d = self.hover.map(|h| h - start).unwrap_or(DVec3::ZERO);
                (l, if d.length() > 1e-9 { d.normalize() } else { self.dir_at(0.0) })
            }
            _ => return Reply::ignored(),
        };
        if length == 0.0 {
            return Reply::ignored();
        }
        if length < 0.0 {
            length = -length;
            dir = -dir;
        }
        self.base = Some(start + dir * length);
        if self.perp().length_squared() < 1e-12 {
            self.base = None;
            return Reply::ignored();
        }
        Reply::redraw()
    }

    fn value(&mut self, value: &Value, host: &mut dyn ToolHost) -> Reply {
        if self.start.is_none() {
            return Reply::ignored();
        }
        if self.base.is_none() {
            return self.edge_value(value);
        }
        if self.hover.is_none() {
            return Reply::ignored();
        }
        let corners = match *value {
            Value::Pair(width, angle) => {
                if width <= 0.0 {
                    return Reply::ignored();
                }
                self.angle = angle;
                self.corners(width, None)
            }
            Value::Length(width) => {
                if width == 0.0 {
                    return Reply::ignored();
                }
                // The side travels inside the hover's angle (180° is the
                // far side): no second sign from the cursor.
                self.corners(width, None)
            }
            _ => return Reply::ignored(),
        };
        if corners.is_empty() {
            return Reply::redraw();
        }
        self.commit(corners, host);
        Reply::committed()
    }

    fn lines(&self) -> Vec<(DVec3, DVec3)> {
        let (Some(start), Some(hover)) = (self.start, self.hover) else { return vec![] };
        let Some(base) = self.base else {
            // The base edge over the first protractor (#70).
            let (u, v) = plane_axes(self.normal(), &self.ax());
            let mut out = vec![(start, hover)];
            out.extend(self.disc(start, u, v));
            return out;
        };
        let (w, a) = self.width_and_angle(hover, self.locked);
        let c = self.corners(w, Some(a));
        // The second protractor: square to the edge at its end, zero along
        // the plane, 90° standing.
        let disc = self.disc(base, self.perp(), self.normal());
        let mut out: Vec<(DVec3, DVec3)> = if c.is_empty() {
            vec![(start, base)]
        } else {
            (0..4).map(|i| (c[i], c[(i + 1) % 4])).collect()
        };
        out.extend(disc);
        out
    }

    /// The readout: `["edge", length, angle | null, mid]` or
    /// `["width", length, width, angle, mid]`.
    fn label(&self) -> serde_json::Value {
        let (Some(start), Some(hover)) = (self.start, self.hover) else { return serde_json::Value::Null };
        let Some(base) = self.base else {
            let d = hover - start;
            let flat = self.in_plane(d);
            let angle = (flat.length() > 1e-9).then(|| self.edge_angle(flat));
            return serde_json::json!(["edge", d.length(), angle, j((start + hover) * 0.5)]);
        };
        let (w, a) = self.width_and_angle(hover, self.locked);
        let c = self.corners(w, Some(a));
        let mid = if c.is_empty() { base } else { (start + c[2]) * 0.5 };
        serde_json::json!(["width", (base - start).length(), w.abs(), a, j(mid)])
    }
}

impl Tool for RotatedRectangleTool {
    fn info(&self) -> &'static ToolInfo { &INFO }

    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Activate => {
                self.reset();
                Reply::redraw()
            }
            ToolEvent::Deactivate => {
                self.reset();
                self.hover = None;
                Reply::redraw()
            }
            ToolEvent::Cancel => {
                self.reset();
                Reply::redraw()
            }
            ToolEvent::Hover(p) => match p.world {
                Some(w) => self.hover(d3(w), p.mods.shift, p.snap.as_ref().map(|s| s.kind.as_str())),
                None => Reply::ignored(),
            },
            ToolEvent::Click(p) | ToolEvent::DoubleClick(p) | ToolEvent::TripleClick(p) => match p.world {
                Some(w) => self.click(d3(w), p.snap.as_ref().map(|s| s.kind.as_str()), host),
                None => Reply::ignored(),
            },
            ToolEvent::Value(v) => self.value(v, host),
            ToolEvent::View(view) => {
                if let Some((r, near)) = view.protractor {
                    self.near_disc = near;
                    if let Some(r) = r {
                        self.disc_r = r;
                    }
                }
                self.view = view.clone();
                Reply::ignored()
            }
            ToolEvent::WorkPlane(plane) => {
                self.work_plane = plane.map(|(p, n)| (d3(p), d3(n)));
                Reply::ignored()
            }
            ToolEvent::Frame { axes, .. } => {
                self.axes = Some(Axes { o: d3(axes[0]), x: d3(axes[1]), y: d3(axes[2]), z: d3(axes[3]) });
                Reply::ignored()
            }
            _ => Reply::ignored(),
        }
    }

    fn preview(&self) -> Preview {
        let mut out = Preview::default();
        for (a, b) in self.lines() {
            out.lines.push(Preview::rubber(j(a), j(b)));
        }
        if let Some(h) = self.hover.filter(|_| self.start.is_some()) {
            out.value = Some(ValueAnchor { at: j(h), label: if self.base.is_none() { "Length; angle" } else { "Width; angle" } });
        }
        out
    }

    fn busy(&self) -> bool { self.start.is_some() }

    fn state(&self) -> serde_json::Value {
        let pt = |v: Option<DVec3>| v.map(|v| serde_json::json!(j(v))).unwrap_or(serde_json::Value::Null);
        let plane = |p: Option<Plane>| p.map(|(a, n)| serde_json::json!([j(a), j(n)])).unwrap_or(serde_json::Value::Null);
        // The protractor the next hover measures: its centre and axis.
        let disc = match (self.start, self.base, self.edge()) {
            (Some(s), None, _) => serde_json::json!([j(s), j(self.normal())]),
            (Some(_), Some(b), Some(e)) => serde_json::json!([j(b), j(e.normalize_or_zero())]),
            _ => serde_json::Value::Null,
        };
        serde_json::json!({
            "start_point": pt(self.start),
            "base_point": pt(self.base),
            "hover_point": pt(self.hover),
            "work_plane": plane(self.work_plane),
            "angle": self.angle,
            "locked": self.locked.map(j),
            "label": self.label(),
            "disc": disc,
        })
    }

    fn set(&mut self, key: &str, value: &serde_json::Value) -> bool {
        let point = super::json_p3(value).map(d3);
        match key {
            "start_point" => self.start = point,
            "base_point" => self.base = point,
            "hover_point" => self.hover = point,
            "work_plane" => {
                self.work_plane = value.as_array().and_then(|a| {
                    Some((d3(super::json_p3(a.first()?)?), d3(super::json_p3(a.get(1)?)?)))
                });
            }
            "angle" => self.angle = value.as_f64().unwrap_or(0.0),
            "reset" => self.reset(),
            _ => return false,
        }
        true
    }

    fn plane_lock(&mut self) -> Option<&mut PlaneLock> { Some(&mut self.lock) }

    fn query(&self, what: &str, arg: &serde_json::Value) -> serde_json::Value {
        let point = |v: &serde_json::Value| super::json_p3(v).map(d3);
        match what {
            "corners" => {
                let width = arg.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0);
                let angle = arg.get(1).and_then(|v| v.as_f64());
                serde_json::json!(self.corners(width, angle).into_iter().map(j).collect::<Vec<_>>())
            }
            "width_and_angle" => match arg.get(0).and_then(point) {
                Some(c) => {
                    let forced = arg.get(1).and_then(point);
                    let (w, a) = self.width_and_angle(c, forced);
                    serde_json::json!([w, a])
                }
                None => serde_json::Value::Null,
            },
            "perp" => serde_json::json!(j(self.perp())),
            "locked_dir" => self.locked_dir().map(|d| serde_json::json!(j(d))).unwrap_or_default(),
            "point_snap" => serde_json::json!(arg.as_str().is_some_and(|k| POINT_SNAPS.contains(&k))),
            "normal" => serde_json::json!(j(self.normal())),
            _ => serde_json::Value::Null,
        }
    }
}
