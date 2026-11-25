// SPDX-License-Identifier: GPL-3.0-or-later
//! Push/Pull gestures over the host's picked face, staged mesh and undo record.
use super::{Reply, Tool, ToolEvent, ToolHost, ToolInfo, Value};
use serde_json::{json, Value as Json};
const MIN_DISTANCE: f64 = 2e-4;
static INFO: ToolInfo = ToolInfo {
    kind: "pushpull",
    name: "Push / Pull",
    shortcut: "P",
    description: "Push or pull a face to extrude it into a solid or cut it back.",
    uses_snap: false,
    box_select: false,
    vcb_label: Some("Distance"),
    magnetic_axis_deg: None,
    screen_axis_px: None,
    axis_source: false,
    axis_source_empty_only: false,
};

#[derive(Default)]
pub struct PushPull {
    active: bool,
    press: Option<[f64; 2]>,
    last: Option<[f64; 2]>,
    tiny: bool,
    ctrl_used: bool,
}

impl PushPull {
    fn modifier(&mut self, host: &mut dyn ToolHost, mode: bool, held: bool, latch: bool) {
        if latch {
            self.ctrl_used = held;
        }
        host.call("modifier", json!([mode, mode || held]));
    }
    fn commit(&mut self, host: &mut dyn ToolHost, distance: Option<f64>) -> Reply {
        host.call("commit", json!(distance));
        self.active = false;
        self.press = None;
        self.last = None;
        self.tiny = false;
        Reply::committed()
    }
    fn click_commit(&mut self, host: &mut dyn ToolHost, distance: f64) -> Reply {
        if distance.abs() >= MIN_DISTANCE {
            return self.commit(host, None);
        }
        if !self.tiny {
            host.call("tiny_push", Json::Null);
            self.tiny = true;
        }
        Reply::redraw()
    }
    fn begin(&mut self, host: &mut dyn ToolHost, px: [f64; 2], pick: bool) -> bool {
        self.active = host.call("begin", json!(pick)).as_bool() == Some(true);
        self.press = (!pick && self.active).then_some(px);
        self.last = self.press;
        self.tiny = false;
        self.active
    }
}
impl Tool for PushPull {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn busy(&self) -> bool {
        self.active
    }
    fn state(&self) -> Json {
        json!({"dragging":self.active})
    }
    fn event(&mut self, ev: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        // The protocol and replay can also arm an operation directly. Read its
        // distance and live history window before deciding what this event does.
        let reading = host.call("reading", Json::Null);
        self.active = reading["dragging"].as_bool().unwrap_or(self.active);
        let mode = reading["mode"].as_bool() == Some(true);
        let d = reading["distance"].as_f64().unwrap_or(0.);
        match ev {
            ToolEvent::Activate | ToolEvent::Deactivate | ToolEvent::Cancel | ToolEvent::Undo => {
                if matches!(ev, ToolEvent::Undo) && !self.active {
                    return Reply::ignored();
                }
                let action = match ev {
                    ToolEvent::Activate => "activate",
                    ToolEvent::Deactivate => "deactivate",
                    _ => "cancel",
                };
                host.call(action, Json::Null);
                let ctrl = self.ctrl_used && !matches!(ev, ToolEvent::Activate);
                *self = Self {
                    ctrl_used: ctrl,
                    ..Default::default()
                };
                Reply::redraw()
            }
            ToolEvent::Hover(p) | ToolEvent::Drag(p) => {
                if self.active {
                    self.last = Some(p.px);
                    self.modifier(host, mode, p.mods.ctrl, false);
                    self.active = host.call("preview", Json::Null).as_bool() == Some(true);
                } else {
                    host.call("hover_face", Json::Null);
                }
                Reply::redraw()
            }
            ToolEvent::Click(p) | ToolEvent::Press(p) | ToolEvent::DoubleClick(p) => {
                let repeat = if matches!(ev, ToolEvent::DoubleClick(_)) {
                    reading["repeat"].as_f64()
                } else {
                    None
                };
                if repeat.is_none() {
                    host.call("close_retype", Json::Null);
                }
                self.modifier(host, mode, p.mods.ctrl, true);
                if let Some(last) = repeat {
                    if !self.active {
                        if !self.begin(host, p.px, true) {
                            return Reply::ignored();
                        }
                    } else if d.abs() > MIN_DISTANCE {
                        return self.commit(host, None);
                    }
                    self.commit(host, Some(last))
                } else if self.active {
                    self.click_commit(host, d)
                } else {
                    self.begin(host, p.px, false);
                    Reply::redraw()
                }
            }
            ToolEvent::Release(p) => {
                if !self.active
                    || !self
                        .press
                        .zip(self.last)
                        .is_some_and(|(a, b)| (b[0] - a[0]).hypot(b[1] - a[1]) >= 6.)
                {
                    return Reply::ignored();
                }
                self.modifier(host, mode, p.mods.ctrl, true);
                self.click_commit(host, d)
            }
            ToolEvent::Key { key, .. } if key == "Control" => Reply::redraw(),
            ToolEvent::KeyRelease { key, tapped } if key == "Control" => {
                if self.ctrl_used {
                    self.ctrl_used = false;
                    return Reply::redraw();
                }
                if !tapped {
                    return Reply::ignored();
                }
                host.call("modifier", json!([!mode, !mode]));
                host.call("show_mode", json!(!mode));
                Reply::redraw()
            }
            ToolEvent::Value(Value::Length(value)) if *value != 0. => {
                if self.active && reading["has_base"] == true {
                    self.commit(host, Some(if d < 0. { -*value } else { *value }))
                } else if let Some(sign) = host.call("restore_retype", Json::Null).as_f64() {
                    self.commit(host, Some(sign * value))
                } else {
                    Reply::ignored()
                }
            }
            _ => Reply::ignored(),
        }
    }
}
