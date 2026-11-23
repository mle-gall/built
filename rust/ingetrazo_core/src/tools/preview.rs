//! What the renderer draws for a tool: lines with a style, shaded faces,
//! the selection box, and where the value field sits.

use super::P3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineStyle {
    /// The rubber band: coloured by the snap or the axis it runs along.
    Rubber,
}

impl LineStyle {
    pub fn name(self) -> &'static str {
        match self { LineStyle::Rubber => "rubber" }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreviewLine { pub a: P3, pub b: P3, pub style: LineStyle }

/// Where the value field sits on the geometry, and its caption.
#[derive(Clone, Debug, PartialEq)]
pub struct ValueAnchor { pub at: P3, pub label: &'static str }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preview {
    pub lines: Vec<PreviewLine>,
    pub faces: Vec<Vec<P3>>,
    /// Select's box in pixels `[x0, y0, x1, y1]` (as dragged) and whether
    /// it is a crossing box.
    pub box_rect: Option<([f64; 4], bool)>,
    pub value: Option<ValueAnchor>,
}

impl Preview {
    pub fn rubber(a: P3, b: P3) -> PreviewLine { PreviewLine { a, b, style: LineStyle::Rubber } }
}
