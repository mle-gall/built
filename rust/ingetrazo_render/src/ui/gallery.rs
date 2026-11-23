//! The design system's reference sheet, in one frame: foundations (colour
//! by role, type, spacing, radii, glass), every molecule across its states,
//! and the organisms with real-looking content. Each cell owns its sample
//! values and input IDs.
use super::{kit::*, tokens::*};
use egui::{pos2, vec2, Align2, Color32, Pos2, Rect, Sense, Stroke};

/// The board, in points; the Qt host scales it to fit.
const BOARD: egui::Vec2 = vec2(1280., 800.);
const STATES: [(&str, State); 6] = [
    ("Rest", State::Idle),
    ("Hover", State::Hover),
    ("Pressed", State::Press),
    ("Selected", State::On),
    ("Focus", State::Focus),
    ("Disabled", State::Off),
];
/// Colour roles, grouped as the docs list them. Tokens not listed here still
/// appear, under "Other", so a new token is never missing from the sheet.
const GROUPS: [(&str, &[&str]); 6] = [
    ("Surfaces", &["SHEET", "SIDE", "CARD", "CARD_HOVER", "WELL", "PANEL", "FIELD", "SCRIM", "SHEET_SCRIM"]),
    ("Text", &["INK", "INK_2", "DIM", "FAINT", "OFF", "BRIGHT", "ON_PRIMARY"]),
    (
        "Interaction",
        &[
            "ACCENT", "ACTIVE", "ACCENT_LINE", "FOCUS_RING", "CONTROL", "HOVER", "PRESS", "THUMB",
            "TRACK", "EDGE", "HOVER_EDGE", "PRIMARY_HOVER", "PRIMARY_PRESS",
        ],
    ),
    ("Feedback", &["OK", "WARN", "ERROR", "INFO", "WARN_FILL", "WARNING_HOVER", "WARNING_PRESS"]),
    (
        "Links and agents",
        &["FOUND", "CHANGED", "MISSING", "PACKED", "WORKING", "WAITING", "QUESTION", "DONE", "FAILED"],
    ),
    (
        "Aliases and effects",
        &["GLASS_BAR", "POPOVER", "DIVIDER", "CONTACT", "POP_SHADOW", "IMAGE_HOVER", "IMAGE_BACK", "ON_PRIMARY_HINT", "SWATCH_FALLBACK"],
    ),
];

#[derive(Default)]
struct Cell {
    text: String,
    number: f64,
    choice: usize,
    on: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Row {
    Primary,
    Secondary,
    Destructive,
    Chips,
    Quiet,
    Field,
    Number,
    Slider,
    Choice,
    Toggle,
    Segmented,
    Tabs,
    ListRow,
    MenuRow,
    Tile,
}
const ROWS: [(&str, Row); 15] = [
    ("Primary button", Row::Primary),
    ("Button", Row::Secondary),
    ("Destructive", Row::Destructive),
    ("Command, warning chip", Row::Chips),
    ("Quiet, icon button", Row::Quiet),
    ("Text field", Row::Field),
    ("Number with unit", Row::Number),
    ("Percent slider", Row::Slider),
    ("Choice", Row::Choice),
    ("Switch", Row::Toggle),
    ("Segmented", Row::Segmented),
    ("Underline tabs", Row::Tabs),
    ("List and tree row", Row::ListRow),
    ("Menu row", Row::MenuRow),
    ("Tile", Row::Tile),
];

impl Row {
    /// Whether the state exists for the component; "—" otherwise.
    fn has(self, st: State) -> bool {
        match st {
            State::On => matches!(self, Row::Toggle | Row::ListRow | Row::MenuRow | Row::Tile),
            State::Press => !matches!(self, Row::Field),
            State::Focus => !matches!(self, Row::MenuRow),
            _ => true,
        }
    }
    fn height(self) -> f32 {
        match self {
            Row::Primary | Row::Secondary | Row::Destructive => BUTTON_H,
            _ => CONTROL_H,
        }
    }
}

pub(super) struct Gallery {
    cells: Vec<Cell>,
    font_picker: Option<FontPicker>,
    dock_filter: String,
    dock_tab: usize,
}

impl Default for Gallery {
    fn default() -> Self {
        Self {
            cells: (0..ROWS.len() * STATES.len())
                .map(|i| {
                    let (row, col) = (ROWS[i / STATES.len()].1, i % STATES.len());
                    Cell {
                        text: if row == Row::Field && col != 0 { "Walls".into() } else { String::new() },
                        number: 2400.,
                        choice: 0,
                        on: row == Row::Toggle && STATES[col].1 == State::On,
                    }
                })
                .collect(),
            font_picker: None,
            dock_filter: String::new(),
            dock_tab: 0,
        }
    }
}

/// Lay a closure out inside a fixed rectangle.
fn put<R>(ui: &mut egui::Ui, r: Rect, layout: egui::Layout, f: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.scope_builder(egui::UiBuilder::new().max_rect(r).layout(layout), f).inner
}
fn down() -> egui::Layout {
    egui::Layout::top_down(egui::Align::Min)
}
fn across() -> egui::Layout {
    egui::Layout::left_to_right(egui::Align::Center)
}
/// A section title with its hairline; returns the y below it.
fn section(ui: &egui::Ui, at: Pos2, w: f32, name: &str, note: &str) -> f32 {
    let p = ui.painter();
    p.text(at, Align2::LEFT_TOP, name, title(), INK);
    if !note.is_empty() {
        p.text(pos2(at.x + w, at.y + 2.), Align2::RIGHT_TOP, note, meta(), FAINT);
    }
    p.hline(at.x..=at.x + w, at.y + 22.5, Stroke::new(1.0_f32, DIVIDER));
    at.y + 30.
}
fn sub(ui: &egui::Ui, at: Pos2, name: &str) {
    ui.painter().text(at, Align2::LEFT_TOP, name, caption_font(), DIM);
}

impl Gallery {
    /// The Qt viewport shares its window with native panes. Fit the entire
    /// reference board into the remaining space and map input into board points.
    pub(super) fn fit(raw: &mut egui::RawInput, screen: Rect) -> f32 {
        let zoom = (screen.width() / BOARD.x)
            .min(screen.height() / BOARD.y)
            .clamp(0.01, 1.);
        if zoom < 1. {
            raw.screen_rect = Some(Rect::from_min_size(screen.min, screen.size() / zoom));
            for event in &mut raw.events {
                match event {
                    egui::Event::PointerMoved(p) | egui::Event::PointerButton { pos: p, .. } => {
                        *p = *p / zoom
                    }
                    _ => {}
                }
            }
        }
        zoom
    }

    pub(super) fn show(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(SHEET).inner_margin(SPACE_M as i8))
            .show(ctx, |ui| {
                egui::ScrollArea::both().show(ui, |ui| {
                    let o = ui.cursor().min;
                    let (board, _) = ui.allocate_exact_size(BOARD - vec2(2. * SPACE_M, 2. * SPACE_M), Sense::hover());
                    let p = ui.painter();
                    p.text(o - vec2(0., 2.), Align2::LEFT_TOP, "Built design system", heading(), INK);
                    p.text(
                        pos2(board.max.x, o.y + 4.),
                        Align2::RIGHT_TOP,
                        "Reference sheet · tokens.rs, kit.rs · design pass 2026-11-23",
                        meta(),
                        FAINT,
                    );
                    let top = o.y + 28.;
                    let left_w = 392.;
                    self.foundations(ui, pos2(o.x, top), left_w);
                    let x = o.x + left_w + SPACE_L + SPACE_S;
                    let w = board.max.x - x;
                    let y = self.molecules(ui, pos2(x, top), w);
                    self.organisms(ui, pos2(x, y + SPACE_S), w, board.max.y);
                    if let Some(picker) = &mut self.font_picker {
                        if picker.popup(ctx).is_some() { self.font_picker = None; }
                    }
                });
            });
    }

    fn foundations(&mut self, ui: &mut egui::Ui, at: Pos2, w: f32) {
        let mut y = section(ui, at, w, "Foundations", "tokens.rs");
        // Colour by role.
        let cell_w = w / 4.;
        let mut listed: Vec<&str> = Vec::new();
        let others: Vec<&str> = SWATCHES
            .iter()
            .map(|(n, _)| *n)
            .filter(|n| !GROUPS.iter().any(|(_, g)| g.contains(n)))
            .collect();
        let groups = GROUPS.iter().map(|(g, names)| (*g, names.to_vec())).chain(
            (!others.is_empty()).then(|| ("Other", others.clone())),
        );
        for (group, names) in groups {
            sub(ui, pos2(at.x, y), group);
            y += 17.;
            for (i, name) in names.iter().enumerate() {
                let Some(&(_, color)) = SWATCHES.iter().find(|(n, _)| n == name) else { continue };
                listed.push(name);
                let cx = at.x + (i % 4) as f32 * cell_w;
                let cy = y + (i / 4) as f32 * 17.;
                let sample = Rect::from_min_size(pos2(cx, cy + 1.), vec2(14., 14.));
                ui.painter().rect_filled(sample, R_INSET, color);
                ui.painter().rect_stroke(sample, R_INSET, Stroke::new(1.0_f32, EDGE), egui::StrokeKind::Inside);
                ui.painter().text(
                    pos2(cx + 19., cy + 8.),
                    Align2::LEFT_CENTER,
                    name.to_lowercase(),
                    meta(),
                    INK_2,
                );
            }
            y += names.len().div_ceil(4) as f32 * 17. + SPACE_XS;
        }
        // Type: four sizes; weight and ink make the rest of the hierarchy.
        y += SPACE_XS;
        sub(ui, pos2(at.x, y), "Type");
        y += 18.;
        let specimens: [(&str, &str, egui::FontId, Color32); 5] = [
            ("Heading 16", "Library", heading(), INK),
            ("Title 13", "Walls", title(), INK),
            ("Body 12", "IfcWall · 2 400 mm", body(), INK),
            ("Meta 11", "Predefined type · 48 · ⌘K", meta(), DIM),
            ("Label 11 medium", "Export details", caption_font(), DIM),
        ];
        for (name, sample, font, ink) in specimens {
            let h = if font.size > 14. { 24. } else { 19. };
            ui.painter().text(pos2(at.x, y + h / 2.), Align2::LEFT_CENTER, name, meta(), FAINT);
            ui.painter().text(pos2(at.x + 104., y + h / 2.), Align2::LEFT_CENTER, sample, font, ink);
            y += h;
        }
        // Spacing and radii.
        y += SPACE_S;
        sub(ui, pos2(at.x, y), "Spacing");
        sub(ui, pos2(at.x + w / 2., y), "Radii");
        y += 18.;
        let mut x = at.x;
        for s in SPACE {
            let r = Rect::from_min_size(pos2(x, y + 24. - s), vec2(s, s));
            ui.painter().rect_filled(r, 1, ACCENT_LINE);
            ui.painter().text(pos2(x + s / 2., y + 30.), Align2::CENTER_TOP, format!("{s}"), meta(), FAINT);
            x += s.max(14.) + 18.;
        }
        let mut x = at.x + w / 2.;
        for (r, name) in [(R_INSET, "inset"), (R_ROW, "row"), (R_CONTROL, "control"), (12, "raised"), (14, "panel")] {
            let rect = Rect::from_min_size(pos2(x, y), vec2(28., 24.));
            ui.painter().rect_filled(rect, r, CONTROL);
            ui.painter().rect_stroke(rect, r, Stroke::new(1.0_f32, HOVER_EDGE), egui::StrokeKind::Inside);
            ui.painter().text(pos2(x + 14., y + 30.), Align2::CENTER_TOP, format!("{r}"), meta(), FAINT);
            let _ = name;
            x += 38.;
        }
        y += 50.;
        // Glass levels, lowest to highest.
        sub(ui, pos2(at.x, y), &format!("Glass · sheet blur {SHEET_BLUR}× · scrim {:.0}% · vignette {:.0}%", SHEET_SCRIM.a() as f32 / 2.55, SHEET_VIGNETTE * 100.0));
        y += 20.;
        let tile_w = (w - 3. * SPACE_S) / 4.;
        for (i, surface) in Surface::ALL.into_iter().enumerate() {
            let r = Rect::from_min_size(pos2(at.x + i as f32 * (tile_w + SPACE_S), y), vec2(tile_w, 52.));
            put(ui, r, down(), |ui| {
                glass(surface)
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.set_width(tile_w - 20.);
                        ui.set_height(36.);
                        ui.label(egui::RichText::new(surface.name()).font(strong(TYPE_BODY)).color(INK));
                        let role = match surface {
                            Surface::Bar => "bars",
                            Surface::Panel => "dock",
                            Surface::Pill => "capsule",
                            Surface::Raised => "menus",
                        };
                        ui.label(egui::RichText::new(role).font(meta()).color(DIM));
                    });
            });
        }
    }

    /// One row per component, one column per state. Returns the y below.
    fn molecules(&mut self, ui: &mut egui::Ui, at: Pos2, w: f32) -> f32 {
        let mut y = section(ui, at, w, "Molecules", "kit.rs · one owner per keystroke; Esc unwinds");
        let name_w = 128.;
        let col_w = (w - name_w) / STATES.len() as f32;
        for (c, (name, _)) in STATES.iter().enumerate() {
            ui.painter().text(
                pos2(at.x + name_w + c as f32 * col_w, y),
                Align2::LEFT_TOP,
                *name,
                caption_font(),
                DIM,
            );
        }
        y += 18.;
        for (k, (name, row)) in ROWS.iter().enumerate() {
            let h = row.height();
            ui.painter().text(pos2(at.x, y + h / 2.), Align2::LEFT_CENTER, *name, meta(), INK_2);
            for (c, (_, st)) in STATES.iter().enumerate() {
                let r = Rect::from_min_size(pos2(at.x + name_w + c as f32 * col_w, y), vec2(col_w - 10., h));
                if !row.has(*st) {
                    ui.painter().text(pos2(r.min.x + 2., r.center().y), Align2::LEFT_CENTER, "—", meta(), OFF);
                    continue;
                }
                let cell = &mut self.cells[k * STATES.len() + c];
                ui.push_id((k, c), |ui| {
                    put(ui, r, across(), |ui| {
                        ui.spacing_mut().item_spacing = vec2(SPACE_XS, SPACE_XS);
                        ui.add_enabled_ui(*st != State::Off, |ui| {
                            // A menu row's "selected" is its check mark, not a wash.
                            let forced = if *row == Row::MenuRow && *st == State::On { State::Idle } else { *st };
                            specimen(ui, forced, |ui| Self::molecule(ui, *row, *st, cell));
                        });
                    });
                });
            }
            y += h + 2.;
        }
        // Status chips: one shape; the dot carries the tone.
        y += SPACE_XS;
        ui.painter().text(pos2(at.x, y + CONTROL_H / 2.), Align2::LEFT_CENTER, "Status chip", meta(), INK_2);
        let r = Rect::from_min_size(pos2(at.x + name_w, y), vec2(w - name_w, CONTROL_H));
        put(ui, r, across(), |ui| {
            ui.spacing_mut().item_spacing = vec2(SPACE_XS, 0.);
            for (label, tone) in [
                ("Found", FOUND),
                ("Changed", CHANGED),
                ("Missing link", MISSING),
                ("Packed", PACKED),
                ("Working", WORKING),
                ("Question", QUESTION),
                ("Done", DONE),
                ("Failed", FAILED),
            ] {
                status_chip(ui, label, tone);
            }
        });
        y += CONTROL_H + SPACE_S;
        let r = Rect::from_min_size(pos2(at.x + name_w, y), vec2(w - name_w, CONTROL_H));
        ui.painter().text(pos2(at.x, y + CONTROL_H / 2.), Align2::LEFT_CENTER, "Formatted prose", meta(), INK_2);
        put(ui, r, across(), |ui| {
            prose(ui, &[
                ProseSpan { text: "Fixed ".into(), bold: true, ..Default::default() },
                ProseSpan { text: "face selection. ".into(), ..Default::default() },
                ProseSpan { text: "Release notes".into(), link: "notes".into(), ..Default::default() },
            ]);
        });
        y += CONTROL_H + SPACE_S;
        let height = ROW_H * 2.;
        let r = Rect::from_min_size(pos2(at.x + name_w, y), vec2(w - name_w, height));
        ui.painter().text(pos2(at.x, y + CONTROL_H / 2.), Align2::LEFT_CENTER, "Credits", meta(), INK_2);
        put(ui, r, across(), |ui| {
            credits_roll(ui, "gallery", &[CreditRow { name: "Marco Sumari Tellez".into(),
                role: "Civil Engineer — Arequipa, Peru".into(), ..Default::default() }], height);
        });
        y += height + SPACE_S;
        let (_, preview) = catalogue_grid(CONTROL_H * 2., CONTROL_H * 2.);
        ui.painter().text(pos2(at.x, y + CONTROL_H / 2.), Align2::LEFT_CENTER, "Catalogue preview", meta(), INK_2);
        let rect = Rect::from_min_size(pos2(at.x + name_w, y), preview);
        paint_tile(ui, rect, State::Idle);
        site_pin(ui.painter(), rect.center(), "(0,0)");
        let r = Rect::from_min_size(rect.right_top() + vec2(SPACE_M, 0.), vec2(w - name_w - preview.x - SPACE_M, preview.y));
        put(ui, r, egui::Layout::top_down(egui::Align::Min), |ui| {
            catalogue_fact(ui, "Licence", "MIT");
            catalogue_tag(ui, "Architecture", ui.available_width());
        });
        y + preview.y + SPACE_S
    }

    fn molecule(ui: &mut egui::Ui, row: Row, st: State, cell: &mut Cell) {
        match row {
            Row::Primary => {
                sheet_button(ui, "Export", "↵", 1);
            }
            Row::Secondary => {
                sheet_button(ui, "Cancel", "Esc", 0);
            }
            Row::Destructive => {
                sheet_button(ui, "Discard", "", 2);
            }
            Row::Chips => {
                chip_button(ui, "Edit");
                amber_button(ui, "2 issues");
            }
            Row::Quiet => {
                quiet(ui, "Details");
                icon_button(ui, "×", "Close");
            }
            Row::Field => {
                ui.add(search(&mut cell.text, "Name").desired_width(ui.available_width()));
            }
            Row::Slider => {
                ui.spacing_mut().slider_width = (ui.available_width() - 42.).max(18.);
                if let Some(v) = percent_slider(ui, ui.id().with("slider"), cell.number.min(100.), 100.) {
                    cell.number = v;
                }
            }
            Row::Number => {
                ui.add(number(&mut cell.number, " mm").speed(10.));
            }
            Row::Choice => {
                ui.set_width(ui.available_width());
                choice(ui, "kind", &mut cell.choice, &["IfcWall", "IfcSlab", "IfcRoof"]);
            }
            Row::Toggle => {
                toggle(ui, &mut cell.on, "Visible");
            }
            Row::Segmented => {
                let w = ui.available_width();
                segmented(ui, &["Model", "Sheets"], 0, w);
            }
            Row::Tabs => {
                underline_tabs(ui, &[("Files".into(), "3".into(), true), ("Links".into(), String::new(), true)], 0);
            }
            Row::ListRow => {
                ui.vertical(|ui| {
                    let (r, _) = list_row(ui, ROW_H, st == State::On);
                    dot(ui.painter(), r.left_center() + vec2(12., 0.), if st == State::Off { OFF } else { FOUND });
                    let ink = if st == State::Off { OFF } else { INK };
                    ui.painter().text(r.left_center() + vec2(20., 0.), Align2::LEFT_CENTER, "Walls", body(), ink);
                    ui.painter().text(r.right_center() - vec2(8., 0.), Align2::RIGHT_CENTER, "6", meta(), if st == State::Off { OFF } else { DIM });
                });
            }
            Row::MenuRow => {
                ui.vertical(|ui| {
                    let checked = if st == State::On { 1 } else { 0 };
                    menu_row(ui, &(String::new(), "Edges".into(), "E".into(), "check".into(), checked, st != State::Off));
                });
            }
            Row::Tile => {
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), CONTROL_H), Sense::click());
                paint_tile(ui, r, st);
                let sw = Rect::from_min_size(r.min + vec2(4., 4.), vec2(16., 16.));
                ui.painter().rect_filled(sw, R_INSET, if st == State::Off { OFF } else { SWATCH_FALLBACK });
                ui.painter().text(pos2(sw.max.x + 8., r.center().y), Align2::LEFT_CENTER, "Brick", body(),
                    if st == State::Off { OFF } else { INK });
            }
        }
    }

    fn organisms(&mut self, ui: &mut egui::Ui, at: Pos2, w: f32, bottom: f32) {
        let y = section(ui, at, w, "Organisms", "dock, inspector, menu, question sheet, activity");
        let h = bottom - y;
        let gap = SPACE_S;
        let widths = [222., 214., 168.];
        let mut x = at.x;
        let mut rects = Vec::new();
        for cw in widths {
            rects.push(Rect::from_min_size(pos2(x, y), vec2(cw, h)));
            x += cw + gap;
        }
        rects.push(Rect::from_min_max(pos2(x, y), pos2(at.x + w, bottom)));
        put(ui, rects[0], down(), |ui| self.dock(ui));
        put(ui, rects[1], down(), |ui| Self::inspector(ui));
        put(ui, rects[2], down(), |ui| {
            Self::menu(ui);
            if quiet(ui, "Font picker…").clicked() {
                self.font_picker = Some(FontPicker::new(FontChoice {
                    title: "Sheet text".into(), text: "Ground floor".into(), font: "Helvetica".into(),
                    bold: false, italic: false, height: 14., thickness: None,
                    minimum: 4., maximum: 96., unit: " pt".into(), editable: true,
                    recent: vec!["Helvetica".into()], favourites: vec!["Georgia".into()],
                }));
            }
        });
        put(ui, rects[3], down(), |ui| Self::feedback(ui));
    }

    /// The dock: place switch, sections, filter and the project tree.
    fn dock(&mut self, ui: &mut egui::Ui) {
        glass(Surface::Panel)
            .inner_margin(egui::Margin::same(SPACE_S as i8))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0., SPACE_XS);
                let w = ui.available_width();
                ui.set_width(w);
                segmented(ui, &["Project", "Library"], 0, w);
                if let Some(i) = underline_tabs(
                    ui,
                    &[("Filing".into(), String::new(), true), ("Drawing".into(), String::new(), true), ("Deliver".into(), "2".into(), true)],
                    self.dock_tab,
                ) {
                    self.dock_tab = i;
                }
                ui.add(search(&mut self.dock_filter, "Filter nodes").desired_width(w));
                ui.spacing_mut().item_spacing = vec2(0., 0.);
                // (name, depth, expanded, selected, eye, lock, count, state dot, note)
                let rows: [(&str, usize, Option<bool>, bool, i8, i8, &str, Option<Color32>, &str); 6] = [
                    ("Ride.igz", 0, Some(false), false, 1, 0, "48", Some(FOUND), ""),
                    ("Studio", 0, Some(true), false, 1, 0, "12", None, ""),
                    ("Walls", 1, None, true, 1, 0, "6", None, ""),
                    ("Roof", 1, None, false, 1, 1, "4", None, ""),
                    ("Furniture", 1, None, false, 0, 0, "2", None, ""),
                    ("Survey.igz", 0, None, false, -1, -1, "", Some(MISSING), "via Ride"),
                ];
                for (name, depth, expanded, selected, eye, lock, count, state, note) in rows {
                    let (r, resp) = list_row(ui, ROW_H, selected);
                    let x0 = r.min.x + 6. + depth as f32 * 14.;
                    if let Some(open) = expanded {
                        chevron(ui.painter(), pos2(x0 + 4., r.center().y), open, FAINT);
                    }
                    let mut x = x0 + 14.;
                    let strong_row = depth == 0;
                    let g = ui.painter().layout_no_wrap(
                        name.into(),
                        if strong_row { strong(TYPE_BODY) } else { body() },
                        INK,
                    );
                    let gw = g.size().x;
                    leading(ui.painter(), r, x, g, if resp.hovered() || selected { INK } else { INK_2 });
                    x += gw + 6.;
                    if let Some(c) = state {
                        dot(ui.painter(), pos2(x + 3., r.center().y), c);
                        x += 10.;
                    }
                    if !note.is_empty() {
                        ui.painter().text(pos2(x, r.center().y), Align2::LEFT_CENTER, note, meta(), FAINT);
                    }
                    tree_slots(ui, r.right() - 8., r.center().y, eye, lock, resp.hovered() || eye == 0 || lock == 1 || selected);
                    ui.painter().text(r.right_center() - vec2(8., 0.), Align2::RIGHT_CENTER, count, meta(), DIM);
                }
            });
    }

    /// The inspector: subject, properties, next steps and Export details.
    fn inspector(ui: &mut egui::Ui) {
        glass(Surface::Panel)
            .inner_margin(egui::Margin::same(SPACE_S as i8))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0., 0.);
                ui.set_width(ui.available_width());
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.), Sense::hover());
                ui.painter().text(r.left_center() + vec2(8., 0.), Align2::LEFT_CENTER, "Walls", title(), INK);
                ui.painter().text(r.right_center() - vec2(8., 0.), Align2::RIGHT_CENTER, "Wall · 6 items", meta(), DIM);
                ui.add_space(SPACE_XS);
                identity_card(ui, &[
                    ("Kind".into(), "Wall".into(), "reveal".into(), None),
                    ("Filed in".into(), "Building › Walls".into(), "reveal".into(), None),
                    ("Tag".into(), "A-WALL".into(), "eye".into(), Some(true)),
                ], &mut Vec::new());
                ui.add_space(SPACE_XS);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(SPACE_XS, 0.);
                    ui.add_space(SPACE_S);
                    chip_button(ui, "Measure");
                    amber_button(ui, "2 issues · Show");
                });
                ui.add_space(SPACE_XS);
                disclosure(ui, "gallery-export", "Export details", true, |ui| {
                    prop_row(ui, "IFC class", "IfcWall");
                    prop_row(ui, "Predefined type", "STANDARD");
                    prop_row(ui, "Uniclass", "EF_25_10 Walls");
                });
            });
    }

    /// A row menu: caption lined up with the row text, check lane,
    /// shortcuts, a separator, one row hovered and one disabled.
    fn menu(ui: &mut egui::Ui) {
        menu(ui, 176., |ui| {
            caption_at(ui, "Walls", 12.);
            let item = |label: &str, key: &str, checked: i8, enabled: bool| {
                (String::new(), label.to_owned(), key.to_owned(), "check".to_owned(), checked, enabled)
            };
            menu_row(ui, &item("Select its items", "↵", -1, true));
            specimen(ui, State::Hover, |ui| {
                menu_row(ui, &item("Isolate", "⌥I", -1, true));
            });
            menu_row(ui, &item("Zoom to", "⇧Z", -1, true));
            menu_separator(ui);
            menu_row(ui, &item("Visible", "H", 1, true));
            menu_row(ui, &item("Locked", "", 0, true));
            menu_row(ui, &item("Explode", "", -1, false));
            menu_separator(ui);
            menu_row(ui, &item("Rename", "F2", -1, true));
        });
    }

    /// A question sheet, the activity capsule with measured progress, and a toast.
    fn feedback(ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing = vec2(0., SPACE_S);
        popover()
            .inner_margin(egui::Margin::same(SPACE_M as i8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing = vec2(SPACE_S, SPACE_XS);
                ui.label(egui::RichText::new("Save changes?").font(title()).color(INK));
                ui.label(egui::RichText::new("Courtyard has unsaved edits.").font(body()).color(DIM));
                ui.add_space(SPACE_XS);
                let w = ui.available_width();
                ui.allocate_ui_with_layout(vec2(w, BUTTON_H), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    sheet_button(ui, "Save", "↵", 1);
                    sheet_button(ui, "Cancel", "Esc", 0);
                });
            });
        activity_capsule().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = vec2(SPACE_XS, 0.);
            ui.horizontal(|ui| {
                icon_button(ui, "▾", "Activity history");
                ui.label(egui::RichText::new("Export drawings").font(body()).color(INK));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(SPACE_XS);
                    ui.label(egui::RichText::new("45 %").font(meta()).color(DIM));
                });
            });
            ui.add_space(SPACE_XS);
            progress_bar(ui, 0.45, 3.);
            ui.add_space(SPACE_XS);
        });
        toast(ui, 1., |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = vec2(SPACE_XS, 0.);
            ui.horizontal(|ui| {
                dot(ui.painter(), ui.cursor().min + vec2(3., CONTROL_H / 2.), DONE);
                ui.add_space(SPACE_M);
                ui.label(egui::RichText::new("Drawings exported").font(strong(TYPE_BODY)).color(INK));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    quiet(ui, "Show");
                });
            });
            ui.label(egui::RichText::new("12 pages to Deliveries/Handover").font(meta()).color(DIM));
        });
    }
}
