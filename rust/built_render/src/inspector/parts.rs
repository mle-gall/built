// SPDX-License-Identifier: GPL-3.0-or-later
use super::{kit, label, send, tokens, Ui, Value};
use serde_json::json;

#[derive(Default)]
pub(super) struct Parts {
    target: String,
    rename: Option<(String, String)>,
    focus_name: bool,
    selected: Option<String>,
    anchor: Option<String>,
}

impl Parts {
    pub fn draw(&mut self, ui: &mut Ui, part: &Value, measures: &Value, out: &mut Vec<String>) {
        let parent = label(&part["uid"]);
        if self.target != parent {
            *self = Self {
                target: parent.to_owned(),
                ..Default::default()
            };
        }
        let Some(kids) = part["children"].as_array() else {
            return;
        };
        if kids.is_empty() {
            return;
        }
        let stamp = json!([parent, part["revision"]]);
        if stamp != measures["target"] && ui.clip_rect().intersects(ui.available_rect_before_wrap())
        {
            send(out, "parts.measure", stamp.clone());
        }
        let can_edit = part["editable"] == true;
        let hidden = kids
            .iter()
            .filter(|c| c["editable"] == true && c["hidden"] == true)
            .count();
        let all_action = if hidden == 0 {
            "Hide all parts"
        } else {
            "Show all parts"
        };
        if can_edit && kit::quiet(ui, all_action).clicked() {
            send(out, "parts.visible", json!([parent, null, hidden > 0]));
        }
        let selected = kids
            .iter()
            .find(|c| c["selected"] == true)
            .map(|c| label(&c["uid"]).to_owned());
        let reveal = selected != self.selected;
        self.selected = selected;
        egui::ScrollArea::vertical()
            .id_salt(("parts", parent))
            .max_height(tokens::ROW_H * 14.)
            .show(ui, |ui| {
                for (i, c) in kids.iter().enumerate() {
                    let uid = label(&c["uid"]);
                    let name = label(&c["name"]);
                    let editable = can_edit && c["editable"] == true;
                    ui.push_id(uid, |ui| {
                        ui.horizontal(|ui| {
                            let mut visible = c["hidden"] != true;
                            ui.add_enabled_ui(editable, |ui| {
                                if kit::toggle(ui, &mut visible, "")
                                    .on_hover_text("Show part")
                                    .changed()
                                {
                                    send(out, "parts.visible", json!([parent, uid, visible]));
                                }
                            });
                            if let Some((_, draft)) =
                                self.rename.as_mut().filter(|(id, _)| id == uid && editable)
                            {
                                let r = ui.add(kit::field(draft));
                                if self.focus_name {
                                    r.request_focus();
                                    self.focus_name = false;
                                }
                                if r.lost_focus() {
                                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                        send(
                                            out,
                                            "parts.rename",
                                            json!([parent, uid, draft.trim()]),
                                        );
                                    }
                                    self.rename = None;
                                }
                            } else {
                                let selected = c["selected"] == true;
                                let response = ui.selectable_label(selected, name);
                                if reveal && selected {
                                    response.scroll_to_me(None);
                                }
                                if visible && response.clicked() {
                                    let modifiers = ui.input(|i| i.modifiers);
                                    let anchor = self.anchor.as_ref().and_then(|key| {
                                        kids.iter().position(|k| label(&k["uid"]) == key)
                                    });
                                    if modifiers.shift || modifiers.command || modifiers.ctrl {
                                        let mut picks = Vec::new();
                                        for (at, k) in kids.iter().enumerate() {
                                            let in_range = if modifiers.shift {
                                                let from = anchor.unwrap_or(i);
                                                (from.min(i)..=from.max(i)).contains(&at)
                                            } else {
                                                false
                                            };
                                            let picked = if modifiers.shift {
                                                in_range
                                                    || ((modifiers.command || modifiers.ctrl)
                                                        && k["selected"] == true)
                                            } else {
                                                (k["selected"] == true) ^ (at == i)
                                            };
                                            if picked && k["hidden"] != true {
                                                picks.push(label(&k["uid"]));
                                            }
                                        }
                                        self.selected = picks.first().map(|id| (*id).to_owned());
                                        send(out, "parts.select_many", json!([parent, picks]));
                                    } else {
                                        self.selected = Some(uid.to_owned());
                                        send(out, "parts.select", json!([parent, uid]));
                                    }
                                    if !modifiers.shift {
                                        self.anchor = Some(uid.to_owned());
                                    }
                                }
                                if editable && kit::quiet(ui, "Rename…").clicked() {
                                    self.rename = Some((uid.to_owned(), name.to_owned()));
                                    self.focus_name = true;
                                }
                            }
                        });
                        if measures["target"] == stamp {
                            if let Some(m) = measures["rows"].get(uid) {
                                let size = label(&m[0]);
                                let material = label(&m[1]);
                                kit::caption_plain(
                                    ui,
                                    &if material.is_empty() {
                                        size.to_owned()
                                    } else {
                                        format!("{size} · {material}")
                                    },
                                );
                            }
                        }
                    });
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::containers::scroll_area::State;
    use egui::{vec2, Event, Pos2, Rect};
    use std::collections::HashMap;

    struct Inspector {
        ctx: egui::Context,
        panel: Parts,
        data: Value,
        scroll: egui::Id,
        labels: HashMap<String, Pos2>,
        time: f64,
    }

    impl Inspector {
        fn new() -> Self {
            let kids: Vec<_> = (0..30)
                .map(|i| json!({"uid":format!("p{i}"),"name":format!("Piece {i}"),"editable":true}))
                .collect();
            let mut this = Self {
                ctx: Default::default(),
                panel: Default::default(),
                data: json!({"uid":"root","children":kids,"revision":1,"editable":true}),
                labels: Default::default(),
                scroll: egui::Id::NULL,
                time: 0.,
            };
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "Inter".into(),
                egui::FontData::from_static(include_bytes!(
                    "../../../../resources/fonts/inter/Inter-Regular.ttf"
                ))
                .into(),
            );
            fonts
                .families
                .insert(egui::FontFamily::Proportional, vec!["Inter".into()]);
            this.ctx.set_fonts(fonts);
            this.frame(Vec::new());
            this.frame(Vec::new());
            this
        }

        fn frame(&mut self, events: Vec<Event>) -> Vec<String> {
            let mut out = Vec::new();
            self.time += 0.1;
            let raw = egui::RawInput {
                time: Some(self.time),
                events,
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(450., 650.))),
                ..Default::default()
            };
            let frame = self.ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    self.scroll = ui.make_persistent_id(egui::Id::new(("parts", "root")));
                    self.panel.draw(ui, &self.data, &Value::Null, &mut out);
                });
            });
            self.labels.clear();
            for shape in frame.shapes {
                if let egui::Shape::Text(t) = shape.shape {
                    let center = t.visual_bounding_rect().center();
                    if shape.clip_rect.contains(center) {
                        self.labels.insert(t.galley.text().to_owned(), center);
                    }
                }
            }
            out
        }

        fn click(&mut self, pos: Pos2) -> Vec<String> {
            let mut actions = Vec::new();
            for pressed in [true, false] {
                actions.extend(self.frame(vec![
                    Event::PointerMoved(pos),
                    Event::PointerButton {
                        pos,
                        pressed,
                        button: egui::PointerButton::Primary,
                        modifiers: Default::default(),
                    },
                ]));
            }
            actions
        }
    }

    #[test]
    fn test_a_part_selected_in_the_canvas_scrolls_into_view() {
        let mut ui = Inspector::new();
        assert!(!ui.labels.contains_key("Piece 29"));
        ui.data["children"][29]["selected"] = json!(true);
        for _ in 0..12 {
            ui.frame(vec![]);
        }
        assert!(ui.labels.contains_key("Piece 29"));
    }

    #[test]
    fn test_the_list_updates_in_place_and_keeps_its_scroll() {
        let mut ui = Inspector::new();
        let mut state = State::load(&ui.ctx, ui.scroll).unwrap();
        state.offset.y = 900.;
        state.store(&ui.ctx, ui.scroll);
        for _ in 0..3 {
            ui.frame(vec![]);
        }
        let before = State::load(&ui.ctx, ui.scroll).unwrap().offset.y;
        assert!(before > 0.);
        let actions = ui.click(ui.labels["Piece 28"]);
        assert!(actions
            .iter()
            .any(|a| a.contains("parts.select") && a.contains("p28")));
        ui.data["children"][28]["selected"] = json!(true);
        for _ in 0..4 {
            ui.frame(vec![]);
        }
        assert_eq!(State::load(&ui.ctx, ui.scroll).unwrap().offset.y, before);
    }

    #[test]
    fn test_a_rename_survives_a_refresh_while_typing() {
        let mut ui = Inspector::new();
        let pos = ui.labels["Rename…"];
        ui.click(pos);
        ui.frame(vec![]);
        let id = ui.panel.rename.as_ref().unwrap().0.clone();
        ui.frame(vec![Event::Text(" edited".into())]);
        ui.data["revision"] = json!(2);
        ui.frame(vec![]);
        assert!(ui.panel.rename.as_ref().unwrap().1.contains("edited"));
        let out = ui.frame(vec![Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        assert!(out
            .iter()
            .any(|a| a.contains("parts.rename") && a.contains(&id) && a.contains("edited")));
    }
}
