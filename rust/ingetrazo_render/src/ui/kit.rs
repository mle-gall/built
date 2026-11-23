//! Shared egui controls for Built.
use egui::{pos2, vec2, Color32, CornerRadius, Pos2, Rect, Sense, Stroke};
use super::tokens::*;
pub mod atoms {
use super::*;
/// The visibility eye: open, or closed with a slash.
pub fn paint_eye(ui: &egui::Ui, r: Rect, on: bool) {
    let c = r.center();
    let col = if on { DIM } else { WARN };
    let pts: Vec<Pos2> = (0..=16).map(|i| {
        let t = std::f32::consts::PI * i as f32 / 16.0;
        pos2(c.x - 6.0 * t.cos(), c.y - 3.6 * t.sin())
    }).chain((0..=16).map(|i| {
        let t = std::f32::consts::PI * i as f32 / 16.0;
        pos2(c.x + 6.0 * t.cos(), c.y + 3.6 * t.sin())
    })).collect();
    ui.painter().add(egui::Shape::closed_line(pts, Stroke::new(1.2_f32, col)));
    ui.painter().circle_filled(c, 1.8, col);
    if !on {
        ui.painter().line_segment([c + vec2(-6.0, 5.0), c + vec2(6.0, -5.0)], Stroke::new(1.3_f32, col));
    }
}

/// The lock: shackle closed when locked, open when not.
pub fn paint_lock(ui: &egui::Ui, r: Rect, on: bool) {
    let c = r.center();
    let col = if on { INK } else { FAINT };
    let body = Rect::from_center_size(c + vec2(0.0, 2.0), vec2(9.0, 7.0));
    ui.painter().rect_filled(body, CornerRadius::same(2), col);
    let shackle = Rect::from_center_size(c + vec2(if on { 0.0 } else { 2.5 }, -2.5), vec2(6.0, 7.0));
    ui.painter().rect_stroke(shackle, CornerRadius::same(3), Stroke::new(1.3_f32, col), egui::StrokeKind::Middle);
    ui.painter().rect_filled(body, CornerRadius::same(2), col);
}


pub fn dot(painter: &egui::Painter, center: Pos2, color: Color32) { painter.circle_filled(center, 3., color); }
pub fn label(ui: &mut egui::Ui, text: &str) -> egui::Response { ui.label(egui::RichText::new(text).font(body()).color(INK)) }
}
pub use atoms::*;
pub mod molecules {
use super::*;
/// The one state painter: every control's fill and line come from here.
#[derive(Clone, Copy, PartialEq)]
pub enum State { Idle, Hover, Press, On, Focus, Off }
pub fn state_of(resp: &egui::Response, on: bool, enabled: bool) -> State {
    if !enabled || !resp.enabled() { State::Off }
    else if on { State::On }
    else if resp.is_pointer_button_down_on() { State::Press }
    else if resp.hovered() { State::Hover }
    else if resp.has_focus() { State::Focus } else { State::Idle }
}
pub fn paint_state(ui: &egui::Ui, rect: Rect, st: State, radius: u8) {
    let r = CornerRadius::same(radius);
    match st {
        State::Hover => { ui.painter().rect_filled(rect, r, HOVER); }
        State::Press => { ui.painter().rect_filled(rect, r, PRESS); }
        State::Focus => { ui.painter().rect_stroke(rect, r, Stroke::new(1.5, FOCUS_RING), egui::StrokeKind::Inside); }
        State::On => {
            ui.painter().rect_filled(rect, r, ACTIVE);
            ui.painter().rect_stroke(rect.shrink(0.25), r, Stroke::new(0.5_f32, ACCENT_LINE), egui::StrokeKind::Inside);
        }
        _ => {}
    }
}
pub fn ink_of(st: State) -> Color32 {
    match st { State::Off => OFF, State::On => BRIGHT, State::Idle => INK_2, _ => INK }
}

/// A section caption: small, spaced capitals, faint.
pub fn caption(ui: &mut egui::Ui, text: &str) { caption_at(ui, text, 8.0) }
/// A caption lined up with the text of the rows under it (8 in the column, 12 in menus).
pub fn caption_at(ui: &mut egui::Ui, text: &str, pad: f32) {
    ui.horizontal(|ui| { ui.add_space(pad); caption_plain(ui, text); });
}
pub fn caption_plain(ui: &mut egui::Ui, text: &str) {
    let job = egui::text::LayoutJob::single_section(text.to_uppercase(), egui::TextFormat {
        font_id: strong(10.0), color: FAINT, extra_letter_spacing: 0.6, ..Default::default() });
    ui.label(job);
}

/// A small command chip (inspector next steps): neutral wash, 22 high.
pub fn chip_button(ui: &mut egui::Ui, label: &str) -> bool {
    let g = ui.painter().layout_no_wrap(label.to_owned(), meta(), INK);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 20.0, 24.0), Sense::click());
    let st = state_of(&resp, false, true);
    ui.painter().rect_filled(rect, CornerRadius::same(R_CONTROL), HOVER);
    paint_state(ui, rect, st, R_CONTROL);
    ui.painter().rect_stroke(rect, CornerRadius::same(R_CONTROL), Stroke::new(1.0_f32, EDGE), egui::StrokeKind::Inside);
    ui.painter().galley(rect.center() - g.size() / 2.0, g, INK);
    resp.clicked()
}

/// A sheet button, 28 high: the default in accent, the rest a neutral
/// wash; the key that answers it in faint type after the label.
pub fn sheet_button(ui: &mut egui::Ui, label: &str, hint: &str, role: u8) -> bool {
    let primary = role == 1;
    let ink = if primary { BRIGHT } else if role == 2 { ERROR } else { INK };
    let g = ui.painter().layout_no_wrap(label.to_owned(), strong(12.0), ink);
    let k = (!hint.is_empty()).then(|| ui.painter().layout_no_wrap(hint.to_owned(), meta(),
        if primary { ON_PRIMARY_HINT } else { FAINT }));
    let kw = k.as_ref().map_or(0.0, |k| k.size().x + 8.0);
    let (rect, resp) = ui.allocate_exact_size(vec2((g.size().x + kw + 24.0).max(72.0), 28.0), Sense::click());
    let r = CornerRadius::same(R_CONTROL);
    if primary {
        let fill = if resp.is_pointer_button_down_on() { PRIMARY_PRESS }
                   else if resp.hovered() { PRIMARY_HOVER } else { ACCENT };
        ui.painter().rect_filled(rect, r, fill);
    } else {
        ui.painter().rect_filled(rect, r, HOVER);
        paint_state(ui, rect, state_of(&resp, false, true), R_CONTROL);
        ui.painter().rect_stroke(rect, r, Stroke::new(1.0_f32, EDGE), egui::StrokeKind::Inside);
    }
    let x0 = rect.center().x - (g.size().x + kw) / 2.0;
    ui.painter().galley(pos2(x0, rect.center().y - g.size().y / 2.0), g.clone(), ink);
    if let Some(k) = k {
        ui.painter().galley(pos2(x0 + g.size().x + 8.0, rect.center().y - k.size().y / 2.0), k, ink);
    }
    resp.clicked()
}

/// A warning chip: amber text on an amber wash, no outline.
pub fn amber_button(ui: &mut egui::Ui, label: &str) -> bool {
    let g = ui.painter().layout_no_wrap(label.to_owned(), strong(11.0), WARN);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 20.0, 24.0), Sense::click());
    let fill = if resp.is_pointer_button_down_on() {
        WARNING_PRESS
    } else if resp.hovered() {
        WARNING_HOVER
    } else { WARN_FILL };
    ui.painter().rect_filled(rect, CornerRadius::same(R_CONTROL), fill);
    ui.painter().galley(rect.center() - g.size() / 2.0, g, WARN);
    resp.clicked()
}

/// The dock's place switch: one inset track, the current place raised.
/// Returns the segment clicked (also when it is the current one: a click on
/// the lit "Project" while in Problems returns to the tree).
pub fn segmented(ui: &mut egui::Ui, labels: &[&str], at: usize, width: f32) -> Option<usize> {
    let (track, _) = ui.allocate_exact_size(vec2(width, 24.0), Sense::hover());
    ui.painter().rect_filled(track, CornerRadius::same(R_CONTROL), FIELD);
    let n = labels.len().max(1) as f32;
    let seg_w = (width - 4.0) / n;
    let mut hit = None;
    for (i, label) in labels.iter().enumerate() {
        let r = Rect::from_min_size(pos2(track.min.x + 2.0 + i as f32 * seg_w, track.min.y + 2.0), vec2(seg_w, 20.0));
        let resp = ui.interact(r, ui.id().with(("seg", i)), Sense::click());
        let on = i == at;
        if on {
            ui.painter().rect_filled(r, CornerRadius::same(R_ROW), PRESS);
            ui.painter().rect_stroke(r, CornerRadius::same(R_ROW), Stroke::new(1.0_f32, EDGE), egui::StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(R_ROW), HOVER);
        }
        let (font, ink) = if on { (strong(12.0), INK) } else { (body(), if resp.hovered() { INK } else { DIM }) };
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, *label, font, ink);
        if resp.clicked() { hit = Some(i); }
    }
    hit
}

/// Second-level tabs: text on a hairline, the current one underlined in
/// accent. (label, count or note, enabled). Labels shrink to fit the row.
pub fn underline_tabs(ui: &mut egui::Ui, tabs: &[(String, String, bool)], at: usize) -> Option<usize> {
    let width = ui.available_width();
    let (row, _) = ui.allocate_exact_size(vec2(width, 26.0), Sense::hover());
    ui.painter().hline(row.x_range(), row.max.y - 0.5, Stroke::new(1.0_f32, EDGE));
    let lay = |t: &(String, String, bool), on: bool| {
        let label = ui.painter().layout_no_wrap(t.0.clone(), if on { strong(12.0) } else { body() }, INK);
        let count = (!t.1.is_empty()).then(|| ui.painter().layout_no_wrap(t.1.clone(), egui::FontId::proportional(10.5), FAINT));
        (label, count)
    };
    let galleys: Vec<_> = tabs.iter().enumerate().map(|(i, t)| lay(t, i == at)).collect();
    let natural: f32 = galleys.iter().map(|(l, c)| l.size().x + c.as_ref().map_or(0.0, |c| c.size().x + 4.0) + 16.0).sum();
    // Too wide: drop the counts of the tabs that are not current.
    let squeeze = natural > width;
    let mut x = row.min.x;
    let mut hit = None;
    for (i, ((label, count), t)) in galleys.into_iter().zip(tabs).enumerate() {
        let on = i == at;
        let show_count = count.filter(|_| on || !squeeze);
        let w = label.size().x + show_count.as_ref().map_or(0.0, |c| c.size().x + 4.0) + 16.0;
        let r = Rect::from_min_size(pos2(x, row.min.y), vec2(w, row.height()));
        let sense = if t.2 { Sense::click() } else { Sense::hover() };
        let resp = ui.interact(r, ui.id().with(("utab", i)), sense);
        let ink = if !t.2 { OFF } else if on { INK } else if resp.hovered() { INK } else { DIM };
        let cy = r.center().y - 1.0;
        let lw = label.size().x;
        ui.painter().galley_with_override_text_color(pos2(r.min.x + 8.0, cy - label.size().y / 2.0), label, ink);
        if let Some(c) = show_count {
            ui.painter().galley(pos2(r.min.x + 8.0 + lw + 4.0, cy - c.size().y / 2.0 + 0.5), c, FAINT);
        }
        if on {
            ui.painter().rect_filled(Rect::from_min_max(pos2(r.min.x + 6.0, r.max.y - 2.0), pos2(r.max.x - 6.0, r.max.y)),
                CornerRadius::same(1), ACCENT);
        }
        let resp = if t.2 { resp } else { resp.on_hover_text("Coming") };
        if resp.clicked() && t.2 { hit = Some(i); }
        x += w;
    }
    hit
}

/// The focus ring and hover wash of a card.
pub fn paint_card(ui: &egui::Ui, r: Rect, hovered: bool, focused: bool) {
    let cr = CornerRadius::same(10);
    ui.painter().rect_filled(r, cr, if hovered { CARD_HOVER } else { CARD });
    if focused {
        
        ui.painter().rect_stroke(r, cr, Stroke::new(1.5_f32, FOCUS_RING), egui::StrokeKind::Inside);
    } else {
        ui.painter().rect_stroke(r, cr, Stroke::new(1.0_f32, EDGE), egui::StrokeKind::Inside);
    }
}

pub fn menu_row(ui: &mut egui::Ui, item: &super::super::Item) -> bool {
    menu_row_resp(ui, item).0.clicked()
}

/// A menu row, 24 high: optional check lane (only when the menu has checks:
/// the caller marks that with item.3 == "check"), label, shortcut or › right.
pub fn menu_row_resp(ui: &mut egui::Ui, item: &super::super::Item) -> (egui::Response, Rect) {
    let (_, label, tip, lane, checked, enabled) = item;
    let sense = if *enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), sense);
    let st = state_of(&resp, false, *enabled);
    paint_state(ui, rect, st, R_ROW);
    let ink = if *enabled { INK } else { OFF };
    let x = if lane == "check" || *checked >= 0 { 28.0 } else { 12.0 };
    if *checked == 1 {
        ui.painter().text(pos2(rect.min.x + 14.0, rect.center().y), egui::Align2::CENTER_CENTER, "✓", body(), ACCENT);
    }
    ui.painter().text(pos2(rect.min.x + x, rect.center().y), egui::Align2::LEFT_CENTER, label, body(), ink);
    if !tip.is_empty() {
        ui.painter().text(pos2(rect.max.x - 12.0, rect.center().y), egui::Align2::RIGHT_CENTER, tip, meta(), FAINT);
    }
    (resp, rect)
}


pub fn field(text: &mut String) -> egui::TextEdit<'_> { egui::TextEdit::singleline(text).font(body()) }
pub fn search<'a>(text: &'a mut String, hint: &str) -> egui::TextEdit<'a> {
    field(text).hint_text(egui::RichText::new(hint).color(FAINT))
}
pub fn number<'a>(value: &'a mut f64, units: &str) -> egui::DragValue<'a> {
    egui::DragValue::new(value).suffix(format!(" {units}"))
}
pub fn swatch(ui: &mut egui::Ui, value: &mut Color32) -> egui::Response { ui.color_edit_button_srgba(value) }
pub fn toggle(ui: &mut egui::Ui, value: &mut bool, label: &str) -> egui::Response { ui.toggle_value(value, label) }
pub fn choice(ui: &mut egui::Ui, id: impl std::hash::Hash, value: &mut usize, labels: &[&str]) {
    egui::ComboBox::from_id_salt(id).show_index(ui, value, labels.len(), |i| labels[i]);
}
pub fn progress(fraction: f32) -> egui::ProgressBar { egui::ProgressBar::new(fraction).fill(INK_2) }
pub fn ring(ui: &mut egui::Ui) { ui.add(egui::Spinner::new().color(INK_2)); }
pub fn quiet(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(egui::Button::new(egui::RichText::new(label).font(body())).frame(false))
}
pub fn icon_button(ui: &mut egui::Ui, glyph: &str, name: &str) -> egui::Response {
    quiet(ui, glyph).on_hover_text(name)
}
pub fn badge(ui: &mut egui::Ui, label: &str, tone: Color32) {
    egui::Frame::new().fill(WELL).corner_radius(R_ROW).inner_margin(egui::Margin::symmetric(6,2))
        .show(ui, |ui| ui.label(egui::RichText::new(label).font(meta()).color(tone)));
}
pub fn empty_state(ui: &mut egui::Ui, text: &str) { ui.label(egui::RichText::new(text).font(body()).color(FAINT)); }
pub fn list_row(ui: &mut egui::Ui, height: f32, selected: bool) -> (Rect, egui::Response) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    paint_state(ui, rect, state_of(&resp, selected, true), R_ROW);
    (rect, resp)
}
pub fn tree_slots(ui: &egui::Ui, count_right: f32, y: f32, eye: i8, lock: i8, show: bool) -> (Rect, Rect) {
    let slot = |dx| Rect::from_center_size(pos2(count_right - dx, y), vec2(16.,16.));
    let (e,l) = (slot(64.),slot(44.));
    if show && eye >= 0 { paint_eye(ui,e,eye == 1); }
    if show && lock >= 0 { paint_lock(ui,l,lock == 1); }
    (e,l)
}
}
pub use molecules::*;
pub mod organisms {
use super::*;
use egui::{Margin, Shadow};
/// A glass surface with egui content on it: shadow, then the blurred model
/// through the rounded shape (glass.rs), then the content. Built like an
/// egui Frame so call sites read the same.
pub struct Glass {
    surface: Surface,
    margin: Margin,
    radius: u8,
    accent: Option<Color32>,
}
pub fn frame() -> Glass { glass(Surface::Pill) }
pub fn glass(surface: Surface) -> Glass {
    Glass { surface, margin: Margin::same(4), radius: surface.radius(), accent: None }
}
impl Glass {
    pub fn inner_margin(mut self, m: Margin) -> Self { self.margin = m; self }
    pub fn corner_radius(mut self, r: CornerRadius) -> Self { self.radius = r.nw; self }
    /// An accent outline (a field being typed into).
    pub fn stroke(mut self, s: Stroke) -> Self { if s.color != EDGE { self.accent = Some(s.color); } self }
    pub fn show<R>(self, ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
        let shadow_at = ui.painter().add(egui::Shape::Noop);
        let glass_at = ui.painter().add(egui::Shape::Noop);
        let mut f = egui::Frame::new().inner_margin(self.margin).corner_radius(CornerRadius::same(self.radius));
        if let Some(c) = self.accent {
            f = f.stroke(Stroke::new(1.0_f32, c));
        }
        let inner = f.show(ui, add);
        let rect = inner.response.rect;
        let contact = Shadow { offset: [0, 1], blur: 2, spread: 0, color: CONTACT };
        ui.painter().set(shadow_at, egui::Shape::Vec(vec![
            egui::Shape::Rect(self.surface.shadow().as_shape(rect, CornerRadius::same(self.radius))),
            egui::Shape::Rect(contact.as_shape(rect, CornerRadius::same(self.radius))),
        ]));
        ui.painter().set(glass_at, crate::glass::pane(rect, self.radius as f32, self.surface.look()));
        inner
    }
}


pub fn sheet_frame(ui: &mut egui::Ui, title: &str, side: impl FnOnce(&mut egui::Ui),
    content: impl FnOnce(&mut egui::Ui), detail: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new().fill(SHEET).inner_margin(8).corner_radius(R_CONTROL).show(ui, |ui| {
        ui.label(egui::RichText::new(title).font(heading()).color(INK));
        ui.separator();
        ui.columns(3, |panes| { side(&mut panes[0]); content(&mut panes[1]); detail(&mut panes[2]); });
    });
}
pub fn tree(ui: &mut egui::Ui, id: impl std::hash::Hash, rows: impl FnOnce(&mut egui::Ui)) {
    egui::ScrollArea::vertical().id_salt(id).show(ui, rows);
}
pub fn activity_capsule() -> Glass { glass(Surface::Pill).inner_margin(Margin::symmetric(8,2)) }
pub fn popover() -> Glass { glass(Surface::Raised) }
pub fn toast(ui: &mut egui::Ui, opacity: f32, content: impl FnOnce(&mut egui::Ui)) {
    ui.scope(|ui| { ui.set_opacity(opacity); popover().show(ui,content); });
}
pub fn toast_stack<'a,T:'a>(ui: &mut egui::Ui, rows: impl Iterator<Item=(&'a T,f32)>, mut render: impl FnMut(&mut egui::Ui,&T)) {
    for (row, opacity) in rows.filter(|(_,a)| *a > 0.).take(3) { toast(ui,opacity,|ui| render(ui,row)); }
}
}
pub use organisms::*;
