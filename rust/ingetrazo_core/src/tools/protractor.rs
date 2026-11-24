//! Protractor arms, signed angles and re-aiming the last live guide.
use super::{
    add, dot, json_p3, len, scale, sub, unit, Command, Reply, Tool, ToolEvent, ToolHost, ToolInfo,
    Value, P3,
};
use serde_json::{json, Value as Json};
static INFO: ToolInfo = ToolInfo {
    kind: "protractor",
    name: "Protractor",
    shortcut: "Shift+H",
    description: "Measure angles and place guide lines at an angle.",
    uses_snap: true,
    box_select: false,
    vcb_label: Some("Angle"),
    magnetic_axis_deg: Some(3.),
    screen_axis_px: None,
    axis_source: false,
    axis_source_empty_only: false,
};

#[derive(Clone, Copy)]
struct Arm {
    u: P3,
    v: P3,
    zero: f64,
}
impl Arm {
    fn new(basis: [P3; 2], base: P3) -> Self {
        let [u, v] = basis;
        Self {
            u,
            v,
            zero: dot(base, v).atan2(dot(base, u)),
        }
    }
    fn direction(&self, deg: f64) -> P3 {
        let t = self.zero + deg.to_radians();
        unit(add(scale(self.u, t.cos()), scale(self.v, t.sin())))
    }
}
pub fn angle(basis: [P3; 2], a: P3, b: P3) -> Option<f64> {
    let [u, v] = basis;
    let (ax, ay) = (dot(a, u), dot(a, v));
    let (bx, by) = (dot(b, u), dot(b, v));
    if ax.hypot(ay) < 1e-9 || bx.hypot(by) < 1e-9 {
        return None;
    }
    let deg = (by.atan2(bx) - ay.atan2(ax)).to_degrees();
    Some(if deg <= -180. {
        deg + 360.
    } else if deg > 180. {
        deg - 360.
    } else {
        deg
    })
}
pub fn display(mut deg: f64, near: bool) -> f64 {
    if near {
        deg = (deg / 15.).round_ties_even() * 15.;
        if deg <= -180. {
            deg += 360.;
        }
    }
    // Decimal rounding matches Python round(deg, 1), including exact half ties.
    format!("{deg:.1}").parse().unwrap_or(deg)
}
pub fn direction(basis: [P3; 2], base: P3, deg: f64) -> P3 {
    Arm::new(basis, base).direction(deg)
}

pub struct Protractor {
    centre: Option<P3>,
    reference: Option<P3>,
    hover: Option<P3>,
    basis: [P3; 2],
    near: bool,
    exact: bool,
    guides: bool,
    ctrl: bool,
    last: Option<(Arm, f64)>,
}
impl Default for Protractor {
    fn default() -> Self {
        Self {
            centre: None,
            reference: None,
            hover: None,
            basis: [[1., 0., 0.], [0., 1., 0.]],
            near: false,
            exact: false,
            guides: true,
            ctrl: false,
            last: None,
        }
    }
}
impl Protractor {
    fn toggle(&mut self, host: &mut dyn ToolHost, say: bool) {
        self.guides = !self.guides;
        host.call("guide_toggle", json!([self.guides, say]));
    }
    fn commit(&mut self, host: &mut dyn ToolHost, deg: f64) -> Reply {
        let (Some(c), Some(r)) = (self.centre, self.reference) else {
            return Reply::ignored();
        };
        let arm = Arm::new(self.basis, sub(r, c));
        let mut result = Reply::redraw();
        if !self.guides {
            host.call("measured_angle", json!(deg));
        } else {
            let d = arm.direction(deg);
            if c.iter().chain(d.iter()).all(|a| a.is_finite())
                && len(d) > 1e-12
                && host
                    .run(&Command {
                        name: "add_guide",
                        args: json!({"point":c,"direction":d}),
                    })
                    .ok
            {
                self.last = Some((arm, if deg < 0. { -1. } else { 1. }));
                host.call("remember_guide", Json::Null);
                result = Reply::committed();
            }
        }
        self.centre = None;
        self.reference = None;
        host.call("reset_disc", Json::Null);
        result
    }
}
impl Tool for Protractor {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn busy(&self) -> bool {
        self.centre.is_some()
    }
    fn state(&self) -> Json {
        json!({"start_point":self.centre,"ref_point":self.reference,"guides":self.guides,"retype":self.last.is_some()})
    }
    fn set(&mut self, key: &str, value: &Json) -> bool {
        match key {
            "start_point" => self.centre = json_p3(value),
            "ref_point" => self.reference = json_p3(value),
            "guides" => self.guides = value == true,
            "hover_point" => self.hover = json_p3(value),
            _ => return false,
        }
        true
    }
    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Activate | ToolEvent::Cancel | ToolEvent::Deactivate => {
                self.centre = None;
                self.reference = None;
                self.last = None;
                self.ctrl = false;
                if matches!(ev, ToolEvent::Activate) {
                    self.guides = true
                }
                if matches!(ev, ToolEvent::Deactivate) {
                    self.hover = None
                }
                host.call("reset_disc", Json::Null);
            }
            ToolEvent::ProtractorFrame { basis, near, exact } => {
                self.basis = *basis;
                self.near = *near;
                self.exact = *exact;
            }
            ToolEvent::Hover(p) => self.hover = p.world,
            ToolEvent::Click(p) => {
                self.last = None;
                let Some(pt) = p.world else {
                    return Reply::ignored();
                };
                match (self.centre, self.reference) {
                    (None, _) => {
                        self.centre = Some(pt);
                        host.call("disc_centre", json!(pt));
                    }
                    (Some(c), None) if len(sub(pt, c)) >= 1e-6 => self.reference = Some(pt),
                    (Some(c), Some(r)) => {
                        if let Some(a) = angle(self.basis, sub(r, c), sub(pt, c)) {
                            return self.commit(
                                host,
                                if self.exact && !self.near {
                                    a
                                } else {
                                    display(a, self.near)
                                },
                            );
                        }
                    }
                    _ => return Reply::ignored(),
                }
            }
            ToolEvent::Key { key, .. } if key == "Control" => {
                self.toggle(host, true);
                self.ctrl = true;
            }
            ToolEvent::Key { key, .. } if key == "ToggleGuides" || key == "ToggleGuidesQuiet" => {
                self.toggle(host, key == "ToggleGuides")
            }
            ToolEvent::KeyRelease { key, tapped } if key == "Control" && self.ctrl => {
                self.ctrl = false;
                if !tapped {
                    self.toggle(host, false);
                }
            }
            ToolEvent::Value(Value::Length(v)) if v.is_finite() => {
                if let (Some(c), Some(r)) = (self.centre, self.reference) {
                    let clockwise = self
                        .hover
                        .and_then(|h| angle(self.basis, sub(r, c), sub(h, c)))
                        .is_some_and(|a| a < 0.);
                    return self.commit(host, if clockwise { -v.abs() } else { v.abs() });
                }
                if let Some((arm, sign)) = self.last {
                    if let Some(idx) = host.call("live_guide_index", Json::Null).as_u64() {
                        if host
                            .run(&Command {
                                name: "change_guide",
                                args: json!({"index":idx,"direction":arm.direction(sign*v)}),
                            })
                            .ok
                        {
                            return Reply::committed();
                        }
                    }
                    self.last = None;
                }
                return Reply::ignored();
            }
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
}
