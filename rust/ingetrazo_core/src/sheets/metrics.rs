// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! The sheet font's metrics: what the sheet painters measure text with.
//!
//! `font_metrics.json` is written by scripts/sheet_font_metrics.py from
//! Qt's "Sans Serif" at a pixel size of 100 (advances, pair kerning and the
//! vertical metrics, regular and bold), the font the Python painters
//! measured with; the sums reproduce `QFontMetricsF.horizontalAdvance`
//! exactly on the strings tested. When the PDF writer embeds a font of its
//! own (docs/rust-sheets.md §5), the table is regenerated from that font.

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
        // QFontMetricsF("Sans Serif", px 100).horizontalAdvance on macOS.
        let f = super::face_for(false);
        assert!((f.advance("AVATAR To") - 455.921875).abs() < 1e-6, "{}", f.advance("AVATAR To"));
        assert!((f.advance("PROYECTO") - 496.703125).abs() < 1e-6);
    }
}

/// Lines `text` takes word-wrapped in `width_mm` at `size_mm`, as Qt's
/// `TextWordWrap` lays it out: typed breaks, then breaks after spaces and
/// hyphens wherever the next word would pass the width (a longer word
/// overflows on its own line). `_line_count` in views/composer.py.
pub fn line_count(text: &str, width_mm: f64, size_mm: f64, bold: bool) -> usize {
    if text.is_empty() {
        return 0;
    }
    let face = face_for(bold);
    let s = size_mm / 100.0 * 0.75;
    let limit = width_mm.max(0.5) / s;
    let mut lines = 0;
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
        let mut n = 1;
        let mut line = String::new();
        for t in tokens {
            let candidate = format!("{line}{t}");
            if !line.is_empty() && face.advance(candidate.trim_end_matches(' ')) > limit {
                n += 1;
                line = t;
            } else {
                line = candidate;
            }
        }
        lines += n;
    }
    lines
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
