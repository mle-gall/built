// SPDX-License-Identifier: GPL-3.0-or-later
//! The delivery picker, destination and Project readiness cells share a snapshot.
use super::{kit, lens};
use serde_json::{json, Value};
use std::collections::BTreeMap;

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
        ui.set_height(super::BUTTON_H);
        let room = (ui.available_width() - 95.).max(96.);
        let clicked = kit::dropdown(ui, "Delivery:", delivery_name(report), false, room).clicked();
        kit::caption_plain(ui, &summary(report));
        clicked
    }).inner
}

/// Kept deliveries, checks and completed runs share the active record key.
pub(super) fn destination(ui: &mut egui::Ui, report: &Value, jobs: &[super::Job], cmds: &mut Vec<String>, actions: &mut Vec<String>, rule: &mut Option<String>) -> bool {
    let mut show = false;
    let active = report["active"].as_str().unwrap_or("");
    let prefix = format!("delivery-{active}-");
    let running = jobs.iter().find(|job| job.0.starts_with(&prefix));
    let choices = report["choices"].as_array().map(Vec::as_slice).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if kit::panel_button(ui, "+ New", "", 0, Some(super::BUTTON_H * 2.)).clicked() {
                cmds.push("deliver:new".into());
            }
            kit::inspector_title(ui, "Deliveries", "");
        });
    });
    ui.separator();
    ui.add_space(super::SPACE_S);
    egui::ScrollArea::vertical().id_salt("delivery contents")
        .max_height(ui.available_height())
        .auto_shrink([false, false]).show(ui, |ui| {
        if let Some(review)=report["review"].as_object() {
            kit::inspector_title(ui, "Review handover", delivery_name(report));
            kit::prop_row(ui,"Model revision",&review["revision"].to_string());
            kit::prop_row(ui,"IDS",review["ids_result"].as_str().unwrap_or(""));
            for file in review["ids"].as_array().into_iter().flatten() {
                if let Some(text)=file["message"].as_str() { kit::caption_plain(ui,text); }
                for rule in file["specifications"].as_array().into_iter().flatten().filter(|r|r["status"]==false || r["failed"].as_u64().unwrap_or(0)>0) {
                    kit::prop_row(ui,rule["name"].as_str().unwrap_or("IDS rule"),&format!("{} failed",rule["failed"]));
                    for item in rule["placements"].as_array().into_iter().flatten() {
                        if let Some(path)=item["path"].as_array() {
                            let name=item["name"].as_str().unwrap_or("Placement");
                            if kit::quiet(ui,&format!("Reveal {name}")).clicked() {
                                actions.push(format!("exportshow\u{1f}{}",json!([path])));
                            }
                        }
                    }
                }
                for row in review["outputs"].as_array().into_iter().flatten() {
                    kit::prop_row(
                        ui,
                        &row["format"].as_str().unwrap_or("").to_uppercase(),
                        row["result"].as_str().unwrap_or(""),
                    );
                }
                if kit::quiet(ui, "Back to deliveries").clicked() {
                    cmds.push("deliver:back".into());
                }
                return;
            }
            kit::caption_plain(ui, "Kept deliveries");
            if choices.is_empty() && report.is_object() {
                kit::empty_state(ui, "No kept deliveries yet");
            }
            if kit::quiet(ui,"Back to deliveries").clicked() {cmds.push("deliver:back".into());}
            return;
        }
        let issues = checks(report);
        let status = match issues.len() {
            _ if !report.is_object() => "Checking…".into(),
            0 => "✓ Ready".into(),
            n => format!("⚠ {}", lens::count(n, "issue", "issues")),
        };
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(super::SPACE_S);
                kit::caption_plain(ui, &status);
                kit::caption(ui, "READY TO DELIVER");
            });
        });
        for (label, check) in issues {
            let count = lens::count(check.count, "element", "elements");
            let text = match label {
                "Not filed" => format!("{count} not filed"),
                "Missing IFC class" => format!("{count} missing IFC class"),
                "Status missing" => format!("{count} missing status"),
                _ => format!("{label} · {}", check.count),
            };
            ui.push_id(label, |ui| {
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if kit::row_link(ui, "Show").clicked() {
                            *rule = Some(label.to_owned());
                            show = true;
                            if !check.paths.is_empty() {
                                actions.push(format!("exportshow\u{1f}{}", json!(check.paths)));
                            } else if check.loose {
                                actions.push("item\u{1f}solid:active".into());
                            }
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add_space(super::SPACE_S);
                            ui.label(egui::RichText::new("⚠").font(super::body()).color(super::DIM));
                            ui.add_space(super::SPACE_XS);
                            ui.add(egui::Label::new(egui::RichText::new(&text).font(super::body())).truncate())
                                .on_hover_text(&text);
                        });
                    });
                });
            });
        }
        if let Some(notes) = report["notes"].as_array().filter(|notes| !notes.is_empty()) {
            kit::disclosure(ui, "delivery coverage", "What is checked", false, |ui| {
                for note in notes.iter().filter_map(Value::as_str) {
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(super::SPACE_S);
                        kit::caption_plain(ui, note);
                    });
                }
            });
        }
        ui.add_space(super::SPACE_S);
        ui.separator();
        kit::caption(ui, "KEPT");
        if choices.is_empty() && report.is_object() {
            ui.horizontal(|ui| {
                ui.add_space(super::SPACE_S);
                kit::empty_state(ui, "No deliveries yet");
            });
        }
        for choice in choices {
            let key = choice[0].as_str().unwrap_or("");
            let name = choice[1].as_str().unwrap_or("Delivery");
            if key != active {
                let (rect, response) = kit::list_row(ui, super::ROW_H, false);
                let title = super::fit(ui, name, super::body(), super::INK,
                    rect.width() - super::CONTROL_H, 1);
                super::leading(ui.painter(), rect, rect.left() + super::SPACE_S, title, super::INK);
                ui.painter().text(rect.right_center() - egui::vec2(super::SPACE_S, 0.0),
                    egui::Align2::RIGHT_CENTER, "›", super::body(), super::DIM);
                if response.on_hover_text(name).clicked() {
                    cmds.push(format!("deliver:select:{key}"));
                }
                continue;
            }
            egui::Frame::new().fill(super::WELL).corner_radius(super::R_ROW)
                .inner_margin(egui::Margin::symmetric(super::SPACE_S as i8, super::SPACE_S as i8))
                .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let menu = kit::icon_button(ui, "⋯", "Delivery actions");
                        let id = ui.make_persistent_id(("delivery actions", key));
                        if menu.clicked() { ui.memory_mut(|m| m.toggle_popup(id)); }
                        if ui.memory(|m| m.is_popup_open(id)) {
                            ui.ctx().set_sublayer(ui.layer_id(), egui::LayerId::new(egui::Order::Foreground, id));
                            egui::popup_below_widget(ui, id, &menu, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                                if kit::sheet_button(ui, "Delete delivery", "", 2) {
                                    cmds.push(format!("deliver:delete:{key}"));
                                    ui.memory_mut(|m| m.close_popup());
                                }
                            });
                        }
                        kit::inspector_title(ui, name, "");
                    });
                });
                if let Some(outputs) = report["outputs"].as_array().filter(|outputs| !outputs.is_empty()) {
                    if let Some(job) = running {
                        kit::prop_row(ui, "Outputs", &job.2);
                        kit::progress_bar(ui, job.3, super::SPACE_XS);
                    } else if outputs.len() > 1 {
                        kit::caption(ui, "Outputs");
                    }
                    for output in outputs {
                        let format = output["format"].as_str().unwrap_or("").to_uppercase();
                        let path = output["destination"].as_str().unwrap_or("");
                        let last=report["runs"].as_array().and_then(|runs|runs.last()).and_then(|r|r["outputs"].as_array())
                            .and_then(|rows|rows.iter().find(|row|row["format"]==output["format"]));
                        let result=last.and_then(|row|row["result"].as_str()).filter(|s|*s!="Delivered").unwrap_or(path);
                        let status = match running {
                            Some(job) if job.2.strip_prefix("Writing ") == Some(&format) => "Writing…",
                            Some(_) => path,
                            None => result,
                        };
                        let row = ui.scope(|ui| kit::prop_row(ui, &format, status));
                        row.response.on_hover_text(path);
                    }
                }
                let runs = report["runs"].as_array().map(Vec::as_slice).unwrap_or_default();
                kit::disclosure(ui, ("delivery history", active), "Run history", !runs.is_empty(), |ui| {
                    if runs.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(super::SPACE_S);
                            kit::empty_state(ui, "No runs yet");
                        });
                    }
                    for (i, run) in runs.iter().enumerate().rev() {
                        ui.push_id(i, |ui| {
                            kit::prop_row(ui, run["result"].as_str().unwrap_or("Delivered"), run["time"].as_str().unwrap_or(""));
                            if let Some(ids)=run["ids_result"].as_str() {
                                kit::prop_row(ui,"IDS",ids);
                                if kit::row_link(ui,"Review").clicked() {cmds.push(format!("deliver:review:{active}:{i}"));}
                            }
                            if run["folder"].as_str().is_some_and(|s| !s.is_empty()) && kit::row_link(ui, "Open folder").clicked() {
                                cmds.push(format!("deliver:history:{active}:{i}"));
                            }
                        });
                    }
                });
                ui.add_space(super::SPACE_S);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = super::SPACE_S;
                    ui.add_space(super::SPACE_S);
                    let run = if report["package"] == true { "Run handover package" } else { "Run" };
                    if kit::sheet_button(ui, run, "", 1) { cmds.push(format!("deliver:run:{active}")); }
                    if kit::sheet_button(ui, "Edit setup…", "", 0) { cmds.push(format!("deliver:edit:{active}")); }
                });
            });
        }
    });
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

/// One line of the checklist: every rule with the same name, so "Solid geometry problems" is listed once for the placements and
/// the loose geometry together.
#[derive(Default)]
struct Check {
    paths: Vec<Value>,
    loose: bool,
    count: usize,
}

fn checks(report: &Value) -> BTreeMap<&str, Check> {
    let mut found = BTreeMap::<&str, Check>::new();
    let Some(rules) = report["checks"]["*"].as_object() else { return found };
    for (rule, entry) in rules {
        let Some(items) = entry[1].as_array() else { continue };
        let label = entry[0].as_str().unwrap_or("Check");
        for item in items {
            let check = found.entry(label).or_default();
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

/// Keep the bulk repair with the affected placements after Show opens Project.
pub(super) fn status_fixes(ui: &mut egui::Ui, report: &Value, label: &str, actions: &mut Vec<String>) {
    for (key, entry) in report["checks"]["*"].as_object().into_iter().flatten() {
        let Some(fix) = key.strip_prefix("status/").filter(|fix| !fix.is_empty()) else { continue };
        if entry[0] != label { continue; }
        let paths: Vec<_> = entry[1].as_array().into_iter().flatten().map(|item| &item["path"]).collect();
        ui.push_id(key, |ui| {
            kit::caption(ui, fix);
            ui.horizontal(|ui| {
                let id = ui.make_persistent_id("status");
                let mut index = ui.data(|d| d.get_temp::<usize>(id)).unwrap_or(0);
                let choices = ["Existing", "New", "Demolish", "Temporary"];
                kit::choice(ui, "status choice", &mut index, &choices);
                ui.data_mut(|d| d.insert_temp(id, index));
                if kit::quiet(ui, &format!("Apply to {}", paths.len())).clicked() {
                    let request = json!({"placements": paths, "path": ["psets", fix, "Status"],
                        "value": choices[index].to_uppercase(), "property_kind": "label"});
                    actions.push(format!("exportstatus\u{1f}{request}"));
                }
            });
        });
    }
}

pub(super) fn rule_rows(report: &Value, rule: &str) -> std::collections::HashSet<String> {
    let mut rows = std::collections::HashSet::new();
    if let Some(by_row) = report["checks"].as_object() {
        for (key, rules) in by_row {
            for entry in rules.as_object().into_iter().flatten().map(|(_, entry)| entry) {
                if entry[0] != rule { continue; }
                for item in entry[1].as_array().into_iter().flatten() {
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
        let found = checks(&r);
        let labels: Vec<_> = found.keys().copied().collect();
        assert_eq!(labels, ["Not filed", "Solid geometry problems"]);
        let solid = &found["Solid geometry problems"];
        assert!(solid.loose && solid.count == 3 && solid.paths.len() == 2);
        assert!(rule_rows(&r, "Solid geometry problems").contains("geometry:loose"));
    }
}
