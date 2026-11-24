// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! An image item's picture as the sheet shows it (views/composer.py
//! `composited_image`, `_shape_alpha`): cropped (`cover`) or letterboxed
//! (`contain`) to the box's proportions, or stretched; cut to its shape
//! (rectangle, rounded, ellipse) with a feathered edge; at its opacity. The
//! writers composite it from the file the item names.

use serde_json::Value;

/// How an image item shows its picture.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub fit: String,
    pub shape: String,
    pub radius_mm: f64,
    pub feather_mm: f64,
    pub opacity: f64,
    pub w_mm: f64,
    pub h_mm: f64,
}

impl Look {
    pub fn json(&self) -> Value {
        serde_json::json!({"fit": self.fit, "shape": self.shape, "radius_mm": self.radius_mm,
                           "feather_mm": self.feather_mm, "opacity": self.opacity, "w_mm": self.w_mm, "h_mm": self.h_mm})
    }
    pub fn parse(v: &Value) -> Look {
        let n = |k: &str, d: f64| v[k].as_f64().unwrap_or(d);
        let s = |k: &str, d: &str| v[k].as_str().filter(|s| !s.is_empty()).unwrap_or(d).to_string();
        Look { fit: s("fit", "stretch"), shape: s("shape", "rect"), radius_mm: n("radius_mm", 0.0),
               feather_mm: n("feather_mm", 0.0), opacity: n("opacity", 1.0), w_mm: n("w_mm", 1.0), h_mm: n("h_mm", 1.0) }
    }
    /// Nothing to composite: the picture as it is.
    pub fn plain(&self) -> bool {
        self.shape == "rect" && self.feather_mm <= 0.0 && self.opacity >= 1.0 && self.fit == "stretch"
    }
}

/// An image op's source: the file and, for an image item, its look
/// (``path␟{look}``).
pub fn split_source(source: &str) -> (&str, Option<Look>) {
    match source.split_once('\u{1f}') {
        Some((path, look)) => (path, serde_json::from_str(look).ok().map(|v: Value| Look::parse(&v))),
        None => (source, None),
    }
}

/// `_shape_alpha`: per-pixel alpha of the cut-out, 1 inside, 0 outside,
/// a smoothstep `feather` px wide inward (or a 1 px edge without one).
fn shape_alpha(w: usize, h: usize, shape: &str, radius: f64, feather: f64) -> Vec<f64> {
    let (wf, hf) = (w as f64, h as f64);
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let (xs, ys) = (x as f64 + 0.5, y as f64 + 0.5);
            let dist = if shape == "ellipse" {
                let (a, b) = (wf / 2.0, hf / 2.0);
                let (dx, dy) = (xs - a, ys - b);
                let nd = ((dx / a).powi(2) + (dy / b).powi(2)).sqrt();
                let ang = dy.atan2(dx);
                let r_dir = 1.0 / ((ang.cos() / a).powi(2) + (ang.sin() / b).powi(2)).sqrt();
                (1.0 - nd) * r_dir
            } else {
                let r = (if shape == "rounded" { radius } else { 0.0 }).min(wf / 2.0).min(hf / 2.0).max(0.0);
                let qx = (xs - wf / 2.0).abs() - (wf / 2.0 - r);
                let qy = (ys - hf / 2.0).abs() - (hf / 2.0 - r);
                let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
                let inside = qx.max(qy).min(0.0);
                -(outside + inside - r)
            };
            out.push(if feather <= 0.5 {
                (dist + 0.5).clamp(0.0, 1.0)
            } else {
                let t = (dist / feather).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            });
        }
    }
    out
}

/// `composited_image`: the picture `rgba` (`w` × `h`) as the item shows
/// it; the size it comes out at (2048 px on the long side at most).
pub fn composite(rgba: &[u8], w: u32, h: u32, look: &Look) -> (u32, u32, Vec<u8>) {
    if look.plain() || look.w_mm <= 0.0 || look.h_mm <= 0.0 || w == 0 || h == 0 {
        return (w, h, rgba.to_vec());
    }
    let Some(src) = image::RgbaImage::from_raw(w, h, rgba.to_vec()) else { return (w, h, rgba.to_vec()) };
    let aspect = look.w_mm / look.h_mm;
    let (sw, sh) = (f64::from(w), f64::from(h));
    let (crop, ow, oh) = match look.fit.as_str() {
        "cover" => {
            let (cx, cy, cw, ch) = if sw / sh > aspect {
                let cw = sh * aspect;
                ((sw - cw) / 2.0, 0.0, cw, sh)
            } else {
                let ch = sw / aspect;
                (0.0, (sh - ch) / 2.0, sw, ch)
            };
            (Some((cx, cy, cw, ch)), cw, ch)
        }
        "contain" => {
            let (ow, oh) = if sw / sh > aspect { (sw, sw / aspect) } else { (sh * aspect, sh) };
            (None, ow, oh)
        }
        _ => (None, sw, sw / aspect),
    };
    let m = (2048.0 / ow.max(oh).max(1.0)).min(1.0);
    let (ow, oh) = ((ow * m).round().max(1.0) as u32, (oh * m).round().max(1.0) as u32);
    let mut out = image::RgbaImage::new(ow, oh);
    let filter = image::imageops::FilterType::Triangle;
    match (look.fit.as_str(), crop) {
        ("contain", _) => {
            let k = (f64::from(ow) / sw).min(f64::from(oh) / sh);
            let (dw, dh) = ((sw * k).round().max(1.0) as u32, (sh * k).round().max(1.0) as u32);
            let scaled = image::imageops::resize(&src, dw, dh, filter);
            image::imageops::overlay(&mut out, &scaled, i64::from((ow - dw.min(ow)) / 2), i64::from((oh - dh.min(oh)) / 2));
        }
        (_, Some((cx, cy, cw, ch))) => {
            let part = image::imageops::crop_imm(&src, cx.round() as u32, cy.round() as u32,
                                                 (cw.round() as u32).max(1), (ch.round() as u32).max(1)).to_image();
            out = image::imageops::resize(&part, ow, oh, filter);
        }
        _ => out = image::imageops::resize(&src, ow, oh, filter),
    }
    let px_per_mm = f64::from(ow) / look.w_mm;
    let alpha = shape_alpha(ow as usize, oh as usize, &look.shape, look.radius_mm * px_per_mm, look.feather_mm * px_per_mm);
    let opacity = look.opacity.clamp(0.0, 1.0);
    let mut data = out.into_raw();
    for (px, a) in data.chunks_exact_mut(4).zip(alpha) {
        px[3] = (f64::from(px[3]) * a * opacity).clamp(0.0, 255.0) as u8;
    }
    (ow, oh, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ellipse_cuts_the_corners_and_opacity_fades_the_rest() {
        let rgba = vec![200u8; 40 * 20 * 4];
        let look = Look { fit: "stretch".into(), shape: "ellipse".into(), radius_mm: 0.0, feather_mm: 0.0, opacity: 0.5,
                          w_mm: 40.0, h_mm: 20.0 };
        let (w, h, out) = composite(&rgba, 40, 20, &look);
        assert_eq!((w, h), (40, 20));
        assert_eq!(out[3], 0, "the corner is out");
        let centre = ((10 * 40 + 20) * 4 + 3) as usize;
        assert!((i32::from(out[centre]) - 100).abs() <= 1, "{}", out[centre]);
        let (_, oh, _) = composite(&rgba, 40, 20, &Look { fit: "contain".into(), shape: "rect".into(), h_mm: 40.0,
                                                           ..look.clone() });
        assert_eq!(oh, 40, "letterboxed to the box's proportions");
    }
}
