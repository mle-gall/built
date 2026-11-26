// SPDX-License-Identifier: GPL-3.0-or-later
//! Selection-scoped IFC drafts. Only a changed field or one membership is sent.
use super::{label, send};
use crate::ui::{kit, tokens};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Editor {
    schema: String,
    catalog: Value,
    targets: Vec<String>,
    values: BTreeMap<String, String>,
    editable: BTreeSet<String>,
    mixed: BTreeSet<String>,
    drafts: BTreeMap<String, String>,
    allowed: Option<BTreeSet<String>>,
    picker: bool,
    predefined: BTreeMap<String, usize>,
    class_change: Option<(String, String)>,
    node: Value,
    node_path: String,
    search: String,
    classification: String,
    code: String,
    title: String,
    classified: BTreeMap<(String, String, String), usize>,
    managed_systems: BTreeSet<String>,
    count: usize,
    members: BTreeMap<String, usize>,
    fresh: bool,
    tagged: bool,
    quantities: BTreeMap<String, String>,
    quantity_name: String,
    quantity_value: String,
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
    let options: Vec<_> = options.into_iter().collect();
    let names: Vec<_> = options
        .iter()
        .map(|v| if v.is_empty() { "Not set" } else { v })
        .collect();
    let (chosen, _) = kit::select(
        ui,
        id,
        if selected.is_empty() { title } else { selected },
        &names,
    );
    if let Some(index) = chosen {
        selected.clone_from(&options[index]);
        true
    } else {
        false
    }
}
fn human_name(name: &str) -> String {
    let mut words = String::new();
    for c in name.trim_start_matches("Ifc").chars() {
        if c.is_uppercase() && !words.is_empty() {
            words.push(' ');
            words.push(c.to_ascii_lowercase());
        } else {
            words.push(c);
        }
    }
    words
}
fn class_description(name: &str, domain: &str, types: &Value) -> String {
    match name {
        "IfcFurniture" => "Individual furnishings such as benches, desks and chairs.".into(),
        "IfcSystemFurnitureElement" => {
            "A unit in a modular furniture system, such as a panel or work surface.".into()
        }
        "IfcWall" => "A vertical building element that encloses or divides spaces.".into(),
        "IfcBeam" => "A structural member that carries loads across a span.".into(),
        "IfcSlab" => "A plate-like building element used for floors, roofs and landings.".into(),
        _ => {
            let examples = array(types)
                .filter_map(Value::as_str)
                .filter(|s| !matches!(*s, "USERDEFINED" | "NOTDEFINED"))
                .take(4)
                .map(str::to_lowercase)
                .collect::<Vec<_>>();
            let label = human_name(name);
            if examples.is_empty() {
                format!("{label} in the {domain} domain.")
            } else {
                format!(
                    "{label} in the {domain} domain. Types include {}.",
                    examples.join(", ")
                )
            }
        }
    }
}
impl Editor {
    pub(super) fn load(data: &Value) -> Self {
        let mut e = Self {
            schema: label(&data["schema"]).into(),
            catalog: data["vocabulary"].clone(),
            ..Default::default()
        };
        for (index, o) in array(&data["records"]).enumerate() {
            let first = index == 0;
            e.count += 1;
            if o["editable"] == true {
                e.targets.push(label(&o["uid"]).into());
            }
            if first {
                e.node = o["node"].clone();
                e.node_path = array(&o["node_path"])
                    .map(label)
                    .collect::<Vec<_>>()
                    .join(" › ");
            } else if e.node != o["node"] {
                e.node = Value::Null;
                e.node_path = "Mixed".into();
            }
            if let Some(classes) = o["classes_allowed"].as_array() {
                let allowed: BTreeSet<_> = classes.iter().map(|v| label(v).to_owned()).collect();
                if let Some(existing) = &mut e.allowed {
                    existing.retain(|c| allowed.contains(c));
                } else {
                    e.allowed = Some(allowed);
                }
            }
            e.fresh |= !o["ifc_class"].is_string();
            e.tagged |= o["ifc_class"].is_string();
            let mut can = BTreeSet::new();
            for f in array(&o["fields"]) {
                let path = &f["path"];
                let k = label(&path[0]);
                if k == "classifications" {
                    if f["owner"] == "Filing node" {
                        e.managed_systems.insert(label(&path[1]).into());
                    }
                    *e.classified
                        .entry((
                            label(&path[1]).into(),
                            label(&path[2]).into(),
                            label(&f["value"]).into(),
                        ))
                        .or_default() += 1;
                }
                if path.as_array().is_none_or(|p| p.len() != 1) {
                    continue;
                }
                if f["editable"] == true {
                    can.insert(k.to_owned());
                }
                let value = label(&f["value"]);
                if k == "predefined_type" && !value.is_empty() && o["editable"] == true {
                    *e.predefined.entry(value.into()).or_default() += 1;
                }
                let v = e.values.entry(k.into()).or_insert_with(|| value.into());
                if *v != value {
                    e.mixed.insert(k.into());
                    v.clear();
                }
            }
            if o["editable"] == true {
                if e.targets.len() == 1 {
                    e.editable = can;
                } else {
                    e.editable.retain(|k| can.contains(k));
                }
            }
            for k in array(&o["systems"]) {
                *e.members.entry(label(k).into()).or_default() += 1;
            }
            for (key, q) in entries(&o["quantities"]) {
                let value = q.get("value").unwrap_or(q).to_string();
                let entry = e
                    .quantities
                    .entry(key.clone())
                    .or_insert_with(|| value.clone());
                if *entry != value {
                    entry.clear();
                }
            }
        }
        for (key, value) in &mut e.quantities {
            if array(&data["records"]).any(|r| r["quantities"].get(key).is_none()) {
                value.clear();
            }
        }
        e.drafts = e.values.clone();
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
                    && self.allowed.as_ref().is_none_or(|list| list.contains(name))
                    && (human_name(name).to_lowercase().contains(&needle)
                        || name.to_lowercase().contains(&needle)
                        || domain.to_lowercase().contains(&needle)))
                .then(|| (name.clone(), domain.to_owned(), fav.contains(name.as_str())))
            })
            .collect();
        rows.sort_by(|a, b| (!a.2, &a.1, &a.0).cmp(&(!b.2, &b.1, &b.0)));
        rows
    }

    fn shown(&self, field: &str) -> &str {
        if self.mixed.contains(field) {
            "Mixed"
        } else {
            self.values
                .get(field)
                .map(String::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("Not set")
        }
    }

    fn choose_class(&mut self, name: String, out: &mut Vec<String>) {
        if self.shown("class") == name {
            return;
        }
        let valid = &self.catalog["classes"][&name]["predefined_types"][&self.schema];
        let cleared: Vec<_> = self
            .predefined
            .iter()
            .filter(|(kind, _)| !array(valid).any(|v| v.as_str() == Some(kind.as_str())))
            .map(|(kind, count)| format!("{kind} (×{count})"))
            .collect();
        if cleared.is_empty() {
            send(
                out,
                "record.fields",
                json!([self.targets, {"class": name}, self.schema]),
            );
        } else {
            self.class_change = Some((name, cleared.join(", ")));
        }
    }

    pub(super) fn ready(&self) -> bool {
        !self.fresh
    }
    pub(super) fn identity(
        &mut self,
        ui: &mut egui::Ui,
        out: &mut Vec<String>,
        layers: &mut Vec<egui::Id>,
    ) {
        kit::prop_row(ui, "Schema", &self.schema);
        if self.count > 1 && self.count != self.targets.len() {
            kit::label(
                ui,
                &format!(
                    "{} editable · {} read-only",
                    self.targets.len(),
                    self.count - self.targets.len()
                ),
            );
        }
        kit::property_control(ui, "IFC class", |ui| {
            let resp = ui
                .add_enabled_ui(self.editable.contains("class"), |ui| {
                    kit::dropdown(
                        ui,
                        "",
                        self.shown("class"),
                        self.picker,
                        ui.available_width(),
                    )
                })
                .inner;
            if resp.clicked() {
                self.picker = !self.picker;
                self.search.clear();
            }
            if !self.picker {
                return;
            }
            let area = egui::Area::new(resp.id.with("picker"))
                .order(egui::Order::Foreground)
                .pivot(egui::Align2::RIGHT_TOP)
                .fixed_pos(resp.rect.left_top() - egui::vec2(tokens::SPACE_S, 0.))
                .show(ui.ctx(), |ui| {
                    kit::menu(ui, tokens::DOCK_W, |ui| {
                        layers.push(ui.layer_id().id);
                        let search = ui.add(kit::search(&mut self.search, "Search IFC classes"));
                        if resp.clicked() {
                            search.request_focus();
                        }
                        let mut description = None;
                        egui::ScrollArea::vertical()
                            .max_height(tokens::CONTROL_H * 10.)
                            .show(ui, |ui| {
                                let mut last = String::new();
                                for (name, domain, fav) in self.classes() {
                                    let group = if fav { "Favourites" } else { &domain };
                                    if last != group {
                                        kit::caption_plain(ui, group);
                                        last = group.into();
                                    }
                                    let human = human_name(&name);
                                    let row = kit::command_row(
                                        ui,
                                        &human,
                                        "",
                                        &name,
                                        true,
                                        self.shown("class") == name,
                                    );
                                    if row.hovered()
                                        || (self.shown("class") == name && description.is_none())
                                    {
                                        let types = &self.catalog["classes"][&name]
                                            ["predefined_types"][&self.schema];
                                        description =
                                            Some(class_description(&name, &domain, types));
                                    }
                                    if row.clicked() {
                                        self.choose_class(name, out);
                                        self.picker = false;
                                    }
                                }
                            });
                        if let Some(text) = description {
                            kit::label(ui, &text);
                        }
                    })
                });
            if ui.input(|i| i.key_pressed(egui::Key::Escape))
                || (area.response.clicked_elsewhere() && !resp.clicked())
            {
                self.picker = false;
            }
        });
        if let Some((class, kinds)) = &self.class_change {
            kit::label(ui, &format!("Changing to {class} clears {kinds}."));
            if kit::quiet(ui, "Change class").clicked() {
                send(
                    out,
                    "record.fields",
                    json!([self.targets,{"class":class},self.schema]),
                );
                self.class_change = None;
            } else if kit::quiet(ui, "Cancel").clicked() {
                self.class_change = None;
            }
        }
        let class = self.values.get("class").cloned().unwrap_or_default();
        for (key, title, options) in [
            (
                "predefined_type",
                "Predefined type",
                &self.catalog["classes"][&class]["predefined_types"][&self.schema],
            ),
            ("type", "Type", &self.catalog["types"]),
        ] {
            if !self.tagged && key == "type" {
                continue;
            }
            let mut choices = vec!["Not set"];
            choices.extend(array(options).filter_map(Value::as_str));
            kit::property_control(ui, title, |ui| {
                let enabled = self.editable.contains(key) && self.ready() && choices.len() > 1;
                ui.add_enabled_ui(enabled, |ui| {
                    let current = if !self.tagged {
                        "Choose a class first"
                    } else {
                        self.shown(key)
                    };
                    let (index, layer) = kit::select(ui, key, current, &choices);
                    layers.extend(layer);
                    if let Some(index) = index {
                        let value = if index == 0 {
                            Value::Null
                        } else {
                            json!(choices[index])
                        };
                        send(
                            out,
                            "record.fields",
                            json!([self.targets,{(key):value},self.schema]),
                        );
                    }
                });
            });
        }
        for (field, title) in [
            ("name", "Name"),
            ("description", "Description"),
            ("object_type", "Object type"),
            ("tag", "Tag"),
        ] {
            if !self.tagged {
                break;
            }
            kit::property_control(ui, title, |ui| {
                let enabled = self.ready() && self.editable.contains(field);
                let hint = if self.mixed.contains(field) {
                    "Mixed"
                } else {
                    "Not set"
                };
                let value = self.drafts.entry(field.into()).or_default();
                let response = ui.add_enabled(
                    enabled,
                    kit::field(value)
                        .hint_text(hint)
                        .desired_width(ui.available_width()),
                );
                if response.lost_focus() {
                    let before = self.values.get(field).map(String::as_str).unwrap_or("");
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        *value = before.into();
                    } else if before != value {
                        send(
                            out,
                            "record.fields",
                            json!([self.targets,{(field):value},self.schema]),
                        );
                    }
                }
            });
        }
        if !self.node_path.is_empty() {
            if self.node.is_null() {
                kit::prop_row(ui, "From node", "Mixed");
            } else if kit::command_row(
                ui,
                &format!("From node: {}", self.node_path),
                "",
                "›",
                true,
                false,
            )
            .on_hover_text(format!("Open node defaults · {}", self.node_path))
            .clicked()
            {
                send(out, "rule.open", self.node.clone());
            }
        }
        if self.fresh {
            kit::label(ui, "Choose an IFC class to add record data.");
        }
        if self.tagged
            && self.editable.contains("class")
            && kit::quiet(ui, "Remove record").clicked()
        {
            send(
                out,
                "record.untag",
                json!([self.targets, null, self.schema]),
            );
        }
    }
    pub(super) fn quantities(&mut self, ui: &mut egui::Ui, out: &mut Vec<String>) {
        for (name, value) in &mut self.quantities {
            kit::property_control(ui, name, |ui| {
                let response = ui.add(
                    kit::field(value)
                        .hint_text("Mixed")
                        .desired_width(ui.available_width()),
                );
                if response.lost_focus()
                    && ui
                        .input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Tab))
                {
                    if let Ok(n) = value.parse::<f64>() {
                        if n.is_finite() {
                            send(
                                out,
                                "record.quantity",
                                json!([self.targets, [name, n], self.schema]),
                            );
                        }
                    }
                }
            });
        }
        ui.add(kit::field(&mut self.quantity_name).hint_text("Quantity name"));
        ui.add(kit::field(&mut self.quantity_value).hint_text("Manual value"));
        if kit::quiet(ui, "Add quantity").clicked() {
            if let Some(n) = self
                .quantity_value
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite() && !self.quantity_name.trim().is_empty())
            {
                send(
                    out,
                    "record.quantity",
                    json!([self.targets, [self.quantity_name.trim(), n], self.schema]),
                );
            } else {
                self.error = "Enter a quantity name and finite number".into();
            }
        }
        if !self.error.is_empty() {
            kit::label(ui, &self.error);
        }
    }
    pub(super) fn classifications(&mut self, ui: &mut egui::Ui, out: &mut Vec<String>) {
        for ((system, code, title), count) in &self.classified {
            ui.horizontal_wrapped(|ui| {
                let mixed = if *count != self.count {
                    " · Mixed"
                } else {
                    ""
                };
                kit::label(ui, &format!("{system} · {code} {title}{mixed}"));
                if !self.managed_systems.contains(system) && kit::quiet(ui, "Remove").clicked() {
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
        ui.add(kit::field(&mut self.code).hint_text("Code"));
        ui.add(kit::field(&mut self.title).hint_text("Name"));
        if ui
            .add_enabled_ui(
                !self.managed_systems.contains(&self.classification)
                    && !self.classification.is_empty()
                    && !self.code.trim().is_empty(),
                |ui| kit::quiet(ui, "Add classification"),
            )
            .inner
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
        for (key, v) in entries(&self.catalog["systems"]) {
            let count = self.members.get(key).copied().unwrap_or(0);
            let title = v["name"].as_str().unwrap_or(key);
            let mut checked = count == self.count;
            let label = if count > 0 && !checked {
                format!("{title} · Mixed")
            } else {
                title.into()
            };
            if kit::toggle(ui, &mut checked, &label).changed() {
                send(
                    out,
                    "record.system",
                    json!([self.targets, [key, checked], self.schema]),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incompatible_types_require_confirmation() {
        let mut editor = Editor::default();
        editor.predefined.insert("PILASTER".into(), 2);
        let mut out = Vec::new();
        editor.choose_class("IfcFurniture".into(), &mut out);
        assert!(out.is_empty());
        assert_eq!(
            editor.class_change,
            Some(("IfcFurniture".into(), "PILASTER (×2)".into()))
        );
    }

    #[test]
    fn unset_shared_and_mixed_classes_are_distinct() {
        let group = |class: &str| {
            json!({"uid":"group","editable":true,"fields":[
            {"path":["class"],"value":class,"editable":true}]})
        };
        for (records, display) in [
            (vec![group("")], "Not set"),
            (vec![group("IfcBeam"), group("IfcBeam")], "IfcBeam"),
            (vec![group(""), group("IfcBeam"), group("")], "Mixed"),
        ] {
            assert_eq!(
                Editor::load(&json!({"records":records})).shown("class"),
                display
            );
        }
    }
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
