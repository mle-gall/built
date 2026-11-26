// SPDX-License-Identifier: GPL-3.0-or-later
//! The project tree's columns. Filing, Drawing and Export are presets of
//! one tree: a preset only chooses the trailing cells. Rows, selection,
//! expansion, scroll and the dock's width are the same under every preset.
use egui::{pos2, vec2, Color32, Rect, Stroke};

use super::{ACCENT, CHANGED, EDGE, FAINT, FOUND, MISSING, PACKED};

/// A name keeps at least this much room (points) before the cells compact.
pub const MIN_NAME: f32 = 120.0;
/// Compact cells leave the name a little less, so a 300-pt dock still shows
/// every Drawing swatch without scrolling.
const MIN_NAME_COMPACT: f32 = 100.0;
/// Indent per depth and the room the eye, the triangle and an icon take.
const INDENT: f32 = 14.0;
const LEAD: f32 = 4.0 + 16.0 + 16.0;
/// The depth a layout is planned for: deeper rows may still cut their names.
const PLAN_DEPTH: f32 = 2.0;
const PLAN_DEPTH_COMPACT: f32 = 1.0;

/// A column preset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Lens {
    #[default]
    Filing,
    Drawing,
    Export,
}
impl Lens {
    pub const ALL: [Lens; 3] = [Lens::Filing, Lens::Drawing, Lens::Export];
    pub fn from_index(index: u8) -> Self {
        match index {
            1 => Self::Drawing,
            2 => Self::Export,
            _ => Self::Filing,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Filing => "Filing",
            Self::Drawing => "Drawing",
            Self::Export => "Export",
        }
    }
    fn columns(self) -> &'static [Col] {
        match self {
            Self::Filing => &[Col::Eye, Col::Kind, Col::Items, Col::Lock],
            Self::Drawing => &[
                Col::Eye, Col::Kind, Col::Print, Col::Weight, Col::Hatch, Col::Line, Col::Colour, Col::Lock,
            ],
            Self::Export => &[Col::Eye, Col::Kind, Col::Readiness],
        }
    }
}

/// One column of the tree. The eye leads the row, the kind follows the
/// name, the others are trailing cells in `TRAILING` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Col {
    Eye,
    Kind,
    Items,
    Print,
    Weight,
    Hatch,
    Line,
    Colour,
    Readiness,
    Lock,
}

/// The trailing cells, left to right. The lock keeps the trailing edge.
const TRAILING: [Col; 8] = [
    Col::Items, Col::Print, Col::Weight, Col::Hatch, Col::Line, Col::Colour, Col::Readiness, Col::Lock,
];

impl Col {
    pub const ALL: [Col; 10] = [
        Col::Eye, Col::Lock, Col::Items, Col::Kind, Col::Colour, Col::Line, Col::Weight, Col::Hatch,
        Col::Print, Col::Readiness,
    ];
    /// The name in the Columns menu and the full header.
    pub fn label(self) -> &'static str {
        match self {
            Col::Eye => "Visibility",
            Col::Kind => "Kind",
            Col::Items => "Items",
            Col::Print => "Print",
            Col::Weight => "Line weight",
            Col::Hatch => "Hatch",
            Col::Line => "Line type",
            Col::Colour => "Colour",
            Col::Readiness => "Readiness",
            Col::Lock => "Lock",
        }
    }
    /// The header over a full cell and over a compact one.
    fn header(self, compact: bool) -> &'static str {
        match (self, compact) {
            (Col::Items, false) => "Items",
            (Col::Items, true) => "#",
            (Col::Print, false) => "Print",
            (Col::Print, true) => "P",
            (Col::Weight, _) => "Wt",
            (Col::Hatch, false) => "Hatch",
            (Col::Hatch, true) => "H",
            (Col::Line, false) => "Line",
            (Col::Line, true) => "L",
            (Col::Colour, false) => "Colour",
            (Col::Colour, true) => "C",
            (Col::Readiness, false) => "Readiness",
            (Col::Readiness, true) => "✓",
            _ => "",
        }
    }
    fn bit(self) -> u16 {
        1 << self as u16
    }
    /// A trailing cell's width, full or compact (a swatch or a glyph).
    fn width(self, compact: bool) -> f32 {
        match (self, compact) {
            (Col::Items, false) => 36.0,
            (Col::Items, true) => 28.0,
            (Col::Print, false) => 30.0,
            (Col::Print, true) => 14.0,
            (Col::Weight, false) => 32.0,
            (Col::Weight, true) => 18.0,
            (Col::Hatch, false) => 42.0,
            (Col::Hatch, true) => 20.0,
            (Col::Line, false) => 44.0,
            (Col::Line, true) => 20.0,
            (Col::Colour, false) => 40.0,
            (Col::Colour, true) => 14.0,
            (Col::Readiness, false) => 76.0,
            (Col::Readiness, true) => 24.0,
            (Col::Lock, false) => 20.0,
            (Col::Lock, true) => 16.0,
            _ => 0.0,
        }
    }
}

/// The columns shown: a preset, or the user's own choice of columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Columns {
    /// The preset last chosen (its name shows while its columns are unchanged).
    pub preset: Lens,
    shown: u16,
}

impl Default for Columns {
    fn default() -> Self {
        Self::preset(Lens::Filing)
    }
}

impl Columns {
    pub fn preset(lens: Lens) -> Self {
        Self { preset: lens, shown: lens.columns().iter().fold(0, |mask, col| mask | col.bit()) }
    }
    /// A stored choice: the preset and, when the user changed it, the columns.
    pub fn stored(lens: Lens, mask: Option<u16>) -> Self {
        let all = Col::ALL.iter().fold(0, |mask, col| mask | col.bit());
        match mask {
            Some(mask) => Self { preset: lens, shown: mask & all },
            None => Self::preset(lens),
        }
    }
    pub fn mask(&self) -> u16 {
        self.shown
    }
    pub fn has(&self, col: Col) -> bool {
        self.shown & col.bit() != 0
    }
    pub fn toggle(&mut self, col: Col) {
        self.shown ^= col.bit();
    }
    /// The preset these columns are, if any.
    pub fn matching(&self) -> Option<Lens> {
        Lens::ALL.into_iter().find(|lens| Self::preset(*lens).shown == self.shown)
    }
    /// What the Columns control shows.
    pub fn name(&self) -> &'static str {
        self.matching().map_or("Custom", Lens::label)
    }
}

/// Where a row's cells go. One plan serves the header and every row.
#[derive(Clone, Copy, Debug)]
pub struct RowLayout {
    pub columns: Columns,
    /// Swatch-only cells: the full ones did not fit beside the name.
    pub compact: bool,
    /// Where the row starts (after the eye, when it shows).
    pub lead: f32,
    /// The name's right end. The name column stays pinned.
    pub name_right: f32,
    /// The window the trailing cells show through. They scroll inside it.
    pub area: (f32, f32),
    /// The trailing cells' whole width.
    pub content: f32,
    cells: [Option<(f32, f32)>; TRAILING.len()],
}

impl RowLayout {
    /// The layout for a row spanning `left..right`, its cells scrolled by `scroll`.
    pub fn plan(columns: Columns, left: f32, right: f32, scroll: f32) -> RowLayout {
        let edge = right - 6.0;
        let lead = if columns.has(Col::Eye) { left } else { left - 16.0 };
        let shown: Vec<Col> = TRAILING.into_iter().filter(|col| columns.has(*col)).collect();
        let total = |compact: bool| {
            let gap = if compact { 2.0 } else { 6.0 };
            shown.iter().map(|col| col.width(compact) + gap).sum::<f32>()
        };
        let room = |compact: bool| {
            let (depth, name) = if compact { (PLAN_DEPTH_COMPACT, MIN_NAME_COMPACT) } else { (PLAN_DEPTH, MIN_NAME) };
            (edge - (lead + LEAD + depth * INDENT + name + 4.0)).max(0.0)
        };
        let compact = total(false) > room(false);
        let content = total(compact);
        let width = content.min(room(compact));
        let area = (edge - width, edge);
        let scroll = scroll.clamp(0.0, (content - width).max(0.0));
        let gap = if compact { 2.0 } else { 6.0 };
        let mut cells = [None; TRAILING.len()];
        let mut x = area.0 - scroll + gap;
        for (slot, col) in cells.iter_mut().zip(TRAILING) {
            if columns.has(col) {
                let w = col.width(compact);
                *slot = Some((x, x + w));
                x += w + gap;
            }
        }
        RowLayout { columns, compact, lead, name_right: area.0 - 4.0, area, content, cells }
    }

    /// A trailing cell's span, if its column shows.
    pub fn cell(&self, col: Col) -> Option<(f32, f32)> {
        TRAILING.iter().position(|c| *c == col).and_then(|i| self.cells[i])
    }

    /// How far the cells can scroll under the pinned name.
    pub fn max_scroll(&self) -> f32 {
        (self.content - (self.area.1 - self.area.0)).max(0.0)
    }

    /// The cells' window on a row.
    pub fn area_rect(&self, row: Rect) -> Rect {
        Rect::from_min_max(pos2(self.area.0, row.min.y), pos2(self.area.1, row.max.y))
    }

    /// The header row: "Name" over the names, each column's title over it.
    /// `tags`: the eye and the lock act on every tag (a tree of tags) or on
    /// the top-level groups.
    pub fn header(&self, ui: &mut egui::Ui, left: f32, tags: bool, actions: &mut Vec<String>) -> egui::Response {
        let (r, response) = ui.allocate_exact_size(
            vec2(ui.available_width(), super::ROW_H),
            egui::Sense::hover(),
        );
        let cy = r.center().y;
        let font = super::meta();
        ui.painter().text(pos2(self.lead + 22.0, cy), egui::Align2::LEFT_CENTER, "Name", font.clone(), FAINT);
        ui.painter().hline(r.x_range(), r.max.y - 0.5, Stroke::new(1.0_f32, EDGE));
        let scope = if tags { "tags" } else { "placements" };
        if self.columns.has(Col::Eye) {
            let cell = Rect::from_center_size(pos2(left + 10., cy), vec2(18., super::ROW_H));
            super::kit::paint_eye(ui, cell.shrink(2.), true);
            let tip = if tags { "Show or hide all tags" } else { "Show or hide all top-level groups" };
            if ui.interact(cell, egui::Id::new(("project header", "eye")), egui::Sense::click()).on_hover_text(tip).clicked() {
                actions.push(format!("headereye\u{1f}{scope}"));
            }
        }
        let clip = ui.clip_rect();
        let area = self.area_rect(r);
        ui.set_clip_rect(clip.intersect(area));
        for col in TRAILING {
            let Some((a, b)) = self.cell(col) else { continue };
            let cell = Rect::from_min_max(pos2(a, r.min.y), pos2(b, r.max.y)).intersect(area);
            if !cell.is_positive() {
                continue;
            }
            if col == Col::Lock {
                super::kit::paint_lock(ui, Rect::from_center_size(pos2((a + b) / 2.0, cy), vec2(14., super::ROW_H - 4.)), true);
                let tip = if tags { "Lock or unlock all tags" } else { "Lock or unlock all top-level groups" };
                if ui.interact(cell, egui::Id::new(("project header", "lock")), egui::Sense::click()).on_hover_text(tip).clicked() {
                    actions.push(format!("headerlock\u{1f}{scope}"));
                }
                continue;
            }
            let (x, align) = match col {
                Col::Colour | Col::Print | Col::Hatch | Col::Line | Col::Weight if self.compact => ((a + b) / 2.0, egui::Align2::CENTER_CENTER),
                Col::Colour => ((a + b) / 2.0, egui::Align2::CENTER_CENTER),
                _ => (b, egui::Align2::RIGHT_CENTER),
            };
            ui.painter().text(pos2(x, cy), align, col.header(self.compact), font.clone(), FAINT);
            ui.interact(cell, egui::Id::new(("project header", col.label())), egui::Sense::hover()).on_hover_text(col.label());
        }
        ui.set_clip_rect(clip);
        response
    }
}

/// "1 edge", "2 edges": a count with its noun.
pub fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
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

    const DOCK: f32 = 300.0 - 20.0;

    fn spans(row: &RowLayout) -> Vec<(f32, f32)> {
        TRAILING.into_iter().filter_map(|col| row.cell(col)).collect()
    }

    #[test]
    fn a_preset_only_changes_the_trailing_cells() {
        let rows: Vec<_> = Lens::ALL.into_iter().map(|lens| RowLayout::plan(Columns::preset(lens), 37., 37. + 430., 0.)).collect();
        // The eye and the row's start are where they were under every preset.
        assert!(rows.iter().all(|row| row.lead == rows[0].lead && row.columns.has(Col::Eye)));
        assert!(rows[0].cell(Col::Items).is_some() && rows[0].cell(Col::Colour).is_none());
        assert!(rows[1].cell(Col::Colour).is_some() && rows[1].cell(Col::Readiness).is_none());
        assert!(rows[2].cell(Col::Readiness).is_some() && rows[2].cell(Col::Items).is_none());
    }

    #[test]
    fn every_preset_fits_the_default_dock_without_scrolling() {
        for lens in Lens::ALL {
            let row = RowLayout::plan(Columns::preset(lens), 0., DOCK, 0.);
            assert_eq!(row.max_scroll(), 0., "{lens:?}: {row:?}");
            assert!(row.name_right - (LEAD + PLAN_DEPTH_COMPACT * INDENT) >= MIN_NAME_COMPACT, "{lens:?}");
        }
        // Drawing's swatches compact at the default width and open up in a wide dock.
        assert!(RowLayout::plan(Columns::preset(Lens::Drawing), 0., DOCK, 0.).compact);
        assert!(!RowLayout::plan(Columns::preset(Lens::Drawing), 0., 480., 0.).compact);
        assert!(!RowLayout::plan(Columns::preset(Lens::Filing), 0., DOCK, 0.).compact);
        assert!(!RowLayout::plan(Columns::preset(Lens::Export), 0., DOCK, 0.).compact);
    }

    #[test]
    fn cells_stay_in_order_inside_the_dock_at_every_width() {
        let mut all = Columns::preset(Lens::Drawing);
        all.toggle(Col::Items);
        all.toggle(Col::Readiness);
        for columns in [Columns::preset(Lens::Filing), Columns::preset(Lens::Drawing), Columns::preset(Lens::Export), all] {
            for width in (260..=800).chain((260..800).rev()) {
                let left = 37.;
                let right = left + width as f32;
                let row = RowLayout::plan(columns, left, right, 0.);
                let min = if row.compact { MIN_NAME_COMPACT } else { MIN_NAME };
                let depth = if row.compact { PLAN_DEPTH_COMPACT } else { PLAN_DEPTH };
                assert!(row.name_right - left - LEAD - depth * INDENT >= min - 0.01, "{width}: {row:?}");
                assert!(row.area.0 <= row.area.1 && row.area.1 <= right);
                let mut previous = row.area.0;
                for (start, end) in spans(&row) {
                    assert!(previous <= start && start < end, "{width}: {row:?}");
                    previous = end;
                }
                // Without scrolling the cells end inside their window.
                if row.max_scroll() == 0. {
                    assert!(previous <= row.area.1 + 0.01, "{width}: {row:?}");
                }
            }
        }
    }

    #[test]
    fn too_many_columns_scroll_under_a_pinned_name() {
        let mut all = Columns::preset(Lens::Drawing);
        all.toggle(Col::Items);
        all.toggle(Col::Readiness);
        let still = RowLayout::plan(all, 0., DOCK, 0.);
        assert!(still.compact && still.max_scroll() > 0.);
        let scrolled = RowLayout::plan(all, 0., DOCK, still.max_scroll() + 50.);
        // The name column and the window do not move; the cells do, and stop at the end.
        assert_eq!((scrolled.name_right, scrolled.area), (still.name_right, still.area));
        let (first, moved) = (spans(&still), spans(&scrolled));
        assert!((first[0].0 - moved[0].0 - still.max_scroll()).abs() < 0.01);
        assert!((moved.last().unwrap().1 - scrolled.area.1).abs() < 2.01);
    }

    #[test]
    fn the_control_names_the_preset_until_a_column_is_changed() {
        let mut columns = Columns::preset(Lens::Export);
        assert_eq!(columns.name(), "Export");
        columns.toggle(Col::Items);
        assert_eq!((columns.name(), columns.preset), ("Custom", Lens::Export));
        columns.toggle(Col::Items);
        assert_eq!(columns.name(), "Export");
        assert_eq!(Columns::stored(Lens::Drawing, None), Columns::preset(Lens::Drawing));
        assert_eq!(Columns::stored(Lens::Filing, Some(Columns::preset(Lens::Export).mask())).name(), "Export");
        assert_eq!(Lens::ALL.map(Lens::label), ["Filing", "Drawing", "Export"]);
    }

    #[test]
    fn counts_agree_with_their_noun() {
        assert_eq!(count(1, "edge", "edges"), "1 edge");
        assert_eq!(count(0, "face", "faces"), "0 faces");
        assert_eq!(count(2, "tag", "tags"), "2 tags");
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
