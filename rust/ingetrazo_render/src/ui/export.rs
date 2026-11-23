// SPDX-License-Identifier: GPL-3.0-or-later
//! Delivery controls and repair checklist share one readiness snapshot.
use super::kit;
use serde_json::{json, Value};

pub(super) fn strip(ui: &mut egui::Ui, report: &Value, cmds: &mut Vec<String>) {
    let active = report["active"].as_str().unwrap_or("");
    let choices = report["choices"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let name = choices
        .iter()
        .find(|d| d[0] == active)
        .and_then(|d| d[1].as_str())
        .unwrap_or("Project checks");
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("delivery picker")
            .selected_text(name)
            .show_ui(ui, |ui| {
                for choice in choices {
                    let key = choice[0].as_str().unwrap_or("");
                    if ui
                        .selectable_label(active == key, choice[1].as_str().unwrap_or("Delivery"))
                        .clicked()
                    {
                        cmds.push(format!("deliver:select:{key}"));
                    }
                }
                if kit::quiet(ui, "New delivery…").clicked() {
                    cmds.push("deliver:new".into());
                }
                if !active.is_empty() && kit::quiet(ui, "Delete delivery").clicked() {
                    cmds.push(format!("deliver:delete:{active}"));
                }
                if !active.is_empty() && kit::quiet(ui, "Edit delivery…").clicked() {
                    cmds.push(format!("deliver:edit:{active}"));
                }
            });
        if !active.is_empty() && kit::quiet(ui, "Run").clicked() {
            cmds.push(format!("deliver:run:{active}"));
        }
    });
    if report["total"].as_u64().unwrap_or(0) > 0 {
        ui.label(format!("{} of {} ready", report["ready"], report["total"]));
    } else {
        ui.label("No placements in scope");
    }
}

pub(super) fn checklist(
    ui: &mut egui::Ui,
    report: &Value,
    focus: Option<&str>,
    placements: &std::collections::HashSet<String>,
    actions: &mut Vec<String>,
) {
    let key = focus.unwrap_or("*");
    let mut count = 0;
    kit::caption(ui, "Fix checklist");
    if let Some(rules) = report["checks"][key].as_object() {
        for (rule, entry) in rules {
            let Some(items) = entry[1].as_array() else {
                continue;
            };
            let items: Vec<_> = items
                .iter()
                .filter(|el| placements.is_empty() || placements.contains(&el["path"].to_string()))
                .collect();
            if items.is_empty() {
                continue;
            }
            count += items.len();
            ui.push_id((key, rule), |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!(
                        "{} · {}",
                        entry[0].as_str().unwrap_or("Check"),
                        items.len()
                    ));
                    let paths: Vec<_> = items.iter().map(|el| &el["path"]).collect();
                    if rule == "active-solid" && kit::quiet(ui, "Inspect loose geometry").clicked()
                    {
                        actions.push("item\u{1f}solid:active".into());
                    } else if rule != "active-solid" && kit::quiet(ui, "Show").clicked() {
                        actions.push(format!("exportshow\u{1f}{}", json!(paths)));
                    }
                    if let Some(pset) = items
                        .first()
                        .and_then(|el| el["fix"].as_str())
                        .filter(|s| !s.is_empty())
                    {
                        let id = ui.make_persistent_id("fix status");
                        let mut index = ui.data(|d| d.get_temp::<usize>(id)).unwrap_or(0);
                        let choices = ["Existing", "New", "Demolish", "Temporary"];
                        kit::choice(ui, "status", &mut index, &choices);
                        ui.data_mut(|d| d.insert_temp(id, index));
                        if kit::quiet(ui, &format!("Apply to {}", items.len())).clicked() {
                            let request = json!({"placements":paths,"path":["psets",pset,"Status"],
                                "value":choices[index].to_uppercase(),"property_kind":"label"});
                            actions.push(format!("exportstatus\u{1f}{request}"));
                        }
                    }
                });
            });
        }
    }
    if count == 0 {
        ui.label("No open checks in scope");
    }
    if let Some(notes) = report["notes"].as_array().filter(|n| !n.is_empty()) {
        kit::disclosure(ui, "coverage", "Check coverage", false, |ui| {
            for note in notes {
                if let Some(text) = note.as_str() {
                    ui.label(text);
                }
            }
        });
    }
}
