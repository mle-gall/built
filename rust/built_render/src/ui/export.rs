// SPDX-License-Identifier: GPL-3.0-or-later
//! Export readiness: the delivery picker over the tree, the state in each
//! row's Readiness cell and the Fix checklist of the document inspector all
//! read one readiness snapshot.
use super::{kit, lens};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// A section's rows start where the inspector's other rows do.
const INDENT: f32 = 8.0;

/// The active delivery's name ("Project checks" while none is kept).
pub(super) fn delivery_name(report: &Value) -> &str {
    let active = report["active"].as_str().unwrap_or("");
    report["choices"]
        .as_array()
        .and_then(|choices| choices.iter().find(|d| d[0] == active))
        .and_then(|d| d[1].as_str())
        .unwrap_or("Project checks")
}

/// "3 of 9 ready", or why there is nothing to count.
pub(super) fn summary(report: &Value) -> String {
    if !report.is_object() {
        return "Checking…".into();
    }
    if report["total"].as_u64().unwrap_or(0) > 0 {
        format!("{} of {} ready", report["ready"], report["total"])
    } else {
        "No placements in scope".into()
    }
}

pub(super) fn blocked(report: &Value) -> u64 {
    report["total"].as_u64().unwrap_or(0).saturating_sub(report["ready"].as_u64().unwrap_or(0))
}

/// Link the per-row view to its delivery setup.
pub(super) fn strip(ui: &mut egui::Ui, report: &Value) -> bool {
    ui.horizontal(|ui| {
        let room = (ui.available_width() - 95.).max(96.);
        let clicked = kit::dropdown(ui, "Delivery:", delivery_name(report), false, room).clicked();
        kit::caption_plain(ui, &summary(report));
        clicked
    }).inner
}

/// Kept deliveries, checks and completed runs share the active record key.
pub(super) fn destination(ui: &mut egui::Ui, report: &Value, cmds: &mut Vec<String>, actions: &mut Vec<String>, rule: &mut Option<(String, String)>) -> bool {
    let mut show = false;
    let active = report["active"].as_str().unwrap_or("");
    let choices = report["choices"].as_array().map(Vec::as_slice).unwrap_or_default();
    kit::inspector_title(ui, "Deliveries", "");
    ui.add_space(super::SPACE_S);
    egui::ScrollArea::vertical().id_salt("delivery contents")
        .max_height((ui.available_height() - super::BUTTON_H - super::SPACE_M).max(super::ROW_H))
        .auto_shrink([false, false]).show(ui, |ui| {
        kit::caption_plain(ui, "Kept deliveries");
        if choices.is_empty() && report.is_object() {
            kit::empty_state(ui, "No kept deliveries yet");
        }
        for choice in choices {
            let key = choice[0].as_str().unwrap_or("");
            let row = (String::new(), choice[1].as_str().unwrap_or("Delivery").into(), String::new(), "check".into(), i8::from(key == active), true);
            if kit::menu_row(ui, &row) { cmds.push(format!("deliver:select:{key}")); }
        }
        if kit::quiet(ui, "New delivery…").clicked() { cmds.push("deliver:new".into()); }
        ui.separator();
        kit::inspector_title(ui, delivery_name(report), &summary(report));
        let n = blocked(report);
        if n > 0 && kit::quiet(ui, &format!("Show {n} to fix")).clicked() {
            *rule = None;
            let paths: std::collections::BTreeSet<_> = checks(report, None, &Default::default()).values()
                .flat_map(|c| &c.paths).map(Value::to_string).collect();
            if !paths.is_empty() { actions.push(format!("exportshow\u{1f}[{}]", paths.into_iter().collect::<Vec<_>>().join(","))); }
            show = true;
        }
        kit::disclosure(ui, ("delivery checks", active), "Fix checklist", true, |ui| {
            if let Some(selected) = checklist(ui, report, None, &Default::default(), actions) {
                *rule = Some(selected);
                show = true;
            }
        });
        if !active.is_empty() {
            ui.separator();
            if kit::quiet(ui, "Edit setup…").clicked() { cmds.push(format!("deliver:edit:{active}")); }
            kit::disclosure(ui, ("delivery history", active), "Run history", true, |ui| {
                let runs = report["runs"].as_array().map(Vec::as_slice).unwrap_or_default();
                if runs.is_empty() { kit::empty_state(ui, "No runs yet"); }
                for (i, run) in runs.iter().enumerate().rev() {
                    ui.push_id(i, |ui| {
                        kit::prop_row(ui, run["result"].as_str().unwrap_or("Delivered"), run["time"].as_str().unwrap_or(""));
                        if run["folder"].as_str().is_some_and(|s| !s.is_empty()) && kit::quiet(ui, "Open folder").clicked() {
                            cmds.push(format!("deliver:history:{active}:{i}"));
                        }
                    });
                }
            });
            if kit::quiet(ui, "Delete delivery").clicked() { cmds.push(format!("deliver:delete:{active}")); }
        }
    });
    ui.add_space(super::SPACE_S);
    if !active.is_empty() && kit::sheet_button(ui, "Run", "", 1) { cmds.push(format!("deliver:run:{active}")); }
    show
}

/// What a row's Readiness cell says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Ready {
    /// The snapshot has not arrived yet.
    Checking,
    Ready,
    ToFix(u64),
    /// Outside the delivery's scope.
    NotExported,
    /// Not something a delivery exports (scenes, sheets, section planes).
    NotApplicable,
}

impl Ready {
    /// The cell's text, full or compact (a glyph), and its ink.
    pub fn label(self, compact: bool) -> (String, egui::Color32) {
        match (self, compact) {
            (Ready::Checking, _) => ("…".into(), super::FAINT),
            (Ready::Ready, false) => ("ready".into(), super::FOUND),
            (Ready::Ready, true) => ("✓".into(), super::FOUND),
            (Ready::ToFix(n), false) => (format!("{n} to fix"), super::WARN),
            (Ready::ToFix(n), true) => (n.to_string(), super::WARN),
            (Ready::NotExported, false) => ("not exported".into(), super::FAINT),
            (Ready::NotExported, true) => ("∅".into(), super::FAINT),
            (Ready::NotApplicable, _) => ("–".into(), super::OFF),
        }
    }
    pub fn tip(self) -> String {
        match self {
            Ready::Checking => "Checking the delivery".into(),
            Ready::Ready => "Ready to export".into(),
            Ready::ToFix(n) => format!("{} to fix before export", lens::count(n as usize, "item", "items")),
            Ready::NotExported => "Outside this delivery".into(),
            Ready::NotApplicable => "Not part of a delivery".into(),
        }
    }
}

/// A row's state. Nodes and tags carry their own counts; a placement is
/// to fix when a check names it, and otherwise follows the row it is under.
pub(super) fn row_state(report: &Value, key: &str, above: Option<Ready>) -> Ready {
    if !report.is_object() {
        return Ready::Checking;
    }
    let (kind, id) = key.split_once(':').unwrap_or((key, ""));
    match kind {
        "n" | "preview-tag" => match report["rows"][key].as_array() {
            Some(counts) => {
                let blocked = counts.first().and_then(Value::as_u64).unwrap_or(0);
                let eligible = counts.get(1).and_then(Value::as_u64).unwrap_or(0);
                if blocked > 0 {
                    Ready::ToFix(blocked)
                } else if eligible > 0 {
                    Ready::Ready
                } else {
                    Ready::NotExported
                }
            }
            None => Ready::NotExported,
        },
        "i" | "c" | "g" => match report["items"][id].as_u64() {
            Some(n) if n > 0 => Ready::ToFix(n),
            _ if above == Some(Ready::NotExported) => Ready::NotExported,
            _ => Ready::Ready,
        },
        "d" | "preview-folder" | "m" => above.map_or(Ready::NotApplicable, |state| match state {
            Ready::ToFix(_) => Ready::Ready,
            other => other,
        }),
        "geometry" => {
            if report["checks"]["*"]["active-solid"].is_array() {
                Ready::ToFix(1)
            } else if matches!(report["scope"].as_str(), Some("whole" | "visible")) {
                Ready::Ready
            } else {
                Ready::NotExported
            }
        }
        _ => Ready::NotApplicable,
    }
}

/// One line of the checklist: every rule with the same name and the same
/// fix, so "Solid geometry problems" is listed once for the placements and
/// the loose geometry together.
#[derive(Default)]
struct Check {
    paths: Vec<Value>,
    loose: bool,
    count: usize,
}

fn checks<'a>(
    report: &'a Value,
    focus: Option<&str>,
    placements: &std::collections::HashSet<String>,
) -> BTreeMap<(&'a str, &'a str), Check> {
    let mut found: BTreeMap<(&str, &str), Check> = BTreeMap::new();
    let Some(rules) = report["checks"][focus.unwrap_or("*")].as_object() else {
        return found;
    };
    for (rule, entry) in rules {
        let Some(items) = entry[1].as_array() else { continue };
        let label = entry[0].as_str().unwrap_or("Check");
        for item in items {
            if !placements.is_empty() && !placements.contains(&item["path"].to_string()) {
                continue;
            }
            let check = found.entry((label, item["fix"].as_str().unwrap_or(""))).or_default();
            check.count += 1;
            if rule == "active-solid" {
                check.loose = true;
            } else {
                check.paths.push(item["path"].clone());
            }
        }
    }
    found
}

/// The Fix checklist: one line per open check with its verbs. `focus` and
/// `placements` narrow it to a node or to the selected placements.
pub(super) fn checklist(
    ui: &mut egui::Ui,
    report: &Value,
    focus: Option<&str>,
    placements: &std::collections::HashSet<String>,
    actions: &mut Vec<String>,
) -> Option<(String, String)> {
    if !report.is_object() {
        ui.horizontal(|ui| {
            ui.add_space(INDENT);
            kit::empty_state(ui, "Checking…");
        });
        return None;
    }
    let mut selected = None;
    let found = checks(report, focus, placements);
    for ((label, fix), check) in &found {
        ui.push_id((label, fix), |ui| {
            // One line per check: its verbs keep the trailing edge and the
            // name takes what is left, cut if the dock is narrow.
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = super::SPACE_XS;
                ui.add_space(INDENT);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(super::SPACE_XS);
                    if check.loose {
                        // Beside "Show" the loose geometry is its tree glyph.
                        let inspect = if check.paths.is_empty() {
                            kit::quiet(ui, "Inspect")
                        } else {
                            let button = kit::icon_button(ui, "", "Inspect the loose geometry");
                            kit::row_glyph(ui.painter(), button.rect.center(), "loose", super::INK_2);
                            button
                        };
                        if inspect.on_hover_text("Inspect the loose geometry").clicked() {
                            actions.push("item\u{1f}solid:active".into());
                        }
                    }
                    if !check.paths.is_empty() && kit::quiet(ui, "Show").on_hover_text("Select and zoom to them").clicked() {
                        selected = Some((label.to_string(), fix.to_string()));
                        actions.push(format!("exportshow\u{1f}{}", json!(check.paths)));
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        let text = egui::RichText::new(format!("{label} · {}", check.count)).font(super::body()).color(super::INK);
                        ui.add(egui::Label::new(text).truncate());
                    });
                });
            });
            // A missing status is fixed here, for every placement of the check.
            if !fix.is_empty() && !check.paths.is_empty() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = super::SPACE_XS;
                    ui.add_space(INDENT);
                    let id = ui.make_persistent_id("fix status");
                    let mut index = ui.data(|d| d.get_temp::<usize>(id)).unwrap_or(0);
                    let choices = ["Existing", "New", "Demolish", "Temporary"];
                    kit::choice(ui, "status", &mut index, &choices);
                    ui.data_mut(|d| d.insert_temp(id, index));
                    if kit::quiet(ui, &format!("Apply to {}", check.paths.len())).clicked() {
                        let request = json!({"placements":check.paths,"path":["psets",fix,"Status"],
                            "value":choices[index].to_uppercase(),"property_kind":"label"});
                        actions.push(format!("exportstatus\u{1f}{request}"));
                    }
                });
            }
        });
    }
    if found.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(INDENT);
            kit::empty_state(ui, "No open checks in scope");
        });
    }
    if let Some(notes) = report["notes"].as_array().filter(|n| !n.is_empty()) {
        kit::disclosure(ui, "coverage", "Check coverage", false, |ui| {
            for note in notes {
                if let Some(text) = note.as_str() {
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(INDENT);
                        ui.label(egui::RichText::new(text).font(super::body()).color(super::INK_2));
                    });
                }
            }
        });
    }
    selected
}

pub(super) fn rule_rows(report: &Value, rule: &(String, String)) -> std::collections::HashSet<String> {
    let mut rows = std::collections::HashSet::new();
    if let Some(by_row) = report["checks"].as_object() {
        for (key, rules) in by_row {
            for entry in rules.as_object().into_iter().flatten().map(|(_, entry)| entry) {
                if entry[0] != rule.0 { continue; }
                for item in entry[1].as_array().into_iter().flatten().filter(|item| item["fix"] == rule.1) {
                    rows.insert(key.clone());
                    if let Some(id) = item["row"].as_str() {
                        for kind in ["i", "c", "g"] { rows.insert(format!("{kind}:{id}")); }
                    } else if item["path"].is_null() { rows.insert("geometry:loose".into()); }
                }
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Value {
        json!({
            "active": "d1", "choices": [["d1", "Tender set"]], "ready": 1, "total": 3, "scope": "whole",
            "rows": {"n:walls": [2, 3], "n:roof": [0, 1], "preview-tag:Site": [0, 0]},
            "items": {"a1": 2},
            "checks": {"*": {
                "active-solid": ["Solid geometry problems", [{"name": "Loose geometry", "path": null, "fix": ""}]],
                "solid/": ["Solid geometry problems", [
                    {"name": "Wall", "path": ["a1"], "fix": ""}, {"name": "Wall", "path": ["a2"], "fix": ""}]],
                "unfiled/": ["Not filed", [{"name": "Wall", "path": ["a1"], "fix": ""}]],
            }},
            "notes": [],
        })
    }

    #[test]
    fn the_picker_is_labelled_with_the_delivery_and_the_count_reads_as_a_sentence() {
        assert_eq!(delivery_name(&report()), "Tender set");
        assert_eq!(delivery_name(&json!({"active": "", "choices": []})), "Project checks");
        assert_eq!(summary(&report()), "1 of 3 ready");
        assert_eq!(summary(&json!({"total": 0})), "No placements in scope");
        assert_eq!(summary(&Value::Null), "Checking…");
    }

    #[test]
    fn every_readiness_cell_has_a_state() {
        let r = report();
        assert_eq!(row_state(&r, "n:walls", None), Ready::ToFix(2));
        assert_eq!(row_state(&r, "n:roof", None), Ready::Ready);
        assert_eq!(row_state(&r, "preview-tag:Site", None), Ready::NotExported);
        assert_eq!(row_state(&r, "n:unknown", None), Ready::NotExported);
        assert_eq!(row_state(&r, "i:a1", Some(Ready::ToFix(2))), Ready::ToFix(2));
        assert_eq!(row_state(&r, "i:a3", Some(Ready::ToFix(2))), Ready::Ready);
        assert_eq!(row_state(&r, "i:a3", Some(Ready::NotExported)), Ready::NotExported);
        assert_eq!(row_state(&r, "geometry:loose", None), Ready::ToFix(1));
        assert_eq!(row_state(&r, "saved-scenes:all", None), Ready::NotApplicable);
        assert_eq!(row_state(&Value::Null, "n:walls", None), Ready::Checking);
        for state in [Ready::Checking, Ready::Ready, Ready::ToFix(1), Ready::NotExported, Ready::NotApplicable] {
            for compact in [false, true] {
                assert!(!state.label(compact).0.is_empty() && !state.tip().is_empty());
            }
        }
        assert_eq!(Ready::ToFix(1).tip(), "1 item to fix before export");
    }

    #[test]
    fn solid_problems_are_one_line_for_placements_and_loose_geometry() {
        let r = report();
        let found = checks(&r, None, &Default::default());
        let labels: Vec<_> = found.keys().map(|(label, _)| *label).collect();
        assert_eq!(labels, ["Not filed", "Solid geometry problems"]);
        let solid = &found[&("Solid geometry problems", "")];
        assert!(solid.loose && solid.count == 3 && solid.paths.len() == 2);
        // Narrowed to one selected placement, only its checks are left.
        let only: std::collections::HashSet<String> = [json!(["a2"]).to_string()].into();
        let scoped = checks(&r, None, &only);
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[&("Solid geometry problems", "")].count, 1);
    }
}
