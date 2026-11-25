// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! Dimensions (linear, angular, radial), labels with leaders and terrain
//! profiles: `paint_cota_mm`, `paint_cota_angular_mm`,
//! `paint_cota_radial_mm`, `paint_etiqueta_mm` and `paint_perfil_mm` of
//! views/composer.py with the geometry of their records in
//! core/composition.py, ported line by line.

use super::items::Extra;
use super::metrics::{laid_out_height_mm, text_baseline_rect_mm, text_bg_span_mm, text_cap_rect_mm, text_width_mm};
use super::model::Fields;
use super::paint::{align, arc_points, Cap, Painter, Pen, Rgba};
use super::pymath::hypot;
use super::svg::fmt_g;
use crate::doc::value::Value;
use std::f64::consts::PI;

const PT_TO_MM: f64 = 25.4 / 72.0;
const TEXT_BG_PAD_MM: f64 = 0.5;

/// Python's float `%` (the sign of the divisor).
pub fn py_mod(a: f64, b: f64) -> f64 {
    let r = a % b;
    if r != 0.0 && (r < 0.0) != (b < 0.0) { r + b } else { r }
}

/// built_core.composition.readable_deg.
pub fn readable_deg(dx: f64, dy: f64) -> f64 {
    py_mod(dy.atan2(dx).to_degrees() + 90.0, 180.0) - 90.0
}

/// views/composer.py oblique_tick_angle.
pub fn oblique_tick_angle(line_angle: f64) -> f64 {
    line_angle - PI / 4.0
}

fn or<'a>(s: &'a str, default: &'a str) -> &'a str {
    if s.is_empty() { default } else { s }
}

fn with_opacity(color: &str, opacity: f64) -> Rgba {
    Rgba::parse(color).alpha_f(opacity.clamp(0.0, 1.0))
}

fn arrow_head(p: &mut Painter, tip: [f64; 2], heading: f64, l: f64, base: f64, color: Rgba) {
    p.save();
    p.set_brush(Some(color));
    p.set_pen(Pen::none());
    p.polygon(&[tip, [tip[0] + l * (heading + base).cos(), tip[1] + l * (heading + base).sin()],
                [tip[0] + l * (heading - base).cos(), tip[1] + l * (heading - base).sin()]]);
    p.restore();
}

// ---- Linear dimensions -----------------------------------------------------------------

pub(super) struct Cota<'a> {
    pub(super) f: &'a Fields,
    pub(super) label: String,
}

impl Cota<'_> {
    fn dx(&self) -> f64 {
        self.f.num("dx_mm", 40.0)
    }
    fn dy(&self) -> f64 {
        self.f.num("dy_mm", 0.0)
    }
    fn text_mm(&self) -> f64 {
        self.f.num("text_mm", 2.8)
    }
    pub(super) fn normal(&self) -> (f64, f64) {
        let length = hypot(self.dx(), self.dy());
        if length < 1e-9 {
            return (0.0, -1.0);
        }
        (-self.dy() / length, self.dx() / length)
    }
    pub(super) fn line_points(&self) -> ([f64; 2], [f64; 2]) {
        let s = self.f.num("sep_mm", 0.0);
        match self.f.text("axis", "").as_str() {
            "h" => ([0.0, s], [self.dx(), s]),
            "v" => ([s, 0.0], [s, self.dy()]),
            _ => {
                let (nx, ny) = self.normal();
                ([nx * s, ny * s], [self.dx() + nx * s, self.dy() + ny * s])
            }
        }
    }
    pub(super) fn line_deg(&self) -> f64 {
        let (a, b) = self.line_points();
        readable_deg(b[0] - a[0], b[1] - a[1])
    }
    fn horizontal(&self) -> bool {
        or(&self.f.text("text_align", "aligned"), "aligned") == "horizontal"
    }
    fn tw(&self) -> f64 {
        self.label.chars().count() as f64 * self.text_mm() * 0.62 + 2.0
    }
    fn th(&self) -> f64 {
        self.text_mm() * 1.3 + 0.8
    }
    pub(super) fn label_extent_mm(&self) -> f64 {
        let tw = self.tw();
        if !self.horizontal() {
            return tw;
        }
        let d = self.line_deg().to_radians();
        tw * d.cos().abs() + self.th() * d.sin().abs()
    }
    fn along(&self) -> String {
        or(&self.f.text("text_along", "middle"), "middle").to_string()
    }
    fn dragged(&self) -> bool {
        self.f.num("text_dx_mm", 0.0).abs() > 1e-9 || self.f.num("text_dy_mm", 0.0).abs() > 1e-9
    }
    pub(super) fn text_tail(&self) -> Option<([f64; 2], [f64; 2])> {
        let along = self.along();
        if along != "start" && along != "end" {
            return None;
        }
        if self.dragged() {
            return None;
        }
        let (a, b) = self.line_points();
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let ln = hypot(dx, dy);
        if ln < 1e-9 {
            return None;
        }
        let (ux, uy) = (dx / ln, dy / ln);
        let reach = self.label_extent_mm() + 1.8;
        Some(if along == "start" {
            (a, [a[0] - ux * reach, a[1] - uy * reach])
        } else {
            (b, [b[0] + ux * reach, b[1] + uy * reach])
        })
    }
    fn label_anchor(&self) -> (f64, f64) {
        let (a, b) = self.line_points();
        let (mut mx, mut my) = ((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0);
        let along = self.along();
        if along == "start" || along == "end" {
            let (ldx, ldy) = (b[0] - a[0], b[1] - a[1]);
            let length = hypot(ldx, ldy);
            if length > 1e-9 {
                let (ux, uy) = (ldx / length, ldy / length);
                let mut shift = length / 2.0 + self.label_extent_mm() / 2.0 + 1.0;
                if along == "start" {
                    shift = -shift;
                }
                mx += ux * shift;
                my += uy * shift;
            }
        }
        (mx + self.f.num("text_dx_mm", 0.0), my + self.f.num("text_dy_mm", 0.0))
    }
    fn label_is_automatic(&self) -> bool {
        self.along() == "middle" && !self.dragged()
    }
    fn aside_frame(&self) -> (f64, f64, f64, f64, f64) {
        let mut deg = self.line_deg();
        let horizontal = self.horizontal();
        let (tw, th) = (self.tw(), self.th());
        let d = deg.to_radians();
        let (mut ux, mut uy) = (d.sin(), -d.cos());
        if self.f.text("text_pos", "") == "aside_below" {
            ux = -ux;
            uy = -uy;
        }
        let half = if horizontal { tw / 2.0 * ux.abs() + th / 2.0 * uy.abs() } else { th / 2.0 };
        let dist = self.f.num("offset_mm", 4.0) + half;
        if horizontal {
            deg = 0.0;
        } else if self.f.flag("text_flip", false) {
            deg += 180.0;
        }
        (ux * dist, uy * dist, deg, tw, th)
    }
    fn text_layout(&self) -> (String, f64) {
        let mut pos = or(&self.f.text("text_pos", "above"), "above").to_string();
        if self.horizontal() {
            return (pos, 0.0);
        }
        let mut deg = self.line_deg();
        if self.f.flag("text_flip", false) {
            deg += 180.0;
            pos = match pos.as_str() {
                "above" => "below".into(),
                "below" => "above".into(),
                _ => pos,
            };
        }
        (pos, deg)
    }
}

pub fn paint_dimension(p: &mut Painter, f: &Fields, x: &Extra) {
    let ct = Cota { f, label: x.text("label") };
    let a = [0.0, 0.0];
    let b = [ct.dx(), ct.dy()];
    let (a2, b2) = ct.line_points();
    let color = Rgba::parse(&f.text("color", "#1e242c"));
    let stroke = f.num("stroke_mm", 0.25);
    p.set_pen(Pen::new(color, stroke));
    for (q, q2) in [(a, a2), (b, b2)] {
        let (ex, ey) = (q2[0] - q[0], q2[1] - q[1]);
        let ln = hypot(ex, ey);
        if ln <= 0.05 {
            continue;
        }
        let (ux, uy) = (ex / ln, ey / ln);
        p.line(q[0] + ux, q[1] + uy, q2[0] + ux * 1.2, q2[1] + uy * 1.2);
    }
    if let Some((t0, t1)) = ct.text_tail() {
        p.line(t0[0], t0[1], t1[0], t1[1]);
    }
    let ang = (b2[1] - a2[1]).atan2(b2[0] - a2[0]);
    let label = ct.label.clone();
    let mid = [(a2[0] + b2[0]) / 2.0, (a2[1] + b2[1]) / 2.0];
    let text_pos = or(&f.text("text_pos", "above"), "above").to_string();
    let length = hypot(ct.dx(), ct.dy());
    if text_pos == "centered" && ct.label_is_automatic() {
        let (ux, uy) = (ang.cos(), ang.sin());
        let (tw, th) = (ct.tw(), ct.th());
        let extent = if ct.horizontal() { (tw * ux).abs() + (th * uy).abs() } else { tw };
        let half = extent / 2.0 + 0.5;
        if 2.0 * half < length - 2.0 {
            p.line(a2[0], a2[1], mid[0] - ux * half, mid[1] - uy * half);
            p.line(mid[0] + ux * half, mid[1] + uy * half, b2[0], b2[1]);
        } else {
            p.line(a2[0], a2[1], b2[0], b2[1]);
        }
    } else {
        p.line(a2[0], a2[1], b2[0], b2[1]);
    }
    let ends = f.text("ends", "tick");
    if ends == "arrow" {
        let l = (stroke * 6.0).max(1.8);
        for (pt, dir) in [(a2, ang), (b2, ang + PI)] {
            arrow_head(p, pt, dir, l, 12f64.to_radians(), color);
        }
    } else if ends != "none" {
        let tick = 1.6;
        let t = oblique_tick_angle(ang);
        for pt in [a2, b2] {
            p.line(pt[0] - tick * t.cos(), pt[1] - tick * t.sin(), pt[0] + tick * t.cos(), pt[1] + tick * t.sin());
        }
    }
    p.save();
    let tcol = match f.text("text_color", "").as_str() {
        "" => color,
        c => Rgba::parse(c),
    };
    let (lx, ly) = ct.label_anchor();
    let text_mm = ct.text_mm();
    let (rect, flags);
    if text_pos == "aside" || text_pos == "aside_below" {
        let (ox, oy, deg, _tw, th) = ct.aside_frame();
        p.translate(lx + ox, ly + oy);
        p.rotate(deg);
        rect = [-40.0, -th / 2.0, 80.0, th];
        flags = align::HCENTER | align::VCENTER;
    } else {
        p.translate(lx, ly);
        let (pos, deg) = ct.text_layout();
        p.rotate(deg);
        if pos == "below" {
            rect = text_cap_rect_mm(text_mm, f.num("offset_mm", 4.0), 80.0);
            flags = align::HCENTER | align::TOP;
        } else if pos == "centered" {
            rect = [-40.0, -text_mm * 0.65, 80.0, text_mm * 1.3];
            flags = align::HCENTER | align::VCENTER;
        } else {
            rect = text_baseline_rect_mm(text_mm, -f.num("offset_mm", 4.0), 80.0, false);
            flags = align::HCENTER | align::TOP;
        }
    }
    let bg = f.text("text_bg", "");
    if !bg.is_empty() && !label.is_empty() {
        let tw = ct.tw();
        p.save();
        p.set_pen(Pen::none());
        p.set_brush(Some(with_opacity(&bg, f.num("text_bg_opacity", 1.0))));
        p.rect([-tw / 2.0, rect[1] - 0.4, tw, rect[3] + 0.8]);
        p.restore();
    }
    p.text_mm(rect, &label, text_mm, false, flags, tcol);
    p.restore();
}

pub fn dimension_size(f: &Fields) -> (f64, f64) {
    (f.num("dx_mm", 40.0).abs().max(2.0), f.num("dy_mm", 0.0).abs().max(2.0))
}

// ---- Angular dimensions -------------------------------------------------------------------

pub(super) fn angles(f: &Fields) -> (f64, f64) {
    let a0 = f.num("ay_mm", 0.0).atan2(f.num("ax_mm", 30.0));
    let a1 = f.num("by_mm", -30.0).atan2(f.num("bx_mm", 0.0));
    let mut sweep = a1 - a0;
    while sweep > PI {
        sweep -= 2.0 * PI;
    }
    while sweep <= -PI {
        sweep += 2.0 * PI;
    }
    (a0, sweep)
}

pub fn paint_angular(p: &mut Painter, f: &Fields, x: &Extra) {
    let color = Rgba::parse(&f.text("color", "#1e242c"));
    let stroke = f.num("stroke_mm", 0.25);
    p.set_pen(Pen::new(color, stroke).cap(Cap::Round));
    p.set_brush(None);
    let (a0, sweep) = angles(f);
    let r = f.num("radius_mm", 15.0).max(1.0);
    for (px, py) in [(f.num("ax_mm", 30.0), f.num("ay_mm", 0.0)), (f.num("bx_mm", 0.0), f.num("by_mm", -30.0))] {
        let length = hypot(px, py);
        if length < 1e-9 {
            continue;
        }
        let reach = length.max(r + 1.2);
        p.line(0.0, 0.0, px / length * reach, py / length * reach);
    }
    p.path(&[arc_points([-r, -r, 2.0 * r, 2.0 * r], -a0.to_degrees(), -sweep.to_degrees())]);
    let a1 = a0 + sweep;
    let sgn = if sweep >= 0.0 { 1.0 } else { -1.0 };
    let ends = [(a0, sgn), (a1, -sgn)];
    let kind = f.text("ends", "arrow");
    if kind == "arrow" {
        let l = (stroke * 6.0).max(1.8);
        for (ang, d) in ends {
            let tip = [r * ang.cos(), r * ang.sin()];
            let heading = (d * ang.cos()).atan2(-d * ang.sin());
            arrow_head(p, tip, heading, l, 12f64.to_radians(), color);
        }
    } else if kind != "none" {
        let tick = 1.6;
        for (ang, _d) in ends {
            let (cx, cy) = (r * ang.cos(), r * ang.sin());
            let t = oblique_tick_angle(ang + PI / 2.0);
            p.line(cx - tick * t.cos(), cy - tick * t.sin(), cx + tick * t.cos(), cy + tick * t.sin());
        }
    }
    let am = a0 + sweep / 2.0;
    let text_mm = f.num("text_mm", 2.8);
    let d = r + f.num("offset_mm", 0.8) + text_mm * 0.75;
    let (lx, ly) = (d * am.cos(), d * am.sin());
    let tcol = match f.text("text_color", "").as_str() {
        "" => color,
        c => Rgba::parse(c),
    };
    let label = x.text("label");
    let bg = f.text("text_bg", "");
    if !bg.is_empty() && !label.is_empty() {
        let tw = label.chars().count() as f64 * text_mm * 0.62 + 2.0;
        p.save();
        p.set_pen(Pen::none());
        p.set_brush(Some(with_opacity(&bg, f.num("text_bg_opacity", 1.0))));
        p.rect([lx - tw / 2.0, ly - text_mm * 0.75, tw, text_mm * 1.5]);
        p.restore();
    }
    p.text_mm([lx - 20.0, ly - text_mm * 0.65, 40.0, text_mm * 1.3], &label, text_mm, false,
              align::HCENTER | align::VCENTER, tcol);
}

pub fn angular_size(f: &Fields) -> (f64, f64) {
    let r = f.num("radius_mm", 15.0);
    (f.num("ax_mm", 30.0).abs().max(f.num("bx_mm", 0.0).abs()).max(r).max(2.0),
     f.num("ay_mm", 0.0).abs().max(f.num("by_mm", -30.0).abs()).max(r).max(2.0))
}

// ---- Radius and diameter ---------------------------------------------------------------------

pub(super) struct Radial<'a> {
    pub(super) f: &'a Fields,
    pub(super) label: String,
}

impl Radial<'_> {
    fn r(&self) -> f64 {
        self.f.num("radius_mm", 15.0)
    }
    fn diameter(&self) -> bool {
        self.f.text("kind", "radius") == "diameter"
    }
    fn text_mm(&self) -> f64 {
        self.f.num("text_mm", 2.8)
    }
    pub(super) fn heading(&self) -> (f64, f64) {
        let a = self.f.num("angle_deg", -30.0).to_radians();
        (a.cos(), a.sin())
    }
    pub(super) fn line_points(&self) -> ([f64; 2], [f64; 2]) {
        let (ux, uy) = self.heading();
        let r = self.r();
        if self.diameter() {
            ([-ux * r, -uy * r], [ux * r, uy * r])
        } else {
            ([0.0, 0.0], [ux * r, uy * r])
        }
    }
    pub(super) fn label_width_mm(&self) -> f64 {
        self.label.chars().count() as f64 * self.text_mm() * 0.62 + 2.0
    }
    pub(super) fn outside(&self) -> bool {
        let placement = self.f.text("placement", "auto");
        if placement == "in" || placement == "out" {
            return placement == "out";
        }
        let (a, b) = self.line_points();
        let room = hypot(b[0] - a[0], b[1] - a[1]);
        let arrow = (self.f.num("stroke_mm", 0.25) * 6.0).max(1.8) * if self.diameter() { 2.0 } else { 1.0 };
        self.label_width_mm() + arrow + 1.0 > room
    }
    pub(super) fn text_anchor(&self) -> (f64, f64) {
        let (ux, uy) = self.heading();
        if !self.outside() {
            if self.diameter() {
                let d = readable_deg(ux, uy).to_radians();
                let h = self.r() / 2.0;
                return (d.cos() * h, d.sin() * h);
            }
            let (a, b) = self.line_points();
            return ((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0);
        }
        let reach = self.r() + self.label_width_mm() / 2.0 + 2.0;
        (ux * reach, uy * reach)
    }
    pub(super) fn tail_end(&self) -> (f64, f64) {
        let (ux, uy) = self.heading();
        if !self.outside() {
            return (ux * self.r(), uy * self.r());
        }
        let reach = self.r() + self.label_width_mm() + 2.5;
        (ux * reach, uy * reach)
    }
}

pub fn paint_radial(p: &mut Painter, f: &Fields, x: &Extra) {
    let cr = Radial { f, label: x.text("label") };
    let color = Rgba::parse(&f.text("color", "#1e242c"));
    let stroke = f.num("stroke_mm", 0.25);
    p.set_pen(Pen::new(color, stroke).cap(Cap::Round));
    p.set_brush(None);
    let (a, b) = cr.line_points();
    let (tx, ty) = cr.tail_end();
    let out = cr.outside();
    p.line(a[0], a[1], b[0], b[1]);
    if out {
        p.line(b[0], b[1], tx, ty);
    }
    if f.flag("centre_mark", true) {
        let m = (cr.text_mm() * 0.45).max(1.0);
        p.line(-m, 0.0, m, 0.0);
        p.line(0.0, -m, 0.0, m);
    }
    let (ux, uy) = cr.heading();
    let mut tips = vec![(b, if out { (-ux, -uy) } else { (ux, uy) })];
    if cr.diameter() {
        tips.push((a, if out { (ux, uy) } else { (-ux, -uy) }));
    }
    let ends = f.text("ends", "arrow");
    if ends == "arrow" {
        let l = (stroke * 6.0).max(1.8);
        let base = 12f64.to_radians();
        p.save();
        p.set_brush(Some(color));
        p.set_pen(Pen::none());
        for (tip, (hx, hy)) in &tips {
            let head = hy.atan2(*hx) + PI;
            p.polygon(&[*tip, [tip[0] + l * (head + base).cos(), tip[1] + l * (head + base).sin()],
                        [tip[0] + l * (head - base).cos(), tip[1] + l * (head - base).sin()]]);
        }
        p.restore();
    } else if ends != "none" {
        let t = 1.6;
        let ang = oblique_tick_angle(uy.atan2(ux));
        for (pt, _h) in &tips {
            p.line(pt[0] - t * ang.cos(), pt[1] - t * ang.sin(), pt[0] + t * ang.cos(), pt[1] + t * ang.sin());
        }
    }
    let (lx, ly) = cr.text_anchor();
    let tcol = match f.text("text_color", "").as_str() {
        "" => color,
        c => Rgba::parse(c),
    };
    p.save();
    p.translate(lx, ly);
    p.rotate(readable_deg(ux, uy));
    let rect = text_baseline_rect_mm(cr.text_mm(), -f.num("offset_mm", 0.5), 80.0, false);
    let bg = f.text("text_bg", "");
    if !bg.is_empty() && !cr.label.is_empty() {
        let tw = cr.label_width_mm();
        p.set_pen(Pen::none());
        p.set_brush(Some(with_opacity(&bg, f.num("text_bg_opacity", 1.0))));
        p.rect([-tw / 2.0, rect[1] - 0.4, tw, rect[3] + 0.8]);
    }
    p.text_mm(rect, &cr.label, cr.text_mm(), false, align::HCENTER | align::TOP, tcol);
    p.restore();
}

pub fn radial_size(f: &Fields) -> (f64, f64) {
    let d = (2.0 * f.num("radius_mm", 15.0)).max(2.0);
    (d, d)
}

// ---- Labels with leaders ------------------------------------------------------------------------

struct Label<'a> {
    f: &'a Fields,
    text: String,
}

impl Label<'_> {
    fn size_mm(&self) -> f64 {
        self.f.num("size_pt", 11.0) * PT_TO_MM
    }
    fn bold(&self) -> bool {
        self.f.flag("bold", false)
    }
    fn w(&self) -> f64 {
        self.f.num("w_mm", 50.0)
    }
    fn block_h(&self) -> f64 {
        let h = laid_out_height_mm(&self.text, self.w(), self.size_mm(), self.bold());
        (self.size_mm() * 1.15).max(h)
    }
    fn ink_w(&self) -> f64 {
        let widest = self.text.split('\n').map(|l| text_width_mm(l, self.size_mm(), self.bold())).fold(0.0, f64::max);
        2f64.max(self.w().min(widest + 0.2))
    }
    fn bg_rect(&self) -> [f64; 4] {
        let (y0, y1) = text_bg_span_mm(&self.text, self.w(), self.size_mm(), self.bold());
        [-TEXT_BG_PAD_MM, y0, self.ink_w() + 2.0 * TEXT_BG_PAD_MM, (y1 - y0).max(1.0)]
    }
    fn leader_start(&self, (ax, ay): (f64, f64)) -> (f64, f64) {
        let (tw, h) = (self.ink_w(), self.block_h());
        let (cx, cy) = (tw / 2.0, h / 2.0);
        if ax < 0.0 {
            return (-0.8, cy);
        }
        if ax > tw {
            return (tw + 0.8, cy);
        }
        if ay < 0.0 {
            return (cx, -0.8);
        }
        (cx, h + 0.8)
    }
    fn spots(&self) -> Vec<(f64, f64)> {
        let mut out = vec![(self.f.num("ax_mm", -20.0), self.f.num("ay_mm", 15.0))];
        for ld in self.f.list("leaders") {
            if let Value::Dict(d) = ld {
                let get = |k: &str| d.iter().find(|(kk, _)| matches!(kk, Value::Str(s) if s == k)).and_then(|(_, v)| match v {
                    Value::Float(x) => Some(*x),
                    Value::Int(i) => Some(*i as f64),
                    _ => None,
                }).unwrap_or(0.0);
                out.push((get("ax_mm"), get("ay_mm")));
            }
        }
        out
    }
}

pub fn paint_label(p: &mut Painter, f: &Fields, x: &Extra) {
    let et = Label { f, text: x.text("text") };
    let size = et.size_mm();
    let h = et.block_h();
    let color = Rgba::parse(&f.text("color", "#1e242c"));
    let stroke = f.num("stroke_mm", 0.25);
    let pen = Pen::new(color, stroke).cap(Cap::Round);
    let dot_r = if f.flag("dot", true) { (stroke * 1.8).max(0.45) } else { 0.0 };
    for (ax, ay) in et.spots() {
        let (sx, sy) = et.leader_start((ax, ay));
        p.set_pen(pen.clone());
        p.set_brush(None);
        p.line(sx, sy, ax, ay);
        if dot_r != 0.0 {
            p.save();
            p.set_pen(Pen::none());
            p.set_brush(Some(color));
            p.ellipse(sx, sy, dot_r, dot_r);
            p.restore();
        }
        if f.flag("arrow", true) {
            let ang = (sy - ay).atan2(sx - ax);
            let l = (stroke * 7.0).max(1.8);
            arrow_head(p, [ax, ay], ang, l, 14f64.to_radians(), color);
        }
    }
    let bg = f.text("bg_color", "");
    if !bg.is_empty() {
        p.save();
        p.set_pen(Pen::none());
        p.set_brush(Some(with_opacity(&bg, f.num("bg_opacity", 1.0))));
        p.rect(et.bg_rect());
        p.restore();
    }
    p.draw_text_mm([0.0, 0.0, et.w(), h + size], &et.text, size, et.bold(), align::LEFT | align::TOP, color,
                   f.flag("italic", false), f.flag("underline", false));
}

/// EtiquetaItem's box: its wrap width, 1.4 × the size per typed line (6 mm at least).
pub fn label_size(f: &Fields) -> (f64, f64) {
    let lines = f.text("text", "").matches('\n').count() as f64 + 1.0;
    (f.num("w_mm", 50.0), (f.num("size_pt", 11.0) * PT_TO_MM * 1.4 * lines).max(6.0))
}

// ---- Terrain profiles ------------------------------------------------------------------------------

/// views/composer.py `_nice_ticks`.
fn nice_ticks(lo: f64, hi: f64, target: i32) -> Vec<f64> {
    if hi <= lo {
        return vec![lo];
    }
    let raw = (hi - lo) / target as f64;
    let mag = 10f64.powi(raw.log10().floor() as i32);
    let mut step = mag;
    for m in [1.0, 2.0, 2.5, 5.0, 10.0] {
        step = m * mag;
        if raw <= step {
            break;
        }
    }
    let start = (lo / step).ceil() * step;
    let mut ticks = Vec::new();
    let mut v = start;
    while v <= hi + 1e-6 {
        ticks.push(format!("{v:.6}").parse().unwrap_or(v));
        v += step;
    }
    if ticks.is_empty() { vec![lo, hi] } else { ticks }
}

/// `chainage_step`.
pub fn chainage_step(length: f64, step_m: f64) -> f64 {
    if step_m > 0.0 {
        return step_m;
    }
    let t = nice_ticks(0.0, length, 6);
    if t.len() > 1 { t[1] - t[0] } else if length != 0.0 { length } else { 1.0 }
}

/// `_chainage`: «1+250».
pub fn chainage(s: f64, step: f64) -> String {
    let km = (s / 1000.0).floor() as i64;
    let m = py_mod(s, 1000.0);
    if step >= 1.0 { format!("{km}+{m:03.0}") } else { format!("{km}+{m:06.2}") }
}

pub fn paint_profile(p: &mut Painter, f: &Fields, x: &Extra) {
    let (w, h) = (f.num("w_mm", 180.0), f.num("h_mm", 70.0));
    let ink = Rgba::INK;
    let grey = Rgba(90, 98, 110, 255);
    p.set_pen(Pen::new(ink, 0.25));
    p.set_brush(None);
    p.rect([0.0, 0.0, w, h]);
    let t = f.num("text_mm", 2.4).max(1.0);
    p.text_mm([2.0, 1.0, w - 4.0, t * 1.7], &x.text("title"), t * 1.15, true, align::LEFT | align::TOP, ink);
    let samples: Vec<(f64, Option<f64>)> = match x.values.get("samples") {
        Some(Value::List(v)) => v.iter().filter_map(|s| match s {
            Value::List(pair) | Value::Tuple(pair) if pair.len() == 2 => {
                let st = match &pair[0] { Value::Float(f) => *f, Value::Int(i) => *i as f64, _ => return None };
                let el = match &pair[1] { Value::Float(f) => Some(*f), Value::Int(i) => Some(*i as f64), _ => None };
                Some((st, el))
            }
            _ => None,
        }).collect(),
        _ => Vec::new(),
    };
    let els: Vec<f64> = samples.iter().filter_map(|s| s.1).collect();
    if samples.is_empty() || els.is_empty() {
        p.text_mm([2.0, h / 2.0 - t, w - 4.0, t * 2.2], &x.text("waiting"), t, false, align::CENTER, grey);
        return;
    }
    let (left, right) = (4.0 + t * 4.2, w - 3.0);
    let (top, bottom) = (t * 1.7 + 3.0 + t * 1.4, h - (t * 2.2 + 3.0));
    let (pw, ph) = (right - left, bottom - top);
    if pw < 10.0 || ph < 8.0 {
        return;
    }
    let length = match x.values.get("length") {
        Some(Value::Float(l)) if *l != 0.0 => *l,
        _ => 1.0,
    };
    let (mut elo, mut ehi) = (els.iter().copied().fold(f64::INFINITY, f64::min), els.iter().copied().fold(f64::NEG_INFINITY, f64::max));
    if ehi - elo < 1.0 {
        elo -= 1.0;
        ehi += 1.0;
    }
    let v_ticks = nice_ticks(elo, ehi, 4);
    let gv = f.num("grid_v_m", 0.0);
    let step_v = if gv != 0.0 { gv } else if v_ticks.len() > 1 { v_ticks[1] - v_ticks[0] } else { 1.0 };
    let base = (elo / step_v).floor() * step_v;
    let mut topv = (ehi / step_v).ceil() * step_v;
    if topv <= base {
        topv = base + step_v;
    }
    let mut fitted_h = false;
    let scale_n = f.num("scale_n", 0.0);
    let mut mm_per_m_h;
    if scale_n > 0.0 {
        mm_per_m_h = 1000.0 / scale_n;
        if length * mm_per_m_h > pw {
            mm_per_m_h = pw / length;
            fitted_h = true;
        }
    } else {
        mm_per_m_h = pw / length;
    }
    let exag = f.num("exag", 0.0);
    let mut mm_per_m_v;
    if exag > 0.0 {
        mm_per_m_v = mm_per_m_h * exag;
        if (topv - base) * mm_per_m_v > ph {
            mm_per_m_v = ph / (topv - base);
        }
    } else {
        mm_per_m_v = ph / (topv - base);
    }
    let exag_eff = if mm_per_m_h > 0.0 { mm_per_m_v / mm_per_m_h } else { 1.0 };
    let sx = |s: f64| left + s * mm_per_m_h;
    let sy = |e: f64| bottom - (e - base) * mm_per_m_v;
    let plot_right = sx(length);
    let plot_top = sy(topv);
    let step_h = chainage_step(length, f.num("grid_h_m", 0.0));
    let light = Pen::new(Rgba(200, 206, 214, 255), 0.12);
    let grid = f.flag("grid", true);
    let mut s_val = 0.0;
    while s_val <= length + 1e-6 {
        let xx = sx(s_val);
        if grid {
            p.set_pen(light.clone());
            p.line(xx, plot_top, xx, bottom);
        }
        p.text_mm([xx - 12.0, bottom + 0.8, 24.0, t * 1.4], &chainage(s_val, step_h), t, false, align::HCENTER | align::TOP, grey);
        s_val += step_h;
    }
    let mut e_val = base;
    while e_val <= topv + 1e-6 {
        let y = sy(e_val);
        if grid {
            p.set_pen(light.clone());
            p.line(left, y, plot_right, y);
        }
        p.text_mm([1.0, y - t * 0.7, left - 2.0, t * 1.4], &fmt_g(e_val), t, false, align::RIGHT | align::VCENTER, grey);
        e_val += step_v;
    }
    let mut runs: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut cur: Vec<[f64; 2]> = Vec::new();
    for (st, el) in &samples {
        match el {
            None => {
                if !cur.is_empty() {
                    runs.push(std::mem::take(&mut cur));
                }
            }
            Some(e) => cur.push([sx(*st), sy(*e)]),
        }
    }
    if !cur.is_empty() {
        runs.push(cur);
    }
    for run in &runs {
        if run.len() < 2 {
            continue;
        }
        if f.flag("fill", true) {
            let mut poly = run.clone();
            poly.push([run[run.len() - 1][0], bottom]);
            poly.push([run[0][0], bottom]);
            p.set_pen(Pen::none());
            p.set_brush(Some(Rgba(120, 170, 110, 60)));
            p.polygon(&poly);
            p.set_brush(None);
        }
        p.set_pen(Pen::new(Rgba(60, 110, 50, 255), 0.4));
        p.polyline(run);
    }
    p.set_pen(Pen::new(ink, 0.3));
    p.line(left, plot_top, left, bottom);
    p.line(left, bottom, plot_right, bottom);
    let mut cap = x.text("caption")
        .replace("{h}", &format!("{:.0}", 1000.0 / mm_per_m_h))
        .replace("{v}", &format!("{:.0}", 1000.0 / mm_per_m_v))
        .replace("{k}", &format!("{exag_eff:.1}"));
    if fitted_h {
        cap += &format!("  {}", x.text("fitted"));
    }
    let message = x.text("message");
    if !message.is_empty() {
        cap += &format!("  {message}");
    }
    p.text_mm([2.0, t * 1.7 + 1.6, w - 4.0, t * 1.4], &cap, t * 0.9, false, align::LEFT | align::TOP, grey);
}
