//! The Project tree as a table (tree C, docs/ui-decisions.md 2026-11-23):
//! one column layout per column set, shared by the header and every row so
//! they line up; a minimum name width, so columns drop out before a name
//! is crushed; and the typed file row's icon and state dot (tree A).

use egui::{pos2, vec2, Color32, Rect, Stroke};

use super::{FOUND, MISSING, ACCENT, DIM, EDGE, FAINT, WARN};

/// A name keeps at least this much room (points) before a column hides.
pub const MIN_NAME: f32 = 120.0;
/// Indent per depth and the room the triangle and an icon take, as the rows draw them.
const INDENT: f32 = 14.0;
const LEAD: f32 = 4.0 + 16.0 + 16.0;
/// The depth a layout is planned for: deeper rows may still cut their names.
const PLAN_DEPTH: f32 = 2.0;

/// A file found / changed on disk / not found / packed: the row's dot.

/// Where each column of a column set sits in a row `[left, right]`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Cols {
    pub colset: u8,
    /// The name's right end.
    pub name_right: f32,
    // Drawing: print, weight, line type (may hide), colour swatch.
    pub print: Option<(f32, f32)>,
    pub weight: Option<(f32, f32)>,
    pub line: Option<(f32, f32)>,
    pub swatch: Option<(f32, f32)>,
    // Export: class (and type), quantity.
    pub class: Option<(f32, f32)>,
    pub qty: Option<(f32, f32)>,
    // Filing: the item count's right edge.
    pub count_right: f32,
}

impl Cols {
    /// The layout for a row spanning `left..right`.
    pub fn plan(colset: u8, left: f32, right: f32) -> Cols {
        let edge = right - 6.0;
        let room = |name_right: f32| name_right - (left + LEAD + PLAN_DEPTH * INDENT);
        let mut c = Cols { colset, name_right: right - 8.0, count_right: edge - 40.0, ..Default::default() };
        match colset {
            1 => {
                let print = (edge - 26.0, edge);
                let weight = (print.0 - 40.0, print.0 - 4.0);
                let line = (weight.0 - 64.0, weight.0 - 6.0);
                let with_line = room(line.0 - 28.0) >= MIN_NAME;
                let before = if with_line { line.0 } else { weight.0 };
                let swatch = (before - 22.0, before - 8.0);
                c.print = Some(print);
                c.weight = Some(weight);
                c.line = with_line.then_some(line);
                c.swatch = Some(swatch);
                c.name_right = swatch.0 - 8.0;
            }
            2 => {
                let qty = (edge - 64.0, edge);
                let wide = room(qty.0 - 108.0) >= MIN_NAME;
                let class = (qty.0 - if wide { 100.0 } else { 70.0 }, qty.0 - 8.0);
                c.qty = Some(qty);
                c.class = Some(class);
                c.name_right = class.0 - 8.0;
            }
            _ => {}
        }
        c
    }

    /// The header row: "Name" over the names, each column's title over it.
    pub fn header(&self, ui: &mut egui::Ui, left: f32) {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), egui::Sense::hover());
        let p = ui.painter();
        let cy = r.center().y;
        let font = egui::FontId::proportional(10.0);
        let at = |x: f32, text: &str, align: egui::Align2| {
            p.text(pos2(x, cy), align, text, font.clone(), FAINT);
        };
        at(left + 8.0, "Name", egui::Align2::LEFT_CENTER);
        match self.colset {
            1 => {
                if let Some((_, b)) = self.print { at(b, "Print", egui::Align2::RIGHT_CENTER); }
                if let Some((_, b)) = self.weight { at(b, "Wt", egui::Align2::RIGHT_CENTER); }
                if let Some((_, b)) = self.line { at(b, "Line", egui::Align2::RIGHT_CENTER); }
                if let Some((a, b)) = self.swatch { at((a + b) / 2.0, "Col.", egui::Align2::CENTER_CENTER); }
            }
            2 => {
                if let Some((a, _)) = self.class { at(a, "Class", egui::Align2::LEFT_CENTER); }
                if let Some((_, b)) = self.qty { at(b, "Quantity", egui::Align2::RIGHT_CENTER); }
            }
            _ => at(self.count_right, "Items", egui::Align2::RIGHT_CENTER),
        }
        p.hline(r.x_range(), r.max.y - 0.5, Stroke::new(1.0_f32, EDGE));
    }
}

/// A linked file's row icon: a page with a folded corner.
pub fn paint_file(painter: &egui::Painter, c: egui::Pos2, ink: Color32) {
    let r = Rect::from_center_size(c, vec2(9.0, 11.0));
    let fold = 3.0;
    let pts = vec![
        r.left_top(), pos2(r.max.x - fold, r.min.y), pos2(r.max.x, r.min.y + fold), r.right_bottom(), r.left_bottom(),
        r.left_top(),
    ];
    painter.add(egui::Shape::line(pts, Stroke::new(1.1_f32, ink)));
    painter.line_segment([pos2(r.max.x - fold, r.min.y), pos2(r.max.x - fold, r.min.y + fold)], Stroke::new(1.0_f32, ink));
    painter.line_segment([pos2(r.max.x - fold, r.min.y + fold), pos2(r.max.x, r.min.y + fold)], Stroke::new(1.0_f32, ink));
}

/// The file's state dot: found, changed on disk, not found, packed.
pub fn state_color(state: &str) -> Color32 {
    match state {
        "resolved" => FOUND,
        "modified" => WARN,
        "missing" => MISSING,
        _ => DIM,
    }
}

/// The quiet mark on a row above the one holding the selection.
pub fn paint_partial(painter: &egui::Painter, c: egui::Pos2) {
    painter.circle_filled(c, 2.5, ACCENT);
}

/// The file state from a row's icon ("file:<state>"), if it is a file row.
pub fn file_state(icon: &str) -> Option<&str> {
    icon.strip_prefix("file:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawing_hides_the_line_column_before_the_name_is_crushed() {
        let narrow = Cols::plan(1, 0.0, 280.0);
        let wide = Cols::plan(1, 0.0, 370.0);
        assert!(narrow.line.is_none() && wide.line.is_some());
        for c in [narrow, wide] {
            assert!(c.name_right - (LEAD + PLAN_DEPTH * INDENT) >= MIN_NAME - 30.0, "{c:?}");
        }
    }

    #[test]
    fn export_narrows_its_class_column_on_a_narrow_dock() {
        let narrow = Cols::plan(2, 0.0, 280.0);
        let wide = Cols::plan(2, 0.0, 370.0);
        let w = |c: Cols| c.class.map_or(0.0, |(a, b)| b - a);
        assert!(w(narrow) < w(wide));
        assert!(wide.name_right - (LEAD + PLAN_DEPTH * INDENT) >= MIN_NAME);
    }
}
