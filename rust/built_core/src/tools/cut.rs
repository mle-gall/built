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
    targets: Vec<(String, Option<String>, Option<usize>)>,
    grip: Option<Grip>,
    hover: Json,
    point: Option<P3>,
    face: Json,
    existing_sections: Option<Vec<String>>,
    section: Option<String>,
    ring: Vec<P3>,
    anchor: Json,
    picking_anchor: bool,
    operation: usize,
    preset: Json,
    offset: f64,
}
#[derive(serde::Deserialize)]
struct Grip {
    row: Json,
    kind: String,
    at: glam::DVec3,
    axis: glam::DVec3,
    radius: f64,
}
impl Grip {
    fn field_path(&self) -> &'static str {
        if self.kind == "angle" {
            "/cutter/plane/angle"
        } else if self.row["anchor"].is_object() {
            "/anchor/offset"
        } else if self.row["cutter"]["bisector"].is_string() {
            "/clearance"
        } else {
            "/offset"
        }
    }
    fn current(&self) -> f64 {
        self.row
            .pointer(self.field_path())
            .and_then(Json::as_f64)
            .unwrap_or(0.)
    }
    fn commit(&self, host: &mut dyn ToolHost, value: f64) -> Reply {
        let mut row = self.row.clone();
        match self.field_path() {
            "/cutter/plane/angle" => row["cutter"]["plane"]["angle"] = json!(value),
            "/anchor/offset" => row["anchor"]["offset"] = json!(value),
            "/clearance" => row["clearance"] = json!(value),
            _ => row["offset"] = json!(value),
        }
        let answer = host.run(&Command {
            name: "live_set",
            args: json!({"row":row}),
        });
        if answer.ok {
            Reply::committed()
        } else {
            host.call("cut_error", json!(answer.error));
            Reply::redraw()
        }
    }
    fn dragged(&self, point: P3) -> f64 {
        let distance =
            (glam::DVec3::from(point) - self.at).dot(self.axis) / self.axis.length_squared();
        let delta = if self.kind == "angle" {
            distance.atan2(self.radius).to_degrees()
        } else {
            distance * 1000.
        };
        self.current() + delta
    }
}
impl Tool for CutTool {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn state(&self) -> Json {
        json!({"angle_grip": self.grip.as_ref().is_some_and(|g| g.kind=="angle"),"grip_value":self.grip.as_ref().map(|g| self.point.map_or_else(||g.current(),|p| g.dragged(p))),"editing_grip": self.grip.is_some(),"target":self.targets.first().map(|t| &t.0),"targets":self.targets.iter().map(|t| &t.0).collect::<Vec<_>>(), "cutter":self.hover,"hover_point":self.point,"chip":format!("{} · {}",self.preset["name"].as_str().unwrap_or("Cut"),OPS[self.operation]), "preset":self.preset["name"], "operation":OPS[self.operation], "offset":self.offset})
    }
    fn busy(&self) -> bool {
        self.grip.is_some() || !self.targets.is_empty()
    }
    fn set(&mut self, key: &str, value: &Json) -> bool {
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
        if let Some(grip) = &self.grip {
            let at = self.point.unwrap_or(grip.at.to_array());
            preview.value = Some(preview::ValueAnchor {
                at,
                label: if grip.kind == "angle" {
                    "Angle"
                } else {
                    "Offset"
                },
            });
            preview.lines.push(Preview::rubber(grip.at.to_array(), at));
        } else if self.ring.len() > 2 {
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
                self.targets = host
                    .call("cut_selection", Json::Null)
                    .as_str()
                    .map(|id| (id.to_owned(), None, None))
                    .into_iter()
                    .collect();
            }
            ToolEvent::Cancel | ToolEvent::Deactivate => {
                self.targets.clear();
                self.grip = None;
                self.hover = Json::Null;
                self.ring.clear();
                self.anchor = Json::Null;
                self.picking_anchor = false;
            }
            ToolEvent::Value(Value::Length(v)) if self.grip.is_some() => {
                let grip = self.grip.take().unwrap();
                return grip.commit(host, if grip.kind == "angle" { *v } else { v * 1000. });
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
                self.existing_sections =
                    serde_json::from_value(host.call("cut_sections", Json::Null)).ok();
                host.call("cut_draw_section", json!({"face":self.face}));
            }
            ToolEvent::Hover(p) | ToolEvent::Click(p) => {
                if let Some(previous) = self.existing_sections.take() {
                    let sections: Vec<String> =
                        serde_json::from_value(host.call("cut_sections", Json::Null))
                            .unwrap_or_default();
                    if let Some(id) = sections.into_iter().rev().find(|id| !previous.contains(id)) {
                        self.section = Some(id);
                        self.preset = json!({"name":"Opening","values":{"cutter_kind":"section"}});
                    }
                }
                self.point = p.world;
                if let Some(grip) = &mut self.grip {
                    if matches!(e, ToolEvent::Click(_)) {
                        if let Some(point) = p.world {
                            let v = grip.dragged(point);
                            let reply = grip.commit(host, v);
                            self.grip = None;
                            return reply;
                        }
                    }
                    return Reply::redraw();
                }
                let mut through = Json::Null;
                if matches!(e, ToolEvent::Click(_)) && !p.mods.shift && !self.picking_anchor {
                    through = host.call("cut_grip", json!(p.px));
                    self.grip = serde_json::from_value(through.clone()).ok();
                    if self.grip.is_some() {
                        self.ring.clear();
                        return Reply::redraw();
                    }
                }
                let pick = if through["placement"].is_string() {
                    through
                } else {
                    host.call("cut_pick", json!(p.px))
                };
                self.face = pick["face"].clone();
                self.ring = serde_json::from_value(pick["ring"].clone()).unwrap_or_default();
                let placed = &pick["placement"];
                let click = matches!(e, ToolEvent::Click(_));
                if self.targets.is_empty() || (p.mods.shift && !self.picking_anchor) {
                    if click {
                        if let Some(uid) = placed.as_str() {
                            if let Some(i) = self.targets.iter().position(|t| t.0 == uid) {
                                self.targets.remove(i);
                            } else {
                                self.targets.push((
                                    uid.to_owned(),
                                    pick["end"].as_str().map(str::to_owned),
                                    pick["span"].as_u64().map(|s| s as usize),
                                ));
                            }
                        }
                    }
                    return Reply::redraw();
                };
                if self.picking_anchor {
                    if click
                        && !self
                            .targets
                            .iter()
                            .any(|t| Some(t.0.as_str()) == placed.as_str())
                        && !self.face.is_null()
                    {
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
                    "section" => self
                        .section
                        .as_ref()
                        .map(|id| json!({"section":id}))
                        .unwrap_or(Json::Null),
                    "body" if pick["repeat"].is_string() => json!({"repeat":pick["repeat"]}),
                    _ if placed.is_string()
                        && !self
                            .targets
                            .iter()
                            .any(|t| Some(t.0.as_str()) == placed.as_str()) =>
                    {
                        json!({(kind):placed})
                    }
                    _ => Json::Null,
                };
                if click && !self.hover.is_null() {
                    let rows: Vec<_> = self.targets.iter().map(|(target,end,span)| json!({"kind":"cut","owner":target,"uid":crate::element_ids::new_uid(),"cutter":self.hover,"name":self.preset["name"],
                        "operation":OPS[self.operation],"anchor":anchor,"clearance":if matches!(kind,"body"|"bisector"){self.offset}else{0.},"on":true,"end":end,"span":span})).collect();
                    let answer = host.run(&Command {
                        name: "live_set",
                        args: json!({"row":rows}),
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
