use super::*;

// ID, title, detail, tone, action label and toast opacity (negative once the capsule result expires).
pub type ActivityRow = (String, String, String, String, String, f32);
pub(super) const ERROR: Color32 = Color32::from_rgb(235, 99, 99);

impl Ui {
    pub fn set_activity(&mut self, history: Vec<ActivityRow>, banner: String) {
        self.history = history;
        self.link_banner = banner;
    }

    fn activity_row(
        ui: &mut egui::Ui,
        row: &ActivityRow,
        compact: bool,
        actions: &mut Vec<String>,
    ) {
        let (id, title, detail, tone, label, _) = row;
        let color = match tone.as_str() {
            "error" => ERROR,
            "warning" => WARN,
            _ => INK,
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(title).strong().color(color));
            if !label.is_empty() && ui.small_button(label).clicked() {
                actions.push(format!("activity:{id}"));
            }
        });
        if !detail.is_empty() {
            let text = egui::RichText::new(detail).font(body()).color(DIM);
            if compact {
                ui.add(egui::Label::new(text).truncate());
            } else {
                ui.label(text);
            }
        }
    }

    pub(super) fn jobs_chip(&mut self, ctx: &egui::Context, screen: Rect) {
        if self.jobs.is_empty() && self.history.is_empty() {
            self.activity_open = false;
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
                glass(Surface::Pill)
                    .inner_margin(Margin::symmetric(8, 2))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let count = self.jobs.len() + self.history.len();
                            let label = if count > 1 {
                                format!("+{}", count - 1)
                            } else {
                                "▾".into()
                            };
                            if ui
                                .small_button(label)
                                .on_hover_text("Activity history")
                                .clicked()
                            {
                                self.activity_open = !self.activity_open;
                            }
                            if let Some((id, title, phase, fraction, cancel)) = self.jobs.first() {
                                if *cancel && ui.small_button("×").on_hover_text("Cancel").clicked()
                                {
                                    self.actions.push(format!("job:cancel:{id}"));
                                }
                                if *fraction < 0.0 {
                                    ui.spinner();
                                }
                                let text = if phase.is_empty() {
                                    title.clone()
                                } else {
                                    format!("{title} · {phase}")
                                };
                                ui.add(egui::Label::new(text).truncate());
                            } else {
                                let recent = &self.history[0];
                                let title = if recent.5 < 0.0 {
                                    "Recent activity"
                                } else {
                                    &recent.1
                                };
                                ui.add(egui::Label::new(title).truncate());
                            }
                        });
                        if let Some((_, _, _, fraction, _)) = self.jobs.first() {
                            if *fraction >= 0.0 {
                                ui.add(
                                    egui::ProgressBar::new(*fraction)
                                        .desired_height(3.0)
                                        .fill(ACCENT),
                                );
                            }
                        }
                    });
                if self.activity_open {
                    glass(Surface::Raised).show(ui, |ui| {
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
                                        ui.add(egui::ProgressBar::new(*fraction).show_percentage());
                                    } else {
                                        ui.spinner();
                                    }
                                }
                                for row in &self.history {
                                    ui.separator();
                                    Self::activity_row(ui, row, false, &mut self.actions);
                                }
                            });
                    });
                } else {
                    for row in self.history.iter().filter(|r| r.5 > 0.0).take(3) {
                        ui.scope(|ui| {
                            ui.set_opacity(row.5);
                            glass(Surface::Raised).show(ui, |ui| {
                                Self::activity_row(ui, row, true, &mut self.actions);
                            });
                        });
                    }
                }
            });
    }
}
