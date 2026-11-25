// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! The sheet items' painters, ported line by line from views/composer.py
//! (`paint_*_mm`). Each draws one item in its own frame (item-local paper
//! mm) into a `Painter`; tests/test_sheet_paint_parity.py compares the
//! primitives with the Python painters' goldens.
//!
//! `Extra` carries the strings that still come from Python: translations,
//! expanded fields (`{lamina}`), levels formatted in the document's units,
//! line-type patterns.

use super::metrics::text_width_mm;
use super::model::Fields;
use super::paint::{align, arc_points, Cap, Painter, Pen, Rgba};
use super::pymath::hypot;
use crate::doc::value::Value;
use std::collections::HashMap;

const INK: Rgba = Rgba::INK;
const SQRT2: f64 = std::f64::consts::SQRT_2;

/// Strings and tables the painters take from Python, by name.
#[derive(Default, Clone, Debug)]
pub struct Extra {
    pub values: HashMap<String, Value>,
    pub swatch_ops: HashMap<usize, Vec<super::paint::Op>>,
}

impl Extra {
    pub fn text(&self, k: &str) -> String {
        match self.values.get(k) {
            Some(Value::Str(s)) => s.clone(),
            _ => String::new(),
        }
    }
    pub fn nums(&self, k: &str) -> Vec<f64> {
        match self.values.get(k) {
            Some(Value::List(v)) | Some(Value::Tuple(v)) => v.iter().filter_map(num).collect(),
            _ => Vec::new(),
        }
    }
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Float(f) => Some(*f),
        Value::Int(i) => Some(*i as f64),
        _ => None,
    }
}

// ---- Scale bar -----------------------------------------------------------------

/// built_core.composition.BarraEscala.segment_m.
pub fn scalebar_segment_m(segments: f64, scale_n: f64) -> f64 {
    let target_mm = 100.0 / segments;
    let raw_m = target_mm * scale_n / 1000.0;
    const NICE: [f64; 19] = [0.1, 0.2, 0.25, 0.5, 1.0, 2.0, 2.5, 5.0, 10.0, 20.0, 25.0, 50.0,
                             100.0, 200.0, 250.0, 500.0, 1000.0, 2000.0, 5000.0];
    let mut best = NICE[0];
    for n in NICE {
        if n <= raw_m {
            best = n;
        }
    }
    best
}

pub fn scalebar_size(f: &Fields) -> (f64, f64) {
    let segments = f.num("segments", 4.0);
    let scale_n = f.num("scale_n", 100.0);
    let seg_mm = scalebar_segment_m(segments, scale_n) * 1000.0 / scale_n;
    (seg_mm * segments, 8.0)
}

pub fn paint_scalebar(p: &mut Painter, f: &Fields, x: &Extra) {
    let segments = f.num("segments", 4.0) as i64;
    let scale_n = f.num("scale_n", 100.0);
    let seg_m = scalebar_segment_m(segments as f64, scale_n);
    let seg_mm = seg_m * 1000.0 / scale_n;
    let w_mm = seg_mm * segments as f64;
    let bar_h = 2.4;
    p.set_pen(Pen::new(INK, 0.25));
    for i in 0..segments {
        p.set_brush(Some(if i % 2 == 0 { INK } else { Rgba::WHITE }));
        p.rect([i as f64 * seg_mm, 0.0, seg_mm, bar_h]);
    }
    for i in 0..=segments {
        let v = i as f64 * seg_m;
        p.text_mm([i as f64 * seg_mm - 12.0, bar_h + 0.8, 24.0, 4.0], &super::svg::fmt_g(v), 2.6, false,
                  align::HCENTER | align::TOP, INK);
    }
    p.text_mm([0.0, bar_h + 4.6, w_mm, 4.0], &x.text("caption"), 2.6, false, align::HCENTER | align::TOP, INK);
}

// ---- North arrow ----------------------------------------------------------------

pub fn paint_north(p: &mut Painter, f: &Fields) {
    let sz = f.num("size_mm", 18.0);
    let c = sz / 2.0;
    let rad = sz * 0.30;
    p.save();
    p.translate(c, c);
    p.rotate(f.num("angle_deg", 0.0));
    p.set_pen(Pen::new(INK, 0.35));
    p.set_brush(None);
    p.ellipse(0.0, 0.0, rad * 0.92, rad * 0.92);
    let r = rad * 0.78;
    p.set_brush(Some(INK));
    p.polygon(&[[0.0, -r], [r * 0.28, r * 0.35], [0.0, r * 0.12]]);
    p.set_brush(Some(Rgba::WHITE));
    p.polygon(&[[0.0, -r], [-r * 0.28, r * 0.35], [0.0, r * 0.12]]);
    let h = sz * 0.18;
    p.text_mm([-c, -sz * 0.40 - h / 2.0, sz, h], "N", h, true, align::CENTER, INK);
    p.restore();
}

// ---- Legend ----------------------------------------------------------------------

pub fn legend_height(f: &Fields) -> f64 {
    7.5 + 5.5 * (f.list("rows").len().max(1)) as f64
}

fn swatch<'a>(f: &'a Fields, name: &str) -> Option<&'a Vec<(Value, Value)>> {
    match f.0.get("swatches") {
        Some(Value::Dict(d)) => d.iter().find(|(k, _)| matches!(k, Value::Str(s) if s == name)).and_then(|(_, v)| match v {
            Value::Dict(sw) => Some(sw),
            _ => None,
        }),
        _ => None,
    }
}

fn get<'a>(d: &'a [(Value, Value)], key: &str) -> Option<&'a Value> {
    d.iter().find(|(k, _)| matches!(k, Value::Str(s) if s == key)).map(|(_, v)| v)
}

pub(super) fn legend_chip(i: usize) -> [f64; 4] {
    [2.2, 7.5 + 5.5 * i as f64 + 0.8, 4.0, 3.2]
}

pub fn paint_legend(p: &mut Painter, f: &Fields, x: &Extra) {
    let w = f.num("w_mm", 55.0);
    let h = legend_height(f);
    p.fill_rect([0.0, 0.0, w, h], Rgba::WHITE);
    p.set_pen(Pen::new(INK, 0.3));
    p.set_brush(None);
    p.rect([0.0, 0.0, w, h]);
    p.text_mm([2.0, 1.4, w - 4.0, 5.0], &f.text("title", "LEYENDA"), 3.2, true, align::LEFT | align::TOP, INK);
    let mut y = 7.5;
    let mut rows: Vec<String> = f.list("rows").iter().filter_map(|v| match v {
        Value::Str(s) => Some(s.clone()),
        _ => None,
    }).collect();
    if rows.is_empty() {
        rows.push(x.text("empty"));
    }
    for (i, name) in rows.iter().enumerate() {
        let sw = swatch(f, name);
        let rgb = sw.and_then(|s| get(s, "color")).and_then(|v| match v {
            Value::List(c) | Value::Tuple(c) if c.len() >= 3 => Some([num(&c[0])?, num(&c[1])?, num(&c[2])?]),
            _ => None,
        });
        let fill = rgb.map_or(Rgba(226, 232, 238, 255), |c| Rgba::from_f(c[0], c[1], c[2]));
        p.set_brush(Some(fill));
        p.set_pen(Pen::new(INK, 0.2));
        let chip = legend_chip(i);
        p.rect(chip);
        if let Some(ops) = x.swatch_ops.get(&i) {
            p.ops.extend_from_slice(ops);
            p.set_brush(None);
            p.rect(chip);
        }
        let pattern = x.nums(&format!("pattern.{i}"));
        if !pattern.is_empty() {
            let dash: Vec<f64> = pattern.iter().map(|d| d.max(0.05) / 0.25 * 0.4).collect();
            p.set_pen(Pen::new(INK, 0.25).cap(Cap::Flat).dashed(&dash));
            p.line(2.2, y + 2.4, 6.2, y + 2.4);
        }
        p.text_mm([8.0, y + 0.7, w - 10.0, 5.0], name, 2.8, false, align::LEFT | align::TOP, INK);
        y += 5.5;
    }
}

// ---- Shapes and ground lines --------------------------------------------------------

pub fn paint_shape(p: &mut Painter, f: &Fields) {
    let kind = f.text("kind", "rect");
    let (w, h) = (f.num("w_mm", 40.0), f.num("h_mm", 25.0));
    let stroke = f.num("stroke_mm", 0.35);
    let color = Rgba::parse(&f.text("color", "#1e242c"));
    p.set_pen(Pen::new(color, stroke).cap(Cap::Round));
    let filled = f.flag("fill", false) && matches!(kind.as_str(), "rect" | "elipse" | "poligono");
    p.set_brush(filled.then(|| Rgba::parse(&f.text("fill_color", "#e2e8ee"))));
    match kind.as_str() {
        "rect" => {
            let rad = f.num("radius_mm", 0.0).min(w / 2.0).min(h / 2.0);
            if rad > 0.01 {
                p.rounded_rect([0.0, 0.0, w, h], rad, rad);
            } else {
                p.rect([0.0, 0.0, w, h]);
            }
        }
        "elipse" => p.ellipse_in([0.0, 0.0, w, h]),
        "poligono" => {
            let n = (f.num("sides", 6.0) as i64).clamp(3, 24);
            let (cx, cy) = (w / 2.0, h / 2.0);
            let pts: Vec<[f64; 2]> = (0..n).map(|i| {
                let t = 2.0 * std::f64::consts::PI * i as f64 / n as f64;
                [cx + cx * t.sin(), cy - cy * t.cos()]
            }).collect();
            p.polygon(&pts);
        }
        _ => {
            let invert = f.flag("invert", false);
            let a = [0.0, if invert { h } else { 0.0 }];
            let b = [w, if invert { 0.0 } else { h }];
            if kind == "terreno" {
                paint_ground(p, f, a, b);
            }
            p.line(a[0], a[1], b[0], b[1]);
            if kind == "flecha" {
                let ang = (b[1] - a[1]).atan2(b[0] - a[0]);
                let l = (stroke * 7.0).max(2.5);
                for da in [153f64.to_radians(), -(153f64.to_radians())] {
                    p.line(b[0], b[1], b[0] + l * (ang + da).cos(), b[1] + l * (ang + da).sin());
                }
            }
        }
    }
}

fn paint_ground(p: &mut Painter, f: &Fields, a: [f64; 2], b: [f64; 2]) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = hypot(dx, dy);
    if length < 1e-6 {
        return;
    }
    let (ux, uy) = (dx / length, dy / length);
    let (mut nx, mut ny) = (-uy, ux);
    if ny < 0.0 {
        nx = -nx;
        ny = -ny;
    }
    let step = f.num("tick_step_mm", 3.0).max(0.5);
    let tick = f.num("tick_mm", 2.5).max(0.2);
    let depth = f.num("band_mm", 6.0).max(0.5);
    let color = Rgba::parse(&f.text("color", "#1e242c"));
    let band = [a, b, [b[0] + nx * depth, b[1] + ny * depth], [a[0] + nx * depth, a[1] + ny * depth]];
    let mode = f.text("ground", "ticks");
    let mode = if mode.is_empty() { "ticks".to_string() } else { mode };
    if mode == "band" {
        let col = Rgba::parse(&f.text("fill_color", "#e2e8ee")).alpha_f(0.6);
        p.save();
        p.set_pen(Pen::none());
        p.set_brush(Some(col));
        p.polygon(&band);
        p.restore();
        return;
    }
    let pen = Pen::new(color, (f.num("stroke_mm", 0.35) * 0.6).max(0.1)).cap(Cap::Round);
    p.save();
    p.set_pen(pen.clone());
    p.set_brush(None);
    if mode == "hatch" {
        p.clip_rect([-1e6, -1e6, 2e6, 2e6]);
        p.clip_path(&[band.to_vec()]);
        let reach = depth * SQRT2;
        let mut t = -depth;
        while t < length + depth {
            let (x0, y0) = (a[0] + ux * t, a[1] + uy * t);
            p.line(x0, y0, x0 + (nx - ux) * reach / SQRT2, y0 + (ny - uy) * reach / SQRT2);
            t += step;
        }
        p.restore();
        p.save();
        p.set_pen(pen);
        p.line(a[0] + nx * depth, a[1] + ny * depth, b[0] + nx * depth, b[1] + ny * depth);
        p.restore();
        return;
    }
    let mut t = step / 2.0;
    let k = tick / SQRT2;
    while t <= length {
        let (x0, y0) = (a[0] + ux * t, a[1] + uy * t);
        p.line(x0, y0, x0 + (nx - ux) * k, y0 + (ny - uy) * k);
        t += step;
    }
    p.restore();
}

// ---- The view bubble (frame titles, callouts) ------------------------------------------

/// views/composer.py `_paint_title_bubble`.
pub fn title_bubble(p: &mut Painter, cx: f64, cy: f64, d: f64, number: &str, sheet: &str, ink: Rgba) {
    p.set_pen(Pen::new(ink, 0.35));
    p.set_brush(Some(Rgba::WHITE));
    p.ellipse(cx, cy, d / 2.0, d / 2.0);
    p.set_brush(None);
    if !sheet.is_empty() {
        p.line(cx - d / 2.0, cy, cx + d / 2.0, cy);
        p.text_mm([cx - d / 2.0, cy - d / 2.0, d, d / 2.0], number, d * 0.34, true, align::HCENTER | align::VCENTER, ink);
        p.text_mm([cx - d / 2.0, cy, d, d / 2.0], sheet, d * 0.26, false, align::HCENTER | align::VCENTER, ink);
    } else {
        p.text_mm([cx - d / 2.0, cy - d / 2.0, d, d], number, d * 0.42, true, align::HCENTER | align::VCENTER, ink);
    }
}

// ---- Callouts --------------------------------------------------------------------------

pub fn callout_bubble_mm(f: &Fields) -> f64 {
    (f.num("size_mm", 3.0) * 2.4).max(3.0)
}

fn callout_leader(f: &Fields) -> Option<[f64; 4]> {
    let (cx, cy) = (f.num("bx_mm", 36.0), f.num("by_mm", -6.0));
    let (w, h) = (f.num("w_mm", 30.0), f.num("h_mm", 20.0));
    let (px, py);
    if f.text("shape", "rect") == "circle" {
        let (ox, oy, rx, ry) = (w / 2.0, h / 2.0, w / 2.0, h / 2.0);
        let (dx, dy) = (cx - ox, cy - oy);
        if rx > 1e-9 && ry > 1e-9 && (dx / rx).powi(2) + (dy / ry).powi(2) <= 1.0 {
            return None;
        }
        let ang = (dy / ry.max(1e-9)).atan2(dx / rx.max(1e-9));
        px = ox + rx * ang.cos();
        py = oy + ry * ang.sin();
    } else {
        if (0.0..=w).contains(&cx) && (0.0..=h).contains(&cy) {
            return None;
        }
        px = cx.max(0.0).min(w);
        py = cy.max(0.0).min(h);
    }
    let (dx, dy) = (cx - px, cy - py);
    let ln = hypot(dx, dy);
    let r = callout_bubble_mm(f) / 2.0;
    if ln <= r + 0.3 {
        return None;
    }
    Some([px, py, cx - dx / ln * r, cy - dy / ln * r])
}

pub fn paint_callout(p: &mut Painter, f: &Fields, x: &Extra) {
    let color = Rgba::parse(&f.text("color", "#1e242c"));
    let stroke = f.num("stroke_mm", 0.3);
    p.set_pen(Pen::new(color, stroke).dashed(&[6.0, 3.0]).cap(Cap::Flat));
    p.set_brush(None);
    let bx = [0.0, 0.0, f.num("w_mm", 30.0), f.num("h_mm", 20.0)];
    if f.text("shape", "rect") == "circle" {
        p.ellipse_in(bx);
    } else {
        p.rounded_rect(bx, 1.5, 1.5);
    }
    let leader = callout_leader(f);
    p.set_pen(Pen::new(color, stroke));
    if let Some(l) = leader {
        p.line(l[0], l[1], l[2], l[3]);
    }
    title_bubble(p, f.num("bx_mm", 36.0), f.num("by_mm", -6.0), callout_bubble_mm(f),
                 &x.text("number"), &x.text("sheet"), color);
}

// ---- Level marks ------------------------------------------------------------------------

pub fn level_symbol_mm(f: &Fields) -> f64 {
    (f.num("size_mm", 2.5) * 1.2).max(1.5)
}

pub fn level_line_mm(f: &Fields, label: &str) -> f64 {
    let text_x = level_symbol_mm(f) / 2.0 + 0.6;
    let need = text_x + text_width_mm(label, f.num("size_mm", 2.5), false) + 1.0;
    f.num("line_mm", 14.0).max(need)
}

pub fn paint_level(p: &mut Painter, f: &Fields, x: &Extra) {
    let color = Rgba::parse(&f.text("color", "#1e242c"));
    let stroke = f.num("stroke_mm", 0.25);
    let pen = Pen::new(color, stroke).cap(Cap::Round);
    p.set_pen(pen.clone());
    p.set_brush(None);
    let s = level_symbol_mm(f);
    let mirror = f.flag("mirror", false);
    let sign = if mirror { -1.0 } else { 1.0 };
    let (ax, ay) = (f.num("ax_mm", 0.0), f.num("ay_mm", 0.0));
    if hypot(ax, ay) > 0.3 {
        p.set_pen(Pen::new(color, (stroke * 0.5).max(0.1)));
        p.line(ax, ay, 0.0, 0.0);
        p.set_pen(pen.clone());
    }
    let (line_y, text_x);
    if f.text("symbol", "triangle") == "circle" {
        let r = s / 2.0;
        p.ellipse(0.0, 0.0, r, r);
        let rings: Vec<Vec<[f64; 2]>> = [0.0, 180.0].iter().map(|start| {
            let mut ring = vec![[0.0, 0.0]];
            ring.extend(arc_points([-r, -r, 2.0 * r, 2.0 * r], *start, 90.0));
            ring
        }).collect();
        p.fill_path(&rings, color);
        line_y = -r;
        text_x = sign * (r + 0.6);
    } else {
        let half = s / 2.0;
        let hgt = s * 0.866;
        p.polygon(&[[0.0, 0.0], [-half, -hgt], [half, -hgt]]);
        line_y = -hgt;
        text_x = sign * (half + 0.6);
    }
    let label = x.text("label");
    let line = level_line_mm(f, &label);
    p.line(0.0, line_y, sign * line, line_y);
    let size = f.num("size_mm", 2.5);
    let text_h = size * 1.4;
    let (rect, flags) = if mirror {
        ([-line, line_y - text_h, line - text_x.abs(), text_h], align::RIGHT | align::BOTTOM)
    } else {
        ([text_x, line_y - text_h, line - text_x, text_h], align::LEFT | align::BOTTOM)
    };
    p.text_mm(rect, &label, size, false, flags, color);
}

// ---- Images ---------------------------------------------------------------------------------

/// The image's cut-out outline (`image_outline_path`) as a ring.
pub fn image_outline(f: &Fields) -> Vec<[f64; 2]> {
    let (w, h) = (f.num("w_mm", 60.0), f.num("h_mm", 40.0));
    match f.text("shape", "rect").as_str() {
        "ellipse" => arc_points([0.0, 0.0, w, h], 0.0, 360.0),
        "rounded" => {
            let r = f.num("radius_mm", 4.0).max(0.0).min(w / 2.0).min(h / 2.0);
            let mut ring = Vec::new();
            for (cx, cy, start) in [(w - r, r, 0.0), (r, r, 90.0), (r, h - r, 180.0), (w - r, h - r, 270.0)] {
                ring.extend(arc_points([cx - r, cy - r, 2.0 * r, 2.0 * r], start, 90.0));
            }
            ring
        }
        _ => vec![[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]],
    }
}

/// `paint_image_mm`: the picture when the file reads (the renderer loads
/// it from ``source``), else the grey placeholder with its cross.
pub fn paint_image(p: &mut Painter, f: &Fields, readable: bool) {
    let (w, h) = (f.num("w_mm", 60.0), f.num("h_mm", 40.0));
    let r = [0.0, 0.0, w, h];
    if readable {
        p.save();
        // The file and, for the writers, how the item shows it.
        let look = super::picture::Look {
            fit: f.text("fit", "stretch"), shape: f.text("shape", "rect"), radius_mm: f.num("radius_mm", 0.0),
            feather_mm: f.num("feather_mm", 0.0), opacity: f.num("opacity", 1.0), w_mm: w, h_mm: h,
        };
        p.image(r, &format!("{}\u{1f}{}", f.text("path", ""), look.json()));
        p.restore();
        if f.flag("border", false) {
            p.set_pen(Pen::new(Rgba::parse(&f.text("border_color", "#282e36")), f.num("border_mm", 0.3).max(0.1)));
            p.set_brush(None);
            p.path(&[image_outline(f)]);
        }
    } else {
        p.fill_rect(r, Rgba(240, 240, 242, 255));
        p.set_pen(Pen::new(Rgba(170, 176, 184, 255), 0.25));
        p.rect(r);
        p.line(0.0, 0.0, w, h);
        p.line(w, 0.0, 0.0, h);
    }
}
