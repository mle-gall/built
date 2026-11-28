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
    outline: Vec<[P3; 2]>,
    refusal: Option<String>,
    fitting: Json,
    pointer: Option<Pointer>,
    typed_offset: bool,
    flush_clearance: Option<f64>,
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
    fn project(&self, host: &mut dyn ToolHost, pointer: &Pointer) -> Option<P3> {
        let point = host.call(
            "cut_drag",
            json!({"at":self.at, "axis":self.axis, "px":pointer.px}),
        );
        serde_json::from_value(point).ok().or(pointer.world)
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
impl CutTool {
    fn prompt(&self) -> &str {
        if let Some(reason) = &self.refusal {
            return reason;
        }
        if let Some(prompt) = self.fitting["prompt"].as_str() {
            return prompt;
        }
        if self.grip.is_some() {
            return "Drag or enter a value";
        }
        if self.targets.is_empty() {
            return "Pick the solid to cut";
        }
        if self.picking_anchor {
            return "Pick an anchor face";
        }
        match self.preset["values"]["cutter_kind"].as_str() {
            Some("plane") => "Pick the cutting face",
            Some("bisector") => "Pick the mitre partner",
            Some("section") => "Pick the cutting section",
            _ => "Pick the cutting solid",
        }
    }
}
impl Tool for CutTool {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn state(&self) -> Json {
        json!({"cut_sizes":self.fitting["sizes"],"prompt":self.prompt(),"angle_grip": self.grip.as_ref().is_some_and(|g| g.kind=="angle"),"grip_value":self.grip.as_ref().map(|g| self.point.map_or_else(||g.current(),|p| g.dragged(p))),"editing_grip": self.grip.is_some(),"target":self.targets.first().map(|t| &t.0),"targets":self.targets.iter().map(|t| &t.0).collect::<Vec<_>>(), "cutter":self.hover,"hover_point":self.point,"chip":format!("{} · {}",self.preset["name"].as_str().unwrap_or("Cut"),OPS[self.operation]), "preset":self.preset["name"], "operation":OPS[self.operation], "offset":self.fitting["offset"].as_f64().unwrap_or(self.offset)})
    }
    fn busy(&self) -> bool {
        self.grip.is_some() || !self.targets.is_empty()
    }
    fn set(&mut self, key: &str, value: &Json) -> bool {
        if key != "preset" {
            return false;
        }
        self.fitting = Json::Null;
        self.typed_offset = false;
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
            .lines
            .extend(self.outline.iter().map(|[a, b]| Preview::rubber(*a, *b)));
        preview
    }
    fn event(&mut self, e: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        match e {
            ToolEvent::Activate => {
                self.flush_clearance = host.call("cut_shortcut", json!("c"))["values"]["clearance"].as_f64();
                self.targets = host
                    .call("cut_selection", Json::Null)
                    .as_str()
                    .map(|id| (id.to_owned(), None, None))
                    .into_iter()
                    .collect();
            }
            ToolEvent::Cancel | ToolEvent::Deactivate => {
                self.pointer = None;
                self.typed_offset = false;
                if self.preset.is_null() {
                    self.offset = 0.;
                }
                self.fitting = Json::Null;
                self.outline.clear();
                self.refusal = None;
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
            ToolEvent::Value(Value::Length(v)) => {
                self.typed_offset = true;
                self.offset = v * 1000.;
                if let Some(p) = self.pointer.clone() {
                    return self.event(&ToolEvent::Hover(p), host);
                }
            }
            ToolEvent::Key { key, .. }
                if matches!(key.to_ascii_lowercase().as_str(), "c" | "m" | "p" | "b") =>
            {
                let preset = host.call("cut_shortcut", json!(key.to_ascii_lowercase()));
                if !preset.is_null() {
                    self.set("preset", &preset);
                    if let Some(p) = self.pointer.clone() {
                        return self.event(&ToolEvent::Hover(p), host);
                    }
                }
            }
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
                self.pointer = Some(p.clone());
                if let Some(previous) = self.existing_sections.take() {
                    let sections: Vec<String> =
                        serde_json::from_value(host.call("cut_sections", Json::Null))
                            .unwrap_or_default();
                    if let Some(id) = sections.into_iter().rev().find(|id| !previous.contains(id)) {
                        self.section = Some(id);
                        self.preset = json!({"name":"Opening","values":{"cutter_kind":"section"}});
                    }
                }
                self.refusal = None;
                self.fitting = Json::Null;
                self.outline.clear();
                self.point = p.world;
                if let Some(grip) = &mut self.grip {
                    self.point = grip.project(host, p);
                    if matches!(e, ToolEvent::Click(_)) {
                        if let Some(point) = self.point {
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
                    if let Some(grip) = &self.grip {
                        self.point = Some(grip.at.to_array());
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
                let picking_target =
                    self.targets.is_empty() || (p.mods.shift && !self.picking_anchor);
                let picking_face = !picking_target
                    && (self.picking_anchor || self.preset["values"]["cutter_kind"] == "plane");
                self.ring.clear();
                if picking_face {
                    self.ring = serde_json::from_value(pick["ring"].clone()).unwrap_or_default();
                } else {
                    self.outline =
                        serde_json::from_value(pick["outline"].clone()).unwrap_or_default();
                }
                let placed = &pick["placement"];
                let click = matches!(e, ToolEvent::Click(_));
                self.refusal = pick["reason"].as_str().map(str::to_owned);
                if self.refusal.is_none()
                    && !picking_face
                    && pick["open_edges"].as_u64().unwrap_or(0) > 0
                {
                    self.refusal =
                        Some(format!("{} open edges · Health › Fix", pick["open_edges"]));
                }
                if self.refusal.is_none()
                    && click
                    && !picking_face
                    && placed.is_string()
                    && pick["face"]["path"].is_array()
                    && host.call("cut_solid", pick["face"]["path"].clone()) == false
                {
                    self.refusal = Some("Select a closed solid · Health › Fix".into());
                }
                if self.refusal.is_some() {
                    self.hover = Json::Null;
                    if click {
                        host.call("cut_error", json!(self.prompt()));
                    }
                    return Reply::redraw();
                }
                if picking_target {
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
                let mut kind = self.preset["values"]["cutter_kind"]
                    .as_str()
                    .unwrap_or("body");
                let mut anchor = if self.anchor.is_null() {
                    Json::Null
                } else {
                    json!({"ref":self.anchor,"offset":self.offset})
                };
                let automatic = self.preset.is_null() && self.operation == 0;
                if self.targets.len() == 1
                    && self.anchor.is_null()
                    && !self.face.is_null()
                    && (automatic || kind == "plane")
                    && placed.as_str() != Some(&self.targets[0].0)
                {
                    let (target, _, span) = &self.targets[0];
                    self.fitting = host.call(
                        "cut_end_fit",
                        json!({"target":target, "span":span,
                        "body":placed,"face":self.face,"explicit":!automatic,"offset":self.offset,"typed":self.typed_offset,"flush_clearance":self.flush_clearance}),
                    );
                    if let Some(reason) = self.fitting["reason"].as_str() {
                        self.refusal = Some(reason.to_owned());
                        self.hover = Json::Null;
                        if click {
                            host.call("cut_error", json!(reason));
                        }
                        return Reply::redraw();
                    }
                    if let Some(face) = self.fitting.get("face") {
                        kind = "plane";
                        anchor = json!({"ref":face,"offset":self.fitting["offset"]});
                        if let Ok(lines) =
                            serde_json::from_value::<Vec<[P3; 2]>>(self.fitting["lines"].clone())
                        {
                            self.outline.extend(lines);
                        }
                    }
                }
                if kind == "bisector"
                    && self.targets.len() == 1
                    && self.anchor.is_null()
                    && placed.is_string()
                    && placed.as_str() != Some(&self.targets[0].0)
                {
                    self.fitting = host.call(
                        "cut_corner",
                        json!({"target":self.targets[0].0,"body":placed}),
                    );
                }
                if kind == "body" && self.operation == 0 && self.targets.len() == 1 && placed.is_string()
                    && placed.as_str() != Some(&self.targets[0].0) && self.anchor.is_null() && self.fitting.is_null() {
                    self.fitting = host.call("cut_body_preview", json!({"target":self.targets[0].0,"span":self.targets[0].2,"body":placed,"offset":self.offset}));
                    if let Ok(lines) = serde_json::from_value::<Vec<[P3;2]>>(self.fitting["lines"].clone()) {
                        if !lines.is_empty() { self.outline = lines; }
                    }
                }
                self.hover = match kind {
                    "plane" => {
                        if self.face.is_null() && self.anchor.is_null() {
                            Json::Null
                        } else {
                            if anchor.is_null() {
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
                if click && self.hover.is_null() {
                    self.refusal = Some(
                        if placed.is_string() {
                            "Pick a different cutting object"
                        } else {
                            self.prompt()
                        }
                        .to_owned(),
                    );
                    host.call("cut_error", json!(self.prompt()));
                }
                if click && !self.hover.is_null() {
                    let inside = host.call("cut_context", Json::Null);
                    let fitted = self.fitting["face"].is_object();
                    let mut rows: Vec<_> = self.targets.iter().map(|(target,end,span)| {
                        json!({"kind":"cut","owner":target,"uid":crate::element_ids::new_uid(),
                            "cutter":self.hover,"name":if fitted && automatic {json!("Fit end")} else {self.preset["name"].clone()},
                            "operation":if fitted {"trim"} else {OPS[self.operation]}, "anchor":anchor,
                            "clearance":if matches!(kind,"body"|"bisector"){self.offset}else{0.},"on":true,
                            "end":if fitted {self.fitting["end"].clone()} else {json!(end)},
                            "span":if fitted {self.fitting["span"].clone()} else {json!(span)}})
                    }).collect();
                    if self.fitting["partner_row"].is_string() {
                        let cut = &mut rows[0];
                        cut["end"] = self.fitting["end"].clone();
                        cut["inputs"] = json!([{"sweep":self.fitting["partner_row"],"at":self.fitting["partner_end"]}]);
                        let mut partner = cut.clone();
                        partner["uid"] = json!(crate::element_ids::new_uid());
                        partner["owner"] = placed.clone();
                        partner["cutter"] = json!({"bisector":self.targets[0].0});
                        partner["end"] = self.fitting["partner_end"].clone();
                        partner["inputs"] =
                            json!([{"sweep":self.fitting["target_row"],"at":self.fitting["end"]}]);
                        rows.push(partner);
                    }
                    // Inside a junction (edit context), its cuts on its own
                    // members and parts are this use's; a cut on anything else
                    // stays an ordinary cut on that member.
                    let mine = |row: &Json| inside["members"].as_array().is_none_or(|m| m.iter().any(|u| *u == row["owner"]));
                    if inside["junction"].is_string() && !rows.iter().all(&mine) {
                        host.call("cut_error", json!("Not a member of this junction: the cut stays on its own"));
                    }
                    if let Some(junction) = inside["junction"].as_str().filter(|_| rows.iter().all(&mine)) {
                        for (i, row) in rows.iter_mut().enumerate() {
                            row["junction"] = json!(junction);
                            row["key"] = json!(format!("{}_{i}", inside["key"].as_str().unwrap_or("cut")));
                            row["uid"] = json!("");
                        }
                    }
                    let answer = host.run(&Command {
                        name: "live_set",
                        args: json!({"row":rows}),
                    });
                    if answer.ok {
                        let ids: Vec<_> = rows.iter().map(|row| &row["uid"]).collect();
                        if let Some(note) = host.call("cut_feedback", json!(ids)).as_str() {
                            self.refusal = Some(note.to_owned());
                            host.call("cut_error", json!(note));
                        } else if fitted && automatic {
                            let message = if self.fitting["flush"] == true {
                                "End already flush · live fit saved"
                            } else {
                                "Live end fit saved"
                            };
                            self.fitting = Json::Null;
                            host.call("cut_error", json!(message));
                        } else {
                            self.fitting = Json::Null;
                            host.call("cut_error", json!("Live cut saved · pick another cutter"));
                        }
                        return Reply::committed();
                    }
                    self.refusal = answer.error.clone();
                    host.call("cut_error", json!(answer.error));
                }
            }
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
}
