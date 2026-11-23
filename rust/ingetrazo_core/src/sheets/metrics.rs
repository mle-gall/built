// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! The sheet font's metrics: what the sheet painters measure text with.
//!
//! `font_metrics.json` is written by scripts/sheet_font_metrics.py from
//! the sheet font, the bundled Inter (SemiBold for bold), as Qt measures it
//! at a pixel size of 100: advances, pair kerning (its GPOS) and the
//! vertical metrics. The sums reproduce `QFontMetricsF.horizontalAdvance`
//! exactly on the strings tested, and the advances are Inter's own widths,
//! so the PDF writer can embed it (docs/rust-sheets.md §5).

use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

pub struct Face {
    pub ascent: f64,
    pub descent: f64,
    pub cap_height: f64,
    pub line_spacing: f64,
    pub average: f64,
    advance: HashMap<char, f64>,
    kern: HashMap<(char, char), f64>,
}

pub struct Metrics {
    pub regular: Face,
    pub bold: Face,
}

fn face(v: &Value) -> Face {
    let num = |k: &str| v[k].as_f64().unwrap_or(0.0);
    let advance = v["advance"].as_object().map(|m| m.iter().filter_map(|(k, a)| {
        Some((k.chars().next()?, a.as_f64()?))
    }).collect()).unwrap_or_default();
    let kern = v["kern"].as_object().map(|m| m.iter().filter_map(|(k, a)| {
        let mut c = k.chars();
        Some(((c.next()?, c.next()?), a.as_f64()?))
    }).collect()).unwrap_or_default();
    Face {
        ascent: num("ascent"),
        descent: num("descent"),
        cap_height: num("cap_height"),
        line_spacing: num("line_spacing"),
        average: num("average"),
        advance,
        kern,
    }
}

pub fn metrics() -> &'static Metrics {
    static M: OnceLock<Metrics> = OnceLock::new();
    M.get_or_init(|| {
        let v: Value = serde_json::from_str(include_str!("font_metrics.json")).expect("sheet font metrics");
        Metrics { regular: face(&v["regular"]), bold: face(&v["bold"]) }
    })
}

impl Face {
    /// Advance of `text` at a pixel size of 100.
    pub fn advance(&self, text: &str) -> f64 {
        let mut w = 0.0;
        let mut prev: Option<char> = None;
        for c in text.chars() {
            w += self.advance.get(&c).copied().unwrap_or(self.average);
            if let Some(p) = prev {
                w += self.kern.get(&(p, c)).copied().unwrap_or(0.0);
            }
            prev = Some(c);
        }
        w
    }
}

pub fn face_for(bold: bool) -> &'static Face {
    let m = metrics();
    if bold { &m.bold } else { &m.regular }
}

/// views/composer.py `_text_width_mm`: the advance of `text` drawn by
/// `draw_text_mm` at `size_mm`.
pub fn text_width_mm(text: &str, size_mm: f64, bold: bool) -> f64 {
    if text.is_empty() {
        return 0.0;
    }
    face_for(bold).advance(text) * (size_mm / 100.0 * 0.75)
}

/// `text_baseline_rect_mm`: a top-aligned text rect whose baseline lands on
/// `baseline_y`, centred on x = 0: (x, y, w, h).
pub fn text_baseline_rect_mm(size_mm: f64, baseline_y: f64, width: f64, bold: bool) -> [f64; 4] {
    let f = face_for(bold);
    let s = size_mm / 100.0 * 0.75;
    let (asc, desc) = (f.ascent * s, f.descent * s);
    [-width / 2.0, baseline_y - asc, width, asc + desc]
}

/// `text_cap_rect_mm`: the capitals' top on `cap_top_y`.
pub fn text_cap_rect_mm(size_mm: f64, cap_top_y: f64, width: f64) -> [f64; 4] {
    let cap = face_for(false).cap_height * size_mm / 100.0 * 0.75;
    text_baseline_rect_mm(size_mm, cap_top_y + cap, width, false)
}

#[cfg(test)]
mod tests {
    #[test]
    fn widths_are_qt_s() {
        // QFontMetricsF(Inter, px 100).horizontalAdvance (scripts/sheet_font.py).
        let f = super::face_for(false);
        assert!((f.advance("AVATAR To") - 518.984375).abs() < 1e-6, "{}", f.advance("AVATAR To"));
        assert!((f.advance("PROYECTO") - 540.296875).abs() < 1e-6);
    }
}

/// The lines `text` takes word-wrapped at `limit` (pixel-size-100 units),
/// as Qt's `TextWordWrap` lays it out: typed breaks, then breaks after
/// spaces and hyphens wherever the next word would pass the width (a
/// longer word overflows on its own line). Each line comes with whether a
/// wrap ended it (its trailing spaces then take no room).
pub fn wrap_lines(text: &str, limit: f64, face: &Face) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        // Break opportunities: after a run of spaces, after a hyphen.
        let mut tokens: Vec<String> = Vec::new();
        let mut cur = String::new();
        let chars: Vec<char> = para.chars().collect();
        for (i, c) in chars.iter().enumerate() {
            cur.push(*c);
            let next = chars.get(i + 1);
            let after_space = *c == ' ' && next != Some(&' ');
            let after_hyphen = *c == '-' && next.is_some_and(|n| n.is_alphanumeric());
            if after_space || after_hyphen {
                tokens.push(std::mem::take(&mut cur));
            }
        }
        if !cur.is_empty() {
            tokens.push(cur);
        }
        let mut line = String::new();
        for t in tokens {
            let candidate = format!("{line}{t}");
            if !line.is_empty() && face.advance(candidate.trim_end_matches(' ')) > limit {
                out.push((std::mem::take(&mut line), true));
                line = t;
            } else {
                line = candidate;
            }
        }
        out.push((line, false));
    }
    out
}

/// Lines `text` takes word-wrapped in `width_mm` at `size_mm`.
/// `_line_count` in views/composer.py.
pub fn line_count(text: &str, width_mm: f64, size_mm: f64, bold: bool) -> usize {
    if text.is_empty() {
        return 0;
    }
    let s = size_mm / 100.0 * 0.75;
    wrap_lines(text, width_mm.max(0.5) / s, face_for(bold)).len()
}

/// `QFontMetricsF.boundingRect(box, AlignLeft | TextWordWrap, text)`'s
/// size at pixel size 100 for a box `width` wide: the widest line (a
/// wrapped line without its trailing spaces), and the lines' height (Qt
/// steps each further line by the height and 1/64).
pub fn bounding_units(text: &str, width: f64, bold: bool) -> (f64, f64) {
    let face = face_for(bold);
    let lines = wrap_lines(text, width, face);
    let w = lines.iter().map(|(l, wrapped)| face.advance(if *wrapped { l.trim_end_matches(' ') } else { l }))
        .fold(0.0, f64::max);
    let line_h = face.ascent + face.descent;
    (w, line_h + (lines.len() - 1) as f64 * (line_h + 1.0 / 64.0))
}

/// views/composer.py `_fit_text_size_mm`: the largest size ≤ `base` (by
/// steps of 0.88) at which `text`, word-wrapped, fits a `w` × `h` box.
pub fn fit_text_size_mm(text: &str, w: f64, h: f64, base: f64, bold: bool) -> f64 {
    let mut size = base;
    while size > 1.0 {
        let s = size / 100.0 * 0.75;
        let (bw, bh) = (w / s, h / s);
        let (nw, nh) = bounding_units(text, bw, bold);
        if nh <= bh && nw <= bw {
            return size;
        }
        size *= 0.88;
    }
    1.0
}

/// views/composer.py `_text_height_mm`: the height `text` wants word-wrapped
/// in a column `width_mm` wide at `size_mm`.
pub fn text_height_mm(text: &str, width_mm: f64, size_mm: f64, bold: bool) -> f64 {
    if text.is_empty() {
        return 0.0;
    }
    let s = size_mm / 100.0 * 0.75;
    bounding_units(text, width_mm.max(0.5) / s, bold).1 * s
}

/// `_laid_out_height_mm`: the first line's ascent + descent, then the
/// line spacing per extra line.
pub fn laid_out_height_mm(text: &str, width_mm: f64, size_mm: f64, bold: bool) -> f64 {
    let n = line_count(text, width_mm, size_mm, bold);
    if n == 0 {
        return 0.0;
    }
    let f = face_for(bold);
    let s = size_mm / 100.0 * 0.75;
    (f.ascent + f.descent + (n - 1) as f64 * f.line_spacing) * s
}

/// `text_bg_span_mm`: (y0, y1) of a background hugging the letters.
pub fn text_bg_span_mm(text: &str, width_mm: f64, size_mm: f64, bold: bool) -> (f64, f64) {
    let n = line_count(text, width_mm, size_mm, bold);
    let f = face_for(bold);
    let s = size_mm / 100.0 * 0.75;
    if n == 0 {
        return (0.0, size_mm);
    }
    let y0 = (f.ascent - f.cap_height) * s - 0.25 * size_mm;
    let y1 = (f.ascent + f.descent + (n - 1) as f64 * f.line_spacing) * s + 0.05 * size_mm;
    (y0, y1)
}
