//! The Tasks destination: agent requests grouped by what they need
//! (Needs attention, In progress, Settled), the selected task's detail with
//! its changes and answers, the New task composer, and the tasks' pins over
//! the model. tasks_host.rs sends the data as `panels.data["tasks"]` and
//! runs the `task:` verbs queued here.
use super::*;
use serde_json::Value;

const SEP: char = '\x1f';

/// What the Tasks destination keeps between frames.
#[derive(Default)]
pub struct TaskPane {
    /// The New task composer is open.
    pub compose: bool,
    pub focus: bool,
    pub draft: String,
    pub agent: usize,
    pub pin: bool,
    /// The task whose Request changes… field is open.
    pub requesting: Option<String>,
    pub reply: String,
    /// (guid, x, y, selected, needs attention) in window points.
    pub pins: Vec<(String, f32, f32, bool, bool, usize)>,
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}

fn wrapped(ui: &mut egui::Ui, body_text: &str, font: egui::FontId, ink: Color32) {
    ui.add(egui::Label::new(egui::RichText::new(body_text).font(font).color(ink)).wrap());
}

fn changes_text(n: u64) -> String {
    match n {
        0 => "no change".into(),
        1 => "1 change".into(),
        n => format!("{n} changes"),
    }
}

impl Ui {
    pub fn set_task_pins(&mut self, pins: Vec<(String, f32, f32, bool, bool, usize)>) {
        self.tasks.pins = pins;
    }

    /// ⌘K "Ask an agent…" and the Tasks "New task": the composer, focused.
    pub fn compose_task(&mut self) {
        self.show_destination("tasks");
        self.tasks.compose = true;
        self.tasks.focus = true;
    }

    /// The Tasks destination in the dock's content area.
    pub(super) fn tasks_destination(ui: &mut egui::Ui, data: &Value, pane: &mut TaskPane, cmds: &mut Vec<String>) {
        let attention = data["unresolved"].as_u64().unwrap_or(0);
        kit::inspector_title(ui, "Tasks", &if attention > 0 { format!("{attention} need attention") } else { String::new() });
        ui.add_space(SPACE_XS);
        let composer_h = if pane.compose { 165.0 } else { BUTTON_H + SPACE_M };
        let height = (ui.available_height() - composer_h).max(80.0);
        let top = ui.cursor().min;
        let list = Rect::from_min_size(top, vec2(ui.available_width(), height));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list));
        egui::ScrollArea::vertical().id_salt("tasks list").auto_shrink([false, false]).max_height(height).show(&mut child, |ui| {
            ui.set_width(list.width());
            ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
            if data["empty"].as_bool().unwrap_or(true) {
                egui::Frame::new().inner_margin(egui::Margin::symmetric(SPACE_S as i8, SPACE_S as i8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing = vec2(0.0, SPACE_XS);
                    ui.label(egui::RichText::new("No tasks yet").font(strong(TYPE_BODY)).color(INK));
                    wrapped(ui, "Ask the built-in assistant, or an agent connected over MCP, to change or check the model. Its requests, questions and changes to review appear here.", body(), DIM);
                    if !data["access"].as_bool().unwrap_or(false) {
                        ui.add_space(SPACE_XS);
                        wrapped(ui, "External agents cannot edit this model. Allow them in Settings › AI.", meta(), FAINT);
                    }
                });
                if !data["access"].as_bool().unwrap_or(false) && kit::quiet(ui, "Open Settings › AI").clicked() {
                    cmds.push("task:settings:".into());
                }
                return;
            }
            let selected = text(&data, "selected").to_owned();
            for (i, group) in data["groups"].as_array().into_iter().flatten().enumerate() {
                let rows = group["rows"].as_array().cloned().unwrap_or_default();
                if rows.is_empty() {
                    continue;
                }
                let name = text(group, "name");
                let mut draw = |ui: &mut egui::Ui| {
                    for row in &rows {
                        let guid = text(row, "guid");
                        let meta_text = format!("{} · {}", text(row, "agent"), changes_text(row["count"].as_u64().unwrap_or(0)));
                        let on = guid == selected;
                        if kit::task_row(ui, text(row, "title"), &meta_text, text(row, "label"), text(row, "tone"), on).clicked() {
                            cmds.push(if on { "task:clear:".into() } else { format!("task:select:{guid}") });
                        }
                    }
                };
                if i == 2 {
                    let title = format!("Settled · {}", rows.len());
                    kit::disclosure(ui, "tasks settled", &title, false, |ui| draw(ui));
                } else {
                    kit::caption(ui, name);
                    draw(ui);
                }
                ui.add_space(SPACE_XS);
            }
            if let Some(detail) = data["detail"].as_object().map(|_| &data["detail"]) {
                ui.add_space(SPACE_XS);
                ui.separator();
                Self::task_detail(ui, detail, pane, cmds);
            }
        });
        ui.allocate_rect(list, Sense::hover());
        ui.add_space(SPACE_S);
        Self::task_composer(ui, data, pane, cmds);
    }

    fn task_detail(ui: &mut egui::Ui, d: &Value, pane: &mut TaskPane, cmds: &mut Vec<String>) {
        let guid = text(d, "guid").to_owned();
        let agent = text(d, "agent").to_owned();
        ui.spacing_mut().item_spacing = vec2(SPACE_XS, SPACE_XS);
        egui::Frame::new().inner_margin(egui::Margin { left: SPACE_S as i8, right: SPACE_S as i8, top: SPACE_XS as i8, bottom: 0 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            {
                ui.spacing_mut().item_spacing = vec2(SPACE_XS, SPACE_XS);
                wrapped(ui, text(d, "title"), title(), INK);
                let description = text(d, "description");
                if !description.is_empty() && description != text(d, "title") {
                    wrapped(ui, description, body(), DIM);
                }
                ui.horizontal(|ui| {
                    kit::status_chip(ui, text(d, "label"), kit::task_tone(text(d, "tone")));
                    ui.label(egui::RichText::new(format!("{agent}{}", if d["pinned"].as_bool().unwrap_or(false) { " · pinned" } else { "" })).font(meta()).color(FAINT));
                });
                let summary = text(d, "summary");
                if !summary.is_empty() {
                    wrapped(ui, summary, body(), INK_2);
                }
                // The conversation, latest last.
                let comments = d["comments"].as_array().cloned().unwrap_or_default();
                let shown = comments.len().saturating_sub(4);
                if shown > 0 {
                    ui.label(egui::RichText::new(format!("{shown} earlier messages")).font(meta()).color(FAINT));
                }
                for c in &comments[shown..] {
                    if text(c, "text") == summary || text(c, "text") == text(d, "question") {
                        continue;
                    }
                    ui.label(egui::RichText::new(text(c, "author")).font(meta()).color(FAINT));
                    wrapped(ui, text(c, "text"), body(), INK_2);
                }
                let question = text(d, "question");
                if !question.is_empty() {
                    ui.add_space(SPACE_XS);
                    ui.label(egui::RichText::new("Question").font(caption_font()).color(QUESTION));
                    wrapped(ui, question, body(), INK);
                }
                if let Some(note) = d["note"].as_str() {
                    wrapped(ui, note, body(), WARN);
                }
            }
        });
        // What it changed.
        let changes = d["changes"].as_array().cloned().unwrap_or_default();
        if !changes.is_empty() {
            ui.add_space(SPACE_XS);
            ui.horizontal(|ui| {
                ui.set_min_height(CONTROL_H);
                ui.add_space(SPACE_S);
                let (r, _) = ui.allocate_exact_size(vec2(8.0, CONTROL_H), Sense::hover());
                kit::dot(ui.painter(), r.center(), ACCENT);
                ui.label(egui::RichText::new(format!("{} modified", changes.len())).font(meta()).color(DIM));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if kit::chip_button(ui, "Show in Project") {
                        cmds.push(format!("task:project:{guid}"));
                    }
                });
            });
            kit::disclosure(ui, ("task changes", &guid), "Changed elements", true, |ui| {
                for c in &changes {
                    let (rect, resp) = kit::list_row(ui, ROW_H, false);
                    let label = kit::fit(ui, text(c, "label"), body(), INK, rect.width() - 90.0, 1);
                    kit::leading(ui.painter(), rect, rect.min.x + 22.0, label, INK);
                    let kind = ui.painter().layout_no_wrap(text(c, "change").to_owned(), meta(), FAINT);
                    let x = rect.max.x - SPACE_S - kind.size().x;
                    kit::leading(ui.painter(), rect, x, kind, FAINT);
                    ui.painter().circle_stroke(pos2(rect.min.x + 12.0, rect.center().y), 2.5, Stroke::new(1.0_f32, DIM));
                    if resp.on_hover_text("Select it in the model").clicked() {
                        cmds.push(format!("task:element:{guid}{SEP}{}", text(c, "key")));
                    }
                }
            });
        }
        ui.add_space(SPACE_S);
        // Its answers.
        let can = |k: &str| d[k].as_bool().unwrap_or(false);
        if can("can_reply") {
            Self::reply_field(ui, &mut pane.reply, &format!("Reply to {agent}…"), "Send", |t| cmds.push(format!("task:reply:{guid}{SEP}{t}")));
        } else if pane.requesting.as_deref() == Some(guid.as_str()) {
            let mut close = false;
            Self::reply_field(ui, &mut pane.reply, "What should change?", "Send", |t| {
                cmds.push(format!("task:changes:{guid}{SEP}{t}"));
                close = true;
            });
            ui.horizontal(|ui| {
                ui.add_space(SPACE_S);
                if kit::quiet(ui, "Cancel").clicked() {
                    close = true;
                }
            });
            if close {
                pane.requesting = None;
            }
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.add_space(SPACE_S);
                ui.spacing_mut().item_spacing = vec2(SPACE_XS, SPACE_XS);
                if can("can_accept") && kit::sheet_button(ui, "Accept changes", "", 1) {
                    cmds.push(format!("task:accept:{guid}"));
                }
                if can("can_request") && kit::sheet_button(ui, "Request changes…", "", 0) {
                    pane.requesting = Some(guid.clone());
                    pane.reply.clear();
                }
                if can("can_revert") && kit::quiet(ui, "Revert task").on_hover_text("Undo this task's changes as one step").clicked() {
                    cmds.push(format!("task:revert:{guid}"));
                }
            });
        }
    }

    /// A one-line answer with its button; `send` gets the text.
    fn reply_field(ui: &mut egui::Ui, draft: &mut String, hint: &str, verb: &str, send: impl FnOnce(String)) {
        let mut go = false;
        ui.horizontal(|ui| {
            ui.add_space(SPACE_S);
            let w = (ui.available_width() - 72.0 - 2.0 * SPACE_S).max(60.0);
            let r = ui.add(kit::field(draft).hint_text(egui::RichText::new(hint).color(FAINT)).desired_width(w));
            go = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            go |= kit::sheet_button(ui, verb, "", 1);
        });
        if go && !draft.trim().is_empty() {
            send(std::mem::take(draft).trim().to_owned());
        }
    }

    fn task_composer(ui: &mut egui::Ui, data: &Value, pane: &mut TaskPane, cmds: &mut Vec<String>) {
        ui.painter().hline(ui.max_rect().x_range(), ui.cursor().min.y - SPACE_XS, Stroke::new(1.0_f32, DIVIDER));
        if !pane.compose {
            let empty = data["empty"].as_bool().unwrap_or(true);
            ui.add_space(SPACE_XS);
            if kit::sheet_button(ui, if empty { "New task" } else { "+ New task" }, "", if empty { 1 } else { 0 }) {
                pane.compose = true;
                pane.focus = true;
            }
            return;
        }
        let agents: Vec<String> = data["agents"].as_array().into_iter().flatten().filter_map(|a| a.as_str().map(str::to_owned)).collect();
        let agents = if agents.is_empty() { vec![crate::tasks_host::ASSISTANT.to_owned()] } else { agents };
        pane.agent = pane.agent.min(agents.len() - 1);
        ui.add_space(SPACE_XS);
        ui.spacing_mut().item_spacing = vec2(SPACE_XS, SPACE_XS);
        ui.label(egui::RichText::new("New task").font(strong(TYPE_BODY)).color(INK));
        let r = ui.add(
            egui::TextEdit::multiline(&mut pane.draft)
                .hint_text(egui::RichText::new("Ask an agent to change or check the model…").color(FAINT))
                .font(body())
                .desired_rows(3)
                .desired_width(ui.available_width()),
        );
        if pane.focus {
            r.request_focus();
            pane.focus = false;
        }
        let selection = data["selection"].as_u64().unwrap_or(0);
        let mut send = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = vec2(SPACE_XS, 0.0);
            let names: Vec<&str> = agents.iter().map(String::as_str).collect();
            kit::choice(ui, "task agent", &mut pane.agent, &names);
            let mut pin = pane.pin && selection > 0;
            ui.add_enabled_ui(selection > 0, |ui| {
                if kit::toggle(ui, &mut pin, "Pin").on_hover_text("Anchor the request to the selection").changed() {
                    pane.pin = pin;
                }
            });
        });
        ui.horizontal(|ui| {
            if kit::sheet_button(ui, "Send", "⌘↵", 1) {
                send = true;
            }
            if kit::quiet(ui, "Cancel").clicked() {
                pane.compose = false;
            }
        });
        send |= r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command);
        if send && !pane.draft.trim().is_empty() {
            let agent = &agents[pane.agent];
            let pin = if pane.pin && selection > 0 { "1" } else { "0" };
            cmds.push(format!("task:new:{agent}{SEP}{pin}{SEP}{}", pane.draft.trim()));
            pane.draft.clear();
            pane.compose = false;
        }
    }

    /// Each pinned task's marker over the model; a click opens the task.
    pub(super) fn task_pins(&mut self, ctx: &egui::Context, screen: Rect) {
        if self.tasks.pins.is_empty() {
            return;
        }
        let right = self.free_right(ctx, screen);
        let pins = self.tasks.pins.clone();
        egui::Area::new(egui::Id::new("task pins"))
            .fixed_pos(Pos2::ZERO)
            .order(egui::Order::Middle)
            .interactable(true)
            .show(ctx, |ui| {
                for (guid, x, y, selected, attention, count) in pins {
                    let foot = pos2(x, y);
                    if foot.x > right - 12.0 || foot.y < TOP_H || foot.y > screen.bottom() - BOT_H {
                        continue;
                    }
                    if kit::task_pin(ui, foot, selected, attention, count, right).on_hover_text("Open the task").clicked() {
                        self.destination = Destination::Tasks;
                        self.actions.push(format!("task:select:{guid}"));
                    }
                }
            });
    }
}
