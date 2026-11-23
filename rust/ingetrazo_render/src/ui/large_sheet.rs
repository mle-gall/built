//! The large sheet: one surface for every "browse, pick, set" place of the
//! app (docs/ui-patterns.md): the Library, the start page, Settings, and
//! later the one-off Export, the Extension Manager, About.
//!
//! A sheet is centred below the project tabs over a full-window blur.
//! The model rail, dock and bottom bar retract while it is open. Its parts never change from one sheet to the next:
//!
//! ```text
//! ┌ header: title · tabs · verbs ……………… menus · [search] · × ┐
//! │ side list │ content (blocks, scrolls)     │ detail (blocks)  │
//! └ footer: links ……………………………………………………………… key hints ┘
//! ```
//!
//! Python (views/chrome_sheet.py) owns everything shown: it sends the whole
//! sheet as JSON (`set_large_sheet`) when anything changed, and answers the
//! actions, `lsheet:<sheet id>\u{1f}<verb>[\u{1f}<arg>…]`:
//!
//! | verb | args | when |
//! |---|---|---|
//! | `close` | | Esc (nothing left to dismiss), × |
//! | `search` | text | the search text changed (typing anywhere types into it) |
//! | `side` | id | a side row clicked, or Tab |
//! | `fold` | id | a side row's chevron |
//! | `tab` | id | a header tab |
//! | `verb` | id | a header verb, a footer link, a buttons block |
//! | `focus` | item | an item clicked or reached with the arrows |
//! | `open` | item | an item double-clicked, or Enter on it |
//! | `menu` | item, entry | an entry of an item's right-click menu |
//! | `pick` | item | a row of the detail pane clicked |
//! | `headmenu` | menu, option | an option of a header menu (sort…) |
//! | `v` / `b` | field, value | a fields block (the forms' widgets, ui/dialog.rs) |
//!
//! Input: while it is up the sheet owns the pointer under the top bar and
//! every key (Python keeps ⌘/⌃ shortcuts for the window). Esc closes the
//! innermost thing: a menu, a field being typed in, the search text, then
//! the sheet (when `closable`).

use std::collections::HashMap;

use serde::Deserialize;

use super::*;

const SEP: char = '\u{1f}';
/// How long opening and closing take (seconds).
pub const ANIM_S: f32 = 0.16;

// ---- Look: the design system's names only (ui/tokens.rs, ui/kit.rs),
// in one block. Blue is only the accent: the focused tile, the selected
// row, the primary button, links. No colour is made here.
/// The side list and the panes are set apart by dividers only.
const RULE: Color32 = DIVIDER;
/// A picture's well (no picture yet).
const PICTURE_WELL: Color32 = WELL;
// Secondary text on glass needs the brighter graphite text token.
use super::tokens::DIM as FAINT;

// ---- What Python sends ------------------------------------------------------

#[derive(Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Spec {
    pub id: String,
    pub title: String,
    pub tabs: Vec<Tab>,
    pub default_verb: String,
    pub backdrop_icon: String,
    pub verbs: Vec<Verb>,
    pub search: Option<Search>,
    pub menus: Vec<HeadMenu>,
    #[serde(default = "yes")]
    pub closable: bool,
    pub side: Vec<Item>,
    pub side_width: f32,
    pub content: Pane,
    pub detail: Option<Pane>,
    pub footer: Option<Footer>,
    /// The focused item: taken when the sheet opens, or when `force_focus`.
    pub focus: Option<String>,
    pub force_focus: bool,
    /// The widest the sheet grows (points); 0 = the default.
    pub width: f32,
}
fn yes() -> bool { true }

#[derive(Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Tab { pub id: String, pub label: String, pub count: String, pub on: bool,
                 #[serde(default = "yes")] pub enabled: bool }

/// A button: header verb, footer link, a buttons block's entry. role: 0
/// plain, 1 default (accent), 3 quiet text.
#[derive(Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Verb { pub id: String, pub label: String, pub key: String, pub role: u8,
                  #[serde(default = "yes")] pub enabled: bool, pub tip: String }

#[derive(Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Search { pub placeholder: String, pub text: String, pub force: bool, pub key: String }

#[derive(Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct HeadMenu { pub id: String, pub label: String, pub options: Vec<Verb>, pub on: String,
                      /// Among the verbs (From template ▾) rather than beside the search (Sort ▾).
                      pub left: bool }

#[derive(Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Footer { pub links: Vec<Verb>, pub hint: String }

#[derive(Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Pane {
    pub title: String,
    pub sub: String,
    pub blocks: Vec<Block>,
    pub width: f32,
    /// Shown when there are no blocks.
    pub empty: String,
}

/// One block of a pane. kind: caption, text, tiles, rows, fields, picture,
/// heading, kv, buttons, separator.
#[derive(Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Block {
    pub kind: String,
    pub id: String,
    pub text: String,
    pub sub: String,
    pub right: String,
    /// "", dim, faint, ok, warn, error, accent.
    pub tone: String,
    pub items: Vec<Item>,
    /// kv rows: (label, value, tone).
    pub rows: Vec<(String, String, String)>,
    pub cells: Vec<Vec<String>>,
    pub fields: Vec<dialog::Field>,
    pub spans: Vec<kit::ProseSpan>,
    pub credits: Vec<kit::CreditRow>,
    pub map: Option<super::site_map::MapSpec>,
    pub buttons: Vec<Verb>,
    /// tiles: the picture's side; picture: its height.
    pub size: f32,
    pub icon: String,
    pub hex: String,
    /// buttons: "right" (default), "left" or "fill" (one per line, full width).
    pub align: String,
}

/// A tile, a row, a side entry.
#[derive(Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Item {
    pub id: String,
    pub label: String,
    pub sub: String,
    pub right: String,
    /// A texture uploaded with set_icons, drawn as the picture.
    pub icon: String,
    /// A colour when there is no picture.
    pub hex: String,
    /// A count or word on the tile's corner.
    pub badge: String,
    /// A status dot (ok, warn, error, accent) and its words.
    pub dot: String,
    pub dot_text: String,
    pub depth: u8,
    /// Selected (side rows), current (tiles: the material painting).
    pub on: bool,
    pub dim: bool,
    /// Side rows: "caption" (a heading), "file" (a document glyph), "" plain.
    pub kind: String,
    /// Side rows: -1 no chevron, 0 folded, 1 open.
    #[serde(default = "minus_one")]
    pub fold: i8,
    /// Not focusable (a note, a heading row in a list).
    pub fixed: bool,
    pub tip: String,
    /// Right-click menu: (entry id, label); "-" draws a rule.
    pub menu: Vec<(String, String)>,
}
fn minus_one() -> i8 { -1 }

// ---- What the sheet keeps ---------------------------------------------------

#[derive(Default)]
pub struct LargeSheet {
    pub spec: Option<Spec>,
    /// Shown (false while it animates out).
    pub open: bool,
    pub search: String,
    visible_previews: Vec<String>,
    map: super::site_map::MapState,
    search_reported: String,
    pub focused: Option<String>,
    /// Focusable items as last laid out (content pane, reading order).
    pub focus_rects: Vec<(String, Rect)>,
    scroll_to: bool,
    /// An item's menu: (item, entries, where).
    pub menu: Option<(String, Vec<(String, String)>, Pos2)>,
    /// A header menu: (menu id, under this rect).
    head_menu: Option<(String, Rect)>,
    /// One form per fields block (keyed pane:block), Python's values.
    forms: HashMap<String, dialog::Dialog>,
    focus_search: bool,
    /// Bring the sheet over the rest of the chrome at the next frame.
    raise: bool,
    /// The sheet's rect as last drawn (tests, captures).
    pub rect: Option<Rect>,
}

fn search_id() -> egui::Id { egui::Id::new("lsheet_search") }

impl LargeSheet {
    pub(super) fn choice_open(&self) -> bool {
        self.open && self.forms.values().any(|form| form.choice_open.is_some())
    }

    /// A new spec from Python; `None` closes. Returns whether it opened now.
    pub fn set(&mut self, spec: Option<Spec>) -> bool {
        let Some(mut spec) = spec else {
            self.open = false;
            self.menu = None;
            self.head_menu = None;
            return false;
        };
        if !spec.content.blocks.iter().any(|block| block.map.is_some()) {
            self.map = Default::default();
        }
        if !spec.content.blocks.iter().any(|b| matches!(b.tone.as_str(), "catalog" | "components")) {
            self.visible_previews.clear();
        }
        if spec.id == "start" {
            let template = spec.focus.as_deref().is_some_and(|id| id.starts_with("tpl:"));
            if let Some(new) = spec.verbs.iter_mut().find(|verb| verb.id == "new") {
                new.role = u8::from(!template);
            }
            for button in spec.detail.iter_mut().flat_map(|pane| &mut pane.blocks)
                .flat_map(|block| &mut block.buttons)
            {
                if button.role == 1 {
                    button.role = u8::from(template && button.id == "open");
                }
            }
        }
        let opening = !self.open || self.spec.as_ref().is_none_or(|s| s.id != spec.id);
        if opening {
            self.visible_previews.clear();
            self.map = Default::default();
            self.search.clear();
            self.focused = None;
            self.menu = None;
            self.head_menu = None;
            self.forms.clear();
            self.raise = true;
            self.focus_search = false;
        }
        if let Some(s) = &spec.search {
            if opening || s.force {
                self.search = s.text.clone();
            }
            if opening || s.force { self.search_reported = self.search.clone(); }
        }
        if opening || spec.force_focus {
            if let Some(f) = &spec.focus {
                self.focused = Some(f.clone()).filter(|f| !f.is_empty());
                self.scroll_to = true;
            }
        }
        // The fields blocks' forms: values from Python, typing kept.
        let mut keep = std::collections::HashSet::new();
        let panes = [("c", Some(&spec.content)), ("d", spec.detail.as_ref())];
        for (pane, p) in panes {
            let Some(p) = p else { continue };
            for (i, b) in p.blocks.iter().enumerate().filter(|(_, b)| b.kind == "fields") {
                let key = form_key(pane, i, b);
                let d = self.forms.entry(key.clone()).or_insert_with(|| {
                    let mut d = dialog::Dialog::new(format!("ls.{}.{key}", spec.id));
                    d.prefix = format!("lsheet:{}", spec.id);
                    d
                });
                d.update(String::new(), String::new(), 900.0, b.fields.clone(), Vec::new());
                keep.insert(key);
            }
        }
        self.forms.retain(|k, _| keep.contains(k));
        self.spec = Some(spec);
        self.open = true;
        opening
    }

    fn id(&self) -> String { self.spec.as_ref().map(|s| s.id.clone()).unwrap_or_default() }

    fn act(&self, verb: &str, args: &[&str]) -> String {
        let mut s = format!("lsheet:{}{SEP}{verb}", self.id());
        for a in args {
            s.push(SEP);
            s.push_str(a);
        }
        s
    }

    fn closable(&self) -> bool { self.spec.as_ref().is_some_and(|s| s.closable) }

    /// Escape: the innermost thing goes. Returns an action (close).
    pub fn escape(&mut self, in_field: bool, in_search: bool) -> (bool, Option<String>) {
        if self.menu.take().is_some() || self.head_menu.take().is_some() {
            return (true, None);
        }
        if in_field {
            return (true, Some(String::new()));     // the caller ends typing
        }
        if !self.search.is_empty() {
            self.search.clear();
            return (true, None);
        }
        if in_search {
            return (true, Some(String::new()));
        }
        (true, self.closable().then(|| self.act("close", &[])))
    }

    /// Move the focus to the nearest item that way (by last frame's layout).
    pub fn step(&mut self, dir: &str) -> bool {
        if self.focus_rects.is_empty() { return false; }
        let cur = self.focused.as_ref().and_then(|s| self.focus_rects.iter().position(|f| &f.0 == s));
        let Some(i) = cur else {
            self.focused = Some(self.focus_rects[0].0.clone());
            self.scroll_to = true;
            return true;
        };
        let here = self.focus_rects[i].1;
        let c = here.center();
        let mut best: Option<(f32, usize)> = None;
        for (j, (_, r)) in self.focus_rects.iter().enumerate() {
            if j == i { continue; }
            let d = r.center() - c;
            let (ok, cost) = match dir {
                "ArrowRight" => (r.min.x >= here.max.x - 1.0 && d.y.abs() < here.height() / 2.0, d.x + d.y.abs() * 4.0),
                "ArrowLeft" => (r.max.x <= here.min.x + 1.0 && d.y.abs() < here.height() / 2.0, -d.x + d.y.abs() * 4.0),
                "ArrowDown" => (r.min.y >= here.max.y - 1.0, d.y + (r.min.x - here.min.x).abs() * 2.0),
                _ => (r.max.y <= here.min.y + 1.0, -d.y + (r.min.x - here.min.x).abs() * 2.0),
            };
            if ok && best.is_none_or(|b| cost < b.0) { best = Some((cost, j)); }
        }
        let j = best.map(|b| b.1).or(match dir {
            "ArrowRight" if i + 1 < self.focus_rects.len() => Some(i + 1),
            "ArrowLeft" if i > 0 => Some(i - 1),
            _ => None,
        });
        match j {
            Some(j) => {
                self.focused = Some(self.focus_rects[j].0.clone());
                self.scroll_to = true;
                true
            }
            None => false,
        }
    }

    /// Tab: the next (or previous) side entry that is not a heading.
    pub fn next_side(&self, back: bool) -> Option<String> {
        let spec = self.spec.as_ref()?;
        let rows: Vec<&Item> = spec.side.iter().filter(|i| i.kind != "caption" && !i.id.is_empty()).collect();
        if rows.is_empty() { return None; }
        let at = rows.iter().position(|i| i.on);
        let n = rows.len();
        let j = match (at, back) {
            (None, _) => 0,
            (Some(i), false) => (i + 1) % n,
            (Some(i), true) => (i + n - 1) % n,
        };
        Some(rows[j].id.clone())
    }

    /// Typing with no field focused: into the search.
    pub fn type_text(&mut self, text: &str) -> bool {
        if self.spec.as_ref().is_none_or(|s| s.search.is_none()) { return false; }
        let typed: String = text.chars().filter(|c| !c.is_control()).collect();
        if typed.is_empty() { return false; }
        if self.search.is_empty() && typed.trim().is_empty() { return true; }
        self.search.push_str(&typed);
        true
    }
}

fn form_key(pane: &str, i: usize, b: &Block) -> String {
    if b.id.is_empty() { format!("{pane}{i}") } else { format!("{pane}:{}", b.id) }
}

fn tone(t: &str) -> Color32 {
    match t {
        "dim" => DIM,
        "faint" => FAINT,
        "ok" => FOUND,
        "warn" => CHANGED,
        "error" => MISSING,
        "accent" => ACCENT,
        "off" => OFF,
        _ => INK,
    }
}

/// A picture: the texture cropped to fill (cover), a colour, or a quiet box.
fn picture(ui: &egui::Ui, r: Rect, tex: Option<&egui::TextureHandle>, hex: &str, radius: u8) {
    let cr = CornerRadius::same(radius);
    if let Some(tex) = tex {
        let [w, h] = tex.size();
        let (tw, th) = (w.max(1) as f32, h.max(1) as f32);
        let (rw, rh) = (r.width().max(1.0), r.height().max(1.0));
        // Cover: crop the texture's longer side.
        let uv = if tw / th > rw / rh {
            let u = (rh / th * tw - rw) / (rh / th * tw) / 2.0;
            Rect::from_min_max(pos2(u, 0.0), pos2(1.0 - u, 1.0))
        } else {
            let v = (rw / tw * th - rh) / (rw / tw * th) / 2.0;
            Rect::from_min_max(pos2(0.0, v), pos2(1.0, 1.0 - v))
        };
        egui::Image::new(tex).uv(uv).corner_radius(cr).paint_at(ui, r);
    } else if let Some(c) = parse_hex(hex) {
        ui.painter().rect_filled(r, cr, c);
    } else {
        ui.painter().rect_filled(r, cr, PICTURE_WELL);
        // A box drawing: the document has no picture (yet).
        let c = r.center();
        let k = (r.width().min(r.height()) * 0.16).clamp(5.0, 22.0);
        let pen = Stroke::new(1.2_f32, OFF);
        let top = [c + vec2(0.0, -k), c + vec2(k, -k * 0.5), c + vec2(0.0, 0.0), c + vec2(-k, -k * 0.5)];
        ui.painter().add(egui::Shape::closed_line(top.to_vec(), pen));
        ui.painter().line_segment([c + vec2(-k, -k * 0.5), c + vec2(-k, k * 0.5)], pen);
        ui.painter().line_segment([c + vec2(k, -k * 0.5), c + vec2(k, k * 0.5)], pen);
        ui.painter().line_segment([c, c + vec2(0.0, k)], pen);
        ui.painter().line_segment([c + vec2(-k, k * 0.5), c + vec2(0.0, k)], pen);
        ui.painter().line_segment([c + vec2(k, k * 0.5), c + vec2(0.0, k)], pen);
    }
    ui.painter().rect_stroke(r, cr, Stroke::new(1.0_f32, RULE), egui::StrokeKind::Inside);
}

fn paint_dot(ui: &egui::Ui, c: Pos2, t: &str) {
    dot(ui.painter(), c, tone(t));
}

/// A small document glyph (a linked file, a file row in the side list).
fn file_glyph(ui: &egui::Ui, c: Pos2, ink: Color32) {
    let r = Rect::from_center_size(c, vec2(8.0, 10.0));
    let pen = Stroke::new(1.0_f32, ink);
    let fold = 3.0;
    let pts = vec![r.left_top(), pos2(r.max.x - fold, r.min.y), pos2(r.max.x, r.min.y + fold), r.right_bottom(), r.left_bottom()];
    ui.painter().add(egui::Shape::closed_line(pts, pen));
}

/// A header or block button (role 1 accent, 3 quiet text, else neutral).
fn button(ui: &mut egui::Ui, v: &Verb, width: Option<f32>) -> bool {
    if width.is_none() && v.enabled && v.role != 3 {
        // The kit's sheet button: the same as a question sheet's.
        let clicked = sheet_button(ui, &v.label, &v.key, v.role);
        return clicked;
    }
    if v.role == 3 {
        let g = ui.painter().layout_no_wrap(v.label.clone(), body(), INK);
        let sense = if v.enabled { Sense::click() } else { Sense::hover() };
        let (r, resp) = ui.allocate_exact_size(vec2(width.unwrap_or(g.size().x + SPACE_L), BUTTON_H), sense);
        paint_state(ui, r, state_of(&resp, false, v.enabled), R_CONTROL);
        centered(ui.painter(), r, g,
                            if v.enabled { ACCENT } else { OFF });
        let resp = if v.tip.is_empty() { resp } else { resp.on_hover_text(&v.tip) };
        return resp.clicked() && v.enabled;
    }
    let primary = v.role == 1;
    let ink = if !v.enabled { OFF } else if primary { ON_PRIMARY } else { INK };
    let g = ui.painter().layout_no_wrap(v.label.clone(), if primary { strong(TYPE_BODY) } else { body() }, ink);
    let k = (!v.key.is_empty()).then(|| ui.painter().layout_no_wrap(v.key.clone(), meta(),
        if primary { ON_PRIMARY_HINT } else { FAINT }));
    let kw = k.as_ref().map_or(0.0, |k| k.size().x + 8.0);
    let w = width.unwrap_or((g.size().x + kw + 22.0).max(56.0));
    let sense = if v.enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, BUTTON_H), sense);
    let cr = CornerRadius::same(R_CONTROL);
    if primary && v.enabled {
        let fill = if resp.is_pointer_button_down_on() { PRIMARY_PRESS } else if resp.hovered() { PRIMARY_HOVER } else { ACCENT };
        ui.painter().rect_filled(rect, cr, fill);
    } else {
        ui.painter().rect_filled(rect, cr, HOVER);
        paint_state(ui, rect, state_of(&resp, false, v.enabled), R_CONTROL);
        ui.painter().rect_stroke(rect, cr, Stroke::new(1.0_f32, RULE), egui::StrokeKind::Inside);
    }
    let x0 = rect.center().x - (g.size().x + kw) / 2.0;
    ui.painter().galley(pos2(x0, rect.center().y - g.size().y / 2.0), g.clone(), ink);
    if let Some(k) = k {
        ui.painter().galley(pos2(x0 + g.size().x + 8.0, rect.center().y - k.size().y / 2.0), k, ink);
    }
    let resp = if v.tip.is_empty() { resp } else { resp.on_hover_text(&v.tip) };
    resp.clicked() && v.enabled
}

/// Everything a pane's blocks need, borrowed apart from the Ui.
struct Draw<'a> {
    st: &'a mut LargeSheet,
    icons: &'a HashMap<String, egui::TextureHandle>,
    out: &'a mut Vec<String>,
    /// The content pane: its items take the focus; the detail's are picked.
    focusable: bool,
    pane: &'static str,
    rects: Vec<(String, Rect)>,
}

impl Draw<'_> {
    fn blocks(&mut self, ui: &mut egui::Ui, blocks: &[Block]) {
        for (i, b) in blocks.iter().enumerate() {
            match b.kind.as_str() {
                "caption" => {
                    ui.add_space(if i == 0 { SPACE_XS } else { SPACE_M });
                    ui.horizontal(|ui| {
                        caption_plain(ui, &b.text);
                        if !b.right.is_empty() {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(egui::RichText::new(&b.right).font(meta()).color(FAINT));
                            });
                        }
                    });
                    ui.add_space(SPACE_XS);
                }
                "text" => {
                    let color = if b.tone.is_empty() { DIM } else { tone(&b.tone) };
                    ui.add(egui::Label::new(egui::RichText::new(&b.text).font(body()).color(color)).wrap());
                    ui.add_space(SPACE_XS);
                }
                "credits" => {
                    if let Some(id) = kit::credits_roll(ui, &b.id, &b.credits, ROW_H * 8.) { self.out.push(format!("verb{SEP}{id}")); }
                }
                "prose" => {
                    if let Some(id) = kit::prose(ui, &b.spans) { self.out.push(format!("verb{SEP}{id}")); }
                    ui.add_space(SPACE_XS);
                }
                "separator" => {
                    ui.add_space(SPACE_S);
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0_f32, RULE));
                    ui.add_space(SPACE_S);
                }
                "heading" => self.heading(ui, b),
                "picture" => {
                    let w = ui.available_width();
                    let h = if b.size > 0.0 { b.size } else { (w * 0.45).clamp(80.0, 260.0) };
                    let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
                    picture(ui, r, self.icons.get(&b.icon), &b.hex, 10);
                    ui.add_space(SPACE_M);
                }
                "svg_preview" => {
                    let w = ui.available_width().max(1.);
                    if let Some((dx, dy, zoom)) = kit::svg_preview(ui, self.icons.get(&b.icon),
                            &b.text, vec2(w, w)) {
                        self.out.push(format!("svg_frame{SEP}{dx}{SEP}{dy}{SEP}{zoom}"));
                    }
                }
                "table" => {
                    let columns = b.cells.first().map_or(1, |r| r.len().max(1));
                    let column_width = (ui.available_width() / columns as f32 - SPACE_L).max(55.0);
                    egui::ScrollArea::horizontal().show(ui, |ui| {
                        egui::Grid::new((self.pane, i)).min_col_width(column_width)
                            .spacing(vec2(SPACE_L, SPACE_S)).show(ui, |ui| {
                            for (n, row) in b.cells.iter().enumerate() {
                                let mut bounds = Rect::NOTHING;
                                let id = n.to_string();
                                for cell in row {
                                    let color = if n == 0 { DIM } else if id == b.right { ACCENT } else { INK };
                                    bounds = bounds.union(ui.label(egui::RichText::new(cell).font(body()).color(color)).rect);
                                }
                                if n > 0 && ui.interact(bounds, ui.id().with(n), Sense::click()).clicked() {
                                    self.out.push(format!("pick{SEP}{n}"));
                                }
                                ui.end_row();
                            }
                        });
                    });
                }
                "map" => { if let Some(spec) = &b.map { self.st.map.show(ui,spec,self.out); } }
                "facts" => {
                    for (label, value, _) in &b.rows { kit::catalogue_fact(ui, label, value); }
                }
                "tags" => {
                    let width = ui.available_width();
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(SPACE_XS, SPACE_XS);
                        for tag in &b.items { kit::catalogue_tag(ui, &tag.label, width); }
                    });
                    ui.add_space(SPACE_S);
                }
                "kv" => {
                    for (label, value, t) in &b.rows {
                        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), PROP_H), Sense::hover());
                        let v = fit(ui, value, body(), tone(t), r.width() * 0.62, 1);
                        leading(ui.painter(), r, r.max.x - v.size().x, v, tone(t));
                        let l = fit(ui, label, meta(), DIM, r.width() * 0.36, 1);
                        leading(ui.painter(), r, r.min.x, l, DIM);
                        if t == "warn" || t == "error" || t == "ok" {
                            let vw = ui.painter().layout_no_wrap(value.clone(), body(), INK).size().x.min(r.width() * 0.62);
                            paint_dot(ui, pos2(r.max.x - vw - 9.0, r.center().y), t);
                        }
                    }
                    ui.add_space(SPACE_XS);
                }
                "buttons" => self.buttons(ui, b),
                "tiles" => self.tiles(ui, b),
                "rows" => self.rows(ui, b),
                "fields" => {
                    let key = form_key(self.pane, i, b);
                    if let Some(d) = self.st.forms.get_mut(&key) {
                        let out = &mut *self.out;
                        ui.scope(|ui| {
                            ui.spacing_mut().item_spacing = vec2(0.0, SPACE_XS);
                            // Label and control stay together on a wide sheet.
                            ui.set_max_width(ui.available_width().min(680.0));
                            dialog::fields(ui, d, out);
                        });
                    }
                    ui.add_space(SPACE_S);
                }
                _ => {}
            }
        }
    }

    fn heading(&mut self, ui: &mut egui::Ui, b: &Block) {
        let w = ui.available_width();
        let (r, _) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::hover());
        let right = (!b.right.is_empty()).then(|| fit(ui, &b.right, meta(), FAINT, w * 0.45, 1));
        let rw = right.as_ref().map_or(0.0, |g| g.size().x + 10.0);
        let mut x = r.min.x;
        if !b.hex.is_empty() || !b.icon.is_empty() {
            let sw = Rect::from_min_size(pos2(x, r.center().y - 10.0), vec2(20.0, 20.0));
            picture(ui, sw, self.icons.get(&b.icon), &b.hex, 4);
            x += 28.0;
        }
        let t = fit(ui, &b.text, heading(), INK, r.max.x - x - rw, 1);
        let tw = t.size().x;
        leading(ui.painter(), r, x, t, INK);
        if b.tone == "accent" {
            ui.painter().text(pos2(x + tw + 8.0, r.center().y), egui::Align2::LEFT_CENTER, "★", meta(), ACCENT);
        }
        if let Some(g) = right {
            ui.painter().galley(pos2(r.max.x - g.size().x, r.center().y - g.size().y / 2.0), g, FAINT);
        }
        if !b.sub.is_empty() {
            let g = fit(ui, &b.sub, meta(), FAINT, w, 2);
            let (r, _) = ui.allocate_exact_size(vec2(w, g.size().y + 2.0), Sense::hover());
            ui.painter().galley(r.min, g, FAINT);
        }
        ui.add_space(SPACE_S);
    }

    fn buttons(&mut self, ui: &mut egui::Ui, b: &Block) {
        let w = ui.available_width();
        ui.add_space(SPACE_XS);
        if b.align == "fill" {
            for v in &b.buttons {
                if button(ui, v, Some(w)) { self.out.push(format!("verb{SEP}{}", v.id)); }
                ui.add_space(SPACE_S);
            }
            return;
        }
        let layout = if b.align == "left" { egui::Layout::left_to_right(egui::Align::Center) }
                     else { egui::Layout::right_to_left(egui::Align::Center) };
        ui.allocate_ui_with_layout(vec2(w, BUTTON_H), layout, |ui| {
            ui.spacing_mut().item_spacing = vec2(SPACE_S, 0.0);
            let order: Vec<&Verb> = if b.align == "left" { b.buttons.iter().collect() } else { b.buttons.iter().rev().collect() };
            for v in order {
                if button(ui, v, None) { self.out.push(format!("verb{SEP}{}", v.id)); }
            }
        });
        ui.add_space(SPACE_S);
    }

    /// Report a click / double-click / right-click on an item.
    fn item_input(&mut self, resp: &egui::Response, it: &Item) {
        if it.id.is_empty() { return; }
        if resp.secondary_clicked() && !it.menu.is_empty() {
            let at = resp.interact_pointer_pos().unwrap_or(resp.rect.left_bottom());
            self.st.menu = Some((it.id.clone(), it.menu.clone(), at));
            if self.focusable && !it.fixed {
                self.st.focused = Some(it.id.clone());
                self.out.push(format!("focus{SEP}{}", it.id));
            }
            return;
        }
        if !self.focusable || it.fixed {
            if resp.clicked() { self.out.push(format!("pick{SEP}{}", it.id)); }
            return;
        }
        if resp.double_clicked() {
            self.out.push(format!("open{SEP}{}", it.id));
        } else if resp.clicked() {
            if self.st.focused.as_deref() != Some(it.id.as_str()) {
                self.st.focused = Some(it.id.clone());
                self.out.push(format!("focus{SEP}{}", it.id));
            }
            // Clicking an item ends typing in the search (arrows walk again).
            ui_stop_search(resp);
        }
    }

    fn tiles(&mut self, ui: &mut egui::Ui, b: &Block) {
        let side = if b.size > 0.0 { b.size } else { 96.0 };
        let gap = SPACE_M;
        let w = ui.available_width();
        let catalogue = b.tone == "catalog";
        let report_visible = catalogue || b.tone == "components";
        let (cols, picture_size) = if catalogue { kit::catalogue_grid(w, side) }
            else { ((((w + gap) / (side + gap)).floor() as usize).max(1), vec2(side, side)) };
        let side = picture_size.x;
        let cell = vec2(side, picture_size.y + 36.0);
        let mut visible = Vec::new();
        let rows = b.items.len().div_ceil(cols);
        let h = rows as f32 * (cell.y + gap) - if rows > 0 { gap } else { 0.0 };
        let (block, _) = ui.allocate_exact_size(vec2(w, h.max(0.0)), Sense::hover());
        ui.painter().rect_filled(block, R_ROW, FIELD);
        let clip = ui.clip_rect();
        for (i, it) in b.items.iter().enumerate() {
            let (row, col) = (i / cols, i % cols);
            let r = Rect::from_min_size(block.min + vec2(col as f32 * (side + gap), row as f32 * (cell.y + gap)), cell);
            let focused = self.st.focused.as_deref() == Some(it.id.as_str()) && !it.id.is_empty();
            if self.focusable && !it.fixed && !it.id.is_empty() {
                self.rects.push((it.id.clone(), r));
            }
            if focused && self.st.scroll_to {
                ui.scroll_to_rect(r.expand(8.0), None);
                self.st.scroll_to = false;
            }
            if !clip.intersects(r) { continue; }
            let resp = ui.interact(r, ui.id().with(("lstile", &it.id, i)), Sense::click());
            if report_visible && !it.icon.is_empty() { visible.push(it.icon.clone()); }
            let pic = Rect::from_min_size(r.min, picture_size);
            picture(ui, pic, self.icons.get(&it.icon), &it.hex, 8);
            if resp.hovered() && !focused {
                ui.painter().rect_stroke(pic, CornerRadius::same(8), Stroke::new(1.0_f32, INK_2), egui::StrokeKind::Outside);
            }
            if focused {
                ui.painter().rect_stroke(pic.expand(1.0), CornerRadius::same(9), Stroke::new(2.0_f32, FOCUS_RING), egui::StrokeKind::Outside);
            }
            if it.on {
                // The current one (the material painting): a check on the corner.
                let c = pic.left_top() + vec2(12.0, 12.0);
                ui.painter().circle_filled(c, 8.0, ACCENT);
                ui.painter().text(c, egui::Align2::CENTER_CENTER, "✓", meta(), BRIGHT);
            }
            if !it.badge.is_empty() {
                let g = ui.painter().layout_no_wrap(it.badge.clone(), meta(), INK);
                let br = Rect::from_min_size(pic.right_top() + vec2(-g.size().x - 14.0, 6.0), vec2(g.size().x + 8.0, 16.0));
                ui.painter().rect_filled(br, CornerRadius::same(8), IMAGE_BACK);
                ui.painter().galley(br.center() - g.size() / 2.0, g, INK);
            }
            let ink = if it.dim { FAINT } else { INK };
            let l = fit(ui, &it.label, body(), ink, side, 1);
            ui.painter().galley(pos2(r.min.x, pic.max.y + 5.0), l, ink);
            if !it.sub.is_empty() {
                let s = fit(ui, &it.sub, meta(), FAINT, side, 1);
                ui.painter().galley(pos2(r.min.x, pic.max.y + 20.0), s, FAINT);
            }
            let resp = if it.tip.is_empty() { resp } else { resp.on_hover_text(&it.tip) };
            self.item_input(&resp, it);
        }
        if report_visible && self.st.visible_previews != visible {
            self.out.push(std::iter::once("visible").chain(visible.iter().map(String::as_str)).collect::<Vec<_>>().join(&SEP.to_string()));
            self.st.visible_previews = visible;
        }
        ui.add_space(SPACE_S);
    }

    fn rows(&mut self, ui: &mut egui::Ui, b: &Block) {
        let thumbs = b.items.iter().any(|i| !i.icon.is_empty() || !i.hex.is_empty()) || b.tone == "thumbs";
        let rh = if thumbs { 40.0 } else if b.items.iter().any(|i| !i.sub.is_empty()) { 36.0 } else { ROW_H };
        let w = ui.available_width();
        let h = b.items.len() as f32 * rh;
        let (block, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
        ui.painter().rect_filled(block, R_ROW, FIELD);
        let clip = ui.clip_rect();
        for (i, it) in b.items.iter().enumerate() {
            let r = Rect::from_min_size(block.min + vec2(0.0, i as f32 * rh), vec2(w, rh));
            let focused = self.st.focused.as_deref() == Some(it.id.as_str()) && !it.id.is_empty();
            if self.focusable && !it.fixed && !it.id.is_empty() {
                self.rects.push((it.id.clone(), r));
            }
            if focused && self.st.scroll_to {
                ui.scroll_to_rect(r.expand(4.0), None);
                self.st.scroll_to = false;
            }
            if !clip.intersects(r) { continue; }
            let sense = if it.id.is_empty() { Sense::hover() } else { Sense::click() };
            let resp = ui.interact(r, ui.id().with(("lsrow", &it.id, i)), sense);
            let st = if focused || (it.on && !self.focusable) { State::On }
                     else if it.id.is_empty() || (it.fixed && self.focusable) { State::Idle }
                     else { state_of(&resp, false, true) };
            paint_state(ui, r.shrink2(vec2(0.0, 1.0)), st, R_ROW);
            let mut x = r.min.x + 8.0 + it.depth as f32 * 16.0;
            if thumbs {
                let t = Rect::from_min_size(pos2(x, r.center().y - 15.0), vec2(44.0, 30.0));
                picture(ui, t, self.icons.get(&it.icon), &it.hex, 4);
                x += 54.0;
            } else if it.kind == "file" {
                file_glyph(ui, pos2(x + 5.0, r.center().y), if it.dim { OFF } else { DIM });
                x += 18.0;
            }
            // Right: the dot and its words, then the right text.
            let mut right_x = r.max.x - 8.0;
            if !it.right.is_empty() {
                let g = fit(ui, &it.right, meta(), FAINT, w * 0.3, 1);
                right_x -= g.size().x;
                ui.painter().galley(pos2(right_x, r.center().y - g.size().y / 2.0), g, FAINT);
                right_x -= 12.0;
            }
            let ink = if it.dim { FAINT } else { INK };
            let lw = (right_x - x).max(20.0);
            let l = fit(ui, &it.label, body(), ink, lw, 1);
            let lsize = l.size();
            if it.sub.is_empty() {
                ui.painter().galley(pos2(x, r.center().y - lsize.y / 2.0), l, ink);
            } else {
                ui.painter().galley(pos2(x, r.center().y - lsize.y + 1.0), l, ink);
                let s = fit(ui, &it.sub, meta(), FAINT, lw, 1);
                ui.painter().galley(pos2(x, r.center().y + 1.0), s, FAINT);
            }
            if !it.dot.is_empty() {
                // After the name, as the mockups show a link's state.
                let dx = x + lsize.x + 9.0;
                let cy = if it.sub.is_empty() { r.center().y } else { r.center().y - lsize.y / 2.0 + 1.0 };
                paint_dot(ui, pos2(dx, cy), &it.dot);
                if !it.dot_text.is_empty() {
                    let g = fit(ui, &it.dot_text, meta(), FAINT, (right_x - dx - 10.0).max(10.0), 1);
                    ui.painter().galley(pos2(dx + 8.0, cy - g.size().y / 2.0), g, tone(&it.dot));
                }
            }
            if !it.badge.is_empty() {
                // A count with a dot (problems): amber, before the right text.
                let g = ui.painter().layout_no_wrap(it.badge.clone(), meta(), WARN);
                let bx = right_x - g.size().x;
                ui.painter().galley(pos2(bx, r.center().y - g.size().y / 2.0), g, WARN);
                paint_dot(ui, pos2(bx - 8.0, r.center().y), "warn");
            }
            let resp = if it.tip.is_empty() { resp } else { resp.on_hover_text(&it.tip) };
            self.item_input(&resp, it);
        }
        ui.add_space(SPACE_S);
    }
}

/// A click on an item: the search field stops taking keys.
fn ui_stop_search(resp: &egui::Response) {
    if resp.ctx.memory(|m| m.has_focus(search_id())) {
        resp.ctx.memory_mut(|m| m.surrender_focus(search_id()));
    }
}

fn sheet_bounds(available: Rect, max_width: f32, ease: f32) -> Rect {
    let inset = (available.width() * 0.06).clamp(SPACE_XL, 80.0);
    let width = (available.width() - inset * 2.0).min(max_width);
    let height = (available.height() * 0.84).min(760.0);
    let center = available.center() + vec2(0.0, (1.0 - ease) * 14.0);
    Rect::from_center_size(center, vec2(width, height))
}

fn content_scroll(id: &str) -> egui::ScrollArea {
    egui::ScrollArea::vertical().id_salt(("lsheet_content", id)).auto_shrink([false, false])
}

impl Ui {
    /// Show, update or (empty `json`) close the large sheet. Err: the JSON
    /// did not parse (the sheet stays as it was).
    pub fn set_large_sheet(&mut self, json: &str) -> Result<(), String> {
        if json.trim().is_empty() {
            if self.lsheet.open { self.ctx.memory_mut(|m| m.stop_text_input()); }
            self.lsheet.set(None);
            return Ok(());
        }
        let spec: Spec = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if self.lsheet.set(Some(spec)) {
            // One float at a time: the sheet closes the menus under it.
            self.view_open = false;
            self.menu = None;
            self.cmd_open = false;
            if self.form.take().is_some() { self.actions.push("form:close".into()); }
            if let Some(col) = self.column.as_mut() {
                col.row_menu = None; col.kind_menu = None; col.head_menu = None;
            }
            self.ctx.memory_mut(|m| m.stop_text_input());
        }
        Ok(())
    }

    pub(crate) fn empty_start_shown(&self) -> bool {
        self.lsheet.open && self.lsheet.spec.as_ref().is_some_and(|s| s.id == "start" && !s.closable)
    }

    pub fn large_sheet_shown(&self) -> bool { self.lsheet.open }
    pub fn large_sheet_rect(&self) -> Option<(f32, f32, f32, f32)> {
        self.lsheet.rect.filter(|_| self.lsheet.open).map(|r| (r.min.x, r.min.y, r.width(), r.height()))
    }

    /// The active sheet only. The retained spec paints the closing animation.
    pub fn large_sheet_state(&self) -> (String, String, String) {
        if !self.lsheet.open { return Default::default(); }
        (self.lsheet.id(), self.lsheet.focused.clone().unwrap_or_default(), self.lsheet.search.clone())
    }

    /// Over the sheet: the question sheet, a form, the command palette.
    fn lsheet_covered(&self) -> bool { self.sheet.is_some() || self.dialog.is_some() || self.cmd_open }

    /// Keys while the sheet is up (the caller checked it is, and nothing over it).
    pub(super) fn lsheet_key(&mut self, name: &str, pressed: bool, mods: u8) -> bool {
        let focused = self.ctx.memory(|m| m.focused());
        let in_search = focused == Some(search_id());
        let in_field = focused.is_some() && !in_search;
        let forward = |ui: &mut Self| {
            if let Some(key) = egui::Key::from_name(name) {
                ui.events.push(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: mods_from(mods) });
            }
            true
        };
        if !pressed {
            return if in_field || in_search { forward(self) } else { true };
        }
        let command = mods & 8 != 0;
        match name {
            "Escape" => {
                if self.lsheet.menu.is_none() && self.lsheet.head_menu.is_none()
                    && self.lsheet.forms.values_mut().any(|d| d.escape_control(&self.ctx, &mut self.actions)) {
                    return true;
                }
                let (_, act) = self.lsheet.escape(in_field, in_search);
                match act {
                    Some(a) if a.is_empty() => self.ctx.memory_mut(|m| m.stop_text_input()),
                    Some(a) => { self.ctx.memory_mut(|m| m.stop_text_input()); self.actions.push(a); }
                    None => {}
                }
                true
            }
            "F" if command => {
                if self.lsheet.spec.as_ref().is_some_and(|s| s.search.is_some()) { self.lsheet.focus_search = true; }
                true
            }
            "Enter" if in_field && self.lsheet.spec.as_ref().is_some_and(|spec| !spec.default_verb.is_empty()) => {
                if mods & (2 | 8) == 0 && self.lsheet.forms.values().any(|d| d.multiline_focused(focused)) {
                    return forward(self);
                }
                if let Some(spec) = &self.lsheet.spec {
                    if let Some(verb) = spec.verbs.iter().find(|v| v.enabled && v.id == spec.default_verb) {
                        self.actions.push(self.lsheet.act("verb", &[&verb.id]));
                    }
                }
                true
            }
            "Enter" if !in_field => {
                let id = self.lsheet.focused.clone().or_else(|| self.lsheet.focus_rects.first().map(|f| f.0.clone()));
                if let Some(id) = id {
                    let a = self.lsheet.act("open", &[&id]);
                    self.actions.push(a);
                } else if let Some(spec) = &self.lsheet.spec {
                    if let Some(verb) = spec.verbs.iter().find(|v| v.enabled && v.id == spec.default_verb) {
                        self.actions.push(self.lsheet.act("verb", &[&verb.id]));
                    }
                }
                true
            }
            "ArrowUp" | "ArrowDown" if !in_field => {
                if self.lsheet.step(name) { self.lsheet_report_focus(); }
                true
            }
            "ArrowLeft" | "ArrowRight" if !in_field && !(in_search && !self.lsheet.search.is_empty()) => {
                if self.lsheet.step(name) { self.lsheet_report_focus(); }
                true
            }
            "Tab" if !in_field => {
                if let Some(id) = self.lsheet.next_side(mods & 4 != 0) {
                    let a = self.lsheet.act("side", &[&id]);
                    self.actions.push(a);
                }
                true
            }
            "Backspace" if !in_field && !in_search => {
                self.lsheet.search.pop();
                true
            }
            _ if in_field || in_search => forward(self),
            _ => true,
        }
    }

    fn lsheet_report_focus(&mut self) {
        if let Some(id) = self.lsheet.focused.clone() {
            let a = self.lsheet.act("focus", &[&id]);
            self.actions.push(a);
        }
    }

    /// The sheet's own areas, for the pointer test.
    pub(super) fn lsheet_areas(&self) -> [(&'static str, bool); 3] {
        let on = self.lsheet.open;
        [("lsheet", on), ("lsheet_menu", on && self.lsheet.menu.is_some()),
         ("lsheet_head_menu", on && self.lsheet.head_menu.is_some())]
    }

    /// Draw it (and animate it in or out). Called after the rest of the
    /// chrome, before the command palette, forms and sheet go over it.
    pub(super) fn large_sheet(&mut self, ctx: &egui::Context, screen: Rect) {
        let t = ctx.animate_bool_with_time(egui::Id::new("lsheet_anim"), self.lsheet.open, ANIM_S);
        if t <= 0.0 {
            if !self.lsheet.open && self.lsheet.spec.is_some() {
                self.lsheet.spec = None;
                self.lsheet.forms.clear();
                self.lsheet.focus_rects.clear();
            }
            return;
        }
        let Some(spec) = self.lsheet.spec.take() else { return };
        let ease = 1.0 - (1.0 - t) * (1.0 - t);
        let live = self.lsheet.open && !self.lsheet_covered();
        let mut out: Vec<String> = Vec::new();

        let back = Rect::from_min_max(pos2(screen.left(), TOP_H), screen.max);
        let max_w = if spec.width > 0.0 { spec.width } else { 1240.0 };
        let sheet = sheet_bounds(back, max_w, ease);
        let backdrop = ctx.layer_painter(egui::LayerId::background());
        backdrop.add(crate::glass::pane(screen, 0.0, SHEET_BACKDROP));
        backdrop.rect_filled(screen, CornerRadius::ZERO, SHEET_SCRIM.gamma_multiply(ease));
        self.lsheet.rect = Some(sheet);
        let icons = &self.icons;
        let st = &mut self.lsheet;
        let mut rects: Vec<(String, Rect)> = Vec::new();
        let area = egui::Area::new(egui::Id::new("lsheet"))
            .fixed_pos(back.min)
            .order(egui::Order::Foreground)
            .fade_in(false)
            .interactable(live)
            .show(ctx, |ui| {
                ui.multiply_opacity(ease);
                let (_, scrim) = ui.allocate_exact_size(back.size(), Sense::click_and_drag());
                if scrim.clicked() { st.menu = None; st.head_menu = None; }
                let side_width = if spec.side.is_empty() { 0.0 } else if spec.side_width > 0.0 { spec.side_width } else { 216.0 };
                let detail_width = spec.detail.as_ref().map_or(0.0, |d| if d.width > 0.0 { d.width } else { 340.0 });
                let frame = sheet_frame(ui, sheet, side_width, detail_width, spec.footer.is_some());
                Self::lsheet_header(ui, &spec, st, frame.header, &mut out);
                if let (Some(f), Some(rect)) = (&spec.footer, frame.footer) {
                    Self::lsheet_footer(ui, f, rect, &mut out);
                }
                if let Some(side) = frame.side {
                    Self::lsheet_side(ui, &spec.side, side, &mut out);
                }
                // Content.
                {
                    let mut d = Draw { st: &mut *st, icons, out: &mut out, focusable: true, pane: "c", rects: Vec::new() };
                    let inner = frame.content.shrink(SPACE_M);
                    ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
                        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                        if !spec.content.title.is_empty() {
                            let (r, _) = ui.allocate_exact_size(vec2(inner.width(), ROW_H), Sense::hover());
                            let t = ui.painter().layout_no_wrap(spec.content.title.clone(), title_font(), INK);
                            let tw = t.size().x;
                            leading(ui.painter(), r, r.min.x, t, INK);
                            if !spec.content.sub.is_empty() {
                                ui.painter().text(pos2(r.min.x + tw + 8.0, r.center().y), egui::Align2::LEFT_CENTER,
                                                  &spec.content.sub, meta(), FAINT);
                            }
                            ui.add_space(SPACE_S);
                        }
                        content_scroll(&spec.id)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                                if spec.content.blocks.is_empty() && !spec.content.empty.is_empty() {
                                    ui.add_space(SPACE_XL);
                                    ui.add(egui::Label::new(egui::RichText::new(&spec.content.empty).font(body()).color(FAINT)).wrap());
                                }
                                d.blocks(ui, &spec.content.blocks);
                            });
                    });
                    rects = std::mem::take(&mut d.rects);
                }
                // Detail.
                if let (Some(detail), Some(rect)) = (&spec.detail, frame.detail) {
                    let mut d = Draw { st: &mut *st, icons, out: &mut out, focusable: false, pane: "d", rects: Vec::new() };
                    let inner = rect.shrink(SPACE_M);
                    ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
                        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                        egui::ScrollArea::vertical().id_salt(("lsheet_detail", &spec.id)).auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                                if !detail.title.is_empty() {
                                    ui.label(egui::RichText::new(&detail.title).font(title_font()).color(INK));
                                    ui.add_space(SPACE_S);
                                }
                                if detail.blocks.is_empty() && !detail.empty.is_empty() {
                                    ui.add_space(SPACE_XL);
                                    ui.add(egui::Label::new(egui::RichText::new(&detail.empty).font(body()).color(FAINT)).wrap());
                                }
                                d.blocks(ui, &detail.blocks);
                            });
                    });
                }
            });
        let layer = area.response.layer_id;
        st.focus_rects = rects;
        // An item's menu.
        if let Some((item, entries, at)) = st.menu.clone() {
            let mut close = false;
            let r = egui::Area::new(egui::Id::new("lsheet_menu"))
                .fixed_pos(at)
                .order(egui::Order::Tooltip)
                .constrain_to(screen)
                .show(ctx, |ui| {
                    glass(Surface::Raised).inner_margin(Margin::same(4)).show(ui, |ui| {
                        ui.set_min_width(180.0);
                        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                        for (id, label) in &entries {
                            if id == "-" {
                                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 7.0), Sense::hover());
                                ui.painter().hline(r.x_range().shrink(6.0), r.center().y, Stroke::new(1.0_f32, RULE));
                                continue;
                            }
                            let it = (id.clone(), label.clone(), String::new(), String::new(), -1, true);
                            if menu_row(ui, &it) {
                                out.push(format!("menu{SEP}{item}{SEP}{id}"));
                                close = true;
                            }
                        }
                    });
                });
            let pressed_out = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !r.response.rect.contains(p)));
            if close || (pressed_out && !ctx.input(|i| i.pointer.secondary_pressed())) { st.menu = None; }
        }
        // A header menu.
        if let Some((mid, anchor)) = st.head_menu.clone() {
            let mut close = false;
            if let Some(m) = spec.menus.iter().find(|m| m.id == mid) {
                let r = egui::Area::new(egui::Id::new("lsheet_head_menu"))
                    .fixed_pos(anchor.left_bottom() + vec2(0.0, 4.0))
                    .order(egui::Order::Tooltip)
                    .constrain_to(screen)
                    .show(ctx, |ui| {
                        glass(Surface::Raised).inner_margin(Margin::same(4)).show(ui, |ui| {
                            ui.set_min_width(anchor.width().max(160.0));
                            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                            for o in &m.options {
                                if o.id == "-" {
                                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 7.0), Sense::hover());
                                    ui.painter().hline(r.x_range().shrink(6.0), r.center().y, Stroke::new(1.0_f32, RULE));
                                    continue;
                                }
                                let checked = if o.role == 1 || o.id == m.on { 1 } else { 0 };
                                let it = (o.id.clone(), o.label.clone(), o.key.clone(), "check".to_owned(), checked, o.enabled);
                                if menu_row(ui, &it) {
                                    out.push(format!("headmenu{SEP}{}{SEP}{}", m.id, o.id));
                                    close = true;
                                }
                            }
                        });
                    });
                let pressed_out = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !r.response.rect.contains(p) && !anchor.contains(p)));
                if pressed_out { close = true; }
            } else {
                close = true;
            }
            if close { st.head_menu = None; }
        }
        if st.raise || live {
            // Over the rest of the chrome (the dock, the palette); what may
            // open over the sheet is raised again after it.
            ctx.move_to_top(layer);
            st.raise = false;
            for over in ["menu", "view_menu", "form", "commands", "dialog", "jobs"] {
                ctx.move_to_top(egui::LayerId::new(egui::Order::Foreground, egui::Id::new(over)));
            }
        }
        // The search text, reported once per change.
        if st.search != st.search_reported {
            st.search_reported = st.search.clone();
            out.insert(0, format!("search{SEP}{}", st.search));
        }
        self.lsheet.spec = Some(spec);
        if live {
            let id = self.lsheet.id();
            for a in out {
                // Fields (ui/dialog.rs) carry the prefix already.
                if a.starts_with("lsheet:") { self.actions.push(a); } else { self.actions.push(format!("lsheet:{id}{SEP}{a}")); }
            }
        }
    }

    fn lsheet_header(ui: &mut egui::Ui, spec: &Spec, st: &mut LargeSheet, head: Rect, out: &mut Vec<String>) {
        let inner = head.shrink(SPACE_M);
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            ui.spacing_mut().item_spacing = vec2(SPACE_S, 0.0);
            if !spec.title.is_empty() {
                ui.label(egui::RichText::new(&spec.title).font(heading()).color(INK));
                ui.add_space(SPACE_M);
            }
            if !spec.tabs.is_empty() {
                let tabs: Vec<_> = spec.tabs.iter().map(|t| (t.label.clone(), t.count.clone(), t.enabled)).collect();
                let current = spec.tabs.iter().position(|t| t.on).unwrap_or(tabs.len());
                if let Some(i) = inline_underline_tabs(ui, &tabs, current) {
                    if i != current { out.push(format!("tab{SEP}{}", spec.tabs[i].id)); }
                }
                ui.add_space(SPACE_S);
            }
            for v in &spec.verbs {
                if button(ui, v, None) { out.push(format!("verb{SEP}{}", v.id)); }
            }
            for m in spec.menus.iter().filter(|m| m.left) {
                let open = st.head_menu.as_ref().is_some_and(|h| h.0 == m.id);
                if let Some(r) = menu_button(ui, &m.label, open) {
                    st.head_menu = if open { None } else { Some((m.id.clone(), r)) };
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing = vec2(SPACE_S, 0.0);
                if spec.closable {
                    let (xr, resp) = ui.allocate_exact_size(Vec2::splat(CONTROL_H), Sense::click());
                    paint_state(ui, xr, state_of(&resp, false, true), R_CONTROL);
                    let c = xr.center();
                    let k = 4.5;
                    let pen = Stroke::new(1.4_f32, if resp.hovered() { INK } else { DIM });
                    ui.painter().line_segment([c + vec2(-k, -k), c + vec2(k, k)], pen);
                    ui.painter().line_segment([c + vec2(-k, k), c + vec2(k, -k)], pen);
                    if resp.on_hover_text("Close (esc)").clicked() { out.push("close".into()); }
                }
                if let Some(s) = &spec.search {
                    let sw = (inner.width() * 0.30).clamp(200.0, 320.0);
                    let (fr, _) = ui.allocate_exact_size(vec2(sw, CONTROL_H), Sense::hover());
                    let focused = ui.memory(|m| m.has_focus(search_id()));
                    ui.painter().rect_filled(fr, CornerRadius::same(R_CONTROL), FIELD);
                    ui.painter().rect_stroke(fr, CornerRadius::same(R_CONTROL),
                                             Stroke::new(1.0_f32, if focused { ACCENT } else { EDGE }), egui::StrokeKind::Inside);
                    // The magnifier.
                    let mc = pos2(fr.min.x + 14.0, fr.center().y - 1.0);
                    ui.painter().circle_stroke(mc, 4.0, Stroke::new(1.2_f32, FAINT));
                    ui.painter().line_segment([mc + vec2(3.0, 3.0), mc + vec2(6.0, 6.0)], Stroke::new(1.2_f32, FAINT));
                    let text_r = Rect::from_min_max(pos2(fr.min.x + 24.0, fr.min.y), pos2(fr.max.x - 6.0, fr.max.y));
                    let edit = egui::TextEdit::singleline(&mut st.search).id(search_id()).frame(false).font(body())
                        .hint_text(egui::RichText::new(&s.placeholder).color(FAINT)).margin(Margin::symmetric(4, 6))
                        .desired_width(text_r.width() - 8.0);
                    let resp = ui.put(text_r, edit);
                    if st.focus_search {
                        resp.request_focus();
                        st.focus_search = false;
                    }
                    if !s.key.is_empty() && st.search.is_empty() && !focused {
                        ui.painter().text(pos2(fr.max.x - 8.0, fr.center().y), egui::Align2::RIGHT_CENTER, &s.key, meta(), FAINT);
                    }
                }
                if spec.search.is_some() { ui.add_space(SPACE_XS); }
                for m in spec.menus.iter().rev().filter(|m| !m.left) {
                    let open = st.head_menu.as_ref().is_some_and(|h| h.0 == m.id);
                    if let Some(r) = menu_button(ui, &m.label, open) {
                        st.head_menu = if open { None } else { Some((m.id.clone(), r)) };
                    }
                }
            });
        });
    }

    fn lsheet_footer(ui: &mut egui::Ui, f: &Footer, fr: Rect, out: &mut Vec<String>) {
        let inner = fr.shrink2(vec2(SPACE_M, 2.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            for (i, l) in f.links.iter().enumerate() {
                if i > 0 { ui.label(egui::RichText::new("  ·  ").font(meta()).color(OFF)); }
                let g = ui.painter().layout_no_wrap(l.label.clone(), meta(), FAINT);
                let (r, resp) = ui.allocate_exact_size(g.size() + vec2(0.0, 8.0), Sense::click());
                let ink = if resp.hovered() { INK } else { FAINT };
                ui.painter().galley(pos2(r.min.x, r.center().y - g.size().y / 2.0), g, ink);
                if resp.clicked() { out.push(format!("verb{SEP}{}", l.id)); }
            }
            if !f.hint.is_empty() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(&f.hint).font(meta()).color(FAINT));
                });
            }
        });
    }

    fn lsheet_side(ui: &mut egui::Ui, side: &[Item], side_r: Rect, out: &mut Vec<String>) {
        let inner = side_r.shrink2(vec2(SPACE_S, SPACE_M));
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
            egui::ScrollArea::vertical().id_salt("lsheet_side").auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                for (i, it) in side.iter().enumerate() {
                    if it.kind == "caption" {
                        ui.add_space(if i == 0 { SPACE_XS } else { SPACE_M });
                        ui.horizontal(|ui| { ui.add_space(SPACE_S); caption_plain(ui, &it.label); });
                        ui.add_space(SPACE_XS);
                        continue;
                    }
                    let (r, resp) = list_row(ui, ROW_H, it.on);
                    let mut x = r.min.x + 8.0 + it.depth as f32 * 14.0;
                    let chev = Rect::from_min_size(pos2(x - 2.0, r.min.y), vec2(14.0, r.height()));
                    if it.fold >= 0 {
                        ui.painter().text(chev.center(), egui::Align2::CENTER_CENTER, if it.fold == 1 { "▾" } else { "▸" }, meta(), FAINT);
                        x += 14.0;
                    }
                    if it.kind == "file" {
                        file_glyph(ui, pos2(x + 4.0, r.center().y), if it.dim { OFF } else { DIM });
                        x += 16.0;
                    } else if !it.hex.is_empty() {
                        let sw = Rect::from_center_size(pos2(x + 6.0, r.center().y), vec2(12.0, 12.0));
                        ui.painter().rect_filled(sw, CornerRadius::same(3), parse_hex(&it.hex).unwrap_or(SWATCH_FALLBACK));
                        x += 18.0;
                    }
                    let count_w = if it.right.is_empty() { 0.0 } else { 30.0 };
                    let ink = if it.on { BRIGHT } else if it.dim { FAINT } else { INK };
                    let g = fit(ui, &it.label, if it.on { strong(TYPE_BODY) } else { body() }, ink, r.max.x - x - 8.0 - count_w, 1);
                    ui.painter().galley(pos2(x, r.center().y - g.size().y / 2.0), g, ink);
                    if !it.right.is_empty() {
                        ui.painter().text(pos2(r.max.x - 8.0, r.center().y), egui::Align2::RIGHT_CENTER, &it.right, meta(), FAINT);
                    }
                    if !it.dot.is_empty() { paint_dot(ui, pos2(r.max.x - 8.0 - count_w - 2.0, r.center().y), &it.dot); }
                    let resp = if it.tip.is_empty() { resp } else { resp.on_hover_text(&it.tip) };
                    if resp.clicked() && !it.id.is_empty() {
                        let on_chev = it.fold >= 0 && resp.interact_pointer_pos().is_some_and(|p| p.x < chev.max.x + 2.0);
                        out.push(format!("{}{SEP}{}", if on_chev { "fold" } else { "side" }, it.id));
                    }
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_json() -> String {
        r#"{"id":"t","title":"Test","search":{"placeholder":"Search"},
            "side":[{"id":"a","label":"A","on":true},{"kind":"caption","label":"More"},{"id":"b","label":"B"}],
            "content":{"blocks":[{"kind":"tiles","items":[{"id":"x","label":"X"},{"id":"y","label":"Y"},{"id":"z","label":"Z"}]},
                                 {"kind":"fields","fields":[["f1","check","Snap","",1,0,1,[],0,""]]}]},
            "detail":{"blocks":[{"kind":"kv","rows":[["Base","Image",""]]}]},
            "focus":"y"}"#.to_owned()
    }

    fn laid_out(s: &mut LargeSheet) {
        s.focus_rects = ["x", "y", "z"].iter().enumerate()
            .map(|(i, id)| (id.to_string(), Rect::from_min_size(pos2(i as f32 * 110.0, 0.0), vec2(96.0, 130.0)))).collect();
    }

    #[test]
    fn preferences_shrinks_below_its_tallest_page() {
        let screen = Rect::from_min_size(pos2(0., 0.), vec2(1280., 480.));
        let available = Rect::from_min_max(pos2(0., TOP_H), screen.max);
        let sheet = sheet_bounds(available, 1100., 1.);
        assert!(screen.contains_rect(sheet));
        assert!(sheet.height() <= 400.);
    }

    #[test]
    fn preferences_scrolls_while_its_actions_fit_a_short_screen() {
        let screen = Rect::from_min_size(pos2(0., 0.), vec2(1280., 480.));
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let available = Rect::from_min_max(pos2(0., TOP_H), screen.max);
                let sheet = sheet_bounds(available, 1100., 1.);
                let frame = sheet_frame(ui, sheet, 216., 0., true);
                let footer = frame.footer.unwrap();
                assert!(sheet.contains_rect(footer) && sheet.contains_rect(frame.header));
                assert!(frame.content.bottom() <= footer.top());
                ui.scope_builder(egui::UiBuilder::new().max_rect(frame.content), |ui| {
                    let scroll = content_scroll("settings").show(ui, |ui| {
                        ui.allocate_space(vec2(200., 1400.));
                    });
                    assert!(scroll.content_size.y > scroll.inner_rect.height());
                    assert!(frame.content.contains_rect(scroll.inner_rect));
                });
            });
        });
    }

    #[test]
    fn preferences_stops_growing_at_its_natural_size() {
        let screen = Rect::from_min_size(pos2(0., 0.), vec2(4000., 3000.));
        let available = Rect::from_min_max(pos2(0., TOP_H), screen.max);
        let sheet = sheet_bounds(available, 1100., 1.);
        assert_eq!(sheet.size(), vec2(1100., 760.));
    }

    #[test]
    fn start_promotes_only_the_action_for_the_current_selection() {
        let mut sheet = LargeSheet::default();
        for (focus, header, detail) in [("tpl:blank", 0, 1), ("continue", 1, 0), ("", 1, 0)] {
            let mut spec: Spec = serde_json::from_str(r#"{"id":"start",
                "verbs":[{"id":"new","role":1}],
                "detail":{"blocks":[{"kind":"buttons","buttons":[{"id":"open","role":1}]}]}}"#).unwrap();
            spec.focus = Some(focus.to_owned());
            sheet.set(Some(spec));
            let drawn = sheet.spec.as_ref().unwrap();
            assert_eq!(drawn.verbs[0].role, header);
            assert_eq!(drawn.detail.as_ref().unwrap().blocks[0].buttons[0].role, detail);
        }
    }

    #[test]
    fn a_spec_parses_with_defaults_and_opening_takes_its_focus() {
        let spec: Spec = serde_json::from_str(&spec_json()).unwrap();
        assert!(spec.closable, "closable unless told");
        assert_eq!(spec.side[0].fold, -1);
        assert_eq!(spec.content.blocks[1].fields[0].1, "check");
        let mut s = LargeSheet::default();
        assert!(s.set(Some(spec.clone())), "the first spec opens it");
        assert_eq!(s.focused.as_deref(), Some("y"));
        assert_eq!(s.forms.len(), 1, "one form per fields block");
        s.focused = Some("z".into());
        assert!(!s.set(Some(spec.clone())), "the same sheet again only updates");
        assert_eq!(s.focused.as_deref(), Some("z"), "an update keeps the user's focus");
        let mut forced = spec;
        forced.force_focus = true;
        s.set(Some(forced));
        assert_eq!(s.focused.as_deref(), Some("y"), "unless Python forces it");
        s.set(None);
        assert!(!s.open);
        assert!(s.spec.is_some(), "kept while it animates out");
    }

    #[test]
    fn closed_sheet_state_is_empty_before_its_exit_animation_finishes() {
        let Some(mut ui) = super::super::test_chrome() else { return };
        ui.set_large_sheet(&spec_json()).unwrap();
        assert_eq!(ui.large_sheet_state().0, "t");
        ui.set_large_sheet("").unwrap();
        assert!(ui.lsheet.spec.is_some());
        assert!(!ui.large_sheet_shown());
        assert_eq!(ui.large_sheet_state(), Default::default());
    }

    #[test]
    fn escape_dismisses_the_innermost_thing_then_closes() {
        let mut s = LargeSheet::default();
        s.set(Some(serde_json::from_str(&spec_json()).unwrap()));
        s.menu = Some(("x".into(), vec![("a".into(), "A".into())], Pos2::ZERO));
        assert_eq!(s.escape(false, false), (true, None));
        assert!(s.menu.is_none());
        assert_eq!(s.escape(true, false), (true, Some(String::new())), "a field being typed in stops first");
        s.search = "oak".into();
        assert_eq!(s.escape(false, false), (true, None));
        assert!(s.search.is_empty(), "then the search text goes");
        assert_eq!(s.escape(false, false), (true, Some("lsheet:t\u{1f}close".into())));
        s.spec.as_mut().unwrap().closable = false;
        assert_eq!(s.escape(false, false), (true, None), "a sheet with nothing behind it stays");
    }

    #[test]
    fn arrows_walk_the_items_and_tab_walks_the_side() {
        let mut s = LargeSheet::default();
        s.set(Some(serde_json::from_str(&spec_json()).unwrap()));
        laid_out(&mut s);
        assert!(s.step("ArrowRight"));
        assert_eq!(s.focused.as_deref(), Some("z"));
        assert!(!s.step("ArrowRight"), "the end of the row");
        s.step("ArrowLeft");
        s.step("ArrowLeft");
        assert_eq!(s.focused.as_deref(), Some("x"));
        assert_eq!(s.next_side(false).as_deref(), Some("b"), "headings are skipped");
        assert_eq!(s.next_side(true).as_deref(), Some("b"));
    }

    #[test]
    fn typing_goes_to_the_search_and_a_preset_search_is_taken_on_open() {
        let mut s = LargeSheet::default();
        let mut spec: Spec = serde_json::from_str(&spec_json()).unwrap();
        spec.search.as_mut().unwrap().text = "beech".into();
        s.set(Some(spec.clone()));
        assert_eq!(s.search, "beech");
        assert!(s.type_text("y"));
        assert_eq!(s.search, "beechy");
        spec.search.as_mut().unwrap().text = "b".into();
        s.set(Some(spec.clone()));
        assert_eq!(s.search, "beechy", "Python's echo never undoes typing");
        assert!(s.type_text(" "));
        assert_eq!(s.search, "beechy ");
        s.search.clear();
        assert!(s.type_text(" "));
        assert!(s.search.is_empty(), "a leading space is not a search");
    }

    fn fonts(ctx: &egui::Context) {
        let mut fonts = egui::FontDefinitions::default();
        let base = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
        for name in ["medium", "semibold"] {
            fonts.families.insert(egui::FontFamily::Name(name.into()), base.clone());
        }
        ctx.set_fonts(fonts);
    }

    #[test]
    fn it_lays_out_at_1280_by_800_and_reports_clicks() {
        // The Ui needs a device; draw the sheet with a bare context instead.
        let ctx = egui::Context::default();
        fonts(&ctx);
        let mut s = LargeSheet::default();
        s.set(Some(serde_json::from_str(&spec_json()).unwrap()));
        let icons = HashMap::new();
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0));
        let mut out = Vec::new();
        for _ in 0..2 {
            let spec = s.spec.clone().unwrap();
            let _ = ctx.run(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut d = Draw { st: &mut s, icons: &icons, out: &mut out, focusable: true, pane: "c", rects: Vec::new() };
                    d.blocks(ui, &spec.content.blocks);
                    let rects = std::mem::take(&mut d.rects);
                    s.focus_rects = rects;
                });
            });
        }
        assert_eq!(s.focus_rects.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), ["x", "y", "z"]);
        assert!(out.is_empty(), "laying out reports nothing: {out:?}");
        let x = s.focus_rects[0].1.center();
        for pressed in [true, false] {
            let ev = egui::Event::PointerButton { pos: x, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
            let spec = s.spec.clone().unwrap();
            let _ = ctx.run(egui::RawInput { screen_rect: Some(screen), events: vec![egui::Event::PointerMoved(x), ev], ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut d = Draw { st: &mut s, icons: &icons, out: &mut out, focusable: true, pane: "c", rects: Vec::new() };
                    d.blocks(ui, &spec.content.blocks);
                });
            });
        }
        assert_eq!(out, vec!["focus\u{1f}x".to_owned()]);
        assert_eq!(s.focused.as_deref(), Some("x"));
    }
}
