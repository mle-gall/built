//! Measuring tape, projected edge offsets and construction guides.
use super::{
    add, dot, json_p3, len, scale, sub, unit, Command, Pointer, Preview, Reply, Tool, ToolEvent,
    ToolHost, ToolInfo, Value, P3,
};
use serde_json::{json, Value as Json};

static INFO: ToolInfo = ToolInfo {
    kind: "tape",
    name: "Tape Measure",
    shortcut: "T",
    description: "Measure distances, or pull guide lines off edges to draw against.",
    uses_snap: true,
    box_select: false,
    vcb_label: Some("Distance"),
    magnetic_axis_deg: Some(3.),
    screen_axis_px: Some(9.),
    axis_source: true,
    axis_source_empty_only: false,
};

#[derive(Default, PartialEq, Copy, Clone)]
enum Mode {
    #[default]
    Line,
    Point,
    Measure,
}
impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Point => "point",
            Self::Measure => "measure",
        }
    }
    fn next(self) -> Self {
        match self {
            Self::Line => Self::Point,
            Self::Point => Self::Measure,
            Self::Measure => Self::Line,
        }
    }
}
fn named(kind: &str) -> bool {
    matches!(
        kind,
        "endpoint"
            | "origin"
            | "intersection"
            | "intersection_edge"
            | "intersection_face"
            | "component_origin"
            | "center"
    )
}

#[derive(Default)]
pub struct Tape {
    start: Option<P3>,
    hover: Option<P3>,
    direction: Option<P3>,
    from_point: bool,
    measured: Option<f64>,
    mode: Mode,
}

impl Tape {
    fn reset(&mut self, host: &mut dyn ToolHost) {
        self.start = None;
        self.from_point = false;
        self.direction = None;
        host.call("clear_source", Json::Null);
    }
    fn begin(&mut self, p: &Pointer, point: P3, kind: &str, host: &mut dyn ToolHost) {
        self.from_point = named(kind);
        self.start = Some(point);
        let hit = host.call("tape_source", json!([p.px, kind, self.from_point]));
        if let Some(snapped) = json_p3(&hit["point"]) {
            self.start = Some(snapped);
        }
        if self.mode != Mode::Line
            || matches!(
                kind,
                "endpoint"
                    | "origin"
                    | "close"
                    | "intersection"
                    | "intersection_edge"
                    | "intersection_face"
            )
        {
            return;
        }
        let (Some(a), Some(b)) = (json_p3(&hit["a"]), json_p3(&hit["b"])) else {
            return;
        };
        let d = sub(b, a);
        if len(d) < 1e-6 {
            return;
        }
        let axis = unit(d);
        self.direction = Some(axis);
        self.start = Some(add(a, scale(axis, dot(sub(self.start.unwrap(), a), axis))));
    }
    fn delta(&self, point: P3) -> Option<P3> {
        let d = sub(point, self.start?);
        Some(if let Some(axis) = self.direction {
            sub(d, scale(axis, dot(d, axis)))
        } else {
            d
        })
    }
    fn finish(
        &mut self,
        point: P3,
        kind: &str,
        value: Option<f64>,
        host: &mut dyn ToolHost,
    ) -> Reply {
        let Some(start) = self.start else {
            return Reply::ignored();
        };
        // An edge acquired in line mode stays the measurement reference when Ctrl cycles.
        let line = self.direction.is_some() && self.mode != Mode::Point;
        let mut d = if line {
            self.delta(point).unwrap()
        } else {
            sub(point, start)
        };
        let point_guide = self.mode == Mode::Point
            || (self.mode == Mode::Line
                && self.from_point
                && (value.is_some() || (!named(kind) && len(d) > 1e-9)));
        if let Some(v) = value {
            if (!line && !point_guide) || !v.is_finite() || len(d) < 1e-9 {
                return Reply::ignored();
            }
            d = scale(unit(d), v);
        }
        let dist = len(d);
        let pos = add(start, d);
        let mut result = Reply::redraw();
        if point_guide || line {
            let origin =
                (point_guide && (dist >= 1e-9 || self.mode == Mode::Line)).then_some(start);
            let direction = if line { self.direction } else { None };
            if pos.iter().chain(start.iter()).all(|v| v.is_finite())
                && host
                    .run(&Command {
                        name: "add_guide",
                        args: json!({"point":pos,"direction":direction,"origin":origin}),
                    })
                    .ok
            {
                let label = if line {
                    "line"
                } else if origin.is_some() {
                    "segment"
                } else {
                    "point"
                };
                host.call("tape_message", json!([label, dist]));
                result = Reply::committed();
            }
        } else {
            self.measured = Some(dist);
            host.call("tape_message", json!(["measure", dist]));
        }
        self.reset(host);
        result
    }
}
impl Tool for Tape {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn busy(&self) -> bool {
        self.start.is_some()
    }
    fn state(&self) -> Json {
        json!({"start_point":self.start,"hover_point":self.hover,"direction":self.direction,"mode":self.mode.name(),"measured":self.measured})
    }
    fn set(&mut self, key: &str, v: &Json) -> bool {
        match key {
            "start_point" => self.start = json_p3(v),
            "hover_point" => self.hover = json_p3(v),
            "direction" => self.direction = json_p3(v).map(unit),
            "mode" => {
                self.mode = match v.as_str() {
                    Some("line") => Mode::Line,
                    Some("point") => Mode::Point,
                    Some("measure") => Mode::Measure,
                    _ => return false,
                }
            }
            _ => return false,
        }
        true
    }
    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Activate | ToolEvent::Deactivate | ToolEvent::Cancel => {
                if matches!(ev, ToolEvent::Activate) {
                    self.mode = Mode::Line;
                }
                if matches!(ev, ToolEvent::Deactivate) {
                    self.hover = None;
                }
                self.reset(host);
            }
            ToolEvent::Hover(p) => self.hover = p.world,
            ToolEvent::Click(p) => {
                let Some(pt) = p.world else {
                    return Reply::ignored();
                };
                let k = p.snap.as_ref().map(|s| s.kind.as_str()).unwrap_or("none");
                if self.start.is_some() {
                    return self.finish(pt, k, None, host);
                }
                self.begin(p, pt, k, host);
            }
            ToolEvent::Value(Value::Length(v)) if self.hover.is_some() => {
                return self.finish(self.hover.unwrap(), "none", Some(*v), host)
            }
            ToolEvent::Key { key, .. } if key == "Control" => (),
            ToolEvent::KeyRelease { key, tapped: true } if key == "Control" => {
                self.mode = self.mode.next();
                host.call("tape_mode", json!(self.mode.name()));
            }
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
    fn preview(&self) -> Preview {
        let mut p = Preview::default();
        if let (Some(start), Some(delta)) = (self.start, self.hover.and_then(|p| self.delta(p))) {
            p.lines.push(Preview::rubber(start, add(start, delta)));
        }
        p
    }
    fn query(&self, what: &str, arg: &Json) -> Json {
        if what == "named" {
            return json!(arg.as_str().is_some_and(named));
        }
        let (Some(start), Some(delta)) = (self.start, self.hover.and_then(|p| self.delta(p)))
        else {
            return Json::Null;
        };
        match what {
            "label" => json!([len(delta), add(start, scale(delta, 0.5))]),
            "guide" => self
                .direction
                .map(|axis| json!([add(start, delta), axis]))
                .unwrap_or(Json::Null),
            _ => Json::Null,
        }
    }
}
