//! Scale gesture and hot retype. The host resolves screen-space grips and
//! exposes the original resident rows for live preview.
use serde_json::{json, Value as Json};

use super::scale_grip::{Grip, MIN_FACTOR};
use super::{json_p3, Command, Reply, Tool, ToolEvent, ToolHost, ToolInfo, Value, P3};

static INFO: ToolInfo = ToolInfo {
    kind: "scale",
    name: "Scale",
    shortcut: "S",
    description:
        "Resize the selection by dragging the grips of its box; the corners keep its proportions.",
    uses_snap: false,
    box_select: false,
    vcb_label: Some("Scale"),
    magnetic_axis_deg: None,
    screen_axis_px: None,
    axis_source: false,
    axis_source_empty_only: false,
};

#[derive(Default)]
enum Phase {
    #[default]
    Idle,
    Held {
        px: [f64; 2],
        moved: bool,
    },
    Retype {
        args: Json,
        spec: Grip,
        revision: u64,
    },
}

#[derive(Default)]
pub struct ScaleTool {
    phase: Phase,
    ctrl_pressed: bool,
}

impl ScaleTool {
    fn spec(host: &mut dyn ToolHost) -> Option<Grip> {
        let data = host.call("grip_spec", Json::Null);
        Some(Grip {
            extents: json_p3(&data["extents"])?,
            mask: serde_json::from_value(data["mask"].clone()).ok()?,
            uniform: data["uniform"].as_bool()?,
        })
    }

    fn commit(&mut self, factors: P3, host: &mut dyn ToolHost) -> Reply {
        host.call("revert", Json::Null);
        self.phase = Phase::Idle;
        let changed = factors.iter().any(|f| (f - 1.0).abs() > 1e-9);
        if changed
            && factors
                .iter()
                .all(|f| f.is_finite() && f.abs() >= MIN_FACTOR)
        {
            if let Some(spec) = Self::spec(host) {
                let mut args = host.call("scale_targets", Json::Null);
                args["factors"] = json!(factors);
                let answer = host.run(&Command {
                    name: "scale_selection",
                    args: args.clone(),
                });
                if answer.ok {
                    self.phase = Phase::Retype {
                        args,
                        spec,
                        revision: answer.revision,
                    };
                }
            }
        }
        host.call("end_grip", Json::Null);
        if matches!(self.phase, Phase::Retype { .. }) {
            Reply::committed()
        } else {
            Reply::redraw()
        }
    }

    fn value(&mut self, value: &Value, absolute: bool, host: &mut dyn ToolHost) -> Reply {
        let values = match value {
            Value::Length(x) => vec![*x],
            Value::Pair(x, y) => vec![*x, *y],
            Value::Triple(x, y, z) => vec![*x, *y, *z],
            Value::List(xs) => xs.clone(),
        };
        match &mut self.phase {
            Phase::Held { .. } => {
                let factors = Self::spec(host).and_then(|s| s.typed(&values, absolute));
                factors.map_or_else(Reply::ignored, |f| self.commit(f, host))
            }
            Phase::Retype {
                spec,
                args,
                revision,
            } => {
                if host.revision() != *revision {
                    self.phase = Phase::Idle;
                    return Reply::ignored();
                }
                let Some(f) = spec.typed(&values, absolute) else {
                    return Reply::ignored();
                };
                if f.iter().any(|x| !x.is_finite() || *x == 0.0) {
                    return Reply::ignored();
                }
                if !host
                    .run(&Command {
                        name: "undo",
                        args: json!({}),
                    })
                    .ok
                {
                    return Reply::ignored();
                }
                args["factors"] = json!(f);
                let answer = host.run(&Command {
                    name: "scale_selection",
                    args: args.clone(),
                });
                if !answer.ok {
                    host.run(&Command {
                        name: "redo",
                        args: json!({}),
                    });
                    self.phase = Phase::Idle;
                    return Reply::ignored();
                }
                *revision = answer.revision;
                Reply::committed()
            }
            Phase::Idle => Reply::ignored(),
        }
    }
}

impl Tool for ScaleTool {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn busy(&self) -> bool {
        matches!(self.phase, Phase::Held { .. })
    }

    fn set(&mut self, key: &str, value: &Json) -> bool {
        if key == "grab" {
            if let Ok(px) = serde_json::from_value(value.clone()) {
                self.phase = Phase::Held { px, moved: false };
                return true;
            }
        }
        false
    }

    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Activate | ToolEvent::Cancel | ToolEvent::Deactivate => {
                self.phase = Phase::Idle;
                self.ctrl_pressed = false;
                let verb = match ev {
                    ToolEvent::Activate => "activate_box",
                    ToolEvent::Deactivate => "deactivate_box",
                    _ => "cancel_box",
                };
                host.call(verb, Json::Null);
            }
            ToolEvent::Click(p) if self.busy() => {
                let reading = host.call("factors", json!(p.px));
                if let Some(f) = json_p3(&reading) {
                    return self.commit(f, host);
                }
            }
            ToolEvent::Click(p) => {
                self.phase = Phase::Idle;
                if host.call("pick_grip", json!(p.px)).as_bool() == Some(true) {
                    self.phase = Phase::Held {
                        px: p.px,
                        moved: false,
                    };
                }
            }
            ToolEvent::Hover(p) => {
                if let Phase::Held { px, moved } = &mut self.phase {
                    if let Some(f) = json_p3(&host.call("factors", json!(p.px))) {
                        if f.iter().all(|x| x.is_finite() && x.abs() >= MIN_FACTOR) {
                            *moved |= (p.px[0] - px[0]).hypot(p.px[1] - px[1]) > 4.0;
                            host.call("scale_preview", json!(f));
                        }
                    }
                } else {
                    host.call("hover_grip", json!(p.px));
                }
            }
            ToolEvent::Release(_) if matches!(self.phase, Phase::Held { moved: true, .. }) => {
                if let Some(f) = json_p3(&host.call("preview_factors", Json::Null)) {
                    if f.iter().any(|v| (v - 1.0).abs() > 1e-9) {
                        return self.commit(f, host);
                    }
                }
            }
            ToolEvent::Key { key, .. } if key == "Control" => {
                self.ctrl_pressed = true;
                host.call("toggle_center", json!(true));
            }
            ToolEvent::Key { key, .. } if key == "Shift" => {
                host.call("toggle_uniform", Json::Null);
            }
            ToolEvent::KeyRelease { key, tapped } if key == "Control" && self.ctrl_pressed => {
                self.ctrl_pressed = false;
                if !tapped {
                    host.call("toggle_center", json!(false));
                }
            }
            ToolEvent::Value(v) => return self.value(v, false, host),
            ToolEvent::AbsoluteValue(v) => return self.value(v, true, host),
            ToolEvent::ScaleFactors(f) if self.busy() => return self.commit(*f, host),
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
}
