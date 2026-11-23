//! Rectangle: two opposite corners on the drawing plane.
//!
//! The rules (from `tools/rectangle.py`, issue numbers kept with them):
//! - The plane is the machine's own `PlaneLock` (one owner): a face the
//!   first click captured (`work_plane`), an arrow-key lock, else the
//!   plane the cursor last hit, else the context's ground; snapped points
//!   off it re-plan the shape (#37) unless the cursor stands on a face
//!   (#545).
//! - Within 4 % the sides count as equal and the far corner snaps to a
//!   square ("Cuadrado"); Shift over the cue, or Down, locks the square.
//! - Ctrl toggles the first click between a corner and the centre (#39);
//!   a Ctrl that was part of a chord takes itself back on release (#342).
//! - A typed `W;H` lays the rectangle toward the quadrant the cursor is
//!   heading to; a lone number sets the side being stretched.
//! - Right after a rectangle, typed sizes redraw it in place, one undo
//!   step (the hot retype, #414).
//! - A side of zero draws nothing and says why.
//! - An arrow after the first corner locks the plane through it (#543).
//! - The commit is the protocol's `add_face`, with the stored first
//!   corner as anchor and exact offsets, wound to face the plane.

use glam::DVec3;

use super::plane::{face_the_plane, plane_axes, Axes, Plane, PlaneLock};
use super::{Command, Preview, Reply, Tool, ToolEvent, ToolHost, ToolInfo, Value};
use super::preview::ValueAnchor;

static INFO: ToolInfo = ToolInfo {
    kind: "rectangle",
    name: "Rectangle",
    shortcut: "R",
    description: "Draw a rectangle from two opposite corners.",
    uses_snap: true,
    box_select: false,
    vcb_label: Some("Dimensions"),
    // No axis magnet (#52): a corner pulled onto the axis through the
    // first is a rectangle of zero height.
    magnetic_axis_deg: None,
    screen_axis_px: None,
    axis_source: true,
    axis_source_empty_only: true,
};

/// Within this fraction of the longer side, the sides count as equal.
pub const SQUARE_TOL: f64 = 0.04;
const PLANE_NAMES: [&str; 3] = ["YZ", "XZ", "XY"];
const ARROWS: [&str; 3] = ["\u{2192}", "\u{2190}", "\u{2191}"];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Side { U, V }

/// The rectangle just drawn, for the hot retype.
#[derive(Clone, Debug, PartialEq)]
struct Layout {
    start: DVec3,
    u: DVec3,
    v: DVec3,
    su: f64,
    sv: f64,
    w: f64,
    h: f64,
    from_center: bool,
    live_side: Option<Side>,
    square: bool,
    token: Option<u64>,
}

impl Layout {
    /// Corner offsets from the first click, in drawing order.
    fn offsets(&self) -> [DVec3; 4] {
        let (du, dv) = (self.su * self.w, self.sv * self.h);
        let a = if self.from_center { -(self.u * (du * 0.5)) - self.v * (dv * 0.5) } else { DVec3::ZERO };
        [a, a + self.u * du, a + self.u * du + self.v * dv, a + self.v * dv]
    }
}

#[derive(Default)]
pub struct RectangleTool {
    start: Option<DVec3>,
    hover: Option<DVec3>,
    from_center: bool,
    shift_square: bool,
    arrow_square: bool,
    ctrl_toggled: bool,
    live_side: Option<Side>,
    last: Option<Layout>,
    work_plane: Option<Plane>,
    lock: PlaneLock,
    /// The drawing axes and whether the cursor's snap names a point, as
    /// of the last event.
    axes: Option<Axes>,
    names_point: Option<bool>,
}

fn sign(x: f64) -> f64 { if x < 0.0 { -1.0 } else { 1.0 } }
fn d3(p: [f64; 3]) -> DVec3 { DVec3::from_array(p) }

impl RectangleTool {
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

    fn dims(&self, a: DVec3, b: DVec3) -> (f64, f64) {
        let (u, v) = self.uv();
        ((b - a).dot(u), (b - a).dot(v))
    }

    fn square_locked(&self) -> bool { self.shift_square || self.arrow_square }

    fn square_corner(&self, a: DVec3, b: DVec3, force: bool) -> (DVec3, bool) {
        let (u, v) = self.uv();
        let (du, dv) = self.dims(a, b);
        let (adu, adv) = (du.abs(), dv.abs());
        if adu < 1e-6 || adv < 1e-6 {
            return (b, false);
        }
        if force || (adu - adv).abs() <= SQUARE_TOL * adu.max(adv) {
            let side = adu.max(adv);
            return (a + u * side.copysign(du) + v * side.copysign(dv), true);
        }
        (b, false)
    }

    fn span(&self, cursor: DVec3) -> (DVec3, DVec3) {
        let start = self.start.expect("span with a first corner");
        let (far, _) = self.square_corner(start, cursor, self.square_locked());
        if self.from_center { (start * 2.0 - far, far) } else { (start, far) }
    }

    fn corners(&self, a: DVec3, b: DVec3) -> [DVec3; 4] {
        let (u, v) = self.uv();
        let (du, dv) = self.dims(a, b);
        [a, a + u * du, a + u * du + v * dv, a + v * dv]
    }

    fn note_live_side(&mut self, prev: DVec3, new: DVec3) {
        let (u, v) = self.uv();
        let d = new - prev;
        let (du, dv) = (d.dot(u).abs(), d.dot(v).abs());
        if du > 2.0 * dv && du > 1e-6 {
            self.live_side = Some(Side::U);
        } else if dv > 2.0 * du && dv > 1e-6 {
            self.live_side = Some(Side::V);
        }
    }

    fn reset(&mut self) {
        self.start = None;
        self.work_plane = None;
        self.lock.hover = None;
        self.shift_square = false;
        self.arrow_square = false;
        self.live_side = None;
        self.lock.clear();
    }

    fn degenerate(&self, world: DVec3, host: &mut dyn ToolHost) {
        let (template, args) = self.degenerate_reason(world);
        let args: Vec<(&str, String)> = args.iter().map(|(k, v)| (*k, v.clone())).collect();
        host.message(template, &args);
    }

    /// Why the rectangle came out flat, in the words that help: a far
    /// corner off the locked plane names the arrow for the plane where
    /// the two points span a rectangle (Marco, 2026-09-18).
    fn degenerate_reason(&self, world: DVec3) -> (&'static str, Vec<(&'static str, String)>) {
        let plain = "Rectangle needs two sides — pick the opposite corner";
        let (Some((origin, normal)), Some(start)) = (self.work_plane, self.start) else {
            return (plain, vec![]);
        };
        if (world - origin).dot(normal.normalize_or_zero()).abs() < 1e-3 {
            return (plain, vec![]);
        }
        let span = world - start;
        let ax = self.ax();
        let mut k = 0;
        for i in 1..3 {
            if span.dot(ax.axis(i)).abs() < span.dot(ax.axis(k)).abs() {
                k = i;
            }
        }
        ("That corner is off the drawing plane, so the rectangle has no width. Press {arrow} for the {plane} plane.",
         vec![("arrow", ARROWS[k].to_string()), ("plane", PLANE_NAMES[k].to_string())])
    }

    fn commit(&mut self, mut last: Layout, host: &mut dyn ToolHost) {
        let offsets = last.offsets();
        let normal = last.u.cross(last.v);
        // Wound to face the plane; the offsets follow the same order.
        let wound = face_the_plane(&offsets, Some(normal));
        let ring: Vec<serde_json::Value> = wound.iter().map(|o| serde_json::json!([o.x, o.y, o.z])).collect();
        let s = last.start;
        let answer = host.run(&Command { name: "add_face", args: serde_json::json!({"anchor": [s.x, s.y, s.z], "offsets": ring}) });
        last.token = answer.ok.then_some(answer.revision);
        self.last = Some(last);
        self.reset();
    }

    fn click(&mut self, world: DVec3, host: &mut dyn ToolHost) -> Reply {
        self.last = None; // a click ends the retype window
        let Some(start) = self.start else {
            self.start = Some(world);
            if self.work_plane.is_none() {
                self.work_plane = self.lock.locked(world, &self.ax());
            }
            return Reply::redraw();
        };
        // The plane may follow the far corner: judge it on the click.
        self.hover = Some(world);
        let (anchor, far) = self.span(world);
        let (du, dv) = self.dims(anchor, far);
        if du.abs() < 1e-6 || dv.abs() < 1e-6 {
            self.degenerate(world, host);
            return Reply::redraw();
        }
        let (u, v) = self.uv();
        let last = Layout { start, u, v, su: sign(du), sv: sign(dv), w: du.abs(), h: dv.abs(),
                            from_center: self.from_center, live_side: self.live_side,
                            square: self.square_locked(), token: None };
        self.commit(last, host);
        Reply::committed()
    }

    fn hover(&mut self, world: DVec3, shift: bool) -> Reply {
        if self.start.is_some() {
            let prev = self.hover.or(self.start).unwrap();
            self.note_live_side(prev, world);
        }
        self.hover = Some(world);
        let square = self.start.is_some_and(|s| self.square_corner(s, world, false).1);
        if !shift {
            self.shift_square = false;
        } else if square {
            self.shift_square = true;
        }
        Reply::redraw()
    }

    fn sizes(&self, value: &Value, square: bool, live: Option<Side>, current: (f64, f64)) -> Option<(f64, f64)> {
        let (w, h) = match *value {
            Value::Length(x) => {
                if square {
                    (x, x)
                } else {
                    match live {
                        Some(Side::U) => (x, current.1),
                        Some(Side::V) => (current.0, x),
                        None => return None,
                    }
                }
            }
            Value::Pair(a, b) => (a, b),
            _ => return None,
        };
        (w > 0.0 && h > 0.0).then_some((w, h))
    }

    fn value(&mut self, value: &Value, host: &mut dyn ToolHost) -> Reply {
        let Some(start) = self.start else { return self.retype(value, host) };
        let Some(hover) = self.hover else { return Reply::ignored() };
        let (anchor, far) = self.span(hover);
        let (cw, ch) = self.dims(anchor, far);
        let Some((w, h)) = self.sizes(value, self.square_locked(), self.live_side, (cw.abs(), ch.abs())) else {
            return Reply::ignored();
        };
        let (u, v) = self.uv();
        let (du, dv) = self.dims(start, hover);
        let last = Layout { start, u, v, su: sign(du), sv: sign(dv), w, h, from_center: self.from_center,
                            live_side: self.live_side, square: self.square_locked(), token: None };
        self.commit(last, host);
        Reply::committed()
    }

    fn retype(&mut self, value: &Value, host: &mut dyn ToolHost) -> Reply {
        let Some(last) = self.last.clone() else { return Reply::ignored() };
        if last.token.is_none_or(|t| t != host.revision()) {
            self.last = None;
            return Reply::ignored();
        }
        let Some((w, h)) = self.sizes(value, last.square, last.live_side, (last.w, last.h)) else {
            return Reply::ignored();
        };
        if !host.run(&Command { name: "undo", args: serde_json::json!({}) }).ok {
            return Reply::ignored();
        }
        let new = Layout { w, h, token: None, ..last.clone() };
        self.commit(new, host);
        if self.last.as_ref().is_none_or(|l| l.token.is_none()) {
            // Refused: the rectangle already made comes back.
            host.run(&Command { name: "redo", args: serde_json::json!({}) });
            self.last = Some(last);
        }
        Reply::committed()
    }

    fn toggle_center(&mut self, say: bool, host: &mut dyn ToolHost) {
        self.from_center = !self.from_center;
        if say {
            host.message(if self.from_center { "Rectangle from the centre" } else { "Rectangle from a corner" }, &[]);
        }
    }

    fn key(&mut self, key: &str, pressed: bool, tapped: bool, host: &mut dyn ToolHost) -> Reply {
        if !pressed {
            if key != "Control" || !self.ctrl_toggled {
                return Reply::ignored();
            }
            self.ctrl_toggled = false;
            if !tapped {
                self.toggle_center(false, host);
            }
            return Reply::redraw();
        }
        match key {
            "Control" => {
                self.toggle_center(true, host);
                self.ctrl_toggled = true;
                Reply::redraw()
            }
            "ArrowDown" if self.start.is_some() => {
                if self.arrow_square {
                    self.arrow_square = false;
                    return Reply::redraw();
                }
                let start = self.start.unwrap();
                if self.hover.is_some_and(|h| self.square_corner(start, h, false).1) {
                    self.arrow_square = true;
                    return Reply::redraw();
                }
                Reply::ignored()
            }
            "ArrowRight" | "ArrowLeft" | "ArrowUp" if self.start.is_some() => {
                // An arrow after the first corner still chooses the plane,
                // anchored on that corner (#543).
                let k = match key { "ArrowRight" => 0, "ArrowLeft" => 1, _ => 2 };
                self.lock.toggle(k);
                let start = self.start.unwrap();
                self.work_plane = if self.lock.lock.is_some() { self.lock.locked(start, &self.ax()) } else { None };
                match self.lock.lock {
                    Some(k) => host.message("Drawing plane locked: {plane}", &[("plane", PLANE_NAMES[k].to_string())]),
                    None => host.message("Drawing plane free", &[]),
                }
                Reply::redraw()
            }
            _ => Reply::ignored(),
        }
    }

}

impl Tool for RectangleTool {
    fn info(&self) -> &'static ToolInfo { &INFO }

    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Activate => {
                self.reset();
                self.last = None;
                self.from_center = false;
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
                self.last = None;
                Reply::redraw()
            }
            ToolEvent::Hover(p) => match p.world {
                Some(w) => self.hover(d3(w), p.mods.shift),
                None => Reply::ignored(),
            },
            ToolEvent::Click(p) | ToolEvent::DoubleClick(p) | ToolEvent::TripleClick(p) => match p.world {
                Some(w) => self.click(d3(w), host),
                None => Reply::ignored(),
            },
            ToolEvent::Value(v) => self.value(v, host),
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
        let (Some(_), Some(hover)) = (self.start, self.hover) else { return out };
        let (anchor, far) = self.span(hover);
        let square = self.square_corner(anchor, far, false).1;
        let c = self.corners(anchor, far);
        let p = |v: DVec3| [v.x, v.y, v.z];
        for i in 0..4 {
            out.lines.push(Preview::rubber(p(c[i]), p(c[(i + 1) % 4])));
        }
        if square {
            out.lines.push(Preview::rubber(p(c[0]), p(c[2])));
        }
        out.value = Some(ValueAnchor { at: p((anchor + far) * 0.5), label: "Dimensions" });
        out
    }

    fn busy(&self) -> bool { self.start.is_some() }

    fn state(&self) -> serde_json::Value {
        let pt = |v: Option<DVec3>| v.map(|v| serde_json::json!([v.x, v.y, v.z])).unwrap_or(serde_json::Value::Null);
        let plane = |p: Option<Plane>| p.map(|(a, n)| serde_json::json!([[a.x, a.y, a.z], [n.x, n.y, n.z]]))
            .unwrap_or(serde_json::Value::Null);
        let label = match (self.start, self.hover) {
            (Some(_), Some(h)) => {
                let (anchor, far) = self.span(h);
                let square = self.square_corner(anchor, far, false).1;
                let (du, dv) = self.dims(anchor, far);
                let mid = (anchor + far) * 0.5;
                serde_json::json!([du.abs(), dv.abs(), square, [mid.x, mid.y, mid.z]])
            }
            _ => serde_json::Value::Null,
        };
        serde_json::json!({
            "start_point": pt(self.start),
            "hover_point": pt(self.hover),
            "work_plane": plane(self.work_plane),
            "drawing_plane": plane(Some(self.drawing_plane())),
            "from_center": self.from_center,
            "square": self.square_locked(),
            "label": label,
            "retype": self.last.is_some(),
            "live_side": match self.live_side { Some(Side::U) => "u".into(), Some(Side::V) => "v".into(), None => serde_json::Value::Null },
            "arrow_square": self.arrow_square,
            "shift_square": self.shift_square,
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
            "from_center" => self.from_center = value.as_bool().unwrap_or(false),
            "arrow_square" => self.arrow_square = value.as_bool().unwrap_or(false),
            "shift_square" => self.shift_square = value.as_bool().unwrap_or(false),
            "live_side" => self.live_side = match value.as_str() { Some("u") => Some(Side::U), Some("v") => Some(Side::V), _ => None },
            "reset" => self.reset(),
            _ => return false,
        }
        true
    }

    fn plane_lock(&mut self) -> Option<&mut PlaneLock> { Some(&mut self.lock) }

    fn query(&self, what: &str, arg: &serde_json::Value) -> serde_json::Value {
        let j = |v: DVec3| serde_json::json!([v.x, v.y, v.z]);
        let point = |v: &serde_json::Value| super::json_p3(v).map(d3);
        let pair = || {
            let a = arg.as_array()?;
            Some((point(a.first()?)?, point(a.get(1)?)?))
        };
        match what {
            "span" if self.start.is_some() => match point(arg) {
                Some(c) => { let (a, b) = self.span(c); serde_json::json!([j(a), j(b)]) }
                None => serde_json::Value::Null,
            },
            "dimensions" => pair().map(|(a, b)| { let (u, v) = self.dims(a, b); serde_json::json!([u, v]) })
                .unwrap_or_default(),
            "square_corner" => pair().map(|(a, b)| { let (f, sq) = self.square_corner(a, b, false); serde_json::json!([j(f), sq]) })
                .unwrap_or_default(),
            "corners" => pair().map(|(a, b)| serde_json::json!(self.corners(a, b).map(j))).unwrap_or_default(),
            "degenerate" => match point(arg) {
                Some(w) => {
                    let (t, args) = self.degenerate_reason(w);
                    let m: serde_json::Map<String, serde_json::Value> =
                        args.into_iter().map(|(k, v)| (k.to_string(), v.into())).collect();
                    serde_json::json!([t, m])
                }
                None => serde_json::Value::Null,
            },
            _ => serde_json::Value::Null,
        }
    }
}
