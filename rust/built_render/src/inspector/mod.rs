//! Native inspector and tool controls. Input snapshots belong to one document.
pub(crate) mod bevel;
mod parts;
mod records;
mod record_editor;
mod rule;
mod terrain;

use crate::ui::{kit, tokens};
use egui::Ui;
use serde_json::{json, Value};
pub use terrain::{plot_png, Terrain};

#[derive(Default)]
pub struct Panels {
    pub data: Value,
    pub popup_layers: Vec<egui::Id>,
    parts: parts::Parts,
    curve_editor: kit::CurveEditor,
    bevel_curve: Option<bevel::Curve>,
    bevel_outline: Vec<[f64; 2]>,
    records: records::Records,
    rule: rule::Rule,
    pub terrain: Terrain,
}

/// The Fillet/Chamfer form's Size field (tests focus it by this id).
pub(crate) fn bevel_size_field() -> egui::Id {
    egui::Id::new("bevel.size.field")
}

fn send(out: &mut Vec<String>, key: &str, value: Value) {
    out.push(format!("panel:{}", json!([key, value])));
}

fn label(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

fn bevel_choice<const N: usize>(
    ui: &mut Ui,
    action: &str,
    current: &str,
    options: [(&'static str, &'static str); N],
    out: &mut Vec<String>,
    layers: &mut Vec<egui::Id>,
) {
    let display = options.iter().find(|(key, _)| *key == current).map_or("—", |(_, name)| *name);
    let names = options.map(|(_, name)| name);
    let (selected, popup) = kit::select(ui, action, display, &names);
    layers.extend(popup);
    if let Some(i) = selected {
        send(out, action, json!(options[i].0));
    }
}

fn number(
    ui: &mut Ui,
    key: &str,
    name: &str,
    val: &Value,
    range: std::ops::RangeInclusive<f64>,
    out: &mut Vec<String>,
) {
    kit::property_control(ui, name, |ui| {
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
            self.records.editor = record_editor::Editor::load(&data);
        }
        if key == "bevel" {
            if data.is_null()
                || (data["shape"].is_string() && data["shape"] != self.data[key]["shape"])
            {
                self.curve_editor = Default::default();
            }
            self.bevel_curve = serde_json::from_value(data["points"].clone()).ok();
            self.bevel_outline =
                serde_json::from_value(data["outline"].clone()).unwrap_or_default();
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
        if !self.has_records() { self.rule.draw(ui, &self.data["rulePreview"], out); }
        let p = &s["parts"];
        if p.is_object() {
            kit::disclosure(ui, ("parts", label(&p["uid"])), &format!("Parts · {}", label(&p["name"])), false, |ui| {
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
                    Self::assembly_actions(ui, p, &mut self.popup_layers, out);
                });
            });
        }
    }

    /// The Problems mode shares the same selection and assembly commands as
    /// Project. Its details use the existing inspector content region.
    pub fn assembly_problems(&mut self, ui: &mut Ui, out: &mut Vec<String>) {
        let p = &self.data["selection"]["parts"];
        if !p.is_object() {
            return;
        }
        kit::disclosure(
            ui,
            ("assembly", label(&p["uid"])),
            &format!("Parts · {}", label(&p["name"])),
            false,
            |ui| {
                if kit::quiet(ui, "Cut list…").clicked() {
                    send(out, "parts.report", p["uid"].clone());
                }
                self.parts.draw(ui, p, &self.data["partMeasures"], out);
                ui.add_enabled_ui(p["editable"] == true, |ui| {
                    Self::assembly_actions(ui, p, &mut self.popup_layers, out);
                });
            },
        );
    }

    fn assembly_actions(ui: &mut Ui, p: &Value, popups: &mut Vec<egui::Id>, out: &mut Vec<String>) {
        if p["can_split"] == true && ui.button("Split into pieces").clicked() {
            send(out, "parts.split", p["uid"].clone());
        }
        if p["children"].as_array().is_some_and(|v| v.len() >= 2) {
            let id = ui.make_persistent_id(("explode", &p["uid"].to_string()));
            let current = label(&p["mode"]);
            let mut mode = ui
                .data(|d| d.get_temp::<String>(id))
                .unwrap_or(current.to_owned());
            let before = mode.clone();
            egui::ComboBox::from_id_salt(id)
                .selected_text(&mode)
                .show_ui(ui, |ui| {
                    popups.push(ui.layer_id().id);
                    for m in ["outward", "x", "y", "z"] {
                        ui.selectable_value(&mut mode, m.to_owned(), m);
                    }
                });
            if before != mode && p["factor"].as_f64().unwrap_or_default() != 0. {
                send(out, "parts.explode", json!([p["uid"], p["factor"], mode]));
            }
            ui.data_mut(|d| d.insert_temp(id, mode.clone()));
            let mut factor = ui
                .data(|d| d.get_temp::<f64>(id.with("drag")))
                .unwrap_or_else(|| p["factor"].as_f64().unwrap_or_default());
            let resp = ui.add(egui::Slider::new(&mut factor, 0.0..=3.0).text("Amount"));
            let cancel = resp.dragged() && ui.input(|i| i.key_pressed(egui::Key::Escape));
            if cancel {
                ui.ctx().stop_dragging();
                ui.data_mut(|d| d.remove::<f64>(id.with("drag")));
                send(out, "parts.cancel", json!([p["uid"]]));
            } else if resp.dragged() {
                ui.data_mut(|d| d.insert_temp(id.with("drag"), factor));
                if resp.changed() {
                    send(out, "parts.preview", json!([p["uid"], factor, mode]));
                }
            }
            if !cancel && (resp.drag_stopped() || (resp.changed() && !resp.dragged())) {
                ui.data_mut(|d| d.remove::<f64>(id.with("drag")));
                send(out, "parts.explode", json!([p["uid"], factor, mode]));
            }
            if ui.button("Reassemble").clicked() {
                send(out, "parts.explode", json!([p["uid"], 0., mode]));
            }
        }
    }

    pub fn export(&mut self, ui: &mut Ui, out: &mut Vec<String>, measured: &[(String,String)]) {
        self.records.draw(ui, out, &mut self.popup_layers, measured);
    }
    pub fn has_records(&self) -> bool {
        !self.records.rows.is_empty()
    }

    pub fn bevel_inspector(&mut self, ui: &mut Ui, out: &mut Vec<String>) {
        let b = &self.data["bevel"];
        kit::caption_plain(ui, "Bevel profile");
        if b["shape"].is_null() {
            number(
                ui,
                "bevel.profile",
                "Curve (0–1)",
                &b["profile"],
                0.001..=0.999,
                out,
            );
            return;
        }
        if let Some(curve) = &mut self.bevel_curve {
            if self.curve_editor.show(ui, curve, &self.bevel_outline) {
                send(out, "bevel.curve", json!(curve));
            }
        }
    }

    pub fn bevel(&mut self, ui: &mut Ui, out: &mut Vec<String>) {
        /// The Size field's text while it is being typed (one id for the
        /// field and for Apply, which sends it first).
        fn bevel_amount_id() -> egui::Id {
            egui::Id::new("bevel.amount")
        }
        let b = &self.data["bevel"];
        let shape = if b["numeric_profile"] == true && b["shape"].is_null() {
            "profile value"
        } else {
            bevel::shape_name(b)
        };
        let shapes = [
            ("round", "Round"), ("chamfer", "Chamfer"), ("profile value", "Profile value"),
            ("steps", "Steps"), ("supports", "Supports"), ("cornice", "Cornice"),
            ("crown", "Crown"), ("drawn", "Drawn"),
        ];
        ui.columns(6, |cols| {
            kit::caption_plain(&mut cols[0], "Affect");
            bevel_choice(&mut cols[0], "bevel.affect", label(&b["affect"]),
                         [("edges", "Edges"), ("vertices", "Vertices")], out, &mut self.popup_layers);
            kit::caption_plain(&mut cols[1], "Measure");
            cols[1].add_enabled_ui(b["affect"] == "edges", |ui| {
                bevel_choice(ui, "bevel.measure", label(&b["measure"]),
                             [("radius", "Radius"), ("offset", "Offset"), ("width", "Width"), ("depth", "Depth")],
                             out, &mut self.popup_layers);
            });
            let ui = &mut cols[2];
            kit::caption_plain(ui, "Size");
            let id = bevel_amount_id();
            let mut amount = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| label(&b["amount"]).to_owned());
            let error = b["error"].as_str().filter(|e| !e.is_empty());
            let mut edit = kit::field(&mut amount).id(bevel_size_field()).desired_width(ui.available_width());
            if error.is_some() {
                edit = edit.text_color(tokens::ERROR);
            }
            let field = ui.add(edit);
            if let Some(reason) = error {
                field.clone().on_hover_text(reason);
            }
            if field.changed() { ui.data_mut(|d| d.insert_temp(id, amount.clone())); }
            if field.lost_focus() && ui.data(|d| d.get_temp::<String>(id)).is_some() {
                send(out, "bevel.amount", json!(amount));
                ui.data_mut(|d| d.remove::<String>(id));
            }
            kit::caption_plain(&mut cols[3], "Profile");
            bevel_choice(&mut cols[3], "bevel.shape", shape, shapes, out, &mut self.popup_layers);
            kit::caption_plain(&mut cols[4], "Segments");
            let mut segments = b["segments"].as_f64().unwrap_or(3.);
            if cols[4].add(kit::number(&mut segments, "").range(1..=64).fixed_decimals(0)).changed() {
                send(out, "bevel.segments", json!(segments.round() as i64));
            }
            kit::caption_plain(&mut cols[5], "Corners");
            cols[5].add_enabled_ui(b["affect"] == "edges", |ui| {
                bevel_choice(ui, "bevel.corners", label(&b["corners"]),
                             [("round", "Follow profile"), ("cut", "Flat cut")], out, &mut self.popup_layers);
            });
        });
        self.bevel_inspector(ui, out);
        ui.horizontal(|ui| {
            if kit::sheet_button(ui, "Apply", "↵", 1) {
                // A size still being typed applies with it, not the old one.
                let id = bevel_amount_id();
                if let Some(amount) = ui.data_mut(|d| d.remove_temp::<String>(id)) {
                    send(out, "bevel.amount", json!(amount));
                }
                send(out, "bevel.finish", Value::Null);
            }
            if kit::sheet_button(ui, "Cancel", "Esc", 0) {
                out.push("tool:select".into());
            }
            // The refusal beside Apply, which it explains: what fits instead.
            // On this row the form keeps its height, so the Size field does
            // not move under the pointer as the reason comes and goes.
            if let Some(reason) = self.data["bevel"]["error"].as_str().filter(|e| !e.is_empty()) {
                ui.add_space(tokens::SPACE_S);
                ui.label(egui::RichText::new(reason).font(tokens::meta()).color(tokens::ERROR));
            }
        });
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
        if c["bound"] == true {
            kit::caption_plain(ui, &format!("Editing scene · {}", label(&c["scene"])));
        } else {
            kit::caption_plain(ui, "Current view");
        }
        let mut select = |ui: &mut Ui, name: &str, current: &str, labels: &[&str]| {
            let mut picked = None;
            kit::property_control(ui, name, |ui| {
                let (index, popup) = kit::select(ui, name, current, labels);
                self.popup_layers.extend(popup);
                picked = index;
            });
            picked
        };
        let projections = ["Perspective", "Two-point", "Parallel"];
        let keys = ["perspective", "two_point", "parallel"];
        let index = keys.iter().position(|key| *key == label(&c["projection"])).unwrap_or(0);
        if let Some(i) = select(ui, "Projection", projections[index], &projections) {
            send(out, "camera.projection", json!(keys[i]));
        }
        ui.add_enabled_ui(c["projection"] != "parallel", |ui| {
            number(
                ui,
                "camera.focal",
                "Lens (mm)",
                &c["focal"],
                1.0..=2000.0,
                out,
            );
            let lenses = [14, 18, 24, 28, 35, 50, 85, 105, 135];
            let names = lenses.map(|mm| format!("{mm} mm"));
            let names = names.each_ref().map(String::as_str);
            let current = lenses.iter().position(|&mm| c["focal"].as_f64() == Some(mm as f64))
                .map_or("Custom", |i| names[i]);
            if let Some(i) = select(ui, "Lens preset", current, &names) {
                send(out, "camera.focal", json!(lenses[i]));
            }
            let sensors = [
                ("Full Frame", [36., 24.]),
                ("APS-C", [23.6, 15.6]),
                ("Micro 4/3", [17.3, 13.]),
                ("Super 35", [24.9, 18.7]),
            ];
            let custom = c["custom_sensor"] == true;
            let selected = sensors
                .iter()
                .find(|(_, dims)| c["sensor"] == json!(dims) && !custom)
                .map_or("Custom", |(name, _)| *name);
            let mut names: Vec<_> = sensors.iter().map(|(name, _)| *name).collect();
            names.push("Custom");
            if let Some(i) = select(ui, "Sensor", selected, &names) {
                send(out, "camera.sensor", sensors.get(i).map_or(Value::Null, |(_, dims)| json!(dims)));
            }
            if selected == "Custom" {
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
            }
        });
        let aspect = c["frame_aspect"].as_f64().unwrap_or_default();
        let frames = [("Window", 0.), ("16:9", 16./9.), ("4:3", 4./3.), ("3:2", 1.5),
                      ("A (√2:1)", std::f64::consts::SQRT_2), ("2.39:1", 2.39), ("1:1", 1.), ("9:16", 9./16.)];
        let current = frames.iter().find(|(_, value)| (value - aspect).abs() < 1e-5)
            .map_or_else(|| format!("{aspect:.2} ∶ 1"), |(name, _)| name.to_string());
        if let Some(i) = select(ui, "Frame", &current, &frames.map(|(name, _)| name)) {
            send(out, "camera.frame_aspect", json!(frames[i].1));
        }
        number(
            ui,
            "camera.shift_y",
            "Vertical shift",
            &c["shift_y"],
            -1.0..=1.0,
            out,
        );
        let advanced = c["advanced"] == true;
        let open = kit::disclosure(ui, "camera_advanced", "Advanced", advanced, |ui| {
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
                ("clip_near", "Near clip (0 = auto)", 0., 1e9),
                ("clip_far", "Far clip (0 = auto)", 0., 1e9),
            ] {
                ui.add_enabled_ui(
                    !(key == "roll_deg" && c["projection"] == "two_point"
                        || key == "fov" && c["projection"] == "parallel"),
                    |ui| number(ui, &format!("camera.{key}"), name, &c[key], lo..=hi, out),
                );
            }
        }).is_some();
        if open != advanced { send(out, "camera.advanced", json!(open)); }
        ui.horizontal(|ui| {
            if kit::quiet(ui, "Reset lens").clicked() {
                send(out, "camera.reset", Value::Null);
            }
            if ui.add_enabled_ui(c["saved"] == true, |ui| kit::chip_button(ui, "Update scene")).inner
            {
                send(out, "camera.update", c["scene"].clone());
            }
        });
    }
}
