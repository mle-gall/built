//! The start page: a full-window state of the chrome (views/wgpu_start.py
//! owns its data and its actions). Shown when the window opens without a
//! file, and again from the document menu, ⌘K or File ▸ Home. While it is
//! up, nothing else of the chrome is drawn except the command palette,
//! the jobs capsule (an open in progress) and the sheet.
//!
//! Python sends rows; the page lays them out, filters them as you type,
//! moves a focus ring with the arrows and reports what was chosen as
//! `start:<verb>\u{1f}<arg>` actions.

use super::*;

/// One recent file: (path, name, meta line, project, thumbnail icon key,
/// pinned, missing).
#[derive(Clone, Default)]
pub struct Tile {
    pub path: String,
    pub name: String,
    pub meta: String,
    pub project: String,
    pub thumb: String,
    pub pinned: bool,
    pub missing: bool,
}

/// One recent project and its files: (path, name, meta, pinned, missing,
/// [(path, name, role)]).
#[derive(Clone, Default)]
pub struct Project {
    pub path: String,
    pub name: String,
    pub meta: String,
    pub pinned: bool,
    pub missing: bool,
    pub files: Vec<(String, String, String)>,
}

/// A template: id, name, one-line description.
#[derive(Clone, Default)]
pub struct Template {
    pub id: String,
    pub name: String,
    pub desc: String,
}

/// The document to continue: path, name, where (project · folder), last
/// in (workspace), problems ("13 items to fix" or ""), auto-save notice
/// ("" when the file is the newest), thumbnail key, and whether it is the
/// document open behind the page (then Continue just returns to it).
#[derive(Clone, Default)]
pub struct Continue {
    pub path: String,
    pub name: String,
    pub place: String,
    pub last_in: String,
    pub problems: String,
    pub autosave: String,
    pub thumb: String,
    pub open: bool,
}

#[derive(Default)]
pub struct Start {
    pub shown: bool,
    /// A document is open behind the page: Esc returns to it.
    pub can_return: bool,
    pub files: Vec<Tile>,
    pub projects: Vec<Project>,
    pub templates: Vec<Template>,
    pub cont: Option<Continue>,
    pub footer: String,
    /// Side sections: (id, label). "recent", "projects", "templates" are
    /// the page's own; any other id is reported as `start:nav`.
    pub nav: Vec<(String, String)>,
    /// Side bottom: (id, label), reported as `start:cmd`.
    pub links: Vec<(String, String)>,
    pub section: String,
    pub filter: String,
    /// The focused tile, by its action ("cont", "open\u{1f}<path>", …).
    pub selected: Option<String>,
    /// The focusable tiles as last laid out, in reading order.
    pub focus: Vec<(String, Rect)>,
    pub scroll_to: bool,
    pub expanded: std::collections::HashSet<String>,
    /// A tile's right-click menu: (path, pinned, where).
    pub menu: Option<(String, bool, Pos2)>,
}

const SEP: char = '\u{1f}';
const SIDE_W: f32 = 232.0;
const TILE_W: f32 = 184.0;
const THUMB_H: f32 = 115.0;
const TILE_GAP: f32 = 12.0;
const PAD: f32 = 28.0;

// Page surfaces: the document area's graphite (#16191D) and one step up
// for the side column and cards. Opaque: the page is not over a model.
const PAGE: Color32 = Color32::from_rgb(0x16, 0x19, 0x1D);
const SIDE: Color32 = Color32::from_rgb(0x1B, 0x1F, 0x24);
const CARD: Color32 = Color32::from_rgb(0x1E, 0x22, 0x28);
const CARD_HOVER: Color32 = Color32::from_rgb(0x24, 0x29, 0x30);
const WELL: Color32 = Color32::from_rgb(0x10, 0x12, 0x15);

impl Start {
    fn words(&self) -> Vec<String> {
        self.filter.to_lowercase().split_whitespace().map(str::to_owned).collect()
    }
    fn matches(words: &[String], hay: &[&str]) -> bool {
        if words.is_empty() { return true; }
        let hay = hay.join(" ").to_lowercase();
        words.iter().all(|w| hay.contains(w.as_str()))
    }
    pub fn visible_files(&self) -> Vec<&Tile> {
        let w = self.words();
        self.files.iter().filter(|t| Self::matches(&w, &[&t.name, &t.meta, &t.project, &t.path])).collect()
    }
    pub fn visible_projects(&self) -> Vec<&Project> {
        let w = self.words();
        self.projects.iter().filter(|p| {
            Self::matches(&w, &[&p.name, &p.meta, &p.path]) || p.files.iter().any(|f| Self::matches(&w, &[&f.1]))
        }).collect()
    }
    pub fn visible_templates(&self) -> Vec<&Template> {
        let w = self.words();
        self.templates.iter().filter(|t| Self::matches(&w, &[&t.name, &t.desc])).collect()
    }
    fn section(&self) -> &str { if self.section.is_empty() { "recent" } else { &self.section } }

    /// What Enter does on a focus id.
    fn activate(id: &str) -> String {
        match id.split_once(SEP) {
            Some(("tpl", t)) => format!("template{SEP}{t}"),
            Some((_, p)) => format!("open{SEP}{p}"),
            None if id == "cont" => "continue".into(),
            None => id.to_owned(),
        }
    }

    /// A key while the page is up (no ⌘/⌃: those stay the window's).
    /// Returns (taken, action without the "start:" prefix).
    pub fn key(&mut self, name: &str, text: &str) -> (bool, Option<String>) {
        match name {
            "Escape" => {
                if self.menu.take().is_some() {
                    return (true, None);
                }
                if !self.filter.is_empty() {
                    self.filter.clear();
                    self.selected = None;
                    return (true, None);
                }
                (true, self.can_return.then(|| "close".to_owned()))
            }
            "Enter" => {
                let id = self.selected.clone().or_else(|| self.focus.first().map(|f| f.0.clone()));
                (true, id.map(|i| Self::activate(&i)))
            }
            "Backspace" => {
                self.filter.pop();
                self.selected = None;
                (true, None)
            }
            "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" => {
                self.step(name);
                (true, None)
            }
            "Tab" => {
                let order = ["recent", "projects", "templates"];
                let i = order.iter().position(|s| *s == self.section()).unwrap_or(0);
                self.section = order[(i + 1) % order.len()].to_owned();
                self.selected = None;
                (true, None)
            }
            _ => {
                let typed: String = text.chars().filter(|c| !c.is_control()).collect();
                if typed.is_empty() { return (false, None); }
                if self.filter.is_empty() && typed.trim().is_empty() { return (true, None); }
                self.filter.push_str(&typed);
                self.selected = None;
                (true, None)
            }
        }
    }

    /// Move the focus to the nearest tile in that direction (by the layout
    /// of the last frame: rows of tiles, the Continue card above them).
    pub fn step(&mut self, dir: &str) {
        if self.focus.is_empty() { return; }
        let cur = self.selected.as_ref().and_then(|s| self.focus.iter().position(|f| &f.0 == s));
        let Some(i) = cur else {
            self.selected = Some(self.focus[0].0.clone());
            self.scroll_to = true;
            return;
        };
        let here = self.focus[i].1;
        let c = here.center();
        let mut best: Option<(f32, usize)> = None;
        for (j, (_, r)) in self.focus.iter().enumerate() {
            if j == i { continue; }
            let d = r.center() - c;
            let (ok, cost) = match dir {
                "ArrowRight" => (r.min.x >= here.max.x - 1.0 && d.y.abs() < here.height() / 2.0, d.x + d.y.abs() * 4.0),
                "ArrowLeft" => (r.max.x <= here.min.x + 1.0 && d.y.abs() < here.height() / 2.0, -d.x + d.y.abs() * 4.0),
                // Up and down keep the column, by the left edges (a wide
                // card above a row of tiles leads to the row's first).
                "ArrowDown" => (r.min.y >= here.max.y - 1.0, d.y + (r.min.x - here.min.x).abs() * 2.0),
                _ => (r.max.y <= here.min.y + 1.0, -d.y + (r.min.x - here.min.x).abs() * 2.0),
            };
            if ok && best.is_none_or(|b| cost < b.0) { best = Some((cost, j)); }
        }
        // Right/left run on along the reading order at a row's end.
        let j = best.map(|b| b.1).or(match dir {
            "ArrowRight" if i + 1 < self.focus.len() => Some(i + 1),
            "ArrowLeft" if i > 0 => Some(i - 1),
            _ => None,
        });
        if let Some(j) = j {
            self.selected = Some(self.focus[j].0.clone());
            self.scroll_to = true;
        }
    }
}

/// "a/b/c.built" → a short label for tooltips (the full path).
fn fit_text(ui: &egui::Ui, text: &str, font: egui::FontId, color: Color32, width: f32, rows: usize) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), egui::TextFormat { font_id: font, color, ..Default::default() });
    job.wrap.max_width = width.max(10.0);
    job.wrap.max_rows = rows;
    job.wrap.break_anywhere = rows == 1;
    ui.painter().layout_job(job)
}

/// The picture of a document, cropped to fill ``r`` (cover), or a quiet
/// box drawing when it has none (yet).
fn paint_thumb(ui: &egui::Ui, r: Rect, tex: Option<&egui::TextureHandle>, missing: bool, radius: u8) {
    let cr = CornerRadius::same(radius);
    ui.painter().rect_filled(r, cr, WELL);
    if let Some(tex) = tex {
        let [w, h] = tex.size();
        let (w, h) = (w as f32, h as f32);
        let (ra, ta) = (r.width() / r.height(), w / h.max(1.0));
        let uv = if ta > ra {
            let k = ra / ta; Rect::from_min_max(pos2((1.0 - k) / 2.0, 0.0), pos2((1.0 + k) / 2.0, 1.0))
        } else {
            let k = ta / ra; Rect::from_min_max(pos2(0.0, (1.0 - k) / 2.0), pos2(1.0, (1.0 + k) / 2.0))
        };
        egui::Image::new(tex).uv(uv).corner_radius(cr).paint_at(ui, r);
    } else {
        // A box in two-point view, drawn in the off ink.
        let c = r.center();
        let s = (r.height() * 0.18).min(22.0);
        let ink = Stroke::new(1.2_f32, if missing { Color32::from_rgb(0x4A, 0x50, 0x57) } else { OFF });
        let top = [c + vec2(0.0, -s * 1.1), c + vec2(s * 1.3, -s * 0.45), c + vec2(0.0, s * 0.2), c + vec2(-s * 1.3, -s * 0.45)];
        let down = vec2(0.0, s * 1.2);
        ui.painter().add(egui::Shape::closed_line(top.to_vec(), ink));
        for p in [top[1], top[2], top[3]] { ui.painter().line_segment([p, p + down], ink); }
        ui.painter().line_segment([top[3] + down, top[2] + down], ink);
        ui.painter().line_segment([top[2] + down, top[1] + down], ink);
        if missing {
            ui.painter().text(pos2(c.x, r.max.y - 14.0), egui::Align2::CENTER_CENTER, "Not found", meta(), DIM);
        }
    }
    ui.painter().rect_stroke(r, cr, Stroke::new(1.0_f32, EDGE), egui::StrokeKind::Inside);
}

/// The focus ring and hover wash of a card.
fn paint_card(ui: &egui::Ui, r: Rect, hovered: bool, focused: bool) {
    let cr = CornerRadius::same(10);
    ui.painter().rect_filled(r, cr, if hovered { CARD_HOVER } else { CARD });
    if focused {
        ui.painter().rect_filled(r, cr, ACTIVE);
        ui.painter().rect_stroke(r, cr, Stroke::new(1.5_f32, ACCENT_LINE), egui::StrokeKind::Inside);
    } else {
        ui.painter().rect_stroke(r, cr, Stroke::new(1.0_f32, EDGE), egui::StrokeKind::Inside);
    }
}

/// The pin at a card's corner: shown pinned (★) or on hover (☆). True: it was clicked.
fn pin_star(ui: &egui::Ui, at: Pos2, pinned: bool, card: &egui::Response) -> (bool, bool) {
    let r = Rect::from_center_size(at, vec2(22.0, 22.0));
    let over = card.hover_pos().is_some_and(|p| r.contains(p));
    if pinned || card.hovered() {
        ui.painter().circle_filled(r.center(), 11.0, if over { Color32::from_rgba_premultiplied(0, 0, 0, 170) }
                                                         else { Color32::from_rgba_premultiplied(0, 0, 0, 120) });
        let ink = if pinned { ACCENT } else if over { INK } else { DIM };
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, if pinned { "★" } else { "☆" }, body(), ink);
    }
    let hit = card.clicked() && card.interact_pointer_pos().is_some_and(|p| r.contains(p)) && (pinned || card.hovered());
    (hit, over)
}

/// The × at a tile's other corner, on hover: remove it from the recent
/// list (the file itself is left alone). True: it was clicked.
fn forget_x(ui: &egui::Ui, at: Pos2, card: &egui::Response) -> bool {
    let r = Rect::from_center_size(at, vec2(22.0, 22.0));
    if !card.hovered() { return false; }
    let over = card.hover_pos().is_some_and(|p| r.contains(p));
    ui.painter().circle_filled(r.center(), 11.0, if over { Color32::from_rgba_premultiplied(0, 0, 0, 170) }
                                                 else { Color32::from_rgba_premultiplied(0, 0, 0, 120) });
    let (c, k) = (r.center(), 3.5);
    let pen = Stroke::new(1.3_f32, if over { INK } else { DIM });
    ui.painter().line_segment([c + vec2(-k, -k), c + vec2(k, k)], pen);
    ui.painter().line_segment([c + vec2(-k, k), c + vec2(k, -k)], pen);
    card.clicked() && card.interact_pointer_pos().is_some_and(|p| r.contains(p))
}

/// A caption with a quiet action at the right end of its row ("Clear
/// missing"). True: the action was clicked.
fn caption_with_action(ui: &mut egui::Ui, text: &str, count: usize, action: &str) -> bool {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    let mut clicked = false;
    ui.scope_builder(egui::UiBuilder::new().max_rect(row), |ui| {
        ui.horizontal(|ui| {
            caption_plain(ui, text);
            ui.add_space(4.0);
            ui.label(egui::RichText::new(count.to_string()).font(meta()).color(OFF));
        });
    });
    if !action.is_empty() {
        let g = ui.painter().layout_no_wrap(action.to_owned(), meta(), FAINT);
        let r = Rect::from_min_size(pos2(row.max.x - g.size().x - 8.0, row.center().y - 9.0), vec2(g.size().x + 8.0, 18.0));
        let resp = ui.interact(r, egui::Id::new(("start_caption_action", text)), Sense::click());
        let ink = if resp.hovered() { INK } else { FAINT };
        ui.painter().text(pos2(r.max.x - 4.0, r.center().y), egui::Align2::RIGHT_CENTER, action, meta(), ink);
        clicked = resp.clicked();
    }
    ui.add_space(6.0);
    clicked
}

fn primary_button(ui: &mut egui::Ui, label: &str, key: &str) -> bool { sheet_button(ui, label, key, 1) }
fn plain_button(ui: &mut egui::Ui, label: &str, key: &str) -> bool { sheet_button(ui, label, key, 0) }

fn section_caption(ui: &mut egui::Ui, text: &str, count: Option<usize>) {
    ui.horizontal(|ui| {
        caption_plain(ui, text);
        if let Some(n) = count {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(n.to_string()).font(meta()).color(OFF));
        }
    });
    ui.add_space(6.0);
}

impl Ui {
    pub fn show_start(&mut self, shown: bool, can_return: bool) {
        if shown && !self.home.shown {
            self.home.filter.clear();
            self.home.selected = None;
            self.home.section.clear();
            self.home.menu = None;
            // One float at a time: the page closes the menus.
            self.view_open = false;
            self.sel_more = false;
            self.menu = None;
            if self.form.take().is_some() { self.actions.push("form:close".into()); }
            self.ctx.memory_mut(|m| m.stop_text_input());
        }
        self.home.shown = shown;
        self.home.can_return = can_return;
    }
    pub fn start_shown(&self) -> bool { self.home.shown }

    #[allow(clippy::too_many_arguments)]
    pub fn set_start(&mut self, files: Vec<(Vec<String>, Vec<i64>)>,
                     projects: Vec<(Vec<String>, Vec<i64>, Vec<(String, String, String)>)>,
                     templates: Vec<(String, String, String)>, cont: Vec<String>, footer: String,
                     nav: Vec<(String, String)>, links: Vec<(String, String)>) {
        let s = |v: &Vec<String>, i: usize| v.get(i).cloned().unwrap_or_default();
        let n = |v: &Vec<i64>, i: usize| v.get(i).copied().unwrap_or(0) != 0;
        self.home.files = files.iter().map(|(t, f)| Tile {
            path: s(t, 0), name: s(t, 1), meta: s(t, 2), project: s(t, 3), thumb: s(t, 4),
            pinned: n(f, 0), missing: n(f, 1),
        }).collect();
        self.home.projects = projects.into_iter().map(|(t, f, files)| Project {
            path: s(&t, 0), name: s(&t, 1), meta: s(&t, 2), pinned: n(&f, 0), missing: n(&f, 1), files,
        }).collect();
        self.home.templates = templates.into_iter().map(|(id, name, desc)| Template { id, name, desc }).collect();
        self.home.cont = (!cont.is_empty()).then(|| Continue {
            path: s(&cont, 0), name: s(&cont, 1), place: s(&cont, 2), last_in: s(&cont, 3),
            problems: s(&cont, 4), autosave: s(&cont, 5), thumb: s(&cont, 6), open: s(&cont, 7) == "1",
        });
        self.home.footer = footer;
        self.home.nav = nav;
        self.home.links = links;
    }

    /// A key for the page (Python sends it only while the page is up).
    pub fn start_key(&mut self, name: &str, text: &str) -> bool {
        if !self.home.shown || self.sheet.is_some() || self.cmd_open || self.ctx.wants_keyboard_input() {
            return false;
        }
        let (taken, action) = self.home.key(name, text);
        if let Some(a) = action { self.actions.push(format!("start:{a}")); }
        taken
    }

    pub(super) fn start_page(&mut self, ctx: &egui::Context, screen: Rect) {
        let mut out: Vec<String> = Vec::new();
        let mut focus: Vec<(String, Rect)> = Vec::new();
        let icons = &self.icons;
        let st = &mut self.home;
        egui::Area::new(egui::Id::new("start"))
            .fixed_pos(Pos2::ZERO)
            .order(egui::Order::Middle)
            .fade_in(false)
            .show(ctx, |ui| {
                let (all, _) = ui.allocate_exact_size(screen.size(), Sense::click());
                ui.painter().rect_filled(all, CornerRadius::ZERO, PAGE);
                let side = Rect::from_min_max(all.min, pos2(all.min.x + SIDE_W, all.max.y));
                ui.painter().rect_filled(side, CornerRadius::ZERO, SIDE);
                ui.painter().vline(side.max.x - 0.5, side.y_range(), Stroke::new(1.0_f32, EDGE));
                // ---- Side column ------------------------------------------------
                let inner = Rect::from_min_max(pos2(side.min.x + 10.0, TOP_H + 6.0), pos2(side.max.x - 10.0, side.max.y - 10.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                    ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new("Built").font(strong(15.0)).color(INK));
                    });
                    ui.add_space(12.0);
                    let counts = [("recent", st.files.len()), ("projects", st.projects.len()), ("templates", st.templates.len())];
                    let current = st.section().to_owned();
                    for (id, label) in st.nav.clone() {
                        let n = counts.iter().find(|c| c.0 == id).map(|c| c.1);
                        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                        let on = id == current;
                        paint_state(ui, r, state_of(&resp, on, true), R_ROW);
                        ui.painter().text(pos2(r.min.x + 10.0, r.center().y), egui::Align2::LEFT_CENTER, &label,
                                          if on { strong(12.0) } else { body() }, if on { Color32::WHITE } else { INK });
                        if let Some(n) = n.filter(|n| *n > 0) {
                            ui.painter().text(pos2(r.max.x - 10.0, r.center().y), egui::Align2::RIGHT_CENTER, n.to_string(), meta(), FAINT);
                        }
                        if resp.clicked() {
                            if counts.iter().any(|c| c.0 == id) {
                                st.section = id.clone();
                                st.selected = None;
                            } else {
                                out.push(format!("nav{SEP}{id}"));
                            }
                        }
                    }
                    // Projects, each with its files.
                    if !st.projects.is_empty() {
                        ui.add_space(16.0);
                        caption_at(ui, "Projects", 10.0);
                        ui.add_space(4.0);
                        let room = ui.available_height() - 28.0 * st.links.len() as f32 - 16.0;
                        egui::ScrollArea::vertical().id_salt("start_projects").max_height(room.max(60.0)).auto_shrink([false, true])
                            .show(ui, |ui| {
                            ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                            for p in st.visible_projects().into_iter().cloned().collect::<Vec<_>>() {
                                let open = st.expanded.contains(&p.path) || !st.filter.is_empty();
                                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
                                paint_state(ui, r, state_of(&resp, false, true), R_ROW);
                                let chev = Rect::from_min_size(r.min, vec2(22.0, r.height()));
                                if !p.files.is_empty() {
                                    ui.painter().text(chev.center(), egui::Align2::CENTER_CENTER, if open { "▾" } else { "▸" }, meta(), FAINT);
                                }
                                let ink = if p.missing { OFF } else { INK };
                                let g = fit_text(ui, &p.name, body(), ink, r.width() - 22.0 - 30.0, 1);
                                ui.painter().galley(pos2(r.min.x + 22.0, r.center().y - g.size().y / 2.0), g, ink);
                                let tail = if p.pinned { "★".to_owned() } else if p.files.is_empty() { String::new() } else { p.files.len().to_string() };
                                ui.painter().text(pos2(r.max.x - 8.0, r.center().y), egui::Align2::RIGHT_CENTER, tail, meta(),
                                                  if p.pinned { ACCENT } else { FAINT });
                                let resp = resp.on_hover_text(format!("{}\n{}", p.path, p.meta));
                                if resp.clicked() {
                                    let on_chev = resp.interact_pointer_pos().is_some_and(|q| chev.contains(q));
                                    if on_chev && !p.files.is_empty() {
                                        if !st.expanded.remove(&p.path) { st.expanded.insert(p.path.clone()); }
                                    } else {
                                        out.push(format!("open{SEP}{}", p.path));
                                    }
                                }
                                if resp.secondary_clicked() { out.push(format!("pin{SEP}{}", p.path)); }
                                if open {
                                    for (path, name, role) in &p.files {
                                        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                                        paint_state(ui, r, state_of(&resp, false, true), R_ROW);
                                        let g = fit_text(ui, name, meta(), DIM, r.width() - 34.0 - 50.0, 1);
                                        ui.painter().galley(pos2(r.min.x + 34.0, r.center().y - g.size().y / 2.0), g, DIM);
                                        ui.painter().text(pos2(r.max.x - 8.0, r.center().y), egui::Align2::RIGHT_CENTER, role, meta(), OFF);
                                        if resp.on_hover_text(path).clicked() { out.push(format!("open{SEP}{path}")); }
                                    }
                                }
                            }
                        });
                    }
                    // Preferences · Extensions · Help, at the bottom.
                    let h = 28.0 * st.links.len() as f32;
                    let bottom = Rect::from_min_max(pos2(inner.min.x, inner.max.y - h), inner.max);
                    ui.scope_builder(egui::UiBuilder::new().max_rect(bottom), |ui| {
                        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                        for (id, label) in &st.links {
                            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                            paint_state(ui, r, state_of(&resp, false, true), R_ROW);
                            ui.painter().text(pos2(r.min.x + 10.0, r.center().y), egui::Align2::LEFT_CENTER, label, body(), DIM);
                            if resp.clicked() { out.push(format!("cmd{SEP}{id}")); }
                        }
                    });
                });
                // ---- Main column ------------------------------------------------
                let main = Rect::from_min_max(pos2(side.max.x + PAD, TOP_H + 6.0), pos2(all.max.x - PAD, all.max.y - 30.0));
                let width = main.width().min(TILE_W * 6.0 + TILE_GAP * 5.0);
                let main = Rect::from_min_size(main.min, vec2(width, main.height()));
                ui.scope_builder(egui::UiBuilder::new().max_rect(main), |ui| {
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    // Top row: New model, New project…, Open… | the filter.
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
                        if primary_button(ui, "New model", "⌘N") { out.push("new".into()); }
                        if plain_button(ui, "New project…", "") { out.push("project".into()); }
                        if plain_button(ui, "Open…", "⌘O") { out.push("openfile".into()); }
                        let fw = 260.0_f32.min(ui.available_width());
                        ui.add_space((ui.available_width() - fw).max(0.0));
                        let (r, resp) = ui.allocate_exact_size(vec2(fw, 28.0), Sense::click());
                        ui.painter().rect_filled(r, CornerRadius::same(R_CONTROL), FIELD);
                        let typing = !st.filter.is_empty();
                        ui.painter().rect_stroke(r, CornerRadius::same(R_CONTROL),
                                                 Stroke::new(1.0_f32, if typing { ACCENT_LINE } else { EDGE }), egui::StrokeKind::Inside);
                        ui.painter().text(pos2(r.min.x + 14.0, r.center().y), egui::Align2::CENTER_CENTER, "⌕", body(), FAINT);
                        if typing {
                            let g = fit_text(ui, &format!("{}▏", st.filter), body(), INK, r.width() - 60.0, 1);
                            ui.painter().galley(pos2(r.min.x + 26.0, r.center().y - g.size().y / 2.0), g, INK);
                            ui.painter().text(pos2(r.max.x - 10.0, r.center().y), egui::Align2::RIGHT_CENTER, "esc", meta(), FAINT);
                        } else {
                            ui.painter().text(pos2(r.min.x + 26.0, r.center().y), egui::Align2::LEFT_CENTER, "Type to filter", body(), FAINT);
                        }
                        if resp.clicked() && typing {
                            let x = Rect::from_center_size(pos2(r.max.x - 18.0, r.center().y), vec2(28.0, 24.0));
                            if resp.interact_pointer_pos().is_some_and(|p| x.contains(p)) { st.filter.clear(); st.selected = None; }
                        }
                    });
                    ui.add_space(18.0);
                    let scroll_to = std::mem::take(&mut st.scroll_to);
                    egui::ScrollArea::vertical().id_salt("start_main").auto_shrink([false, false]).show(ui, |ui| {
                        ui.set_width(width);
                        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                        let section = st.section().to_owned();
                        let selected = st.selected.clone();
                        let register = |ui: &egui::Ui, id: String, r: Rect, focus: &mut Vec<(String, Rect)>| {
                            if scroll_to && selected.as_deref() == Some(id.as_str()) {
                                ui.scroll_to_rect(r.expand(12.0), None);
                            }
                            focus.push((id, r));
                        };
                        let mut any = false;
                        // Continue: the last document, as big as it deserves.
                        if section == "recent" && st.filter.is_empty() {
                            if let Some(c) = st.cont.clone() {
                                any = true;
                                section_caption(ui, if c.open { "Open now" } else { "Continue" }, None);
                                let h = 132.0;
                                let (r, resp) = ui.allocate_exact_size(vec2(width, h), Sense::click());
                                let focused = selected.as_deref() == Some("cont");
                                paint_card(ui, r, resp.hovered(), focused);
                                let thumb = Rect::from_min_size(r.min + vec2(6.0, 6.0), vec2(208.0, h - 12.0));
                                paint_thumb(ui, thumb, icons.get(&c.thumb), false, 7);
                                let x = thumb.max.x + 16.0;
                                let tw = r.max.x - x - 16.0;
                                let mut y = r.min.y + 16.0;
                                let g = fit_text(ui, &c.name, strong(15.0), INK, tw, 1);
                                let gh = g.size().y;
                                ui.painter().galley(pos2(x, y), g, INK);
                                y += gh + 4.0;
                                for (text, ink) in [(&c.place, DIM), (&c.last_in, DIM)] {
                                    if text.is_empty() { continue; }
                                    let g = fit_text(ui, text, body(), ink, tw, 1);
                                    let gh = g.size().y;
                                    ui.painter().galley(pos2(x, y), g, ink);
                                    y += gh + 3.0;
                                }
                                if !c.problems.is_empty() {
                                    y += 2.0;
                                    ui.painter().circle_filled(pos2(x + 4.0, y + 7.0), 3.5, WARN);
                                    ui.painter().text(pos2(x + 14.0, y + 7.0), egui::Align2::LEFT_CENTER, &c.problems, body(), WARN);
                                }
                                let mut compare = false;
                                if !c.autosave.is_empty() {
                                    let at = Rect::from_min_max(pos2(x, r.max.y - 38.0), pos2(r.max.x - 120.0, r.max.y - 10.0));
                                    ui.scope_builder(egui::UiBuilder::new().max_rect(at), |ui| {
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
                                            ui.label(egui::RichText::new(&c.autosave).font(body()).color(WARN));
                                            if amber_button(ui, "Open and choose…") { compare = true; }
                                        });
                                    });
                                }
                                let hint = if c.open { "Return  ↩" } else { "Continue  ↩" };
                                ui.painter().text(pos2(r.max.x - 16.0, r.max.y - 22.0), egui::Align2::RIGHT_CENTER, hint, meta(),
                                                  if focused { INK } else { FAINT });
                                if compare {
                                    out.push(format!("compare{SEP}{}", c.path));
                                } else if resp.on_hover_text(&c.path).clicked() {
                                    out.push("continue".into());
                                }
                                register(ui, "cont".into(), r, &mut focus);
                                ui.add_space(24.0);
                            }
                        }
                        // Files: a grid of recent documents.
                        if section == "recent" {
                            let files: Vec<Tile> = st.visible_files().into_iter().cloned().collect();
                            if !files.is_empty() {
                                any = true;
                                let missing = files.iter().filter(|t| t.missing).count();
                                let clear = if missing > 0 { format!("Clear missing ({missing})") } else { String::new() };
                                if caption_with_action(ui, "Files", files.len(), &clear) {
                                    out.push("clearmissing".into());
                                }
                                let cols = (((width + TILE_GAP) / (TILE_W + TILE_GAP)).floor() as usize).max(1);
                                for row in files.chunks(cols) {
                                    let (band, _) = ui.allocate_exact_size(vec2(width, THUMB_H + 64.0), Sense::hover());
                                    for (k, t) in row.iter().enumerate() {
                                        let r = Rect::from_min_size(band.min + vec2(k as f32 * (TILE_W + TILE_GAP), 0.0), vec2(TILE_W, band.height()));
                                        let id = format!("file{SEP}{}", t.path);
                                        let resp = ui.interact(r, egui::Id::new(("start_tile", &t.path)), Sense::click());
                                        let focused = selected.as_deref() == Some(id.as_str());
                                        paint_card(ui, r, resp.hovered(), focused);
                                        let thumb = Rect::from_min_size(r.min + vec2(4.0, 4.0), vec2(TILE_W - 8.0, THUMB_H - 8.0));
                                        paint_thumb(ui, thumb, icons.get(&t.thumb), t.missing, 7);
                                        let (pin, _) = pin_star(ui, pos2(thumb.max.x - 15.0, thumb.min.y + 15.0), t.pinned, &resp);
                                        let forget = forget_x(ui, pos2(thumb.min.x + 15.0, thumb.min.y + 15.0), &resp);
                                        let ink = if t.missing { OFF } else { INK };
                                        let g = fit_text(ui, &t.name, strong(12.0), ink, TILE_W - 20.0, 1);
                                        let gh = g.size().y;
                                        ui.painter().galley(pos2(r.min.x + 10.0, thumb.max.y + 8.0), g, ink);
                                        let m = fit_text(ui, &t.meta, meta(), FAINT, TILE_W - 20.0, 2);
                                        ui.painter().galley(pos2(r.min.x + 10.0, thumb.max.y + 10.0 + gh), m, FAINT);
                                        let resp = resp.on_hover_text(&t.path);
                                        if resp.secondary_clicked() {
                                            let at = resp.interact_pointer_pos().unwrap_or(r.center());
                                            st.menu = Some((t.path.clone(), t.pinned, at));
                                        } else if pin {
                                            out.push(format!("pin{SEP}{}", t.path));
                                        } else if forget {
                                            out.push(format!("forget{SEP}{}", t.path));
                                        } else if resp.clicked() {
                                            out.push(format!("open{SEP}{}", t.path));
                                        }
                                        register(ui, id, r, &mut focus);
                                    }
                                    ui.add_space(TILE_GAP);
                                }
                                ui.add_space(12.0);
                            }
                        }
                        // Projects, as cards (their own section).
                        if section == "projects" {
                            let projects: Vec<Project> = st.visible_projects().into_iter().cloned().collect();
                            if !projects.is_empty() {
                                any = true;
                                section_caption(ui, "Projects", Some(projects.len()));
                                let cols = (((width + TILE_GAP) / (TILE_W * 1.5 + TILE_GAP)).floor() as usize).max(1);
                                let w = (width - TILE_GAP * (cols as f32 - 1.0)) / cols as f32;
                                for row in projects.chunks(cols) {
                                    let (band, _) = ui.allocate_exact_size(vec2(width, 76.0), Sense::hover());
                                    for (k, p) in row.iter().enumerate() {
                                        let r = Rect::from_min_size(band.min + vec2(k as f32 * (w + TILE_GAP), 0.0), vec2(w, band.height()));
                                        let id = format!("proj{SEP}{}", p.path);
                                        let resp = ui.interact(r, egui::Id::new(("start_proj", &p.path)), Sense::click());
                                        let focused = selected.as_deref() == Some(id.as_str());
                                        paint_card(ui, r, resp.hovered(), focused);
                                        let (pin, _) = pin_star(ui, pos2(r.max.x - 18.0, r.min.y + 18.0), p.pinned, &resp);
                                        let ink = if p.missing { OFF } else { INK };
                                        let g = fit_text(ui, &p.name, strong(13.0), ink, w - 56.0, 1);
                                        ui.painter().galley(pos2(r.min.x + 14.0, r.min.y + 12.0), g, ink);
                                        let files: Vec<&str> = p.files.iter().map(|f| f.1.as_str()).collect();
                                        let line = if files.is_empty() { p.meta.clone() } else { format!("{} · {}", p.meta, files.join(", ")) };
                                        let m = fit_text(ui, line.trim_start_matches(" · "), meta(), FAINT, w - 28.0, 2);
                                        ui.painter().galley(pos2(r.min.x + 14.0, r.min.y + 34.0), m, FAINT);
                                        let resp = resp.on_hover_text(&p.path);
                                        if resp.secondary_clicked() {
                                            let at = resp.interact_pointer_pos().unwrap_or(r.center());
                                            st.menu = Some((p.path.clone(), p.pinned, at));
                                        } else if pin {
                                            out.push(format!("pin{SEP}{}", p.path));
                                        } else if resp.clicked() {
                                            out.push(format!("open{SEP}{}", p.path));
                                        }
                                        register(ui, id, r, &mut focus);
                                    }
                                    ui.add_space(TILE_GAP);
                                }
                                ui.add_space(12.0);
                            }
                        }
                        // Start from: templates.
                        if section == "recent" || section == "templates" {
                            let templates: Vec<Template> = st.visible_templates().into_iter().cloned().collect();
                            if !templates.is_empty() {
                                any = true;
                                section_caption(ui, "Start from", None);
                                let cols = (((width + TILE_GAP) / (TILE_W + TILE_GAP)).floor() as usize).max(1);
                                for row in templates.chunks(cols) {
                                    let (band, _) = ui.allocate_exact_size(vec2(width, 64.0), Sense::hover());
                                    for (k, t) in row.iter().enumerate() {
                                        let r = Rect::from_min_size(band.min + vec2(k as f32 * (TILE_W + TILE_GAP), 0.0), vec2(TILE_W, band.height()));
                                        let id = format!("tpl{SEP}{}", t.id);
                                        let resp = ui.interact(r, egui::Id::new(("start_tpl", &t.id)), Sense::click());
                                        let focused = selected.as_deref() == Some(id.as_str());
                                        paint_card(ui, r, resp.hovered(), focused);
                                        let well = Rect::from_min_size(r.min + vec2(8.0, 8.0), vec2(48.0, 48.0));
                                        ui.painter().rect_filled(well, CornerRadius::same(7), WELL);
                                        let glyph = t.name.chars().next().map(|c| c.to_string()).unwrap_or_default();
                                        ui.painter().text(well.center(), egui::Align2::CENTER_CENTER, glyph, strong(16.0), INK_2);
                                        let x = well.max.x + 10.0;
                                        let g = fit_text(ui, &t.name, strong(12.0), INK, r.max.x - x - 8.0, 1);
                                        let gh = g.size().y;
                                        ui.painter().galley(pos2(x, r.min.y + 12.0), g, INK);
                                        let m = fit_text(ui, &t.desc, meta(), FAINT, r.max.x - x - 8.0, 2);
                                        ui.painter().galley(pos2(x, r.min.y + 14.0 + gh), m, FAINT);
                                        if resp.clicked() { out.push(format!("template{SEP}{}", t.id)); }
                                        register(ui, id, r, &mut focus);
                                    }
                                    ui.add_space(TILE_GAP);
                                }
                            }
                        }
                        if !any {
                            ui.add_space(40.0);
                            let text = if st.filter.is_empty() { "Nothing here yet.".to_owned() }
                                       else { format!("Nothing matches “{}”.", st.filter) };
                            ui.label(egui::RichText::new(text).font(body()).color(FAINT));
                        }
                        ui.add_space(24.0);
                    });
                });
                // The engine line, quiet, at the foot of the main column.
                if !st.footer.is_empty() {
                    ui.painter().text(pos2(side.max.x + PAD, all.max.y - 15.0), egui::Align2::LEFT_CENTER, &st.footer, meta(), OFF);
                }
                let hint = if st.can_return { "↑↓←→ choose · ↩ open · esc back" } else { "↑↓←→ choose · ↩ open · type to filter" };
                ui.painter().text(pos2(all.max.x - PAD, all.max.y - 15.0), egui::Align2::RIGHT_CENTER, hint, meta(), OFF);
            });
        // A tile's right-click menu, over the page.
        if let Some((path, pinned, at)) = st.menu.clone() {
            let mut close = false;
            let area = egui::Area::new(egui::Id::new("start_menu"))
                .fixed_pos(at)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    glass(Surface::Raised).inner_margin(Margin::same(6)).show(ui, |ui| {
                        ui.set_width(176.0);
                        ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                        let label = if pinned { "Unpin" } else { "Pin to top" };
                        if menu_row(ui, &("pin".into(), label.into(), String::new(), String::new(), -1i8, true)) {
                            out.push(format!("pin{SEP}{path}"));
                            close = true;
                        }
                        if menu_row(ui, &("forget".into(), "Remove from recents".into(), String::new(), String::new(), -1i8, true)) {
                            out.push(format!("forget{SEP}{path}"));
                            close = true;
                        }
                    });
                });
            // A press anywhere else closes it (the press that chose a row
            // has already been read above).
            let outside = ctx.input(|i| i.pointer.any_pressed()
                && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p)));
            if close || outside { st.menu = None; }
        }
        // The focus follows the layout: keep it on a tile that is still there.
        if st.selected.as_ref().is_some_and(|s| !focus.iter().any(|f| &f.0 == s)) {
            st.selected = None;
        }
        if st.selected.is_none() {
            st.selected = focus.first().map(|f| f.0.clone());
        }
        st.focus = focus;
        for a in out {
            if let Some(p) = a.strip_prefix(&format!("pin{SEP}")) {
                // Pinning moves the tile: keep the focus on it.
                st.selected = Some(format!("file{SEP}{p}"));
            }
            self.actions.push(format!("start:{a}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> Start {
        let mut s = Start { shown: true, ..Default::default() };
        s.files = ["alpha", "beta", "gamma"].iter().map(|n| Tile { path: format!("/x/{n}.built"), name: format!("{n}.built"), ..Default::default() }).collect();
        // Continue above, three tiles in a row below.
        s.focus = vec![("cont".into(), Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 130.0)))];
        for (i, n) in ["a", "b", "c"].iter().enumerate() {
            s.focus.push((format!("file{SEP}/x/{n}.built"), Rect::from_min_size(pos2(i as f32 * 200.0, 160.0), vec2(184.0, 180.0))));
        }
        s
    }

    #[test]
    fn arrows_walk_the_grid_and_enter_opens() {
        let mut s = page();
        s.step("ArrowDown");                      // nothing focused: the first
        assert_eq!(s.selected.as_deref(), Some("cont"));
        s.step("ArrowDown");
        assert_eq!(s.selected.as_deref(), Some("file\u{1f}/x/a.built"));
        s.step("ArrowRight");
        s.step("ArrowRight");
        assert_eq!(s.selected.as_deref(), Some("file\u{1f}/x/c.built"));
        s.step("ArrowUp");
        assert_eq!(s.selected.as_deref(), Some("cont"));
        s.selected = Some("file\u{1f}/x/b.built".into());
        assert_eq!(s.key("Enter", ""), (true, Some("open\u{1f}/x/b.built".into())));
        s.selected = Some("cont".into());
        assert_eq!(s.key("Enter", ""), (true, Some("continue".into())));
    }

    #[test]
    fn typing_filters_and_escape_clears_then_returns() {
        let mut s = page();
        assert_eq!(s.key("", "gam"), (true, None));
        assert_eq!(s.visible_files().len(), 1);
        assert_eq!(s.key("Escape", ""), (true, None));
        assert!(s.filter.is_empty());
        assert_eq!(s.key("Escape", ""), (true, None));       // nothing behind: stays
        s.can_return = true;
        assert_eq!(s.key("Escape", ""), (true, Some("close".into())));
        assert_eq!(s.key("", ""), (false, None));
    }

    #[test]
    fn escape_closes_a_tile_menu_before_anything_else() {
        let mut s = page();
        s.can_return = true;
        s.filter = "a".into();
        s.menu = Some(("/x/a.built".into(), false, pos2(10.0, 10.0)));
        assert_eq!(s.key("Escape", ""), (true, None));
        assert!(s.menu.is_none());
        assert_eq!(s.filter, "a");
        assert_eq!(s.key("Escape", ""), (true, None));
        assert_eq!(s.key("Escape", ""), (true, Some("close".into())));
    }
}
