//! View-only navigation. Camera writes use the host's set_camera verb.
//! The host supplies visible-face ray distances and owns the input timer.
use super::{
    add, json_p3, len, scale, sub, unit, Mods, Preview, Reply, Tool, ToolEvent, ToolHost, ToolInfo,
    Value, P3,
};
use serde_json::{json, Value as Json};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq)]
pub enum Navigation {
    Place {
        base: P3,
        look: Option<P3>,
    },
    Look([f64; 2]),
    Tick {
        dt: f64,
        mods: Mods,
    },
    Walk {
        delta: [f64; 2],
        dt: f64,
        mods: Mods,
    },
    Keys {
        keys: Vec<u32>,
        dt: f64,
        mods: Mods,
    },
}

const fn info(kind: &'static str, name: &'static str, desc: &'static str, place: bool) -> ToolInfo {
    ToolInfo {
        kind,
        name,
        description: desc,
        shortcut: "",
        uses_snap: place,
        box_select: false,
        vcb_label: Some(if place { "Height offset" } else { "Eye height" }),
        magnetic_axis_deg: None,
        screen_axis_px: None,
        axis_source: false,
        axis_source_empty_only: false,
    }
}
static PLACE: ToolInfo = info("position_camera", "Position Camera", "Click where to stand and the eye goes there at a person's height; drag to also say where to look.", true);
static LOOK: ToolInfo = info(
    "look_around",
    "Look Around",
    "Drag to turn your head; the eye stays where it is.",
    false,
);
static WALK: ToolInfo = info("walk", "Walk", "Drag to walk through the model at eye height: up and down go forward and back, left and right turn.", false);
static FIRST: ToolInfo = info(
    "first_person",
    "First Person",
    "Walk as in a game: W, A, S and D move, the mouse turns the head.",
    false,
);

fn forward(host: &mut dyn ToolHost) -> P3 {
    json_p3(&host.call("forward", Json::Null)).unwrap_or([0., 1., 0.])
}
fn heading(host: &mut dyn ToolHost) -> P3 {
    let [x, y, _] = forward(host);
    let flat = [x, y, 0.];
    if len(flat) > 1e-6 {
        unit(flat)
    } else {
        [0., 1., 0.]
    }
}
fn height(host: &mut dyn ToolHost) -> f64 {
    host.call("eye_height", Json::Null).as_f64().unwrap_or(1.68)
}
fn ray(host: &mut dyn ToolHost, at: P3, dir: P3, reach: Option<f64>) -> Option<f64> {
    host.call("ray_distance", json!([at, dir, reach])).as_f64()
}
pub(super) fn blocked(
    host: &mut dyn ToolHost,
    eye: P3,
    delta: P3,
    h: f64,
    clearance: f64,
    knee: f64,
) -> bool {
    let length = len(delta);
    if length < 1e-12 {
        return false;
    }
    let reach = length + clearance;
    let dir = scale(delta, 1. / length);
    [eye, sub(eye, [0., 0., (h - knee).max(0.)])]
        .into_iter()
        .any(|at| ray(host, at, dir, Some(reach)).is_some_and(|d| d < reach))
}

fn advance(host: &mut dyn ToolHost, delta: P3, follow_floor: bool, through: bool) {
    let Some(eye) = host.eye() else { return };
    let h = height(host);
    if !through && blocked(host, eye, delta, h, 0.35, 0.55) {
        return;
    }
    host.call("set_camera", json!({"translation": delta}));
    if through || !follow_floor {
        return;
    }
    let Some([x, y, z]) = host.eye() else { return };
    let above = z - h + 0.5;
    let floor = ray(host, [x, y, above], [0., 0., -1.], None)
        .map_or(0_f64.min(z - h), |distance| above - distance);
    let dz = floor + h - z;
    if dz.abs() > 1e-6 {
        host.call("set_camera", json!({"translation": [0., 0., dz]}));
    }
}

fn walk(host: &mut dyn ToolHost, delta: [f64; 2], dt: f64, mods: Mods) {
    let gain = |p: f64| {
        if p.abs() < 4. {
            0.
        } else {
            (p / 150.).clamp(-1., 1.)
        }
    };
    let [x, y] = delta.map(gain);
    let stride = 1.5 * if mods.ctrl { 3. } else { 1. } * dt;
    if mods.shift {
        let [fx, fy, _] = heading(host);
        let delta = scale([fy * x, -fx * x, y], stride);
        if len(delta) >= 1e-9 {
            advance(host, delta, false, mods.alt);
        }
    } else {
        let turn = x * 60. * dt;
        if turn.abs() > 1e-9 {
            host.call("set_camera", json!({"turn": [turn, 0.]}));
        }
        if (y * stride).abs() >= 1e-12 {
            let delta = scale(heading(host), y * stride);
            advance(host, delta, true, mods.alt);
        }
    }
}
fn motion(key: u32) -> Option<P3> {
    match char::from_u32(key)? {
        'W' => Some([1., 0., 0.]),
        'S' => Some([-1., 0., 0.]),
        'D' => Some([0., 1., 0.]),
        'A' => Some([0., -1., 0.]),
        'E' => Some([0., 0., 1.]),
        'Q' => Some([0., 0., -1.]),
        _ => None,
    }
}
fn first_person(host: &mut dyn ToolHost, keys: impl Iterator<Item = u32>, dt: f64, mods: Mods) {
    let [f, r, z] = keys.filter_map(motion).fold([0.; 3], add);
    if [f, r, z] == [0.; 3] {
        return;
    }
    let [x, y, _] = heading(host);
    let dir = unit([x * f + y * r, y * f - x * r, z]);
    let speed = if mods.shift { 4.5 } else { 1.5 };
    advance(host, scale(dir, speed * dt), z == 0., mods.alt);
}

pub struct Walkthrough {
    info: &'static ToolInfo,
    anchor: Option<[f64; 2]>,
    cursor: [f64; 2],
    mods: Mods,
    pressed: Option<P3>,
    look_at: Option<P3>,
    last_base: Option<P3>,
    preview_height: f64,
    held: BTreeSet<u32>,
}
impl Walkthrough {
    pub fn new(kind: &str) -> Self {
        let info = match kind {
            "position_camera" => &PLACE,
            "look_around" => &LOOK,
            "walk" => &WALK,
            "first_person" => &FIRST,
            _ => unreachable!(),
        };
        Self {
            info,
            anchor: None,
            cursor: [0.; 2],
            mods: Mods::default(),
            pressed: None,
            look_at: None,
            last_base: None,
            preview_height: 1.68,
            held: BTreeSet::new(),
        }
    }
    fn place(&mut self, host: &mut dyn ToolHost, base: P3, at: Option<P3>) {
        let h = height(host);
        let eye = add(base, [0., 0., h]);
        let direction = at
            .map(|p| sub(p, eye))
            .filter(|d| len(*d) > 1e-6)
            .unwrap_or_else(|| heading(host));
        host.call(
            "set_camera",
            json!({"eye":eye, "direction":direction, "ortho":false}),
        );
        self.last_base = Some(base);
        host.call("eye_placed", json!(h));
    }
    fn clear(&mut self) {
        self.pressed = None;
        self.look_at = None;
        self.anchor = None;
        self.held.clear();
    }
}
impl Tool for Walkthrough {
    fn info(&self) -> &'static ToolInfo {
        self.info
    }
    fn state(&self) -> Json {
        json!({"anchor":self.anchor, "held":self.held})
    }
    fn set(&mut self, key: &str, val: &Json) -> bool {
        if key != "anchor" {
            return false;
        }
        self.anchor = serde_json::from_value(val.clone()).ok().flatten();
        true
    }
    fn query(&self, what: &str, arg: &Json) -> Json {
        if what == "claims_key" {
            return json!(arg
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .and_then(motion)
                .is_some());
        }
        Json::Null
    }
    fn preview(&self) -> Preview {
        let mut preview = Preview::default();
        if let (Some(base), Some(at)) = (self.pressed, self.look_at) {
            preview.lines.push(Preview::rubber(
                add(base, [0., 0., self.preview_height]),
                at,
            ));
        }
        preview
    }
    fn event(&mut self, event: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        let kind = self.info.kind;
        match event {
            ToolEvent::Activate | ToolEvent::Deactivate | ToolEvent::Cancel => {
                self.clear();
                if matches!(kind, "walk" | "first_person") {
                    host.call("timer", json!(false));
                }
            }
            ToolEvent::Click(p) if kind != "first_person" => {
                self.anchor = Some(p.px);
                self.cursor = p.px;
                self.mods = p.mods;
                self.pressed = if kind == "position_camera" {
                    p.world
                } else {
                    None
                };
                self.look_at = None;
                if kind == "walk" {
                    host.call("timer", json!(true));
                }
            }
            ToolEvent::Hover(p) if self.anchor.is_some() => {
                let [ax, ay] = self.anchor.unwrap();
                match kind {
                    "position_camera"
                        if self.look_at.is_some() || (p.px[0] - ax).hypot(p.px[1] - ay) > 6. =>
                    {
                        self.look_at = p.world;
                        self.preview_height = height(host);
                    }
                    "look_around" => {
                        let size = host.call("view_size", Json::Null);
                        let w = size[0].as_f64().unwrap_or(1000.).max(1.);
                        let h = size[1].as_f64().unwrap_or(700.).max(1.);
                        host.call(
                            "set_camera",
                            json!({"turn":[(p.px[0]-ax)*180./w, (ay-p.px[1])*90./h]}),
                        );
                        self.anchor = Some(p.px);
                    }
                    "walk" => {
                        self.cursor = p.px;
                        self.mods = p.mods;
                    }
                    _ => return Reply::ignored(),
                }
            }
            ToolEvent::Release(_) if kind != "first_person" => {
                if let Some(base) = self.pressed.take() {
                    let at = self.look_at.take();
                    self.place(host, base, at);
                    host.call("handoff", json!("look_around"));
                }
                self.anchor = None;
                if kind == "walk" {
                    host.call("timer", json!(false));
                }
            }
            ToolEvent::Value(Value::Length(v)) => {
                let eye = if kind == "position_camera" {
                    if *v <= 0. {
                        return Reply::ignored();
                    }
                    host.call("set_eye_height", json!(v));
                    self.preview_height = height(host);
                    self.last_base.map(|p| add(p, [0., 0., *v]))
                } else {
                    host.eye().map(|[x, y, _]| [x, y, *v])
                };
                if let Some(eye) = eye {
                    let direction = forward(host);
                    host.call("set_camera", json!({"eye":eye, "direction":direction}));
                }
            }
            ToolEvent::Key { key, mods } if kind == "first_person" && !mods.ctrl => {
                let Some(k) = key.parse().ok().filter(|&k| motion(k).is_some()) else {
                    return Reply::ignored();
                };
                self.held.insert(k);
                host.call("timer", json!(true));
            }
            ToolEvent::KeyRelease { key, .. } if kind == "first_person" => {
                let Some(k) = key.parse().ok().filter(|&k| motion(k).is_some()) else {
                    return Reply::ignored();
                };
                self.held.remove(&k);
                if self.held.is_empty() {
                    host.call("timer", json!(false));
                }
            }
            ToolEvent::Navigate(nav) => match nav {
                Navigation::Place { base, look } => self.place(host, *base, *look),
                Navigation::Look([dx, dy]) => {
                    let rate = host.call("look_rate", Json::Null).as_f64().unwrap_or(0.05);
                    host.call("set_camera", json!({"turn":[dx*rate,-dy*rate]}));
                }
                Navigation::Walk { delta, dt, mods } => walk(host, *delta, *dt, *mods),
                Navigation::Keys { keys, dt, mods } => {
                    first_person(host, keys.iter().copied(), *dt, *mods)
                }
                Navigation::Tick { dt, mods } if kind == "walk" => {
                    let Some([x, y]) = self.anchor else {
                        return Reply::ignored();
                    };
                    let held = Mods {
                        shift: mods.shift || self.mods.shift,
                        ctrl: mods.ctrl || self.mods.ctrl,
                        alt: mods.alt || self.mods.alt,
                        meta: mods.meta || self.mods.meta,
                    };
                    walk(host, [self.cursor[0] - x, y - self.cursor[1]], *dt, held);
                }
                Navigation::Tick { dt, mods } if !self.held.is_empty() => {
                    first_person(host, self.held.iter().copied(), *dt, *mods)
                }
                _ => return Reply::ignored(),
            },
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
}
