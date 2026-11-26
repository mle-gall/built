//! The model view's own overlay (UX pass 2, M03): the axis gizmo in the
//! free viewport's top-right corner, left of the dock and under the top
//! bar, and the quiet view label beside it ("Front · Parallel"). The label
//! opens the View menu's views. Everything the gizmo does goes back as
//! actions: `act:view_<name>` and `act:gizmo_orbit:dx,dy`.
use super::kit::view_gizmo::{inside, view_gizmo, GizmoEvent, GIZMO_R};
use super::*;

/// What the window last told the chrome about its active camera.
#[derive(Default)]
pub(super) struct Overlay {
    /// (yaw, pitch) in radians and the label text, or None before the
    /// first frame of a model view.
    view: Option<(f64, f64, String)>,
    /// Where the gizmo and its label were drawn last frame (hit tests).
    shown: Option<(Pos2, Rect)>,
    /// The View menu was opened from the label: views, projection and style only.
    pub(super) views_only: bool,
}

impl Overlay {
    /// Is `pos` on the gizmo's circle or its label? The rest of the
    /// gizmo's square is the model's.
    pub(super) fn takes(&self, pos: Pos2) -> bool {
        self.shown.is_some_and(|(centre, label)| inside(pos - centre) || label.contains(pos))
    }
}

/// The View menu's rows the label shows: views, projection and style.
pub(super) fn views_section(items: &[Item]) -> bool {
    items.iter().any(|i| ["view_", "projection_", "style_"].iter().any(|p| i.0.starts_with(p)))
}

/// The gizmo's centre and the label's right edge for a model area `free`.
pub(super) fn gizmo_centre(free: Rect) -> Pos2 {
    pos2(free.max.x - SPACE_M - GIZMO_R, free.min.y + SPACE_M + GIZMO_R)
}

impl Ui {
    /// The active camera's direction and the label to show beside the gizmo.
    pub fn set_view_state(&mut self, yaw: f64, pitch: f64, label: String) {
        self.gizmo.view = (yaw.is_finite() && pitch.is_finite()).then_some((yaw, pitch, label));
    }

    /// The part of the window the model is seen in, in logical points:
    /// [`Ui::free_ndc`] back in the chrome's coordinates.
    pub(super) fn model_rect(&self) -> Option<Rect> {
        let [x0, y0, x1, y1] = self.free_ndc()?;
        let s = self.screen;
        let x = |v: f64| (v as f32 + 1.0) / 2.0 * s.width();
        let y = |v: f64| (1.0 - v as f32) / 2.0 * s.height();
        Some(Rect::from_min_max(pos2(x(x0), y(y1)), pos2(x(x1), y(y0))))
    }

    /// Draw the gizmo and label for this frame and queue what they did.
    pub(super) fn view_overlay(&mut self, ctx: &egui::Context) {
        self.gizmo.shown = None;
        let Some((yaw, pitch, label)) = self.gizmo.view.clone() else { return };
        let Some(mut free) = self.model_rect() else { return };
        // In a layout of several views the gizmo belongs to the active one.
        if self.model_panes.len() > 1 {
            if let Some(&(_, pane, _, _)) = self.model_panes.iter().find(|p| p.2) {
                free = pane.intersect(free);
            }
        }
        if free.width() < 4. * GIZMO_R || free.height() < 3. * GIZMO_R {
            return;
        }
        let centre = gizmo_centre(free);
        let names = |view: &str| {
            let id = format!("view_{view}");
            self.view_menu.iter().flat_map(|(_, rows)| rows).find(|r| r.0 == id).map_or(view.to_owned(), |r| r.1.clone())
        };
        let mut event = None;
        let mut label_clicked = false;
        let mut label_rect = Rect::NOTHING;
        let area = Rect::from_center_size(centre, Vec2::splat(2. * GIZMO_R));
        egui::Area::new(egui::Id::new("view_gizmo"))
            .fixed_pos(area.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.set_min_size(area.size());
                event = view_gizmo(ui, centre, yaw, pitch, names);
                let galley = ui.painter().layout_no_wrap(label.clone(), meta(), DIM);
                let size = galley.size() + vec2(2. * SPACE_S, 8.);
                let rect = Rect::from_min_size(pos2(area.min.x - SPACE_XS - size.x, centre.y - size.y / 2.), size);
                let response = ui.interact(rect, ui.id().with("view label"), Sense::click());
                let open = self.view_open && self.gizmo.views_only;
                // A small graphite chip, like the chrome: it reads on a white
                // sky and on a dark background alike.
                ui.painter().rect_filled(rect, R_ROW, POPOVER);
                if response.hovered() || open {
                    ui.painter().rect_filled(rect, R_ROW, if open { PRESS } else { HOVER });
                }
                let ink = if response.hovered() || open { INK } else { INK_2 };
                ui.painter().galley_with_override_text_color(rect.min + vec2(SPACE_S, 4.), galley, ink);
                label_clicked = response.clicked();
                label_rect = rect;
            });
        self.gizmo.shown = Some((centre, label_rect));
        match event {
            Some(GizmoEvent::View(view)) => self.actions.push(format!("act:view_{view}")),
            Some(GizmoEvent::Orbit(d)) => {
                self.actions.push(format!("act:{}{},{}", crate::commands::GIZMO_ORBIT, d.x, d.y))
            }
            None => {}
        }
        if label_clicked {
            if self.view_open && self.gizmo.views_only {
                self.view_open = false;
            } else {
                if self.form.take().is_some() { self.actions.push("form:close".into()); }
                self.view_open = true;
                self.gizmo.views_only = true;
                self.view_anchor = label_rect;
                self.menu = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ui: &mut Ui, events: Vec<egui::Event>) {
        ui.events.extend(events);
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1600., 1000.));
        ui.screen = screen;
        let ctx = ui.ctx.clone();
        let _ = ctx.run(egui::RawInput { screen_rect: Some(screen), events: std::mem::take(&mut ui.events), ..Default::default() }, |ctx| {
            ui.view_overlay(ctx);
            ui.view_popup(ctx);
        });
    }

    fn click(ui: &mut Ui, pos: Pos2) {
        for kind in ["move", "press", "release"] {
            ui.pointer(kind, pos.x, pos.y, 1, 0);
            frame(ui, Vec::new());
        }
    }

    fn chrome() -> Option<Ui> {
        let mut ui = super::super::test_chrome()?;
        ui.ctx = super::super::test_context();
        let (cluster, menu) = crate::commands::command_chrome(Default::default(), Default::default(), Default::default());
        ui.set_cluster(cluster, menu);
        // A free perspective view.
        ui.set_view_state(-std::f64::consts::FRAC_PI_4, 0.5, "Perspective".into());
        frame(&mut ui, Vec::new());
        frame(&mut ui, Vec::new());
        Some(ui)
    }

    #[test]
    fn the_gizmo_sits_top_right_of_the_free_area_left_of_the_dock() {
        let Some(ui) = chrome() else { return };
        let (centre, label) = ui.gizmo.shown.unwrap();
        let free = ui.model_rect().unwrap();
        assert!(centre.x + GIZMO_R < free.max.x && centre.x + GIZMO_R > free.max.x - 2. * SPACE_M);
        assert!(centre.y - GIZMO_R > TOP_H);
        assert!(label.max.x < centre.x - GIZMO_R && (label.center().y - centre.y).abs() < 1.);
    }

    #[test]
    fn clicking_an_axis_end_asks_for_its_view_and_outside_clicks_pass_through() {
        let Some(mut ui) = chrome() else { return };
        let (centre, _) = ui.gizmo.shown.unwrap();
        let (yaw, pitch, _) = ui.gizmo.view.clone().unwrap();
        let top = super::super::kit::view_gizmo::layout(yaw, pitch)[4].1;
        click(&mut ui, centre + top);
        assert!(ui.take_actions().contains(&"act:view_top".to_owned()));
        // The square's corner outside the circle is the model's.
        let corner = centre + vec2(GIZMO_R - 3., GIZMO_R - 3.);
        assert!(!ui.over_chrome_at(corner.x, corner.y));
        assert!(!ui.pointer("press", corner.x, corner.y, 1, 0));
        ui.pointer("release", corner.x, corner.y, 1, 0);
        assert!(ui.over_chrome_at(centre.x, centre.y));
    }

    #[test]
    fn the_aligned_end_clicked_again_flips_to_the_opposite_view() {
        let Some(mut ui) = chrome() else { return };
        ui.set_view_state(-std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2, "Top · Parallel".into());
        frame(&mut ui, Vec::new());
        let (centre, _) = ui.gizmo.shown.unwrap();
        click(&mut ui, centre);
        assert!(ui.take_actions().contains(&"act:view_bottom".to_owned()));
    }

    #[test]
    fn a_drag_on_the_gizmo_orbits() {
        let Some(mut ui) = chrome() else { return };
        let (centre, _) = ui.gizmo.shown.unwrap();
        let from = centre + vec2(-GIZMO_R / 2., GIZMO_R / 2.);
        ui.pointer("move", from.x, from.y, 1, 0);
        frame(&mut ui, Vec::new());
        ui.pointer("press", from.x, from.y, 1, 0);
        frame(&mut ui, Vec::new());
        for i in 1..=4 {
            ui.pointer("move", from.x + 6. * i as f32, from.y, 1, 0);
            frame(&mut ui, Vec::new());
        }
        ui.pointer("release", from.x + 24., from.y, 1, 0);
        frame(&mut ui, Vec::new());
        let acts = ui.take_actions();
        assert!(acts.iter().any(|a| a.starts_with("act:gizmo_orbit:")), "{acts:?}");
        assert!(!acts.iter().any(|a| a.starts_with("act:view_")), "{acts:?}");
    }

    #[test]
    fn the_label_opens_the_views_of_the_view_menu() {
        let Some(mut ui) = chrome() else { return };
        let (_, label) = ui.gizmo.shown.unwrap();
        click(&mut ui, label.center());
        assert!(ui.view_open && ui.gizmo.views_only);
        assert!(views_section(&ui.view_menu.iter().find(|(t, _)| t == "Views").unwrap().1));
        assert!(!views_section(&ui.view_menu.iter().find(|(t, _)| t == "Display").unwrap().1));
    }
}
