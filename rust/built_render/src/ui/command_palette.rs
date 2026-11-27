//! The command palette for the active modelling or Sheets workspace.
use super::*;

pub fn fit_box(cursor: Pos2, size: Vec2, field_height: f32, work: Rect, screen: Rect) -> Rect {
    fn axis(pos: f32, size: f32, lo: f32, hi: f32, outer_lo: f32, outer_hi: f32) -> f32 {
        let (lo, hi) = if size <= hi - lo {
            (lo, hi)
        } else {
            (outer_lo, outer_hi)
        };
        pos.clamp(lo, (hi - size).max(lo))
    }
    let size = size.min(screen.size());
    let x = axis(
        cursor.x - size.x / 2.,
        size.x,
        work.left(),
        work.right(),
        screen.left(),
        screen.right(),
    );
    let y = axis(
        cursor.y - field_height / 2.,
        size.y,
        work.top(),
        work.bottom(),
        screen.top(),
        screen.bottom(),
    );
    Rect::from_min_size(pos2(x, y), size)
}

impl Ui {
    pub fn start_commands(&mut self, text: String, scope: String) {
        self.cmd_open = true;
        self.cmd_results.clear();
        self.cmd_extra.clear();
        self.cmd_anchor = self.ctx.pointer_latest_pos();
        self.cmd_at = 0;
        self.cmd_focus = true;
        self.actions.push(format!("cmdscope:{scope}"));
        self.actions.push(format!("cmdq:{text}"));
        self.cmd_scope = scope;
        self.cmd_query = text;
    }

    pub fn command_details(&mut self, details: Vec<(String, bool, bool)>) {
        self.cmd_extra = details;
    }

    /// ⌘K / F3: every command of the window by name, at the top of the model.
    pub(super) fn command_palette(&mut self, ctx: &egui::Context, screen: Rect) {
        if !self.cmd_open {
            return;
        }
        let mut run: Option<usize> = None;
        let mut changed = false;
        let mut change_shortcut = None;
        let bounds = {
            let work = Rect::from_min_max(
                pos2(GUT + 2. * CONTROL_H, TOP_H),
                pos2(self.free_right(ctx, screen), screen.bottom() - BOT_H),
            );
            fit_box(
                self.cmd_anchor.unwrap_or(work.center()),
                vec2(560., (CONTROL_H + 1.) * 9. + 85.),
                34.,
                work,
                screen,
            )
        };
        egui::Area::new(egui::Id::new("commands"))
            .fixed_pos(bounds.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                glass(Surface::Raised)
                    .inner_margin(Margin::same(8))
                    .show(ui, |ui| {
                        let width = (bounds.width() - 18.).max(1.);
                        ui.set_width(width);
                        if !self.cmd_scope.is_empty() {
                            ui.label(
                                egui::RichText::new(&self.cmd_scope)
                                    .font(meta())
                                    .color(FAINT),
                            );
                        }
                        let r = ui.add(
                            kit::field(&mut self.cmd_query)
                                .hint_text(egui::RichText::new(if self.cmd_scope=="Sweep profiles" {"Search profiles"}else{"Search commands"}).color(FAINT))
                                .font(body())
                                .margin(Margin::symmetric(10, 7))
                                .desired_width((width - 24.).max(1.))
                                .clip_text(true),
                        );
                        if self.cmd_focus {
                            r.request_focus();
                            self.cmd_focus = false;
                        }
                        if r.changed() {
                            changed = true;
                            self.cmd_at = 0;
                        }
                        ui.add_space(SPACE_S);
                        ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                        let rows_height = ((CONTROL_H + 1.) * 9.).min((bounds.height() - 85.).max(CONTROL_H));
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .min_scrolled_height(rows_height)
                            .max_height(rows_height)
                            .show(ui, |ui| {
                                if self.cmd_results.is_empty() && !self.cmd_query.is_empty() {
                                    ui.label(
                                        egui::RichText::new("No command matches")
                                            .font(body())
                                            .color(FAINT),
                                    );
                                }
                                for (i, (label, path, key, enabled)) in
                                    self.cmd_results.iter().enumerate()
                                {
                                    let detail = self.cmd_extra.get(i);
                                    let checked = detail.is_some_and(|d| d.1);
                                    let text = if checked {
                                        format!("✓ {label}")
                                    } else {
                                        label.clone()
                                    };
                                    let row = command_row(
                                        ui,
                                        &text,
                                        path,
                                        key,
                                        *enabled,
                                        self.cmd_at == i,
                                    );
                                    let row = row.on_hover_ui(|ui| {
                                        ui.set_max_width(300.);
                                        ui.label(
                                            egui::RichText::new(label)
                                                .font(title_font())
                                                .color(INK),
                                        );
                                        if let Some((tip, _, _)) =
                                            detail.filter(|d| !d.0.is_empty())
                                        {
                                            ui.label(tip);
                                        }
                                        if !key.is_empty() {
                                            ui.label(
                                                egui::RichText::new(key).font(meta()).color(FAINT),
                                            );
                                        }
                                    });
                                    if detail.is_some_and(|d| d.2) {
                                        row.context_menu(|ui| {
                                            if quiet(ui, "Change shortcut…").clicked() {
                                                change_shortcut = Some(i);
                                                ui.close_menu();
                                            }
                                        });
                                    }
                                    if self.cmd_scroll && i == self.cmd_at {
                                        row.scroll_to_me(None);
                                    }
                                    if *enabled && row.clicked() {
                                        run = Some(i);
                                    }
                                    if *enabled
                                        && row.hovered()
                                        && ui.input(|i| i.pointer.delta() != Vec2::ZERO)
                                    {
                                        self.cmd_at = i;
                                    }
                                }
                            });
                        self.cmd_scroll = false;
                    });
            });
        if changed {
            self.actions.push(format!("cmdq:{}", self.cmd_query));
        }
        if let Some(i) = change_shortcut {
            self.cmd_open = false;
            self.actions.push(format!("cmdshortcut:{i}"));
        }
        if let Some(i) = run {
            self.cmd_open = false;
            self.actions.push(format!("cmdrun:{i}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_tracks_pointer_and_stays_between_the_docks_if_it_fits() {
        let work = Rect::from_min_size(pos2(300., 100.), vec2(1300., 800.));
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1920., 1040.));
        let size = vec2(560., 330.);
        let r = fit_box(pos2(900., 400.), size, 30., work, screen);
        assert_eq!((r.center().x, r.top()), (900., 385.));
        for at in [work.min, work.max, screen.min, screen.max] {
            let r = fit_box(at, size, 30., work, screen);
            assert!(work.contains_rect(r));
            assert_eq!(r.size(), size);
        }
        for work_size in [vec2(400., 250.), vec2(1300., 250.)] {
            let small = Rect::from_min_size(work.min, work_size);
            let r = fit_box(small.center(), size, 30., small, screen);
            assert!(screen.contains_rect(r));
            assert_eq!(r.size(), size);
            if work_size.x >= size.x {
                assert!(r.left() >= small.left() && r.right() <= small.right());
            }
        }
        let tiny = Rect::from_min_size(Pos2::ZERO, vec2(500., 300.));
        assert_eq!(fit_box(tiny.center(), size, 30., tiny, tiny), tiny);
    }

    #[test]
    fn keyboard_skips_disabled_rows_clears_scope_and_cancels_without_running() {
        let Some(mut chrome) = super::super::test_chrome() else { return };
        chrome.start_commands(String::new(), "Edit".into());
        chrome.key("Backspace", true, 0);
        assert!(chrome.cmd_scope.is_empty());
        chrome.set_command_results(vec![
            ("First".into(), "Edit".into(), "".into(), true),
            ("Disabled".into(), "Edit".into(), "".into(), false),
            ("Third".into(), "Edit".into(), "".into(), true),
        ]);
        chrome.actions.clear();
        chrome.key("ArrowDown", true, 0);
        chrome.key("Enter", true, 0);
        assert_eq!(chrome.actions, ["cmdrun:2"]);
        assert!(!chrome.cmd_open);
        chrome.open_commands();
        chrome.actions.clear();
        chrome.key("Escape", true, 0);
        assert!(!chrome.cmd_open && chrome.actions.is_empty());
    }
}
