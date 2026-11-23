//! Sheets: a workspace of the main window (views/wgpu_sheets.py owns the
//! composer, its data and its actions). While it is on, the frame shows the
//! page layer Python renders (lib.rs `render_page`) instead of the model,
//! and the chrome around it is the sheet's: the sheet tools on the left
//! rail, the sheet list and the selected item's inspector in the dock, the
//! paper, the scale, the tool hint and the zoom field in the bottom bar.
//! The top bar stays the window's (document, workspace, View ▾).
//!
//! Nothing here owns sheet state: Python sends rows and reads back
//! `sheets:<verb>\u{1f}<arg>` actions.

use super::*;

/// One sheet of the list: name, caption (paper, items), current.
pub type SheetRow = (String, String, bool);

#[derive(Default)]
pub struct Sheets {
    pub shown: bool,
    tools: Vec<Vec<Tool>>,
    active: String,
    list: Vec<SheetRow>,
    /// Inline rename: the row and the text being typed.
    renaming: Option<(usize, String)>,
    focus_rename: bool,
    /// The row menu: row and where.
    row_menu: Option<(usize, Pos2)>,
    title: String,
    inspector: Vec<(String, Vec<(String, String)>)>,
    edits: std::collections::HashMap<String, String>,
    /// An open choice: field id, options, current, under which rect.
    pick: Option<(String, Vec<String>, usize, Rect)>,
    /// (id, label, enabled) under the list.
    exports: Vec<(String, String, bool)>,
    paper: String,
    scale: String,
    tool: String,
    hint: String,
    cursor: String,
    zoom: String,
    zoom_buf: Option<String>,
    /// A question in the dock (delete a sheet, where to export): title,
    /// body, a text field or None, (id, label) buttons (the first is the default).
    prompt: Option<(String, String, Option<String>, Vec<(String, String)>)>,
    focus_prompt: bool,
}

impl Sheets {
    /// New inspector rows. A field being typed into keeps its text while its
    /// own row did not change; another item (a new title) starts afresh.
    fn set_inspector(&mut self, title: String, sections: Vec<(String, Vec<(String, String)>)>) {
        if self.title == title && self.inspector == sections { return; }
        if self.title != title {
            self.edits.clear();
            self.pick = None;
        } else {
            let row_of = |secs: &[(String, Vec<(String, String)>)], id: &str| secs.iter().flat_map(|s| s.1.iter())
                .find(|(_, v)| v.starts_with(ED) && v.split(ED).nth(2) == Some(id)).cloned();
            let stale: Vec<String> = self.edits.keys()
                .filter(|k| row_of(&self.inspector, k) != row_of(&sections, k) || row_of(&sections, k).is_none())
                .cloned().collect();
            for k in stale { self.edits.remove(&k); }
        }
        self.title = title;
        self.inspector = sections;
    }
    fn close_popups(&mut self) {
        self.row_menu = None;
        self.pick = None;
    }
    pub(super) fn dismissible_menu_open(&self) -> bool { self.row_menu.is_some() || self.pick.is_some() }
    fn popup_open(&self) -> bool {
        self.dismissible_menu_open() || self.renaming.is_some() || self.prompt.is_some()
    }
}

impl Sheets {
    /// A question over the dock (delete a sheet…): answered in place.
    fn prompt_place(&mut self, ui: &mut egui::Ui, out: &mut Vec<String>) {
        let s = self;
        let mut clear_prompt = false;
        if let Some((ptitle, pbody, field, buttons)) = s.prompt.as_mut() {
            // The question, over everything else in the dock.
            let mut answer: Option<String> = None;
            egui::Frame::new().fill(FIELD).corner_radius(CornerRadius::same(R_CONTROL)).inner_margin(Margin::same(10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing = vec2(0.0, SPACE_S);
                ui.label(egui::RichText::new(ptitle.as_str()).font(title_font()).color(INK));
                if !pbody.is_empty() { ui.label(egui::RichText::new(pbody.as_str()).font(body()).color(DIM)); }
                let mut enter = false;
                if let Some(text) = field.as_mut() {
                    let r = ui.add(super::kit::field(text).font(body()).margin(Margin::symmetric(6, 4))
                        .desired_width(ui.available_width() - 4.0));
                    if s.focus_prompt { r.request_focus(); s.focus_prompt = false; }
                    enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(SPACE_S, 0.0);
                    for (bi, (id, label)) in buttons.iter().enumerate() {
                        if sheet_button(ui, label, "", if bi == 0 { 1 } else { 0 }) { answer = Some(id.clone()); }
                    }
                });
                if enter { answer = buttons.first().map(|b| b.0.clone()); }
            });
            ui.add_space(SPACE_S);
            if let Some(id) = answer {
                let text = field.clone().unwrap_or_default();
                out.push(format!("answer{SEP}{id}{SEP}{text}"));
                clear_prompt = true;
            }
        }
        if clear_prompt { s.prompt = None; }
    }

    /// The sheets: header (name, +, ⋯) and the ordered list, at most
    /// ``max_h`` high, scrolled past that.
    fn list_place(&mut self, ui: &mut egui::Ui, out: &mut Vec<String>, max_h: f32) {
        let s = self;
        // Header: the place's name, New sheet, the current sheet's menu.
        ui.horizontal(|ui| {
            ui.set_min_height(28.0);
            ui.add_space(SPACE_S);
            ui.label(egui::RichText::new("Sheets").font(title()).color(INK));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let cur = s.list.iter().position(|r| r.2);
                let dots = (String::new(), "⋯".to_owned(), "This sheet: rename, duplicate, move, delete, classic editor".to_owned(), String::new(), -1i8, cur.is_some());
                let before = ui.cursor();
                if item_button(ui, &dots, None) {
                    if let Some(i) = cur { s.row_menu = Some((i, pos2(before.max.x - 180.0, before.min.y + 30.0))); }
                }
                let add = (String::new(), "+".to_owned(), "New sheet".to_owned(), String::new(), -1i8, true);
                if item_button(ui, &add, None) { out.push("add".into()); }
            });
        });
        ui.add_space(SPACE_S);
        let list_h = (s.list.len() as f32 * 26.0).min(max_h).max(26.0);
        egui::ScrollArea::vertical().id_salt("sheets_list").max_height(list_h).auto_shrink([false, true]).show(ui, |ui| {
            for (i, (name, cap, current)) in s.list.iter().enumerate() {
                if s.renaming.as_ref().is_some_and(|(r, _)| *r == i) {
                    let (_, buf) = s.renaming.as_mut().unwrap();
                    let resp = ui.add(super::kit::field(buf).font(body()).margin(Margin::symmetric(8, 4))
                        .desired_width(ui.available_width() - 4.0));
                    if s.focus_rename { resp.request_focus(); s.focus_rename = false; }
                    if resp.lost_focus() {
                        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let text = buf.trim().to_owned();
                        if enter && !text.is_empty() && &text != name { out.push(format!("rename{SEP}{i}{SEP}{text}")); }
                        s.renaming = None;
                    }
                    continue;
                }
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), CONTROL_H), Sense::click());
                paint_state(ui, r, state_of(&resp, *current, true), R_ROW);
                let c = ui.painter().layout_no_wrap(cap.clone(), meta(), FAINT);
                let cw = c.size().x.min(r.width() * 0.5);
                ui.painter().galley(pos2(r.max.x - 8.0 - cw, r.center().y - c.size().y / 2.0), c, FAINT);
                let mut job = egui::text::LayoutJob::single_section(name.clone(), egui::TextFormat {
                    font_id: if *current { strong(TYPE_BODY) } else { body() },
                    color: if *current { BRIGHT } else { INK }, ..Default::default() });
                job.wrap.max_width = (r.width() - cw - 26.0).max(20.0);
                job.wrap.max_rows = 1;
                job.wrap.break_anywhere = true;
                let g = ui.painter().layout_job(job);
                ui.painter().galley(pos2(r.min.x + 8.0, r.center().y - g.size().y / 2.0), g, INK);
                if resp.double_clicked() {
                    s.renaming = Some((i, name.clone()));
                    s.focus_rename = true;
                } else if resp.clicked() && !*current {
                    out.push(format!("open{SEP}{i}"));
                }
                if resp.secondary_clicked() {
                    let at = resp.interact_pointer_pos().unwrap_or(r.left_bottom());
                    s.row_menu = Some((i, at));
                }
            }
        });
    }

    /// The export commands.
    fn exports_place(&mut self, ui: &mut egui::Ui, out: &mut Vec<String>) {
        let s = self;
        if !s.exports.is_empty() {
            ui.add_space(SPACE_S);
            caption(ui, "Export");
            ui.add_space(SPACE_XS);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(SPACE_XS, SPACE_XS);
                ui.add_space(SPACE_S);
                for (id, label, enabled) in &s.exports {
                    let item = (id.clone(), label.clone(), String::new(), String::new(), -1i8, *enabled);
                    if item_button(ui, &item, None) { out.push(format!("export{SEP}{id}")); }
                }
            });
        }
    }

    /// What is selected on the page (or the sheet itself): its fields.
    fn item_place(&mut self, ui: &mut egui::Ui, out: &mut Vec<String>) {
        let s = self;
        // What is selected on the page (or the sheet itself).
        if !s.title.is_empty() {
            ui.horizontal(|ui| {
                ui.set_min_height(26.0);
                ui.add_space(SPACE_S);
                ui.label(egui::RichText::new(&s.title).font(title()).color(INK));
            });
        }
        egui::ScrollArea::vertical().id_salt("sheets_inspector").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, SPACE_XS);
            for (si, (caption_text, rows)) in s.inspector.iter().enumerate() {
                if si > 0 { ui.add_space(SPACE_S); }
                if !caption_text.is_empty() { caption(ui, caption_text); ui.add_space(SPACE_XS); }
                sheet_rows(ui, rows, &mut s.edits, &mut s.pick, out);
            }
        });
    }
}

const ED: char = '\u{1d}';
const SEP: char = '\u{1f}';

impl Ui {
    // ---- What Python sends ---------------------------------------------------
    pub fn show_sheets(&mut self, shown: bool) {
        if self.sheets.shown != shown {
            self.sheets.close_popups();
            self.sheets.renaming = None;
            self.sheets.zoom_buf = None;
            self.menu = None;
            self.view_open = false;
            self.form = None;
            self.ctx.memory_mut(|m| m.stop_text_input());
        }
        self.sheets.shown = shown;
    }
    pub fn sheets_shown(&self) -> bool { self.sheets.shown }
    /// ``groups``: per group, (id, label, shortcut, icon width, height, RGBA).
    pub fn set_sheet_tools(&mut self, groups: Vec<Vec<(String, String, String, usize, usize, Vec<u8>)>>) {
        self.sheets.tools = groups.into_iter().map(|g| g.into_iter().map(|(id, label, key, w, h, rgba)| {
            let icon = (w > 0 && rgba.len() == w * h * 4).then(|| {
                let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                self.ctx.load_texture(format!("sheet:{id}"), image, egui::TextureOptions::LINEAR)
            });
            Tool { id, label, key, icon, enabled: true }
        }).collect()).collect();
    }
    pub fn set_sheet_active(&mut self, id: String) { self.sheets.active = id; }
    pub fn set_sheet_list(&mut self, rows: Vec<SheetRow>) {
        if self.sheets.renaming.as_ref().is_some_and(|(i, _)| *i >= rows.len()) { self.sheets.renaming = None; }
        self.sheets.list = rows;
    }
    pub fn set_sheet_inspector(&mut self, title: String, sections: Vec<(String, Vec<(String, String)>)>) {
        self.sheets.set_inspector(title, sections);
    }
    pub fn set_sheet_exports(&mut self, exports: Vec<(String, String, bool)>) { self.sheets.exports = exports; }
    pub fn set_sheet_status(&mut self, paper: String, scale: String, tool: String, hint: String, cursor: String, zoom: String) {
        let s = &mut self.sheets;
        s.paper = paper; s.scale = scale; s.tool = tool; s.hint = hint; s.cursor = cursor; s.zoom = zoom;
    }
    /// A question in the dock; an empty title takes it down. ``field``: the
    /// text of an editable line (a path), or None.
    pub fn set_sheet_prompt(&mut self, title: String, body: String, field: Option<String>, buttons: Vec<(String, String)>) {
        if title.is_empty() { self.sheets.prompt = None; return; }
        self.sheets.focus_prompt = field.is_some();
        self.sheets.prompt = Some((title, body, field, buttons));
    }
    /// Start renaming row ``i`` in place (F2, the row menu, a double click).
    pub fn rename_sheet(&mut self, i: usize) {
        if let Some(row) = self.sheets.list.get(i) {
            self.sheets.renaming = Some((i, row.0.clone()));
            self.sheets.focus_rename = true;
        }
    }

    // ---- Keys and pointer ------------------------------------------------------
    pub(super) fn sheets_close_popups(&mut self) { self.sheets.close_popups(); }

    /// The chrome keeps a key in the workspace only while one of its fields
    /// has focus, or Escape while one of its popups is open; everything else
    /// is the page's (views/wgpu_sheets.py hands it to the composer).
    pub(super) fn sheets_claims_key(&self, name: &str) -> bool {
        self.sheet.is_some() || self.cmd_open || self.ctx.wants_keyboard_input()
            || (name == "Escape" && (self.view_open || self.menu.is_some() || self.form.is_some() || self.sheets.popup_open()))
    }
    pub(super) fn sheets_key(&mut self, name: &str, pressed: bool, mods: u8) -> bool {
        if name == "Escape" && (self.view_open || self.menu.is_some() || self.form.is_some() || self.sheets.popup_open()) {
            if pressed {
                if self.form.take().is_some() { self.actions.push("form:close".into()); }
                self.view_open = false;
                self.menu = None;
                self.sheets.close_popups();
                self.sheets.renaming = None;
                self.sheets.zoom_buf = None;
                if self.sheets.prompt.take().is_some() { self.actions.push(format!("sheets:answer{SEP}{SEP}")); }
                self.ctx.memory_mut(|m| m.stop_text_input());
            }
            return true;
        }
        if !self.ctx.wants_keyboard_input() { return false; }
        if let Some(key) = egui::Key::from_name(name) {
            self.events.push(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: mods_from(mods) });
        }
        true
    }

    /// Areas that take the pointer while the workspace is on.
    pub(super) fn sheets_areas(&self) -> [(&'static str, bool); 4] {
        let on = self.sheets.shown && !self.review;
        [("sheets_rail", on && !self.sheets.tools.is_empty()), ("sheets_dock", on),
         ("sheets_menu", on && self.sheets.row_menu.is_some()), ("sheets_pick", on && self.sheets.pick.is_some())]
    }

    /// The workspace's chrome under the top bar (which the caller draws).
    pub(super) fn sheets_frame(&mut self, ctx: &egui::Context, screen: Rect) {
        if self.review { return; }
        self.sheets_rail(ctx, screen);
        self.sheets_dock(ctx, screen);
        self.sheets_bottom(ctx, screen);
        self.sheets_row_menu(ctx);
        self.sheets_pick(ctx);
    }

    // ---- The tool rail -----------------------------------------------------------
    fn sheets_rail(&mut self, ctx: &egui::Context, screen: Rect) {
        if self.sheets.tools.is_empty() { return; }
        let mut clicked = None;
        let (band_top, band_bottom) = self.band(ctx, screen);
        let room = band_bottom - band_top;
        let groups = &self.sheets.tools;
        let tall = |cols: usize| groups.iter().map(|g| g.len().div_ceil(cols) as f32 * 32.0).sum::<f32>()
            + groups.len().saturating_sub(1) as f32 * 9.0 + 8.0;
        let cols = (2..=5).find(|c| tall(*c) <= room).unwrap_or(5);
        let active = self.sheets.active.clone();
        egui::Area::new(egui::Id::new("sheets_rail"))
            .fixed_pos(pos2(GUT, band_top))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                frame().corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(4)).show(ui, |ui| {
                    ui.set_min_height(room - 8.0);
                    ui.spacing_mut().item_spacing = vec2(SPACE_XS, SPACE_XS);
                    for (gi, group) in groups.iter().enumerate() {
                        if gi > 0 {
                            let (r, _) = ui.allocate_exact_size(vec2(cols as f32 * 32.0 - 2.0, 7.0), Sense::hover());
                            ui.painter().hline(r.x_range().shrink(2.0), r.center().y, Stroke::new(1.0_f32, EDGE));
                        }
                        for row in group.chunks(cols) {
                            ui.horizontal(|ui| {
                                for tool in row {
                                    if tool_button(ui, tool, tool.id == active) { clicked = Some(tool.id.clone()); }
                                }
                            });
                        }
                    }
                });
            });
        if let Some(id) = clicked {
            self.sheets.active = id.clone();
            self.actions.push(format!("sheets:tool{SEP}{id}"));
        }
    }

    // ---- The dock: the sheets, then what is selected on the page ----------------
    /// The dock. Today one column: the sheet list over the selected item;
    /// each part is a place of its own (`Sheets::*_place`), so a later dock
    /// with Sheets | Views | Item tabs shows one place per tab.
    fn sheets_dock(&mut self, ctx: &egui::Context, screen: Rect) {
        let (top, bottom) = self.band(ctx, screen);
        #[allow(non_snake_case)]
        let W: f32 = self.dock_w.min((screen.width() * 0.45).max(DOCK_MIN_W));
        let height = (bottom - top).max(200.0);
        let mut out: Vec<String> = Vec::new();
        let s = &mut self.sheets;
        egui::Area::new(egui::Id::new("sheets_dock"))
            .fixed_pos(pos2(screen.max.x - GUT - W, top))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                glass(Surface::Panel).inner_margin(Margin::same(10)).show(ui, |ui| {
                    ui.set_width(W - 20.0);
                    ui.set_max_width(W - 20.0);
                    ui.set_height(height - 20.0);
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    s.prompt_place(ui, &mut out);
                    // The list: a third of the dock at most, scrolled past that.
                    let list_h = ui.available_height() * 0.34;
                    s.list_place(ui, &mut out, list_h);
                    s.exports_place(ui, &mut out);
                    ui.add_space(SPACE_S);
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
                    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0_f32, EDGE));
                    s.item_place(ui, &mut out);
                });
            });
        for a in out { self.actions.push(format!("sheets:{a}")); }
    }

    fn sheets_row_menu(&mut self, ctx: &egui::Context) {
        let Some((i, at)) = self.sheets.row_menu else { return };
        let n = self.sheets.list.len();
        let mut clicked: Option<String> = None;
        let mut rename = false;
        egui::Area::new(egui::Id::new("sheets_menu"))
            .fixed_pos(at)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                glass(Surface::Raised).inner_margin(Margin::same(6)).show(ui, |ui| {
                    ui.set_width(180.0);
                    ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                    let row = |id: &str, label: &str, enabled: bool| (id.to_owned(), label.to_owned(), String::new(), String::new(), -1i8, enabled);
                    if menu_row(ui, &row("rename", "Rename…", true)) { rename = true; }
                    if menu_row(ui, &row("dup", "Duplicate", true)) { clicked = Some(format!("dup{SEP}{i}")); }
                    if menu_row(ui, &row("up", "Move earlier", i > 0)) { clicked = Some(format!("up{SEP}{i}")); }
                    if menu_row(ui, &row("down", "Move later", i + 1 < n)) { clicked = Some(format!("down{SEP}{i}")); }
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 7.0), Sense::hover());
                    ui.painter().hline(r.x_range().shrink(6.0), r.center().y, Stroke::new(1.0_f32, EDGE));
                    if menu_row(ui, &row("del", "Delete…", n > 1)) { clicked = Some(format!("del{SEP}{i}")); }
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 7.0), Sense::hover());
                    ui.painter().hline(r.x_range().shrink(6.0), r.center().y, Stroke::new(1.0_f32, EDGE));
                    if menu_row(ui, &row("classic", "Open in classic sheet editor…", true)) { clicked = Some("classic".into()); }
                });
            });
        if rename {
            self.sheets.row_menu = None;
            self.rename_sheet(i);
        }
        if let Some(a) = clicked {
            self.sheets.row_menu = None;
            self.actions.push(format!("sheets:{a}"));
        }
    }

    fn sheets_pick(&mut self, ctx: &egui::Context) {
        let Some((id, options, at, anchor)) = self.sheets.pick.clone() else { return };
        let mut chosen: Option<usize> = None;
        egui::Area::new(egui::Id::new("sheets_pick"))
            .pivot(egui::Align2::RIGHT_TOP)
            .fixed_pos(pos2(anchor.max.x, anchor.max.y + 4.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                glass(Surface::Raised).inner_margin(Margin::same(6)).show(ui, |ui| {
                    ui.set_width(anchor.width().max(160.0));
                    ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                    egui::ScrollArea::vertical().id_salt("sheets_pick_rows").max_height(320.0).show(ui, |ui| {
                        for (i, label) in options.iter().enumerate() {
                            let row = (String::new(), label.clone(), String::new(), "check".to_owned(), if i == at { 1 } else { 0 }, true);
                            if menu_row(ui, &row) { chosen = Some(i); }
                        }
                    });
                });
            });
        if let Some(i) = chosen {
            self.sheets.pick = None;
            if i != at { self.actions.push(format!("sheets:edit{SEP}{id}{SEP}{i}")); }
        }
    }

    // ---- The bottom bar: paper · scale | tool and hint | where, zoom -------------
    fn sheets_bottom(&mut self, ctx: &egui::Context, screen: Rect) {
        let mut out: Vec<String> = Vec::new();
        let tool_icon = self.sheets.tools.iter().flatten().find(|t| t.id == self.sheets.active).and_then(|t| t.icon.clone());
        let s = &mut self.sheets;
        // Same place and id as the model's bar: the band above it is the same.
        egui::Area::new(egui::Id::new("bottom"))
            .fixed_pos(pos2(0.0, screen.max.y - BOT_H))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                glass(Surface::Bar).inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
                    ui.set_width(screen.width() - 16.0);
                    ui.set_height(BOT_H - 8.0);
                    ui.spacing_mut().interact_size.y = 28.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = vec2(SPACE_XS, 0.0);
                        for (text, tip) in [(&s.paper, "Paper"), (&s.scale, "Scale of the selected view")] {
                            if text.is_empty() { continue; }
                            let g = ui.painter().layout_no_wrap(text.clone(), meta(), DIM);
                            let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 16.0, 28.0), Sense::hover());
                            ui.painter().galley(pos2(r.min.x + 8.0, r.center().y - g.size().y / 2.0), g, DIM);
                            resp.on_hover_text(tip);
                        }
                        bar_divider(ui);
                        ui.add_space(SPACE_S);
                        if let Some(tex) = &tool_icon {
                            let (r, _) = ui.allocate_exact_size(vec2(22.0, 28.0), Sense::hover());
                            egui::Image::new(tex).tint(INK).paint_at(ui, Rect::from_center_size(r.center(), vec2(22.0, 22.0)));
                            ui.add_space(SPACE_XS);
                        }
                        if !s.tool.is_empty() {
                            ui.label(egui::RichText::new(&s.tool).font(strong(TYPE_BODY)).color(INK));
                            ui.add_space(SPACE_S);
                        }
                        // The right end: the cursor on the paper, then the zoom field.
                        let cursor = ui.painter().layout_no_wrap(s.cursor.clone(), meta(), FAINT);
                        let right_w = cursor.size().x + 16.0 + 9.0 + 64.0 + 4.0 + 40.0;
                        let room = (ui.available_width() - right_w - 8.0).max(40.0);
                        let mut job = egui::text::LayoutJob::single_section(s.hint.clone(), egui::TextFormat {
                            font_id: body(), color: DIM, ..Default::default() });
                        job.wrap.max_width = room;
                        job.wrap.max_rows = 1;
                        job.wrap.break_anywhere = true;
                        let g = ui.painter().layout_job(job);
                        let (r, _) = ui.allocate_exact_size(vec2(room, 28.0), Sense::hover());
                        ui.painter().galley(pos2(r.min.x, r.center().y - g.size().y / 2.0), g, DIM);
                        let (r, _) = ui.allocate_exact_size(vec2(cursor.size().x + 16.0, 28.0), Sense::hover());
                        ui.painter().galley(pos2(r.min.x + 8.0, r.center().y - cursor.size().y / 2.0), cursor, FAINT);
                        bar_divider(ui);
                        // The value field: the zoom, typed (Enter applies).
                        let buf = s.zoom_buf.get_or_insert_with(|| s.zoom.clone());
                        let resp = ui.add(super::kit::field(buf).font(body()).margin(Margin::symmetric(6, 3))
                            .desired_width(52.0).horizontal_align(egui::Align::RIGHT));
                        let resp = resp.on_hover_text("Zoom: type a percentage and press Enter");
                        if resp.lost_focus() {
                            if ui.input(|i| i.key_pressed(egui::Key::Enter)) && buf.trim() != s.zoom {
                                out.push(format!("zoom{SEP}{}", buf.trim()));
                            }
                            s.zoom_buf = None;
                        } else if !resp.has_focus() {
                            s.zoom_buf = None;      // follows the page until typed into
                        }
                        if text_button(ui, "Fit", "Fit the page in the window", true) { out.push("fit".into()); }
                    });
                });
            });
        for a in out { self.actions.push(format!("sheets:{a}")); }
    }
}

/// The inspector's rows. Beyond the model inspector's kinds (plain values,
/// `edit`, `choice`, `color`, drawn by `inspector_rows`), the sheet has:
/// "\u{1d}pick\u{1d}<id>\u{1d}A|B|C\u{1d}<at>": a list to choose from;
/// "\u{1d}check\u{1d}<id>\u{1d}0|1": a switch;
/// "\u{1d}text\u{1d}<id>\u{1d}<text>": several lines, applied with the button
/// or ⌘↩; "\u{1d}button\u{1d}<id>": a command (the label is the button's).
fn sheet_rows(ui: &mut egui::Ui, rows: &[(String, String)], edits: &mut std::collections::HashMap<String, String>,
              pick: &mut Option<(String, Vec<String>, usize, Rect)>, out: &mut Vec<String>) {
    let label_cell = |ui: &mut egui::Ui, label: &str| {
        let (lr, _) = ui.allocate_exact_size(vec2(88.0, CONTROL_H), Sense::hover());
        let mut job = egui::text::LayoutJob::single_section(label.to_owned(), egui::TextFormat {
            font_id: meta(), color: DIM, ..Default::default() });
        job.wrap.max_width = 86.0;
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let g = ui.painter().layout_job(job);
        ui.painter().galley(pos2(lr.min.x, lr.center().y - g.size().y / 2.0), g, DIM);
    };
    let mut buttons: Vec<(String, String)> = Vec::new();
    for (label, value) in rows {
        let parts: Vec<&str> = value.split(ED).collect();
        let kind = if value.starts_with(ED) { parts.get(1).copied().unwrap_or_default() } else { "" };
        let id = parts.get(2).copied().unwrap_or_default();
        match kind {
            "pick" => {
                let options: Vec<String> = parts.get(3).copied().unwrap_or_default().split('|').map(str::to_owned).collect();
                let at = parts.get(4).and_then(|a| a.parse::<usize>().ok()).unwrap_or(0);
                ui.horizontal(|ui| {
                    ui.add_space(SPACE_S);
                    label_cell(ui, label);
                    let (r, resp) = ui.allocate_exact_size(vec2((ui.available_width() - 8.0).max(60.0), CONTROL_H), Sense::click());
                    let open = pick.as_ref().is_some_and(|p| p.0 == id);
                    ui.painter().rect_filled(r, CornerRadius::same(R_ROW), FIELD);
                    paint_state(ui, r, state_of(&resp, open, true), R_ROW);
                    let shown = options.get(at).cloned().unwrap_or_default();
                    let mut job = egui::text::LayoutJob::single_section(shown, egui::TextFormat {
                        font_id: body(), color: INK, ..Default::default() });
                    job.wrap.max_width = (r.width() - 28.0).max(20.0);
                    job.wrap.max_rows = 1;
                    job.wrap.break_anywhere = true;
                    let g = ui.painter().layout_job(job);
                    ui.painter().galley(pos2(r.min.x + 8.0, r.center().y - g.size().y / 2.0), g, INK);
                    ui.painter().text(pos2(r.max.x - 10.0, r.center().y), egui::Align2::CENTER_CENTER, "▾", meta(), DIM);
                    if resp.clicked() {
                        *pick = if open { None } else { Some((id.to_owned(), options.clone(), at, r)) };
                    }
                });
            }
            "check" => {
                let on = parts.get(3).copied() == Some("1");
                ui.horizontal(|ui| {
                    ui.add_space(SPACE_S);
                    label_cell(ui, label);
                    let (r, resp) = ui.allocate_exact_size(vec2((ui.available_width() - 8.0).max(60.0), CONTROL_H), Sense::click());
                    paint_state(ui, r, state_of(&resp, false, true), R_ROW);
                    let b = Rect::from_center_size(pos2(r.min.x + 12.0, r.center().y), vec2(14.0, 14.0));
                    ui.painter().rect_filled(b, CornerRadius::same(4), if on { ACCENT } else { FIELD });
                    ui.painter().rect_stroke(b, CornerRadius::same(4), Stroke::new(1.0_f32, if on { ACCENT } else { EDGE }), egui::StrokeKind::Inside);
                    if on { ui.painter().text(b.center(), egui::Align2::CENTER_CENTER, "✓", meta(), BRIGHT); }
                    ui.painter().text(pos2(b.max.x + 8.0, r.center().y), egui::Align2::LEFT_CENTER, if on { "On" } else { "Off" }, body(), INK);
                    if resp.clicked() { out.push(format!("edit{SEP}{id}{SEP}{}", if on { 0 } else { 1 })); }
                });
            }
            "text" => {
                let text = parts.get(3).copied().unwrap_or_default();
                let buf = edits.entry(id.to_owned()).or_insert_with(|| text.to_owned());
                ui.horizontal(|ui| {
                    ui.add_space(SPACE_S);
                    label_cell(ui, label);
                });
                ui.horizontal(|ui| {
                    ui.add_space(SPACE_S);
                    let w = (ui.available_width() - 8.0 - 12.0 - 4.0).max(40.0);
                    let r = ui.add(egui::TextEdit::multiline(buf).font(body()).margin(Margin::symmetric(6, 4))
                        .desired_width(w).desired_rows(3));
                    let commit = r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command);
                    if commit && buf.as_str() != text {
                        out.push(format!("edit{SEP}{id}{SEP}{}", buf.trim_end_matches('\n')));
                    }
                });
                if buf.as_str() != text {
                    let pending = buf.clone();
                    ui.horizontal(|ui| {
                        ui.add_space(SPACE_S);
                        if chip_button(ui, "Apply  ⌘↩") { out.push(format!("edit{SEP}{id}{SEP}{pending}")); }
                        ui.add_space(SPACE_XS);
                        if chip_button(ui, "Revert") { edits.remove(id); }
                    });
                }
            }
            "button" => buttons.push((id.to_owned(), label.clone())),
            _ => {
                let mut acts = Vec::new();
                inspector_rows(ui, std::slice::from_ref(&(label.clone(), value.clone())), edits, &mut acts);
                out.extend(acts);
            }
        }
    }
    if !buttons.is_empty() {
        ui.add_space(SPACE_XS);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(SPACE_XS, SPACE_XS);
            ui.add_space(SPACE_S);
            for (id, label) in buttons {
                if chip_button(ui, &label) { out.push(format!("button{SEP}{id}")); }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(v: &[(&str, &str)]) -> Vec<(String, Vec<(String, String)>)> {
        vec![(String::new(), v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect())]
    }

    #[test]
    fn typing_survives_a_resend_but_not_another_item() {
        let mut s = Sheets::default();
        s.set_inspector("Text".into(), rows(&[("Text", "\u{1d}text\u{1d}text\u{1d}Hello")]));
        s.edits.insert("text".into(), "Hello there".into());
        // Another row changed (the size), the text's own row did not.
        s.set_inspector("Text".into(), rows(&[("Text", "\u{1d}text\u{1d}text\u{1d}Hello"), ("Size", "\u{1d}edit\u{1d}size\u{1d}12")]));
        assert_eq!(s.edits.get("text").map(String::as_str), Some("Hello there"));
        // The text itself changed underneath (undo): the typing goes.
        s.set_inspector("Text".into(), rows(&[("Text", "\u{1d}text\u{1d}text\u{1d}Bye")]));
        assert!(s.edits.is_empty());
        s.edits.insert("text".into(), "x".into());
        s.set_inspector("Model view".into(), vec![]);
        assert!(s.edits.is_empty());
    }

    #[test]
    fn popups_hold_escape() {
        let mut s = Sheets::default();
        assert!(!s.popup_open());
        s.row_menu = Some((0, Pos2::ZERO));
        assert!(s.popup_open());
        s.close_popups();
        assert!(!s.popup_open());
        s.prompt = Some(("Delete?".into(), String::new(), None, vec![("yes".into(), "Delete".into())]));
        assert!(s.popup_open());
    }
}
