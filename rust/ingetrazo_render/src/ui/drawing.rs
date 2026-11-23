// SPDX-License-Identifier: GPL-3.0-or-later
//! Drawing controls for the focused Project node.
use super::{kit, parse_hex, Node, SWATCH_FALLBACK};

pub(super) enum DraggedRow {
    Node(String),
    Placement { key: String, selected: bool },
}

impl DraggedRow {
    fn action(&self, parent: &str) -> Option<String> {
        match self {
            Self::Node(key) if key.strip_prefix("n:") != Some(parent) => {
                Some(format!("parent\u{1f}{key}\u{1f}{parent}"))
            }
            Self::Placement { selected: true, .. } if !parent.is_empty() => {
                Some(format!("filehere\u{1f}n:{parent}"))
            }
            Self::Placement { key, .. } if !parent.is_empty() => {
                Some(format!("fileitem\u{1f}{key}\u{1f}{parent}"))
            }
            _ => None,
        }
    }
}

pub(super) fn drop_on(
    ui: &egui::Ui,
    target: &egui::Response,
    parent: &str,
    actions: &mut Vec<String>,
) {
    if target
        .dnd_hover_payload::<DraggedRow>()
        .and_then(|row| row.action(parent))
        .is_some()
    {
        kit::paint_focus(ui, target.rect, super::R_ROW);
    }
    if let Some(action) = target
        .dnd_release_payload::<DraggedRow>()
        .and_then(|row| row.action(parent))
    {
        actions.push(action);
    }
}

use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct StyleEditor {
    pub owner: usize,
    pub lines: Vec<String>,
    pattern_source: String,
    patterns: BTreeMap<String, serde_json::Value>,
    hatch_filter: String,
    samples: BTreeMap<String, Option<kit::PaperMarks>>,
    // Tag keys, not row labels: two filing nodes can refer to one tag.
    selected: BTreeMap<String, (String, Vec<String>)>,
    patch: BTreeMap<String, String>,
}

impl StyleEditor {
    pub fn set_patterns(&mut self, source: String) {
        if self.pattern_source != source {
            self.patterns = serde_json::from_str(&source).unwrap_or_default();
            self.pattern_source = source;
            self.samples.clear();
        }
    }

    pub fn selected(&self, n: &Node) -> bool {
        n.cells
            .get(5)
            .is_some_and(|key| self.selected.contains_key(key))
    }

    pub fn pick(&mut self, n: &Node, extend: bool) {
        let Some(key) = n.cells.get(5) else { return };
        self.patch.clear();
        if !extend {
            self.selected.clear();
        }
        if extend && self.selected.remove(key).is_some() {
            return;
        }
        self.selected
            .insert(key.clone(), (n.name.clone(), n.cells.clone()));
    }

    pub fn refresh(&mut self, nodes: &[Node]) {
        for n in nodes {
            if let Some(entry) = n.cells.get(5).and_then(|key| self.selected.get_mut(key)) {
                entry.1.clone_from(&n.cells);
            }
        }
    }

    pub fn draw(&mut self, ui: &mut egui::Ui, actions: &mut Vec<String>) -> bool {
        if self.selected.is_empty() {
            return false;
        }
        let fields_height =
            (ui.available_height() - super::BUTTON_H - super::SPACE_S).max(super::CONTROL_H);
        egui::ScrollArea::vertical()
            .id_salt("drawing fields")
            .max_height(fields_height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = super::SPACE_XS;
                self.fields(ui);
                if kit::quiet(ui, "Import hatch patterns…").clicked() {
                    actions.push("importpat\u{1f}".into());
                }
            });
        self.apply_row(ui, actions);
        true
    }

    fn fields(&mut self, ui: &mut egui::Ui) {
        ui.strong(format!(
            "Drawing style · {} {}",
            self.selected.len(),
            if self.selected.len() == 1 {
                "tag"
            } else {
                "tags"
            }
        ));
        ui.label(
            self.selected
                .values()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        );
        let weights: Vec<String> = [
            "auto", "0.13", "0.18", "0.25", "0.35", "0.50", "0.70", "1.00",
        ]
        .map(str::to_owned)
        .into();
        for (index, field, name, choices) in [
            (6, "cut_weight", "Cut weight (mm)", weights.clone()),
            (0, "color", "Colour", vec![]),
            (4, "hatch", "Hatch", vec![]),
            (8, "hatch_angle", "Hatch angle (°)", vec![]),
            (9, "hatch_scale", "Hatch scale", vec![]),
            (2, "weight", "Projection weight (mm)", weights),
            (1, "linetype", "Line type", self.lines.clone()),
            (3, "print", "Print", vec!["0".into(), "1".into()]),
            (7, "lock", "Lock", vec!["0".into(), "1".into()]),
        ] {
            let mut values = self
                .selected
                .values()
                .map(|(_, cells)| cells.get(index).map_or("", String::as_str));
            let first = values.next().unwrap_or("");
            let mixed = values.any(|v| v != first);
            if !mixed && self.patch.get(field).is_some_and(|value| value == first) {
                self.patch.remove(field);
            }
            let current = self
                .patch
                .get(field)
                .map(String::as_str)
                .or((!mixed).then_some(first))
                .map(str::to_owned);
            kit::property_control(ui, name, |ui| {
                if field == "hatch" {
                    if let Some(chosen) = self.pick_hatch(ui, current.as_deref()) {
                        self.patch.insert(field.into(), chosen);
                    }
                } else if matches!(field, "hatch_angle" | "hatch_scale") {
                    let angle = field == "hatch_angle";
                    let mut value = current
                        .as_deref()
                        .and_then(|v| v.parse::<f64>().ok())
                        .unwrap_or(if angle { 0. } else { 1. });
                    if ui
                        .add(
                            kit::number(&mut value, if angle { "°" } else { "×" })
                                .range(if angle { -360.0..=360.0 } else { 0.05..=20.0 }),
                        )
                        .changed()
                    {
                        self.patch.insert(field.into(), value.to_string());
                    }
                    if current.is_none() {
                        kit::caption_plain(ui, "Mixed");
                    }
                } else if field == "color" {
                    let mut color = current
                        .as_deref()
                        .and_then(parse_hex)
                        .unwrap_or(SWATCH_FALLBACK);
                    if kit::swatch(ui, &mut color).changed() {
                        self.patch.insert(
                            field.into(),
                            format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b()),
                        );
                    }
                    if current.is_none() {
                        ui.label("Mixed");
                    }
                } else {
                    let label = |s: &str| match (field, s) {
                        ("print" | "lock", "0") => "Off".into(),
                        ("print" | "lock", "1") => "On".into(),
                        (_, "") => "Automatic".into(),
                        _ => s.replace('_', " "),
                    };
                    egui::ComboBox::from_id_salt(("style", field))
                        .width(ui.available_width().min(168.))
                        .selected_text(current.as_deref().map(label).unwrap_or("Mixed".into()))
                        .show_ui(ui, |ui| {
                            for value in choices {
                                if ui
                                    .selectable_label(
                                        current.as_ref() == Some(&value),
                                        label(&value),
                                    )
                                    .clicked()
                                {
                                    self.patch.insert(field.into(), value);
                                }
                            }
                        });
                }
            });
        }
    }

    fn pick_hatch(&mut self, ui: &mut egui::Ui, current: Option<&str>) -> Option<String> {
        let label = |key: &str| {
            if key.is_empty() {
                "Automatic".into()
            } else {
                key.trim_start_matches("pat:").replace('_', " ")
            }
        };
        let mut choice = None;
        egui::ComboBox::from_id_salt("drawing hatch")
            .width(ui.available_width().min(168.))
            .selected_text(current.map(label).unwrap_or_else(|| "Mixed".into()))
            .show_ui(ui, |ui| {
                ui.add(kit::search(&mut self.hatch_filter, "Find a hatch"));
                let query = self.hatch_filter.trim().to_lowercase();
                let imported = self
                    .patterns
                    .iter()
                    .map(|(k, v)| (format!("pat:{k}"), Some(v)));
                let standard = std::iter::once("")
                    .chain(kit::HATCH_KEYS.lines())
                    .map(|k| (k.to_owned(), None));
                for (key, pattern) in imported.chain(standard) {
                    let text = label(&key);
                    let description = pattern.and_then(|p| p["label"].as_str()).unwrap_or("");
                    if !text.to_lowercase().contains(&query)
                        && !description.to_lowercase().contains(&query)
                    {
                        continue;
                    }
                    let (rect, row) = kit::list_row(ui, super::ROW_H, current == Some(&key));
                    if ui.is_rect_visible(rect) {
                        let sample = egui::Rect::from_center_size(
                            rect.left_center() + egui::vec2(22., 0.),
                            egui::vec2(36., 14.),
                        );
                        let marks = pattern.and_then(|p| {
                            self.samples
                                .entry(key.clone())
                                .or_insert_with(|| kit::imported_hatch(&p["families"]))
                                .as_ref()
                        });
                        kit::hatch_sample(ui, sample, &key, marks);
                        ui.painter().text(
                            rect.left_center() + egui::vec2(46., 0.),
                            egui::Align2::LEFT_CENTER,
                            text,
                            super::body(),
                            super::INK,
                        );
                    }
                    if row.on_hover_text(description).clicked() {
                        choice = Some(key);
                        self.hatch_filter.clear();
                        ui.close_menu();
                    }
                }
            });
        choice
    }

    fn apply_row(&mut self, ui: &mut egui::Ui, actions: &mut Vec<String>) {
        ui.add_space(super::SPACE_XS);
        let pending = !self.patch.is_empty();
        let count = self.selected.len();
        let label = format!(
            "Apply to {count} {}",
            if count == 1 { "tag" } else { "tags" }
        );
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = super::SPACE_S;
            let apply = ui.add_enabled_ui(pending, |ui| kit::sheet_button(ui, &label, "", 1));
            if apply.inner {
                let tags: Vec<_> = self.selected.keys().collect();
                actions.push(format!(
                    "drawingpatch\u{1f}{}",
                    serde_json::json!([tags, self.patch])
                ));
                self.patch.clear();
            }
            if !pending {
                kit::caption_plain(ui, "Change a field to apply.");
            }
        });
    }
}
