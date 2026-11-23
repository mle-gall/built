//! The tool rail's layout (review council B10, 2026-11-23): decided from the
//! band's height alone, the same way in the model and Sheets workspaces and
//! in both hosts.
//!
//! - Two columns when every row fits between the bars.
//! - Three columns when two do not.
//! - Only then three columns that scroll, as a last resort.
//!
//! A tool's place is its index in its group (row `i / cols`, column
//! `i % cols`), so a tool never moves for another one, and the only change a
//! window size can make is two columns or three.
use super::tokens::{EDGE, GUT, SPACE_XS};
use super::{kit::frame, tool_button, Tool};
use egui::{pos2, vec2, CornerRadius, Margin, Sense, Stroke};

/// A tool button's cell (`tool_button`).
pub const CELL: f32 = 30.0;
/// Between cells, across and down.
pub const GAP: f32 = SPACE_XS;
/// The rule between two groups.
pub const RULE: f32 = 7.0;
/// The rail's inner margin.
pub const PAD: f32 = 4.0;

/// How the rail is laid out for one band height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RailPlan {
    pub cols: usize,
    /// The content is taller than the band even in three columns.
    pub scroll: bool,
}

/// The rail's outer height with `cols` columns: rows and rules, the gaps
/// between them and the margin. `groups` holds each group's tool count.
pub fn height(groups: &[usize], cols: usize) -> f32 {
    let rows: usize = groups.iter().map(|n| n.div_ceil(cols)).sum();
    let rules = groups.len().saturating_sub(1);
    let items = rows + rules;
    rows as f32 * CELL + rules as f32 * RULE + items.saturating_sub(1) as f32 * GAP + 2.0 * PAD
}

/// The rail's outer width with `cols` columns.
pub fn width(cols: usize) -> f32 {
    cols as f32 * CELL + cols.saturating_sub(1) as f32 * GAP + 2.0 * PAD
}

/// The width of a group rule (the content width).
pub fn rule_width(cols: usize) -> f32 {
    width(cols) - 2.0 * PAD
}

/// Two columns, then three, then three that scroll.
pub fn plan(groups: &[usize], room: f32) -> RailPlan {
    for cols in [2, 3] {
        if height(groups, cols) <= room {
            return RailPlan { cols, scroll: false };
        }
    }
    RailPlan { cols: 3, scroll: true }
}

/// Draw a rail of `groups` in the area `id`, its top at `top` and `room`
/// high, slid left by `retract` (0 shown, 1 gone). Returns the clicked
/// tool's id and the rail's outer width.
pub(super) fn show(ctx: &egui::Context, id: &str, groups: &[Vec<Tool>], active: &str,
                   top: f32, room: f32, retract: f32) -> (Option<String>, f32) {
    let counts: Vec<usize> = groups.iter().map(Vec::len).collect();
    let RailPlan { cols, scroll } = plan(&counts, room);
    let outer = width(cols);
    let mut clicked = None;
    let tools = |ui: &mut egui::Ui, clicked: &mut Option<String>| {
        ui.spacing_mut().item_spacing = vec2(GAP, GAP);
        for (gi, group) in groups.iter().enumerate() {
            if gi > 0 {
                let (r, _) = ui.allocate_exact_size(vec2(rule_width(cols), RULE), Sense::hover());
                ui.painter().hline(r.x_range().shrink(2.0), r.center().y, Stroke::new(1.0_f32, EDGE));
            }
            for row in group.chunks(cols) {
                ui.horizontal(|ui| {
                    for tool in row {
                        if tool_button(ui, tool, tool.id == active) {
                            *clicked = Some(tool.id.clone());
                        }
                    }
                });
            }
        }
    };
    egui::Area::new(egui::Id::new(id))
        .constrain(false)
        .interactable(retract == 0.0)
        .fixed_pos(pos2(GUT - retract * (outer + 2.0 * GUT), top))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            frame().corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(PAD as i8)).show(ui, |ui| {
                // Docked full height: the rail always reaches the bottom bar.
                ui.set_min_height(room - 2.0 * PAD);
                ui.set_width(rule_width(cols));
                if scroll {
                    egui::ScrollArea::vertical()
                        .id_salt((id, "scroll"))
                        .max_height(room - 2.0 * PAD)
                        .show(ui, |ui| tools(ui, &mut clicked));
                } else {
                    tools(ui, &mut clicked);
                }
            });
        });
    (clicked, outer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::tokens::{BOT_H, GUT, TOP_H};

    /// The model rail (`commands::rail_layout`): select, draw, modify,
    /// measure and annotate, solids.
    const MODEL: [usize; 5] = [5, 10, 10, 8, 6];

    fn band(window_h: f32) -> f32 {
        window_h - TOP_H - BOT_H - 2.0 * GUT
    }

    #[test]
    fn heights_add_up_like_the_layout() {
        // Two rows of a group of three, a rule, one row: 3 rows, 1 rule,
        // three gaps, the margin.
        assert_eq!(height(&[3, 1], 2), 3.0 * CELL + RULE + 3.0 * GAP + 2.0 * PAD);
        assert_eq!(width(2), 2.0 * CELL + GAP + 2.0 * PAD);
    }

    #[test]
    fn short_windows_go_to_three_columns_then_scroll() {
        assert_eq!(plan(&MODEL, band(1000.0)), RailPlan { cols: 2, scroll: false });
        let at_800 = plan(&MODEL, band(800.0));
        assert_eq!(at_800.cols, 3);
        assert!(height(&MODEL, at_800.cols) <= band(800.0));
        assert_eq!(plan(&MODEL, band(300.0)), RailPlan { cols: 3, scroll: true });
    }

    #[test]
    fn the_plan_only_depends_on_the_height() {
        // The same height gives the same layout; growing never adds columns.
        let mut last = 3;
        for h in (300..1400).step_by(10) {
            let p = plan(&MODEL, band(h as f32));
            assert_eq!(p, plan(&MODEL, band(h as f32)));
            assert!(p.cols <= last);
            last = p.cols;
        }
    }

    /// Lay the rail out in a real egui frame of `h` points and return its rect.
    fn laid_out(h: f32) -> egui::Rect {
        let groups: Vec<Vec<Tool>> = MODEL.iter().enumerate().map(|(g, &n)| (0..n).map(|i| Tool {
            enabled: true, id: format!("t{g}.{i}"), label: String::new(), key: String::new(), icon: None,
        }).collect()).collect();
        let ctx = egui::Context::default();
        let top = TOP_H + GUT;
        for _ in 0..2 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, h))),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| { show(ctx, "palette", &groups, "", top, band(h), 0.0); });
        }
        ctx.memory(|m| m.area_rect(egui::Id::new("palette"))).unwrap()
    }

    #[test]
    fn the_rail_ends_above_the_bottom_bar() {
        for h in [500.0, 700.0, 800.0, 900.0, 1100.0] {
            let r = laid_out(h);
            assert!(r.max.y <= h - BOT_H - GUT + 0.5, "{h}: rail ends at {}", r.max.y);
            let cols = plan(&MODEL, band(h)).cols;
            assert!((r.width() - width(cols)).abs() < 0.5, "{h}: {} wide", r.width());
        }
    }
}
