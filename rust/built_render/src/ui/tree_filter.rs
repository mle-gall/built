// SPDX-License-Identifier: GPL-3.0-or-later
//! The filter popover's quick choices edit the same query as the field.
use super::kit;

/// (query word, label). The last three only mean something for tags.
const FILTERS: [(&str, &str); 11] = [
    ("", "All rows"),
    ("is:visible", "Visible"),
    ("is:hidden", "Hidden"),
    ("is:locked", "Locked"),
    ("is:selected", "Selected"),
    ("is:component", "Components"),
    ("is:image", "Images"),
    ("is:section", "Section planes"),
    ("is:unprinted", "Tags not printed"),
    ("is:used", "Tags in use"),
    ("is:unused", "Unused tags"),
];
/// The choices for a tree without tag rows.
const PLAIN: usize = 8;

type Filters = [(&'static str, &'static str)];

fn filters(tags: bool) -> &'static Filters {
    if tags { &FILTERS } else { &FILTERS[..PLAIN] }
}

fn preset(filters: &Filters, word: &str) -> Option<usize> {
    filters
        .iter()
        .position(|(key, _)| !key.is_empty() && word.eq_ignore_ascii_case(key))
}

/// Whether the query holds a quick choice or is inverted (the field's
/// button shows it).
pub(super) fn active(query: &str) -> bool {
    selection(&FILTERS, query) != (0, false)
}

/// The quick filter and its inversion. `tags`: the tree lists tags, so the
/// tag choices are offered too. Returns whether the query changed.
pub(super) fn controls(ui: &mut egui::Ui, query: &mut String, tags: bool) -> bool {
    let filters = filters(tags);
    let (current, invert) = selection(filters, query);
    let mut choice = current;
    let mut inverted = invert;
    kit::property_control(ui, "Show", |ui| {
        let labels: Vec<&str> = filters.iter().map(|(_, label)| *label).collect();
        kit::choice(ui, "project_filter", &mut choice, &labels);
    });
    if kit::switch_row(ui, "Invert the filter", inverted, true).clicked() {
        inverted = !inverted;
    }
    if current == choice && inverted == invert {
        return false;
    }
    *query = replace_selection(filters, query, choice, inverted);
    true
}

fn selection(filters: &Filters, query: &str) -> (usize, bool) {
    let current = query.split_whitespace().find_map(|word| preset(filters, word)).unwrap_or(0);
    (current, query.split_whitespace().next() == Some("!"))
}

fn replace_selection(filters: &Filters, query: &str, choice: usize, inverted: bool) -> String {
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
    fn tag_choices_keep_the_name_query_and_invert_the_combined_filter() {
        let all = filters(true);
        for (i, (key, _)) in all.iter().enumerate().skip(1) {
            let query = replace_selection(all, "Site* is:visible", i, true);
            assert_eq!(query, format!("! Site* {key}"));
            assert_eq!(selection(all, &query), (i, true));
            assert_eq!(replace_selection(all, &query, 0, false), "Site*");
        }
    }

    #[test]
    fn typed_queries_and_quick_choices_share_state() {
        let filters = filters(false);
        assert_eq!(filters.len(), PLAIN);
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

    #[test]
    fn the_field_button_shows_a_quick_choice_or_an_inversion() {
        assert!(!active("") && !active("muro"));
        assert!(active("muro is:hidden") && active("! muro") && active("is:unused"));
    }
}
