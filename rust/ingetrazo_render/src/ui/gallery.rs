//! One-frame reference. Each column owns its sample values and input IDs.
use super::{kit::*, tokens::*};
use egui::{vec2, Align2, Color32, Rect, Sense};

#[derive(Default)]
struct Samples {
    name: String,
    filter: String,
    length: f64,
    choice: usize,
    toggle: bool,
    color: Option<Color32>,
}

#[derive(Default)]
pub(super) struct Gallery {
    samples: [Samples; 6],
}

impl Gallery {
    pub(super) fn show(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(SHEET).inner_margin(12))
            .show(ctx, |ui| {
                egui::ScrollArea::both().show(ui, |ui| {
                    ui.set_min_width(1110.);
                    ui.label(
                        egui::RichText::new("Built design system")
                            .font(heading())
                            .color(INK),
                    );
                    ui.add_space(SPACE[0]);
                    let width = (ui.available_width() / 10. - 3.).max(98.);
                    for row in SWATCHES.chunks(10) {
                        ui.horizontal(|ui| {
                            for &(name, color) in row {
                                let (r, _) =
                                    ui.allocate_exact_size(vec2(width, 18.), Sense::hover());
                                let sample = Rect::from_min_size(r.min, vec2(16., 16.));
                                ui.painter().rect_filled(sample, 3, color);
                                ui.painter().rect_stroke(
                                    sample,
                                    3,
                                    egui::Stroke::new(1.0_f32, EDGE),
                                    egui::StrokeKind::Inside,
                                );
                                ui.painter().text(
                                    r.left_center() + vec2(20., 0.),
                                    Align2::LEFT_CENTER,
                                    name.to_lowercase(),
                                    meta(),
                                    INK_2,
                                );
                            }
                        });
                    }
                    ui.add_space(SPACE[1]);
                    ui.columns(6, |columns| {
                        let states = [
                            ("Rest", State::Idle),
                            ("Hover", State::Hover),
                            ("Pressed", State::Press),
                            ("Selected", State::On),
                            ("Focus", State::Focus),
                            ("Disabled", State::Off),
                        ];
                        for (i, ((ui, (name, state)), sample)) in columns
                            .iter_mut()
                            .zip(states)
                            .zip(&mut self.samples)
                            .enumerate()
                        {
                            ui.push_id(i, |ui| {
                                ui.label(egui::RichText::new(name).font(title()).color(INK));
                                ui.add_enabled_ui(state != State::Off, |ui| {
                                    sample.controls(ui, state)
                                });
                            });
                        }
                    });
                    ui.add_space(SPACE[1]);
                    ui.columns(3, |cols| {
                        sheet_frame(
                            &mut cols[0],
                            "Library",
                            |ui| {
                                label(ui, "Materials");
                            },
                            |ui| {
                                badge(ui, "Oak", INK);
                            },
                            |ui| {
                                label(ui, "Detail");
                            },
                        );
                        activity_capsule().show(&mut cols[1], |ui| {
                            ui.horizontal(|ui| {
                                ring(ui);
                                label(ui, "Export drawings");
                            });
                            ui.add(progress(0.45).desired_height(4.));
                        });
                        toast_stack(
                            &mut cols[1],
                            [(&"Drawings exported", 1.)].into_iter(),
                            |ui, s| {
                                label(ui, s);
                            },
                        );
                        popover().show(&mut cols[2], |ui| {
                            caption_plain(ui, "File");
                            menu_row(
                                ui,
                                &(
                                    "".into(),
                                    "Show in tree".into(),
                                    "↵".into(),
                                    "".into(),
                                    -1,
                                    true,
                                ),
                            );
                        });
                    });
                });
            });
    }
}

impl Samples {
    fn controls(&mut self, ui: &mut egui::Ui, state: State) {
        let selected = state == State::On;
        let interactive = &ui.visuals().widgets;
        let mut visual = match state {
            State::Hover => interactive.hovered,
            State::Press | State::Focus => interactive.active,
            _ => interactive.inactive,
        };
        if state == State::Focus {
            visual.bg_stroke = egui::Stroke::new(1.5_f32, FOCUS_RING);
        }
        if selected {
            visual.bg_fill = ACTIVE;
            visual.weak_bg_fill = ACTIVE;
        }
        ui.visuals_mut().widgets.inactive = visual;
        egui::Frame::new()
            .fill(CARD)
            .corner_radius(R_CONTROL)
            .inner_margin(6)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                for (role, label) in [(1, "Create"), (0, "Cancel"), (2, "Discard")] {
                    sheet_button_sample(ui, label, role, state);
                }
                ui.horizontal(|ui| {
                    let _ = quiet(ui, "Details");
                    let _ = icon_button(ui, "×", "Close");
                    chip_button(ui, "Edit");
                });
                segmented(
                    ui,
                    &["Model", "Sheets"],
                    usize::from(selected),
                    ui.available_width(),
                );
                underline_tabs(
                    ui,
                    &[
                        ("Files".into(), "3".into(), true),
                        ("Links".into(), "".into(), true),
                    ],
                    usize::from(selected),
                );
                ui.add(
                    field(&mut self.name)
                        .hint_text("Name")
                        .desired_width(f32::INFINITY),
                );
                ui.add(search(&mut self.filter, "Search").desired_width(f32::INFINITY));
                ui.horizontal(|ui| {
                    ui.add(number(&mut self.length, "mm"));
                    swatch(ui, self.color.get_or_insert(OK));
                });
                choice(ui, "kind", &mut self.choice, &["Material", "Colour"]);
                toggle(ui, &mut self.toggle, "Visible");
                caption_plain(ui, "Project tree");
                tree(ui, "tree", |ui| {
                    let (r, _) = list_row(ui, 24., selected);
                    dot(ui.painter(), r.left_center() + vec2(5., 0.), FOUND);
                    ui.painter().text(
                        r.left_center() + vec2(12., 0.),
                        Align2::LEFT_CENTER,
                        "Walls",
                        body(),
                        INK,
                    );
                    tree_slots(ui, r.right(), r.center().y, 1, 0, true);
                    ui.painter()
                        .text(r.right_center(), Align2::RIGHT_CENTER, "3", meta(), DIM);
                });
                let (r, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 28.), Sense::hover());
                paint_card(ui, r, state == State::Hover, state == State::Focus);
                ui.painter().text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    "Material tile",
                    body(),
                    INK,
                );
                ui.horizontal(|ui| {
                    badge(ui, "Found", OK);
                    amber_button(ui, "Missing link");
                });
                empty_state(ui, "No matching files");
                paint_state(ui, r, state, R_ROW);
            });
    }
}
