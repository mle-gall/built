// SPDX-License-Identifier: GPL-3.0-or-later
//! The Junction tool (docs/junctions.md §11, rail, J): a detail in the
//! chip; hovering a meeting previews it (ghost) with its fit; a click
//! applies it; Alt-click applies it outside its rule as custom; L opens the
//! junction details, ↓ cycles those that fit the meeting under the cursor.
//! On a Repeat copy it asks once: every copy, this end, or this meeting.
//! A detail between parts (a bolt group) binds its plies by pick order.
use super::*;
use serde_json::{json, Value as Json};

const INFO: ToolInfo = ToolInfo::basic(
    "junction",
    "Junction",
    "",
    "Apply a junction detail at meetings.",
    false,
    None,
);

#[derive(Default)]
pub struct JunctionTool {
    detail: Json,
    hover: Json,
    plies: Vec<Json>,
    point: Option<P3>,
    message: Option<String>,
    /// A parts detail starts from the selected part (as the Cut tool
    /// starts from its selected target): checked once per detail.
    selection_read: bool,
    /// The selection when the tool armed (picking a detail may clear it).
    armed: Json,
}

impl JunctionTool {
    fn name(&self) -> &str {
        self.detail["name"].as_str().unwrap_or("No detail")
    }
    fn parts(&self) -> bool {
        self.detail["values"]["meeting"] == "parts"
    }
    fn uid(&self) -> &str {
        self.detail["uid"].as_str().unwrap_or("")
    }
    fn fit_words(&self) -> &str {
        if self.parts() {
            return if self.hover["placement"].is_string() { "click to pick" } else { "" };
        }
        match self.hover["fit"].as_str() {
            Some("match") => "fits",
            Some("near") => "outside",
            Some("applied") => "applied",
            Some("detailed") => "has another junction",
            Some("skipped") => "skipped before",
            _ if self.hover.is_null() => "",
            _ => "other meeting",
        }
    }
    fn prompt(&self) -> String {
        if let Some(m) = &self.message {
            return m.clone();
        }
        if self.detail.is_null() {
            return "Save a junction first, or press L for junction details".into();
        }
        if self.parts() {
            return if self.plies.is_empty() {
                "Click the first ply (the part the fasteners go through first), or select it first".into()
            } else {
                "Click the member it is fastened to".into()
            };
        }
        match self.hover["fit"].as_str() {
            Some("match") => "Click applies · Alt-click custom · ↓ next detail".into(),
            Some("near") => format!(
                "Outside the detail: {} · Alt-click applies it as custom",
                self.hover["near"]["criterion"].as_str().unwrap_or("rule")
            ),
            Some("applied") => "This meeting already has this detail".into(),
            Some("detailed") => "This meeting has another junction".into(),
            Some(_) => "Not this detail's kind of meeting · ↓ for one that fits".into(),
            None => "Hover a meeting of two members".into(),
        }
    }
    fn apply(&mut self, host: &mut dyn ToolHost, bindings: Json, force: bool, repeat: bool) -> Reply {
        let answer = host.call("junction_apply", json!({"detail": self.uid(), "bindings": bindings, "force": force, "repeat": repeat}));
        if answer["ok"] == true {
            self.message = answer["message"].as_str().map(str::to_owned);
            self.hover = Json::Null;
            return if answer["asked"] == true { Reply::redraw() } else { Reply::committed() };
        }
        self.message = Some(answer["error"].as_str().unwrap_or("Could not apply the detail").to_owned());
        Reply::redraw()
    }
}

impl Tool for JunctionTool {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }
    fn busy(&self) -> bool {
        !self.plies.is_empty()
    }
    fn state(&self) -> Json {
        let roles = if self.parts() {
            format!("ply {}", self.plies.len() + 1)
        } else {
            self.hover["label"].as_str().unwrap_or("").to_owned()
        };
        let mut chip = self.name().to_owned();
        for part in [roles.as_str(), self.fit_words()] {
            if !part.is_empty() {
                chip.push_str(" · ");
                chip.push_str(part);
            }
        }
        json!({"chip": chip, "prompt": self.prompt(), "detail": self.detail["uid"], "hover": self.hover,
               "hover_point": self.point, "fit": self.hover["fit"], "plies": self.plies.len()})
    }
    fn set(&mut self, key: &str, value: &Json) -> bool {
        match key {
            "preset" if value["primitive"] == "junction" => {
                self.detail = value.clone();
                self.plies.clear();
                self.selection_read = false;
                self.hover = Json::Null;
                self.message = None;
                true
            }
            _ => false,
        }
    }
    fn preview(&self) -> Preview {
        let mut out = Preview::default();
        for line in self.hover["ghost"].as_array().into_iter().flatten() {
            if let (Some(a), Some(b)) = (super::json_p3(&line[0]), super::json_p3(&line[1])) {
                out.lines.push(Preview::rubber(a, b));
            }
        }
        out
    }
    fn event(&mut self, e: &ToolEvent, host: &mut dyn ToolHost) -> Reply {
        match e {
            ToolEvent::Activate => {
                self.armed = host.call("junction_selection", Json::Null);
                let d = host.call("junction_default", Json::Null);
                if !d.is_null() {
                    self.detail = d;
                }
            }
            ToolEvent::Cancel | ToolEvent::Deactivate => {
                self.plies.clear();
                self.selection_read = matches!(e, ToolEvent::Cancel);
                self.hover = Json::Null;
                self.message = None;
            }
            ToolEvent::Key { key, .. } if key.eq_ignore_ascii_case("l") => {
                host.call("junction_picker", Json::Null);
            }
            ToolEvent::Key { key, .. } if matches!(key.to_lowercase().as_str(), "down" | "arrowdown" | "key_down") => {
                let next = host.call("junction_cycle", json!({"key": self.hover["key"], "detail": self.uid()}));
                if next.is_object() {
                    self.detail = next;
                    self.hover = Json::Null;
                    self.message = None;
                } else {
                    self.message = Some("No other detail fits this meeting".into());
                }
            }
            ToolEvent::Hover(p) => {
                self.message = None;
                self.point = p.world;
                if self.detail.is_null() {
                    return Reply::redraw();
                }
                if self.parts() && self.plies.is_empty() && !self.selection_read {
                    self.selection_read = true;
                    let mut picked = host.call("junction_selection", Json::Null);
                    if !picked["placement"].is_string() {
                        picked = std::mem::take(&mut self.armed);
                    }
                    if picked["placement"].is_string() {
                        self.plies.push(json!({"placement": picked["placement"]}));
                    }
                }
                self.hover = if self.parts() {
                    host.call("junction_ply", json!({"px": p.px}))
                } else {
                    host.call("junction_hover", json!({"px": p.px, "world": p.world, "detail": self.uid()}))
                };
            }
            ToolEvent::Click(p) => {
                self.message = None;
                if self.detail.is_null() {
                    host.call("junction_picker", Json::Null);
                    return Reply::redraw();
                }
                if self.parts() {
                    let ply = host.call("junction_ply", json!({"px": p.px}));
                    if !ply["placement"].is_string() {
                        self.message = Some("Click a part or a member".into());
                        return Reply::redraw();
                    }
                    if self.plies.iter().any(|q| q["placement"] == ply["placement"]) {
                        return Reply::redraw();
                    }
                    let mut binding = json!({"placement": ply["placement"]});
                    if let Some(face) = ply["face"].as_str() {
                        binding["face"] = json!(face);
                    }
                    self.plies.push(binding);
                    if self.plies.len() == 2 {
                        let bindings = json!({"ply_1": self.plies[0], "ply_2": self.plies[1]});
                        self.plies.clear();
                        return self.apply(host, bindings, false, false);
                    }
                    return Reply::redraw();
                }
                let hover = host.call("junction_hover", json!({"px": p.px, "world": p.world, "detail": self.uid()}));
                self.hover = hover.clone();
                let alt = p.mods.alt;
                match hover["fit"].as_str() {
                    Some("match" | "skipped") => return self.apply(host, hover["roles"].clone(), false, hover["repeat"] == true),
                    Some("near") | Some("other") if alt => return self.apply(host, hover["roles"].clone(), true, false),
                    _ => {}
                }
                self.message = Some(self.prompt());
            }
            _ => return Reply::ignored(),
        }
        Reply::redraw()
    }
}
