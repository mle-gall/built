// SPDX-License-Identifier: GPL-3.0-or-later
//! The Filing lens presets edit the same query as the search field.
use super::kit;

const FILING: [(&str, &str); 8] = [
    ("", "All rows"),
    ("is:visible", "Visible"),
    ("is:hidden", "Hidden"),
    ("is:locked", "Locked"),
    ("is:selected", "Selected"),
    ("is:component", "Components"),
    ("is:image", "Images"),
    ("is:section", "Section planes"),
];

const DRAWING: [(&str, &str); 8] = [
    ("", "All tags"),
    ("is:visible", "Visible"),
    ("is:hidden", "Hidden"),
    ("is:locked", "Locked"),
    ("is:unprinted", "Not printed"),
    ("is:used", "In use"),
    ("is:unused", "Unused"),
    ("is:selected", "Tags of selection"),
];

type Presets = [(&'static str, &'static str); 8];

fn preset(filters: &Presets, word: &str) -> Option<usize> {
    filters
        .iter()
        .position(|(key, _)| !key.is_empty() && word.eq_ignore_ascii_case(key))
}

pub(super) fn controls(ui: &mut egui::Ui, query: &mut String, drawing: bool) -> bool {
    let filters = if drawing { &DRAWING } else { &FILING };
    let (current, invert) = selection(filters, query);
    let mut choice = current;
    let mut inverted = invert;
    kit::property_control(ui, "Filter", |ui| {
        kit::choice(
            ui,
            "project_filter",
            &mut choice,
            &filters.map(|(_, label)| label),
        );
    });
    kit::toggle(ui, &mut inverted, "Invert filter");
    if current == choice && inverted == invert {
        return false;
    }
    *query = replace_selection(filters, query, choice, inverted);
    true
}

fn selection(filters: &Presets, query: &str) -> (usize, bool) {
    let current = query.split_whitespace().find_map(|word| preset(filters, word)).unwrap_or(0);
    (current, query.split_whitespace().next() == Some("!"))
}

fn replace_selection(filters: &Presets, query: &str, choice: usize, inverted: bool) -> String {
    let (current, invert) = selection(filters, query);
    // Preserve other user-typed predicates, including negative ones.
    let mut parts = Vec::new();
    if inverted {
        parts.push("!");
    }
    let mut replaced = false;
    for word in query.split_whitespace().skip(usize::from(invert)) {
        if current != 0 && !replaced && preset(filters, word) == Some(current) {
            replaced = true;
        } else {
            parts.push(word);
        }
    }
    if choice != 0 {
        parts.push(filters[choice].0);
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawing_presets_keep_the_name_query_and_invert_the_combined_filter() {
        for (i, (key, _)) in DRAWING.iter().enumerate().skip(1) {
            let query = replace_selection(&DRAWING, "Site* is:visible", i, true);
            assert_eq!(query, format!("! Site* {key}"));
            assert_eq!(selection(&DRAWING, &query), (i, true));
            assert_eq!(replace_selection(&DRAWING, &query, 0, false), "Site*");
        }
    }

    #[test]
    fn typed_queries_and_quick_choices_share_state() {
        let filters = &FILING;
        assert_eq!(selection(filters, "muro custom:value"), (0, false));
        let section = preset(filters, "is:section").unwrap();
        let hidden = preset(filters, "is:hidden").unwrap();
        assert_eq!(filters[section].1, "Section planes");
        let query = "! Muro IS:HIDDEN -is:locked";
        assert_eq!(selection(filters, query), (hidden, true));
        assert_eq!(
            replace_selection(filters, query, section, false),
            "Muro -is:locked is:section"
        );
        assert_eq!(replace_selection(filters, query, 0, true), "! Muro -is:locked");
        assert_eq!(
            replace_selection(filters, "custom:value", hidden, false),
            "custom:value is:hidden"
        );
    }
}
