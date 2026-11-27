//! The design system's reference sheet, in one frame: foundations (colour
//! by role, type, spacing, radii, glass), every molecule across its states,
//! and the organisms with real-looking content. Each cell owns its sample
//! values and input IDs.
use super::{kit::*, tokens::*};
use egui::{pos2, vec2, Align2, Color32, Pos2, Rect, Sense, Stroke};

/// The board, in points; the Qt host scales it to fit.
const BOARD: egui::Vec2 = vec2(1280., 1580.);
/// The surfaces band at the bottom: large sheet, value field, atoms, toasts.
const BAND_H: f32 = 206.;
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
    ("Surfaces", &["SHEET", "SIDE", "CARD", "CARD_HOVER", "WELL", "DESK", "PANEL", "FIELD", "SCRIM", "SHEET_SCRIM"]),
    ("Text", &["INK", "INK_2", "DIM", "FAINT", "OFF", "BRIGHT", "ON_PRIMARY"]),
    (
        "Interaction",
        &[
            "ACCENT", "ACTIVE", "LIT", "ACCENT_LINE", "FOCUS_RING", "CONTROL", "HOVER", "PRESS", "THUMB",
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
    SwitchRow,
    ToolButton,
    Segmented,
    Tabs,
    ListRow,
    MenuRow,
    CommandRow,
    Tile,
}
const ROWS: [(&str, Row); 18] = [
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
    ("Switch row (form)", Row::SwitchRow),
    ("Tool, bar toggle", Row::ToolButton),
    ("Segmented", Row::Segmented),
    ("Underline tabs", Row::Tabs),
    ("List and tree row", Row::ListRow),
    ("Menu row", Row::MenuRow),
    ("Command result", Row::CommandRow),
    ("Tile", Row::Tile),
];

impl Row {
    /// Whether the state exists for the component; "—" otherwise.
    fn has(self, st: State) -> bool {
        match st {
            State::On => matches!(self, Row::Toggle | Row::SwitchRow | Row::ToolButton | Row::ListRow | Row::MenuRow | Row::CommandRow | Row::Tile),
            State::Press => !matches!(self, Row::Field),
            State::Focus => !matches!(self, Row::MenuRow),
            _ => true,
        }
    }
    fn height(self) -> f32 {
        match self {
            Row::Primary | Row::Secondary | Row::Destructive => BUTTON_H,
            Row::ToolButton => 30.,
            _ => CONTROL_H,
        }
    }
}

pub(super) struct Gallery {
    cells: Vec<Cell>,
    font_picker: Option<FontPicker>,
    dock_filter: String,
    dock_tab: usize,
    sheet_tab: usize,
    swatch: Color32,
    sheet_name: String,
    sample_path: String,
    sample_rgb: [u8; 3],
    strip: super::tabs::Strip,
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
            sheet_tab: 0,
            swatch: SWATCH_FALLBACK,
            sheet_name: "Oak, oiled".into(),
            sample_path: "/Applications/Blender.app/Contents/MacOS/Blender".into(),
            sample_rgb: [96, 146, 180],
            strip: {
                let mut strip = super::tabs::Strip::default();
                strip.tabs = ["Model A", "Model B"].into_iter().enumerate().map(|(i, name)|
                    (i.to_string(), name.into(), i == 0, false, false, String::new(), String::new())).collect();
                strip
            },
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
    /// Fit the board width and map pointer input into board points. Height scrolls.
    pub(super) fn fit(raw: &mut egui::RawInput, screen: Rect) -> f32 {
        let zoom = (screen.width() / BOARD.x)
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
                        "Reference sheet · tokens.rs, kit.rs · design pass and review fixes 2026-11-23",
                        meta(),
                        FAINT,
                    );
                    let top = o.y + 28.;
                    let left_w = 392.;
                    self.foundations(ui, pos2(o.x, top), left_w);
                    let x = o.x + left_w + SPACE_L + SPACE_S;
                    let w = board.max.x - x;
                    let y = self.molecules(ui, pos2(x, top), w);
                    self.organisms(ui, pos2(x, y + SPACE_S), w, board.max.y - BAND_H - SPACE_M);
                    self.surfaces(ui, pos2(o.x, board.max.y - BAND_H), board.width());
                    put(ui, Rect::from_min_size(pos2(o.x, board.max.y + SPACE_M), vec2(board.width(), 240.)), down(), |ui| {
                        caption(ui, "Plugin controls and viewport chrome");
                        ui.columns(3, |cols| {
                            let controls = &mut cols[0];
                            image_file_row(controls, "Image", "Quarter-sawn European beech.jpg");
                            property_control(controls, "Executable", |ui| { path_field(ui, &mut self.sample_path); });
                            property_control(controls, "Colour", |ui| { colour_well(ui, &mut self.sample_rgb); });
                            property_control(controls, "Lens", |ui| { select(ui, "lens_sample", "35 mm", &["24 mm", "35 mm", "50 mm"]); });
                            property_control(controls, "Render", |ui| progress_bar(ui, 0.6, 4.));
                            table_row(controls, &["Entity".into(), "Count".into()], false, true);
                            table_row(controls, &["Face".into(), "24".into()], true, false);
                            let pair = ["Keep Built".to_string(), "Take Blender".to_string()];
                            let reserve = review_actions_width(controls, &pair);
                            review_row(controls, &["Changed".into(), "Built".into(), "Blender".into()], true, false, &[], reserve);
                            review_row(controls, &["Bench seat · Position X".into(), "1.25 m".into(), "1.10 m".into()], false, true, &pair, reserve);
                            review_row(controls, &["Oak · Roughness".into(), "0.80".into(), "0.35".into()], false, false,
                                       &["Apply".to_string(), "Ignore".to_string()], reserve);
                            let chrome = &mut cols[1];
                            chrome.horizontal(|ui| {
                                ui.set_height(super::TOP_H);
                                let actions = self.strip.show(ui, ui.available_width());
                                for action in actions {
                                    if let Some(index) = action.strip_prefix("ptab:activate:").and_then(|s| s.parse::<usize>().ok()) {
                                        for (i, tab) in self.strip.tabs.iter_mut().enumerate() { tab.2 = i == index; }
                                    }
                                }
                            });
                            super::context_menu::segment_row(chrome, "Intersect", &[
                                ("model".into(),"Model".into(),String::new(),true,-1),
                                ("selection".into(),"Selection".into(),String::new(),true,-1)]);
                            glass(Surface::Bar).show(chrome, |ui| {
                                ui.horizontal(|ui| {
                                    label(ui, "Move");
                                    super::item_button(ui, &(String::new(), "Copy  Alt".into(), String::new(), String::new(), 0, true), None);
                                    ui.label(egui::RichText::new("Click to place").font(body()).color(DIM));
                                });
                            });
                            let form = &mut cols[2];
                            caption(form, "Make component");
                            property_control(form, "Name", |ui| { ui.add(field(&mut self.sheet_name)); });
                            form.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                sheet_button(ui, "Create", "↩", 1);
                                sheet_button(ui, "Cancel", "esc", 0);
                            });
                        });
                    });
                    if let Some(picker) = &mut self.font_picker {
                        if picker.popup(ctx, ctx.screen_rect().center_top() + vec2(0., SPACE_S)).is_some() { self.font_picker = None; }
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
        y += preview.y + SPACE_S;
        ui.painter().text(pos2(at.x, y + CONTROL_H / 2.), Align2::LEFT_CENTER, "SVG preview", meta(), INK_2);
        let page = vec2(CONTROL_H * 3., CONTROL_H * 2.);
        let rect = Rect::from_min_size(pos2(at.x + name_w, y), page);
        put(ui, rect, across(), |ui| { svg_preview(ui, None, "Thirds", page); });
        let image = Rect::from_min_size(rect.right_top() + vec2(SPACE_M, 0.), vec2(CONTROL_H * 7., page.y));
        put(ui, image, down(), |ui| render_preview(ui, None, vec2(image.width(), CONTROL_H)));
        y + page.y + SPACE_S
    }

    fn molecule(ui: &mut egui::Ui, row: Row, st: State, cell: &mut Cell) {
        match row {
            Row::Primary => {
                sheet_button(ui, "Export", "↵", 1);
            }
            Row::Secondary => {
                panel_button(ui, "Cancel", "Esc", 0, None);
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
                row_link(ui, "Show");
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
            Row::SwitchRow => {
                // Off in every column but Selected: the empty track is the off value.
                ui.vertical(|ui| { switch_row(ui, "Snap", st == State::On, st != State::Off); });
            }
            Row::ToolButton => {
                // The armed tool and a bar toggle that is on share the lit state.
                let (r, _) = ui.allocate_exact_size(vec2(30., 30.), Sense::click());
                if st == State::On { paint_lit(ui, r, R_CONTROL) } else { paint_state(ui, r, st, R_CONTROL) }
                let ink = ink_of(st);
                ui.painter().rect_stroke(r.shrink(9.), R_INSET, Stroke::new(1.5_f32, ink), egui::StrokeKind::Middle);
                ui.painter().circle_filled(r.shrink(9.).left_bottom(), 2.5, ink);
                ui.painter().circle_filled(r.shrink(9.).right_top(), 2.5, ink);
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
            Row::CommandRow => {
                command_row(ui, "Undo", "Edit", "⌘Z", st != State::Off, st == State::On);
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
        let widths = [222. + DESTINATION_STRIP_W, 214., 168.];
        let mut x = at.x;
        let mut rects = Vec::new();
        for cw in widths {
            rects.push(Rect::from_min_size(pos2(x, y), vec2(cw, h)));
            x += cw + gap;
        }
        rects.push(Rect::from_min_max(pos2(x, y), pos2(at.x + w, bottom)));
        let mut content = rects[0];
        content.max.x -= DESTINATION_STRIP_W;
        put(ui, content, down(), |ui| self.dock(ui));
        let strip = Rect::from_min_max(content.right_top(), rects[0].right_bottom());
        destination_strip(ui, strip, |ui| {
            destination_button(ui, "Project", "folder", "⌥⌘1", "", true);
            destination_button(ui, "Tasks", "tasks", "⌥⌘2", "2", false);
            destination_button(ui, "Deliveries", "component", "⌥⌘3", "!", false);
        });
        put(ui, rects[1], down(), |ui| Self::inspector(ui));
        put(ui, rects[2], down(), |ui| {
            Self::menu(ui);
            if quiet(ui, "Font picker…").clicked() {
                self.font_picker = Some(FontPicker::new(FontChoice {
                    title: "Sheet text".into(), text: "Ground floor".into(), font: "Helvetica".into(),
                    bold: false, italic: false, height: 14., thickness: None, lengths: None,
                    minimum: 4., maximum: 96., unit: " pt".into(), editable: true,
                    recent: vec!["Helvetica".into()], favourites: vec!["Georgia".into()],
                }));
            }
        });
        put(ui, rects[3], down(), |ui| Self::feedback(ui));
    }

    /// The large sheet, the value field on the geometry, the atoms and the
    /// toast stack (review fixes 2026-11-23).
    fn surfaces(&mut self, ui: &mut egui::Ui, at: Pos2, w: f32) {
        let y = section(ui, at, w, "Surfaces", "large sheet, value field, atoms, toasts");
        let h = BAND_H - (y - at.y);
        // The shared large sheet: header tabs, side list, catalogue cards,
        // detail pane and footer, on the sheet glass.
        let sheet = Rect::from_min_size(pos2(at.x, y), vec2(560., h));
        let f = sheet_frame(ui, sheet, 112., 150., true);
        put(ui, f.header.shrink2(vec2(SPACE_M, SPACE_M)), across(), |ui| {
            let tabs = [("Materials".into(), "48".into(), true), ("Components".into(), String::new(), true), ("Profiles".into(), String::new(), false)];
            if let Some(i) = inline_underline_tabs(ui, &tabs, self.sheet_tab) { self.sheet_tab = i; }
        });
        if let Some(side) = f.side {
            put(ui, side.shrink(SPACE_S), down(), |ui| {
                caption_plain(ui, "Bundled");
                let (r, _) = list_row(ui, ROW_H, true);
                ui.painter().text(r.left_center() + vec2(8., 0.), Align2::LEFT_CENTER, "Wood", body(), INK);
                let (r, _) = list_row(ui, ROW_H, false);
                ui.painter().text(r.left_center() + vec2(8., 0.), Align2::LEFT_CENTER, "Stone", body(), INK_2);
            });
        }
        let (cols, card) = catalogue_grid(f.content.width() - 2. * SPACE_S, 64.);
        for (i, (name, tone)) in [("Oak", parse_hex("#b58c60")), ("Walnut", parse_hex("#6e4e34"))].into_iter().enumerate().take(cols.max(1)) {
            let r = Rect::from_min_size(f.content.min + vec2(SPACE_S + i as f32 * (card.x + SPACE_S), SPACE_S), card);
            let resp = ui.interact(r, ui.id().with(("card", i)), Sense::click());
            paint_card(ui, r, resp.hovered(), i == 1);
            let swatch = Rect::from_min_size(r.min + vec2(4., 4.), vec2(r.width() - 8., r.height() - 24.));
            // Material colours are document data, decoded by the token module.
            ui.painter().rect_filled(swatch, R_INSET, tone.unwrap_or(SWATCH_FALLBACK));
            let g = ui.painter().layout_no_wrap(name.into(), meta(), INK_2);
            centered(ui.painter(), Rect::from_min_max(pos2(r.min.x, r.max.y - 20.), r.max), g, INK_2);
        }
        if let Some(detail) = f.detail {
            put(ui, detail.shrink(SPACE_S), down(), |ui| {
                ui.add(field(&mut self.sheet_name).desired_width(ui.available_width()));
                catalogue_fact(ui, "Finish", "Oiled");
                catalogue_tag(ui, "Timber", ui.available_width());
                ui.add(progress(0.6).desired_height(3.));
                empty_state(ui, "No texture yet");
            });
        }
        if let Some(foot) = f.footer {
            let g = ui.painter().layout_no_wrap("↑↓ choose · ↵ paint · esc close".into(), meta(), FAINT);
            centered(ui.painter(), foot, g, FAINT);
        }
        // The value field on the geometry: typing, then read back after Enter.
        let x = sheet.max.x + SPACE_L;
        sub(ui, pos2(x, y), "Value field");
        put(ui, Rect::from_min_size(pos2(x, y + 18.), vec2(180., 40.)), down(), |ui| value_pill(ui, "Length", "2.35 m", true, false));
        put(ui, Rect::from_min_size(pos2(x, y + 62.), vec2(180., 40.)), down(), |ui| value_pill(ui, "Length", "2.35 m", false, true));
        // A refused value (Fillet: the size does not fit), in the error colour.
        put(ui, Rect::from_min_size(pos2(x + 190., y + 62.), vec2(300., 40.)), down(),
            |ui| value_pill_tone(ui, "Radius", "Radius too large: 9.5 mm fits.", false, false, true));
        // The keystroke overlay for recordings (keycast.rs).
        let chips = [
            crate::keycast::Chip { keys: "R".into(), tool: "Rectangle".into(), typed: false },
            crate::keycast::Chip { keys: "2.5m".into(), tool: String::new(), typed: true },
            crate::keycast::Chip { keys: "⇧⌘Z".into(), tool: String::new(), typed: false },
        ];
        put(ui, Rect::from_min_size(pos2(x + 190., y + 18.), vec2(300., 40.)), down(), |ui| keystroke_pill(ui, &chips, 1.0));
        // Atoms: visibility, lock, disclosure, state dot, focus ring,
        // selected-row marker, badge, swatch, activity ring.
        sub(ui, pos2(x, y + 112.), "Atoms");
        let mut ax = x;
        let ay = y + 130.;
        for (i, on) in [true, false].into_iter().enumerate() {
            paint_eye(ui, Rect::from_min_size(pos2(ax + i as f32 * 20., ay), vec2(16., 16.)), on);
            paint_lock(ui, Rect::from_min_size(pos2(ax + 40. + i as f32 * 20., ay), vec2(16., 16.)), on);
        }
        paint_row_eye(ui, Rect::from_min_size(pos2(ax + 80., ay), vec2(16., 16.)), 2);
        ax += 104.;
        chevron(ui.painter(), pos2(ax, ay + 8.), true, FAINT);
        chevron(ui.painter(), pos2(ax + 14., ay + 8.), false, FAINT);
        dot(ui.painter(), pos2(ax + 30., ay + 8.), FOUND);
        let focus = Rect::from_min_size(pos2(ax + 44., ay), vec2(40., 16.));
        ui.painter().rect_filled(focus, R_INSET, CONTROL);
        paint_focus(ui, focus, R_INSET);
        let row = Rect::from_min_size(pos2(ax + 92., ay - 4.), vec2(70., ROW_H));
        paint_selected_row(ui, row);
        // A library tile's source chip (where the item lives).
        let tile = Rect::from_min_size(pos2(ax + 170., ay - 26.), vec2(84., 48.));
        ui.painter().rect_filled(tile, R_INSET, CONTROL);
        source_chip(ui.painter(), tile, "My library");
        put(ui, Rect::from_min_size(pos2(x, ay + 24.), vec2(260., CONTROL_H)), across(), |ui| {
            badge(ui, "Packed", PACKED);
            swatch(ui, &mut self.swatch);
            let (sample, _) = ui.allocate_exact_size(vec2(14., 14.), egui::Sense::hover());
            tag_swatch(ui, sample, None);
            ring(ui);
        });
        // The three visible results use the same tone marks as the live stack.
        let tx = x + 280.;
        sub(ui, pos2(tx, y), "Toast stack");
        put(ui, Rect::from_min_size(pos2(tx, y + 18.), vec2(w - (tx - at.x), h - 18.)), down(), |ui| {
            ui.spacing_mut().item_spacing = vec2(0., SPACE_XS);
            let rows = [("error", "File not found"), ("warning", "Select a group first"), ("success", "Saved")];
            toast_stack(ui, rows.iter().map(|row| (row, 1.)), |ui, (tone, title)| {
                let (mark, ink) = notice_mark(tone);
                ui.colored_label(ink, format!("{mark}  {title}"));
            });
        });
    }

    /// The dock: place switch, sections, filter and the project tree.
    fn dock(&mut self, ui: &mut egui::Ui) {
        glass(Surface::Panel)
            .inner_margin(egui::Margin::same(SPACE_S as i8))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0., SPACE_XS);
                let w = ui.available_width();
                ui.set_width(w);
                // The dock's header as it ships: the tree's columns (a preset
                // or the user's own set), then the filter with its button.
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = SPACE_XS;
                    let presets = ["Filing", "Drawing", "Export"];
                    if dropdown(ui, "Columns", presets[self.dock_tab % 3], false, w - 2. * CONTROL_H).clicked() {
                        self.dock_tab = (self.dock_tab + 1) % 3;
                    }
                    filter_button(ui, self.dock_tab == 1, false);
                    filter_button(ui, false, true);
                });
                inspector_title(ui, "Walls", "Tag · 24 items");
                tag_inspector(ui, "preview-tag:Walls", true, &mut Vec::new());
                ui.horizontal(|ui| {
                    for tool in ["outer_shell", "solid_union", "solid_subtract", "solid_trim", "solid_intersect", "solid_split"] {
                        let (rect, _) = ui.allocate_exact_size(vec2(CONTROL_H, CONTROL_H), Sense::hover());
                        solid_glyph(ui.painter(), rect.shrink(3.), tool, INK);
                    }
                });
                // One glyph per kind of tree row.
                {
                    let kinds = ["tag", "folder", "loose", "section", "scene", "sheet", "group", "component", "image", "link"];
                    let (r, _) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::hover());
                    for (i, kind) in kinds.iter().enumerate() {
                        row_glyph(ui.painter(), pos2(r.min.x + 12. + i as f32 * 20., r.center().y), kind, INK_2);
                    }
                }
                host_visibility(ui,"Plaza.built",&[("Survey".into(),"Survey".into(),true)], &mut Vec::new());
                path_bar(ui, &[("Plaza.built".into(), String::new()), ("Core block".into(), String::new())], &mut Vec::new());
                scene_preview_row(ui,"Ground floor",true,None);
                disclosure(ui,"gallery-piece","Masonry piece",false,|ui|piece_sample(ui,"block",&[390.,140.,190.,10.]));
                caption(ui, "Line samples");
                {
                    let (r, _) = ui.allocate_exact_size(vec2(w.min(160.), 14.), Sense::hover());
                    line_sample(ui, r.with_max_x(r.center().x), "0.35", "dashed", INK);
                    line_type_preview(ui,r.with_min_x(r.center().x),&[4., -1.], "GAS", 2.5, INK);
                }
                ui.add(search(&mut self.dock_filter, "Filter").desired_width(w));
                ui.spacing_mut().item_spacing = vec2(0., 0.);
                // (name, depth, expanded, selected, eye, lock, count, state dot, note)
                let rows: [(&str, usize, Option<bool>, bool, i8, i8, &str, Option<Color32>, &str); 7] = [
                    ("Ride.igz", 0, Some(false), false, 1, 0, "48", Some(FOUND), ""),
                    ("Studio", 0, Some(true), false, 1, 0, "12", None, ""),
                    ("Walls", 1, None, true, 1, 0, "6", None, ""),
                    ("Roof", 1, Some(true), false, 1, 1, "4", None, ""),
                    ("Rafter", 2, None, false, 1, 2, "", None, ""),
                    ("Furniture", 1, None, false, 0, 0, "2", None, ""),
                    ("Survey.igz", 0, None, false, -1, -1, "", Some(MISSING), "via Ride"),
                ];
                tree(ui, "gallery tree", |ui| for (name, depth, expanded, selected, eye, lock, count, state, note) in rows {
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
                    tree_slots(ui, r.right() - 8., r.center().y, eye, lock, resp.hovered() || eye != 1 || lock >= 1 || selected);
                    ui.painter().text(r.right_center() - vec2(8., 0.), Align2::RIGHT_CENTER, count, meta(), DIM);
                });
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
                rename_preview(ui, "Before", "After", &["Wall".into(), "W01".into(), "Door".into(), "Door".into()]);
                property_control(ui, "Hatch", |ui| {
                    let (sample, _) = ui.allocate_exact_size(vec2(44., 16.), Sense::hover());
                    hatch_sample(ui, sample, "brick", None);
                    let (sample, _) = ui.allocate_exact_size(vec2(44., 16.), Sense::hover());
                    hatch_swatch(ui,sample,"crosshatch",None,FIELD,INK);
                });
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(SPACE_XS, 0.);
                    ui.add_space(SPACE_S);
                    chip_button(ui, "Measure");
                    amber_button(ui, "2 issues · Show");
                });
                ui.add_space(SPACE_XS);
                disclosure(ui,"gallery-bevel","Bevel profile",false,|ui| {
                    let id = ui.make_persistent_id("bevel fixture");
                    let (mut editor, mut curve) = ui.data(|d| d.get_temp::<(CurveEditor, crate::inspector::bevel::Curve)>(id))
                        .unwrap_or_else(|| (Default::default(),serde_json::from_value(serde_json::json!(
                            [[1.,0.,false],[0.7,0.7,true],[0.,1.,false]])).unwrap()));
                    let outline: Vec<_> = curve.points().iter().map(|p| [p.0,p.1]).collect();
                    editor.show(ui,&mut curve,&outline);
                    ui.data_mut(|d|d.insert_temp(id,(editor,curve)));
                });
                material_readout(ui, "Material", super::SWATCH_FALLBACK, "#808080");
                let id = ui.make_persistent_id("gallery-export");
                let mut open = ui.data_mut(|d| *d.get_persisted_mut_or(id, true));
                if disclosure_toggle(ui, "Export details", open) {
                    open = !open;
                    ui.data_mut(|d| d.insert_persisted(id, open));
                }
                if open {
                    prop_row(ui, "IFC class", "IfcWall");
                    prop_row(ui, "Predefined type", "STANDARD");
                    prop_row(ui, "Uniclass", "EF_25_10 Walls");
                }
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
        drawer_frame().show(ui, |ui| {
            let tabs = ["Render", "Console", "Diagnostics"].map(|title| (title.into(), String::new(), true));
            inline_underline_tabs(ui, &tabs, 0);
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
                let (mark, ink) = notice_mark("success");
                ui.colored_label(ink, mark);
                ui.label(egui::RichText::new("Drawings exported").font(strong(TYPE_BODY)).color(INK));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    quiet(ui, "Show");
                });
            });
            ui.label(egui::RichText::new("12 pages to Deliveries/Handover").font(meta()).color(DIM));
        });
        // Agent tasks (Tasks destination): rows, a state chip, a pin and
        // the setup command Settings › AI shows.
        caption(ui, "Needs attention");
        task_row(ui, "Align the backrest slats", "Codex · 3 changes", "To review", "warn", true);
        task_row(ui, "Check clearance behind the bench", "Claude Code · no change", "Question", "warn", false);
        task_row(ui, "Check sheet callouts", "Built-in assistant · no change", "Working", "dim", false);
        status_chip(ui, "Done", task_tone("ok"));
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.), egui::Sense::hover());
        task_pin(ui, r.center_bottom() - vec2(0., 2.), true, false, 3, r.right());
        code_block(ui, "claude mcp add built -- \"/Applications/Built.app/Contents/MacOS/Built\" --mcp");
    }
}

#[cfg(test)]
mod coverage {
    /// Kit functions that are not components drawn on their own: the style
    /// installer, state helpers every molecule uses, and a variant of a
    /// component the gallery already shows.
    const NOT_COMPONENTS: [&str; 6] = [
        "popover_pos",   // where a menu opens, not something drawn
        "install_style", // style, not a component
        "state_of",      // reads a response's state for every molecule
        "paint_state",   // paints that state under every molecule
        "ink_of",        // the ink for a state
        "menu_row_resp", // menu_row, returning its response
    ];

    /// The kit's public functions at module level (not methods): every
    /// `pub fn` whose enclosing blocks are all modules.
    fn kit_components() -> Vec<String> {
        let src = include_str!("kit.rs");
        let mut stack: Vec<bool> = Vec::new(); // true: a module block
        let mut out = Vec::new();
        for line in src.lines() {
            let code = line.split("//").next().unwrap_or("");
            let t = code.trim_start();
            if stack.iter().all(|m| *m) {
                if let Some(rest) = t.strip_prefix("pub fn ") {
                    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                    if !name.is_empty() { out.push(name); }
                }
            }
            let module = t.starts_with("pub mod ") || t.starts_with("mod ");
            let mut first = true;
            for ch in code.chars() {
                match ch {
                    '{' => { stack.push(module && first); first = false; }
                    '}' => { stack.pop(); }
                    _ => {}
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    #[test]
    fn a_short_window_scrolls_the_full_size_board() {
        let mut raw = egui::RawInput::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600., 800.));
        assert_eq!(super::Gallery::fit(&mut raw, screen), 1.);
    }

    #[test]
    fn every_public_kit_component_is_in_the_gallery() {
        let gallery = include_str!("gallery.rs");
        let gallery = &gallery[..gallery.find("mod coverage").unwrap()];
        let found = kit_components();
        assert!(found.len() > 40 && found.contains(&"sheet_frame".to_owned()) && found.contains(&"value_pill".to_owned()), "{found:?}");
        let missing: Vec<&String> = found.iter()
            .filter(|name| !NOT_COMPONENTS.contains(&name.as_str()))
            .filter(|name| !gallery.contains(&format!("{name}(")))
            .collect();
        assert!(missing.is_empty(), "public kit components missing from the gallery (ui/gallery.rs): {missing:?}");
    }

    #[test]
    fn methods_are_not_components() {
        let found = kit_components();
        for method in ["corner_radius", "stroke", "type_filter", "show", "popup"] {
            assert!(!found.contains(&method.to_owned()), "{method}");
        }
    }
}
