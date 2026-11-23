//! The Library: a typed resource browser in the dock (materials, colours,
//! components, later profiles…). Python owns the catalogue (providers in
//! `views/wgpu_library.py`): it sends the type tabs and the rows already
//! filtered, sorted and folded; this draws them as a grid of thumbnails or a
//! dense list, keeps the keyboard cursor, and reports picks as
//! `lib:<verb>\x1f<key>[\x1f<arg>]` actions.
//!
//! Also home to the two tab controls the dock shares (Maxence 2026-11-22:
//! tab rows drawn as loose buttons "feel like a hack"): a segmented switch
//! for the dock's places (Project | Library) and underline tabs for a
//! second level (column sets, resource types).

use egui::{pos2, vec2, Color32, CornerRadius, Margin, Pos2, Rect, Sense, Stroke};

use super::{BRIGHT, IMAGE_HOVER, body, caption_plain, glass, menu_row, meta, paint_state, parse_hex, state_of, strong, Surface,
            ACCENT, ACTIVE, DIM, EDGE, FAINT, FIELD, HOVER, INK, INK_2, OFF, R_CONTROL, R_ROW, State};

const ROW: f32 = 22.0;
/// Grid tile: thumbnail edge, and the name line under it.
const THUMB: f32 = 48.0;
const TILE_TEXT: f32 = 14.0;
const SEP: char = '\u{1f}';

/// One resource type tab: id, label, count, ready (false: "coming").
pub struct TypeTab {
    pub id: String,
    pub label: String,
    pub count: u32,
    pub ready: bool,
}

/// A row as Python sends it, in display order.
pub struct Entry {
    /// 0 section (In this model, My library, Bundled), 1 category, 2 item,
    /// 3 note (a sentence in place of items).
    pub kind: u8,
    pub key: String,
    pub name: String,
    /// Items: category · source (tooltip); headers: unused.
    pub meta: String,
    pub hex: String,
    pub icon: String,
    /// Item verbs for its menu, comma separated (ids from `verbs`).
    pub verbs: String,
    /// Items: uses in the model; headers: items under it.
    pub count: u32,
    /// Items: the current paint (ringed).
    pub active: bool,
    pub expanded: bool,
}

#[derive(Default)]
pub struct Library {
    pub types: Vec<TypeTab>,
    pub active_type: String,
    pub entries: Vec<Entry>,
    /// 0 grid, 1 list.
    pub view: u8,
    pub sort: String,
    pub sorts: Vec<(String, String)>,
    /// Verb id → label, for the item menu.
    pub verbs: Vec<(String, String)>,
    /// The type's own commands (⋯): id, label.
    pub type_actions: Vec<(String, String)>,
    /// What an empty type says.
    pub empty: String,
    pub filter: String,
    pub focus_filter: bool,
    filter_focused: bool,
    /// The keyboard cursor (an item key) and whether to bring it into view.
    pub cursor: Option<String>,
    scroll_to_cursor: bool,
    /// Tiles per grid row, as last laid out.
    cols: usize,
    /// Open menu: "item:<key>", "sort", "more"; and where.
    pub menu: Option<(String, Pos2)>,
    pub renaming: Option<(String, String)>,
}

impl Library {
    /// rows: ([key, name, meta, hex, icon, verbs], [kind, count, active, expanded]).
    #[allow(clippy::too_many_arguments)]
    pub fn set(&mut self, types: Vec<(String, String, u32, bool)>, active: String, view: u8, sort: String,
               sorts: Vec<(String, String)>, verbs: Vec<(String, String)>, type_actions: Vec<(String, String)>,
               empty: String, filter: String, rows: Vec<(Vec<String>, Vec<i64>)>) {
        if active != self.active_type {
            self.cursor = None;
            self.menu = None;
            self.renaming = None;
        }
        self.types = types.into_iter().map(|(id, label, count, ready)| TypeTab { id, label, count, ready }).collect();
        self.active_type = active;
        self.view = view.min(1);
        self.sort = sort;
        self.sorts = sorts;
        self.verbs = verbs;
        self.type_actions = type_actions;
        self.empty = empty;
        // Python echoes the filter it applied; while the field is being
        // typed in, what is typed wins.
        if !self.filter_focused { self.filter = filter; }
        let s = |v: &Vec<String>, i: usize| v.get(i).cloned().unwrap_or_default();
        let g = |v: &Vec<i64>, i: usize| v.get(i).copied().unwrap_or(0);
        self.entries = rows.into_iter().map(|(t, n)| Entry {
            key: s(&t, 0), name: s(&t, 1), meta: s(&t, 2), hex: s(&t, 3), icon: s(&t, 4), verbs: s(&t, 5),
            kind: g(&n, 0) as u8, count: g(&n, 1).max(0) as u32, active: g(&n, 2) != 0, expanded: g(&n, 3) != 0,
        }).collect();
        if let Some(c) = &self.cursor {
            if !self.entries.iter().any(|e| e.kind == 2 && &e.key == c) { self.cursor = None; }
        }
    }

    /// Opened from somewhere (Paint strip, ⌘K, the rail): typing filters.
    pub fn opened(&mut self) {
        self.focus_filter = true;
        self.menu = None;
    }

    fn items(&self) -> impl Iterator<Item = &Entry> { self.entries.iter().filter(|e| e.kind == 2) }

    /// The keys the library owns while its filter has focus: arrows move,
    /// Enter applies. Esc unwinds: menu, rename, filter text, then the
    /// caller leaves the Library. Returns Some(action) or Some("") when the
    /// key was taken without an action; None when it is not the library's.
    pub fn key(&mut self, name: &str) -> Option<String> {
        if name == "Escape" {
            if self.menu.take().is_some() || self.renaming.take().is_some() { return Some(String::new()); }
            if !self.filter.is_empty() {
                self.filter.clear();
                return Some(format!("filter{SEP}"));
            }
            return None;
        }
        if !self.filter_focused || self.renaming.is_some() { return None; }
        let keys: Vec<String> = self.items().map(|e| e.key.clone()).collect();
        if keys.is_empty() { return matches!(name, "ArrowUp" | "ArrowDown" | "Enter").then(String::new); }
        let at = self.cursor.as_ref().and_then(|c| keys.iter().position(|k| k == c));
        let step = |d: i64| -> usize {
            match at {
                None => if d < 0 { keys.len() - 1 } else { 0 },
                Some(i) => (i as i64 + d).clamp(0, keys.len() as i64 - 1) as usize,
            }
        };
        let row = if self.view == 0 { self.cols.max(1) as i64 } else { 1 };
        let to = match name {
            "ArrowDown" => step(row),
            "ArrowUp" => step(-row),
            // In the list the caret keeps Left/Right; in the grid they move.
            "ArrowRight" if self.view == 0 => step(1),
            "ArrowLeft" if self.view == 0 => step(-1),
            "Enter" => {
                let key = keys[at.unwrap_or(0)].clone();
                self.cursor = Some(key.clone());
                return Some(format!("apply{SEP}{key}"));
            }
            _ => return None,
        };
        self.cursor = Some(keys[to].clone());
        self.scroll_to_cursor = true;
        Some(format!("focus{SEP}{}", keys[to]))
    }

    /// The browser: type tabs, the filter row, then the rows.
    pub fn show(&mut self, ui: &mut egui::Ui, icons: &std::collections::HashMap<String, egui::TextureHandle>,
                height: f32, out: &mut Vec<String>) {
        // Resource types: underline tabs with their counts.
        let tabs: Vec<(String, String, bool)> = self.types.iter()
            .map(|t| (t.label.clone(), if t.ready { t.count.to_string() } else { "soon".into() }, t.ready)).collect();
        let at = self.types.iter().position(|t| t.id == self.active_type).unwrap_or(0);
        if let Some(i) = underline_tabs(ui, &tabs, at) {
            out.push(format!("type{SEP}{}", self.types[i].id));
            self.focus_filter = true;
        }
        ui.add_space(6.0);
        // Filter, then grid/list, sort and the type's commands.
        let label = self.types.get(at).map(|t| t.label.to_lowercase()).unwrap_or_default();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 0.0);
            // Two toggles, sort, ⋯, four gaps, and the field's own margins.
            let tools_w = 24.0 * 2.0 + 62.0 + 24.0 + 4.0 * 4.0 + 16.0 + 2.0;
            let edit = egui::TextEdit::singleline(&mut self.filter)
                .hint_text(egui::RichText::new(format!("Filter {label}")).color(FAINT))
                .margin(Margin::symmetric(8, 5))
                .desired_width((ui.available_width() - tools_w).max(60.0))
                .font(egui::FontId::proportional(12.0));
            let resp = ui.add(edit);
            if self.focus_filter {
                resp.request_focus();
                self.focus_filter = false;
            }
            self.filter_focused = resp.has_focus();
            if resp.changed() { out.push(format!("filter{SEP}{}", self.filter)); }
            if icon_toggle(ui, self.view == 0, "Grid", paint_grid_glyph) && self.view != 0 { out.push(format!("view{SEP}grid")); }
            if icon_toggle(ui, self.view == 1, "List", paint_list_glyph) && self.view != 1 { out.push(format!("view{SEP}list")); }
            let first = self.sort.split(',').next().unwrap_or_default().to_owned();
            let sort_label = self.sorts.iter().find(|(id, _)| *id == first).map(|(_, l)| l.clone()).unwrap_or_default();
            if let Some(r) = small_menu_button(ui, &sort_label, 62.0, "Sort") {
                self.menu = Some(("sort".into(), pos2(r.min.x, r.max.y + 4.0)));
            }
            let (r, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
            paint_state(ui, r, state_of(&resp, false, !self.type_actions.is_empty()), R_CONTROL);
            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "⋯", body(),
                if self.type_actions.is_empty() { OFF } else { INK_2 });
            if resp.on_hover_text("Commands for this type").clicked() && !self.type_actions.is_empty() {
                self.menu = Some(("more".into(), pos2(r.max.x, r.max.y + 4.0)));
            }
        });
        ui.add_space(6.0);
        if self.entries.is_empty() {
            ui.add_space(8.0);
            let text = if self.filter.is_empty() { self.empty.clone() } else { format!("Nothing matches “{}”.", self.filter) };
            ui.horizontal_wrapped(|ui| {
                ui.add_space(8.0);
                ui.set_max_width(ui.available_width() - 8.0);
                ui.label(egui::RichText::new(text).font(body()).color(DIM));
            });
            ui.allocate_space(vec2(ui.available_width(), (height - 120.0).max(0.0)));
            return;
        }
        let scroll_to = std::mem::take(&mut self.scroll_to_cursor);
        egui::ScrollArea::vertical().id_salt("library").max_height(height.max(80.0)).auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                let width = ui.available_width();
                let cols = if self.view == 0 { ((width + 4.0) / (THUMB + 8.0)).floor().max(1.0) as usize } else { 1 };
                self.cols = cols;
                let tile_w = (width - (cols as f32 - 1.0) * 4.0) / cols as f32;
                let mut i = 0;
                while i < self.entries.len() {
                    let kind = self.entries[i].kind;
                    if kind == 2 && self.view == 0 {
                        // A run of items: tiles, row by row.
                        let start = i;
                        while i < self.entries.len() && self.entries[i].kind == 2 { i += 1; }
                        for chunk in (start..i).collect::<Vec<_>>().chunks(cols) {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing = vec2(4.0, 0.0);
                                for &j in chunk {
                                    self.tile(ui, j, tile_w, icons, scroll_to, out);
                                }
                            });
                            ui.add_space(4.0);
                        }
                        continue;
                    }
                    match kind {
                        0 | 1 => self.header(ui, i, out),
                        2 => self.list_row(ui, i, icons, scroll_to, out),
                        _ => {
                            let e = &self.entries[i];
                            ui.horizontal_wrapped(|ui| {
                                ui.add_space(22.0);
                                ui.set_max_width(width - 30.0);
                                ui.label(egui::RichText::new(&e.name).font(meta()).color(FAINT));
                            });
                            ui.add_space(6.0);
                        }
                    }
                    i += 1;
                }
                ui.add_space(4.0);
            });
    }

    /// A section (caps caption) or a category (name) with its count; both fold.
    fn header(&mut self, ui: &mut egui::Ui, i: usize, out: &mut Vec<String>) {
        let e = &self.entries[i];
        if e.kind == 0 && i > 0 { ui.add_space(6.0); }
        let h = if e.kind == 0 { 24.0 } else { ROW };
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
        if resp.hovered() { ui.painter().rect_filled(rect, CornerRadius::same(R_ROW), HOVER); }
        let cy = rect.center().y;
        ui.painter().text(pos2(rect.min.x + 11.0, cy), egui::Align2::CENTER_CENTER, if e.expanded { "▾" } else { "▸" },
            meta(), FAINT);
        let count = ui.painter().layout_no_wrap(e.count.to_string(), meta(), DIM);
        let cw = count.size().x;
        ui.painter().galley(pos2(rect.max.x - 8.0 - cw, cy - count.size().y / 2.0), count, DIM);
        if e.kind == 0 {
            let job = egui::text::LayoutJob::single_section(e.name.to_uppercase(), egui::TextFormat {
                font_id: strong(10.0), color: FAINT, extra_letter_spacing: 0.6, ..Default::default() });
            let g = ui.painter().layout_job(job);
            ui.painter().galley(pos2(rect.min.x + 22.0, cy - g.size().y / 2.0), g, FAINT);
        } else {
            let g = ui.painter().layout_job(one_line(e.name.clone(), body(), INK_2, rect.width() - 40.0 - cw));
            ui.painter().galley(pos2(rect.min.x + 22.0, cy - g.size().y / 2.0), g, INK_2);
        }
        if resp.clicked() { out.push(format!("toggle{SEP}{}", e.key)); }
    }

    fn lit(&self, e: &Entry) -> bool { self.cursor.as_deref() == Some(e.key.as_str()) }

    fn list_row(&mut self, ui: &mut egui::Ui, i: usize, icons: &std::collections::HashMap<String, egui::TextureHandle>,
                scroll_to: bool, out: &mut Vec<String>) {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
        let e = &self.entries[i];
        let lit = self.lit(e);
        if lit {
            ui.painter().rect_filled(rect, CornerRadius::same(R_ROW), ACTIVE);
            if scroll_to { ui.scroll_to_rect(rect, None); }
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(R_ROW), HOVER);
        }
        let cy = rect.center().y;
        let sw = Rect::from_center_size(pos2(rect.min.x + 30.0, cy), vec2(16.0, 16.0));
        paint_thumb(ui, sw, e, icons, 3);
        if e.active {
            ui.painter().rect_stroke(sw.expand(2.0), CornerRadius::same(4), Stroke::new(1.5_f32, ACCENT), egui::StrokeKind::Outside);
        }
        let right = rect.max.x - 8.0;
        let uses = (e.count > 0).then(|| ui.painter().layout_no_wrap(format!("{}", e.count), meta(), DIM));
        let uw = uses.as_ref().map_or(0.0, |g| g.size().x + 10.0);
        let x = sw.max.x + 8.0;
        if self.renaming.as_ref().is_some_and(|(k, _)| *k == e.key) {
            self.rename_field(ui, Rect::from_min_max(pos2(x - 4.0, rect.min.y + 1.0), pos2(right, rect.max.y - 1.0)), out);
            return;
        }
        let ink = if lit { BRIGHT } else { INK_2 };
        let g = ui.painter().layout_job(one_line(e.name.clone(), body(), ink, right - uw - x));
        ui.painter().galley(pos2(x, cy - g.size().y / 2.0), g, ink);
        if let Some(g) = uses {
            ui.painter().galley(pos2(right - g.size().x, cy - g.size().y / 2.0), g, DIM);
        }
        self.item_response(resp, i, out);
    }

    fn tile(&mut self, ui: &mut egui::Ui, i: usize, w: f32, icons: &std::collections::HashMap<String, egui::TextureHandle>,
            scroll_to: bool, out: &mut Vec<String>) {
        let (rect, resp) = ui.allocate_exact_size(vec2(w, THUMB + TILE_TEXT + 6.0), Sense::click());
        let e = &self.entries[i];
        let lit = self.lit(e);
        if lit {
            ui.painter().rect_filled(rect, CornerRadius::same(R_ROW), ACTIVE);
            if scroll_to { ui.scroll_to_rect(rect, None); }
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(R_ROW), HOVER);
        }
        let thumb = Rect::from_center_size(pos2(rect.center().x, rect.min.y + 2.0 + (THUMB - 4.0) / 2.0),
                                           vec2(THUMB - 4.0, THUMB - 4.0));
        paint_thumb(ui, thumb, e, icons, 5);
        if e.active {
            ui.painter().rect_stroke(thumb.expand(2.0), CornerRadius::same(6), Stroke::new(1.5_f32, ACCENT), egui::StrokeKind::Outside);
        }
        if e.count > 0 {
            // Uses, as a small badge on the thumbnail's corner.
            let g = ui.painter().layout_no_wrap(e.count.to_string(), egui::FontId::proportional(9.5), INK);
            let b = Rect::from_min_size(pos2(thumb.max.x - g.size().x - 6.0, thumb.max.y - 14.0), vec2(g.size().x + 6.0, 13.0));
            ui.painter().rect_filled(b, CornerRadius::same(4), IMAGE_HOVER);
            ui.painter().galley(b.center() - g.size() / 2.0, g, INK);
        }
        let text_r = Rect::from_min_max(pos2(rect.min.x + 2.0, thumb.max.y + 3.0), pos2(rect.max.x - 2.0, rect.max.y));
        if self.renaming.as_ref().is_some_and(|(k, _)| *k == e.key) {
            self.rename_field(ui, text_r, out);
            return;
        }
        let ink = if lit { BRIGHT } else { INK_2 };
        let g = ui.painter().layout_job(one_line(e.name.clone(), egui::FontId::proportional(10.5), ink, text_r.width()));
        ui.painter().galley(pos2(text_r.center().x - g.size().x / 2.0, text_r.min.y), g, ink);
        self.item_response(resp, i, out);
    }

    fn item_response(&mut self, resp: egui::Response, i: usize, out: &mut Vec<String>) {
        let e = &self.entries[i];
        let tip = if e.meta.is_empty() { e.name.clone() } else { format!("{}\n{}", e.name, e.meta) };
        let resp = resp.on_hover_text(tip);
        if resp.double_clicked() {
            out.push(format!("edit{SEP}{}", e.key));
        } else if resp.clicked() {
            self.cursor = Some(e.key.clone());
            out.push(format!("apply{SEP}{}", e.key));
        }
        if resp.secondary_clicked() && !e.verbs.is_empty() {
            self.cursor = Some(e.key.clone());
            out.push(format!("focus{SEP}{}", e.key));
            self.menu = resp.interact_pointer_pos().map(|p| (format!("item:{}", e.key), p + vec2(4.0, 4.0)));
        }
    }

    fn rename_field(&mut self, ui: &mut egui::Ui, at: Rect, out: &mut Vec<String>) {
        let Some((_, buf)) = self.renaming.as_mut() else { return };
        let field = egui::TextEdit::singleline(buf).font(body()).margin(Margin::symmetric(4, 1)).clip_text(true);
        let r = ui.put(at, field);
        r.request_focus();
        if r.lost_focus() {
            let (key, name) = self.renaming.take().unwrap();
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) && !name.trim().is_empty() {
                out.push(format!("rename{SEP}{key}{SEP}{}", name.trim()));
            }
            self.focus_filter = true;
        }
    }

    /// Menus over the dock: an item's verbs, the sorts, the type's commands.
    pub fn popups(&mut self, ctx: &egui::Context, out: &mut Vec<String>) {
        let Some((what, at)) = self.menu.clone() else { return };
        let mut close = false;
        let rows: Vec<(String, String, i8)> = if let Some(key) = what.strip_prefix("item:") {
            let verbs = self.entries.iter().find(|e| e.key == key).map(|e| e.verbs.clone()).unwrap_or_default();
            verbs.split(',').filter(|v| !v.is_empty()).map(|v| {
                if v == "-" { return ("-".into(), String::new(), -1); }
                let label = self.verbs.iter().find(|(id, _)| id == v).map(|(_, l)| l.clone()).unwrap_or_else(|| v.to_owned());
                (format!("{v}{SEP}{key}"), label, -1)
            }).collect()
        } else if what == "sort" {
            // Sorts, then (after "-") facets; `sort` names the ones in force: "name,facet:all".
            let on: Vec<&str> = self.sort.split(',').collect();
            self.sorts.iter().map(|(id, l)| {
                if id == "-" { ("-".to_owned(), String::new(), -1) }
                else { (format!("sort{SEP}{id}"), l.clone(), on.contains(&id.as_str()) as i8) }
            }).collect()
        } else {
            self.type_actions.iter().map(|(id, l)| (format!("typeact{SEP}{id}"), l.clone(), -1)).collect()
        };
        let pivot = if what == "more" { egui::Align2::RIGHT_TOP } else { egui::Align2::LEFT_TOP };
        egui::Area::new(egui::Id::new("lib_menu"))
            .pivot(pivot)
            .fixed_pos(at)
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                glass(Surface::Raised).inner_margin(Margin::same(6)).show(ui, |ui| {
                    ui.set_width(if what == "sort" { 160.0 } else { 210.0 });
                    ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                    for (id, label, checked) in &rows {
                        if id == "-" {
                            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 7.0), Sense::hover());
                            ui.painter().hline(r.x_range().shrink(6.0), r.center().y, Stroke::new(1.0_f32, EDGE));
                            continue;
                        }
                        let lane = if what == "sort" { "check" } else { "" };
                        if menu_row(ui, &(id.clone(), label.clone(), String::new(), lane.to_owned(), *checked, true)) {
                            if let Some(key) = id.strip_prefix(&format!("rename{SEP}")) {
                                let name = self.entries.iter().find(|e| e.key == key).map(|e| e.name.clone()).unwrap_or_default();
                                self.renaming = Some((key.to_owned(), name));
                            } else {
                                out.push(id.clone());
                            }
                            close = true;
                        }
                    }
                });
            });
        if close { self.menu = None; }
    }

    pub fn menu_open(&self) -> bool { self.menu.is_some() }
}

/// Swatch, texture thumbnail, or the component glyph.
fn paint_thumb(ui: &egui::Ui, r: Rect, e: &Entry, icons: &std::collections::HashMap<String, egui::TextureHandle>, radius: u8) {
    if let Some(tex) = icons.get(&e.icon).filter(|_| !e.icon.is_empty()) {
        egui::Image::new(tex).corner_radius(CornerRadius::same(radius)).paint_at(ui, r);
    } else if let Some(c) = parse_hex(&e.hex) {
        ui.painter().rect_filled(r, CornerRadius::same(radius), c);
    } else {
        ui.painter().rect_filled(r, CornerRadius::same(radius), FIELD);
        paint_cube(ui, r.shrink(r.width() * 0.22), if r.width() > 20.0 { DIM } else { FAINT });
    }
    ui.painter().rect_stroke(r, CornerRadius::same(radius), Stroke::new(1.0_f32, EDGE), egui::StrokeKind::Inside);
}

/// A component's stand-in: an isometric box outline.
fn paint_cube(ui: &egui::Ui, r: Rect, col: Color32) {
    let c = r.center();
    let w = r.width() / 2.0;
    let h = w * 0.55;
    let top = [pos2(c.x, c.y - w), pos2(c.x + w, c.y - w + h), pos2(c.x, c.y - w + 2.0 * h), pos2(c.x - w, c.y - w + h)];
    let s = Stroke::new(if r.width() > 14.0 { 1.3_f32 } else { 1.0_f32 }, col);
    ui.painter().add(egui::Shape::closed_line(top.to_vec(), s));
    let d = w * 0.9;
    for p in [top[1], top[2], top[3]] {
        ui.painter().line_segment([p, p + vec2(0.0, d)], s);
    }
    ui.painter().line_segment([top[1] + vec2(0.0, d), top[2] + vec2(0.0, d)], s);
    ui.painter().line_segment([top[3] + vec2(0.0, d), top[2] + vec2(0.0, d)], s);
}

fn one_line(text: String, font: egui::FontId, color: Color32, width: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::single_section(text, egui::TextFormat { font_id: font, color, ..Default::default() });
    job.wrap.max_width = width.max(10.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job
}

fn paint_grid_glyph(ui: &egui::Ui, r: Rect, col: Color32) {
    for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let c = r.center() + vec2(dx * 3.2, dy * 3.2);
        ui.painter().rect_filled(Rect::from_center_size(c, vec2(5.0, 5.0)), CornerRadius::same(1), col);
    }
}

fn paint_list_glyph(ui: &egui::Ui, r: Rect, col: Color32) {
    for dy in [-4.0, 0.0, 4.0] {
        let y = r.center().y + dy;
        ui.painter().line_segment([pos2(r.center().x - 6.0, y), pos2(r.center().x + 6.0, y)], Stroke::new(1.4_f32, col));
    }
}

/// A 24-pt latched icon button (grid / list).
fn icon_toggle(ui: &mut egui::Ui, on: bool, tip: &str, glyph: fn(&egui::Ui, Rect, Color32)) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
    let st = state_of(&resp, on, true);
    paint_state(ui, r, st, R_CONTROL);
    glyph(ui, r, if on { BRIGHT } else if st == State::Hover { INK } else { DIM });
    resp.on_hover_text(tip).clicked()
}

/// A compact menu button (sort): label and ▾, fixed width.
fn small_menu_button(ui: &mut egui::Ui, label: &str, w: f32, tip: &str) -> Option<Rect> {
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
    paint_state(ui, rect, state_of(&resp, false, true), R_CONTROL);
    let g = ui.painter().layout_job(one_line(label.to_owned(), meta(), INK_2, w - 22.0));
    ui.painter().galley(pos2(rect.min.x + 8.0, rect.center().y - g.size().y / 2.0), g, INK_2);
    ui.painter().text(pos2(rect.max.x - 9.0, rect.center().y), egui::Align2::CENTER_CENTER, "▾", meta(), FAINT);
    let resp = resp.on_hover_text(tip);
    resp.clicked().then_some(rect)
}

pub use super::kit::{segmented, underline_tabs};

/// The dock header's place switch and, under Project, its mode (Problems)
/// with a way back. Used by `Ui::right_column`.
pub fn caption_mode(ui: &mut egui::Ui, mode: &str) -> bool {
    ui.add_space(8.0);
    caption_plain(ui, mode);
    let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
    paint_state(ui, r, state_of(&resp, false, true), R_ROW);
    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "×", body(), if resp.hovered() { INK } else { FAINT });
    resp.on_hover_text("Back to the tree (Esc)").clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib(view: u8) -> Library {
        let mut l = Library::default();
        let row = |kind: i64, key: &str| (vec![key.to_owned(), key.to_owned()], vec![kind, 0, 0, 1]);
        l.set(vec![("materials".into(), "Materials".into(), 3, true)], "materials".into(), view, "name".into(),
              vec![], vec![], vec![], String::new(), String::new(),
              vec![row(0, "s:model"), row(2, "a"), row(2, "b"), row(2, "c"), row(0, "s:bundled"), row(1, "c:x"), row(2, "d")]);
        l.filter_focused = true;
        l.cols = 2;
        l
    }

    #[test]
    fn arrows_walk_items_and_skip_headers() {
        let mut l = lib(1);
        assert_eq!(l.key("ArrowDown").as_deref(), Some("focus\u{1f}a"));
        assert_eq!(l.key("ArrowDown").as_deref(), Some("focus\u{1f}b"));
        l.key("ArrowDown");
        assert_eq!(l.key("ArrowDown").as_deref(), Some("focus\u{1f}d"));
        assert_eq!(l.key("ArrowDown").as_deref(), Some("focus\u{1f}d"), "stops at the last item");
        assert_eq!(l.key("ArrowUp").as_deref(), Some("focus\u{1f}c"));
        // The list keeps Left/Right for the filter's caret.
        assert_eq!(l.key("ArrowLeft"), None);
        assert_eq!(l.key("Enter").as_deref(), Some("apply\u{1f}c"));
    }

    #[test]
    fn grid_rows_move_by_columns() {
        let mut l = lib(0);
        l.key("ArrowRight");
        assert_eq!(l.cursor.as_deref(), Some("a"));
        assert_eq!(l.key("ArrowDown").as_deref(), Some("focus\u{1f}c"));
        assert_eq!(l.key("ArrowLeft").as_deref(), Some("focus\u{1f}b"));
    }

    #[test]
    fn escape_unwinds_menu_then_filter_then_leaves() {
        let mut l = lib(1);
        l.menu = Some(("sort".into(), Pos2::ZERO));
        l.filter = "oak".into();
        assert_eq!(l.key("Escape").as_deref(), Some(""));
        assert!(l.menu.is_none());
        assert_eq!(l.key("Escape").as_deref(), Some("filter\u{1f}"));
        assert!(l.filter.is_empty());
        assert_eq!(l.key("Escape"), None, "nothing left: the dock leaves the Library");
    }

    #[test]
    fn keys_pass_through_when_the_filter_has_no_focus() {
        let mut l = lib(1);
        l.filter_focused = false;
        assert_eq!(l.key("ArrowDown"), None);
        assert_eq!(l.key("Enter"), None);
    }
}
