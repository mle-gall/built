//! Rotation centre, reference arm, typed angles and the polar array window.
use super::{json_p3, len, sub, Command, Reply, Tool, ToolEvent, ToolHost, ToolInfo, Value, P3};
use serde_json::{json, Value as Json};

static INFO: ToolInfo = ToolInfo {
    kind: "rotate",
    name: "Rotate",
    shortcut: "Q",
    description:
        "Turn the selection around a centre with the protractor; Ctrl leaves a copy behind.",
    uses_snap: true,
    box_select: false,
    vcb_label: Some("Angle"),
    magnetic_axis_deg: Some(3.),
    screen_axis_px: None,
    axis_source: false,
    axis_source_empty_only: false,
};

struct Window {
    args: Json,
    angle: f64,
    sign: f64,
    revision: u64,
}

#[derive(Default)]
pub struct Rotate {
    centre: Option<P3>,
    reference: Option<P3>,
    copy: bool,
    ctrl_pressed: bool,
    window: Option<Window>,
}

impl Rotate {
    fn reset(&mut self) {
        self.centre = None;
        self.reference = None;
        self.copy = false;
    }

    fn commit(&mut self, angle: f64, host: &mut dyn ToolHost) -> Reply {
        host.call("park_rotation", Json::Null);
        self.window = None;
        if angle.is_finite() && angle.abs() > 1e-9 {
            let mut args = host.call("rotation_args", Json::Null);
            args["angles"] = json!([angle]);
            let result = host.run(&Command {
                name: "rotate_selection",
                args: args.clone(),
            });
            if result.ok {
                if args["copy"] == true {
                    args["repeat_uid"] = json!(crate::element_ids::new_uid());
                }
                self.window = Some(Window {
                    args,
                    angle,
                    sign: angle.signum(),
                    revision: result.revision,
                });
            }
        }
        self.reset();
        host.call("end_rotation", Json::Null);
        if self.window.is_some() {
            Reply::committed()
        } else {
            Reply::redraw()
        }
    }

    fn retype(
        &mut self,
        value: Option<f64>,
        array: Option<(i64, &str, Option<f64>)>,
        host: &mut dyn ToolHost,
    ) -> Reply {
        let Some(w) = &mut self.window else {
            return Reply::ignored();
        };
        if w.revision != host.revision() {
            self.window = None;
            return Reply::ignored();
        }
        let array = array.or_else(|| {
            (w.args["copy"] == true && w.args["placements"].as_array()?.len() == 1)
                .then_some((1, "spacing", value))
        });
        let mut rule = None;
        let angles = if let Some((count, mode, step)) = array {
            if w.args["copy"] != true {
                return Reply::ignored();
            }
            let span = w.angle.abs();
            let (count, increment, field) = match mode {
                "spacing" | "fit_max" => {
                    let Some(step) = step.filter(|n| n.is_finite() && *n > 0.) else {
                        return Reply::ignored();
                    };
                    let count = if mode == "spacing" {
                        (span / step + 1e-10).floor()
                    } else {
                        (span / step - 1e-12).ceil()
                    } as i64;
                    let increment = if mode == "spacing" {
                        step
                    } else {
                        span / count as f64
                    };
                    (count, increment.copysign(w.angle), json!({(mode): step}))
                }
                "/" => (count, w.angle / count as f64, json!({"divide":count})),
                _ => (count, w.angle, json!({"count":count})),
            };
            if !(1..=10000).contains(&count) {
                return Reply::ignored();
            }
            rule = Some(field);
            (1..=count).map(|i| increment * i as f64).collect()
        } else if let Some(v) = value {
            vec![w.sign * v]
        } else {
            return Reply::ignored();
        };
        if !host
            .run(&Command {
                name: "undo",
                args: json!({}),
            })
            .ok
        {
            return Reply::ignored();
        }
        w.args["angles"] = json!(angles);
        if let Some(rule) = rule {
            w.args["repeat_rule"] = rule;
            w.args["repeat_angle"] = json!(w.angle);
        } else if let Some(args) = w.args.as_object_mut() {
            args.remove("repeat_rule");
            args.remove("repeat_angle");
        }
        let result = host.run(&Command {
            name: "rotate_selection",
            args: w.args.clone(),
        });
        if !result.ok {
            host.run(&Command {
                name: "redo",
                args: json!({}),
            });
            self.window = None;
            return Reply::ignored();
        }
        w.revision = result.revision;
        if array.is_some() {
            let count = angles.len();
            host.message("{n} copies", &[("n", count.to_string())]);
        } else {
            w.angle = angles[0];
        }
        Reply::committed()
    }

    fn toggle_copy(&mut self, host: &mut dyn ToolHost, say: bool) {
        self.copy = !self.copy;
        host.call("copy_changed", json!([self.copy, say]));
    }
}

impl Tool for Rotate {
    fn cursor_preview(&self, plane: &super::preview::CursorPlane, hover: Option<super::P3>) -> super::Preview {
        plane.protractor(self.centre, self.reference, hover)
    }
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn busy(&self) -> bool {
        self.centre.is_some()
    }
    fn state(&self) -> Json {
        json!({ "start_point": self.centre, "ref_point": self.reference, "copy": self.copy,
             "array_token": self.window.as_ref().filter(|w| w.args["copy"] == true).map(|w| w.revision),
             "repeat_count":self.window.as_ref().and_then(|w|w.args["angles"].as_array()).map(Vec::len),
             "retype": self.window.is_some() })
    }
    fn set(&mut self, key: &str, value: &Json) -> bool {
        match key {
            "start_point" => self.centre = json_p3(value),
            "ref_point" => self.reference = json_p3(value),
            "copy" => self.copy = value.as_bool().unwrap_or(false),
            "close_window" => self.window = None,
            _ => return false,
        }
        true
    }

    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        match ev {
            ToolEvent::Activate | ToolEvent::Deactivate | ToolEvent::Cancel => {
                if !matches!(ev, ToolEvent::Activate) {
                    host.call("park_rotation", Json::Null);
                }
                self.reset();
                self.window = None;
                self.ctrl_pressed = false;
                host.call("end_rotation", Json::Null);
            }
            ToolEvent::Key { key, .. } if key == "ToggleCopy" || key == "ToggleCopyQuiet" => {
                self.toggle_copy(host, key == "ToggleCopy");
            }
            ToolEvent::Key { key, .. } if key == "Control" => {
                self.toggle_copy(host, true);
                self.ctrl_pressed = true;
            }
            ToolEvent::KeyRelease { key, tapped } if key == "Control" && self.ctrl_pressed => {
                self.ctrl_pressed = false;
                if !tapped {
                    self.toggle_copy(host, false);
                }
            }
            ToolEvent::Click(p) => {
                self.window = None;
                let Some(point) = p.world else {
                    return Reply::ignored();
                };
                if let Some(centre) = self.centre {
                    if self.reference.is_none() {
                        if len(sub(point, centre)) >= 1e-6 {
                            self.reference = Some(point);
                        }
                    } else if let Some(angle) = host.call("rotation_angle", json!(point)).as_f64() {
                        return self.commit(angle, host);
                    }
                } else if host.call("pick_rotation", json!(point)).as_bool() == Some(true) {
                    self.centre = Some(point);
                }
            }
            ToolEvent::Hover(p) => {
                host.call("protractor_hover", Json::Null);
                if self.reference.is_some() && !self.copy {
                    if let Some(angle) = host.call("rotation_angle", json!(p.world)).as_f64() {
                        host.call("rotation_preview", json!(angle));
                    }
                }
            }
            ToolEvent::Value(Value::Length(v)) if v.is_finite() => {
                if self.reference.is_some() {
                    let current = host.call("swing_angle", Json::Null).as_f64().unwrap_or(0.);
                    return self.commit(if current < 0. { -v.abs() } else { v.abs() }, host);
                }
                return self.retype(Some(*v), None, host);
            }
            ToolEvent::Array { count, mode, step } if self.reference.is_none() => {
                return self.retype(None, Some((*count, mode, *step)), host)
            }
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
}
