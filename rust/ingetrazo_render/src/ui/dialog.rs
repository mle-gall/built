//! A modal form drawn by the chrome (rust/ingetrazo_render/src/forms_host.rs): name and number
//! prompts, Preferences, export options, Location, Scale, Soften, About.
//!
//! Python owns the values. It sends the fields with `set_dialog`; each edit
//! comes back as `dlg:<seq>\u{1f}v\u{1f}<field id>\u{1f}<value>` and each
//! button as `dlg:<seq>\u{1f}b\u{1f}<button id>`, so Python always holds what
//! is on screen and a button needs no flush. Enter is the default button
//! (⌘Enter in a multi-line field), Esc the cancel button. The scrim takes
//! every click, as the sheet's does; a sheet asked from inside the form
//! (a confirmation) goes over it.

use std::collections::HashMap;

use egui::{pos2, vec2, Color32, CornerRadius, Margin, Rect, Sense, Stroke};

use super::{SPACE_XS, SPACE_S, SPACE_M, CONTROL_H, SCRIM, SWATCH_FALLBACK, body, caption_at, chip_button, glass, menu_row, meta, parse_hex, sheet_button, title_font,
            Surface, Ui, ACCENT, DIM, EDGE, FAINT, FIELD, GUT, HOVER, INK, OFF, R_ROW, TOP_H};

/// One field: (id, kind, label, text, value, min, max, options, flags, tip).
/// Kinds: page (starts a tab), caption, text (a note), edit, number, check,
/// choice, color, slider, path (text + Choose…), button.
pub type Field = (String, String, String, String, f64, f64, f64, Vec<String>, u32, String);
/// A dialog button: (id, label, key hint, role: 0 plain, 1 default, 2 discard, 3 cancel).
pub type Button = (String, String, String, u8);

fn reason_color(index: u8) -> Color32 {
    match index {
        0 => super::tokens::OK,
        1 => super::tokens::ERROR,
        2 => super::tokens::WARN,
        3 => super::tokens::INK_2,
        _ => ACCENT,
    }
}

pub const DISABLED: u32 = 1;
pub const FOCUS: u32 = 2;
pub const MULTILINE: u32 = 4;
const COMMIT_ON_BLUR: u32 = 32;
pub const PASSWORD: u32 = 8;
/// No label column: the control takes the whole row.
pub const FULL: u32 = 16;

pub struct Dialog {
    pub seq: String,
    /// What every action starts with: `dlg:<seq>` for a modal form,
    /// `lsheet:<sheet id>` for the fields of a large sheet (ui/large_sheet.rs).
    pub(super) prefix: String,
    title: String,
    note: String,
    width: f32,
    join_marks: Option<Vec<(u8, f32, f32)>>,
    fields: Vec<Field>,
    buttons: Vec<Button>,
    page: usize,
    /// Text being typed, per edit/number field.
    bufs: HashMap<String, String>,
    /// The text Python last sent per field: a new one replaces the buffer.
    sent: HashMap<String, String>,
    /// Focus the field marked FOCUS (a prompt's) at the next frame.
    focus_pending: bool,
    /// Tab with nothing focused: the first field of the page takes it.
    tab_focus: bool,
    pub(super) choice_open: Option<String>,
    color_open: Option<(String, Color32)>,
}

impl Dialog {
    pub(super) fn new(seq: String) -> Self {
        Dialog { prefix: format!("dlg:{seq}"), seq, title: String::new(), note: String::new(), width: 420.0, join_marks: None, fields: Vec::new(),
                 buttons: Vec::new(), page: 0, bufs: HashMap::new(), sent: HashMap::new(),
                 focus_pending: true, tab_focus: false, choice_open: None, color_open: None }
    }

    pub(super) fn update(&mut self, title: String, note: String, width: f32, fields: Vec<Field>, buttons: Vec<Button>) {
        for f in &fields {
            if f.1 == "edit" || f.1 == "number" {
                if self.sent.get(&f.0) != Some(&f.3) {
                    self.bufs.insert(f.0.clone(), f.3.clone());
                    self.sent.insert(f.0.clone(), f.3.clone());
                }
            }
        }
        self.title = title;
        self.note = note;
        self.width = width.clamp(280.0, 900.0);
        self.fields = fields;
        self.buttons = buttons;
        let pages = self.fields.iter().filter(|f| f.1 == "page").count();
        if pages > 0 && self.page >= pages { self.page = 0; }
    }

    fn act(&self, kind: &str, id: &str, value: Option<&str>) -> String {
        match value {
            Some(v) => format!("{}\u{1f}{kind}\u{1f}{id}\u{1f}{v}", self.prefix),
            None => format!("{}\u{1f}{kind}\u{1f}{id}", self.prefix),
        }
    }

    /// Inner controls handle Escape before their form or containing sheet.
    pub(super) fn escape_control(&mut self, ctx: &egui::Context, actions: &mut Vec<String>) -> bool {
        if self.choice_open.take().is_some() || self.color_open.take().is_some() {
            return true;
        }
        let Some(focus) = ctx.memory(|m| m.focused()) else { return false };
        // Deferred numeric edits close on Escape without committing a partial value.
        if self.fields.iter().any(|f| f.8 & COMMIT_ON_BLUR != 0 &&
                focus == egui::Id::new(("dlg_edit", &self.seq, &f.0))) {
            return false;
        }
        for (id, buf) in &mut self.bufs {
            if focus == egui::Id::new(("dlg_edit", &self.seq, id)) {
                if buf.is_empty() {
                    ctx.memory_mut(|m| m.surrender_focus(focus));
                } else {
                    buf.clear();
                    actions.push(format!("{}\u{1f}v\u{1f}{id}\u{1f}", self.prefix));
                }
                return true;
            }
        }
        false
    }

    pub(super) fn multiline_focused(&self, focused: Option<egui::Id>) -> bool {
        self.fields.iter().any(|field| field.8 & MULTILINE != 0
            && focused == Some(egui::Id::new(("dlg_edit", &self.seq, &field.0))))
    }

    fn button_of(&self, role: u8) -> Option<String> {
        self.buttons.iter().find(|b| b.3 == role).map(|b| b.0.clone())
    }
}

impl Ui {
    /// Shows (or refreshes) the form; an empty `seq` takes it down.
    pub fn set_dialog(&mut self, seq: String, title: String, note: String, width: f32, fields: Vec<Field>,
                      buttons: Vec<Button>) {
        if seq.is_empty() {
            if self.dialog.take().is_some() { self.ctx.memory_mut(|m| m.stop_text_input()); }
            return;
        }
        if self.dialog.as_ref().is_none_or(|d| d.seq != seq) {
            // One float at a time: the form closes whatever menu was open.
            self.view_open = false;
            self.menu = None;
            self.cmd_open = false;
            self.form = None;
            if let Some(col) = self.column.as_mut() {
                col.row_menu = None; col.kind_menu = None; col.head_menu = None;
            }
            self.ctx.memory_mut(|m| m.stop_text_input());
            self.dialog = Some(Dialog::new(seq));
        }
        if let Some(d) = self.dialog.as_mut() { d.update(title, note, width, fields, buttons); }
    }

    pub fn set_join_marks(&mut self, seq: &str, points: Vec<(u8, f32, f32)>) {
        if let Some(d) = self.dialog.as_mut().filter(|d| d.seq == seq) {
            d.join_marks = Some(points);
        }
    }

    pub fn dialog_open(&self) -> bool { self.dialog.is_some() }

    /// Enter and Esc answer the form; the rest goes to its focused field.
    /// Always true while it is up: no key reaches the model or the tools.
    pub(super) fn dialog_key(&mut self, name: &str, pressed: bool, mods: u8) -> bool {
        let Some(d) = self.dialog.as_mut() else { return false };
        if pressed && name == "Escape" {
            if d.escape_control(&self.ctx, &mut self.actions) { return true; }
            if let Some(id) = d.button_of(3) { self.actions.push(d.act("b", &id, None)); }
            return true;
        }
        if pressed && mods & (2 | 8) != 0 && d.fields.iter().any(|f| f.8 & COMMIT_ON_BLUR != 0) {
            let action = match (name, super::mods_from(mods).shift) {
                ("Z" | "z", false) => Some("undo"),
                ("Z" | "z", true) | ("Y" | "y", _) => Some("redo"),
                _ => None,
            };
            if let Some(id) = action.filter(|id| d.fields.iter().any(|f| f.0 == *id && f.1 == "button")) {
                self.actions.push(d.act("b", id, None));
                return true;
            }
        }
        if pressed && name == "Enter" {
            let focus = self.ctx.memory(|m| m.focused());
            if let Some(field) = d.fields.iter().find(|f| f.8 & COMMIT_ON_BLUR != 0 &&
                    focus == Some(egui::Id::new(("dlg_edit", &d.seq, &f.0)))) {
                if let Some(text) = d.bufs.get(&field.0) {
                    self.actions.push(d.act("v", &field.0, Some(text)));
                }
                return true;
            }
            let multiline = d.multiline_focused(self.ctx.memory(|m| m.focused()));
            let command = mods & (2 | 8) != 0;
            if !multiline || command {
                if let Some(id) = d.button_of(1) { self.actions.push(d.act("b", &id, None)); }
                return true;
            }
        }
        if self.ctx.wants_keyboard_input() {
            if let Some(key) = egui::Key::from_name(name) {
                self.events.push(egui::Event::Key { key, physical_key: None, pressed, repeat: false,
                                                    modifiers: super::mods_from(mods) });
            }
        } else if pressed && name == "Tab" {
            // Nothing focused yet: Tab goes to the first field.
            if let Some(d) = self.dialog.as_mut() { d.tab_focus = true; }
        }
        true
    }

    pub(super) fn dialog_popup(&mut self, ctx: &egui::Context, screen: Rect) {
        let review = self.review;
        let left = if review { GUT } else { GUT + 72.0 + GUT };
        let right = if review { screen.max.x - GUT } else { self.free_right(ctx, screen) - GUT };
        let Some(d) = self.dialog.as_mut() else { return };
        let w = d.width.min((right - left).max(300.0)).min(screen.width() - 2.0 * GUT);
        let join = d.join_marks.is_some();
        let x = (if join { right - w } else { (left + right - w) / 2.0 }).clamp(GUT, (screen.max.x - GUT - w).max(GUT));
        let top = if join {
            (screen.bottom() - 184.0 - 24.0 * d.fields.len() as f32).max(TOP_H + GUT)
        } else { TOP_H + 36.0 };
        let max_body = (screen.height() - top - 24.0 - 120.0).max(120.0);
        let mut out: Vec<String> = Vec::new();
        egui::Area::new(egui::Id::new("dialog"))
            .fade_in(false)
            .fixed_pos(pos2(0.0, 0.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let (all, _) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
                if let Some(marks) = &d.join_marks {
                    let paint = ui.painter().with_clip_rect(Rect::from_min_max(pos2(left, TOP_H), pos2(right, all.max.y)));
                    for &(reason, x, y) in marks {
                        let radius = if reason == 0 { 4.0 } else { 6.0 };
                        paint.circle(pos2(x, y), radius, reason_color(reason), Stroke::new(1.5_f32, INK));
                    }
                } else {
                    ui.painter().rect_filled(Rect::from_min_max(pos2(all.min.x, TOP_H), all.max), CornerRadius::ZERO, SCRIM);
                }
                let at = Rect::from_min_size(pos2(x, top), vec2(w, screen.height() - top - GUT));
                ui.scope_builder(egui::UiBuilder::new().max_rect(at), |ui| {
                    glass(Surface::Raised).inner_margin(Margin::same(14)).show(ui, |ui| {
                        ui.set_width(w - 28.0);
                        ui.spacing_mut().item_spacing = vec2(0.0, SPACE_XS);
                        ui.add(egui::Label::new(egui::RichText::new(&d.title).font(title_font()).color(INK)).wrap());
                        if !d.note.is_empty() {
                            ui.add(egui::Label::new(egui::RichText::new(&d.note).font(body()).color(DIM)).wrap());
                        }
                        let pages: Vec<String> = d.fields.iter().filter(|f| f.1 == "page").map(|f| f.2.clone()).collect();
                        if pages.len() > 1 {
                            ui.add_space(SPACE_S);
                            let labels: Vec<&str> = pages.iter().map(String::as_str).collect();
                            if let Some(i) = super::widgets::segmented(ui, &labels, d.page, ui.available_width()) {
                                if i != d.page {
                                    d.page = i;
                                    d.choice_open = None;
                                    d.color_open = None;
                                    out.push(d.act("p", &i.to_string(), None));
                                }
                            }
                        }
                        ui.add_space(SPACE_S);
                        egui::ScrollArea::vertical().max_height(max_body).auto_shrink([false, true]).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            fields(ui, d, &mut out);
                        });
                        ui.add_space(SPACE_M);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = vec2(SPACE_S, 0.0);
                            for (id, label, hint, role) in d.buttons.iter().filter(|b| b.3 == 2) {
                                if sheet_button(ui, label, hint, *role) { out.push(d.act("b", id, None)); }
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                for (id, label, hint, role) in d.buttons.iter().rev().filter(|b| b.3 != 2) {
                                    if ui.add_enabled_ui(role & 128 == 0, |ui| sheet_button(ui, label, hint, role & 127)).inner { out.push(d.act("b", id, None)); }
                                }
                            });
                        });
                    });
                });
            });
        self.actions.extend(out);
    }
}

/// The label column: wrapped to `w`, dim; its tip on hover.
fn label_cell(ui: &mut egui::Ui, label: &str, tip: &str, w: f32, h: f32, enabled: bool) {
    let mut job = egui::text::LayoutJob::single_section(label.to_owned(), egui::TextFormat {
        font_id: body(), color: if enabled { DIM } else { OFF }, ..Default::default() });
    job.wrap.max_width = w - 10.0;
    let g = ui.painter().layout_job(job);
    let (r, resp) = ui.allocate_exact_size(vec2(w, g.size().y.max(h)), Sense::hover());
    ui.painter().galley(pos2(r.min.x + 4.0, r.center().y - g.size().y / 2.0), g, DIM);
    if !tip.is_empty() { resp.on_hover_text(tip); }
}

pub(super) fn fields(ui: &mut egui::Ui, d: &mut Dialog, out: &mut Vec<String>) {
    let label_w = (ui.available_width() * 0.40).clamp(110.0, 220.0);
    let mut page = 0usize;
    let mut has_pages = false;
    let mut focus_done = false;
    let seq = d.seq.clone();
    let prefix = d.prefix.clone();
    let fields = d.fields.clone();
    for f in &fields {
        let (id, kind, label, text, value, min, max, options, flags, tip) =
            (&f.0, f.1.as_str(), &f.2, &f.3, f.4, f.5, f.6, &f.7, f.8, &f.9);
        if kind == "page" {
            if has_pages { page += 1; }
            has_pages = true;
            continue;
        }
        if has_pages && page != d.page { continue; }
        let enabled = flags & DISABLED == 0;
        let full = flags & FULL != 0;
        match kind {
            "rename_preview" => {
                super::kit::rename_preview(ui, label, text, options);
            }
            "caption" => { ui.add_space(SPACE_S); caption_at(ui, label, 4.0); }
            "text" if id.starts_with("join_reason_") => {
                let index = match &id["join_reason_".len()..] {
                    "join" => 0, "kink" => 1, "branch" => 2, "flags" => 3, _ => 4,
                };
                ui.scope(|ui| super::kit::status_chip(ui, label, reason_color(index)))
                    .response.on_hover_text(tip);
            }
            "text" => {
                egui::Frame::NONE.inner_margin(Margin { left: 4, right: 4, top: 0, bottom: 2 }).show(ui, |ui| {
                    ui.add(egui::Label::new(egui::RichText::new(if text.is_empty() { label } else { text })
                        .font(if flags & FULL != 0 { body() } else { meta() })
                        .color(if flags & FULL != 0 { DIM } else { FAINT })).wrap());
                });
            }
            "edit" | "number" => {
                let multiline = flags & MULTILINE != 0;
                let tid = egui::Id::new(("dlg_edit", &seq, id));
                ui.horizontal(|ui| {
                    if !full && !label.is_empty() { label_cell(ui, label, tip, label_w, 26.0, enabled); }
                    let suffix = if kind == "number" { options.first().cloned().unwrap_or_default() } else { String::new() };
                    let sw = if suffix.is_empty() { 0.0 } else {
                        ui.painter().layout_no_wrap(suffix.clone(), body(), DIM).size().x + 8.0 };
                    let w = (ui.available_width() - sw - 16.0).max(60.0);
                    let buf = d.bufs.entry(id.clone()).or_insert_with(|| text.clone());
                    let before = buf.clone();
                    // Numeric fields may specify a keyboard step after their unit.
                    if kind == "number" && enabled && ui.memory(|m| m.has_focus(tid)) {
                        if let Some(step) = options.get(1).and_then(|s| s.parse::<f64>().ok()) {
                            let delta = ui.input_mut(|i| {
                                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) { step }
                                else if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) { -step }
                                else { 0. }
                            });
                            if delta != 0. {
                                if let Ok(value) = buf.trim().replace(',', ".").parse::<f64>() {
                                    *buf = (value + delta).clamp(min, max).to_string();
                                }
                            }
                        }
                    }
                    let edit = if multiline {
                        egui::TextEdit::multiline(buf).desired_rows(4)
                    } else {
                        super::kit::field(buf)
                    };
                    let edit = edit.id(tid).font(body()).margin(Margin::symmetric(6, 4)).desired_width(w)
                        .password(flags & PASSWORD != 0).clip_text(true);
                    let output = ui.add_enabled_ui(enabled, |ui| edit.show(ui)).inner;
                    if !suffix.is_empty() {
                        ui.add_space(SPACE_S);
                        ui.label(egui::RichText::new(&suffix).font(body()).color(DIM));
                    }
                    let resp = output.response.clone();
                    if !tip.is_empty() && full { resp.clone().on_hover_text(tip); }
                    if !focus_done && enabled && ((d.focus_pending && flags & FOCUS != 0) || d.tab_focus) {
                        focus_done = true;
                        resp.request_focus();
                        let mut state = output.state;
                        let n = buf.chars().count();
                        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0), egui::text::CCursor::new(n))));
                        state.store(ui.ctx(), resp.id);
                    }
                    let send = if flags & COMMIT_ON_BLUR != 0 {
                        resp.lost_focus() && buf != text
                    } else { *buf != before };
                    if send {
                        out.push(format!("{prefix}\u{1f}v\u{1f}{id}\u{1f}{buf}"));
                    }
                });
            }
            "check" => {
                let item = (id.clone(), label.clone(), String::new(), "check".to_owned(), if value > 0.5 { 1 } else { 0 }, enabled);
                let resp_clicked = ui.horizontal(|ui| {
                    if !full && !text.is_empty() { label_cell(ui, text, tip, label_w, 24.0, enabled); }
                    let response = super::kit::menu_row_resp(ui, &item).0;
                    if !tip.is_empty() {
                        response.clone().on_hover_text(tip);
                    }
                    response.clicked()
                }).inner;
                if resp_clicked && enabled {
                    let v = if value > 0.5 { 0 } else { 1 };
                    if let Some(fm) = d.fields.iter_mut().find(|x| &x.0 == id) { fm.4 = v as f64; }
                    out.push(format!("{prefix}\u{1f}v\u{1f}{id}\u{1f}{v}"));
                }
            }
            "choice" => {
                let open = d.choice_open.as_deref() == Some(id.as_str());
                let mut toggle = false;
                ui.horizontal(|ui| {
                    if !full { label_cell(ui, label, tip, label_w, 26.0, enabled); }
                    let w = ui.available_width() - 4.0;
                    let sense = if enabled { Sense::click() } else { Sense::hover() };
                    let (r, resp) = ui.allocate_exact_size(vec2(w, 26.0), sense);
                    ui.painter().rect_filled(r, CornerRadius::same(R_ROW), FIELD);
                    if resp.hovered() { ui.painter().rect_filled(r, CornerRadius::same(R_ROW), HOVER); }
                    let shown = options.get(value.max(0.0) as usize).cloned().unwrap_or_else(|| text.clone());
                    ui.painter().text(r.left_center() + vec2(8.0, 0.0), egui::Align2::LEFT_CENTER, &shown, body(),
                                      if enabled { INK } else { OFF });
                    ui.painter().text(r.right_center() - vec2(8.0, 0.0), egui::Align2::RIGHT_CENTER,
                                      if open { "▴" } else { "▾" }, body(), FAINT);
                    if resp.clicked() { toggle = true; }
                });
                if toggle { d.choice_open = if open { None } else { Some(id.clone()) }; }
                if open {
                    ui.horizontal(|ui| {
                        if !full { ui.add_space(label_w); }
                        ui.vertical(|ui| {
                            for (j, opt) in options.iter().enumerate() {
                                let item = (j.to_string(), opt.clone(), String::new(), "check".to_owned(),
                                            if j as f64 == value { 1 } else { -1 }, true);
                                if menu_row(ui, &item) {
                                    if let Some(fm) = d.fields.iter_mut().find(|x| &x.0 == id) { fm.4 = j as f64; }
                                    out.push(format!("{prefix}\u{1f}v\u{1f}{id}\u{1f}{j}"));
                                    d.choice_open = None;
                                }
                            }
                        });
                    });
                }
            }
            "slider" => {
                ui.horizontal(|ui| {
                    if !full { label_cell(ui, label, tip, label_w, 24.0, enabled); }
                    let w = ui.available_width() - 56.0;
                    let (sr, resp) = ui.allocate_exact_size(vec2(w.max(60.0), 24.0),
                                                           if enabled { Sense::click_and_drag() } else { Sense::hover() });
                    let track = Rect::from_center_size(sr.center(), vec2(sr.width() - 16.0, 4.0));
                    let span = (max - min).max(1e-9);
                    let mut v = value;
                    if resp.dragged() || resp.clicked() {
                        if let Some(p) = resp.interact_pointer_pos() {
                            let t = ((p.x - track.min.x) / track.width()).clamp(0.0, 1.0) as f64;
                            v = (min + t * span).round();
                            if v != value {
                                if let Some(fm) = d.fields.iter_mut().find(|x| &x.0 == id) { fm.4 = v; }
                                out.push(format!("{prefix}\u{1f}v\u{1f}{id}\u{1f}{v}"));
                            }
                        }
                    }
                    let t = (((v - min) / span).clamp(0.0, 1.0)) as f32;
                    ui.painter().rect_filled(track, CornerRadius::same(2), FIELD);
                    let filled = Rect::from_min_max(track.min, pos2(track.min.x + t * track.width(), track.max.y));
                    ui.painter().rect_filled(filled, CornerRadius::same(2), if enabled { ACCENT } else { OFF });
                    let knob = pos2(track.min.x + t * track.width(), track.center().y);
                    ui.painter().circle_filled(knob, if resp.hovered() || resp.dragged() { 7.0 } else { 6.0 },
                                               if enabled { INK } else { OFF });
                    ui.add_space(SPACE_S);
                    ui.label(egui::RichText::new(text).font(body()).color(INK));
                });
            }
            "color" => {
                let mut open_now = false;
                ui.horizontal(|ui| {
                    if !full { label_cell(ui, label, tip, label_w, 26.0, enabled); }
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width() - 4.0, 26.0),
                                                           if enabled { Sense::click() } else { Sense::hover() });
                    if resp.hovered() { ui.painter().rect_filled(r, CornerRadius::same(R_ROW), HOVER); }
                    let sw = Rect::from_min_size(r.min + vec2(4.0, 4.0), vec2(36.0, 18.0));
                    ui.painter().rect_filled(sw, CornerRadius::same(4), parse_hex(text).unwrap_or(SWATCH_FALLBACK));
                    ui.painter().rect_stroke(sw, CornerRadius::same(4), Stroke::new(1.0_f32, EDGE), egui::StrokeKind::Inside);
                    ui.painter().text(pos2(sw.max.x + 8.0, r.center().y), egui::Align2::LEFT_CENTER,
                                      text.to_uppercase(), body(), INK);
                    if resp.clicked() { open_now = true; }
                });
                if open_now {
                    d.color_open = Some((id.clone(), parse_hex(text).unwrap_or(SWATCH_FALLBACK)));
                }
                let mut done = None;
                if let Some((cid, c)) = d.color_open.as_mut().filter(|(cid, _)| cid == id) {
                    ui.horizontal(|ui| {
                        if !full { ui.add_space(label_w); }
                        ui.vertical(|ui| {
                            egui::widgets::color_picker::color_picker_color32(ui, c, egui::widgets::color_picker::Alpha::Opaque);
                            ui.horizontal(|ui| {
                                if chip_button(ui, "Apply") { done = Some(Some(format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b()))); }
                                if chip_button(ui, "Cancel") { done = Some(None); }
                            });
                        });
                    });
                    let _ = cid;
                }
                if let Some(applied) = done {
                    d.color_open = None;
                    if let Some(hex) = applied {
                        if let Some(fm) = d.fields.iter_mut().find(|x| &x.0 == id) { fm.3 = hex.clone(); }
                        out.push(format!("{prefix}\u{1f}v\u{1f}{id}\u{1f}{hex}"));
                    }
                }
            }
            "path" => {
                ui.horizontal(|ui| {
                    if !full { label_cell(ui, label, tip, label_w, 26.0, enabled); }
                    let choose = options.first().cloned().unwrap_or_else(|| "Choose…".to_owned());
                    let cw = ui.painter().layout_no_wrap(choose.clone(), meta(), INK).size().x + 28.0;
                    let w = (ui.available_width() - cw - 8.0).max(40.0);
                    let (r, _) = ui.allocate_exact_size(vec2(w, CONTROL_H), Sense::hover());
                    ui.painter().rect_filled(r, CornerRadius::same(R_ROW), FIELD);
                    let mut job = egui::text::LayoutJob::single_section(
                        if text.is_empty() { "—".to_owned() } else { text.clone() },
                        egui::TextFormat { font_id: body(), color: if text.is_empty() { FAINT } else { INK }, ..Default::default() });
                    job.wrap.max_width = w - 12.0;
                    job.wrap.max_rows = 1;
                    job.wrap.break_anywhere = true;
                    let g = ui.painter().layout_job(job);
                    ui.painter().galley(pos2(r.min.x + 6.0, r.center().y - g.size().y / 2.0), g, INK);
                    ui.add_space(SPACE_S);
                    if enabled && chip_button(ui, &choose) { out.push(format!("{prefix}\u{1f}b\u{1f}{id}")); }
                });
            }
            "button" => {
                ui.horizontal(|ui| {
                    if !full { ui.add_space(label_w); }
                    let clicked = chip_button(ui, label);
                    if clicked && enabled { out.push(format!("{prefix}\u{1f}b\u{1f}{id}")); }
                    if !text.is_empty() {
                        ui.add_space(SPACE_S);
                        ui.label(egui::RichText::new(text).font(meta()).color(FAINT));
                    }
                });
            }
            _ => {}
        }
    }
    d.focus_pending = false;
    d.tab_focus = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(id: &str, kind: &str, text: &str, flags: u32) -> Field {
        (id.into(), kind.into(), format!("{id} label"), text.into(), 0.0, 0.0, 10.0, Vec::new(), flags, String::new())
    }

    fn form() -> Dialog {
        let mut d = Dialog::new("7".into());
        d.update("Make Component".into(), String::new(), 440.0,
                 vec![field("name", "edit", "Component #1", FOCUS), field("n", "number", "2", 0)],
                 vec![("cancel".into(), "Cancel".into(), "esc".into(), 3), ("ok".into(), "OK".into(), "↩".into(), 1)]);
        d
    }

    #[test]
    fn python_text_fills_the_buffer_once_then_typing_owns_it() {
        let mut d = form();
        assert_eq!(d.bufs["name"], "Component #1");
        d.bufs.insert("name".into(), "Caja".into());          // typed
        let fields = d.fields.clone();
        let buttons = d.buttons.clone();
        d.update("Make Component".into(), String::new(), 440.0, fields.clone(), buttons.clone());
        assert_eq!(d.bufs["name"], "Caja", "the same text from Python keeps what is typed");
        let mut changed = fields;
        changed[0].3 = "Reset".into();                         // Python sets a new value
        d.update("Make Component".into(), String::new(), 440.0, changed, buttons);
        assert_eq!(d.bufs["name"], "Reset");
    }

    #[test]
    fn actions_name_the_form_and_the_roles_find_enter_and_escape() {
        let d = form();
        assert_eq!(d.act("b", "ok", None), "dlg:7\u{1f}b\u{1f}ok");
        assert_eq!(d.act("v", "name", Some("Caja")), "dlg:7\u{1f}v\u{1f}name\u{1f}Caja");
        assert_eq!(d.button_of(1).as_deref(), Some("ok"));
        assert_eq!(d.button_of(3).as_deref(), Some("cancel"));
        assert_eq!(d.button_of(2), None);
    }

    #[test]
    fn every_kind_lays_out_and_the_prompt_field_takes_focus() {
        let mut d = Dialog::new("1".into());
        let mut choice = field("c", "choice", "", 0);
        choice.7 = vec!["A".into(), "B".into()];
        choice.4 = 1.0;
        d.update("All".into(), "note".into(), 520.0,
                 vec![field("p1", "page", "", 0), field("cap", "caption", "", 0), field("t", "text", "a note", 0),
                      field("e", "edit", "x", FOCUS), field("num", "number", "3", 0), field("k", "check", "", 0),
                      choice, field("s", "slider", "5", 0), field("col", "color", "#336699", 0),
                      field("path", "path", "/tmp/a.ifc", 0), field("btn", "button", "", 0),
                      field("p2", "page", "", 0), field("e2", "edit", "hidden", DISABLED)],
                 Vec::new());
        let ctx = egui::Context::default();
        // The chrome's named families, on egui's own fonts.
        let mut fonts = egui::FontDefinitions::default();
        let base = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
        for name in ["medium", "semibold"] {
            fonts.families.insert(egui::FontFamily::Name(name.into()), base.clone());
        }
        ctx.set_fonts(fonts);
        let mut out = Vec::new();
        let mut focused = None;
        for _ in 0..2 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| fields(ui, &mut d, &mut out));
            });
            focused = ctx.memory(|m| m.focused());
        }
        assert_eq!(focused, Some(egui::Id::new(("dlg_edit", "1", "e"))));
        assert!(out.is_empty(), "laying out reports nothing: {out:?}");
        assert!(!d.focus_pending);
        assert!(!d.bufs.contains_key("e2") || d.bufs["e2"] == "hidden");
    }
}
