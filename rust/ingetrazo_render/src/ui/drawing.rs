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
            Self::Placement { key, .. } if parent.is_empty() => Some(format!("root\u{1f}{key}")),
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

pub(super) fn drop_on_occurrence(
    ui: &egui::Ui,
    target: &egui::Response,
    key: &str,
    actions: &mut Vec<String>,
) {
    let Some(payload) = target.dnd_hover_payload::<DraggedRow>() else {
        return;
    };
    let DraggedRow::Placement { key: source, .. } = payload.as_ref() else {
        return;
    };
    if source == key {
        return;
    }
    let beside = ui.input(|i| {
        i.pointer
            .hover_pos()
            .is_some_and(|p| p.y < target.rect.top() + target.rect.height() * 0.2)
    });
    if beside {
        ui.painter().hline(
            target.rect.x_range(),
            target.rect.top(),
            egui::Stroke::new(2.0_f32, super::ACCENT),
        );
    } else {
        kit::paint_focus(ui, target.rect, super::R_ROW);
    }
    if target.dnd_release_payload::<DraggedRow>().is_some() {
        let verb = if beside { "beside" } else { "into" };
        actions.push(format!("{verb}\u{1f}{source}\u{1f}{key}"));
    }
}

use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct StyleEditor {
    pub owner: usize,
    pub catalog: super::drawing_catalog::Catalog,
    folder_focus: bool,
    pub hatches: super::drawing_hatches::Hatches,
    cuts: Option<(
        super::drawing_cuts::CutSpec,
        Option<super::drawing_cuts::Cuts>,
    )>,
    preview: Option<(Vec<String>, Option<kit::PaperMarks>)>,
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

    pub fn state_action(&self, n: &Node, lock: bool) -> Option<String> {
        if !self.selected(n) { return None; }
        let tags: Vec<_> = self.selected.keys().collect();
        let edit = if lock {
            serde_json::json!({"tags":tags, "locked": n.lock == 0})
        } else {
            serde_json::json!({"tags":tags, "visible": n.eye == 0})
        };
        Some(format!("tagstate\u{1f}{edit}"))
    }

    pub fn pick(&mut self, n: &Node, extend: bool) {
        let Some(key) = n.cells.get(5) else { return };
        self.patch.clear();
        self.catalog.close_line();
        self.folder_focus = false;
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

    pub fn applied(&mut self, verb: &str, key: &str) {
        if verb == "drawingpatch" {
            self.patch.clear();
        }
        if verb == "documentpiece" {
            self.patch.remove("brick");
        }
        self.catalog.applied(verb, key);
    }
    pub fn clear_focus(&mut self) {
        self.selected.clear();
        self.patch.clear();
        self.folder_focus = false;
        self.catalog.close_line();
    }
    pub fn pick_folder(&mut self, key: &str) {
        self.catalog.close_line();
        self.selected.clear();
        self.patch.clear();
        self.folder_focus = true;
        self.catalog.select_folder(key);
    }

    pub fn active(&self) -> bool {
        self.folder_focus || self.catalog.editing_line() || !self.selected.is_empty()
    }

    pub fn draw(&mut self, ui: &mut egui::Ui, actions: &mut Vec<String>) -> bool {
        let tags: Vec<_> = self.selected.keys().cloned().collect();
        let fields_height =
            (ui.available_height() - super::BUTTON_H - super::SPACE_S).max(super::CONTROL_H);
        egui::ScrollArea::vertical()
            .id_salt("drawing fields")
            .max_height(fields_height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = super::SPACE_XS;
                if self.catalog.editing_line() {
                    self.catalog.lines(ui, &tags, &self.lines, actions);
                    return;
                }
                if self.folder_focus {
                    self.catalog.folders(ui, &tags, actions);
                }
                if !tags.is_empty() {
                    self.fields(ui, actions);
                }
                if !self.folder_focus {
                    kit::disclosure(ui, "tag folders", "Tag folders", false, |ui| {
                        self.catalog.folders(ui, &tags, actions)
                    });
                }
                kit::disclosure(
                    ui,
                    "document line types",
                    "Document line types",
                    false,
                    |ui| self.catalog.lines(ui, &tags, &self.lines, actions),
                );
                if kit::quiet(ui, "Export hatch patterns…").clicked() {
                    actions.push("exportpat\u{1f}".into());
                }
                if kit::quiet(ui, "Import hatch patterns…").clicked() {
                    actions.push("importpat\u{1f}".into());
                }
            });
        if !tags.is_empty() && !self.catalog.editing_line() {
            self.apply_row(ui, actions);
        }
        true
    }

    fn committed(&self, index: usize, field: &str) -> Option<String> {
        let mut values = self.selected.values().map(|(_, cells)| {
            if field == "brick"
                && self.patch.get("hatch").is_some_and(|h| {
                    self.hatches.kinds.get(h)
                        != cells.get(4).and_then(|old| self.hatches.kinds.get(old))
                })
            {
                return "";
            }
            cells.get(index).map_or("", String::as_str)
        });
        let first = values.next()?;
        values.all(|v| v == first).then(|| first.to_owned())
    }

    fn common(&self, index: usize, field: &str) -> Option<String> {
        self.patch
            .get(field)
            .cloned()
            .or_else(|| self.committed(index, field))
    }

    fn fields(&mut self, ui: &mut egui::Ui, actions: &mut Vec<String>) {
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
            self.field(ui, index, field, name, choices, actions);
        }
        kit::disclosure(ui, "hatch detail", "Hatch details", false, |ui| {
            let hatch = self.common(4, "hatch").unwrap_or_default();
            self.preview(ui, &hatch);
            if !hatch.is_empty() {
                let pinned = self.hatches.favourites.contains(&hatch);
                if kit::quiet(ui, if pinned { "Unpin hatch" } else { "Pin hatch" }).clicked() {
                    actions.push(format!("hatchpref\u{1f}pin\u{1f}{hatch}"));
                }
            }
            for (index, field, name, choices) in [
                (11, "hatch_ground", "Ground", vec![]),
                (12, "hatch_color", "Ink", vec![]),
                (13, "hatch_on_paper", "Scale with", vec!["", "0", "1"]),
                (14, "hatch_offset", "Start X, Y (mm)", vec![]),
                (15, "hatch_cross", "Cross cut grain", vec!["0", "1"]),
                (16, "joints", "Joints", vec!["", "single", "double", "none"]),
                (17, "mortar", "Mortar", vec!["0", "1"]),
                (18, "brick", "Piece size (mm)", vec![]),
                (19, "opacity", "Opacity", vec![]),
            ] {
                let relevant = match field {
                    "hatch_offset" => self.hatches.placed.contains(&hatch),
                    "hatch_cross" => self.hatches.wood.contains(&hatch),
                    "joints" | "mortar" => self.hatches.coursed.contains(&hatch),
                    "brick" => self.hatches.kinds.contains_key(&hatch),
                    _ => true,
                };
                if !relevant {
                    continue;
                }
                if field == "brick" {
                    let kind = &self.hatches.kinds[&hatch];
                    let current = self.common(index, field);
                    if let Some(size) = self.hatches.piece(
                        ui,
                        kind,
                        current.as_deref(),
                        !self.patch.contains_key("hatch"),
                        actions,
                    ) {
                        self.patch.insert("brick".into(), size);
                    }
                }
                self.field(
                    ui,
                    index,
                    field,
                    name,
                    choices.into_iter().map(str::to_owned).collect(),
                    actions,
                );
            }
        });
    }

    fn field(
        &mut self,
        ui: &mut egui::Ui,
        index: usize,
        field: &str,
        name: &str,
        choices: Vec<String>,
        actions: &mut Vec<String>,
    ) {
        let persisted = self.committed(index, field);
        if self
            .patch
            .get(field)
            .is_some_and(|value| Some(value) == persisted.as_ref())
        {
            self.patch.remove(field);
        }
        let current = self.patch.get(field).cloned().or(persisted);
        kit::property_control(ui, name, |ui| {
            if field == "hatch" {
                if let Some(chosen) = self.pick_hatch(ui, current.as_deref()) {
                    if !chosen.is_empty() {
                        actions.push(format!("hatchpref\u{1f}recent\u{1f}{chosen}"));
                    }
                    if self.hatches.kinds.get(&chosen)
                        != current.as_ref().and_then(|s| self.hatches.kinds.get(s))
                    {
                        self.patch.remove("brick");
                    }
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
                        kit::number(&mut value, if angle { "°" } else { "×" }).range(if angle {
                            -360.0..=360.0
                        } else {
                            0.05..=20.0
                        }),
                    )
                    .changed()
                {
                    self.patch.insert(field.into(), value.to_string());
                }
                if current.is_none() {
                    kit::caption_plain(ui, "Mixed");
                }
            } else if matches!(field, "brick" | "hatch_offset") {
                let mut text = current.clone().unwrap_or_default();
                let hint = if field == "brick" {
                    "L, W, H, bed, head"
                } else {
                    "X, Y"
                };
                if ui
                    .add(
                        kit::field(&mut text)
                            .hint_text(hint)
                            .desired_width(ui.available_width()),
                    )
                    .changed()
                {
                    self.patch.insert(field.into(), text);
                }
            } else if field == "opacity" {
                let mut n = current
                    .as_deref()
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(1.);
                if ui.add(kit::number(&mut n, "").range(0.1..=1.)).changed() {
                    self.patch.insert(field.into(), n.to_string());
                }
            } else if matches!(field, "color" | "hatch_ground" | "hatch_color") {
                if field != "color" {
                    let mode = match current.as_deref() {
                        None => "Mixed",
                        Some("layer") => "By tag",
                        Some("") => "Automatic",
                        _ => "Custom",
                    };
                    egui::ComboBox::from_id_salt(field)
                        .width(82.)
                        .selected_text(mode)
                        .show_ui(ui, |ui| {
                            for (label, value) in [
                                ("Automatic", ""),
                                ("By tag", "layer"),
                                ("Custom", "#808080"),
                            ] {
                                if ui.selectable_label(mode == label, label).clicked()
                                    && mode != label
                                {
                                    self.patch.insert(field.into(), value.into());
                                }
                            }
                        });
                }
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
                    ("hatch_on_paper", "0") => "Drawing".into(),
                    ("hatch_on_paper", "1") => "Paper".into(),
                    ("print" | "lock" | "mortar" | "hatch_cross", "0") => "Off".into(),
                    ("print" | "lock" | "mortar" | "hatch_cross", "1") => "On".into(),
                    (_, "") => "Automatic".into(),
                    _ => s.replace('_', " "),
                };
                egui::ComboBox::from_id_salt(("style", field))
                    .width(ui.available_width().min(168.))
                    .selected_text(current.as_deref().map(label).unwrap_or("Mixed".into()))
                    .show_ui(ui, |ui| {
                        for value in choices {
                            if ui
                                .selectable_label(current.as_ref() == Some(&value), label(&value))
                                .clicked()
                            {
                                self.patch.insert(field.into(), value);
                            }
                        }
                    });
            }
        });
    }

    fn preview(&mut self, ui: &mut egui::Ui, key: &str) {
        let mut cells = vec![String::new(); 11];
        cells[4] = key.into();
        cells[8] = self.common(8, "hatch_angle").unwrap_or_else(|| "0".into());
        cells[9] = self.common(9, "hatch_scale").unwrap_or_else(|| "1".into());
        if let Some(pattern) = key.strip_prefix("pat:").and_then(|k| self.patterns.get(k)) {
            cells[10] = pattern["families"].to_string();
        }
        if self.preview.as_ref().is_none_or(|(prev, _)| prev != &cells) {
            let marks = kit::hatch_override(&cells);
            self.preview = Some((cells, marks));
        }
        let tag = self.common(0, "color");
        let color = |data: Option<String>, ground| {
            super::drawing_hatches::preview_color(data.as_deref(), tag.as_deref(), ground)
        };
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().min(160.), 48.),
            egui::Sense::hover(),
        );
        let ground = color(self.common(11, "hatch_ground"), true);
        let ink = color(self.common(12, "hatch_color"), false);
        kit::hatch_swatch(
            ui,
            rect,
            key,
            self.preview.as_ref().and_then(|(_, p)| p.as_ref()),
            ground,
            ink,
        );
        let cross = self.hatches.wood.contains(key)
            && self.common(15, "hatch_cross").as_deref() == Some("1");
        let fitted = self.hatches.fitted.contains(key);
        if self.hatches.coursed.contains(key) || fitted || cross {
            let kind = self.hatches.kinds.get(key).cloned().unwrap_or_default();
            let size = self
                .hatches
                .dimensions(key, self.common(18, "brick").as_deref());
            let spec = super::drawing_cuts::CutSpec {
                hatch: key.into(),
                kind,
                size,
                offset: self
                    .common(14, "hatch_offset")
                    .unwrap_or_default()
                    .split(',')
                    .filter_map(|t| t.trim().parse::<f64>().ok())
                    .filter(|n| n.is_finite() && n.abs() <= 10000.)
                    .collect(),
                joints: self.common(16, "joints").unwrap_or_default(),
                mortar: self.common(17, "mortar").as_deref() == Some("1"),
                cross,
                fitted,
                scale: self
                    .common(9, "hatch_scale")
                    .and_then(|n| n.parse::<f64>().ok())
                    .unwrap_or(1.)
                    .clamp(0.05, 20.),
            };
            if self.cuts.as_ref().is_none_or(|(prev, _)| prev != &spec) {
                let cuts = spec.build().ok();
                self.cuts = Some((spec, cuts));
            }
            if let Some((look, Some(cuts))) = &self.cuts {
                cuts.show(ui, look, ground, ink);
                if !look.size.is_empty() {
                    kit::piece_sample(ui, &look.kind, &look.size);
                }
            }
        }
    }

    fn pick_hatch(&mut self, ui: &mut egui::Ui, current: Option<&str>) -> Option<String> {
        let label = |key: &str| {
            if key.is_empty() {
                "Automatic".into()
            } else {
                self.hatches
                    .labels
                    .get(key)
                    .cloned()
                    .unwrap_or_else(|| key.trim_start_matches("pat:").replace('_', " "))
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
                let shortcuts = self
                    .hatches
                    .favourites
                    .iter()
                    .chain(&self.hatches.recent)
                    .filter(|key| {
                        self.hatches.labels.contains_key(*key)
                            || key
                                .strip_prefix("pat:")
                                .is_some_and(|k| self.patterns.contains_key(k))
                    })
                    .map(|k| {
                        (
                            k.clone(),
                            k.strip_prefix("pat:")
                                .and_then(|name| self.patterns.get(name)),
                        )
                    });
                let mut seen = std::collections::BTreeSet::new();
                for (key, pattern) in shortcuts.chain(imported).chain(standard) {
                    if !seen.insert(key.clone()) {
                        continue;
                    }
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
            }
            if pending && kit::quiet(ui, "Discard").clicked() {
                self.patch.clear();
            }
            if !pending {
                kit::caption_plain(ui, "Change a field to apply.");
            }
        });
    }
}
