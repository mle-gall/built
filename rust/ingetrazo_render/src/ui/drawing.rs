// SPDX-License-Identifier: GPL-3.0-or-later
//! Drawing controls for the focused Project node.
use super::{kit, parse_hex, Node, SWATCH_FALLBACK};
use egui::{Color32, Pos2};

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

pub(super) fn inspector(
    ui: &mut egui::Ui,
    n: &Node,
    catalog: &[(String, String)],
    actions: &mut Vec<String>,
    color_pick: &mut Option<(String, Color32, Pos2)>,
) {
    let [color, line, weight, print, hatch, ..] = n.cells.as_slice() else {
        return;
    };
    let mut set = |field: &str, value: &str| {
        actions.push(format!("{field}\u{1f}{}\u{1f}{value}", n.key));
    };
    kit::caption(ui, "Drawing");
    for (label, value, verb) in [
        ("Visible", n.eye, "eye"),
        ("Locked", n.lock, "lock"),
        ("Print", i8::from(print == "1"), "print"),
    ] {
        if value >= 0 {
            let mut on = value != 0;
            if kit::toggle(ui, &mut on, label).changed() {
                set(verb, if on { "1" } else { "0" });
            }
        }
    }
    ui.horizontal(|ui| {
        kit::caption_plain(ui, "Colour");
        let response = kit::quiet(ui, if color.is_empty() { "Automatic" } else { color });
        if response.clicked() {
            *color_pick = Some((
                n.key.clone(),
                parse_hex(color).unwrap_or(SWATCH_FALLBACK),
                response.rect.left_bottom(),
            ));
        }
    });
    let options = |kind: &str| -> Vec<&str> {
        catalog
            .iter()
            .filter(|(group, _)| group == kind)
            .map(|(_, key)| key.as_str())
            .collect()
    };
    let mut hatches = vec![""];
    hatches.extend(options("hatch"));
    for (label, field, current, options) in [
        ("Line", "linetype", line.as_str(), options("line")),
        (
            "Weight (mm)",
            "weight",
            weight.as_str(),
            vec![
                "auto", "0.13", "0.18", "0.25", "0.35", "0.50", "0.70", "1.00",
            ],
        ),
        ("Hatch", "hatch", hatch.as_str(), hatches),
    ] {
        let mut choices = options;
        if !choices.contains(&current) {
            choices.push(current);
        }
        let old = choices.iter().position(|v| *v == current).unwrap_or(0);
        let mut selected = old;
        let labels: Vec<_> = choices
            .iter()
            .map(|s| {
                if s.is_empty() {
                    "Automatic".into()
                } else {
                    s.replace('_', " ")
                }
            })
            .collect();
        ui.horizontal(|ui| {
            kit::caption_plain(ui, label);
            kit::choice(
                ui,
                (&n.key, field),
                &mut selected,
                &labels.iter().map(String::as_str).collect::<Vec<_>>(),
            );
        });
        if old != selected {
            set(field, choices[selected]);
        }
    }
}
