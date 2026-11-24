// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! The view frame's painter, ported from views/composer.py `paint_frame_mm`
//! and its helpers: the missing-scene notice, the vector drawing (the
//! frame's cut fills, solid or 45° hatch, and the line classes with their
//! pens), the picture of a shaded frame, the placeholder, the graticule,
//! the border, the title in its three styles (`_paint_view_title_mm`,
//! `view_title_extent`) and the scale label.
//!
//! Not yet here (the classic composer still draws them): layer pens and
//! the words of a line type (`styles`), layer hatch patterns and fills by
//! layer (`fill_styles`), and the annotations overlay.

use super::items::{title_bubble, Extra};
use super::metrics::text_width_mm;
use super::model::Fields;
use super::paint::{align, Cap, Painter, Pen, Rgba};

const INK: Rgba = Rgba(30, 36, 44, 255);
const RENDER_DPI: f64 = 300.0;
/// core.hlr line classes.
pub const KIND_EDGE: i8 = 0;
pub const KIND_PROFILE: i8 = 1;
pub const KIND_CUT: i8 = 2;
pub const KIND_HIDDEN: i8 = 3;

/// What fills the frame: worked out elsewhere (sheets::geometry and the
/// hidden-line pass, the renderer), handed to the painter.
#[derive(Default, Clone, Debug)]
pub struct FrameContent {
    /// The notice for a scene the model no longer has.
    pub missing: Option<String>,
    /// The vector drawing's segments (frame mm) and their classes; None
    /// until worked out.
    pub segs: Option<Vec<[f64; 4]>>,
    pub kinds: Option<Vec<i8>>,
    /// The section-cut rings to fill (frame mm).
    pub fills: Vec<Vec<[f64; 2]>>,
    /// A shaded frame's picture: its pixel size and where the writer finds
    /// it (`frame:<key>`).
    pub image: Option<(u32, u32, String)>,
}

fn or(s: String, default: &str) -> String {
    if s.is_empty() { default.to_string() } else { s }
}

/// `vector_pens`: the pen of each line class.
pub fn vector_pen(f: &Fields, kind: i8) -> Pen {
    let edge = f.num("pen_edge_mm", 0.18);
    let w = match kind {
        KIND_PROFILE => f.num("pen_profile_mm", 0.35),
        KIND_CUT => f.num("pen_cut_mm", 0.5),
        _ => edge,
    }.max(0.05);
    let pen = Pen::new(INK, w).cap(Cap::Round);
    if kind == KIND_HIDDEN {
        // Dashed in paper mm whatever the pen: 1.5 mm dash, 0.8 gap.
        return pen.cap(Cap::Flat).dashed(&[1.5 / w, 0.8 / w]);
    }
    pen
}

/// `hatch_pen_mm`.
pub fn hatch_pen_mm(f: &Fields) -> f64 {
    let own = f.num("pen_hatch_mm", 0.0);
    if own > 0.0 {
        return own.max(0.03);
    }
    (0.7 * f.num("pen_edge_mm", 0.18)).max(0.03)
}

/// `_paint_hlr_lines_mm` without layer styles or a budget: thin → profile
/// → cut, the heavy lines on top.
fn paint_lines(p: &mut Painter, f: &Fields, segs: &[[f64; 4]], kinds: Option<&[i8]>) {
    let groups: Vec<(i8, Vec<[f64; 4]>)> = match kinds.filter(|k| k.len() == segs.len()) {
        None => vec![(KIND_EDGE, segs.to_vec())],
        Some(k) => [KIND_HIDDEN, KIND_EDGE, KIND_PROFILE, KIND_CUT].iter()
            .map(|&kind| (kind, segs.iter().zip(k).filter(|(_, &c)| c == kind).map(|(s, _)| *s).collect()))
            .collect(),
    };
    for (kind, rows) in groups {
        if rows.is_empty() {
            continue;
        }
        p.set_pen(vector_pen(f, kind));
        for s in rows {
            p.line(s[0], s[1], s[2], s[3]);
        }
    }
}

/// `_fill_cut_rings` the frame's own way (no layer look): solid, or 45°
/// lines phase-locked to the frame.
fn paint_cut_fills(p: &mut Painter, f: &Fields, fills: &[Vec<[f64; 2]>]) {
    let mode = or(f.text("cut_fill", "solid"), "solid");
    if fills.is_empty() || mode == "none" {
        return;
    }
    let rings: Vec<Vec<[f64; 2]>> = fills.iter().filter(|r| r.len() >= 3).cloned().collect();
    if rings.is_empty() {
        return;
    }
    let color = Rgba::parse(&or(f.text("cut_fill_color", ""), "#595e69"));
    p.save();
    if mode == "hatch" {
        p.clip_path(&rings);
        let step = f.num("cut_hatch_mm", 1.5).max(0.3);
        p.set_pen(Pen::new(color, hatch_pen_mm(f)));
        let (mut l, mut t, mut r, mut b) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for q in rings.iter().flatten() {
            l = l.min(q[0]);
            t = t.min(q[1]);
            r = r.max(q[0]);
            b = b.max(q[1]);
        }
        let mut c = ((l + t) / step).floor() * step;
        let c_end = r + b;
        while c <= c_end {
            let xa = l.max(c - b);
            let xb = r.min(c - t);
            if xb > xa {
                p.line(xa, c - xa, xb, c - xb);
            }
            c += step;
        }
    } else {
        p.set_pen(Pen::none());
        p.set_brush(Some(color));
        p.path(&rings);
    }
    p.restore();
}

/// `view_title_extent`: paper the title adds as (left, top, bottom) mm.
pub fn title_extent(f: &Fields, x: &Extra) -> (f64, f64, f64) {
    if !f.flag("show_title", false) {
        return (0.0, 0.0, 0.0);
    }
    let size = f.num("title_mm", 4.0).max(1.5);
    let style = or(f.text("title_style", "layout"), "layout");
    let (subtitle, scale, number) = (x.text("subtitle"), x.text("scale"), x.text("number"));
    if style == "bar" {
        let mut w = 2.0 + size * 1.5;
        if !subtitle.is_empty() {
            w += size * 1.1;
        }
        if !scale.is_empty() {
            w += size * 1.1;
        }
        return (w + 1.0, 0.0, 0.0);
    }
    let h = if style == "simple" {
        1.2 + size * 1.6 + if subtitle.is_empty() { 0.0 } else { size }
    } else {
        let bubble = if number.is_empty() { 0.0 } else { size * 2.4 };
        1.2 + bubble.max(size * 1.5) + 0.6 + if subtitle.is_empty() { 0.0 } else { size + 0.6 }
    };
    if or(f.text("title_pos", "below"), "below") == "above" { (0.0, h, 0.0) } else { (0.0, 0.0, h) }
}

/// `_paint_view_title_mm`.
fn paint_title(p: &mut Painter, f: &Fields, x: &Extra) {
    let size = f.num("title_mm", 4.0).max(1.5);
    let style = or(f.text("title_style", "layout"), "layout");
    let al = or(f.text("title_align", "left"), "left");
    let (title, subtitle, scale) = (x.text("title"), x.text("subtitle"), x.text("scale"));
    let (number, sheet) = (x.text("number"), x.text("sheet"));
    let (w, h) = (f.num("w_mm", 100.0), f.num("h_mm", 100.0));
    let (left, top, bottom) = title_extent(f, x);
    if style == "bar" {
        let strip_w = left - 1.0;
        p.set_pen(Pen::new(INK, 0.25));
        p.set_brush(None);
        p.rect([-left, 0.0, strip_w, h]);
        p.save();
        // Turn the strip: local x runs up the page, local y toward the frame.
        p.translate(-left, h);
        p.rotate(-90.0);
        let mut x0 = 0.0;
        if !number.is_empty() {
            let d = strip_w.min(size * 2.4) * 0.9;
            p.save();
            p.rotate(90.0); // the bubble reads upright
            title_bubble(p, strip_w / 2.0, -(d / 2.0 + 1.0), d, &number, &sheet, INK);
            p.restore();
            x0 = d + 2.0;
        }
        let mut cols = vec![(title, size, true, size * 1.5)];
        if !subtitle.is_empty() {
            cols.push((subtitle, size * 0.75, false, size * 1.1));
        }
        if !scale.is_empty() {
            cols.push((scale, size * 0.75, false, size * 1.1));
        }
        let mut y = 1.0;
        for (text, sz, bold, col_h) in cols {
            p.text_mm([x0 + 1.0, y, h - x0 - 2.0, col_h], &text, sz, bold, align::LEFT | align::VCENTER, INK);
            y += col_h;
        }
        p.restore();
        return;
    }
    let above = or(f.text("title_pos", "below"), "below") == "above";
    let block_h = if above { top } else { bottom };
    let by = if above { -block_h } else { h + 1.2 };
    if style == "simple" {
        let text = if scale.is_empty() { title } else { format!("{title} — {}", scale.chars().skip(5).collect::<String>()) };
        let qa = match al.as_str() {
            "left" => align::LEFT,
            "right" => align::RIGHT,
            _ => align::HCENTER,
        };
        p.text_mm([0.0, by, w, size * 1.6], &text, size, true, qa | align::TOP, INK);
        if !subtitle.is_empty() {
            p.text_mm([0.0, by + size * 1.6, w, size], &subtitle, size * 0.7, false, qa | align::TOP, INK);
        }
        return;
    }
    // "layout": bubble + title + scale over a rule, subtitle under it.
    let d = if number.is_empty() { 0.0 } else { size * 2.4 };
    let gap = size * 0.5;
    let title_w = text_width_mm(&title, size, true);
    let scale_w = text_width_mm(&scale, size * 0.8, false);
    let text_w = title_w + if scale.is_empty() { 0.0 } else { gap + scale_w };
    let lead = if d != 0.0 { d + gap } else { 0.0 };
    let total = lead + text_w;
    let x_b = match al.as_str() {
        "right" => (w - total).max(0.0),
        "center" => ((w - total) / 2.0).max(0.0),
        _ => 0.0,
    };
    let x_text = x_b + lead;
    let line_h = d.max(size * 1.5);
    let rule_y = by + line_h;
    if d != 0.0 {
        title_bubble(p, x_b + d / 2.0, rule_y - d / 2.0, d, &number, &sheet, INK);
    }
    p.text_mm([x_text, rule_y - size * 1.5, w, size * 1.5], &title, size, true, align::LEFT | align::BOTTOM, INK);
    if !scale.is_empty() {
        p.text_mm([x_text + title_w + gap, rule_y - size * 1.5, w, size * 1.5 - size * 0.08], &scale, size * 0.8, false,
                  align::LEFT | align::BOTTOM, INK);
    }
    p.set_pen(Pen::new(INK, 0.35));
    let x_rule = if al == "left" && d != 0.0 { x_b + d } else { 0.0 };
    p.line(x_rule, rule_y + 0.2, w, rule_y + 0.2);
    if !subtitle.is_empty() {
        p.text_mm([x_text, rule_y + 0.6, w, size], &subtitle, size * 0.7, false, align::LEFT | align::TOP, INK);
    }
}

/// `_paint_scale_label_mm`.
fn paint_scale_label(p: &mut Painter, f: &Fields, x: &Extra) {
    let size = f.num("scale_mm", 3.0).max(1.5);
    let text = x.text("scale_label");
    let pos = or(f.text("scale_pos", "under-right"), "under-right");
    let (w, h_frame) = (f.num("w_mm", 100.0), f.num("h_mm", 100.0));
    let h = size * 1.4;
    let ink = Rgba(40, 46, 54, 255);
    if pos.starts_with("under") {
        let a = if pos == "under-left" { align::LEFT } else { align::RIGHT };
        p.text_mm([0.0, h_frame + 1.0, w, h], &text, size, true, a | align::TOP, ink);
        return;
    }
    let pad = 1.0;
    let bw = text.chars().count() as f64 * size * 0.62 + 2.0 * pad;
    let bx = if pos == "inside-bl" { pad } else { w - bw - pad };
    let rect = [bx, h_frame - h - pad, bw, h];
    p.save();
    p.clip_rect([0.0, 0.0, w, h_frame]);
    p.set_pen(Pen::none());
    p.set_brush(Some(Rgba(255, 255, 255, 220)));
    p.rect(rect);
    p.text_mm(rect, &text, size, true, align::HCENTER | align::VCENTER, ink);
    p.restore();
}

/// `paint_frame_mm` (printed: no screen-only guide). ``x``: the title's
/// strings (`title`, `subtitle`, `scale`, `number`, `sheet`), the scale
/// label and the translated notices (`placeholder`).
pub fn paint_frame(p: &mut Painter, f: &Fields, x: &Extra, c: &FrameContent) {
    let (w, h) = (f.num("w_mm", 100.0), f.num("h_mm", 100.0));
    let r = [0.0, 0.0, w, h];
    if let Some(missing) = &c.missing {
        p.fill_rect(r, Rgba(253, 246, 236, 255));
        p.save();
        p.set_pen(Pen::new(Rgba(214, 120, 40, 255), 0.35).dash_line());
        p.set_brush(None);
        p.rect(r);
        p.restore();
        p.text_mm([3.0, 3.0, w - 6.0, h - 6.0], missing, 3.5, false, align::LEFT | align::TOP, Rgba(170, 90, 20, 255));
        return;
    }
    let placeholder = |p: &mut Painter| {
        p.text_mm([2.0, 2.0, w - 4.0, h - 4.0], &x.text("placeholder"), 3.5, false, align::LEFT | align::TOP,
                  Rgba(140, 150, 160, 255));
    };
    if f.text("style", "") == "vectorial" {
        p.fill_rect(r, Rgba(255, 255, 255, 255));
        match &c.segs {
            Some(segs) if !segs.is_empty() => {
                p.save();
                p.clip_rect(r);
                paint_cut_fills(p, f, &c.fills);
                paint_lines(p, f, segs, c.kinds.as_deref());
                p.restore();
            }
            _ => placeholder(p),
        }
    } else if let Some((wpx, hpx, source)) = &c.image {
        // Never stretched: drawn at the size it was rendered for, clipped.
        let img = [0.0, 0.0, f64::from(*wpx) * 25.4 / RENDER_DPI, f64::from(*hpx) * 25.4 / RENDER_DPI];
        if (img[2] - w).abs() < 0.1 && (img[3] - h).abs() < 0.1 {
            p.image(r, source);
        } else {
            p.fill_rect(r, Rgba(255, 255, 255, 255));
            p.save();
            p.clip_rect(r);
            p.image(img, source);
            p.restore();
        }
    } else {
        p.fill_rect(r, Rgba(245, 246, 248, 255));
        placeholder(p);
    }
    let grid = f.num("grid_m", 0.0);
    if grid > 0.0 && !f.flag("perspective", false) {
        let model_h = h * f.num("scale_n", 100.0) / 1000.0;
        let step = grid * h / model_h;
        if step >= 2.0 {
            p.save();
            p.clip_rect(r);
            p.set_pen(Pen::new(Rgba(90, 140, 190, 120), 0.12));
            let mut gx = step;
            while gx < w {
                p.line(gx, 0.0, gx, h);
                gx += step;
            }
            let mut gy = step;
            while gy < h {
                p.line(0.0, gy, w, gy);
                gy += step;
            }
            p.restore();
        }
    }
    if f.flag("border", false) {
        p.set_pen(Pen::new(Rgba::parse(&f.text("border_color", "#282e36")), f.num("border_mm", 0.3).max(0.1)));
        p.set_brush(None);
        p.rect(r);
    }
    if f.flag("show_title", false) {
        paint_title(p, f, x);
    }
    if f.flag("show_scale", false) {
        paint_scale_label(p, f, x);
    }
}
