// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use serde_json::{json, Value as Json};

const INFO: ToolInfo = ToolInfo::basic(
    "cut",
    "Cut",
    "",
    "Pick a target and its cutter.",
    false,
    Some("Offset"),
);
const OPS: [&str; 4] = ["subtract", "intersect", "union", "trim"];

#[derive(Default)]
pub struct CutTool {
    target: Option<String>,
    hover: Json,
    point: Option<P3>,
    face: Json,
    ring: Vec<P3>,
    end: Option<String>,
    anchor: Json,
    picking_anchor: bool,
    operation: usize,
    preset: Json,
    offset: f64,
}
impl Tool for CutTool {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn state(&self) -> Json {
        json!({"target":self.target, "cutter":self.hover,"hover_point":self.point,"chip":format!("{} · {}",self.preset["name"].as_str().unwrap_or("Cut"),OPS[self.operation]), "preset":self.preset["name"], "operation":OPS[self.operation], "offset":self.offset})
    }
    fn busy(&self) -> bool {
        self.target.is_some()
    }
    fn set(&mut self, key: &str, value: &Json) -> bool {
        if key == "section_target" {
            self.target = value.as_str().map(str::to_owned);
            self.preset = json!({"name":"Opening","values":{"cutter_kind":"section"}});
            return true;
        }
        if key != "preset" {
            return false;
        }
        self.preset = value.clone();
        self.operation = OPS
            .iter()
            .position(|op| value["values"]["operation"] == *op)
            .unwrap_or(0);
        self.offset = value["values"]["clearance"]
            .as_f64()
            .or(value["values"]["offset"].as_f64())
            .unwrap_or(0.);
        true
    }
    fn preview(&self) -> Preview {
        let mut preview = Preview::default();
        if self.ring.len() > 2 {
            preview.faces.push(self.ring.clone());
            for (a, b) in self.ring.iter().zip(self.ring.iter().cycle().skip(1)) {
                preview.lines.push(Preview::rubber(*a, *b));
            }
        }
        preview
    }
    fn event(&mut self, e: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        match e {
            ToolEvent::Activate => {
                self.target = host
                    .call("cut_selection", Json::Null)
                    .as_str()
                    .map(str::to_owned);
            }
            ToolEvent::Cancel | ToolEvent::Deactivate => {
                self.target = None;
                self.hover = Json::Null;
                self.ring.clear();
                self.anchor = Json::Null;
                self.picking_anchor = false;
            }
            ToolEvent::Value(Value::Length(v)) => self.offset = v * 1000.,
            ToolEvent::Key { key, .. } if key.eq_ignore_ascii_case("tab") => {
                self.operation = (self.operation + 1) % OPS.len()
            }
            ToolEvent::Key { key, .. } if key.eq_ignore_ascii_case("a") => {
                self.picking_anchor = true;
            }
            ToolEvent::Key { key, .. } if key.eq_ignore_ascii_case("l") => {
                host.call("cut_picker", Json::Null);
            }
            ToolEvent::Key { key, .. } if key.eq_ignore_ascii_case("d") => {
                host.call(
                    "tool_sketch",
                    json!({"tool":"cut","target":self.target,"face":self.face}),
                );
            }
            ToolEvent::Hover(p) | ToolEvent::Click(p) => {
                self.point = p.world;
                let pick = host.call("cut_pick", json!(p.px));
                self.face = pick["face"].clone();
                self.ring = serde_json::from_value(pick["ring"].clone()).unwrap_or_default();
                let placed = &pick["placement"];
                let click = matches!(e, ToolEvent::Click(_));
                let Some(target) = &self.target else {
                    if click {
                        self.target = placed.as_str().map(str::to_owned);
                        self.end = pick["end"].as_str().map(str::to_owned);
                    }
                    return Reply::redraw();
                };
                if self.picking_anchor {
                    if click && placed.as_str() != Some(target) && !self.face.is_null() {
                        self.anchor = self.face.clone();
                        self.picking_anchor = false;
                    }
                    return Reply::redraw();
                }
                let kind = self.preset["values"]["cutter_kind"]
                    .as_str()
                    .unwrap_or("body");
                let mut anchor = if self.anchor.is_null() {
                    Json::Null
                } else {
                    json!({"ref":self.anchor,"offset":self.offset})
                };
                self.hover = match kind {
                    "plane" => {
                        if self.face.is_null() && self.anchor.is_null() {
                            Json::Null
                        } else {
                            if self.anchor.is_null() {
                                anchor = json!({"ref":self.face,"offset":self.offset});
                            }
                            json!({"plane":{}})
                        }
                    }
                    "section" => host.call("cut_section", Json::Null),
                    _ if placed.is_string() && placed.as_str() != Some(target) => {
                        json!({(kind):placed})
                    }
                    _ => Json::Null,
                };
                if click && !self.hover.is_null() {
                    let uid = crate::element_ids::new_uid();
                    let row = json!({"kind":"cut","owner":target,"uid":uid,"cutter":self.hover,"name":self.preset["name"],
                        "operation":OPS[self.operation],"anchor":anchor,"clearance":if kind=="body"{self.offset}else{0.},"on":true,"end":self.end});
                    let answer = host.run(&Command {
                        name: "live_set",
                        args: json!({"row":row}),
                    });
                    if answer.ok {
                        return Reply::committed();
                    }
                    host.call("cut_error", json!(answer.error));
                }
            }
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
}
