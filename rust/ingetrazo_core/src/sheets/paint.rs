// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! The sheet painters' display list (docs/rust-sheets.md §3, §5).
//!
//! `Painter` keeps the state a QPainter keeps for the mm-space painters of
//! views/composer.py (transform stack, pen, brush, font) and records what
//! they draw as primitives in the painter's base frame (item-local paper
//! mm). The renderer paints the list; the PDF and DXF writers write it. The
//! record has the shape tests/sheet_paint_recorder.py gives the Python
//! painters, so the two compare primitive by primitive.

use serde_json::{json, Value};

/// A colour as the painters name it: `#rrggbb`, `#rrggbbaa` or rgb.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    pub const INK: Rgba = Rgba(30, 36, 44, 255);
    pub const WHITE: Rgba = Rgba(255, 255, 255, 255);
    pub fn hex(&self) -> String {
        format!("#{:02x}{:02x}{:02x}{:02x}", self.0, self.1, self.2, self.3)
    }
    /// `QColor(str)`; an invalid name is black, as Qt's.
    pub fn parse(s: &str) -> Rgba {
        let h = s.trim().trim_start_matches('#');
        let v = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0);
        match h.len() {
            6 => Rgba(v(0), v(2), v(4), 255),
            8 => Rgba(v(2), v(4), v(6), v(0)), // Qt's #AARRGGBB
            3 => {
                let d = |i: usize| u8::from_str_radix(&h[i..i + 1], 16).map(|x| x * 17).unwrap_or(0);
                Rgba(d(0), d(1), d(2), 255)
            }
            _ => Rgba(0, 0, 0, 255),
        }
    }
    /// `QColor.fromRgbF`.
    pub fn from_f(r: f64, g: f64, b: f64) -> Rgba {
        let c = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
        Rgba(c(r), c(g), c(b), 255)
    }
    /// `setAlphaF`.
    pub fn alpha_f(mut self, a: f64) -> Rgba {
        self.3 = (a.clamp(0.0, 1.0) * 255.0).round() as u8;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cap {
    Flat,
    Square,
    Round,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pen {
    pub color: Rgba,
    pub width: f64,
    /// Dash pattern in pen widths (Qt's `setDashPattern`); empty = solid.
    pub dash: Vec<f64>,
    pub cap: Cap,
    pub none: bool,
}

impl Pen {
    pub fn new(color: Rgba, width: f64) -> Pen {
        Pen { color, width, dash: Vec::new(), cap: Cap::Square, none: false }
    }
    pub fn none() -> Pen {
        Pen { none: true, ..Pen::new(Rgba(0, 0, 0, 255), 1.0) }
    }
    pub fn cap(mut self, cap: Cap) -> Pen {
        self.cap = cap;
        self
    }
    pub fn dashed(mut self, pattern: &[f64]) -> Pen {
        self.dash = pattern.to_vec();
        self
    }
}

/// Qt's text flags the painters use.
pub mod align {
    pub const LEFT: i64 = 0x1;
    pub const RIGHT: i64 = 0x2;
    pub const HCENTER: i64 = 0x4;
    pub const TOP: i64 = 0x20;
    pub const BOTTOM: i64 = 0x40;
    pub const VCENTER: i64 = 0x80;
    pub const CENTER: i64 = HCENTER | VCENTER;
    pub const WORD_WRAP: i64 = 0x1000;
}

/// An affine map as QTransform keeps it (row vectors: x' = m11·x + m21·y + dx).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xf {
    pub m11: f64,
    pub m12: f64,
    pub m21: f64,
    pub m22: f64,
    pub dx: f64,
    pub dy: f64,
}

impl Default for Xf {
    fn default() -> Self {
        Xf { m11: 1.0, m12: 0.0, m21: 0.0, m22: 1.0, dx: 0.0, dy: 0.0 }
    }
}

impl Xf {
    pub fn map(&self, x: f64, y: f64) -> [f64; 2] {
        [self.m11 * x + self.m21 * y + self.dx, self.m12 * x + self.m22 * y + self.dy]
    }
    pub fn translate(&mut self, x: f64, y: f64) {
        self.dx += x * self.m11 + y * self.m21;
        self.dy += y * self.m22 + x * self.m12;
    }
    pub fn scale(&mut self, sx: f64, sy: f64) {
        self.m11 *= sx;
        self.m12 *= sx;
        self.m21 *= sy;
        self.m22 *= sy;
    }
    /// QTransform::rotate (degrees, the quarter turns exact).
    pub fn rotate(&mut self, a: f64) {
        if a == 0.0 {
            return;
        }
        let (sina, cosa) = if a == 90.0 || a == -270.0 {
            (1.0, 0.0)
        } else if a == 270.0 || a == -90.0 {
            (-1.0, 0.0)
        } else if a == 180.0 {
            (0.0, -1.0)
        } else {
            let b = std::f64::consts::PI / 180.0 * a;
            (b.sin(), b.cos())
        };
        let (m11, m12, m21, m22) = (self.m11, self.m12, self.m21, self.m22);
        self.m11 = cosa * m11 + sina * m21;
        self.m12 = cosa * m12 + sina * m22;
        self.m21 = -sina * m11 + cosa * m21;
        self.m22 = -sina * m12 + cosa * m22;
    }
    pub fn det_scale(&self) -> f64 {
        (self.m11 * self.m22 - self.m12 * self.m21).abs().sqrt()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub family: String,
}

impl Default for Font {
    fn default() -> Self {
        Font { size: 100.0, bold: false, italic: false, underline: false, family: "Sans Serif".into() }
    }
}

#[derive(Clone)]
struct State {
    xf: Xf,
    pen: Pen,
    brush: Option<Rgba>,
    font: Font,
}

/// One primitive, in the painter's base frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Prim {
    Line { pts: [f64; 4] },
    Poly { pts: Vec<[f64; 2]>, closed: bool },
    Ellipse { center: [f64; 2], ax: [f64; 2], ay: [f64; 2] },
    RRect { pts: [[f64; 2]; 4], rx: f64, ry: f64 },
    Path { rings: Vec<Vec<[f64; 2]>> },
    Text { rect: [[f64; 2]; 4], flags: i64, text: String, size: f64, bold: bool, italic: bool, underline: bool, color: Rgba },
    Image { rect: [[f64; 2]; 4], source: String },
    Clip { rect: Option<[[f64; 2]; 4]>, rings: Vec<Vec<[f64; 2]>> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Op {
    pub prim: Prim,
    pub pen: Option<Pen>,
    pub brush: Option<Rgba>,
}

#[derive(Clone)]
pub struct Painter {
    st: State,
    stack: Vec<State>,
    pub ops: Vec<Op>,
}

impl Default for Painter {
    fn default() -> Self {
        Painter {
            st: State { xf: Xf::default(), pen: Pen::new(Rgba(0, 0, 0, 255), 1.0), brush: None, font: Font::default() },
            stack: Vec::new(),
            ops: Vec::new(),
        }
    }
}

/// Points of a flattened arc (QPainterPath.arcTo's ellipse arc), degrees
/// counter-clockwise from 3 o'clock with y up, as Qt measures them.
pub fn arc_points(rect: [f64; 4], start: f64, sweep: f64) -> Vec<[f64; 2]> {
    let [x, y, w, h] = rect;
    let (cx, cy, rx, ry) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
    let n = ((sweep.abs() / 3.0).ceil() as usize).max(2);
    (0..=n).map(|i| {
        let a = (start + sweep * i as f64 / n as f64).to_radians();
        [cx + rx * a.cos(), cy - ry * a.sin()]
    }).collect()
}

impl Painter {
    pub fn save(&mut self) {
        self.stack.push(self.st.clone());
    }
    pub fn restore(&mut self) {
        if let Some(s) = self.stack.pop() {
            self.st = s;
        }
    }
    pub fn translate(&mut self, x: f64, y: f64) {
        self.st.xf.translate(x, y);
    }
    pub fn rotate(&mut self, deg: f64) {
        self.st.xf.rotate(deg);
    }
    pub fn scale(&mut self, sx: f64, sy: f64) {
        self.st.xf.scale(sx, sy);
    }
    pub fn set_pen(&mut self, pen: Pen) {
        self.st.pen = pen;
    }
    pub fn pen(&self) -> Pen {
        self.st.pen.clone()
    }
    pub fn set_brush(&mut self, brush: Option<Rgba>) {
        self.st.brush = brush;
    }
    pub fn set_font(&mut self, font: Font) {
        self.st.font = font;
    }

    fn pt(&self, x: f64, y: f64) -> [f64; 2] {
        self.st.xf.map(x, y)
    }
    fn rect_pts(&self, r: [f64; 4]) -> [[f64; 2]; 4] {
        let [x, y, w, h] = r;
        [self.pt(x, y), self.pt(x + w, y), self.pt(x + w, y + h), self.pt(x, y + h)]
    }
    fn mapped_pen(&self) -> Option<Pen> {
        let p = &self.st.pen;
        if p.none {
            return None;
        }
        let s = self.st.xf.det_scale();
        Some(Pen { width: p.width * s, dash: p.dash.iter().map(|d| d * p.width * s).collect(), ..p.clone() })
    }
    fn push(&mut self, prim: Prim, stroke: bool, fill: bool) {
        let pen = if stroke { self.mapped_pen() } else { None };
        let brush = if fill { self.st.brush } else { None };
        self.ops.push(Op { prim, pen, brush });
    }

    pub fn line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64) {
        let (a, b) = (self.pt(x0, y0), self.pt(x1, y1));
        self.push(Prim::Line { pts: [a[0], a[1], b[0], b[1]] }, true, false);
    }
    pub fn rect(&mut self, r: [f64; 4]) {
        self.push(Prim::Poly { pts: self.rect_pts(r).to_vec(), closed: true }, true, true);
    }
    pub fn fill_rect(&mut self, r: [f64; 4], fill: Rgba) {
        let pts = self.rect_pts(r).to_vec();
        self.ops.push(Op { prim: Prim::Poly { pts, closed: true }, pen: None, brush: Some(fill) });
    }
    pub fn rounded_rect(&mut self, r: [f64; 4], rx: f64, ry: f64) {
        let s = self.st.xf.det_scale();
        self.push(Prim::RRect { pts: self.rect_pts(r), rx: rx * s, ry: ry * s }, true, true);
    }
    pub fn ellipse(&mut self, cx: f64, cy: f64, rx: f64, ry: f64) {
        let o = self.pt(cx, cy);
        let a = self.pt(cx + rx, cy);
        let b = self.pt(cx, cy + ry);
        self.push(Prim::Ellipse { center: o, ax: [a[0] - o[0], a[1] - o[1]], ay: [b[0] - o[0], b[1] - o[1]] }, true, true);
    }
    pub fn ellipse_in(&mut self, r: [f64; 4]) {
        self.ellipse(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, r[2] / 2.0, r[3] / 2.0);
    }
    pub fn polygon(&mut self, pts: &[[f64; 2]]) {
        let pts = pts.iter().map(|p| self.pt(p[0], p[1])).collect();
        self.push(Prim::Poly { pts, closed: true }, true, true);
    }
    pub fn polyline(&mut self, pts: &[[f64; 2]]) {
        let pts = pts.iter().map(|p| self.pt(p[0], p[1])).collect();
        self.push(Prim::Poly { pts, closed: false }, true, false);
    }
    fn rings(&self, rings: &[Vec<[f64; 2]>]) -> Vec<Vec<[f64; 2]>> {
        rings.iter().map(|r| r.iter().map(|p| self.pt(p[0], p[1])).collect()).collect()
    }
    /// `drawPath` of a path given as closed rings (item-local).
    pub fn path(&mut self, rings: &[Vec<[f64; 2]>]) {
        let rings = self.rings(rings);
        self.push(Prim::Path { rings }, true, true);
    }
    pub fn fill_path(&mut self, rings: &[Vec<[f64; 2]>], fill: Rgba) {
        let rings = self.rings(rings);
        self.ops.push(Op { prim: Prim::Path { rings }, pen: None, brush: Some(fill) });
    }
    pub fn clip_rect(&mut self, r: [f64; 4]) {
        let rect = self.rect_pts(r);
        self.ops.push(Op { prim: Prim::Clip { rect: Some(rect), rings: Vec::new() }, pen: None, brush: None });
    }
    pub fn clip_path(&mut self, rings: &[Vec<[f64; 2]>]) {
        let rings = self.rings(rings);
        self.ops.push(Op { prim: Prim::Clip { rect: None, rings }, pen: None, brush: None });
    }
    pub fn text(&mut self, r: [f64; 4], flags: i64, text: &str) {
        let f = &self.st.font;
        let prim = Prim::Text {
            rect: self.rect_pts(r), flags, text: text.to_string(),
            size: f.size * self.st.xf.det_scale(), bold: f.bold, italic: f.italic, underline: f.underline,
            color: self.st.pen.color,
        };
        self.ops.push(Op { prim, pen: None, brush: None });
    }
    pub fn image(&mut self, r: [f64; 4], source: &str) {
        let rect = self.rect_pts(r);
        self.ops.push(Op { prim: Prim::Image { rect, source: source.to_string() }, pen: None, brush: None });
    }

    /// views/composer.py `_draw_text_mm`: `text` inside `rect` (mm) at
    /// `size_mm` tall, word-wrapped.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_mm(&mut self, rect: [f64; 4], text: &str, size_mm: f64, bold: bool, flags: i64,
                        color: Rgba, italic: bool, underline: bool) {
        if text.is_empty() {
            return;
        }
        self.save();
        self.set_font(Font { size: 100.0, bold, italic, underline, family: "Sans Serif".into() });
        self.set_pen(Pen::new(color, 1.0));
        let s = size_mm / 100.0 * 0.75;
        self.scale(s, s);
        self.text([rect[0] / s, rect[1] / s, rect[2] / s, rect[3] / s], flags | align::WORD_WRAP, text);
        self.restore();
    }
    /// `_draw_text_mm` with its defaults (regular, ink, not italic).
    pub fn text_mm(&mut self, rect: [f64; 4], text: &str, size_mm: f64, bold: bool, flags: i64, color: Rgba) {
        self.draw_text_mm(rect, text, size_mm, bold, flags, color, false, false);
    }
}

// ---- The record --------------------------------------------------------------

fn pts_json(p: &[[f64; 2]]) -> Value {
    json!(p)
}

fn pen_json(p: &Option<Pen>) -> Value {
    match p {
        None => Value::Null,
        Some(p) => json!({
            "color": p.color.hex(), "width": p.width,
            "style": if p.dash.is_empty() { "solid" } else { "custom" },
            "dash": p.dash,
            "cap": match p.cap { Cap::Flat => "flat", Cap::Square => "square", Cap::Round => "round" },
        }),
    }
}

impl Op {
    pub fn to_json(&self) -> Value {
        let mut v = match &self.prim {
            Prim::Line { pts } => json!({"op": "line", "pts": pts}),
            Prim::Poly { pts, closed } => json!({"op": "poly", "pts": pts_json(pts), "closed": closed}),
            Prim::Ellipse { center, ax, ay } => json!({"op": "ellipse", "center": center, "ax": ax, "ay": ay}),
            Prim::RRect { pts, rx, ry } => json!({"op": "rrect", "pts": pts, "rx": rx, "ry": ry}),
            Prim::Path { rings } => json!({"op": "path", "rings": rings}),
            Prim::Text { rect, flags, text, size, bold, italic, underline, color } => json!({
                "op": "text", "rect": rect, "flags": flags, "text": text, "size": size, "bold": bold,
                "italic": italic, "underline": underline, "color": color.hex()}),
            Prim::Image { rect, source } => json!({"op": "image", "rect": rect, "source": source}),
            Prim::Clip { rect, rings } => match rect {
                Some(r) => json!({"op": "clip", "rect": r}),
                None => json!({"op": "clip", "rings": rings}),
            },
        };
        v["pen"] = pen_json(&self.pen);
        v["brush"] = self.brush.map(|b| Value::String(b.hex())).unwrap_or(Value::Null);
        v
    }
}

/// The box the primitives cover (item-local mm), or None.
pub fn bounds(ops: &[Op]) -> Option<[f64; 4]> {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    let mut take = |p: [f64; 2]| {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    };
    for op in ops {
        match &op.prim {
            Prim::Line { pts } => {
                take([pts[0], pts[1]]);
                take([pts[2], pts[3]]);
            }
            Prim::Poly { pts, .. } => pts.iter().for_each(|p| take(*p)),
            Prim::Ellipse { center, ax, ay } => {
                let (ex, ey) = (ax[0].abs() + ay[0].abs(), ax[1].abs() + ay[1].abs());
                take([center[0] - ex, center[1] - ey]);
                take([center[0] + ex, center[1] + ey]);
            }
            Prim::RRect { pts, .. } | Prim::Text { rect: pts, .. } | Prim::Image { rect: pts, .. } => {
                pts.iter().for_each(|p| take(*p))
            }
            Prim::Path { rings } => rings.iter().flatten().for_each(|p| take(*p)),
            Prim::Clip { .. } => {}
        }
    }
    b[0].is_finite().then_some(b)
}

pub fn ops_json(ops: &[Op]) -> Value {
    Value::Array(ops.iter().map(Op::to_json).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms_are_qtransform_s() {
        let mut x = Xf::default();
        x.translate(10.0, 5.0);
        x.rotate(90.0);
        assert_eq!(x.map(1.0, 0.0), [10.0, 6.0]);
        x.scale(2.0, 2.0);
        assert_eq!(x.map(1.0, 0.0), [10.0, 7.0]);
        assert_eq!(Rgba::parse("#1e242c"), Rgba(30, 36, 44, 255));
    }
}
