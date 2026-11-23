use super::{label, send};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Records {
    pub rows: BTreeMap<Vec<String>, Row>,
    total: usize,
    writable: Vec<String>,
    set_name: String,
    property_name: String,
    property_value: String,
    pending: Option<Value>,
    error: String,
}
#[derive(Default)]
pub(super) struct Row {
    value: Value,
    mixed: bool,
    count: usize,
    owner: String,
    targets: Vec<String>,
    draft: String,
    kind: String,
}

impl Records {
    pub fn new(data: &Value, selection_count: usize) -> Self {
        let mut result = Self::default();
        for object in data.as_array().into_iter().flatten() {
            result.total += 1;
            if object["editable"] == true {
                result.writable.push(label(&object["uid"]).into());
            }
            for f in object["fields"].as_array().into_iter().flatten() {
                let path = f["path"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| label(v).to_owned())
                    .collect();
                let r = result.rows.entry(path).or_insert_with(|| Row {
                    value: f["value"].clone(),
                    owner: label(&f["owner"]).into(),
                    kind: label(&f["kind"]).into(),
                    ..Default::default()
                });
                r.count += 1;
                r.mixed |= r.value != f["value"];
                if r.owner != label(&f["owner"]) {
                    r.owner = "Mixed sources".into();
                }
                if f["editable"] == true && r.kind == label(&f["kind"]) {
                    r.targets.push(label(&object["uid"]).into());
                }
            }
        }
        result.total = result.total.max(selection_count);
        for r in result.rows.values_mut() {
            r.mixed |= r.count != result.total;
            r.draft = if r.mixed {
                String::new()
            } else {
                r.value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| r.value.to_string())
            };
        }
        result
    }

    pub fn draw(&mut self, ui: &mut egui::Ui, out: &mut Vec<String>) {
        ui.weak("Edit a value and press Enter");
        let mut ordered: Vec<_> = self.rows.iter_mut().collect();
        ordered.sort_by_key(|(path, _)| match path[0].as_str() {
            "class" => 0,
            "predefined_type" => 1,
            "type" => 2,
            "psets" => 3,
            "classifications" => 4,
            _ => 5,
        });
        for (path, r) in ordered {
            ui.push_id(path, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let title = match path[0].as_str() {
                        "class" => "IFC class".to_owned(),
                        "predefined_type" => "Predefined type".to_owned(),
                        "object_type" => "Object type".to_owned(),
                        "psets" | "classifications" => path[1..].join(" › "),
                        field => field.to_owned(),
                    };
                    ui.strong(title);
                    ui.weak(&r.owner);
                    if path[0] == "psets"
                        && r.owner == "This object"
                        && ui
                            .small_button("Reset")
                            .on_hover_text("Restore inherited value")
                            .clicked()
                    {
                        self.pending = Some(json!([r.targets, path, Value::Null]));
                    }
                });
                if r.targets.is_empty() {
                    ui.label(if r.mixed { "Mixed" } else { &r.draft });
                    return;
                }
                let response = ui.add(
                    egui::TextEdit::singleline(&mut r.draft)
                        .desired_width(ui.available_width())
                        .hint_text(if r.mixed { "Mixed" } else { "Enter value" }),
                );
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let val = match r.kind.as_str() {
                        "text" | "label" | "enum" => Some(json!(r.draft)),
                        "boolean" => r.draft.parse::<bool>().ok().map(Value::Bool),
                        "integer" => r.draft.parse::<i64>().ok().map(Value::from),
                        _ => r
                            .draft
                            .parse::<f64>()
                            .ok()
                            .filter(|n| n.is_finite())
                            .map(Value::from),
                    };
                    if let Some(v) = val {
                        self.error.clear();
                        if self.total > 1 {
                            self.pending = Some(json!([r.targets, path, v]));
                        } else {
                            send(out, "record.patch", json!([r.targets, path, v]));
                        }
                    } else {
                        self.error = format!("Enter a {} value", r.kind);
                    }
                }
            });
        }
        if !self.error.is_empty() {
            ui.label(&self.error);
        }
        if !self.writable.is_empty() {
            ui.collapsing("Add property", |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.set_name).hint_text("Property set"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.property_name).hint_text("Property name"),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.property_value)
                        .hint_text("Text, number, or true / false"),
                );
                if ui
                    .add_enabled(
                        !self.set_name.trim().is_empty() && !self.property_name.trim().is_empty(),
                        egui::Button::new("Preview addition"),
                    )
                    .clicked()
                {
                    let path = vec![
                        "psets".to_string(),
                        self.set_name.trim().into(),
                        self.property_name.trim().into(),
                    ];
                    if self.rows.contains_key(&path) {
                        self.error = "This property already has a row above".into();
                    } else {
                        let raw = &self.property_value;
                        let (kind, value) = match serde_json::from_str::<Value>(raw) {
                            Ok(v @ Value::Bool(_)) => ("boolean", v),
                            Ok(v @ Value::Number(_)) if v.is_i64() => ("integer", v),
                            Ok(v @ Value::Number(_)) => ("real", v),
                            _ => ("text", json!(raw)),
                        };
                        self.error.clear();
                        self.pending = Some(json!([self.writable, path, value, kind]));
                    }
                }
            });
        }
        if let Some(request) = self.pending.clone() {
            ui.separator();
            let path = request[1]
                .as_array()
                .into_iter()
                .flatten()
                .map(label)
                .collect::<Vec<_>>()
                .join(" › ");
            let n = request[0].as_array().map_or(0, Vec::len);
            let value = if request[2].is_null() {
                "inherited value".into()
            } else {
                request[2].to_string()
            };
            ui.label(format!("{path} → {value} · {n} of {} objects", self.total));
            ui.horizontal(|ui| {
                if ui.button("Apply patch").clicked() {
                    send(out, "record.patch", request);
                    self.pending = None;
                }
                if ui.button("Cancel").clicked() {
                    self.pending = None;
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_patch_excludes_linked_and_absent_fields() {
        let field = |value, editable| json!({"path":["psets","Survey","Checked"],"value":value,"owner":"This object","kind":"boolean","editable":editable});
        let data = json!([{"uid":"a","fields":[field(true,true)]},{"uid":"b","fields":[field(false,true)]},{"uid":"link","fields":[field(true,false)]},{"uid":"wall","fields":[]}]);
        let vm = Records::new(&data, 4);
        let row = vm.rows.values().next().unwrap();
        assert!(row.mixed);
        assert_eq!(vm.total, 4);
        assert_eq!(row.targets, ["a", "b"]);
    }
}
