// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! The painters the page used to draw its own way, ported line by line
//! from views/composer.py: text blocks (`paint_text_mm`), the title block
//! (`paint_cajetin_mm`, its outline and row heights) and the sheet's
//! border (`paint_sheet_border_mm`). Same discipline as `items`: the
//! primitives match the Python painters' goldens.

use super::items::Extra;
use super::metrics::{fit_text_size_mm, text_bg_span_mm, text_height_mm};
use super::model::Fields;
use super::paint::{align, arc_points, Painter, Pen, Rgba};
use crate::doc::value::Value;

/// core.composition.PT_TO_MM.
const PT_TO_MM: f64 = 25.4 / 72.0;
/// views/composer.py TEXT_BG_PAD_MM.
const TEXT_BG_PAD_MM: f64 = 0.5;
const ROW_GROW: f64 = 3.0;
const ROW_SHRINK: f64 = 0.6;

fn with_opacity(color: &str, opacity: f64) -> Rgba {
    Rgba::parse(color).alpha_f(opacity.clamp(0.0, 1.0))
}

fn or(s: String, default: &str) -> String {
    if s.is_empty() { default.to_string() } else { s }
}

/// `paint_text_mm`. ``x["text"]``: the text with its fields expanded.
pub fn paint_text(p: &mut Painter, f: &Fields, x: &Extra) {
    let size = f.num("size_pt", 14.0) * PT_TO_MM;
    let text = x.text("text");
    let (w, bold) = (f.num("w_mm", 80.0), f.flag("bold", false));
    let bg = f.text("bg_color", "");
    if !bg.is_empty() {
        let (y0, y1) = text_bg_span_mm(&text, w, size, bold);
        p.save();
        p.set_pen(Pen::none());
        p.set_brush(Some(with_opacity(&bg, f.num("bg_opacity", 1.0))));
        p.rect([-TEXT_BG_PAD_MM, y0, w + 2.0 * TEXT_BG_PAD_MM, (y1 - y0).max(1.0)]);
        p.restore();
    }
    let lines = text.matches('\n').count() as f64;
    let rect = [0.0, 0.0, w, size * 1.35 * (lines + 3.0)];
    let h = match f.text("align", "left").as_str() {
        "center" => align::HCENTER,
        "right" => align::RIGHT,
        _ => align::LEFT,
    };
    p.draw_text_mm(rect, &text, size, bold, h | align::TOP, Rgba::parse(&f.text("color", "#1e242c")),
                   f.flag("italic", false), f.flag("underline", false));
}

/// `_outline_path`: square, rounded or chamfered corners, as one ring.
pub fn outline_ring(w: f64, h: f64, corner: &str, r: f64) -> Vec<[f64; 2]> {
    let r = r.max(0.0).min(w / 2.0).min(h / 2.0);
    if corner == "rounded" && r > 0.0 {
        return rounded_ring([0.0, 0.0, w, h], r, r);
    }
    if corner == "chamfer" && r > 0.0 {
        return vec![[r, 0.0], [w - r, 0.0], [w, r], [w, h - r], [w - r, h], [r, h], [0.0, h - r], [0.0, r]];
    }
    vec![[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]]
}

/// `QPainterPath.addRoundedRect` flattened.
pub fn rounded_ring(r: [f64; 4], rx: f64, ry: f64) -> Vec<[f64; 2]> {
    let [x, y, w, h] = r;
    let (rx, ry) = (rx.min(w / 2.0), ry.min(h / 2.0));
    let (dx, dy) = (2.0 * rx, 2.0 * ry);
    let mut ring = arc_points([x + w - dx, y, dx, dy], 0.0, 90.0);
    ring.extend(arc_points([x, y, dx, dy], 90.0, 90.0));
    ring.extend(arc_points([x, y + h - dy, dx, dy], 180.0, 90.0));
    ring.extend(arc_points([x + w - dx, y + h - dy, dx, dy], 270.0, 90.0));
    ring
}

fn rows_of(x: &Extra) -> Vec<(String, String)> {
    match x.0.get("rows") {
        Some(Value::List(rows)) => rows.iter().filter_map(|r| match r {
            Value::List(c) | Value::Tuple(c) if c.len() >= 2 => {
                let s = |v: &Value| match v {
                    Value::Str(s) => s.clone(),
                    _ => String::new(),
                };
                Some((s(&c[0]), s(&c[1])))
            }
            _ => None,
        }).collect(),
        _ => Vec::new(),
    }
}

/// `cajetin_row_heights`.
pub fn row_heights(rows: &[(String, String)], cols: usize, per: usize, body_h: f64, col_w: f64,
                   label_w: f64, layout: &str) -> Vec<f64> {
    let base = body_h / per.max(1) as f64;
    if per <= 1 || base <= 0.0 {
        return vec![body_h; per.max(1)];
    }
    let minimal = layout == "minimal";
    let inner = (if minimal { col_w - 3.0 } else { col_w - label_w - 3.0 }).max(1.0);
    let frac = if minimal { 0.62 } else { 1.0 };
    let nominal = base * if minimal { 0.42 } else { 0.52 };
    let lab_w = (label_w - 2.0).max(1.0);
    let lab_nominal = base * if minimal { 0.24 } else { 0.38 };
    let mut weights = Vec::new();
    for j in 0..per {
        let mut want: f64 = 0.0;
        for k in 0..cols {
            let idx = k * per + j;
            let Some((label, value)) = rows.get(idx) else { continue };
            let v = if value.is_empty() { "M" } else { value.as_str() };
            want = want.max(text_height_mm(v, inner, nominal, false) / frac + 1.0);
            if !minimal {
                want = want.max(text_height_mm(label, lab_w, lab_nominal, true) + 1.0);
            }
        }
        weights.push((want / base).max(ROW_SHRINK).min(ROW_GROW));
    }
    let total: f64 = weights.iter().sum();
    weights.iter().map(|w| body_h * w / total).collect()
}

/// `paint_cajetin_mm`. ``x["rows"]``: the ``[label, value]`` rows with the
/// fields expanded (an empty scale row reads the main frame's).
pub fn paint_titleblock(p: &mut Painter, f: &Fields, x: &Extra) {
    let (w, h) = (f.num("w_mm", 180.0), f.num("h_mm", 33.0));
    let corner = or(f.text("corner", "square"), "square");
    let radius = f.num("radius_mm", 0.0);
    let outline = outline_ring(w, h, &corner, radius);
    let layout = or(f.text("layout", "grid"), "grid");
    let fill = f.text("fill_color", "");
    let line_color = Rgba::parse(&or(f.text("line_color", ""), "#1e242c"));
    let label_color = Rgba::parse(&or(f.text("label_color", ""), "#5a626c"));
    let text_color = Rgba::parse(&or(f.text("text_color", ""), "#1e242c"));
    let heavy = Pen::new(line_color, f.num("border_mm", 0.5));
    let light = Pen::new(line_color, f.num("line_mm", 0.2));
    let rows = rows_of(x);

    p.save();
    p.set_pen(Pen::none());
    p.set_brush(Some(Rgba(255, 255, 255, 255)));
    p.path(std::slice::from_ref(&outline));
    p.clip_path(std::slice::from_ref(&outline));
    let (band, rest): (Option<&(String, String)>, &[(String, String)]) =
        if layout == "banded" && rows.len() > 1 { (Some(&rows[0]), &rows[1..]) } else { (None, &rows[..]) };
    let mut band_h = 0.0;
    if let Some((label, value)) = band {
        band_h = h * 1.5 / (rest.len() as f64 + 1.5);
        if !fill.is_empty() {
            p.fill_rect([0.0, 0.0, w, band_h], Rgba::parse(&fill));
        }
        p.set_pen(light.clone());
        p.line(0.0, band_h, w, band_h);
        p.text_mm([1.5, 0.4, w - 3.0, band_h * 0.32], label, (band_h * 0.2).max(1.4), true,
                  align::LEFT | align::TOP, label_color);
        let vrect = [1.5, band_h * 0.3, w - 3.0, band_h * 0.68];
        let vsize = fit_text_size_mm(value, vrect[2], vrect[3], band_h * 0.4, true);
        p.text_mm(vrect, value, vsize, true, align::HCENTER | align::VCENTER, text_color);
    }
    let y_top = band_h;
    let body_h = h - band_h;
    let cols = (f.num("columns", 1.0) as i64).min(rest.len().max(1) as i64).max(1) as usize;
    let per = (rest.len() as f64 / cols as f64).ceil().max(1.0) as usize;
    let col_w = w / cols as f64;
    let label_mm = f.num("label_mm", 0.0);
    let label_w = if label_mm > 0.0 { (col_w * 0.6).min(label_mm) } else { 28f64.min(col_w * 0.3) };
    let row_hs = row_heights(rest, cols, per, body_h, col_w, label_w, &layout);
    let base_h = body_h / per as f64;
    for k in 0..cols {
        let x0 = k as f64 * col_w;
        let chunk = &rest[(k * per).min(rest.len())..((k + 1) * per).min(rest.len())];
        if layout == "minimal" {
            let mut y = y_top;
            for (j, (label, value)) in chunk.iter().enumerate() {
                let row_h = row_hs[j];
                p.text_mm([x0 + 1.5, y + 0.4, col_w - 3.0, row_h * 0.36], label, (base_h * 0.24).max(1.4), true,
                          align::LEFT | align::TOP, label_color);
                let vrect = [x0 + 1.5, y + row_h * 0.36, col_w - 3.0, row_h * 0.62];
                let vsize = fit_text_size_mm(value, vrect[2], vrect[3], base_h * 0.42, false);
                p.text_mm(vrect, value, vsize, false, align::LEFT | align::VCENTER, text_color);
                y += row_h;
            }
            continue;
        }
        if !fill.is_empty() {
            p.fill_rect([x0, y_top, label_w, body_h], Rgba::parse(&fill));
        }
        p.set_pen(light.clone());
        if k > 0 {
            p.line(x0, y_top, x0, h);
        }
        p.line(x0 + label_w, y_top, x0 + label_w, h);
        let mut y = y_top;
        for (j, (label, value)) in chunk.iter().enumerate() {
            let row_h = row_hs[j];
            if j > 0 {
                p.line(x0, y, x0 + col_w, y);
            }
            let lrect = [x0 + 1.2, y + 0.5, label_w - 2.0, row_h - 1.0];
            let lsize = fit_text_size_mm(label, lrect[2], lrect[3], base_h * 0.38, true);
            p.text_mm(lrect, label, lsize, true, align::LEFT | align::VCENTER, label_color);
            let vrect = [x0 + label_w + 1.5, y + 0.5, col_w - label_w - 3.0, row_h - 1.0];
            let vsize = fit_text_size_mm(value, vrect[2], vrect[3], base_h * 0.52, false);
            p.text_mm(vrect, value, vsize, false, align::LEFT | align::VCENTER, text_color);
            y += row_h;
        }
    }
    p.restore();
    p.set_pen(heavy);
    p.set_brush(None);
    p.path(std::slice::from_ref(&outline));
    if f.flag("double_border", false) {
        let inset = 1.2;
        let inner = outline_ring(w - 2.0 * inset, h - 2.0 * inset, &corner, radius - inset);
        p.save();
        p.translate(inset, inset);
        p.set_pen(light);
        p.path(std::slice::from_ref(&inner));
        p.restore();
    }
}

/// `paint_sheet_border_mm` on a `pw` × `ph` page: the margin rectangle
/// with the sheet's width, colour, corners and line type.
pub fn paint_sheet_border(p: &mut Painter, f: &Fields, pw: f64, ph: f64) {
    if !f.flag("border", false) {
        return;
    }
    let m = f.num("margin_mm", 10.0);
    let r = [m, m, pw - 2.0 * m, ph - 2.0 * m];
    let width = f.num("border_mm", 0.5).max(0.1);
    let radius = f.num("border_radius_mm", 0.0).max(0.0);
    let color = Rgba::parse(&f.text("border_color", "#1e242c"));
    let style = f.text("border_style", "single");
    let mut pen = Pen::new(color, width);
    if style == "dashed" {
        pen = pen.dash_line();
    }
    p.save();
    p.set_pen(pen);
    p.set_brush(None);
    let draw = |p: &mut Painter, rr: [f64; 4]| {
        if radius > 0.01 { p.rounded_rect(rr, radius, radius) } else { p.rect(rr) }
    };
    draw(p, r);
    if style == "double" {
        let inset = width * 2.0 + 1.5;
        p.set_pen(Pen::new(color, (width * 0.5).max(0.1)));
        draw(p, [r[0] + inset, r[1] + inset, r[2] - 2.0 * inset, r[3] - 2.0 * inset]);
    }
    p.restore();
}
