// SPDX-License-Identifier: GPL-3.0-or-later
use egui::{pos2, vec2, Color32, Rect, Stroke};

use super::{ACCENT, CHANGED, EDGE, FAINT, FOUND, MISSING, PACKED};

/// A name keeps at least this much room (points) before a column hides.
pub const MIN_NAME: f32 = 120.0;
/// Indent per depth and the room the triangle and an icon take, as the rows draw them.
const INDENT: f32 = 14.0;
const LEAD: f32 = 4.0 + 16.0 + 16.0;
/// The depth a layout is planned for: deeper rows may still cut their names.
const PLAN_DEPTH: f32 = 2.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Lens {
    #[default]
    Filing,
    Drawing,
    Export,
}
impl Lens {
    pub fn from_index(index: u8) -> Self {
        match index {
            1 => Self::Drawing,
            2 => Self::Export,
            _ => Self::Filing,
        }
    }
}
/// Reserve room for the active task's controls before truncating the row label.
#[derive(Clone, Copy, Debug, Default)]
pub struct RowLayout {
    pub lens: Lens,
    pub flags: Option<(f32, f32)>,
    /// The name's right end.
    pub name_right: f32,
    // Drawing: print, weight, line type (may hide), colour swatch.
    pub print: Option<(f32, f32)>,
    pub weight: Option<(f32, f32)>,
    pub hatch: Option<(f32, f32)>,
    pub line: Option<(f32, f32)>,
    pub swatch: Option<(f32, f32)>,
    // Filing: the item count's right edge.
    pub count_right: f32,
}

impl RowLayout {
    /// The layout for a row spanning `left..right`.
    pub fn plan(lens: Lens, left: f32, right: f32) -> RowLayout {
        let edge = right - 6.0;
        let room = |name_right: f32| name_right - (left + LEAD + PLAN_DEPTH * INDENT);
        let mut c = RowLayout {
            lens,
            name_right: right - 8.0,
            count_right: edge - 40.0,
            ..Default::default()
        };
        match lens {
            Lens::Drawing => {
                // Lock stays on the trailing edge. Drop style previews when the name needs room.
                c.flags = Some((edge - 20.0, edge));
                let mut end = edge - 24.0;
                for (cell, width) in [
                    (&mut c.swatch, 31.0),
                    (&mut c.line, 44.0),
                    (&mut c.hatch, 42.0),
                    (&mut c.weight, 32.0),
                    (&mut c.print, 28.0),
                ] {
                    if room(end - width - 6.0 - 4.0) >= MIN_NAME {
                        *cell = Some((end - width, end));
                        end -= width + 6.0;
                    }
                }
                c.name_right = end - 4.0;
            }
            Lens::Export => {
                c.name_right = edge - 84.;
            }
            _ => {}
        }
        c
    }

    /// The header row: "Name" over the names, each column's title over it.
    pub fn header(&self, ui: &mut egui::Ui, left: f32) -> egui::Response {
        let (r, response) = ui.allocate_exact_size(
            vec2(ui.available_width(), super::ROW_H),
            egui::Sense::hover(),
        );
        let p = ui.painter();
        let cy = r.center().y;
        let font = super::meta();
        let at = |x: f32, text: &str, align: egui::Align2| {
            p.text(pos2(x, cy), align, text, font.clone(), FAINT);
        };
        at(left + 22.0, "Name", egui::Align2::LEFT_CENTER);
        match self.lens {
            Lens::Drawing => {
                if let Some((_, b)) = self.hatch {
                    at(b, "Hatch", egui::Align2::RIGHT_CENTER);
                }
                if let Some((_, b)) = self.print {
                    at(b, "Print", egui::Align2::RIGHT_CENTER);
                }
                if let Some((_, b)) = self.weight {
                    at(b, "Wt", egui::Align2::RIGHT_CENTER);
                }
                if let Some((_, b)) = self.line {
                    at(b, "Line", egui::Align2::RIGHT_CENTER);
                }
                if let Some((a, b)) = self.swatch {
                    at((a + b) / 2.0, "Colour", egui::Align2::CENTER_CENTER);
                }
            }
            Lens::Export => {
                at(r.right() - 8., "Readiness", egui::Align2::RIGHT_CENTER);
            }
            _ => at(self.count_right, "Items", egui::Align2::RIGHT_CENTER),
        }
        p.hline(r.x_range(), r.max.y - 0.5, Stroke::new(1.0_f32, EDGE));
        response
    }
}

/// A linked file's row icon: a page with a folded corner.
pub fn paint_file(painter: &egui::Painter, c: egui::Pos2, ink: Color32) {
    let r = Rect::from_center_size(c, vec2(9.0, 11.0));
    let fold = 3.0;
    let pts = vec![
        r.left_top(),
        pos2(r.max.x - fold, r.min.y),
        pos2(r.max.x, r.min.y + fold),
        r.right_bottom(),
        r.left_bottom(),
        r.left_top(),
    ];
    painter.add(egui::Shape::line(pts, Stroke::new(1.1_f32, ink)));
    painter.line_segment(
        [
            pos2(r.max.x - fold, r.min.y),
            pos2(r.max.x - fold, r.min.y + fold),
        ],
        Stroke::new(1.0_f32, ink),
    );
    painter.line_segment(
        [
            pos2(r.max.x - fold, r.min.y + fold),
            pos2(r.max.x, r.min.y + fold),
        ],
        Stroke::new(1.0_f32, ink),
    );
}

/// The file's state dot: found, changed on disk, not found, packed.
pub fn state_color(state: &str) -> Color32 {
    match state {
        "resolved" => FOUND,
        "modified" => CHANGED,
        "missing" => MISSING,
        _ => PACKED,
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
        let narrow = RowLayout::plan(Lens::Drawing, 0.0, 280.0);
        let wide = RowLayout::plan(Lens::Drawing, 0.0, 370.0);
        assert!(narrow.line.is_none() && wide.line.is_some());
        for c in [narrow, wide] {
            assert!(
                c.name_right - (LEAD + PLAN_DEPTH * INDENT) >= MIN_NAME - 30.0,
                "{c:?}"
            );
        }
    }

    #[test]
    fn drawing_cells_fit_inside_the_dock_as_it_widens_and_narrows() {
        for width in (280..=800).chain((280..800).rev()) {
            let left = 37.;
            let right = left + width as f32;
            let row = RowLayout::plan(Lens::Drawing, left, right);
            assert!(row.name_right - left - LEAD - PLAN_DEPTH * INDENT >= MIN_NAME);
            let mut previous = row.name_right;
            for (start, end) in [row.print, row.weight, row.hatch, row.line, row.swatch, row.flags]
                .into_iter().flatten()
            {
                assert!(previous <= start && start < end && end <= right);
                previous = end;
            }
        }
    }

    #[test]
    fn export_reserves_only_the_readiness_label() {
        let row = RowLayout::plan(Lens::Export, 0., 370.);
        assert!(row.swatch.is_none() && row.name_right > 275.);
    }
}

#[derive(Default)]
pub(super) struct Overview {
    pub file: String,
    pub nodes: usize,
    pub placements: usize,
    pub links: usize,
    pub tags: usize,
    pub folders: usize,
}

impl Overview {
    pub fn read(
        file: String,
        tree: &pyo3::Bound<'_, pyo3::PyAny>,
        counts: &pyo3::Bound<'_, pyo3::PyAny>,
    ) -> pyo3::PyResult<Self> {
        use pyo3::prelude::*;
        let scene = tree.getattr("scene")?;
        let mut result = Self {
            file,
            tags: scene.getattr("layers")?.len()?,
            folders: scene.getattr("layer_folders")?.len()?,
            links: tree.getattr("links")?.len()?,
            ..Self::default()
        };
        for pair in tree.getattr("nodes")?.call_method0("items")?.try_iter()? {
            let pair = pair?;
            let node = pair.get_item(1)?;
            if node.get_item("kind")?.extract::<String>()? == "host" {
                result.nodes += 1;
                if node.get_item("parent")?.is_none() {
                    result.placements += counts.get_item(pair.get_item(0)?)?.extract::<usize>()?;
                }
            }
        }
        Ok(result)
    }
}
