//! A bounded point editor shared by the inspector and component gallery.
use super::*;
use crate::inspector::bevel::{Curve, Point};

#[derive(Default, Clone)]
pub struct CurveEditor {
    selected: Option<usize>,
    corner: bool,
}

impl CurveEditor {
    pub fn show(&mut self, ui: &mut egui::Ui, curve: &mut Curve, outline: &[[f64; 2]]) -> bool {
        if curve.points().len() < 2 {
            return false;
        }
        let mut changed = false;
        self.selected = self.selected.filter(|&i| curve.interior(i));
        let width = ui.available_width().min(180.);
        let (well, event) = ui.allocate_exact_size(vec2(width, width), Sense::click_and_drag());
        let r = well.shrink(SPACE_S);
        let position = |Point(x, y, _)| r.lerp_inside(vec2(x as f32, 1. - y as f32));
        ui.painter().rect_filled(well, R_CONTROL, WELL);
        for pair in outline.windows(2) {
            let screen = |p: [f64; 2]| position(Point(p[0], p[1], false));
            ui.painter()
                .line_segment([screen(pair[0]), screen(pair[1])], Stroke::new(1.5_f32, INK_2));
        }
        for (i, &p) in curve.points().iter().enumerate() {
            let ink = if self.selected == Some(i) {
                ACCENT
            } else if curve.interior(i) {
                INK
            } else {
                DIM
            };
            ui.painter().circle_filled(position(p), SPACE_XS, ink);
        }
        if let Some(pos) = event.interact_pointer_pos() {
            let x = f64::from((pos.x - r.left()) / r.width());
            let y = f64::from((r.bottom() - pos.y) / r.height());
            let hit = |pos: Pos2| {
                curve
                    .points()
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &p)| {
                        let d = position(p).distance(pos);
                        (d <= SPACE_S).then_some((i, d))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|v| v.0)
            };
            if event.secondary_clicked() {
                changed = hit(pos).is_some_and(|i| curve.remove(i));
                self.selected = None;
            } else if event.double_clicked() {
                if let Some(i) = hit(pos) {
                    let Point(px, py, smooth) = curve.points()[i];
                    changed = curve.set(i, Point(px, py, !smooth));
                    self.selected = curve.interior(i).then_some(i);
                } else {
                    let smooth = !self.corner ^ ui.input(|i| i.modifiers.shift);
                    self.selected = curve.insert(x, y, smooth);
                    changed = self.selected.is_some();
                }
            } else {
                if event.clicked() || event.drag_started() {
                    let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(pos);
                    self.selected = hit(origin).filter(|&i| curve.interior(i));
                }
                if event.dragged() {
                    if let Some(i) = self.selected {
                        changed = curve.set(i, Point(x, y, curve.points()[i].2));
                    }
                }
            }
        }
        event.on_hover_text(
            "Double-click to add a point or change its kind. Drag to move. Right-click to remove.",
        );
        if let Some(i) = segmented(ui, &["Smooth", "Corner"], self.corner as usize, width) {
            self.corner = i == 1;
        }
        ui.horizontal(|ui| {
            if quiet(ui, "Add point").clicked() {
                let i = self.selected.unwrap_or(0);
                let [a, b] = [curve.points()[i], curve.points()[i + 1]];
                self.selected = curve.insert((a.0 + b.0) / 2., (a.1 + b.1) / 2., !self.corner);
                changed = true;
            }
            if let Some(i) = self.selected {
                if quiet(ui, "Remove point").clicked() {
                    changed |= curve.remove(i);
                    self.selected = None;
                }
            }
        });
        if let Some(i) = self.selected {
            let mut p = curve.points()[i];
            ui.horizontal(|ui| {
                for (axis, n) in [("X", &mut p.0), ("Y", &mut p.1)] {
                    ui.label(axis);
                    ui.add(number(n, "").range(0.0..=1.0).speed(0.01));
                }
                ui.checkbox(&mut p.2, "Smooth");
            });
            changed |= curve.set(i, p);
        }
        changed
    }
}
