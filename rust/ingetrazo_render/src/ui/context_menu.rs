//! The viewport right-click menu (docs/context-menu.md; Python builds it in
//! views/context_menu.py). There is no More › and there are no submenus.
//! A header names what was hit and where it is filed. Each row is one verb,
//! or a family of verbs as segments ("Intersect: Model | Selection |
//! Context"). A click reports `ctx:<id>` and closes the menu.
//!
//! The menu opens at the pointer. It flips left when it would cover the
//! project dock, and it moves up to stay above the bottom bar.

use super::*;

/// One verb: (id, label, key hint, enabled, checked: -1 none / 0 / 1).
pub type Seg = (String, String, String, bool, i8);

/// A row: a family label ("" for a plain row) and its verbs.
pub type Row = (String, Vec<Seg>);

pub struct Menu {
    at: Pos2,
    title: String,
    subtitle: String,
    rows: Vec<Row>,
}

/// The width of the menu (points) and of a segmented row's family label.
const WIDTH: f32 = 272.0;
const FAMILY_W: f32 = 84.0;

impl Ui {
    /// Python's menu for the right-click at (x, y); no rows closes it.
    pub fn set_context_menu(&mut self, x: f32, y: f32, title: String, subtitle: String, rows: Vec<Row>) {
        self.ctx_menu = (!rows.is_empty()).then(|| Menu { at: pos2(x, y), title, subtitle, rows });
    }

    pub fn context_menu_open(&self) -> bool { self.ctx_menu.is_some() }

    pub fn close_context_menu(&mut self) { self.ctx_menu = None; }

    /// The row labels as drawn, for tests and harnesses ("Hide",
    /// "Intersect: Model | Selection | Context").
    pub fn context_menu_rows(&self) -> Vec<String> {
        self.ctx_menu.as_ref().map_or_else(Vec::new, |m| m.rows.iter().map(|(family, segs)| {
            if family.is_empty() {
                segs.first().map_or(String::new(), |s| s.1.clone())
            } else {
                format!("{family}: {}", segs.iter().map(|s| s.1.as_str()).collect::<Vec<_>>().join(" | "))
            }
        }).collect())
    }

    pub(super) fn context_menu(&mut self, ctx: &egui::Context, screen: Rect) {
        let Some(menu) = &self.ctx_menu else { return };
        // Last frame's size places it: flipped left off the dock, up off
        // the bottom bar, never over the top bar.
        let size = ctx.memory(|m| m.area_rect(egui::Id::new("ctx_menu"))).map_or(vec2(WIDTH + 12.0, 300.0), |r| r.size());
        let right = self.free_right(ctx, screen);
        let mut x = menu.at.x + 2.0;
        if x + size.x > right && menu.at.x - size.x - 2.0 > GUT {
            x = menu.at.x - size.x - 2.0;
        }
        let bottom = if self.review { screen.max.y - GUT } else { screen.max.y - BOT_H - GUT };
        let y = (menu.at.y + 2.0).min(bottom - size.y).max(TOP_H + GUT);
        let mut clicked: Option<String> = None;
        egui::Area::new(egui::Id::new("ctx_menu"))
            .fixed_pos(pos2(x, y))
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                glass(Surface::Raised).inner_margin(Margin::same(6)).show(ui, |ui| {
                    ui.set_width(WIDTH);
                    ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                    // The header: what was hit, and where it is filed.
                    ui.add_space(2.0);
                    let title = ui.painter().layout_no_wrap(menu.title.clone(), strong(12.0), INK);
                    let mut job = egui::text::LayoutJob::single_section(menu.subtitle.clone(), egui::TextFormat {
                        font_id: meta(), color: FAINT, ..Default::default() });
                    job.wrap = egui::text::TextWrapping { max_width: WIDTH - 24.0, max_rows: 1, break_anywhere: true,
                                                          overflow_character: Some('…') };
                    let sub = ui.painter().layout_job(job);
                    let h = title.size().y + if menu.subtitle.is_empty() { 0.0 } else { sub.size().y + 2.0 } + 8.0;
                    let (r, _) = ui.allocate_exact_size(vec2(WIDTH, h), Sense::hover());
                    ui.painter().galley(pos2(r.min.x + 12.0, r.min.y + 3.0), title.clone(), INK);
                    if !menu.subtitle.is_empty() {
                        ui.painter().galley(pos2(r.min.x + 12.0, r.min.y + 5.0 + title.size().y), sub, FAINT);
                    }
                    divider(ui);
                    for (i, (family, segs)) in menu.rows.iter().enumerate() {
                        if family == "-" { divider(ui); continue; }
                        if family.is_empty() && segs.len() == 1 {
                            let Some((id, label, key, enabled, checked)) = segs.first() else { continue };
                            let lane = if *checked >= 0 { "check" } else { "" };
                            let item = (id.clone(), label.clone(), key.clone(), lane.to_owned(), *checked, *enabled);
                            if menu_row(ui, &item) && *enabled { clicked = Some(id.clone()); }
                        } else if let Some(id) = segment_row(ui, family, segs) {
                            clicked = Some(id);
                        }
                        // Repeat stands apart from the verbs under it.
                        if i == 0 && menu.rows.len() > 1 && segs.first().is_some_and(|s| s.0 == "repeat") {
                            divider(ui);
                        }
                    }
                });
            });
        if let Some(id) = clicked {
            self.ctx_menu = None;
            self.actions.push(format!("ctx:{id}"));
        }
    }
}

fn divider(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 7.0), Sense::hover());
    ui.painter().hline(r.x_range().shrink(6.0), r.center().y, Stroke::new(1.0_f32, EDGE));
}

/// A family of verbs on one row: its name, then each verb as a segment
/// sharing the rest of the width. Returns the verb clicked.
fn segment_row(ui: &mut egui::Ui, family: &str, segs: &[Seg]) -> Option<String> {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    // A family ("Intersect") names the row and its segments are its
    // options, centred; without one ("Make group | Make component…") each
    // segment is a verb of its own, read from the left like a row.
    let plain = family.is_empty();
    if !plain {
        ui.painter().text(pos2(row.min.x + 12.0, row.center().y), egui::Align2::LEFT_CENTER, family, body(), DIM);
    }
    let left = row.min.x + if plain { 0.0 } else { FAMILY_W };
    let n = segs.len().max(1) as f32;
    let w = (row.max.x - left) / n;
    let mut out = None;
    for (i, (id, label, _key, enabled, checked)) in segs.iter().enumerate() {
        let r = Rect::from_min_size(pos2(left + w * i as f32, row.min.y), vec2(w, row.height())).shrink2(vec2(1.0, 0.0));
        let sense = if *enabled { Sense::click() } else { Sense::hover() };
        let resp = ui.interact(r, ui.id().with(("seg", id)), sense);
        let st = state_of(&resp, *checked == 1, *enabled);
        paint_state(ui, r, st, R_ROW);
        let ink = if !*enabled { OFF } else if *checked == 1 { Color32::WHITE } else { INK };
        let g = ui.painter().layout_no_wrap(label.clone(), body(), ink);
        let g = if g.size().x > r.width() - 6.0 {
            let mut job = egui::text::LayoutJob::single_section(label.clone(), egui::TextFormat {
                font_id: body(), color: ink, ..Default::default() });
            job.wrap = egui::text::TextWrapping { max_width: r.width() - 6.0, max_rows: 1, break_anywhere: true,
                                                  overflow_character: Some('…') };
            ui.painter().layout_job(job)
        } else { g };
        let at = if plain { pos2(r.min.x + 11.0, r.center().y - g.size().y / 2.0) } else { r.center() - g.size() / 2.0 };
        ui.painter().galley(at, g, ink);
        if i > 0 {
            ui.painter().vline(r.min.x - 1.0, r.y_range().shrink(6.0), Stroke::new(1.0_f32, EDGE));
        }
        if resp.clicked() && *enabled { out = Some(id.clone()); }
    }
    out
}
