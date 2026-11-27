use super::{label, send};
use crate::ui::{kit, tokens};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Records {
    pub(super) editor: super::record_editor::Editor,
    pub rows: BTreeMap<Vec<String>, Row>,
    sweep:Value,
    sweep_editable:bool,
    total: usize,
    writable: Vec<String>,
    set_name: String,
    property_name: String,
    property_value: String,
    property_kind: String,
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
    editing: bool,
    kind: String,
}

impl Records {
    pub fn new(data: &Value, selection_count: usize) -> Self {
        let mut result = Self::default();
        for object in data.as_array().into_iter().flatten() {
            if selection_count==1{
                result.sweep=object["sweep"].clone();
                result.sweep_editable=object["editable"]==true;
            }
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

    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        out: &mut Vec<String>,
        layers: &mut Vec<egui::Id>,
        measured: &[(String, String)],
    ) {
        if let Some(row)=self.sweep.as_object(){
            kit::disclosure(ui,"sweep record","Sweep",true,|ui|{
                let visible=(ui.clip_rect().right()-ui.cursor().min.x-8.).max(1.);
                ui.set_width(ui.available_width().min(visible));
                for (key,title) in [("profile_name","Profile"),("preset","Preset"),("attachment","Attachment"),("state","State")]{
                    if let Some(value)=row.get(key).and_then(Value::as_str){kit::prop_row(ui,title,value);}
                }
                kit::prop_row(ui,"Rotation",&format!("{}°",self.sweep["rotation"]));
                kit::prop_row(ui,"Mirror",if self.sweep["mirror"]==true{"On"}else{"Off"});
                if let Some(path)=self.sweep["resolved_points"].as_array() {
                    kit::prop_row(ui,"Path",&format!("{} spans",path.len().saturating_sub(1)+usize::from(self.sweep["path"]["closed"]==true)));
                    if path.len()>2{
                        let current=usize::from(self.sweep["corner"]=="butt");
                        let mut selected=current;
                        ui.add_enabled_ui(self.sweep_editable,|ui|ui.horizontal(|ui|{
                            ui.label("Corner");
                            kit::choice(ui,"sweep corner",&mut selected,&["Mitre","Butt"]);
                        }));
                        if current!=selected{send(out,if selected==1{"sweep.butt"}else{"sweep.mitre"},self.sweep["owner"].clone());}
                    }
                }
                ui.add_enabled_ui(self.sweep_editable,|ui|ui.horizontal(|ui|{
                    if self.sweep["path"]["points"].is_array() && kit::quiet(ui,"Edit path").clicked(){
                        send(out,"sweep.path",self.sweep["owner"].clone());
                    }
                    if kit::quiet(ui,"Release").clicked(){send(out,"sweep.release",self.sweep["owner"].clone());}
                }));
            });
        }
        kit::disclosure(ui, "record identity", "Identity", true, |ui| {
            self.editor.identity(ui, out, layers)
        });
        let ready = self.editor.ready();
        kit::disclosure(ui, "record psets", "Property sets", false, |ui| {
            ui.add_enabled_ui(ready, |ui| self.properties(ui, out))
        });
        kit::disclosure(ui, "record quantities", "Quantities", false, |ui| {
            for (unit, total) in measured {
                kit::prop_row(ui, &format!("Measured · {unit}"), total);
            }
            ui.add_enabled_ui(ready && !self.writable.is_empty(), |ui| {
                self.editor.quantities(ui, out)
            })
        });
        kit::disclosure(
            ui,
            "record classifications",
            "Classifications",
            false,
            |ui| {
                ui.add_enabled_ui(ready && !self.writable.is_empty(), |ui| {
                    self.editor.classifications(ui, out)
                })
            },
        );
    }
    fn properties(&mut self, ui: &mut egui::Ui, out: &mut Vec<String>) {
        for (path, r) in self
            .rows
            .iter_mut()
            .filter(|(path, _)| path.first().is_some_and(|s| s == "psets"))
        {
            ui.push_id(path, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let title = path[1..].join(" › ");
                    ui.strong(title);
                    ui.add_space(tokens::SPACE_S);
                    ui.weak(&r.owner);
                    ui.add_space(tokens::SPACE_XS);
                    if path[0] == "psets"
                        && r.owner == "This object"
                        && kit::quiet(ui, "Reset")
                            .on_hover_text("Restore inherited value")
                            .clicked()
                    {
                        send(out, "record.patch", json!([r.targets, path, Value::Null]));
                    }
                });
                if r.targets.is_empty() {
                    ui.label(if r.mixed { "Mixed" } else { &r.draft });
                    return;
                }
                let response = ui.add(
                    kit::field(&mut r.draft)
                        .desired_width(ui.available_width())
                        .hint_text(if r.mixed { "Mixed" } else { "Enter value" }),
                );
                r.editing |= response.changed();
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    r.draft = if r.mixed {
                        String::new()
                    } else {
                        r.value
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| r.value.to_string())
                    };
                    r.editing = false;
                }
                if r.editing
                    && response.lost_focus()
                    && ui
                        .input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Tab))
                {
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
                        r.editing = false;
                        self.error.clear();
                        let targets = if r.targets.len() == r.count {
                            &self.writable
                        } else {
                            &r.targets
                        };
                        send(out, "record.patch", json!([targets, path, v, r.kind]));
                    } else {
                        self.error = format!("Enter a {} value", r.kind);
                        response.request_focus();
                    }
                }
            });
        }
        if !self.error.is_empty() {
            ui.label(&self.error);
        }
        if !self.writable.is_empty() {
            kit::disclosure(ui, "add property", "Add property", false, |ui| {
                ui.add(kit::field(&mut self.set_name).hint_text("Property set"));
                ui.add(kit::field(&mut self.property_name).hint_text("Property name"));
                let kinds = [
                    "Automatic",
                    "text",
                    "label",
                    "enum",
                    "boolean",
                    "integer",
                    "real",
                    "length",
                    "area",
                    "volume",
                ];
                kit::property_control(ui, "Property kind", |ui| {
                    let value = if self.property_kind.is_empty() {
                        kinds[0]
                    } else {
                        &self.property_kind
                    };
                    if let (Some(index), _) = kit::select(ui, "property kind", value, &kinds) {
                        self.property_kind = if index == 0 {
                            String::new()
                        } else {
                            kinds[index].into()
                        };
                    }
                });
                ui.add(
                    kit::field(&mut self.property_value).hint_text("Text, number, or true / false"),
                );
                if ui
                    .add_enabled_ui(
                        !self.set_name.trim().is_empty() && !self.property_name.trim().is_empty(),
                        |ui| kit::quiet(ui, "Add property"),
                    )
                    .inner
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
                        let (kind, value) = match self.property_kind.as_str() {
                            "" => (kind, value),
                            k @ ("text" | "label" | "enum") => (k, json!(raw)),
                            k @ ("real" | "length" | "area" | "volume") if value.is_number() => {
                                (k, value)
                            }
                            k if k == kind => (k, value),
                            k => {
                                self.error = format!("Enter a {k} value");
                                return;
                            }
                        };
                        send(
                            out,
                            "record.patch",
                            json!([self.writable, path, value, kind]),
                        );
                    }
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
