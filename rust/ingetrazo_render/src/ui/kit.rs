//! Shared egui controls for Built.
//!
//! Every inline control is `CONTROL_H` high and centres its text with
//! [`centered`], so a row of them shares one baseline. State looks come from
//! [`paint_state`]: hover and press are neutral washes, selection is a quiet
//! accent wash (rows add a leading marker), focus is a thin ring outside the
//! control and never a fill.
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
    s.spacing.item_spacing = vec2(2.0, 2.0);
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
        painter.galley(pos2(at.x.round(), at.y.round()), galley, ink);
    }
    /// Text at a control's leading edge, vertically centred like [`centered`].
    pub fn leading(painter: &egui::Painter, rect: Rect, x: f32, galley: std::sync::Arc<egui::Galley>, ink: Color32) {
        let y = rect.center().y - galley.size().y / 2.0;
        painter.galley(pos2(x.round(), y.round()), galley, ink);
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
            BRIGHT
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
        leading(ui.painter(), rect, x0, g.clone(), ink);
        if let Some(k) = k {
            let hint_ink = if off { OFF } else if primary { ON_PRIMARY_HINT } else { FAINT };
            leading(ui.painter(), rect, x0 + g.size().x + 8.0, k, hint_ink);
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
        let (track, _) = ui.allocate_exact_size(vec2(width, CONTROL_H), Sense::hover());
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
            let resp = ui.interact(r, ui.id().with(("seg", i)), Sense::click());
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
        let width = ui.available_width();
        let (row, _) = ui.allocate_exact_size(vec2(width, CONTROL_H), Sense::hover());
        ui.painter()
            .hline(row.x_range(), row.max.y - 0.5, Stroke::new(1.0_f32, DIVIDER));
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

    pub fn sheet_frame(
        ui: &mut egui::Ui,
        title: &str,
        side: impl FnOnce(&mut egui::Ui),
        content: impl FnOnce(&mut egui::Ui),
        detail: impl FnOnce(&mut egui::Ui),
    ) {
        egui::Frame::new()
            .fill(SHEET)
            .inner_margin(8)
            .corner_radius(R_CONTROL)
            .show(ui, |ui| {
                ui.label(egui::RichText::new(title).font(heading()).color(INK));
                ui.separator();
                ui.columns(3, |panes| {
                    side(&mut panes[0]);
                    content(&mut panes[1]);
                    detail(&mut panes[2]);
                });
            });
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
