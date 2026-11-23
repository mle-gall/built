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
    // Tag keys, not row labels: two filing nodes can refer to one tag.
    selected: BTreeMap<String, (String, Vec<String>)>,
    patch: BTreeMap<String, String>,
}

impl StyleEditor {
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
        let mut hatch = vec![String::new()];
        hatch.extend(kit::HATCH_KEYS.lines().map(str::to_owned));
        let weights: Vec<String> = [
            "auto", "0.13", "0.18", "0.25", "0.35", "0.50", "0.70", "1.00",
        ]
        .map(str::to_owned)
        .into();
        for (index, field, name, choices) in [
            (6, "cut_weight", "Cut weight (mm)", weights.clone()),
            (0, "color", "Colour", vec![]),
            (4, "hatch", "Hatch", hatch),
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
                if field == "color" {
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
