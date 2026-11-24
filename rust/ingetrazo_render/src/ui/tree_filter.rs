// SPDX-License-Identifier: GPL-3.0-or-later
//! The Filing lens presets edit the same query as the search field.
use super::kit;

const FILTERS: [(&str, &str); 8] = [
    ("", "All rows"),
    ("is:visible", "Visible"),
    ("is:hidden", "Hidden"),
    ("is:locked", "Locked"),
    ("is:selected", "Selected"),
    ("is:component", "Components"),
    ("is:image", "Images"),
    ("is:section", "Section planes"),
];

fn preset(word: &str) -> Option<usize> {
    FILTERS
        .iter()
        .position(|(key, _)| !key.is_empty() && word.eq_ignore_ascii_case(key))
}

pub(super) fn controls(ui: &mut egui::Ui, query: &mut String) -> bool {
    let (current, invert) = selection(query);
    let mut choice = current;
    let mut inverted = invert;
    kit::property_control(ui, "Filter", |ui| {
        kit::choice(
            ui,
            "project_filter",
            &mut choice,
            &FILTERS.map(|(_, label)| label),
        );
    });
    kit::toggle(ui, &mut inverted, "Invert filter");
    if current == choice && inverted == invert {
        return false;
    }
    *query = replace_selection(query, choice, inverted);
    true
}

fn selection(query: &str) -> (usize, bool) {
    let current = query.split_whitespace().find_map(preset).unwrap_or(0);
    (current, query.split_whitespace().next() == Some("!"))
}

fn replace_selection(query: &str, choice: usize, inverted: bool) -> String {
    let (current, invert) = selection(query);
    // Preserve other user-typed predicates, including negative ones.
    let mut parts = Vec::new();
    if inverted {
        parts.push("!");
    }
    let mut replaced = false;
    for word in query.split_whitespace().skip(usize::from(invert)) {
        if current != 0 && !replaced && preset(word) == Some(current) {
            replaced = true;
        } else {
            parts.push(word);
        }
    }
    if choice != 0 {
        parts.push(FILTERS[choice].0);
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_queries_and_quick_choices_share_state() {
        assert_eq!(selection("muro custom:value"), (0, false));
        let section = preset("is:section").unwrap();
        let hidden = preset("is:hidden").unwrap();
        assert_eq!(FILTERS[section].1, "Section planes");
        let query = "! Muro IS:HIDDEN -is:locked";
        assert_eq!(selection(query), (hidden, true));
        assert_eq!(
            replace_selection(query, section, false),
            "Muro -is:locked is:section"
        );
        assert_eq!(replace_selection(query, 0, true), "! Muro -is:locked");
        assert_eq!(
            replace_selection("custom:value", hidden, false),
            "custom:value is:hidden"
        );
    }
}
