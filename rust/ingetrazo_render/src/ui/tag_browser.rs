// SPDX-License-Identifier: GPL-3.0-or-later
use super::{kit, tokens::*};
use std::cmp::Ordering;

/// Stable key, display path, own visibility and lock flags, label and folder key.
pub type Row = (String, String, bool, bool, String, String);

struct DraggedTag(String);

#[derive(Default)]
pub struct Browser {
    rows: Vec<Row>,
    shown: Vec<usize>,
    filter: String,
    descending: bool,
    folders: Vec<(String, String)>,
    active: String,
    editing: Option<(String, String)>,
}

/// Compare case-folded paths with numeric runs ordered by magnitude.
/// Comparing digit lengths also handles identifiers too large for u64.
fn natural(mut a: &str, mut b: &str) -> Ordering {
    while !a.is_empty() && !b.is_empty() {
        if a.as_bytes()[0].is_ascii_digit() && b.as_bytes()[0].is_ascii_digit() {
            let digits = |s: &str| s.bytes().take_while(u8::is_ascii_digit).count();
            let (x, rest_a) = a.split_at(digits(a));
            let (y, rest_b) = b.split_at(digits(b));
            let (x, y) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
            let cmp = x.len().cmp(&y.len()).then_with(|| x.cmp(y));
            if cmp != Ordering::Equal {
                return cmp;
            }
            (a, b) = (rest_a, rest_b);
        } else {
            let mut ac = a.chars();
            let mut bc = b.chars();
            let cmp = ac.next().cmp(&bc.next());
            if cmp != Ordering::Equal {
                return cmp;
            }
            (a, b) = (ac.as_str(), bc.as_str());
        }
    }
    a.len().cmp(&b.len())
}

impl Browser {
    pub fn replace(&mut self, rows: Vec<Row>, folders: Vec<(String, String)>, active: String) {
        self.active = active;
        self.folders = folders;
        if let Some((key, buffer)) = &mut self.editing {
            if let Some(current) = rows.iter().find(|r| &r.0 == key) {
                // Follow undo and edits from another surface unless typing a new name.
                if self.rows.iter().any(|r| &r.0 == key && &r.4 == buffer) {
                    buffer.clone_from(&current.4);
                }
            } else {
                self.editing = None;
            }
        }
        self.rows = rows;
        self.refresh();
    }

    fn refresh(&mut self) {
        let filter = self.filter.trim().to_lowercase();
        let mut matches: Vec<_> = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(i, row)| {
                let path = row.1.to_lowercase();
                path.contains(&filter).then_some((i, path))
            })
            .collect();
        matches.sort_by(|(i, a), (j, b)| {
            let default = |i: usize| self.rows[i].0 == "Layer 0";
            default(*j).cmp(&default(*i)).then_with(|| {
                if self.descending {
                    natural(b, a)
                } else {
                    natural(a, b)
                }
            })
        });
        self.shown = matches.into_iter().map(|(i, _)| i).collect();
    }

    pub fn show(&mut self, ui: &mut egui::Ui, commands: &mut Vec<String>) {
        let edit = |key: &str, field: &str, value: &str| {
            format!("tag-edit:{}", serde_json::json!([field, key, value]))
        };
        ui.horizontal(|ui| {
            kit::caption_plain(ui, "Active tag");
            let active = self
                .rows
                .iter()
                .position(|r| r.0 == self.active)
                .unwrap_or(0);
            let mut selected = active;
            let labels: Vec<_> = self.rows.iter().map(|r| r.1.as_str()).collect();
            kit::choice(ui, "active-tag", &mut selected, &labels);
            if selected != active {
                commands.push(edit(&self.rows[selected].0, "active", ""));
            }
        });
        if ui
            .add(kit::search(&mut self.filter, "Filter tags"))
            .changed()
        {
            self.refresh();
        }
        if kit::quiet(
            ui,
            if self.descending {
                "Name ↓"
            } else {
                "Name ↑"
            },
        )
        .clicked()
        {
            self.descending = !self.descending;
            self.refresh();
        }
        if self.shown.is_empty() {
            kit::empty_state(ui, "No matching tags");
            return;
        }
        let editor_height = if self.editing.is_some() {
            3.0 * CONTROL_H + SPACE_L
        } else {
            0.0
        };
        egui::ScrollArea::vertical()
            .id_salt("tag-rows")
            .max_height((ui.available_height() - editor_height).max(ROW_H))
            .show_rows(ui, ROW_H, self.shown.len(), |ui, range| {
                for &i in &self.shown[range] {
                    let (key, path, visible, locked, name, folder) = &self.rows[i];
                    ui.push_id(key, |ui| {
                        ui.horizontal(|ui| {
                            for (verb, on, tip, paint) in [
                                (
                                    "visible",
                                    *visible,
                                    "Tag visibility",
                                    kit::paint_eye as fn(&egui::Ui, egui::Rect, bool),
                                ),
                                ("locked", *locked, "Tag lock", kit::paint_lock),
                            ] {
                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(CONTROL_H, ROW_H),
                                    egui::Sense::click(),
                                );
                                kit::paint_state(
                                    ui,
                                    rect,
                                    kit::state_of(&response, false, true),
                                    R_ROW,
                                );
                                paint(ui, rect.shrink(SPACE_XS), on);
                                if response.on_hover_text(tip).clicked() {
                                    commands.push(format!("tag-{verb}:{}:{key}", u8::from(!on)));
                                }
                            }
                            let selected = self.editing.as_ref().is_some_and(|(id, _)| id == key);
                            let (rect, response) = kit::list_row(ui, ROW_H, selected);
                            let response = response
                                .interact(egui::Sense::click_and_drag())
                                .on_hover_text(path);
                            let painter = ui.painter().with_clip_rect(rect);
                            let text = painter.layout_no_wrap(path.clone(), body(), INK);
                            kit::leading(&painter, rect, rect.left() + SPACE_XS, text, INK);
                            if key != "Layer 0" && response.drag_started() {
                                response.dnd_set_drag_payload(DraggedTag(key.clone()));
                            }
                            if response.dnd_hover_payload::<DraggedTag>().is_some() {
                                kit::paint_state(ui, response.rect, kit::State::Hover, R_ROW);
                            }
                            if let Some(tag) = response.dnd_release_payload::<DraggedTag>() {
                                commands.push(edit(&tag.0, "folder", folder));
                            }
                            if response.clicked() {
                                self.editing =
                                    (key != "Layer 0").then(|| (key.clone(), name.clone()));
                            }
                        })
                    });
                }
            });
        if let Some((key, buffer)) = &mut self.editing {
            if let Some(row) = self.rows.iter().find(|r| &r.0 == key) {
                ui.horizontal(|ui| {
                    kit::caption_plain(ui, "Name");
                    let r = ui.add(
                        kit::field(buffer)
                            .id_salt(("tag-name", &*key))
                            .desired_width(110.0),
                    );
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let save = kit::quiet(ui, "Apply").clicked() || enter;
                    if save && !buffer.trim().is_empty() && buffer.trim() != "Layer 0" {
                        commands.push(edit(key, "name", buffer.trim()));
                    }
                });
                ui.horizontal(|ui| {
                    kit::caption_plain(ui, "Folder");
                    let mut labels = vec!["Top level"];
                    labels.extend(self.folders.iter().map(|f| f.1.as_str()));
                    let old = self
                        .folders
                        .iter()
                        .position(|f| f.0 == row.5)
                        .map_or(0, |i| i + 1);
                    let mut index = old;
                    kit::choice(ui, ("tag-folder", &*key), &mut index, &labels);
                    if index != old {
                        commands.push(edit(
                            key,
                            "folder",
                            index.checked_sub(1).map_or("", |i| &self.folders[i].0),
                        ));
                    }
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn browser(paths: &[&str]) -> Browser {
        let mut browser = Browser::default();
        browser.replace(
            paths
                .iter()
                .map(|p| {
                    (
                        p.to_string(),
                        p.to_string(),
                        true,
                        false,
                        p.to_string(),
                        String::new(),
                    )
                })
                .collect(),
            vec![],
            String::new(),
        );
        browser
    }

    #[test]
    fn sorting_keeps_the_default_layer_first() {
        let mut tags = browser(&["zeta", "Layer 0", "alfa"]);
        assert_eq!(tags.shown, [1, 2, 0]);
        tags.descending = true;
        tags.refresh();
        assert_eq!(tags.shown, [1, 0, 2]);
    }

    #[test]
    fn names_sort_with_their_numbers_read_as_numbers() {
        let tags = browser(&["Capa 10", "capa 2", "Capa 1", "Alzado", "Layer 0"]);
        assert_eq!(tags.shown, [4, 3, 2, 1, 0]);
        assert!(natural("planta 9", "planta 12").is_lt());
    }

    #[test]
    fn filtering_matches_folder_or_tag_and_survives_document_edits() {
        let mut tags = browser(&["Muros", "Arq / Puertas"]);
        for query in ["PUER", "Arq"] {
            tags.filter = query.into();
            tags.refresh();
            assert_eq!(tags.shown, [1]);
        }
        tags.replace(
            vec![(
                "stable".into(),
                "Changed".into(),
                false,
                true,
                "Changed".into(),
                String::new(),
            )],
            vec![],
            String::new(),
        );
        assert!(tags.shown.is_empty());
        tags.filter.clear();
        tags.refresh();
        assert_eq!(tags.shown, [0]);
    }
}
