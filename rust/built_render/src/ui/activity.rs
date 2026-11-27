use super::*;

// ID, title, detail, tone, action label and toast opacity.
pub type ActivityRow = (String, String, String, String, String, f32);

impl Ui {
    pub fn set_activity(
        &mut self,
        history: Vec<ActivityRow>,
        banner: String,
        secondary: std::collections::HashMap<String, String>,
    ) {
        self.activity_secondary = secondary;
        self.history = history;
        self.link_banner = banner;
    }

    fn activity_row(
        ui: &mut egui::Ui,
        row: &ActivityRow,
        second: Option<&String>,
        compact: bool,
        actions: &mut Vec<String>,
    ) {
        let (id, title, detail, tone, label, _) = row;
        let (mark, color) = notice_mark(tone);
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(color, mark);
            ui.label(egui::RichText::new(title).strong().color(color));
            if compact && quiet(ui, "×").on_hover_text(super::tr("Dismiss")).clicked() {
                actions.push(format!("dismiss:{id}"));
            }
        });
        if !label.is_empty() || second.is_some() {
            ui.horizontal_wrapped(|ui| {
                if !label.is_empty() && ui.small_button(label).clicked() {
                    actions.push(format!("activity:{id}"));
                }
                if let Some(label) = second {
                    if ui.small_button(label).clicked() {
                        actions.push(format!("activity2:{id}"));
                    }
                }
            });
        }
        if !detail.is_empty() {
            let text = egui::RichText::new(detail).font(body()).color(DIM);
            if compact {
                ui.add(egui::Label::new(text).truncate());
            } else {
                ui.label(text);
            }
        }
    }

    pub fn open_activity(&mut self) {
        self.activity_open = true;
    }

    pub(super) fn notices(&mut self, ctx: &egui::Context, screen: Rect) {
        let mut hovered = None;
        if self.history.iter().any(|row| row.5 > 0.) {
            let right = if self.lsheet.open {
                screen.right()
            } else {
                self.free_right(ctx, screen)
            };
            let center = (right + GUT) / 2.;
            egui::Area::new("notices".into())
                .order(egui::Order::Foreground)
                .pivot(egui::Align2::CENTER_BOTTOM)
                .fixed_pos(pos2(center, screen.bottom() - BOT_H - SPACE_S))
                .fade_in(false)
                .show(ctx, |ui| {
                    ui.set_max_width((right - GUT).clamp(100., 440.));
                    for row in self.history.iter().filter(|r| r.5 > 0.).take(3) {
                        let response = ui
                            .push_id(&row.0, |ui| {
                                toast(ui, row.5, |ui| {
                                    Self::activity_row(
                                        ui,
                                        row,
                                        self.activity_secondary.get(&row.0),
                                        true,
                                        &mut self.actions,
                                    )
                                });
                            })
                            .response;
                        if response.contains_pointer() {
                            hovered = Some(row.0.clone());
                        }
                    }
                });
        }
        if hovered != self.hovered_toast {
            if let Some(id) = self.hovered_toast.take() {
                self.actions.push(format!("resume-toast:{id}"));
            }
            if let Some(id) = &hovered {
                self.actions.push(format!("pause-toast:{id}"));
            }
            self.hovered_toast = hovered;
        }
    }

    pub(super) fn jobs_chip(&mut self, ctx: &egui::Context, screen: Rect) {
        if self.jobs.is_empty() && !self.activity_open {
            return;
        }
        let left = self.top_groups.first().map_or(GUT, |r| r.max.x) + 5.0;
        let right = self
            .top_groups
            .get(1)
            .map_or(screen.right() - GUT, |r| r.min.x)
            - 5.0;
        let width = (right - left).clamp(110.0, 360.0);
        let x = (screen.center().x - width / 2.0).clamp(left, (right - width).max(left));
        egui::Area::new("jobs".into())
            .fixed_pos(pos2(x, 3.0))
            .fade_in(false)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.set_max_width(width);
                ui.set_max_height(screen.height() - TOP_H);
                if !self.jobs.is_empty() {
                    activity_capsule().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let count = self.jobs.len();
                            let label = if count > 1 {
                                format!("+{}", count - 1)
                            } else {
                                "▾".into()
                            };
                            if ui
                                .small_button(label)
                                .on_hover_text(super::tr("Activity history"))
                                .clicked()
                            {
                                self.activity_open = !self.activity_open;
                            }
                            if let Some((id, title, _, fraction, cancel)) = self.jobs.first() {
                                if *cancel && ui.small_button("×").on_hover_text(super::tr("Cancel")).clicked()
                                {
                                    self.actions.push(format!("job:cancel:{id}"));
                                }
                                if *fraction < 0.0 {
                                    ring(ui);
                                }
                                ui.add(egui::Label::new(title).truncate())
                                    .on_hover_text(title);
                            }
                        });
                        if let Some((_, _, phase, fraction, _)) = self.jobs.first() {
                            if !phase.is_empty() {
                                ui.add(
                                    egui::Label::new(egui::RichText::new(phase).small()).truncate(),
                                )
                                .on_hover_text(phase);
                            }
                            if *fraction >= 0.0 {
                                ui.add_space(SPACE_XS);
                                progress_bar(ui, *fraction, 3.0);
                            }
                        }
                    });
                }
                if self.activity_open {
                    popover().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.strong(super::tr("Recent activity"));
                            if quiet(ui, "×").on_hover_text(super::tr("Close")).clicked() {
                                self.activity_open = false;
                            }
                        });
                        if self.history.is_empty() && self.jobs.is_empty() {
                            ui.label(super::tr("No recent activity"));
                        }
                        egui::ScrollArea::vertical()
                            .min_scrolled_height(120.0)
                            .max_height(screen.height() * 0.55)
                            .show(ui, |ui| {
                                for (id, title, phase, fraction, cancel) in &self.jobs {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(title);
                                        ui.label(phase);
                                        if *cancel && ui.small_button("×").clicked() {
                                            self.actions.push(format!("job:cancel:{id}"));
                                        }
                                    });
                                    if *fraction >= 0.0 {
                                        ui.add(progress(*fraction).show_percentage());
                                    } else {
                                        ring(ui);
                                    }
                                }
                                for row in &self.history {
                                    ui.separator();
                                    Self::activity_row(
                                        ui,
                                        row,
                                        self.activity_secondary.get(&row.0),
                                        false,
                                        &mut self.actions,
                                    );
                                }
                            });
                    });
                }
            });
    }
}

#[test]
fn idle_history_has_no_job_chip() {
    let Some(mut ui) = test_chrome() else { return };
    ui.ctx = test_context();
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1600., 1000.));
    ui.history.push(("1".into(), "Saved".into(), "".into(), "success".into(), "".into(), 1.));
    let ctx = ui.ctx.clone();
    let _ = ctx.run(Default::default(), |c| ui.jobs_chip(c, screen));
    assert!(ui.area_rect("jobs").is_none());
    ui.open_activity();
    let _ = ctx.run(Default::default(), |c| ui.jobs_chip(c, screen));
    assert!(ui.area_rect("jobs").is_some());
}
