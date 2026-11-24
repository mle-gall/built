//! The plan compass and the angle clockwise from model green.
use super::{add, sub, Command, Preview, Reply, Tool, ToolEvent, ToolHost, ToolInfo, Value, P3};
use serde_json::{json, Value as Json};
static INFO: ToolInfo = ToolInfo::basic(
    "north",
    "North",
    "Shift+N",
    "Set where north lies: click a centre, then click toward north. The sun's shadows turn with it.",
    true,
    Some("Angle"),
);
fn wrap(a: f64) -> f64 {
    let d = a % 360.;
    if d <= -180. {
        d + 360.
    } else if d > 180. {
        d - 360.
    } else {
        d
    }
}
fn toward(c: P3, p: P3) -> Option<f64> {
    let [x, y, _] = sub(p, c);
    (x.hypot(y) >= 1e-9).then(|| wrap(x.atan2(y).to_degrees()))
}
fn compass(c: P3, r: f64, deg: f64) -> Preview {
    let (s, co) = deg.to_radians().sin_cos();
    let at = |a: f64, b: f64| add(c, [s * a + co * b, co * a - s * b, 0.]);
    let rim = |i: i32| {
        let (y, x) = (std::f64::consts::TAU * i as f64 / 64.).sin_cos();
        at(r * x, r * y)
    };
    let mut p = Preview::default();
    for i in 0..64 {
        p.lines.push(Preview::rubber(rim(i), rim((i + 1) % 64)));
    }
    for i in 0..24 {
        let (y, x) = (15. * i as f64).to_radians().sin_cos();
        let inner = if i % 6 == 0 { 0.78 } else { 0.9 };
        p.lines.push(Preview::rubber(
            at(r * inner * x, r * inner * y),
            at(r * x, r * y),
        ));
    }
    for (a, b) in [
        ((-0.5, 0.), (1.15, 0.)),
        ((1.15, 0.), (0.97, 0.108)),
        ((1.15, 0.), (0.97, -0.108)),
        ((1.27, -0.11), (1.57, -0.11)),
        ((1.57, -0.11), (1.27, 0.11)),
        ((1.27, 0.11), (1.57, 0.11)),
    ] {
        p.lines
            .push(Preview::rubber(at(a.0 * r, a.1 * r), at(b.0 * r, b.1 * r)));
    }
    p
}
pub struct North {
    center: Option<P3>,
    hover: Option<P3>,
    radius: f64,
    near: bool,
    exact: bool,
    north: f64,
}
impl Default for North {
    fn default() -> Self {
        Self {
            center: None,
            hover: None,
            radius: 1.,
            near: false,
            exact: false,
            north: 0.,
        }
    }
}
impl North {
    fn pending(&self) -> Option<f64> {
        let d = toward(self.center?, self.hover?)?;
        Some(if self.near {
            wrap((d / 15.).round_ties_even() * 15.)
        } else if self.exact {
            d
        } else {
            wrap((d * 10.).round_ties_even() / 10.)
        })
    }
    fn reset(&mut self) {
        self.center = None;
        self.near = false;
        self.exact = false;
    }
    fn commit(&mut self, h: &mut dyn ToolHost, d: f64) -> Reply {
        let a = wrap(d);
        let answer = h.run(&Command {
            name: "set_north",
            args: json!({"degrees":a}),
        });
        self.north = a;
        self.reset();
        h.call("north_done", json!([a, answer.ok]));
        if answer.ok {
            Reply::committed()
        } else {
            Reply::redraw()
        }
    }
}
impl Tool for North {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn state(&self) -> Json {
        json!({"centre":self.center,"hover_point":self.hover,"radius":self.radius,"pending":self.pending()})
    }
    fn set(&mut self, key: &str, value: &Json) -> bool {
        if key != "north" { return false; }
        if let Some(deg) = value.as_f64() { self.north = deg; return true; }
        false
    }
    fn query(&self, what: &str, v: &Json) -> Json {
        match what {
            "angle" => serde_json::from_value::<[P3; 2]>(v.clone())
                .ok()
                .map(|[a, b]| json!(toward(a, b)))
                .unwrap_or_default(),
            "compass" => serde_json::from_value::<(P3, f64, f64)>(v.clone())
                .ok()
                .map(|(a, r, d)| {
                    json!(compass(a, r, d)
                        .lines
                        .iter()
                        .map(|l| (l.a, l.b))
                        .collect::<Vec<_>>())
                })
                .unwrap_or_default(),
            _ => Json::Null,
        }
    }
    fn event(&mut self, ev: &ToolEvent, h: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Activate => {
                self.reset();
                self.north = h.call("north_angle", Json::Null).as_f64().unwrap_or(0.);
            }
            ToolEvent::Cancel | ToolEvent::Deactivate => {
                self.reset();
                if matches!(ev, ToolEvent::Deactivate) {
                    self.hover = None;
                }
            }
            ToolEvent::Value(Value::Length(d)) => return self.commit(h, *d),
            ToolEvent::Click(p) if self.center.is_none() => self.center = p.world,
            ToolEvent::Click(p) | ToolEvent::Hover(p) => {
                self.hover = p.world;
                self.exact = p
                    .snap
                    .as_ref()
                    .is_some_and(|s| !matches!(s.kind.as_str(), "none" | "on_face" | ""));
                self.north = h.call("north_angle", Json::Null).as_f64().unwrap_or(0.);
                if let Some(c) = self.center.or(self.hover) {
                    let projections = h.call(
                        "north_pixels",
                        json!([c, add(c, [1., 0., 0.]), add(c, [0., 1., 0.])]),
                    );
                    if let Ok([o, x, y]) = serde_json::from_value::<[[f64; 2]; 3]>(projections) {
                        let dist = |v: [f64; 2]| (v[0] - o[0]).hypot(v[1] - o[1]);
                        let scale = dist(x).max(dist(y));
                        if scale > 1e-9 {
                            self.radius = 70. / scale;
                        }
                        self.near = self.center.is_some() && dist(p.px) <= 87.5;
                    }
                }
                if matches!(ev, ToolEvent::Click(_)) {
                    if let Some(d) = self.pending() {
                        return self.commit(h, d);
                    }
                }
            }
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
    fn preview(&self) -> Preview {
        let Some(c) = self.center.or(self.hover) else {
            return Preview::default();
        };
        let mut p = compass(c, self.radius, self.pending().unwrap_or(self.north));
        if let (Some(a), Some(b)) = (self.center, self.hover) {
            p.lines.push(Preview::rubber(a, [b[0], b[1], a[2]]));
        }
        p
    }
}
