// SPDX-License-Identifier: GPL-3.0-or-later
use super::{kit, tokens::*};
use std::cmp::Ordering;

/// Stable key, display path, own visibility and lock flags.
pub type Row = (String, String, bool, bool);

#[derive(Default)]
pub struct Browser {
    rows: Vec<Row>,
    shown: Vec<usize>,
    filter: String,
    descending: bool,
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
    pub fn replace(&mut self, rows: Vec<Row>) {
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
        egui::ScrollArea::vertical().id_salt("tag-rows").show_rows(
            ui,
            ROW_H,
            self.shown.len(),
            |ui, range| {
                for &i in &self.shown[range] {
                    let (key, path, visible, locked) = &self.rows[i];
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
                            ui.add(
                                egui::Label::new(egui::RichText::new(path).font(body()).color(INK))
                                    .truncate(),
                            )
                            .on_hover_text(path);
                        })
                    });
                }
            },
        );
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
                .map(|p| (p.to_string(), p.to_string(), true, false))
                .collect(),
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
        tags.replace(vec![("stable".into(), "Changed".into(), false, true)]);
        assert!(tags.shown.is_empty());
        tags.filter.clear();
        tags.refresh();
        assert_eq!(tags.shown, [0]);
    }
}
