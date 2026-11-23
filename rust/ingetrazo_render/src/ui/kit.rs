//! Shared egui controls for Built.
//!
//! Every inline control is `CONTROL_H` high and centres its text with
//! [`centered`], so a row of them shares one baseline. State looks come from
//! [`paint_state`]: hover and press are neutral washes, selection is a quiet
//! accent wash (rows add a leading marker), focus is a thin ring outside the
//! control and never a fill.
mod bevel_curve;
pub use bevel_curve::CurveEditor;
use super::tokens::*;
use egui::{pos2, vec2, Color32, CornerRadius, Pos2, Rect, Sense, Stroke};

/// The egui style every Built context starts from: the kit's state looks for
/// egui's own widgets (text fields, numbers, choices).
pub fn install_style(s: &mut egui::Style) {
    s.interaction.tooltip_delay = 0.25;
    s.visuals = egui::Visuals::dark();
    s.animation_time = MOTION_QUICK;
    s.visuals.window_fill = POPOVER;
    s.visuals.panel_fill = PANEL;
    s.visuals.override_text_color = Some(INK);
    s.visuals.popup_shadow = egui::Shadow {
        offset: [0, 4],
        blur: 12,
        spread: 0,
        color: POP_SHADOW,
    };
    s.visuals.window_stroke = Stroke::new(1.0_f32, EDGE);
    s.visuals.window_corner_radius = CornerRadius::same(R_CONTROL);
    s.spacing.item_spacing = vec2(SPACE_XS, SPACE_XS);
    s.spacing.button_padding = vec2(8.0, 4.0);
    s.spacing.interact_size.y = CONTROL_H;
    s.visuals.extreme_bg_color = FIELD;
    let w = &mut s.visuals.widgets;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, DIVIDER);
    // At rest a field is a quiet well: no outline. Hover adds a hairline,
    // focus the accent ring (selection.stroke, one point out: expansion 1).
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.weak_bg_fill = CONTROL;
    w.inactive.expansion = 0.0;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, HOVER_EDGE);
    w.hovered.weak_bg_fill = PRESS;
    w.hovered.expansion = 0.0;
    w.active.bg_stroke = Stroke::new(1.0_f32, HOVER_EDGE);
    w.active.weak_bg_fill = THUMB;
    w.active.expansion = FOCUS_GAP;
    w.open.bg_stroke = Stroke::new(1.0_f32, HOVER_EDGE);
    w.open.weak_bg_fill = PRESS;
    for v in [&mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        v.corner_radius = CornerRadius::same(R_ROW);
    }
    s.visuals.selection.stroke = Stroke::new(FOCUS_RING_W, FOCUS_RING);
    s.visuals.selection.bg_fill = ACTIVE;
    s.visuals.text_cursor.stroke = Stroke::new(1.5_f32, BRIGHT);
}

pub mod atoms {
    use super::*;
    /// The visibility eye: open, or closed with a slash.
    pub fn paint_eye(ui: &egui::Ui, r: Rect, on: bool) {
        let c = r.center();
        let col = if on { DIM } else { WARN };
        let pts: Vec<Pos2> = (0..=16)
            .map(|i| {
                let t = std::f32::consts::PI * i as f32 / 16.0;
                pos2(c.x - 6.0 * t.cos(), c.y - 3.6 * t.sin())
            })
            .chain((0..=16).map(|i| {
                let t = std::f32::consts::PI * i as f32 / 16.0;
                pos2(c.x + 6.0 * t.cos(), c.y + 3.6 * t.sin())
            }))
            .collect();
        ui.painter()
            .add(egui::Shape::closed_line(pts, Stroke::new(1.2_f32, col)));
        ui.painter().circle_filled(c, 1.8, col);
        if !on {
            ui.painter().line_segment(
                [c + vec2(-6.0, 5.0), c + vec2(6.0, -5.0)],
                Stroke::new(1.3_f32, col),
            );
        }
    }

    /// The lock: shackle closed when locked, open when not.
    pub fn paint_lock(ui: &egui::Ui, r: Rect, on: bool) {
        let c = r.center();
        let col = if on { INK } else { FAINT };
        let body = Rect::from_center_size(c + vec2(0.0, 2.0), vec2(9.0, 7.0));
        ui.painter().rect_filled(body, CornerRadius::same(2), col);
        let shackle =
            Rect::from_center_size(c + vec2(if on { 0.0 } else { 2.5 }, -2.5), vec2(6.0, 7.0));
        ui.painter().rect_stroke(
            shackle,
            CornerRadius::same(3),
            Stroke::new(1.3_f32, col),
            egui::StrokeKind::Middle,
        );
        ui.painter().rect_filled(body, CornerRadius::same(2), col);
    }

    pub fn dot(painter: &egui::Painter, center: Pos2, color: Color32) {
        painter.circle_filled(center, 3., color);
    }
    pub fn label(ui: &mut egui::Ui, text: &str) -> egui::Response {
        ui.label(egui::RichText::new(text).font(body()).color(INK))
    }
    /// Text centred in a control, snapped to whole points: every kit control
    /// places its label through here so a row of controls shares a baseline.
    pub fn centered(painter: &egui::Painter, rect: Rect, galley: std::sync::Arc<egui::Galley>, ink: Color32) {
        let at = rect.center() - galley.size() / 2.0;
        painter.galley_with_override_text_color(pos2(at.x.round(), at.y.round()), galley, ink);
    }
    /// Text at a control's leading edge, vertically centred like [`centered`].
    pub fn leading(painter: &egui::Painter, rect: Rect, x: f32, galley: std::sync::Arc<egui::Galley>, ink: Color32) {
        let y = rect.center().y - galley.size().y / 2.0;
        painter.galley_with_override_text_color(pos2(x.round(), y.round()), galley, ink);
    }
    /// Keyboard focus: a thin accent ring just outside the control.
    pub fn paint_focus(ui: &egui::Ui, rect: Rect, radius: u8) {
        ui.painter().rect_stroke(
            rect.expand(FOCUS_GAP),
            CornerRadius::same(radius.saturating_add(FOCUS_GAP as u8)),
            Stroke::new(FOCUS_RING_W, FOCUS_RING),
            egui::StrokeKind::Inside,
        );
    }
    /// A picked row: a quiet accent wash and a short accent marker at its
    /// leading edge (the project tree's look).
    pub fn paint_selected_row(ui: &egui::Ui, rect: Rect) {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(R_ROW), ACTIVE);
        ui.painter().rect_filled(
            Rect::from_center_size(
                pos2(rect.min.x + MARKER_W / 2.0, rect.center().y),
                vec2(MARKER_W, 12.0),
            ),
            CornerRadius::same(1),
            ACCENT,
        );
    }
    /// A disclosure chevron: right when closed, down when open.
    pub fn chevron(painter: &egui::Painter, center: Pos2, open: bool, color: Color32) {
        let pts = if open {
            vec![center + vec2(-3.5, -1.75), center + vec2(3.5, -1.75), center + vec2(0.0, 2.25)]
        } else {
            vec![center + vec2(-1.75, -3.5), center + vec2(-1.75, 3.5), center + vec2(2.25, 0.0)]
        };
        painter.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
    }
}
pub use atoms::*;
pub mod molecules {
    use super::*;
    /// The one state painter: every control's fill and line come from here.
    #[derive(Clone, Copy, PartialEq, Debug)]
    pub enum State {
        Idle,
        Hover,
        Press,
        On,
        Focus,
        Off,
    }
    /// Apply the same state painters to a whole specimen without synthesizing input.
    pub(crate) fn specimen(ui: &mut egui::Ui, state: State, draw: impl FnOnce(&mut egui::Ui)) {
        let tag = egui::UiStackInfo::default().with_tag_value("kit-specimen", state);
        ui.scope_builder(egui::UiBuilder::new().ui_stack_info(tag), |ui| {
            specimen_visuals(ui, state);
            draw(ui)
        });
    }
    /// The state a gallery specimen forces, if any.
    pub(crate) fn specimen_state(ui: &egui::Ui) -> Option<State> {
        ui.stack()
            .iter()
            .find_map(|node| node.tags().get_downcast::<State>("kit-specimen").copied())
    }
    /// Show egui's own widgets (fields, numbers, choices) in a forced state by
    /// giving their resting look the state's look.
    fn specimen_visuals(ui: &mut egui::Ui, state: State) {
        let v = ui.visuals().clone();
        let w = &mut ui.visuals_mut().widgets;
        match state {
            State::Hover => w.inactive = v.widgets.hovered,
            State::Press => w.inactive = v.widgets.active,
            State::Focus => {
                w.inactive = v.widgets.active;
                w.inactive.bg_stroke = v.selection.stroke;
            }
            _ => {}
        }
    }
    fn control_state(ui: &egui::Ui, resp: &egui::Response, selected: bool) -> State {
        specimen_state(ui).unwrap_or_else(|| state_of(resp, selected, true))
    }
    pub fn state_of(resp: &egui::Response, on: bool, enabled: bool) -> State {
        if !enabled || !resp.enabled() {
            State::Off
        } else if on {
            State::On
        } else if resp.is_pointer_button_down_on() {
            State::Press
        } else if resp.hovered() {
            State::Hover
        } else if resp.has_focus() {
            State::Focus
        } else {
            State::Idle
        }
    }
    /// Hover and press: neutral washes. Selected: a quiet accent wash, no
    /// outline. Focus: the ring outside the control, no fill.
    pub fn paint_state(ui: &egui::Ui, rect: Rect, st: State, radius: u8) {
        let r = CornerRadius::same(radius);
        match st {
            State::Hover => {
                ui.painter().rect_filled(rect, r, HOVER);
            }
            State::Press => {
                ui.painter().rect_filled(rect, r, PRESS);
            }
            State::Focus => paint_focus(ui, rect, radius),
            State::On => {
                ui.painter().rect_filled(rect, r, ACTIVE);
            }
            _ => {}
        }
    }
    pub fn ink_of(st: State) -> Color32 {
        match st {
            State::Off => OFF,
            State::On => BRIGHT,
            State::Idle => INK_2,
            _ => INK,
        }
    }

    /// A section label: 11 medium, sentence case, DIM. Subordinate to a
    /// title (13 semibold INK) by size, weight and ink.
    pub fn caption(ui: &mut egui::Ui, text: &str) {
        caption_at(ui, text, 8.0)
    }
    /// A caption lined up with the text of the rows under it (8 in the column, 12 in menus).
    pub fn caption_at(ui: &mut egui::Ui, text: &str, pad: f32) {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
        let g = ui.painter().layout_no_wrap(text.to_owned(), caption_font(), DIM);
        leading(ui.painter(), r, r.min.x + pad, g, DIM);
    }
    pub fn caption_plain(ui: &mut egui::Ui, text: &str) {
        ui.label(egui::RichText::new(text).font(caption_font()).color(DIM));
    }

    /// The shared chip geometry: CONTROL_H high, R_ROW corners, 10 pt of
    /// padding each side of the content.
    fn chip_rect(ui: &mut egui::Ui, content_w: f32, sense: Sense) -> (Rect, egui::Response) {
        ui.allocate_exact_size(vec2(content_w + 20.0, CONTROL_H), sense)
    }

    /// A command chip (inspector next steps): the neutral control wash.
    pub fn chip_button(ui: &mut egui::Ui, label: &str) -> bool {
        let g = ui.painter().layout_no_wrap(label.to_owned(), meta(), INK);
        let (rect, resp) = chip_rect(ui, g.size().x, Sense::click());
        let st = control_state(ui, &resp, false);
        let r = CornerRadius::same(R_ROW);
        if st != State::Off {
            ui.painter().rect_filled(rect, r, CONTROL);
        }
        paint_state(ui, rect, st, R_ROW);
        centered(ui.painter(), rect, g, if st == State::Off { OFF } else { INK });
        resp.clicked()
    }

    /// A status chip: the chip geometry, a dot carrying the tone, the label
    /// in secondary ink. Not interactive.
    pub fn status_chip(ui: &mut egui::Ui, label: &str, tone: Color32) {
        let g = ui.painter().layout_no_wrap(label.to_owned(), meta(), INK_2);
        let (rect, _) = chip_rect(ui, g.size().x + 10.0, Sense::hover());
        let off = !ui.is_enabled();
        ui.painter()
            .rect_filled(rect, CornerRadius::same(R_ROW), CONTROL);
        dot(ui.painter(), pos2(rect.min.x + 12.0, rect.center().y), tone);
        leading(ui.painter(), rect, rect.min.x + 20.0, g, if off { OFF } else { INK_2 });
    }

    /// A sheet button, BUTTON_H high: the default in accent, the rest the
    /// neutral control wash; the key that answers it in faint type after the label.
    pub fn sheet_button(ui: &mut egui::Ui, label: &str, hint: &str, role: u8) -> bool {
        let primary = role == 1;
        let font = strong(TYPE_BODY);
        let g = ui.painter().layout_no_wrap(label.to_owned(), font, INK);
        let k = (!hint.is_empty()).then(|| ui.painter().layout_no_wrap(hint.to_owned(), meta(), FAINT));
        let kw = k.as_ref().map_or(0.0, |k| k.size().x + 8.0);
        let (rect, resp) = ui.allocate_exact_size(
            vec2((g.size().x + kw + 24.0).max(72.0), BUTTON_H),
            Sense::click(),
        );
        let state = control_state(ui, &resp, false);
        let off = state == State::Off;
        let ink = if off {
            OFF
        } else if primary {
            ON_PRIMARY
        } else if role == 2 {
            ERROR
        } else {
            INK
        };
        let r = CornerRadius::same(R_CONTROL);
        if primary && !off {
            let fill = match state {
                State::Press => PRIMARY_PRESS,
                State::Hover => PRIMARY_HOVER,
                _ => ACCENT,
            };
            ui.painter().rect_filled(rect, r, fill);
            if state == State::Focus {
                paint_focus(ui, rect, R_CONTROL);
            }
        } else {
            ui.painter().rect_filled(rect, r, CONTROL);
            paint_state(ui, rect, state, R_CONTROL);
        }
        let x0 = (rect.center().x - (g.size().x + kw) / 2.0).round();
        let mut text = ui.painter().clone();
        if off && ui.is_visible() {
            // OFF already carries the disabled contrast.
            text.set_fade_to_color(None);
        }
        leading(&text, rect, x0, g.clone(), ink);
        if let Some(k) = k {
            let hint_ink = if off { OFF } else if primary { ON_PRIMARY_HINT } else { INK_2 };
            leading(&text, rect, x0 + g.size().x + 8.0, k, hint_ink);
        }
        resp.clicked()
    }

    /// A warning chip: the chip geometry, amber text on an amber wash.
    pub fn amber_button(ui: &mut egui::Ui, label: &str) -> bool {
        let g = ui.painter().layout_no_wrap(label.to_owned(), meta(), WARN);
        let (rect, resp) = chip_rect(ui, g.size().x, Sense::click());
        let st = control_state(ui, &resp, false);
        let fill = match st {
            State::Press => WARNING_PRESS,
            State::Hover => WARNING_HOVER,
            State::Off => CONTROL,
            _ => WARN_FILL,
        };
        ui.painter()
            .rect_filled(rect, CornerRadius::same(R_ROW), fill);
        if st == State::Focus {
            paint_focus(ui, rect, R_ROW);
        }
        centered(ui.painter(), rect, g, if st == State::Off { OFF } else { WARN });
        resp.clicked()
    }

    /// A quiet text button: no fill at rest, the hover wash on hover.
    pub fn quiet(ui: &mut egui::Ui, label: &str) -> egui::Response {
        let g = ui.painter().layout_no_wrap(label.to_owned(), body(), INK_2);
        let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 16.0, CONTROL_H), Sense::click());
        let st = control_state(ui, &resp, false);
        paint_state(ui, rect, st, R_ROW);
        centered(ui.painter(), rect, g, ink_of(st));
        resp
    }
    /// A square icon button, CONTROL_H on a side; its name is the tooltip.
    pub fn icon_button(ui: &mut egui::Ui, glyph: &str, name: &str) -> egui::Response {
        let g = ui.painter().layout_no_wrap(glyph.to_owned(), body(), DIM);
        let (rect, resp) = ui.allocate_exact_size(vec2(CONTROL_H, CONTROL_H), Sense::click());
        let st = control_state(ui, &resp, false);
        paint_state(ui, rect, st, R_ROW);
        let ink = match st {
            State::Off => OFF,
            State::Idle => DIM,
            _ => INK,
        };
        centered(ui.painter(), rect, g, ink);
        resp.on_hover_text(name)
    }

    /// A mode or place switch: an inset track, the current segment a raised
    /// neutral thumb. Returns the segment clicked (also when it is the
    /// current one: a click on the lit "Project" while in Problems returns
    /// to the tree). Not to be confused with [`underline_tabs`], which
    /// navigate the sections of one panel.
    pub fn segmented(ui: &mut egui::Ui, labels: &[&str], at: usize, width: f32) -> Option<usize> {
        let (track, control) = ui.allocate_exact_size(vec2(width, CONTROL_H), Sense::hover());
        let forced = specimen_state(ui);
        let off = forced == Some(State::Off) || !ui.is_enabled();
        ui.painter()
            .rect_filled(track, CornerRadius::same(R_ROW), FIELD);
        let n = labels.len().max(1) as f32;
        let seg_w = (width - 4.0) / n;
        let first_other = (0..labels.len()).find(|&i| i != at);
        let mut hit = None;
        for (i, label) in labels.iter().enumerate() {
            let r = Rect::from_min_size(
                pos2(track.min.x + 2.0 + i as f32 * seg_w, track.min.y + 2.0),
                vec2(seg_w, CONTROL_H - 4.0),
            );
            let resp = ui.interact(r, control.id.with(i), Sense::click());
            let on = i == at;
            let st = match forced {
                Some(s) if Some(i) == first_other && s != State::On => s,
                Some(State::Off) => State::Off,
                _ => state_of(&resp, false, !off),
            };
            if on {
                ui.painter()
                    .rect_filled(r, CornerRadius::same(R_INSET), THUMB);
                if st == State::Focus {
                    paint_focus(ui, r, R_INSET);
                }
            } else {
                paint_state(ui, r, st, R_INSET);
            }
            let ink = if off {
                OFF
            } else if on {
                INK
            } else {
                match st {
                    State::Hover | State::Press | State::Focus => INK,
                    _ => DIM,
                }
            };
            let font = if on { strong(TYPE_BODY) } else { body() };
            let g = ui.painter().layout_no_wrap((*label).to_owned(), font, ink);
            centered(ui.painter(), r, g, ink);
            if resp.clicked() && !off {
                hit = Some(i);
            }
        }
        hit
    }

    /// Second-level tabs: text on a hairline, the current one in medium ink
    /// with a 2-pt accent underline across its label. No box in any state:
    /// hover only brightens the ink. (label, count or note, enabled).
    /// Labels shrink to fit the row.
    pub fn underline_tabs(
        ui: &mut egui::Ui,
        tabs: &[(String, String, bool)],
        at: usize,
    ) -> Option<usize> {
        draw_underline_tabs(ui, tabs, at, false)
    }

    /// A sheet header shares the row with verbs and search.
    pub fn inline_underline_tabs(ui: &mut egui::Ui, tabs: &[(String, String, bool)], at: usize) -> Option<usize> {
        draw_underline_tabs(ui, tabs, at, true)
    }

    fn draw_underline_tabs(ui: &mut egui::Ui, tabs: &[(String, String, bool)], at: usize, inline: bool) -> Option<usize> {
        let forced = specimen_state(ui);
        let first_other = (0..tabs.len()).find(|&i| i != at && tabs[i].2);
        // Measure every label in the current tab's weight so the row does not
        // shift when the current tab changes.
        let measure: Vec<f32> = tabs
            .iter()
            .map(|t| ui.painter().layout_no_wrap(t.0.clone(), strong(TYPE_BODY), INK).size().x)
            .collect();
        let counts: Vec<_> = tabs
            .iter()
            .map(|t| (!t.1.is_empty()).then(|| ui.painter().layout_no_wrap(t.1.clone(), meta(), FAINT)))
            .collect();
        let natural: f32 = measure
            .iter()
            .zip(&counts)
            .map(|(l, c)| l + c.as_ref().map_or(0.0, |c| c.size().x + 4.0) + 16.0)
            .sum();
        // Too wide: drop the counts of the tabs that are not current.
        let width = if inline { natural.min(ui.available_width()) } else { ui.available_width() };
        let (row, _) = ui.allocate_exact_size(vec2(width, CONTROL_H), Sense::hover());
        ui.painter().hline(row.x_range(), row.max.y - 0.5, Stroke::new(1.0_f32, DIVIDER));
        let squeeze = natural > width;
        let mut x = row.min.x;
        let mut hit = None;
        for (i, (t, count)) in tabs.iter().zip(counts).enumerate() {
            let on = i == at;
            let show_count = count.filter(|_| on || !squeeze);
            let lw = measure[i];
            let w = lw + show_count.as_ref().map_or(0.0, |c| c.size().x + 4.0) + 16.0;
            let r = Rect::from_min_size(pos2(x, row.min.y), vec2(w, row.height()));
            let sense = if t.2 { Sense::click() } else { Sense::hover() };
            let resp = ui.interact(r, ui.id().with(("utab", i)), sense);
            let st = match forced {
                Some(s) if Some(i) == first_other && s != State::On => s,
                Some(State::Off) => State::Off,
                _ => state_of(&resp, false, t.2),
            };
            let ink = match (st, on) {
                (State::Off, _) => OFF,
                (_, true) => INK,
                (State::Hover | State::Press | State::Focus, _) => INK,
                _ => DIM,
            };
            let font = if on { strong(TYPE_BODY) } else { body() };
            let label = ui.painter().layout_no_wrap(t.0.clone(), font, ink);
            let text = Rect::from_min_size(pos2(r.min.x + 8.0, r.min.y), vec2(lw, r.height() - 2.0));
            leading(ui.painter(), text, text.min.x, label, ink);
            if let Some(c) = show_count {
                leading(ui.painter(), text, text.max.x + 4.0, c, if st == State::Off { OFF } else { FAINT });
            }
            let underline = Rect::from_min_max(pos2(r.min.x + 8.0, r.max.y - 2.0), pos2(r.max.x - 8.0, r.max.y));
            if on {
                ui.painter().rect_filled(underline, CornerRadius::same(1), if st == State::Off { OFF } else { ACCENT });
            } else if st == State::Press {
                ui.painter().rect_filled(underline, CornerRadius::same(1), HOVER_EDGE);
            }
            if st == State::Focus {
                paint_focus(ui, text.expand2(vec2(4.0, -2.0)), R_INSET);
            }
            let resp = if t.2 {
                resp
            } else {
                resp.on_hover_text("Coming")
            };
            if resp.clicked() && t.2 {
                hit = Some(i);
            }
            x += w;
        }
        hit
    }

    /// A tile's surface: the card fill, lighter on hover, the ring on focus.
    /// No outline at rest.
    pub fn paint_card(ui: &egui::Ui, r: Rect, hovered: bool, focused: bool) {
        let st = match (hovered, focused) {
            (_, true) => State::Focus,
            (true, _) => State::Hover,
            _ => State::Idle,
        };
        paint_tile(ui, r, st);
    }
    /// A tile (Library item, start-page file) in any state: selected adds the
    /// quiet accent wash, never an accent fill.
    pub fn paint_tile(ui: &egui::Ui, r: Rect, st: State) {
        let cr = CornerRadius::same(R_CONTROL);
        let fill = match st {
            State::Hover | State::Press => CARD_HOVER,
            _ => CARD,
        };
        ui.painter().rect_filled(r, cr, fill);
        match st {
            State::Press => {
                ui.painter().rect_filled(r, cr, HOVER);
            }
            State::On => {
                ui.painter().rect_filled(r, cr, ACTIVE);
            }
            State::Focus => paint_focus(ui, r, R_CONTROL),
            _ => {}
        }
    }

    pub fn menu_row(ui: &mut egui::Ui, item: &super::super::Item) -> bool {
        menu_row_resp(ui, item).0.clicked()
    }

    /// Keep the command name and shortcut readable as the menu path shrinks.
    pub fn command_row(ui: &mut egui::Ui, label: &str, path: &str, keys: &str,
                       enabled: bool, selected: bool) -> egui::Response {
        let sense = if enabled { Sense::click() } else { Sense::hover() };
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), CONTROL_H), sense);
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let state = if enabled { control_state(ui, &response, selected) } else { State::Off };
        if state == State::On {
            paint_selected_row(ui, rect);
        } else {
            paint_state(ui, rect, state, R_ROW);
        }
        let ink = if state == State::Off { OFF } else { INK };
        let available = (rect.width() - 2.0 * SPACE_M).max(0.0);
        let shortcut = fit(ui, keys, meta(), FAINT, available / 2.0, 1);
        let shortcut_x = rect.right() - SPACE_M - shortcut.size().x;
        let mut x = rect.left() + SPACE_M;
        let room = (shortcut_x - x - SPACE_S).max(0.0);
        let title = fit(ui, label, body(), ink, room, 1);
        let path_room = room - title.size().x - SPACE_S;
        if !path.is_empty() && path_room > SPACE_M {
            let trail = fit(ui, path, meta(), FAINT, path_room, 1);
            let width = trail.size().x;
            leading(ui.painter(), rect, x, trail, FAINT);
            x += width + SPACE_S;
        }
        leading(ui.painter(), rect, x, title, ink);
        leading(ui.painter(), rect, shortcut_x, shortcut, FAINT);
        response
    }

    /// A menu row, ROW_H high: optional check lane (only when the menu has checks:
    /// the caller marks that with item.3 == "check"), label, shortcut or › right.
    pub fn menu_row_resp(ui: &mut egui::Ui, item: &super::super::Item) -> (egui::Response, Rect) {
        let (_, label, tip, lane, checked, enabled) = item;
        let sense = if *enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), sense);
        let st = control_state(ui, &resp, false);
        let st = if *enabled { st } else { State::Off };
        paint_state(ui, rect, st, R_ROW);
        let ink = if st == State::Off { OFF } else { INK };
        let x = if lane == "check" || *checked >= 0 {
            28.0
        } else {
            12.0
        };
        if *checked == 1 {
            let g = ui.painter().layout_no_wrap("✓".into(), body(), ACCENT);
            centered(
                ui.painter(),
                Rect::from_center_size(pos2(rect.min.x + 14.0, rect.center().y), vec2(16.0, rect.height())),
                g,
                if st == State::Off { OFF } else { ACCENT },
            );
        }
        let g = ui.painter().layout_no_wrap(label.clone(), body(), ink);
        leading(ui.painter(), rect, rect.min.x + x, g, ink);
        if !tip.is_empty() {
            let g = ui.painter().layout_no_wrap(tip.clone(), meta(), FAINT);
            let w = g.size().x;
            leading(ui.painter(), rect, rect.max.x - 12.0 - w, g, FAINT);
        }
        (resp, rect)
    }
    /// A hairline between groups of menu rows.
    pub fn menu_separator(ui: &mut egui::Ui) {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
        ui.painter()
            .hline(r.x_range().shrink(8.0), r.center().y, Stroke::new(1.0_f32, DIVIDER));
    }

    /// A single-line text field: CONTROL_H high, a quiet well at rest.
    pub fn field(text: &mut String) -> egui::TextEdit<'_> {
        egui::TextEdit::singleline(text)
            .font(body())
            .margin(egui::Margin::symmetric(8, 4))
            .min_size(vec2(0.0, CONTROL_H))
            .vertical_align(egui::Align::Center)
    }
    pub fn search<'a>(text: &'a mut String, hint: &str) -> egui::TextEdit<'a> {
        field(text).hint_text(egui::RichText::new(hint).color(FAINT))
    }
    pub fn number<'a>(value: &'a mut f64, units: &str) -> egui::DragValue<'a> {
        egui::DragValue::new(value).suffix(units)
    }
    pub fn swatch(ui: &mut egui::Ui, value: &mut Color32) -> egui::Response {
        ui.color_edit_button_srgba(value)
    }
    /// A labelled switch: the label, then a small track; the accent shows
    /// only in the track when on.
    pub fn toggle(ui: &mut egui::Ui, value: &mut bool, label: &str) -> egui::Response {
        let g = ui.painter().layout_no_wrap(label.to_owned(), body(), INK_2);
        let track_w = 26.0;
        let (rect, mut resp) =
            ui.allocate_exact_size(vec2(g.size().x + 12.0 + track_w + 4.0, CONTROL_H), Sense::click());
        if resp.clicked() {
            *value = !*value;
            resp.mark_changed();
        }
        let st = control_state(ui, &resp, false);
        let on = *value || (st == State::On);
        let off = st == State::Off;
        let label_ink = match st {
            State::Off => OFF,
            State::Idle => INK_2,
            _ => INK,
        };
        leading(ui.painter(), rect, rect.min.x + 4.0, g, label_ink);
        let track = Rect::from_min_size(
            pos2(rect.max.x - track_w - 4.0, rect.center().y - 7.0),
            vec2(track_w, 14.0),
        );
        let fill = match (on, off) {
            (true, false) if st == State::Press => PRIMARY_PRESS,
            (true, false) if st == State::Hover => PRIMARY_HOVER,
            (true, false) => ACCENT,
            (false, false) if matches!(st, State::Hover | State::Press) => HOVER_EDGE,
            _ => TRACK,
        };
        ui.painter().rect_filled(track, CornerRadius::same(7), fill);
        let knob_x = if on { track.max.x - 7.0 } else { track.min.x + 7.0 };
        let knob = match (on, off) {
            (_, true) => OFF,
            (true, _) => BRIGHT,
            _ => INK_2,
        };
        ui.painter().circle_filled(pos2(knob_x, track.center().y), 5.0, knob);
        if st == State::Focus {
            paint_focus(ui, track, 7);
        }
        resp
    }
    pub fn choice(ui: &mut egui::Ui, id: impl std::hash::Hash, value: &mut usize, labels: &[&str]) {
        if labels.is_empty() {
            ui.add_enabled(false, egui::Button::new("—"));
            return;
        }
        *value = (*value).min(labels.len() - 1);
        egui::ComboBox::from_id_salt(id).show_index(ui, value, labels.len(), |i| labels[i]);
    }
    /// Keep a drag's value local until release so the edit makes one undo step.
    pub fn percent_slider(ui: &mut egui::Ui, id: egui::Id, committed: f64, max: f64) -> Option<f64> {
        let mut value = ui.data(|d| d.get_temp::<f64>(id)).unwrap_or(committed);
        let r = ui.add(
            egui::Slider::new(&mut value, 0.0..=max)
                .step_by(1.0)
                .suffix("%"),
        );
        if r.is_pointer_button_down_on() {
            ui.data_mut(|d| d.insert_temp(id, value));
        }
        if r.drag_stopped() || (r.changed() && !r.is_pointer_button_down_on()) {
            ui.data_mut(|d| d.remove::<f64>(id));
            if value != committed {
                return Some(value);
            }
        }
        None
    }

    pub fn progress(fraction: f32) -> egui::ProgressBar {
        egui::ProgressBar::new(fraction).fill(INK_2)
    }
    /// Measured progress that fits its container: a visible track the full
    /// available width, the done part in secondary ink.
    pub fn progress_bar(ui: &mut egui::Ui, fraction: f32, height: f32) {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        let cr = CornerRadius::same((height / 2.0).round() as u8);
        ui.painter().rect_filled(r, cr, TRACK);
        let done = Rect::from_min_size(r.min, vec2(r.width() * fraction.clamp(0.0, 1.0), height));
        ui.painter().rect_filled(done, cr, INK_2);
    }
    pub fn ring(ui: &mut egui::Ui) {
        ui.add(egui::Spinner::new().size(14.0).color(INK_2));
    }
    /// Kept for callers: a status chip.
    pub fn badge(ui: &mut egui::Ui, label: &str, tone: Color32) {
        status_chip(ui, label, tone)
    }
    pub fn empty_state(ui: &mut egui::Ui, text: &str) {
        ui.label(egui::RichText::new(text).font(body()).color(FAINT));
    }
    /// A label and its value on one PROP_H row: the label 11 DIM at the
    /// leading edge, the value 12 INK at the trailing edge.
    pub fn prop_row(ui: &mut egui::Ui, label: &str, value: &str) {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), PROP_H), Sense::hover());
        let off = !ui.is_enabled();
        let g = ui.painter().layout_no_wrap(label.to_owned(), meta(), DIM);
        leading(ui.painter(), r, r.min.x + 8.0, g, if off { OFF } else { DIM });
        let mut job = egui::text::LayoutJob::single_section(
            value.to_owned(),
            egui::TextFormat { font_id: body(), color: INK, ..Default::default() },
        );
        job.wrap.max_width = (r.width() * 0.6).max(40.0);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let v = ui.painter().layout_job(job);
        let x = r.max.x - 8.0 - v.size().x;
        leading(ui.painter(), r, x, v, if off { OFF } else { INK });
    }
    /// One editable property, with a shared leading label and control baseline.
    pub fn property_control(
        ui: &mut egui::Ui, label: &str, control: impl FnOnce(&mut egui::Ui),
    ) {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), CONTROL_H), Sense::hover());
        let column = (r.width() * 0.44).min(160.);
        let label_left = r.left() + SPACE_S;
        let text = super::fit(ui, label, meta(), DIM, column - SPACE_S * 2., 1);
        leading(ui.painter(), r, label_left, text, DIM);
        let controls = Rect::from_min_max(pos2(r.left() + column, r.top()), r.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(controls)
            .layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = SPACE_S;
            control(ui);
        });
    }

    /// A collapsible section: the caption with a chevron; its open state is
    /// kept per id. Returns the body's value when open.
    pub fn disclosure<R>(
        ui: &mut egui::Ui,
        id: impl std::hash::Hash,
        title: &str,
        default_open: bool,
        body: impl FnOnce(&mut egui::Ui) -> R,
    ) -> Option<R> {
        let id = ui.make_persistent_id(id);
        let mut open = ui.data_mut(|d| *d.get_persisted_mut_or(id, default_open));
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
        let st = control_state(ui, &resp, false);
        if resp.clicked() {
            open = !open;
            ui.data_mut(|d| d.insert_persisted(id, open));
        }
        let ink = match st {
            State::Off => OFF,
            State::Hover | State::Press | State::Focus => INK,
            _ => DIM,
        };
        if st == State::Focus {
            paint_focus(ui, r, R_ROW);
        }
        chevron(ui.painter(), pos2(r.min.x + 12.0, r.center().y), open, ink);
        let g = ui.painter().layout_no_wrap(title.to_owned(), caption_font(), ink);
        leading(ui.painter(), r, r.min.x + 22.0, g, ink);
        open.then(|| body(ui))
    }
    pub fn list_row(ui: &mut egui::Ui, height: f32, selected: bool) -> (Rect, egui::Response) {
        let (rect, resp) =
            ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
        match control_state(ui, &resp, selected) {
            State::On => paint_selected_row(ui, rect),
            st => paint_state(ui, rect, st, R_ROW),
        }
        (rect, resp)
    }
    pub fn tree_slots(
        ui: &egui::Ui,
        count_right: f32,
        y: f32,
        eye: i8,
        lock: i8,
        show: bool,
    ) -> (Rect, Rect) {
        let slot = |dx| Rect::from_center_size(pos2(count_right - dx, y), vec2(16., 16.));
        let (e, l) = (slot(64.), slot(44.));
        if show && eye >= 0 {
            paint_eye(ui, e, eye == 1);
        }
        if show && lock >= 0 {
            paint_lock(ui, l, lock == 1);
        }
        (e, l)
    }
}
pub use molecules::*;
pub mod organisms {
    use super::*;
    use egui::{Margin, Shadow};
    pub fn path_bar(ui: &mut egui::Ui, path: &[(String, String)], actions: &mut Vec<String>) {
        if path.len() < 2 { return; }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x=SPACE_XS;
            if quiet(ui, "↑").on_hover_text("Up one level").clicked() {
                actions.push(path[path.len()-2].1.clone());
            }
            for (index, (label, action)) in path.iter().enumerate() {
                if index > 0 { ui.label(egui::RichText::new("›").color(FAINT)); }
                if ui.link(label).clicked() { actions.push(action.clone()); }
            }
        });
    }

    /// Selection addresses share a fixed label column; the full path remains
    /// available on hover even in the narrow Filing lens.
    pub fn identity_card(ui: &mut egui::Ui, rows: &[(String, String, String, Option<bool>)], actions: &mut Vec<String>) {
        if rows.is_empty() { return; }
        egui::Frame::NONE.fill(CARD).corner_radius(R_ROW).inner_margin(Margin::same(SPACE_XS as i8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.;
            for (label, value, target, visible) in rows {
                ui.horizontal(|ui| {
                    ui.add_space(SPACE_S);
                    ui.add_sized([62., PROP_H], egui::Label::new(egui::RichText::new(label).font(meta()).color(DIM))
                        .halign(egui::Align::Min));
                    if let Some(on) = visible {
                        let (rect, eye) = ui.allocate_exact_size(vec2(16., PROP_H), Sense::click());
                        paint_eye(ui, rect, *on);
                        ui.add_space(SPACE_XS);
                        if eye.on_hover_text(format!("Show / hide {value}")).clicked() { actions.push(target.clone()); }
                    }
                    let enabled = !target.is_empty();
                    let text = egui::RichText::new(value).font(body()).color(if enabled { ACCENT } else { INK_2 });
                    let response = ui.add(egui::Label::new(text).truncate().sense(if enabled { Sense::click() } else { Sense::hover() }));
                    if response.clicked() { actions.push(target.clone()); }
                    if enabled {
                        let tip = if label == "Tag" { format!("Show / hide {value}") } else { value.clone() };
                        response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip);
                    }
                });
            }
        });
    }

    /// A glass surface with egui content on it: shadow, then the blurred model
    /// through the rounded shape (glass.rs), then the content. Built like an
    /// egui Frame so call sites read the same.
    pub struct Glass {
        surface: Surface,
        margin: Margin,
        radius: u8,
        accent: Option<Color32>,
    }
    pub fn frame() -> Glass {
        glass(Surface::Pill)
    }
    pub fn glass(surface: Surface) -> Glass {
        Glass {
            surface,
            margin: Margin::same(4),
            radius: surface.radius(),
            accent: None,
        }
    }
    impl Glass {
        pub fn inner_margin(mut self, m: Margin) -> Self {
            self.margin = m;
            self
        }
        pub fn corner_radius(mut self, r: CornerRadius) -> Self {
            self.radius = r.nw;
            self
        }
        /// An accent outline (a field being typed into).
        pub fn stroke(mut self, s: Stroke) -> Self {
            if s.color != EDGE {
                self.accent = Some(s.color);
            }
            self
        }
        pub fn show<R>(
            self,
            ui: &mut egui::Ui,
            add: impl FnOnce(&mut egui::Ui) -> R,
        ) -> egui::InnerResponse<R> {
            let shadow_at = ui.painter().add(egui::Shape::Noop);
            let glass_at = ui.painter().add(egui::Shape::Noop);
            let mut f = egui::Frame::new()
                .inner_margin(self.margin)
                .corner_radius(CornerRadius::same(self.radius));
            if let Some(c) = self.accent {
                f = f.stroke(Stroke::new(1.0_f32, c));
            }
            let inner = f.show(ui, add);
            let rect = inner.response.rect;
            let contact = Shadow {
                offset: [0, 1],
                blur: 2,
                spread: 0,
                color: CONTACT,
            };
            let mut shadows = vec![egui::Shape::Rect(
                self.surface
                    .shadow()
                    .as_shape(rect, CornerRadius::same(self.radius)),
            )];
            if self.surface != Surface::Bar {
                shadows.push(egui::Shape::Rect(
                    contact.as_shape(rect, CornerRadius::same(self.radius)),
                ));
            }
            ui.painter().set(shadow_at, egui::Shape::Vec(shadows));
            ui.painter().set(
                glass_at,
                crate::glass::pane(rect, self.radius as f32, self.surface.look()),
            );
            inner
        }
    }

    /// The value field on the geometry (ui/value_field.rs): caption and value
    /// on raised glass. Typing shows an accent edge and a caret; a committed
    /// value reads back with a check until the next action.
    pub fn value_pill(ui: &mut egui::Ui, caption: &str, value: &str, typing: bool, committed: bool) {
        let edge = if typing { ACCENT } else { EDGE };
        glass(Surface::Raised)
            .stroke(Stroke::new(1.0_f32, edge))
            .corner_radius(CornerRadius::same(R_CONTROL))
            .inner_margin(Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(SPACE_S, 0.0);
                    if !caption.is_empty() {
                        ui.label(egui::RichText::new(caption).font(meta()).color(FAINT));
                    }
                    let v = ui.label(egui::RichText::new(value).font(title_font()).color(if typing { BRIGHT } else { INK }));
                    if typing {
                        // A drawn caret: the "▏" glyph read as an "l" after a unit.
                        let x = v.rect.max.x + 2.0;
                        ui.painter().vline(x, v.rect.y_range().shrink(2.0), Stroke::new(1.5_f32, ACCENT));
                        ui.add_space(2.0);
                    }
                    if committed {
                        ui.label(egui::RichText::new("✓").font(meta()).color(OK));
                    }
                });
            });
    }

    /// Regions of a large glass sheet. Callers supply each region's content
    /// and keep ownership of their focus, scrolling and commands.
    pub struct SheetFrame {
        pub header: Rect,
        pub side: Option<Rect>,
        pub content: Rect,
        pub detail: Option<Rect>,
        pub footer: Option<Rect>,
    }

    pub fn sheet_frame(ui: &mut egui::Ui, rect: Rect, side_width: f32, detail_width: f32, footer: bool) -> SheetFrame {
        let glass = Surface::Panel;
        let radius = glass.radius();
        let p = ui.painter();
        p.add(glass.shadow().as_shape(rect, CornerRadius::same(radius)));
        p.add(crate::glass::pane(rect, radius as f32, sheet_glass()));
        let rule = Stroke::new(1.0_f32, DIVIDER);
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), BUTTON_H + 2.0 * SPACE_M));
        p.hline(rect.x_range(), header.bottom() - 0.5, rule);
        let footer = footer.then(|| {
            let top = rect.bottom() - CONTROL_H - 2.0 * SPACE_XS;
            p.hline(rect.x_range(), top + 0.5, rule);
            Rect::from_min_max(pos2(rect.left(), top), rect.max)
        });
        let bottom = footer.map_or(rect.bottom(), |r| r.top());
        let mut content = Rect::from_min_max(pos2(rect.left(), header.bottom()), pos2(rect.right(), bottom));
        let side = (side_width > 0.0).then(|| {
            let side = Rect::from_min_size(content.min, vec2(side_width.min(content.width()), content.height()));
            content.min.x = side.right();
            p.vline(side.right() - 0.5, side.y_range(), rule);
            side
        });
        let detail = (detail_width > 0.0).then(|| {
            let x = content.right() - detail_width.min(content.width() * 0.5);
            let detail = Rect::from_min_max(pos2(x, content.top()), content.max);
            content.max.x = x;
            p.vline(x + 0.5, detail.y_range(), rule);
            detail
        });
        ui.interact(rect, ui.id().with("sheet_body"), Sense::click());
        SheetFrame { header, side, content, detail, footer }
    }
    pub fn tree(ui: &mut egui::Ui, id: impl std::hash::Hash, rows: impl FnOnce(&mut egui::Ui)) {
        egui::ScrollArea::vertical().id_salt(id).show(ui, rows);
    }
    pub fn activity_capsule() -> Glass {
        glass(Surface::Pill).inner_margin(Margin::symmetric(8, 3))
    }
    pub fn popover() -> Glass {
        glass(Surface::Raised)
    }
    /// A menu: the raised glass padded 4, rows edge to edge inside it (their
    /// text at 12, the caption lined up with it).
    pub fn menu<R>(ui: &mut egui::Ui, width: f32, rows: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
        popover().inner_margin(Margin::same(4)).show(ui, |ui| {
            ui.set_width(width - 8.0);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            rows(ui)
        })
    }
    pub fn toast(ui: &mut egui::Ui, opacity: f32, content: impl FnOnce(&mut egui::Ui)) {
        ui.scope(|ui| {
            ui.set_opacity(opacity);
            popover()
                .inner_margin(Margin::symmetric(10, 6))
                .show(ui, content);
        });
    }
    pub fn toast_stack<'a, T: 'a>(
        ui: &mut egui::Ui,
        rows: impl Iterator<Item = (&'a T, f32)>,
        mut render: impl FnMut(&mut egui::Ui, &T),
    ) {
        for (row, opacity) in rows.filter(|(_, a)| *a > 0.).take(3) {
            toast(ui, opacity, |ui| render(ui, row));
        }
    }
}
pub use organisms::*;

/// Shared picker input. Sizes are in the caller's display unit.
#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct FontChoice {
    pub text: String,
    pub font: String,
    pub bold: bool,
    pub italic: bool,
    pub height: f64,
    pub thickness: Option<f64>,
    #[serde(default)]
    pub lengths: Option<crate::font_lengths::FontLengths>,
    pub unit: String,
    pub minimum: f64,
    pub maximum: f64,
    pub title: String,
    pub editable: bool,
    pub recent: Vec<String>,
    pub favourites: Vec<String>,
}

pub struct FontPicker {
    pub choice: FontChoice,
    families: Vec<String>,
    filter: String,
    focus: bool,
    previews: std::collections::HashMap<String, egui::TextureHandle>,
    sample_key: Option<(String, bool, bool, u32, u32)>,
}

impl FontPicker {
    pub fn new(mut choice: FontChoice) -> Self {
        #[cfg(target_os = "macos")]
        let families = super::system_fonts::families();
        #[cfg(not(target_os = "macos"))]
        let families = Vec::new();
        if choice.font.is_empty() {
            choice.font = families
                .iter()
                .find(|s| s.as_str() == "Helvetica")
                .or(families.first())
                .cloned()
                .unwrap_or_default();
        }
        Self {
            choice,
            families,
            filter: String::new(),
            focus: true,
            previews: Default::default(),
            sample_key: None,
        }
    }

    fn matches(&self) -> Vec<String> {
        let query = self.filter.trim().to_lowercase();
        let mut rows: Vec<_> = self
            .families
            .iter()
            .filter(|s| s.to_lowercase().contains(&query))
            .cloned()
            .collect();
        rows.sort_by_key(|s| {
            (
                self.choice
                    .recent
                    .iter()
                    .position(|r| r == s)
                    .unwrap_or(usize::MAX),
                !self.choice.favourites.contains(s),
            )
        });
        rows
    }

    pub fn type_filter(&mut self, text: &str) {
        self.filter.push_str(text);
        let rows = self.matches();
        if !rows.contains(&self.choice.font) {
            self.choice.font = rows.first().cloned().unwrap_or_default();
        }
        self.focus = true;
    }

    fn toggle_star(&mut self, name: &str) {
        if self.choice.favourites.iter().any(|s| s == name) {
            self.choice.favourites.retain(|s| s != name);
        } else {
            self.choice.favourites.push(name.into());
        }
    }

    fn remember(&mut self) {
        self.choice.recent.retain(|s| s != &self.choice.font);
        self.choice.recent.insert(0, self.choice.font.clone());
        self.choice.recent.truncate(10);
    }

    fn step(&mut self, rows: &[String], down: bool) -> Option<usize> {
        if rows.is_empty() {
            return None;
        }
        let at = rows.iter().position(|r| r == &self.choice.font);
        let next = at.map_or(0, |i| {
            if down {
                (i + 1).min(rows.len() - 1)
            } else {
                i.saturating_sub(1)
            }
        });
        self.choice.font = rows[next].clone();
        Some(next)
    }

    pub fn popup(&mut self, ctx: &egui::Context) -> Option<bool> {
        let id = egui::Id::new("font_picker");
        let area = egui::Modal::default_area(id).anchor(
            egui::Align2::CENTER_BOTTOM,
            vec2(0., -super::BOT_H - SPACE_S),
        );
        let modal = egui::Modal::new(id)
            .area(area)
            .frame(egui::Frame::NONE)
            .backdrop_color(CLEAR)
            .show(ctx, |ui| {
                let mut answer = None;
                glass(Surface::Raised)
                    .inner_margin(egui::Margin::same(SPACE_S as i8))
                    .show(ui, |ui| {
                        ui.set_width(320.);
                        answer = self.show(ui);
                    });
                answer
            });
        modal
            .inner
            .or_else(|| modal.backdrop_response.clicked().then_some(false))
    }

    /// Some(true) applies and Some(false) cancels the draft.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<bool> {
        use egui::{Key, Modifiers};
        ui.label(egui::RichText::new(&self.choice.title).font(title_font()));
        if self.choice.editable {
            ui.horizontal(|ui| {
                ui.label("Text");
                ui.add(field(&mut self.choice.text).desired_width(ui.available_width()));
            });
        }
        let search =
            ui.add(search(&mut self.filter, "Search fonts").desired_width(ui.available_width()));
        if self.focus {
            search.request_focus();
            self.focus = false;
        }
        let rows = self.matches();
        if search.changed() && !rows.contains(&self.choice.font) {
            self.choice.font = rows.first().cloned().unwrap_or_default();
        }
        let mut offset = None;
        let row_height = CONTROL_H + ui.spacing().item_spacing.y;
        for (key, down) in [(Key::ArrowUp, false), (Key::ArrowDown, true)] {
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, key)) {
                offset = self.step(&rows, down).map(|idx| row_height * idx as f32);
            }
        }
        let mut list = egui::ScrollArea::vertical().max_height(row_height * 5.);
        if let Some(y) = offset {
            list = list.vertical_scroll_offset(y);
        }
        if rows.is_empty() {
            ui.label("No matching families");
        }
        let scale = ui.ctx().pixels_per_point();
        let key = (
            self.choice.text.clone(),
            self.choice.bold,
            self.choice.italic,
            scale.to_bits(),
            ui.available_width().to_bits(),
        );
        if self.sample_key.as_ref() != Some(&key) {
            self.previews.clear();
            self.sample_key = Some(key);
        }
        list.show_rows(ui, CONTROL_H, rows.len(), |ui, range| {
            self.previews
                .retain(|name, _| rows[range.clone()].contains(name));
            for name in &rows[range] {
                ui.horizontal(|ui| {
                    let star = if self.choice.favourites.contains(name) {
                        "★"
                    } else {
                        "☆"
                    };
                    if quiet(ui, star).on_hover_text("Favourite font").clicked() {
                        self.toggle_star(name);
                    }
                    let selected = name == &self.choice.font;
                    let (rect, response) = list_row(ui, CONTROL_H, selected);
                    let left = rect.left_center() + vec2(SPACE_S, 0.);
                    let mid = left + vec2(rect.width() * 0.48, 0.);
                    ui.painter()
                        .with_clip_rect(rect.with_max_x(mid.x - SPACE_XS))
                        .text(left, egui::Align2::LEFT_CENTER, name, body(), INK);
                    let sample = if self.choice.text.is_empty() {
                        name
                    } else {
                        &self.choice.text
                    };
                    let bounds = Rect::from_min_max(pos2(mid.x, rect.top()), rect.max);
                    #[cfg(target_os = "macos")]
                    if !self.previews.contains_key(name) {
                        if let Some(image) = super::system_fonts::sample(
                            name,
                            sample,
                            self.choice.bold,
                            self.choice.italic,
                            TYPE_BODY,
                            scale,
                            bounds.size(),
                        ) {
                            let texture =
                                ui.ctx()
                                    .load_texture(name, image, egui::TextureOptions::LINEAR);
                            self.previews.insert(name.clone(), texture);
                        }
                    }
                    if let Some(texture) = self.previews.get(name) {
                        ui.painter().image(
                            texture.id(),
                            bounds,
                            Rect::from_min_max(Pos2::ZERO, pos2(1., 1.)),
                            INK_2,
                        );
                    }
                    if response.clicked() {
                        self.choice.font = name.clone();
                    }
                });
            }
        });
        ui.horizontal(|ui| {
            toggle(ui, &mut self.choice.bold, "Bold");
            toggle(ui, &mut self.choice.italic, "Italic");
            ui.label("Size");
            if let Some(lengths) = &mut self.choice.lengths {
                lengths.edit(0, ui, &mut self.choice.height, self.choice.minimum, self.choice.maximum);
            } else {
                ui.add(number(&mut self.choice.height, &self.choice.unit).range(self.choice.minimum..=self.choice.maximum));
            }
        });
        if let Some(depth) = &mut self.choice.thickness {
            ui.horizontal(|ui| {
                ui.label("Extrusion");
                if let Some(lengths) = &mut self.choice.lengths {
                    lengths.edit(1, ui, depth, 0., self.choice.maximum);
                } else {
                    ui.add(number(depth, &self.choice.unit).range(0.0..=self.choice.maximum));
                }
            });
        }
        let length_error = self.choice.lengths.as_ref().and_then(|draft| draft.error());
        if let Some(message) = length_error {
            caption_plain(ui, message);
        }
        let lengths_valid = length_error.is_none();
        let mut answer = None;
        ui.horizontal(|ui| {
            if quiet(ui, "Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                answer = Some(false);
            }
            let valid = lengths_valid && rows.contains(&self.choice.font)
                && (!self.choice.editable || !self.choice.text.trim().is_empty());
            ui.add_enabled_ui(valid, |ui| {
                if sheet_button(ui, "Apply", "↩", 1) || valid && ui.input(|i| i.key_pressed(Key::Enter)) {
                    self.remember();
                    answer = Some(true);
                }
            });
        });
        answer
    }
}

/// One run of formatted prose. Link ids are verbs owned by the sheet.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ProseSpan {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub heading: bool,
    pub code: bool,
    pub link: String,
}

pub fn prose(ui: &mut egui::Ui, spans: &[ProseSpan]) -> Option<String> {
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.;
        for span in spans {
            let font = if span.heading { heading() } else if span.bold { strong(TYPE_BODY) }
                else if span.code { egui::FontId::monospace(TYPE_BODY) } else { body() };
            let mut text = egui::RichText::new(&span.text).font(font).color(INK_2);
            if span.italic { text = text.italics(); }
            if span.link.is_empty() {
                ui.add(egui::Label::new(text).wrap());
            } else if ui.add(egui::Label::new(text.underline().color(ACCENT)).wrap()
                .sense(egui::Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                clicked = Some(span.link.clone());
            }
        }
    });
    clicked
}


#[derive(serde::Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct CreditRow {
    pub name: String,
    pub role: String,
    pub verb: String,
    pub starred: bool,
}

#[derive(Clone, Default)]
struct CreditsRoll {
    offset: f32,
}

impl CreditsRoll {
    fn advance(&mut self, seconds: f32, hovered: bool, wheel: f32, span: f32) {
        if span > 0. {
            let movement = if hovered { -wheel } else { seconds / 0.045 };
            self.offset = (self.offset + movement).rem_euclid(span);
        }
    }
}

/// Loop the credits in a clipped band. Hovering pauses the roll and gives
/// the wheel to the band until the pointer leaves it.
pub fn credits_roll(ui: &mut egui::Ui, id: &str, rows: &[CreditRow], height: f32) -> Option<String> {
    let (band, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());
    if rows.is_empty() || !ui.is_rect_visible(band) { return None; }
    let painter = ui.painter().with_clip_rect(band.intersect(ui.clip_rect()));
    let mut span = 0.;
    let layouts: Vec<_> = rows.iter().map(|row| {
        let offset = span;
        let name = if row.starred { format!("★ {}", row.name) } else { row.name.clone() };
        let name = painter.layout(name, strong(TYPE_BODY), if row.verb.is_empty() { INK } else { ACCENT }, band.width());
        let role = painter.layout(row.role.clone(), body(), INK_2, band.width());
        span += name.size().y + role.size().y + SPACE_M;
        (row, offset, name, role)
    }).collect();
    let key = ui.make_persistent_id(("credits", id));
    let hovered = ui.rect_contains_pointer(band);
    let (dt,wheel) = ui.input_mut(|i| (i.stable_dt.min(0.1), if hovered { std::mem::take(&mut i.smooth_scroll_delta.y) } else { 0. }));
    let offset = ui.data_mut(|data| {
        let roll = data.get_temp_mut_or_default::<CreditsRoll>(key);
        if span > height { roll.advance(dt,hovered,wheel,span); }
        roll.offset
    });
    let mut clicked = None;
    let copies = if span > height { 2 } else { 1 };
    for copy in 0..copies {
        for (row, y, name, role) in &layouts {
            let pos = band.left_top() + egui::vec2(0., y - offset + copy as f32 * span);
            let name_box = egui::Rect::from_min_size(pos, name.size());
            let role_box = egui::Rect::from_min_size(name_box.left_bottom(), role.size());
            if !band.intersects(name_box.union(role_box)) {continue;}
            painter.galley(pos,name.clone(),INK);
            painter.galley(role_box.min,role.clone(),INK_2);
            if !row.verb.is_empty() && band.intersects(name_box) {
                let hit = ui.interact(name_box.intersect(band), key.with((&row.verb,copy)), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
                hit.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Link,true,&row.name));
                if hit.hovered() || hit.has_focus() { painter.line_segment([name_box.left_bottom(), name_box.right_bottom()],egui::Stroke::new(1.,ACCENT)); }
                if hit.clicked() {clicked=Some(row.verb.clone());}
            }
        }
    }
    if !hovered && span > height {ui.ctx().request_repaint_after(std::time::Duration::from_millis(45));}
    clicked
}

#[cfg(test)]
mod credits_tests {
    use super::CreditsRoll;

    #[test]
    fn automatic_credits_move_and_wrap_in_the_band() {
        let mut roll = CreditsRoll::default();
        roll.advance(0.045,false,0.,5.);
        assert_eq!(roll.offset,1.);
        for _ in 0..5 {roll.advance(0.045,false,0.,5.);}
        assert_eq!(roll.offset,1.);
    }
    #[test]
    fn hover_pauses_and_allows_scrolling_in_both_directions() {
        let mut roll = CreditsRoll::default();
        roll.advance(1.,true,0.,100.);
        assert_eq!(roll.offset,0.);
        roll.advance(1.,true,-24.,100.);
        assert_eq!(roll.offset,24.);
        roll.advance(1.,true,24.,100.);
        assert_eq!(roll.offset,0.);
        roll.advance(0.045,false,0.,100.);
        assert_eq!(roll.offset,1.);
    }
}

#[cfg(test)]
mod font_picker_tests {
    use super::*;
    #[test]
    fn filter_navigation_favourites_and_recent_order() {
        let mut picker = FontPicker::new(FontChoice {
            title: "Font".into(),
            text: "A".into(),
            font: "C".into(),
            bold: false,
            italic: false,
            height: 9.,
            minimum: 6.,
            maximum: 28.,
            thickness: None,
            lengths: None,
            unit: " pt".into(),
            editable: false,
            recent: vec!["C".into()],
            favourites: vec!["B".into()],
        });
        picker.families = ["A", "B", "C"].map(str::to_owned).into();
        assert_eq!(picker.matches(), ["C", "B", "A"]);
        picker.step(&picker.matches(), true);
        assert_eq!(picker.choice.font, "B");
        picker.remember();
        assert_eq!(picker.matches(), ["B", "C", "A"]);
        picker.step(&picker.matches(), false);
        assert_eq!(picker.choice.font, "B");
        picker.toggle_star("B");
        assert!(picker.choice.favourites.is_empty());
        picker.filter = " a ".into();
        assert_eq!(picker.matches(), ["A"]);
        picker.filter = "unknown".into();
        assert_eq!(picker.step(&picker.matches(), true), None);
    }
}

/// A responsive strip of 16:10 catalogue pictures. The last incomplete row
/// keeps the same card width as the full rows above it.
pub fn catalogue_grid(width: f32, ideal: f32) -> (usize, egui::Vec2) {
    let columns = (((width + SPACE_M) / (ideal + SPACE_M)).floor() as usize).max(1);
    let card = ((width - (columns - 1) as f32 * SPACE_M) / columns as f32).max(1.);
    (columns, vec2(card, card * 10. / 16.))
}

fn svg_guide_divisions(guide: &str) -> &'static [f32] {
    match guide {
        "Thirds" => &[1. / 3., 2. / 3.],
        "Golden section" => &[0.382, 0.618],
        "Grid" => &[0.25, 0.5, 0.75],
        _ => &[],
    }
}

#[cfg(test)]
mod svg_guides {
    use super::svg_guide_divisions;

    #[test]
    fn thirds() { assert_eq!(svg_guide_divisions("Thirds"), &[1./3., 2./3.]); }
    #[test]
    fn golden() { assert_eq!(svg_guide_divisions("Golden section"), &[0.382, 0.618]); }
    #[test]
    fn grid() { assert_eq!(svg_guide_divisions("Grid"), &[0.25, 0.5, 0.75]); }
}

/// Fit the entire exported page in its well. Guides are chrome overlays.
pub fn svg_preview(ui: &mut egui::Ui, texture: Option<&egui::TextureHandle>, guide: &str,
                   size: egui::Vec2) -> Option<(f32, f32, f32)> {
    let (well, response) = ui.allocate_exact_size(size, Sense::drag());
    ui.painter().rect_filled(well, R_CONTROL, WELL);
    let available = well.shrink(SPACE_S).size().max(vec2(1., 1.));
    let pixels = texture.map_or(vec2(297., 210.), |tex| tex.size_vec2());
    let page = Rect::from_center_size(well.center(), pixels * (available / pixels).min_elem());
    ui.painter().rect_filled(page, 0, BRIGHT);
    if let Some(tex) = texture {
        egui::Image::new(tex).paint_at(ui, page);
    }
    let painter = ui.painter_at(page);
    let line = |a, b| {
        let points = [page.lerp_inside(a), page.lerp_inside(b)];
        painter.line_segment(points, Stroke::new(2., WELL.gamma_multiply(0.55)));
        painter.line_segment(points, Stroke::new(1., BRIGHT.gamma_multiply(0.8)));
    };
    for &p in svg_guide_divisions(guide) {
        line(vec2(0., p), vec2(1., p));
        line(vec2(p, 0.), vec2(p, 1.));
    }
    if guide == "Diagonals" {
        line(vec2(0., 0.), vec2(1., 1.));
        line(vec2(1., 0.), vec2(0., 1.));
    }
    if guide == "Centre" {
        let side = page.size().min_elem();
        let arm = vec2(0.12 * side, 0.12 * side) / page.size();
        let center = vec2(0.5, 0.5);
        line(center - vec2(arm.x, 0.), center + vec2(arm.x, 0.));
        line(center - vec2(0., arm.y), center + vec2(0., arm.y));
        for pen in [Stroke::new(2., WELL.gamma_multiply(0.55)), Stroke::new(1., BRIGHT.gamma_multiply(0.8))] {
            painter.circle_stroke(page.center(), side * 0.04, pen);
        }
    }
    let drag = response.drag_delta() / page.size();
    let wheel = if response.hovered() {
        ui.input_mut(|i| std::mem::take(&mut i.smooth_scroll_delta.y))
    } else { 0. };
    (drag != egui::Vec2::ZERO || wheel != 0.).then(|| (drag.x, drag.y, (wheel / 300.).exp()))
}

/// A catalogue fact may contain an entire licence or a long local path.
pub fn catalogue_fact(ui: &mut egui::Ui, label: &str, value: &str) -> Rect {
    let w = ui.available_width();
    let key_width = (w * 0.34).max(1.);
    let key = ui.painter().layout(label.to_owned(), meta(), DIM, key_width);
    let val = ui.painter().layout(value.to_owned(), body(), INK, (w - key_width - SPACE_S).max(1.));
    let height = key.size().y.max(val.size().y).max(PROP_H);
    let (r, _) = ui.allocate_exact_size(vec2(w, height), Sense::hover());
    ui.painter().galley(r.min, key, DIM);
    ui.painter().galley(r.min + vec2(key_width + SPACE_S, 0.), val, INK);
    r
}

/// Lay out a tag against the full pane before putting it into a wrapping row.
/// A short tag therefore moves intact to the next line.
pub fn catalogue_tag(ui: &mut egui::Ui, label: &str, pane_width: f32) -> Rect {
    let galley = ui.painter().layout(label.to_owned(), meta(), INK_2, (pane_width - 2. * SPACE_S).max(1.));
    let (r, _) = ui.allocate_exact_size(galley.size() + vec2(2. * SPACE_S, SPACE_XS), Sense::hover());
    ui.painter().rect_filled(r, R_ROW, CONTROL);
    ui.painter().galley(r.min + vec2(SPACE_S, SPACE_XS / 2.), galley, INK_2);
    r
}

#[cfg(test)]
mod catalogue_tests {
    use super::*;
    fn draw(f: impl Fn(&mut egui::Ui)) {
        let ctx = egui::Context::default();
        let mut fonts = egui::FontDefinitions::default();
        let data = include_bytes!("../../../../resources/fonts/inter/Inter-Regular.ttf");
        fonts.font_data.insert("inter".into(), std::sync::Arc::new(egui::FontData::from_static(data)));
        fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0,"inter".into());
        ctx.set_fonts(fonts);
        ctx.style_mut(install_style);
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let area=Rect::from_min_size(ui.cursor().min,vec2(200.,600.));
                ui.scope_builder(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Min)),|ui| f(ui));
            });
        });
    }

    #[test]
    fn catalogue_grid_tracks_pane_width() {
        for (w, n) in [(160.,1),(380.,2),(580.,3)] {
            let (cols, tile)=catalogue_grid(w,176.);
            assert_eq!(cols,n);
            assert!((tile.x*n as f32 + SPACE_M*(n-1) as f32 - w).abs()<0.001);
        }
    }
    #[test]
    fn two_catalogue_columns_have_sixteen_by_ten_previews() {
        let (cols,tile)=catalogue_grid(480.,176.);
        assert_eq!(cols,2);
        assert_eq!(tile.x*10.,tile.y*16.);
    }
    #[test]
    fn long_authors_and_licences_wrap_inside_the_detail_pane() {
        draw(|ui| {
            let short = catalogue_fact(ui,"Licence","MIT");
            for label in ["Author","License"] {
                let rect = catalogue_fact(ui,label,&"Long catalogue metadata ".repeat(8));
                assert!(rect.height()>short.height() && rect.width()<=201., "{short:?} {rect:?}");
            }
        });
    }
    #[test]
    fn only_tags_wider_than_the_pane_wrap() {
        draw(|ui| {
            let short=catalogue_tag(ui,"data import export",200.);
            let long=catalogue_tag(ui,&"very long tag ".repeat(10),200.);
            assert!(short.height()<CONTROL_H && long.height()>short.height()*2.);
            assert!(long.width()<=201.);
        });
    }
    #[test]
    fn catalogue_cards_follow_the_dark_chrome_palette() {
        draw(|ui| {
            assert!(ui.visuals().dark_mode);
            assert_eq!(ui.visuals().panel_fill,PANEL);
            let rect=catalogue_tag(ui,"Drawing",200.);
            let shapes=ui.ctx().graphics(|g|g.get(ui.layer_id()).unwrap().all_entries().cloned().collect::<Vec<_>>());
            assert!(shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.rect==rect && r.fill==CONTROL)));
        });
    }
}

pub fn site_pin(painter:&egui::Painter, center:Pos2, label:&str) {
    for (width,ink) in [(4.,FIELD),(MARKER_W,ACCENT)] {
        painter.line_segment([center-vec2(SPACE_M,0.),center+vec2(SPACE_M,0.)],Stroke::new(width,ink));
        painter.line_segment([center-vec2(0.,SPACE_M),center+vec2(0.,SPACE_M)],Stroke::new(width,ink));
    }
    let text=painter.layout_no_wrap(label.to_owned(),meta(),INK);
    let pos=center+vec2(SPACE_M,SPACE_XS);
    painter.rect_filled(Rect::from_min_size(pos,text.size()).expand(SPACE_XS),R_INSET,PANEL);
    painter.galley(pos,text,INK);
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    use egui::{Event, PointerButton};

    #[test]
    fn material_base_and_surface_switches_receive_independent_clicks() {
        let ctx = egui::Context::default();
        let mut fonts = egui::FontDefinitions::empty();
        fonts.font_data.insert(
            "Inter".into(),
            egui::FontData::from_static(include_bytes!(
                "../../../../resources/fonts/inter/Inter-Regular.ttf"
            ))
            .into(),
        );
        for family in [
            egui::FontFamily::Proportional,
            egui::FontFamily::Name("medium".into()),
        ] {
            fonts.families.insert(family, vec!["Inter".into()]);
        }
        ctx.set_fonts(fonts);
        let frame = |events| {
            let mut hit = [None, None];
            let input = egui::RawInput {
                events,
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(320., 180.))),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    hit[0] = segmented(ui, &["Colour", "Image"], 0, 180.);
                    hit[1] = segmented(ui, &["Off", "On"], 1, 180.);
                });
            });
            hit
        };
        frame(vec![]);
        for (pos, expected) in [
            (pos2(145., 18.), [Some(1), None]),
            (pos2(36., 18. + super::CONTROL_H + 3.), [None, Some(0)]),
        ] {
            for pressed in [true, false] {
                let hit = frame(vec![
                    Event::PointerMoved(pos),
                    Event::PointerButton {
                        pos,
                        pressed,
                        button: PointerButton::Primary,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
                if !pressed {
                    assert_eq!(hit, expected);
                }
            }
        }
    }

    #[test]
    fn dragging_a_material_slider_commits_once_on_release() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("opacity-test");
        let mut time = 0.0;
        let mut frame = |events| {
            time += 0.1;
            let mut edit = None;
            let input = egui::RawInput {
                time: Some(time),
                events,
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 100.0))),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.spacing_mut().slider_width = 140.0;
                    edit = percent_slider(ui, id, 10.0, 100.0);
                });
            });
            edit
        };
        let button = |x, pressed| Event::PointerButton {
            pos: pos2(x, 17.0),
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        assert_eq!(frame(vec![]), None);
        assert_eq!(
            frame(vec![
                Event::PointerMoved(pos2(30.0, 17.0)),
                button(30.0, true)
            ]),
            None
        );
        for x in [65.0, 90.0, 110.0] {
            assert_eq!(frame(vec![Event::PointerMoved(pos2(x, 17.0))]), None);
        }
        let draft = ctx.data(|d| d.get_temp::<f64>(id)).unwrap();
        assert!(draft > 60.0);
        assert_eq!(frame(vec![button(110.0, false)]), Some(draft));
        assert_eq!(ctx.data(|d| d.get_temp::<f64>(id)), None);
        assert_eq!(frame(vec![]), None);
    }
}

/// One line (or `rows` lines) of text cut to `width`.
pub(super) fn fit(ui: &egui::Ui, text: &str, font: egui::FontId, color: Color32, width: f32, rows: usize) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), egui::TextFormat { font_id: font, color, ..Default::default() });
    job.wrap.max_width = width.max(10.0);
    job.wrap.max_rows = rows;
    job.wrap.break_anywhere = rows == 1;
    ui.painter().layout_job(job)
}

/// Native samples of the standard cut patterns. The geometry is an 8 × 3 mm
/// window of core.hatches.hatch_marks, clipped and rounded to 0.01 paper mm.
pub const HATCH_KEYS: &str = include_str!("assets/hatches.txt");

mod hatch;
pub(super) use hatch::{hatch_override, PaperMarks};

pub fn hatch_sample(ui: &egui::Ui, rect: Rect, key: &str, custom: Option<&PaperMarks>) {
    let painter = ui.painter();
    painter.rect_filled(rect, R_INSET, WELL);
    let paper = rect.shrink(1.);
    if key == "solid" {
        painter.rect_filled(paper, R_INSET, INK_2);
        return;
    }
    let Some(marks) = custom.or_else(|| hatch::standard(key)) else { return };
    let scale = (paper.width() / 8.).max(paper.height() / 3.);
    let origin = paper.center() - vec2(4., 1.5) * scale;
    let pos = |x, y| origin + vec2(x, y) * scale;
    let painter = painter.with_clip_rect(painter.clip_rect().intersect(paper));
    let stroke = Stroke::new((0.16 * scale).max(1. / ui.ctx().pixels_per_point()), INK_2);
    for &[x, y, u, v] in &marks.lines {
        painter.line_segment([pos(x, y), pos(u, v)], stroke);
    }
    for &[x, y] in &marks.dots {
        painter.circle_filled(pos(x, y), stroke.width * 0.9, INK_2);
    }
    for &[x, y, r] in &marks.circles {
        painter.circle_stroke(pos(x, y), r * scale, stroke);
    }
}

pub fn line_sample(ui: &egui::Ui, rect: Rect, mm: &str, pattern: &str, color: Color32) {
    let width = (mm.parse::<f32>().unwrap_or(0.18)*3.8).clamp(0.7, 5.);
    let dash: &[f32] = match pattern {
        "dotted" => &[1.,3.], "dashed_short" => &[3.,2.], "dashed_long" => &[12.,4.],
        "dashed" => &[7.,3.], "dashdot" | "center" => &[9.,3.,1.,3.],
        "dashdotdot" | "phantom" | "border" => &[8.,2.,1.,2.,1.,2.], _ => &[],
    };
    let y=rect.center().y;
    let stroke=Stroke::new(width,color);
    if dash.is_empty() { ui.painter().hline(rect.x_range(),y,stroke); return; }
    let mut x=rect.left();
    for run in dash.chunks_exact(2).cycle() {
        ui.painter().hline(x..=(x+run[0]).min(rect.right()),y,stroke);
        x+=run[0]+run[1];
        if x>=rect.right() { break; }
    }
}

/// Host tag overrides belong to the host tab, including while its source is edited.
pub fn host_visibility(ui: &mut egui::Ui, host: &str, tags: &[(String, String, bool)], actions: &mut Vec<String>) {
    if host.is_empty() { return; }
    let popup_id = ui.make_persistent_id(("host visibility", host));
    let button = quiet(ui, "Host visibility ▾");
    if button.clicked() { ui.memory_mut(|m| m.toggle_popup(popup_id)); }
    if !ui.memory(|m| m.is_popup_open(popup_id)) { return; }
    // The dock is itself foreground. Keep its rows below the popup on every frame.
    if ui.layer_id().order == egui::Order::Foreground {
        ui.ctx().set_sublayer(ui.layer_id(), egui::LayerId::new(egui::Order::Foreground, popup_id));
    }
    let content_width = ui.available_width() - ui.spacing().menu_margin.sum().x;
    egui::popup_below_widget(ui, popup_id, &button, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_width(content_width);
        caption(ui, host);
        let id = ui.make_persistent_id(("host tag search", host));
        let mut filter = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_default();
        ui.add(search(&mut filter, "Filter host tags"));
        ui.data_mut(|d| d.insert_temp(id, filter.clone()));
        let word = filter.to_lowercase();
        egui::ScrollArea::vertical().max_height(280.).show(ui, |ui| {
            for (key,label,visible) in tags {
                if !label.to_lowercase().contains(&word) { continue; }
                let (rect,resp) = ui.allocate_exact_size(vec2(ui.available_width(),ROW_H), Sense::click());
                paint_state(ui,rect,state_of(&resp,false,true),R_ROW);
                paint_eye(ui,Rect::from_center_size(pos2(rect.left()+12.,rect.center().y),vec2(16.,16.)),*visible);
                ui.painter().text(rect.left_center()+vec2(26.,0.),egui::Align2::LEFT_CENTER,label,body(),INK);
                if resp.clicked() { actions.push(format!("hosteye\u{1f}{key}\u{1f}{}",u8::from(!visible))); }
            }
        });
        if quiet(ui,"Done").clicked() { ui.memory_mut(|m| m.close_popup()); }
    });
}

pub fn tag_inspector(ui:&mut egui::Ui, key:&str, name:&str, count:u32, cells:&[String], actions:&mut Vec<String>) -> Option<Pos2> {
    ui.strong(name);
    caption_plain(ui,&format!("Tag · {count} items"));
    let mut menu=None;
    ui.horizontal_wrapped(|ui|{
        ui.spacing_mut().item_spacing.x=SPACE_XS;
        for (verb,label) in [("tagselect","Select items"),("tagisolate","Isolate")] {
            if quiet(ui,label).clicked(){actions.push(format!("{verb}\x1f{key}"));}
        }
        if key!="preview-tag:Layer 0" {
            let button=quiet(ui,"Give it a kind…");
            if button.clicked(){menu=Some(button.rect.left_bottom());}
        }
    });
    if cells.len()>=5 {
        prop_row(ui,"Line",&format!("{} · {}",cells[1],if cells[2]=="auto" {"Automatic".into()}else{format!("{} mm",cells[2])}));
        prop_row(ui,"Colour",if cells[0].is_empty(){"By material"}else{&cells[0]});
    }
    menu
}
