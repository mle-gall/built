//! Inspector drafts for document line types and tag folders.
use super::{kit, tokens::*};
use pyo3::prelude::*;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Catalog {
    lines: BTreeMap<String, Value>,
    folders: BTreeMap<String, String>,
    labels: BTreeMap<String, String>,
    parents: BTreeMap<String, Option<String>>,
    pub folder: String,
    pub new_hidden: bool,
    name: String,
    parent: String,
    line: Option<Line>,
    external_tags: Vec<String>,
    standard_inks: BTreeMap<String, [f64; 3]>,
    standard: String,
    revision: Option<u64>,
}

struct Line {
    original: String,
    name: String,
    pieces: String,
    text: String,
    size: f64,
    color: Option<[f64; 3]>,
}

impl Line {
    fn new(name: &str, value: &Value) -> Self {
        let pieces = value
            .get("pieces")
            .or_else(|| value.get("pattern"))
            .and_then(Value::as_array)
            .map(|v| {
                v.iter()
                    .map(Value::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|| "6, -2".into());
        let color = serde_json::from_value(value["color"].clone()).unwrap_or(None);
        Self {
            original: name.into(),
            name: name.into(),
            pieces,
            text: value["text"].as_str().unwrap_or("").into(),
            size: value["text_size"].as_f64().unwrap_or(2.5),
            color,
        }
    }

    fn numbers(&self) -> Option<Vec<f64>> {
        let v: Vec<f64> = self
            .pieces
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .map(str::parse)
            .collect::<Result<_, _>>()
            .ok()?;
        (v.len() <= 32
            && v.iter().all(|n| n.is_finite() && n.abs() <= 1000.)
            && v.iter().any(|n| *n < 0.)
            && v.iter().any(|n| *n >= 0.)
            && v.iter().map(|n| n.abs()).sum::<f64>() >= 0.05)
            .then_some(v)
    }
    fn append(&mut self, value: &str) {
        if !self.pieces.trim().is_empty() {
            self.pieces.push_str(", ");
        }
        self.pieces.push_str(value);
    }
    fn show(&mut self, ui: &mut egui::Ui, tags: &[String], actions: &mut Vec<String>) -> bool {
        ui.strong(if self.original.is_empty() {
            "New line type"
        } else {
            "Edit line type"
        });
        for (label, raw) in [
            ("Name", &mut self.name),
            ("Dash / gap (mm)", &mut self.pieces),
            ("Text", &mut self.text),
        ] {
            kit::property_control(ui, label, |ui| {
                ui.add(kit::field(raw).desired_width(ui.available_width()));
            });
        }
        ui.horizontal_wrapped(|ui| {
            for (label, value) in [("+ Dash", "3"), ("+ Gap", "-1.5"), ("+ Dot", "0")] {
                if kit::quiet(ui, label).clicked() {
                    self.append(value);
                }
            }
        });
        if !self.text.is_empty() {
            kit::property_control(ui, "Text height", |ui| {
                ui.add(kit::number(&mut self.size, " mm").range(0.5..=20.));
            });
        }
        kit::property_control(ui, "Line colour", |ui| {
            line_ink(ui, &mut self.color);
        });
        let numbers = self.numbers();
        let valid = numbers.is_some()
            && !self.name.trim().is_empty()
            && self.name.trim().chars().count() <= 64
            && self.text.chars().count() <= 24;
        if let Some(ref pieces) = numbers {
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), CONTROL_H),
                egui::Sense::hover(),
            );
            kit::line_type_preview(ui, rect, pieces, &self.text, self.size, ink(self.color));
        } else {
            kit::caption_plain(ui, "Enter a dash and a negative gap in millimetres.");
        }
        let mut cancel = false;
        ui.horizontal_wrapped(|ui|{
            if ui.add_enabled_ui(valid,|ui|kit::sheet_button(ui,"Save line type","",1)).inner {
                let rgb=self.color;
                let data=json!({"original":self.original,"name":self.name,"pieces":numbers.unwrap_or_default(),"text":self.text,"text_size":self.size,"color":rgb,"tags":if self.original.is_empty(){tags}else{&[]}});
                actions.push(format!("editline\u{1f}save\u{1f}{data}"));
            }
            if !self.original.is_empty() && kit::quiet(ui,"Delete line type").clicked(){
                actions.push(format!("editline\u{1f}delete\u{1f}{}",json!({"original":self.original})));
            }
            cancel=kit::quiet(ui,"Close editor").clicked();
        });
        cancel
    }
}

fn ink(rgb: Option<[f64; 3]>) -> egui::Color32 {
    rgb.map(|c| document_rgb(c.map(|v| (v.clamp(0., 1.) * 255.).round() as u8)))
        .unwrap_or(INK)
}
fn line_ink(ui: &mut egui::Ui, rgb: &mut Option<[f64; 3]>) {
    let presets = [
        ("By tag", None),
        ("Black", Some([0., 0., 0.])),
        ("Yellow", Some([0.85, 0.65, 0.])),
        ("Blue", Some([0., 0.40, 0.85])),
        ("Red", Some([0.85, 0.10, 0.10])),
        ("Green", Some([0., 0.55, 0.25])),
    ];
    let name = presets
        .iter()
        .find(|(_, c)| c == rgb)
        .map_or("Custom", |(label, _)| *label);
    egui::ComboBox::from_id_salt("line ink")
        .selected_text(name)
        .width(72.)
        .show_ui(ui, |ui| {
            for (label, c) in presets {
                ui.selectable_value(rgb, c, label);
            }
        });
    let mut c = ink(*rgb);
    if kit::swatch(ui, &mut c).changed() {
        *rgb = Some([c.r(), c.g(), c.b()].map(|v| f64::from(v) / 255.));
    }
}

impl Catalog {
    pub fn open_line(&mut self, name: &str, tags: Vec<String>) -> bool {
        if !name.is_empty() && !self.lines.contains_key(name) {
            return false;
        }
        self.line = Some(Line::new(
            name,
            self.lines.get(name).unwrap_or(&Value::Null),
        ));
        self.external_tags = tags;
        true
    }
    pub fn close_line(&mut self) {
        self.line = None;
        self.external_tags.clear();
    }
    pub fn editing_line(&self) -> bool {
        self.line.is_some()
    }

    pub fn sync(
        &mut self,
        scene: &Bound<'_, PyAny>,
        selected: &mut BTreeMap<String, (String, Vec<String>)>,
        selected_folders: &mut BTreeSet<String>,
    ) -> PyResult<()> {
        let version = scene.getattr("version")?.extract()?;
        if self.revision == Some(version) {
            return Ok(());
        }
        let py = scene.py();
        self.new_hidden = crate::tree_grouping::tag_controls::new_hidden(py)?;
        let entries: String = py
            .import("json")?
            .call_method1("dumps", (scene.getattr("line_types")?,))?
            .extract()?;
        self.lines = serde_json::from_str(&entries)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        self.standard_inks = scene.getattr("linetype_colors")?.extract()?;
        let labels: BTreeMap<String, String> = scene.getattr("layer_folder_names")?.extract()?;
        let parents: BTreeMap<String, Option<String>> =
            scene.getattr("folder_parents")?.extract()?;
        let mut keys: BTreeSet<String> = scene
            .getattr("layer_folders")?
            .extract::<Vec<_>>()?
            .into_iter()
            .collect();
        let mut tags = BTreeSet::new();
        for layer in scene.getattr("layers")?.try_iter()? {
            let layer = layer?;
            if !selected.is_empty() {
                tags.insert(layer.getattr("key")?.extract::<String>()?);
            }
            if let Some(key) = layer.getattr("folder")?.extract::<Option<String>>()? {
                keys.insert(key);
            }
        }
        selected.retain(|key, _| tags.contains(key));
        selected_folders.retain(|key| keys.contains(key));
        self.folders = keys
            .iter()
            .map(|key| {
                let mut parts = vec![];
                let mut seen = BTreeSet::new();
                let mut at = key;
                while seen.insert(at) {
                    parts.push(labels.get(at).unwrap_or(at).as_str());
                    let Some(next) = parents
                        .get(at)
                        .and_then(Option::as_ref)
                        .filter(|p| keys.contains(*p))
                    else {
                        break;
                    };
                    at = next;
                }
                parts.reverse();
                (key.clone(), parts.join(" / "))
            })
            .collect();
        self.parents = parents;
        self.labels = labels;
        if !self.folder.is_empty() && !self.folders.contains_key(&self.folder) {
            self.select_folder("");
        }
        self.revision = Some(version);
        Ok(())
    }
    pub fn select_folder(&mut self, key: &str) {
        self.folder = key.into();
        self.name = self.labels.get(key).map_or(key, String::as_str).to_owned();
        self.parent = self.parents.get(key).cloned().flatten().unwrap_or_default();
    }
    pub fn applied(&mut self, verb: &str, key: &str) {
        if verb == "editline" {
            self.line = None;
        }
        if verb == "foldernew" {
            self.select_folder(key);
        }
    }
    fn folder_choice(
        ui: &mut egui::Ui,
        id: &str,
        current: &mut String,
        rows: &BTreeMap<String, String>,
    ) -> bool {
        let mut changed = false;
        egui::ComboBox::from_id_salt(id)
            .selected_text(rows.get(current).map_or("Root", String::as_str))
            .show_ui(ui, |ui| {
                changed |= ui
                    .selectable_value(current, String::new(), "Root")
                    .changed();
                for (key, path) in rows {
                    changed |= ui.selectable_value(current, key.clone(), path).changed();
                }
            });
        changed
    }
    pub fn folders(&mut self, ui: &mut egui::Ui, tags: &[String], actions: &mut Vec<String>) {
        kit::property_control(ui, "Folder", |ui| {
            if Self::folder_choice(ui, "folder selection", &mut self.folder, &self.folders) {
                self.select_folder(&self.folder.clone());
            }
        });
        kit::property_control(ui, "Name", |ui| {
            ui.add(kit::field(&mut self.name).desired_width(ui.available_width()));
        });
        kit::property_control(ui, "Destination", |ui| {
            Self::folder_choice(ui, "folder destination", &mut self.parent, &self.folders);
        });
        let mut send = |op: &str| {
            let data =
                json!({"folder":self.folder,"parent":self.parent,"name":self.name,"tags":tags});
            actions.push(format!("tagfolder\u{1f}{op}\u{1f}{data}"));
        };
        ui.horizontal_wrapped(|ui| {
            if !self.name.trim().is_empty() && kit::quiet(ui, "New in destination").clicked() {
                send("new");
            }
            if !self.folder.is_empty() {
                if !self.name.trim().is_empty() && kit::quiet(ui, "Rename").clicked() {
                    send("rename");
                }
                if kit::quiet(ui, "Move folder").clicked() {
                    send("move");
                }
                if kit::quiet(ui, "Remove folder")
                    .on_hover_text("Move its contents to the parent folder")
                    .clicked()
                {
                    send("delete");
                }
            }
            if !tags.is_empty() && kit::quiet(ui, "Move selected tags").clicked() {
                send("file");
            }
        });
    }
    pub fn lines(
        &mut self,
        ui: &mut egui::Ui,
        tags: &[String],
        standard: &[String],
        actions: &mut Vec<String>,
    ) {
        if let Some(editor) = self.line.as_mut() {
            if editor.show(
                ui,
                if self.external_tags.is_empty() {
                    tags
                } else {
                    &self.external_tags
                },
                actions,
            ) {
                self.line = None;
            }
            return;
        }
        if kit::quiet(ui, "New line type…").clicked() {
            self.external_tags.clear();
            self.line = Some(Line::new("", &Value::Null));
        }
        for (name, value) in &self.lines {
            if kit::quiet(ui, name)
                .on_hover_text("Edit line type")
                .clicked()
            {
                self.line = Some(Line::new(name, value));
            }
        }
        kit::property_control(ui, "Standard line ink", |ui| {
            if self.standard.is_empty() {
                self.standard = "continuous".into();
            }
            egui::ComboBox::from_id_salt("standard line")
                .selected_text(&self.standard)
                .show_ui(ui, |ui| {
                    for key in standard.iter().filter(|k| !k.starts_with("lin:")) {
                        ui.selectable_value(&mut self.standard, key.clone(), key);
                    }
                });
        });
        kit::property_control(ui, "Ink", |ui| {
            let before = self.standard_inks.get(&self.standard).copied();
            let mut current = before;
            line_ink(ui, &mut current);
            if before != current {
                actions.push(format!(
                    "editline\u{1f}ink\u{1f}{}",
                    json!({"name":self.standard,"color":current})
                ));
            }
        });
        ui.horizontal_wrapped(|ui| {
            if kit::quiet(ui, "Import LIN…").clicked() {
                actions.push("importlin\u{1f}".into());
            }
            if kit::quiet(ui, "Export LIN…").clicked() {
                actions.push("exportlin\u{1f}".into());
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_previews_typed_dashes_and_refuses_patterns_without_gaps() {
        let mut line = Line::new("Borde", &json!({"pattern":[6.0,-1.5]}));
        assert_eq!(line.numbers().unwrap(), [6., -1.5]);
        line.pieces = "5, 5".into();
        assert_eq!(line.numbers(), None);
        line.pieces.clear();
        for piece in ["3", "-1.5", "0"] {
            line.append(piece);
        }
        assert_eq!(line.numbers().unwrap(), [3., -1.5, 0.]);
        let ctx = crate::ui::test_context();
        let output = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                line.show(ui, &[], &mut vec![]);
            });
        });
        assert!(!output.shapes.is_empty());
    }

    #[test]
    fn editor_restores_original_pieces_words_and_height() {
        let v = json!({"pieces":[8,-2],"pattern":[8,-10],"text":"GAS","text_size":2.0});
        let line = Line::new("Gas", &v);
        assert_eq!(line.original, "Gas");
        assert_eq!(line.numbers().unwrap(), [8., -2.]);
        assert_eq!((line.text.as_str(), line.size), ("GAS", 2.));
    }

    #[test]
    fn line_editor_keeps_by_tag_standard_and_custom_colours_without_rounding() {
        assert_eq!(Line::new("", &Value::Null).color, None);
        for rgb in [[0., 0.4, 0.85], [0.85, 0.1, 0.1], [0.3, 0.2, 0.1]] {
            assert_eq!(Line::new("Line", &json!({"color":rgb})).color, Some(rgb));
        }
    }
}
