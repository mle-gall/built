// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! Painting a sheet item's display list (ingetrazo_core sheets::paint):
//! lines, polygons, ellipses, rounded rectangles, paths, text in a box and
//! pictures, in paper mm, onto the page at its zoom. The colours are the
//! drawing's own.

use egui::{pos2, vec2, Color32, FontId, Mesh, Pos2, Rect, Shape, Stroke};
use serde_json::Value as J;
use std::collections::HashMap;

/// Where the item's frame lands on screen and how big a millimetre is.
#[derive(Clone, Copy)]
pub struct Place {
    pub origin: Pos2,
    pub zoom: f32,
}

impl Place {
    fn pt(&self, p: &J) -> Pos2 {
        pos2(self.origin.x + p[0].as_f64().unwrap_or(0.0) as f32 * self.zoom,
             self.origin.y + p[1].as_f64().unwrap_or(0.0) as f32 * self.zoom)
    }
    fn pts(&self, v: &J) -> Vec<Pos2> {
        v.as_array().map(|a| a.iter().map(|p| self.pt(p)).collect()).unwrap_or_default()
    }
}

/// `#rrggbbaa` (the record's colours).
pub fn rgba(s: &str) -> Color32 {
    let h = s.trim_start_matches('#');
    let v = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("ff"), 16).unwrap_or(0);
    if h.len() >= 8 {
        Color32::from_rgba_unmultiplied(v(0), v(2), v(4), v(6))
    } else {
        Color32::from_rgb(v(0), v(2), v(4))
    }
}

/// Pictures the display lists name, by file, loaded once.
#[derive(Default)]
pub struct Pictures {
    loaded: HashMap<String, Option<egui::TextureHandle>>,
}

impl Pictures {
    fn get(&mut self, ctx: &egui::Context, path: &str) -> Option<egui::TextureId> {
        self.loaded.entry(path.to_string()).or_insert_with(|| {
            let img = image::open(path).ok()?;
            let img = if img.width().max(img.height()) > 2048 { img.thumbnail(2048, 2048) } else { img };
            let rgba = img.to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
            Some(ctx.load_texture(format!("sheet-picture:{path}"), color, egui::TextureOptions::LINEAR))
        }).as_ref().map(|t| t.id())
    }
}

fn stroke_of(op: &J, zoom: f32) -> Option<(Stroke, Vec<f32>)> {
    let p = op.get("pen")?;
    if p.is_null() {
        return None;
    }
    let w = (p["width"].as_f64().unwrap_or(0.25) as f32 * zoom).max(0.6);
    let dash: Vec<f32> = p["dash"].as_array().map(|a| a.iter().filter_map(J::as_f64).map(|d| d as f32 * zoom).collect())
        .unwrap_or_default();
    Some((Stroke::new(w, rgba(p["color"].as_str().unwrap_or("#000000ff"))), dash))
}

fn fill_of(op: &J) -> Option<Color32> {
    op.get("brush").and_then(J::as_str).map(rgba)
}

fn line(shapes: &mut Vec<Shape>, a: Pos2, b: Pos2, stroke: Stroke, dash: &[f32]) {
    if dash.len() >= 2 && dash.iter().all(|d| *d >= 0.0) && dash.iter().any(|d| *d > 0.05) {
        let (on, off): (Vec<f32>, Vec<f32>) = dash.chunks(2).map(|c| (c[0].max(0.3), c.get(1).copied().unwrap_or(c[0]).max(0.3))).unzip();
        shapes.extend(Shape::dashed_line_with_offset(&[a, b], stroke, &on, &off, 0.0));
    } else {
        shapes.push(Shape::line_segment([a, b], stroke));
    }
}

fn outline(shapes: &mut Vec<Shape>, pts: &[Pos2], closed: bool, stroke: Stroke, dash: &[f32]) {
    if pts.len() < 2 {
        return;
    }
    let n = if closed { pts.len() } else { pts.len() - 1 };
    if dash.is_empty() {
        shapes.push(if closed { Shape::closed_line(pts.to_vec(), stroke) } else { Shape::line(pts.to_vec(), stroke) });
        return;
    }
    for i in 0..n {
        line(shapes, pts[i], pts[(i + 1) % pts.len()], stroke, dash);
    }
}

fn fill(shapes: &mut Vec<Shape>, pts: &[Pos2], colour: Color32) {
    if pts.len() >= 3 {
        shapes.push(Shape::convex_polygon(pts.to_vec(), colour, Stroke::NONE));
    }
}

/// Cyrus–Beck: the part of segment a→b inside the convex polygon `poly`.
fn clip_segment(a: Pos2, b: Pos2, poly: &[Pos2]) -> Option<(Pos2, Pos2)> {
    if poly.len() < 3 {
        return Some((a, b));
    }
    let area: f32 = (0..poly.len()).map(|i| {
        let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
        p.x * q.y - q.x * p.y
    }).sum();
    let sign = if area >= 0.0 { 1.0 } else { -1.0 };
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    let d = b - a;
    for i in 0..poly.len() {
        let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
        let e = q - p;
        // inside: cross(e, x - p) * sign >= 0
        let num = sign * (e.x * (a.y - p.y) - e.y * (a.x - p.x));
        let den = sign * (e.x * d.y - e.y * d.x);
        if den.abs() < 1e-9 {
            if num < 0.0 {
                return None;
            }
        } else {
            let t = -num / den;
            if den > 0.0 { t0 = t0.max(t) } else { t1 = t1.min(t) }
        }
        if t0 > t1 {
            return None;
        }
    }
    Some((a + d * t0, a + d * t1))
}

fn ellipse_points(c: Pos2, ax: egui::Vec2, ay: egui::Vec2) -> Vec<Pos2> {
    let n = 72;
    (0..n).map(|i| {
        let t = std::f32::consts::TAU * i as f32 / n as f32;
        c + ax * t.cos() + ay * t.sin()
    }).collect()
}

fn rounded_points(p: &[Pos2], rx: f32, ry: f32) -> Vec<Pos2> {
    if p.len() < 4 {
        return p.to_vec();
    }
    let u = (p[1] - p[0]).normalized();
    let v = (p[3] - p[0]).normalized();
    let (w, h) = ((p[1] - p[0]).length(), (p[3] - p[0]).length());
    let (rx, ry) = (rx.min(w / 2.0), ry.min(h / 2.0));
    let corner = |cx: f32, cy: f32, start: f32| -> Vec<Pos2> {
        (0..=8).map(|i| {
            let a = start + std::f32::consts::FRAC_PI_2 * i as f32 / 8.0;
            p[0] + u * (cx + rx * a.cos()) + v * (cy + ry * a.sin())
        }).collect()
    };
    use std::f32::consts::PI;
    let mut out = corner(w - rx, ry, -PI / 2.0);
    out.extend(corner(w - rx, h - ry, 0.0));
    out.extend(corner(rx, h - ry, PI / 2.0));
    out.extend(corner(rx, ry, PI));
    out
}

/// Paint `ops` (one item's display list) at `place`.
pub fn paint(painter: &egui::Painter, ctx: &egui::Context, pictures: &mut Pictures, ops: &J, place: Place) {
    let Some(ops) = ops.as_array() else { return };
    let z = place.zoom;
    let mut clip: Option<Vec<Pos2>> = None;
    let mut shapes: Vec<Shape> = Vec::new();
    for op in ops {
        let stroke = stroke_of(op, z);
        let brush = fill_of(op);
        match op["op"].as_str().unwrap_or("") {
            "line" => {
                let p = &op["pts"];
                let (a, b) = (place.pt(&J::from(vec![p[0].clone(), p[1].clone()])), place.pt(&J::from(vec![p[2].clone(), p[3].clone()])));
                if let Some((s, dash)) = stroke {
                    let seg = match &clip {
                        Some(poly) => clip_segment(a, b, poly),
                        None => Some((a, b)),
                    };
                    if let Some((a, b)) = seg {
                        line(&mut shapes, a, b, s, &dash);
                    }
                }
            }
            "poly" => {
                let pts = place.pts(&op["pts"]);
                let closed = op["closed"].as_bool().unwrap_or(true);
                if let Some(c) = brush.filter(|_| closed) {
                    fill(&mut shapes, &pts, c);
                }
                if let Some((s, dash)) = stroke {
                    outline(&mut shapes, &pts, closed, s, &dash);
                }
            }
            "ellipse" => {
                let c = place.pt(&op["center"]);
                let v = |k: &str| vec2(op[k][0].as_f64().unwrap_or(0.0) as f32 * z, op[k][1].as_f64().unwrap_or(0.0) as f32 * z);
                let pts = ellipse_points(c, v("ax"), v("ay"));
                if let Some(col) = brush {
                    fill(&mut shapes, &pts, col);
                }
                if let Some((s, dash)) = stroke {
                    outline(&mut shapes, &pts, true, s, &dash);
                }
            }
            "rrect" => {
                let pts = rounded_points(&place.pts(&op["pts"]), op["rx"].as_f64().unwrap_or(0.0) as f32 * z,
                                         op["ry"].as_f64().unwrap_or(0.0) as f32 * z);
                if let Some(col) = brush {
                    fill(&mut shapes, &pts, col);
                }
                if let Some((s, dash)) = stroke {
                    outline(&mut shapes, &pts, true, s, &dash);
                }
            }
            "path" => {
                for ring in op["rings"].as_array().into_iter().flatten() {
                    let pts = place.pts(ring);
                    if let Some(col) = brush {
                        fill(&mut shapes, &pts, col);
                    }
                    if let Some((s, dash)) = &stroke {
                        outline(&mut shapes, &pts, true, *s, dash);
                    }
                }
            }
            "clip" => {
                if let Some(ring) = op["rings"].as_array().and_then(|r| r.first()) {
                    clip = Some(place.pts(ring));
                }
            }
            "text" => {
                let r = place.pts(&op["rect"]);
                if r.len() == 4 {
                    text(&mut shapes, ctx, op, &r, z);
                }
            }
            "image" => {
                let r = place.pts(&op["rect"]);
                if let (4, Some(tex)) = (r.len(), pictures.get(ctx, op["source"].as_str().unwrap_or(""))) {
                    let mut mesh = Mesh::with_texture(tex);
                    let uv = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
                    for (p, t) in r.iter().zip(uv) {
                        mesh.vertices.push(egui::epaint::Vertex { pos: *p, uv: t, color: Color32::WHITE });
                    }
                    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
                    shapes.push(Shape::mesh(mesh));
                }
            }
            _ => {}
        }
    }
    painter.extend(shapes);
}

/// Qt's text box: the words laid out in the box's frame (which may turn),
/// aligned per the flags, wrapped at its width.
fn text(shapes: &mut Vec<Shape>, ctx: &egui::Context, op: &J, r: &[Pos2], z: f32) {
    let txt = op["text"].as_str().unwrap_or("");
    if txt.is_empty() {
        return;
    }
    let flags = op["flags"].as_i64().unwrap_or(0);
    let size = (op["size"].as_f64().unwrap_or(3.0) as f32 * z).max(1.0);
    let colour = rgba(op["color"].as_str().unwrap_or("#1e242cff"));
    let u = r[1] - r[0];
    let v = r[3] - r[0];
    let (w, h) = (u.length(), v.length());
    let angle = u.y.atan2(u.x);
    let mut job = egui::text::LayoutJob::default();
    job.append(txt, 0.0, egui::TextFormat {
        font_id: FontId::proportional(size),
        color: colour,
        italics: op["italic"].as_bool().unwrap_or(false),
        underline: if op["underline"].as_bool().unwrap_or(false) { Stroke::new((size / 14.0).max(0.6), colour) } else { Stroke::NONE },
        ..Default::default()
    });
    if flags & 0x1000 != 0 {
        job.wrap.max_width = w.max(1.0);
    }
    let galley = ctx.fonts(|f| f.layout_job(job));
    let (gw, gh) = (galley.size().x, galley.size().y);
    let dx = if flags & 0x2 != 0 { w - gw } else if flags & 0x4 != 0 { (w - gw) / 2.0 } else { 0.0 };
    let dy = if flags & 0x40 != 0 { h - gh } else if flags & 0x80 != 0 { (h - gh) / 2.0 } else { 0.0 };
    let (un, vn) = (if w > 0.0 { u / w } else { vec2(1.0, 0.0) }, if h > 0.0 { v / h } else { vec2(0.0, 1.0) });
    let pos = r[0] + un * dx + vn * dy;
    let bold = op["bold"].as_bool().unwrap_or(false);
    shapes.push(Shape::Text(egui::epaint::TextShape::new(pos, galley.clone(), colour).with_angle(angle)));
    if bold {
        // Faux bold: the font set has one weight.
        let nudge = un * (size / 28.0).max(0.4);
        shapes.push(Shape::Text(egui::epaint::TextShape::new(pos + nudge, galley, colour).with_angle(angle)));
    }
}

/// The screen box of an item drawn from its display list (for picking).
pub fn screen_bounds(bounds: [f64; 4], place: Place) -> Rect {
    Rect::from_min_size(pos2(place.origin.x + bounds[0] as f32 * place.zoom, place.origin.y + bounds[1] as f32 * place.zoom),
                        vec2(bounds[2] as f32 * place.zoom, bounds[3] as f32 * place.zoom))
}
