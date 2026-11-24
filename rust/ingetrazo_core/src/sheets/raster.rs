// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! A sheet as a picture (the image export, docs/rust-sheets.md §5): the
//! same page the PDF writer writes (`pdf::pages_of`), drawn on white paper
//! at a chosen dpi with tiny-skia, the text in Inter's own outlines laid
//! out as the PDF lays it (`pdf::text_lines`). It replaces the composer's
//! `export_image` (a QPainter on a QImage).

use super::metrics::face_for;
use super::paint::{Cap, Op, Pen, Prim, Rgba};
use super::pdf::{text_lines, underline_of, Page, Picture};
use ab_glyph::{Font as _, FontRef, OutlineCurve};
use std::collections::HashMap;
use tiny_skia::{FillRule, LineCap, LineJoin, Mask, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke, StrokeDash, Transform};

static INTER: [&[u8]; 2] = [
    include_bytes!("../../../../resources/fonts/inter/Inter-Regular.ttf"),
    include_bytes!("../../../../resources/fonts/inter/Inter-SemiBold.ttf"),
];

fn paint_of(c: Rgba) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c.0, c.1, c.2, c.3);
    p.anti_alias = true;
    p
}

fn stroke_of(pen: &Pen) -> Stroke {
    let mut s = Stroke {
        width: pen.width.max(0.0) as f32,
        line_cap: match pen.cap {
            Cap::Flat => LineCap::Butt,
            Cap::Round => LineCap::Round,
            Cap::Square => LineCap::Square,
        },
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    let dash: Vec<f32> = pen.dash.iter().map(|d| (*d as f32).max(1e-3)).collect();
    if dash.len() >= 2 && dash.len() % 2 == 0 {
        s.dash = StrokeDash::new(dash, 0.0);
    }
    s
}

fn ring_path(rings: &[&[[f64; 2]]], close: bool) -> Option<tiny_skia::Path> {
    let mut b = PathBuilder::new();
    for ring in rings {
        for (i, p) in ring.iter().enumerate() {
            if i == 0 { b.move_to(p[0] as f32, p[1] as f32) } else { b.line_to(p[0] as f32, p[1] as f32) }
        }
        if close {
            b.close();
        }
    }
    b.finish()
}

struct Canvas<'a> {
    pix: Pixmap,
    base: Transform,
    clip: Option<Mask>,
    pictures: &'a HashMap<String, Picture>,
    loaded: HashMap<String, Option<Pixmap>>,
    fonts: [FontRef<'static>; 2],
}

impl Canvas<'_> {
    fn mask_of(&self, rect: Option<&[[f64; 2]; 4]>, rings: &[Vec<[f64; 2]>]) -> Option<Mask> {
        let mut m = Mask::new(self.pix.width(), self.pix.height())?;
        let path = match rect {
            Some(r) => ring_path(&[&r[..]], true),
            None => ring_path(&rings.iter().map(Vec::as_slice).collect::<Vec<_>>(), true),
        }?;
        m.fill_path(&path, FillRule::EvenOdd, true, self.base);
        Some(m)
    }
    fn fill_stroke(&mut self, path: &tiny_skia::Path, pen: Option<&Pen>, brush: Option<Rgba>, rule: FillRule) {
        if let Some(b) = brush {
            self.pix.fill_path(path, &paint_of(b), rule, self.base, self.clip.as_ref());
        }
        if let Some(p) = pen.filter(|p| !p.none) {
            self.pix.stroke_path(path, &paint_of(p.color), &stroke_of(p), self.base, self.clip.as_ref());
        }
    }
    fn picture(&mut self, source: &str) -> Option<Pixmap> {
        if !self.loaded.contains_key(source) {
            let made = match self.pictures.get(source) {
                Some(p) => pixmap_of(p.w, p.h, &p.rgba),
                None => image::open(source).ok().and_then(|im| {
                    let im = im.to_rgba8();
                    pixmap_of(im.width(), im.height(), im.as_raw())
                }),
            };
            self.loaded.insert(source.to_string(), made);
        }
        self.loaded.get(source).cloned().flatten()
    }
    fn op(&mut self, op: &Op) {
        let pen = op.pen.as_ref().filter(|p| !p.none);
        match &op.prim {
            Prim::Clip { rect, rings } => self.clip = self.mask_of(rect.as_ref(), rings),
            Prim::Unclip { clip } => {
                self.clip = match clip.as_deref() {
                    Some(Prim::Clip { rect, rings }) => self.mask_of(rect.as_ref(), rings),
                    _ => None,
                };
            }
            Prim::Line { pts } => {
                if let (Some(p), Some(path)) = (pen, ring_path(&[&[[pts[0], pts[1]], [pts[2], pts[3]]][..]], false)) {
                    self.pix.stroke_path(&path, &paint_of(p.color), &stroke_of(p), self.base, self.clip.as_ref());
                }
            }
            Prim::Poly { pts, closed } => {
                if let Some(path) = ring_path(&[pts.as_slice()], *closed) {
                    self.fill_stroke(&path, pen, op.brush.filter(|_| *closed), FillRule::EvenOdd);
                }
            }
            Prim::Path { rings } => {
                if let Some(path) = ring_path(&rings.iter().map(Vec::as_slice).collect::<Vec<_>>(), true) {
                    self.fill_stroke(&path, pen, op.brush, FillRule::EvenOdd);
                }
            }
            Prim::Ellipse { center: c, ax, ay } => {
                let pts: Vec<[f64; 2]> = (0..96).map(|i| {
                    let t = std::f64::consts::TAU * f64::from(i) / 96.0;
                    [c[0] + ax[0] * t.cos() + ay[0] * t.sin(), c[1] + ax[1] * t.cos() + ay[1] * t.sin()]
                }).collect();
                if let Some(path) = ring_path(&[pts.as_slice()], true) {
                    self.fill_stroke(&path, pen, op.brush, FillRule::Winding);
                }
            }
            Prim::RRect { pts, rx, ry } => {
                let ring = super::pdf::rounded(pts, *rx, *ry);
                if let Some(path) = ring_path(&[ring.as_slice()], true) {
                    self.fill_stroke(&path, pen, op.brush, FillRule::Winding);
                }
            }
            Prim::Image { rect, source } => {
                let Some(pic) = self.picture(source) else { return };
                let (w, h) = (pic.width() as f64, pic.height() as f64);
                let (o, x1, y1) = (rect[0], rect[1], rect[3]);
                let t = Transform::from_row(((x1[0] - o[0]) / w) as f32, ((x1[1] - o[1]) / w) as f32,
                                            ((y1[0] - o[0]) / h) as f32, ((y1[1] - o[1]) / h) as f32, o[0] as f32, o[1] as f32);
                let paint = PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..PixmapPaint::default() };
                self.pix.draw_pixmap(0, 0, pic.as_ref(), &paint, self.base.pre_concat(t), self.clip.as_ref());
            }
            Prim::Text { .. } => self.text(op),
        }
    }
    fn text(&mut self, op: &Op) {
        let Prim::Text { rect, flags, text, size, bold, italic, underline, color } = &op.prim else { return };
        let Some(lay) = text_lines(rect, *flags, text, *size, *bold) else { return };
        let face = face_for(*bold);
        let font = &self.fonts[usize::from(*bold)];
        let upem = f64::from(font.units_per_em().unwrap_or(2048.0));
        let k = size / upem;
        let (un, vn) = (lay.u, lay.v);
        let shear = if *italic { 0.2 } else { 0.0 };
        let paint = paint_of(*color);
        for line in &lay.lines {
            let mut pen_x = 0.0;
            let mut prev: Option<char> = None;
            let mut builder = PathBuilder::new();
            for ch in line.text.chars() {
                if let Some(p) = prev {
                    pen_x += face.kern(p, ch) * size / 100.0;
                }
                let o = [line.origin[0] + un[0] * pen_x, line.origin[1] + un[1] * pen_x];
                // Font units (y up) → paper mm along the box (y down).
                let map = |x: f32, y: f32| -> (f32, f32) {
                    let (gx, gy) = (f64::from(x) + shear * f64::from(y), f64::from(y));
                    ((o[0] + k * (un[0] * gx - vn[0] * gy)) as f32, (o[1] + k * (un[1] * gx - vn[1] * gy)) as f32)
                };
                if let Some(outline) = font.outline(font.glyph_id(ch)) {
                    let mut last: Option<ab_glyph::Point> = None;
                    for c in &outline.curves {
                        let (start, end) = match c {
                            OutlineCurve::Line(a, b) | OutlineCurve::Quad(a, _, b) | OutlineCurve::Cubic(a, _, _, b) => (*a, *b),
                        };
                        if last != Some(start) {
                            if last.is_some() {
                                builder.close();
                            }
                            let (x, y) = map(start.x, start.y);
                            builder.move_to(x, y);
                        }
                        match c {
                            OutlineCurve::Line(_, b) => {
                                let (x, y) = map(b.x, b.y);
                                builder.line_to(x, y);
                            }
                            OutlineCurve::Quad(_, q, b) => {
                                let ((qx, qy), (x, y)) = (map(q.x, q.y), map(b.x, b.y));
                                builder.quad_to(qx, qy, x, y);
                            }
                            OutlineCurve::Cubic(_, c1, c2, b) => {
                                let ((ax, ay), (bx, by), (x, y)) = (map(c1.x, c1.y), map(c2.x, c2.y), map(b.x, b.y));
                                builder.cubic_to(ax, ay, bx, by, x, y);
                            }
                        }
                        last = Some(end);
                    }
                    if last.is_some() {
                        builder.close();
                    }
                }
                pen_x += face.advance(&ch.to_string()) * size / 100.0;
                prev = Some(ch);
            }
            if let Some(path) = builder.finish() {
                self.pix.fill_path(&path, &paint, FillRule::Winding, self.base, self.clip.as_ref());
            }
            if *underline && !line.text.is_empty() {
                let (a, b, t) = underline_of(&lay, line, *size);
                if let Some(path) = ring_path(&[&[a, b][..]], false) {
                    let pen = Pen::new(*color, t).cap(Cap::Flat);
                    self.pix.stroke_path(&path, &paint, &stroke_of(&pen), self.base, self.clip.as_ref());
                }
            }
        }
    }
}

fn pixmap_of(w: u32, h: u32, rgba: &[u8]) -> Option<Pixmap> {
    let mut pix = Pixmap::new(w, h)?;
    for (dst, src) in pix.data_mut().chunks_exact_mut(4).zip(rgba.chunks_exact(4)) {
        let a = u32::from(src[3]);
        for i in 0..3 {
            dst[i] = ((u32::from(src[i]) * a + 127) / 255) as u8;
        }
        dst[3] = src[3];
    }
    Some(pix)
}

/// The page on white paper at `dpi`: RGBA rows, its width and height.
pub fn draw(page: &Page, pictures: &HashMap<String, Picture>, dpi: f64) -> Option<(u32, u32, Vec<u8>)> {
    let scale = dpi / 25.4;
    let (w, h) = ((page.size[0] * scale).round().max(1.0) as u32, (page.size[1] * scale).round().max(1.0) as u32);
    let mut pix = Pixmap::new(w, h)?;
    pix.fill(tiny_skia::Color::WHITE);
    let fonts = [FontRef::try_from_slice(INTER[0]).ok()?, FontRef::try_from_slice(INTER[1]).ok()?];
    let mut c = Canvas { pix, base: Transform::identity(), clip: None, pictures, loaded: HashMap::new(), fonts };
    for item in &page.items {
        c.base = Transform::from_scale(scale as f32, scale as f32).pre_translate(item.origin[0] as f32, item.origin[1] as f32);
        c.clip = None;
        for op in &item.ops {
            c.op(op);
        }
    }
    let data = c.pix.data().to_vec();
    Some((w, h, data))
}

// ---- Python -------------------------------------------------------------------------

use pyo3::prelude::*;
use pyo3::types::PyDict;

/// Write sheet ``comp`` as a picture at ``path`` (PNG, or JPEG by its
/// extension, quality 92) at ``dpi``; the rest as ``sheet_pdf``.
#[pyfunction]
#[pyo3(signature = (path, comp, dpi=200.0, contents=None, pictures=None, profiles=None, scene=None, doc_path=None))]
#[allow(clippy::too_many_arguments)]
pub fn sheet_image(py: Python<'_>, path: std::path::PathBuf, comp: Bound<'_, PyAny>, dpi: f64,
                   contents: Option<Bound<'_, PyDict>>, pictures: Option<Bound<'_, PyDict>>,
                   profiles: Option<Bound<'_, PyAny>>, scene: Option<Bound<'_, PyAny>>,
                   doc_path: Option<Bound<'_, PyAny>>) -> PyResult<(u32, u32)> {
    let pages = super::pdf::pages_of(py, std::slice::from_ref(&comp), contents.as_ref(), profiles.as_ref(),
                                     scene.as_ref(), doc_path.as_ref())?;
    let pics = super::pdf::pictures_of(pictures.as_ref())?;
    let err = |e: String| pyo3::exceptions::PyOSError::new_err(format!("{}: {e}", path.display()));
    let (w, h, data) = py.detach(|| draw(&pages[0], &pics, dpi)).ok_or_else(|| err("no picture".into()))?;
    // White paper: the premultiplied pixels are the colours.
    let img = image::RgbaImage::from_raw(w, h, data).ok_or_else(|| err("pixels".into()))?;
    let rgb = image::DynamicImage::ImageRgba8(img).to_rgb8();
    let jpeg = path.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_lowercase().as_str(), "jpg" | "jpeg"));
    if jpeg {
        let file = std::fs::File::create(&path).map_err(|e| err(e.to_string()))?;
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(file), 92);
        enc.encode_image(&rgb).map_err(|e| err(e.to_string()))?;
    } else {
        rgb.save_with_format(&path, image::ImageFormat::Png).map_err(|e| err(e.to_string()))?;
    }
    Ok((w, h))
}
