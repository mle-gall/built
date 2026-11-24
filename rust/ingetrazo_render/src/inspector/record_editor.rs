// SPDX-License-Identifier: GPL-3.0-or-later
//! Selection-scoped IFC drafts. Only a changed field or one membership is sent.
use super::{label, send};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Editor {
    schema: String,
    catalog: Value,
    targets: Vec<String>,
    values: BTreeMap<String, String>,
    editable: BTreeSet<String>,
    dirty: Map<String, Value>,
    search: String,
    classification: String,
    code: String,
    title: String,
    classified: BTreeSet<(String, String, String)>,
    members: BTreeMap<String, usize>,
    fresh: bool,
    tagged: bool,
    quantities: String,
    error: String,
}
fn entries(v: &Value) -> impl Iterator<Item = (&String, &Value)> {
    v.as_object().into_iter().flatten()
}
fn array(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_array().into_iter().flatten()
}
fn pick(
    ui: &mut egui::Ui,
    id: &str,
    title: &str,
    selected: &mut String,
    options: impl IntoIterator<Item = String>,
) -> bool {
    let before = selected.clone();
    egui::ComboBox::from_id_salt(id)
        .selected_text(if selected.is_empty() {
            title
        } else {
            selected.as_str()
        })
        .show_ui(ui, |ui| {
            for key in options {
                ui.selectable_value(
                    selected,
                    key.clone(),
                    if key.is_empty() { "—" } else { &key },
                );
            }
        });
    before != *selected
}
impl Editor {
    pub(super) fn load(data: &Value) -> Self {
        let mut e = Self {
            schema: label(&data["schema"]).into(),
            catalog: data["vocabulary"].clone(),
            ..Default::default()
        };
        for o in array(&data["records"]).filter(|o| o["editable"] == true) {
            let first = e.targets.is_empty();
            e.targets.push(label(&o["uid"]).into());
            e.fresh |= !o["ifc_class"].is_string();
            e.tagged |= o["ifc_class"].is_string();
            let mut can = BTreeSet::new();
            for f in array(&o["fields"]) {
                let path = &f["path"];
                let k = label(&path[0]);
                if k == "classifications" {
                    e.classified.insert((
                        label(&path[1]).into(),
                        label(&path[2]).into(),
                        label(&f["value"]).into(),
                    ));
                }
                if path.as_array().is_none_or(|p| p.len() != 1) {
                    continue;
                }
                if f["editable"] == true {
                    can.insert(k.to_owned());
                }
                let value = label(&f["value"]);
                let v = e.values.entry(k.into()).or_insert_with(|| value.into());
                if *v != value {
                    v.clear();
                }
            }
            if first {
                e.editable = can;
            } else {
                e.editable.retain(|k| can.contains(k));
            }
            for k in array(&o["systems"]) {
                *e.members.entry(label(k).into()).or_default() += 1;
            }
            let q = if o["quantities"].is_object() {
                o["quantities"].to_string()
            } else {
                "{}".into()
            };
            if first {
                e.quantities = q
            } else if e.quantities != q {
                e.quantities.clear();
            }
        }
        if e.fresh && !e.tagged {
            e.values.insert("class".into(), "IfcWall".into());
        }
        e
    }

    fn classes(&self) -> Vec<(String, String, bool)> {
        let needle = self.search.to_lowercase();
        let fav: BTreeSet<_> = array(&self.catalog["favourites"])
            .filter_map(Value::as_str)
            .collect();
        let mut rows: Vec<_> = entries(&self.catalog["classes"])
            .filter_map(|(name, info)| {
                let domain = label(&info["domain"]);
                (array(&info["schemas"]).any(|s| s == &self.schema)
                    && (name.to_lowercase().contains(&needle)
                        || domain.to_lowercase().contains(&needle)))
                .then(|| (name.clone(), domain.to_owned(), fav.contains(name.as_str())))
            })
            .collect();
        rows.sort_by(|a, b| (!a.2, &a.1, &a.0).cmp(&(!b.2, &b.1, &b.0)));
        rows
    }

    pub(super) fn draw(&mut self, ui: &mut egui::Ui, out: &mut Vec<String>) {
        if self.targets.is_empty() {
            return;
        }
        ui.collapsing("Edit IFC record", |ui| {
            pick(
                ui,
                "schema",
                "Schema",
                &mut self.schema,
                ["IFC4".into(), "IFC4X3_ADD2".into()],
            );
            let class = self.values.get("class").cloned().unwrap_or_default();
            ui.add_enabled_ui(self.editable.contains("class"), |ui| {
                egui::ComboBox::from_id_salt("ifc_class")
                    .selected_text(if class.is_empty() {
                        "Mixed classes"
                    } else {
                        &class
                    })
                    .show_ui(ui, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.search)
                                .hint_text("Filter class or domain"),
                        );
                        let mut last = None;
                        for (name, domain, fav) in self.classes() {
                            let heading = if fav { "Favourites" } else { &domain };
                            if last.as_deref() != Some(heading) {
                                ui.weak(heading);
                                last = Some(heading.to_owned());
                            }
                            if ui.selectable_label(name == class, &name).clicked() {
                                self.dirty.insert("class".into(), json!(name));
                                self.values.insert("class".into(), name);
                                self.values.insert("predefined_type".into(), String::new());
                                self.dirty.insert("predefined_type".into(), Value::Null);
                                self.search.clear();
                            }
                        }
                    });
            });
            let class = self.values.get("class").cloned().unwrap_or_default();
            for (field, title, options) in [
                (
                    "predefined_type",
                    "Predefined type",
                    &self.catalog["classes"][&class]["predefined_types"][&self.schema],
                ),
                ("type", "Type", &self.catalog["types"]),
            ] {
                if !self.editable.contains(field) {
                    continue;
                }
                let opts = std::iter::once(String::new())
                    .chain(array(options).filter_map(Value::as_str).map(str::to_owned));
                let value = self.values.entry(field.into()).or_default();
                ui.label(title);
                if pick(ui, field, title, value, opts) {
                    self.dirty.insert(
                        field.into(),
                        if value.is_empty() {
                            Value::Null
                        } else {
                            json!(value)
                        },
                    );
                }
            }
            for (field, title) in [
                ("object_type", "Object type"),
                ("name", "Name"),
                ("tag", "Tag"),
                ("description", "Description"),
            ] {
                if !self.editable.contains(field) {
                    continue;
                }
                let value = self.values.entry(field.into()).or_default();
                ui.label(title);
                if ui.text_edit_singleline(value).changed() {
                    self.dirty.insert(field.into(), json!(value));
                }
            }
            ui.collapsing("Quantities", |ui| {
                if ui
                    .add(
                        egui::TextEdit::multiline(&mut self.quantities)
                            .hint_text("Quantities (JSON)"),
                    )
                    .changed()
                {
                    match serde_json::from_str::<Value>(&self.quantities) {
                        Ok(value) if value.is_object() => {
                            self.error.clear();
                            self.dirty.insert("quantities".into(), value);
                        }
                        _ => self.error = "Enter a quantities object".into(),
                    }
                }
            });
            if !self.error.is_empty() {
                ui.label(&self.error);
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        (self.fresh || !self.dirty.is_empty()) && self.error.is_empty(),
                        egui::Button::new("Apply record"),
                    )
                    .clicked()
                {
                    send(
                        out,
                        "record.fields",
                        json!([self.targets, self.dirty, self.schema]),
                    );
                }
                if self.tagged && ui.button("Remove record").clicked() {
                    send(
                        out,
                        "record.untag",
                        json!([self.targets, null, self.schema]),
                    );
                }
            });
            ui.collapsing("System membership", |ui| {
                for (key, v) in entries(&self.catalog["systems"]) {
                    let count = self.members.get(key).copied().unwrap_or(0);
                    let mut checked = count == self.targets.len();
                    let title = v["name"].as_str().unwrap_or(key);
                    let mixed = count > 0 && !checked;
                    if ui
                        .add(egui::Checkbox::new(&mut checked, title).indeterminate(mixed))
                        .changed()
                    {
                        send(
                            out,
                            "record.system",
                            json!([self.targets, [key, checked], self.schema]),
                        );
                    }
                }
            });
            ui.collapsing("Classifications", |ui| {
                for (system, code, title) in &self.classified {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("{system} · {code} {title}"));
                        if ui.small_button("Remove").clicked() {
                            send(
                                out,
                                "record.classification",
                                json!([self.targets, [system, code, title, false], self.schema]),
                            );
                        }
                    });
                }
                let systems = &self.catalog["classifications"];
                if pick(
                    ui,
                    "classification-system",
                    "Classification system",
                    &mut self.classification,
                    entries(systems).map(|(k, _)| k.clone()),
                ) {
                    self.code.clear();
                    self.title.clear();
                }
                let codes = &systems[&self.classification]["codes"];
                if pick(
                    ui,
                    "classification-code",
                    "Choose code",
                    &mut self.code,
                    entries(codes).map(|(k, _)| k.clone()),
                ) {
                    self.title = label(&codes[&self.code]["name"]).into();
                }
                ui.add(egui::TextEdit::singleline(&mut self.code).hint_text("Code"));
                ui.add(egui::TextEdit::singleline(&mut self.title).hint_text("Name"));
                if ui
                    .add_enabled(
                        !self.classification.is_empty() && !self.code.trim().is_empty(),
                        egui::Button::new("Add classification"),
                    )
                    .clicked()
                {
                    send(
                        out,
                        "record.classification",
                        json!([
                            self.targets,
                            [self.classification, self.code, self.title, true],
                            self.schema
                        ]),
                    );
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn class_picker_keeps_favourites_first_and_searches_domains_within_the_schema() {
        let mut editor = Editor::load(&json!({"schema":"IFC4","vocabulary":{
        "favourites":["IfcWall"],"classes":{
           "IfcWall":{"domain":"building","schemas":["IFC4","IFC4X3_ADD2"]},
           "IfcBeam":{"domain":"building","schemas":["IFC4","IFC4X3_ADD2"]},
           "IfcRail":{"domain":"infrastructure","schemas":["IFC4X3_ADD2"]}
        }}}));
        assert_eq!(
            editor
                .classes()
                .into_iter()
                .map(|r| r.0)
                .collect::<Vec<_>>(),
            ["IfcWall", "IfcBeam"]
        );
        editor.search = "WALL".into();
        assert_eq!(editor.classes().len(), 1);
        editor.search = "infra".into();
        assert!(editor.classes().is_empty());
        editor.schema = "IFC4X3_ADD2".into();
        assert_eq!(editor.classes()[0].0, "IfcRail");
    }
}
