//! Circle and Polygon: a centre click, then a radius (R2.5 slice 7).
//!
//! The rules (from `tools/circle.py`, issue numbers kept with them):
//! - Both draw a regular N-gon on the drawing plane, one vertex toward the
//!   cursor; a circle is a many-sided one (24), a polygon has 6.
//! - The plane is the machine's own `PlaneLock`, as the Rectangle's.
//! - Before the centre a typed number is the side count (unitless, #176);
//!   after it, the radius along the cursor's direction. `Ns` sets the
//!   sides at any moment.
//! - Right after a shape, a typed radius or `Ns` redraws it in place, one
//!   undo step (the hot retype, #305, #414); anything done in between
//!   closes that window.
//! - Ctrl toggles the Circle between a face and an analytic guide circle;
//!   a Ctrl that was part of a chord takes itself back on release (#342).
//! - The commit is the protocol's `add_face` with `curve`, the guide's
//!   `add_guide_circle`.

use glam::DVec3;
use std::f64::consts::PI;

use super::plane::{plane_axes, Axes, Plane, PlaneLock};
use super::preview::ValueAnchor;
use super::{Command, Preview, Reply, Tool, ToolEvent, ToolHost, ToolInfo, Value};

static CIRCLE: ToolInfo = ToolInfo {
    kind: "circle",
    name: "Circle",
    shortcut: "C",
    description: "Draw a circle from its centre and radius.",
    uses_snap: true,
    box_select: false,
    vcb_label: Some("Radius"),
    // The axis magnet is `tools.base.AxisMagnet`'s, read per hover.
    magnetic_axis_deg: None,
    screen_axis_px: None,
    axis_source: true,
    axis_source_empty_only: true,
};

static POLYGON: ToolInfo = ToolInfo {
    kind: "polygon",
    name: "Polygon",
    shortcut: "",   // none by default: G is Make Component
    description: "Draw a regular polygon from its centre and radius; type a number followed by «s» to change the sides.",
    uses_snap: true,
    box_select: false,
    vcb_label: Some("Radius"),
    magnetic_axis_deg: None,
    screen_axis_px: None,
    axis_source: false,
    axis_source_empty_only: false,
};

/// The shape just drawn, for the hot retype: same centre, plane and
/// vertex direction.
#[derive(Clone, Debug, PartialEq)]
struct Last {
    center: DVec3,
    rim: DVec3,
    u: DVec3,
    v: DVec3,
    token: Option<u64>,
}

pub struct RadialTool {
    info: &'static ToolInfo,
    guide_capable: bool,
    sides: usize,
    start: Option<DVec3>,
    hover: Option<DVec3>,
    work_plane: Option<Plane>,
    lock: PlaneLock,
    last: Option<Last>,
    guide_mode: bool,
    ctrl_toggled: bool,
    axes: Option<Axes>,
    names_point: Option<bool>,
}

impl RadialTool {
    fn new(info: &'static ToolInfo, sides: usize, guide_capable: bool) -> Self {
        RadialTool {
            info, guide_capable, sides, start: None, hover: None, work_plane: None,
            lock: PlaneLock::default(), last: None, guide_mode: false, ctrl_toggled: false,
            axes: None, names_point: None,
        }
    }

    pub fn circle() -> Self { Self::new(&CIRCLE, 24, true) }
    pub fn polygon() -> Self { Self::new(&POLYGON, 6, false) }

    fn ax(&self) -> Axes { self.axes.unwrap_or_else(Axes::world) }

    fn plane_points(&self) -> Vec<DVec3> {
        match self.start {
            None => vec![],
            Some(s) => std::iter::once(s).chain(self.hover).collect(),
        }
    }

    pub fn drawing_plane(&self) -> Plane {
        self.lock.drawing(self.work_plane, &self.plane_points(), self.names_point, &self.ax())
    }

    fn uv(&self) -> (DVec3, DVec3) { plane_axes(self.drawing_plane().1, &self.ax()) }

    fn guiding(&self) -> bool { self.guide_capable && self.guide_mode }

    /// The radius drawn: the rim projected onto the plane.
    fn radius(&self, center: DVec3, rim: DVec3) -> f64 {
        let (u, v) = self.uv();
        let d = rim - center;
        d.dot(u).hypot(d.dot(v))
    }

    fn ring(center: DVec3, u: DVec3, v: DVec3, r: f64, a0: f64, n: usize) -> Vec<DVec3> {
        (0..n)
            .map(|k| {
                let a = a0 + 2.0 * PI * k as f64 / n as f64;
                center + (u * a.cos() + v * a.sin()) * r
            })
            .collect()
    }

    /// The shape's corners, one toward `rim`; none for a zero radius.
    fn points(&self, center: DVec3, rim: DVec3) -> Vec<DVec3> {
        let (u, v) = self.uv();
        let d = rim - center;
        let (du, dv) = (d.dot(u), d.dot(v));
        let r = du.hypot(dv);
        if r < 1e-6 {
            return vec![];
        }
        Self::ring(center, u, v, r, dv.atan2(du), self.sides)
    }

    fn reset(&mut self) {
        self.start = None;
        self.work_plane = None;
        self.lock.hover = None;
        self.lock.clear();
    }

    fn commit(&mut self, pts: &[DVec3], axes: Option<(DVec3, DVec3)>, center: Option<DVec3>, host: &mut dyn ToolHost) {
        let (u, v) = axes.unwrap_or_else(|| self.uv());
        let center = center.unwrap_or_else(|| pts.iter().fold(DVec3::ZERO, |s, p| s + *p) / pts.len() as f64);
        let ring: Vec<serde_json::Value> = pts.iter().map(|p| serde_json::json!([p.x, p.y, p.z])).collect();
        let answer = host.run(&Command { name: "add_face", args: serde_json::json!({"outer": ring, "curve": true}) });
        self.last = Some(Last { center, rim: pts[0], u, v, token: answer.ok.then_some(answer.revision) });
        self.reset();
    }

    fn retype_open(&self, host: &mut dyn ToolHost) -> bool {
        self.start.is_none() && self.last.as_ref().is_some_and(|l| l.token.is_some_and(|t| t == host.revision()))
    }

    /// Undo the shape just drawn and draw it again in place with `n` sides
    /// and, if given, the new `radius`: one undo step however many times.
    fn redraw_last(&mut self, n: usize, radius: Option<f64>, host: &mut dyn ToolHost) {
        let Some(previous) = self.last.clone() else { return };
        let Last { center, rim, u, v, .. } = previous;
        let d = rim - center;
        let r = radius.unwrap_or_else(|| d.dot(u).hypot(d.dot(v)));
        let a0 = d.dot(v).atan2(d.dot(u));
        let pts = Self::ring(center, u, v, r, a0, n);
        if !host.run(&Command { name: "undo", args: serde_json::json!({}) }).ok {
            return;
        }
        self.commit(&pts, Some((u, v)), Some(center), host);
        if self.last.as_ref().is_none_or(|l| l.token.is_none()) {
            // Refused: the shape already made comes back.
            host.run(&Command { name: "redo", args: serde_json::json!({}) });
            self.last = Some(previous);
        }
    }

    fn commit_guide(&mut self, rim: DVec3, host: &mut dyn ToolHost) {
        let Some(center) = self.start else { return };
        let r = self.radius(center, rim);
        if r < 1e-9 {
            return;
        }
        let n = self.drawing_plane().1;
        let answer = host.run(&Command {
            name: "add_guide_circle",
            args: serde_json::json!({"center": [center.x, center.y, center.z], "normal": [n.x, n.y, n.z], "radius": r}),
        });
        if !answer.ok {
            return;
        }
        host.message("Guide circle, radius {r}", &[("r", format!("len:{r}"))]);
        self.reset();
    }

    fn click(&mut self, world: DVec3, host: &mut dyn ToolHost) -> Reply {
        let Some(start) = self.start else {
            self.last = None; // a new centre ends the retype window
            self.start = Some(world);
            if self.work_plane.is_none() {
                self.work_plane = self.lock.locked(world, &self.ax());
            }
            return Reply::redraw();
        };
        if self.guiding() {
            self.commit_guide(world, host);
            return Reply::committed();
        }
        let pts = self.points(start, world);
        if pts.is_empty() {
            return Reply::redraw();
        }
        self.commit(&pts, None, None, host);
        Reply::committed()
    }

    fn say_sides(&self, host: &mut dyn ToolHost) {
        host.message("{n} sides", &[("n", self.sides.to_string())]);
    }

    fn segments(&mut self, n: i64, host: &mut dyn ToolHost) -> Reply {
        if n < 3 {
            return Reply::ignored();
        }
        self.sides = n as usize;
        self.say_sides(host);
        if self.retype_open(host) {
            self.redraw_last(self.sides, None, host);
        }
        Reply::redraw()
    }

    fn value(&mut self, value: &Value, host: &mut dyn ToolHost) -> Reply {
        let Value::Length(x) = *value else { return Reply::ignored() };
        if self.retype_open(host) {
            if x <= 0.0 {
                return Reply::ignored();
            }
            self.redraw_last(self.sides, Some(x), host);
            return Reply::committed();
        }
        let Some(start) = self.start else {
            // Python's round(): halves to even.
            let n = x.round_ties_even();
            if n >= 3.0 {
                self.sides = n as usize;
                self.say_sides(host);
                return Reply::redraw();
            }
            return Reply::ignored();
        };
        let Some(hover) = self.hover else { return Reply::ignored() };
        if x <= 0.0 {
            return Reply::ignored();
        }
        // Keep the cursor's direction, override only the radius.
        let (u, v) = self.uv();
        let d = hover - start;
        let ang = d.dot(v).atan2(d.dot(u));
        let rim = start + (u * ang.cos() + v * ang.sin()) * x;
        if self.guiding() {
            self.commit_guide(rim, host);
            return Reply::committed();
        }
        let pts = self.points(start, rim);
        if !pts.is_empty() {
            self.commit(&pts, None, None, host);
        }
        Reply::committed()
    }

    fn toggle_guide(&mut self, say: bool, host: &mut dyn ToolHost) {
        self.guide_mode = !self.guide_mode;
        if say {
            host.message(if self.guide_mode { "Guide circle" } else { "Circle" }, &[]);
        }
    }

    fn key(&mut self, key: &str, pressed: bool, tapped: bool, host: &mut dyn ToolHost) -> Reply {
        if key != "Control" || !self.guide_capable {
            return Reply::ignored();
        }
        if pressed {
            self.toggle_guide(true, host);
            self.ctrl_toggled = true;
            return Reply::redraw();
        }
        // Toggled on the press so the mode changes at once; a Ctrl that
        // was part of a shortcut takes it back on the release (#342).
        if !self.ctrl_toggled {
            return Reply::ignored();
        }
        self.ctrl_toggled = false;
        if !tapped {
            self.toggle_guide(false, host);
        }
        Reply::redraw()
    }

    fn lines(pts: &[DVec3]) -> Vec<([f64; 3], [f64; 3])> {
        let p = |v: DVec3| [v.x, v.y, v.z];
        (0..pts.len()).map(|i| (p(pts[i]), p(pts[(i + 1) % pts.len()]))).collect()
    }

    fn live_ring(&self) -> Vec<DVec3> {
        match (self.start, self.hover) {
            (Some(s), Some(h)) => self.points(s, h),
            _ => vec![],
        }
    }
}

fn d3(p: [f64; 3]) -> DVec3 { DVec3::from_array(p) }

impl Tool for RadialTool {
    fn info(&self) -> &'static ToolInfo { self.info }

    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Activate => {
                self.reset();
                self.last = None;
                self.guide_mode = false; // a fresh pick-up starts as a face
                Reply::redraw()
            }
            ToolEvent::Deactivate => {
                self.reset();
                self.last = None;
                self.hover = None;
                Reply::redraw()
            }
            ToolEvent::Cancel => {
                self.reset();
                self.last = None; // Esc closes the retype window
                Reply::redraw()
            }
            ToolEvent::Hover(p) => match p.world {
                Some(w) => {
                    self.hover = Some(d3(w));
                    Reply::redraw()
                }
                None => Reply::ignored(),
            },
            ToolEvent::Click(p) | ToolEvent::DoubleClick(p) | ToolEvent::TripleClick(p) => match p.world {
                Some(w) => self.click(d3(w), host),
                None => Reply::ignored(),
            },
            ToolEvent::Value(v) => self.value(v, host),
            ToolEvent::Segments(n) => self.segments(*n, host),
            ToolEvent::Key { key, .. } => self.key(key, true, false, host),
            ToolEvent::KeyRelease { key, tapped } => self.key(key, false, *tapped, host),
            ToolEvent::WorkPlane(plane) => {
                self.work_plane = plane.map(|(p, n)| (d3(p), d3(n)));
                Reply::ignored()
            }
            ToolEvent::Frame { axes, names_point } => {
                self.axes = Some(Axes { o: d3(axes[0]), x: d3(axes[1]), y: d3(axes[2]), z: d3(axes[3]) });
                self.names_point = *names_point;
                Reply::ignored()
            }
            _ => Reply::ignored(),
        }
    }

    fn preview(&self) -> Preview {
        let mut out = Preview::default();
        if self.guiding() {
            // The guide's rim is drawn dashed (`state.guide_ring`), never
            // as a solid rubber band.
            return out;
        }
        for (a, b) in Self::lines(&self.live_ring()) {
            out.lines.push(Preview::rubber(a, b));
        }
        if let (Some(_), Some(h)) = (self.start, self.hover) {
            out.value = Some(ValueAnchor { at: [h.x, h.y, h.z], label: "Radius" });
        }
        out
    }

    fn busy(&self) -> bool { self.start.is_some() }

    fn state(&self) -> serde_json::Value {
        let pt = |v: Option<DVec3>| v.map(|v| serde_json::json!([v.x, v.y, v.z])).unwrap_or(serde_json::Value::Null);
        let plane = |p: Option<Plane>| p.map(|(a, n)| serde_json::json!([[a.x, a.y, a.z], [n.x, n.y, n.z]]))
            .unwrap_or(serde_json::Value::Null);
        let radius = match (self.start, self.hover) {
            (Some(s), Some(h)) => serde_json::json!(self.radius(s, h)),
            _ => serde_json::Value::Null,
        };
        let guide_ring: Vec<_> = if self.guiding() { Self::lines(&self.live_ring()) } else { vec![] };
        serde_json::json!({
            "start_point": pt(self.start),
            "hover_point": pt(self.hover),
            "work_plane": plane(self.work_plane),
            "drawing_plane": plane(Some(self.drawing_plane())),
            "sides": self.sides,
            "radius": radius,
            "guide_mode": self.guide_mode,
            "guide_ring": guide_ring,
            "last_token": self.last.as_ref().and_then(|l| l.token),
        })
    }

    fn set(&mut self, key: &str, value: &serde_json::Value) -> bool {
        let point = super::json_p3(value).map(d3);
        match key {
            "start_point" => self.start = point,
            "hover_point" => self.hover = point,
            "work_plane" => {
                self.work_plane = value.as_array().and_then(|a| {
                    Some((d3(super::json_p3(a.first()?)?), d3(super::json_p3(a.get(1)?)?)))
                });
            }
            "sides" => match value.as_u64() {
                Some(n) if n >= 1 => self.sides = n as usize,
                _ => return false,
            },
            "guide_mode" => self.guide_mode = value.as_bool().unwrap_or(false),
            "forget_last" => self.last = None,
            "reset" => self.reset(),
            _ => return false,
        }
        true
    }

    fn plane_lock(&mut self) -> Option<&mut PlaneLock> { Some(&mut self.lock) }

    fn query(&self, what: &str, arg: &serde_json::Value) -> serde_json::Value {
        let point = |v: &serde_json::Value| super::json_p3(v).map(d3);
        let j = |v: DVec3| serde_json::json!([v.x, v.y, v.z]);
        match what {
            // The corners for a centre and a rim, on the drawing plane.
            "points" => (|| {
                let a = arg.as_array()?;
                Some(serde_json::json!(self.points(point(a.first()?)?, point(a.get(1)?)?).into_iter().map(j).collect::<Vec<_>>()))
            })()
            .unwrap_or_default(),
            "axes" => { let (u, v) = self.uv(); serde_json::json!([j(u), j(v)]) }
            _ => serde_json::Value::Null,
        }
    }
}
