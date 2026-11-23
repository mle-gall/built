//! Native inspector and tool controls. Input snapshots belong to one document.
mod records;
mod rule;
mod terrain;

use crate::ui::{kit, tokens};
use egui::{DragValue, Ui};
use serde_json::{json, Value};
pub use terrain::{plot_png, Terrain};

#[derive(Default)]
pub struct Panels {
    pub data: Value,
    pub popup_layers: Vec<egui::Id>,
    records: records::Records,
    rule: rule::Rule,
    pub terrain: Terrain,
}

fn send(out: &mut Vec<String>, key: &str, value: Value) {
    out.push(format!("panel:{}", json!([key, value])));
}

fn label(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

fn choice(
    ui: &mut Ui,
    key: &str,
    current: &str,
    choices: &[&str],
    out: &mut Vec<String>,
    layers: &mut Vec<egui::Id>,
) {
    egui::ComboBox::from_id_salt(key)
        .width(72.)
        .selected_text(current)
        .show_ui(ui, |ui| {
            layers.push(ui.layer_id().id);
            for &v in choices {
                if ui.selectable_label(v == current, v).clicked() {
                    send(out, key, json!(v));
                }
            }
        });
}

fn number(
    ui: &mut Ui,
    key: &str,
    name: &str,
    val: &Value,
    range: std::ops::RangeInclusive<f64>,
    out: &mut Vec<String>,
) {
    ui.horizontal(|ui| {
        ui.label(name);
        let mut n = val.as_f64().unwrap_or_default();
        if ui
            .add(kit::number(&mut n, "").range(range).speed(0.05))
            .changed()
        {
            send(out, key, json!(n));
        }
    });
}

impl Panels {
    pub fn set(&mut self, key: &str, data: Value) {
        if key == "selection" && data != self.data[key] {
            self.rule.set(&data["rule"]);
            self.data["rulePreview"] = Value::Null;
            self.records = records::Records::new(
                &data["records"],
                data["selection_count"].as_u64().unwrap_or_default() as usize,
            );
        }
        if key == "terrain" {
            self.terrain.load(&data);
        }
        self.data[key] = data;
    }

    pub fn inspector(&mut self, ui: &mut Ui, out: &mut Vec<String>) {
        let s = &self.data["selection"];
        if s["path"] == true && ui.button("Terrain profile…").clicked() {
            send(out, "terrain.open", Value::Null);
        }
        self.rule.draw(ui, &self.data["rulePreview"], out);
        let p = &s["parts"];
        if p.is_object() {
            ui.collapsing(format!("Parts · {}", label(&p["name"])), |ui| {
                if ui.button("Cut list…").clicked() {
                    send(out, "parts.report", p["uid"].clone());
                }
                ui.add_enabled_ui(p["editable"] == true, |ui| {
                    for child in p["children"].as_array().into_iter().flatten() {
                        ui.push_id(label(&child["uid"]), |ui| {
                            ui.horizontal(|ui| {
                                let mut visible = child["hidden"] != true;
                                if ui
                                    .checkbox(&mut visible, "")
                                    .on_hover_text("Show part")
                                    .changed()
                                {
                                    send(
                                        out,
                                        "parts.visible",
                                        json!([p["uid"], child["uid"], visible]),
                                    );
                                }
                                if ui
                                    .selectable_label(
                                        child["selected"] == true,
                                        label(&child["name"]),
                                    )
                                    .clicked()
                                    && visible
                                {
                                    send(out, "parts.select", json!([p["uid"], child["uid"]]));
                                }
                            })
                        });
                    }
                    if ui.button("Split into pieces").clicked() {
                        send(out, "parts.split", p["uid"].clone());
                    }
                    if p["children"].as_array().is_some_and(|v| v.len() >= 2) {
                        let id = ui.make_persistent_id(("explode", &p["uid"].to_string()));
                        let current = label(&p["mode"]);
                        let mut mode = ui
                            .data(|d| d.get_temp::<String>(id))
                            .unwrap_or(current.to_owned());
                        egui::ComboBox::from_id_salt(id)
                            .selected_text(&mode)
                            .show_ui(ui, |ui| {
                                self.popup_layers.push(ui.layer_id().id);
                                for m in ["outward", "x", "y", "z"] {
                                    ui.selectable_value(&mut mode, m.to_owned(), m);
                                }
                            });
                        ui.data_mut(|d| d.insert_temp(id, mode.clone()));
                        let mut factor = ui
                            .data(|d| d.get_temp::<f64>(id.with("drag")))
                            .unwrap_or_else(|| p["factor"].as_f64().unwrap_or_default());
                        let resp = ui.add(egui::Slider::new(&mut factor, 0.0..=3.0).text("Amount"));
                        if resp.dragged() {
                            ui.data_mut(|d| d.insert_temp(id.with("drag"), factor));
                        }
                        if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                            ui.data_mut(|d| d.remove::<f64>(id.with("drag")));
                            send(out, "parts.explode", json!([p["uid"], factor, mode]));
                        }
                        if ui.button("Reassemble").clicked() {
                            send(out, "parts.explode", json!([p["uid"], 0., mode]));
                        }
                    }
                });
            });
        }
    }

    pub fn export(&mut self, ui: &mut Ui, out: &mut Vec<String>) {
        ui.strong(label(&self.data["selection"]["schema"]));
        self.records.draw(ui, out);
    }
    pub fn has_records(&self) -> bool {
        !self.records.rows.is_empty()
    }

    pub fn bevel(&mut self, ui: &mut Ui, out: &mut Vec<String>) {
        let b = &self.data["bevel"];
        choice(
            ui,
            "bevel.affect",
            label(&b["affect"]),
            &["edges", "vertices"],
            out,
            &mut self.popup_layers,
        );
        ui.add_enabled_ui(b["affect"] == "edges", |ui| {
            choice(
                ui,
                "bevel.measure",
                label(&b["measure"]),
                &["radius", "offset", "width", "depth"],
                out,
                &mut self.popup_layers,
            )
        });
        let id = ui.make_persistent_id("bevel.amount");
        let mut amount = ui
            .data(|d| d.get_temp::<String>(id))
            .unwrap_or_else(|| label(&b["amount"]).to_owned());
        let r = ui.add(egui::TextEdit::singleline(&mut amount).desired_width(64.));
        if r.changed() {
            ui.data_mut(|d| d.insert_temp(id, amount.clone()));
        }
        if r.lost_focus() && ui.data(|d| d.get_temp::<String>(id)).is_some() {
            send(out, "bevel.amount", json!(amount));
            ui.data_mut(|d| d.remove::<String>(id));
        }
        choice(
            ui,
            "bevel.shape",
            label(&b["shape"]),
            &[
                "round", "chamfer", "steps", "supports", "cornice", "crown", "custom",
            ],
            out,
            &mut self.popup_layers,
        );
        let mut seg = b["segments"].as_i64().unwrap_or(3);
        if ui
            .add(DragValue::new(&mut seg).range(1..=64).suffix(" segments"))
            .changed()
        {
            send(out, "bevel.segments", json!(seg));
        }
        ui.menu_button("Profile…", |ui| {
            self.popup_layers.push(ui.layer_id().id);
            ui.label("Corner fill");
            choice(
                ui,
                "bevel.corners",
                label(&b["corners"]),
                &["round", "cut"],
                out,
                &mut self.popup_layers,
            );
            let mut points = b["points"].clone();
            if let Some(pts) = points.as_array_mut() {
                let end = pts.len().saturating_sub(1);
                let mut changed = false;
                ui.label("Custom curve · x / y");
                let (rect, _) = ui.allocate_exact_size(egui::vec2(140., 80.), egui::Sense::hover());
                let xy = |p: &Value| {
                    egui::pos2(
                        rect.left() + rect.width() * p[0].as_f64().unwrap_or_default() as f32,
                        rect.bottom() - rect.height() * p[1].as_f64().unwrap_or_default() as f32,
                    )
                };
                for pair in pts.windows(2) {
                    ui.painter().line_segment(
                        [xy(&pair[0]), xy(&pair[1])],
                        egui::Stroke::new(1.5_f32, tokens::INK_2),
                    );
                }
                for p in pts.iter_mut().take(end).skip(1) {
                    ui.horizontal(|ui| {
                        for i in 0..2 {
                            let mut v = p[i].as_f64().unwrap_or_default();
                            if ui
                                .add(DragValue::new(&mut v).range(0.0..=1.0).speed(0.01))
                                .changed()
                            {
                                p[i] = json!(v);
                                changed = true;
                            }
                        }
                        let mut smooth = p[2] == true;
                        if ui.checkbox(&mut smooth, "Smooth").changed() {
                            p[2] = json!(smooth);
                            changed = true;
                        }
                    });
                }
                if changed {
                    send(out, "bevel.curve", points);
                }
            }
        });
        if ui.button("Apply").clicked() {
            send(out, "bevel.apply", Value::Null);
        }
        if ui.button("Done").clicked() {
            out.push("tool:select".into());
        }
    }

    pub fn frame_guide(&self, ctx: &egui::Context, screen: egui::Rect) {
        let Some(aspect) = self.data["camera"]["frame_aspect"]
            .as_f64()
            .filter(|v| *v > 0.)
        else {
            return;
        };
        let w = screen.width().min(screen.height() * aspect as f32);
        let frame = egui::Rect::from_center_size(screen.center(), egui::vec2(w, w / aspect as f32));
        let painter = ctx.layer_painter(egui::LayerId::background());
        let shade = tokens::SCRIM;
        for rect in [
            egui::Rect::from_min_max(screen.min, egui::pos2(frame.left(), screen.bottom())),
            egui::Rect::from_min_max(egui::pos2(frame.right(), screen.top()), screen.max),
            egui::Rect::from_min_max(screen.min, egui::pos2(screen.right(), frame.top())),
            egui::Rect::from_min_max(egui::pos2(screen.left(), frame.bottom()), screen.max),
        ] {
            painter.rect_filled(rect, 0., shade);
        }
        painter.rect_stroke(
            frame.shrink(1.),
            0.,
            egui::Stroke::new(1.0_f32, tokens::INK),
            egui::StrokeKind::Inside,
        );
    }

    pub fn camera(&mut self, ui: &mut Ui, out: &mut Vec<String>) {
        let c = &self.data["camera"];
        ui.horizontal(|ui| {
            ui.heading("Camera");
            if ui.button("Close").clicked() {
                send(out, "camera.close", Value::Null);
            }
        });
        ui.weak(label(&c["scene"]));
        ui.add_space(7.);
        ui.strong("Lens and frame");
        ui.add_space(4.);
        ui.label("Projection");
        choice(
            ui,
            "camera.projection",
            label(&c["projection"]),
            &["perspective", "two_point", "parallel"],
            out,
            &mut self.popup_layers,
        );
        ui.add_enabled_ui(c["projection"] != "parallel", |ui| {
            number(
                ui,
                "camera.focal",
                "Lens (mm)",
                &c["focal"],
                1.0..=2000.0,
                out,
            );
            egui::ComboBox::from_label("Lens preset")
                .selected_text("Choose…")
                .show_ui(ui, |ui| {
                    self.popup_layers.push(ui.layer_id().id);
                    for mm in [18, 24, 35, 50, 85, 135] {
                        if ui.button(format!("{mm} mm")).clicked() {
                            send(out, "camera.focal", json!(mm));
                        }
                    }
                });
            for (i, name) in ["Sensor width (mm)", "Sensor height (mm)"]
                .iter()
                .enumerate()
            {
                number(
                    ui,
                    &format!("camera.sensor{i}"),
                    name,
                    &c["sensor"][i],
                    1.0..=100.0,
                    out,
                );
            }
        });
        let aspect = c["frame_aspect"].as_f64().unwrap_or_default();
        egui::ComboBox::from_label("Frame")
            .selected_text(if aspect > 0. {
                format!("{aspect:.2} ∶ 1")
            } else {
                "Window".into()
            })
            .show_ui(ui, |ui| {
                self.popup_layers.push(ui.layer_id().id);
                for (name, v) in [
                    ("Window", 0.),
                    ("16:9", 16. / 9.),
                    ("4:3", 4. / 3.),
                    ("1:1", 1.),
                    ("9:16", 9. / 16.),
                ] {
                    if ui.button(name).clicked() {
                        send(out, "camera.frame_aspect", json!(v));
                    }
                }
            });
        ui.collapsing("Advanced", |ui| {
            for key in ["eye", "target"] {
                for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
                    number(
                        ui,
                        &format!("camera.{key}{i}"),
                        &format!("{key} {axis} (m)"),
                        &c[key][i],
                        -1e9..=1e9,
                        out,
                    );
                }
            }
            for (key, name, lo, hi) in [
                ("fov", "Vertical FOV (°)", 1., 120.),
                ("roll_deg", "Roll (°)", -180., 180.),
                ("shift_x", "Horizontal shift", -1., 1.),
                ("shift_y", "Vertical shift", -1., 1.),
                ("clip_near", "Near clip (0 = auto)", 0., 1e9),
                ("clip_far", "Far clip (0 = auto)", 0., 1e9),
            ] {
                ui.add_enabled_ui(
                    !(key == "roll_deg" && c["projection"] == "two_point"
                        || key == "fov" && c["projection"] == "parallel"),
                    |ui| number(ui, &format!("camera.{key}"), name, &c[key], lo..=hi, out),
                );
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Reset lens").clicked() {
                send(out, "camera.reset", Value::Null);
            }
            if ui
                .add_enabled(c["saved"] == true, egui::Button::new("Update scene"))
                .clicked()
            {
                send(out, "camera.update", c["scene"].clone());
            }
        });
    }
}
