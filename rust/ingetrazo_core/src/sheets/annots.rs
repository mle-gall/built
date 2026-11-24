// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! A frame's model annotations: the paper overlay every style carries
//! (views/composer.py `compute_annotations` and `_paint_annots_mm`).
//! Traced georef paths always (they are the design, drawn on the ground as
//! the viewport drapes them), their chainage marks and the section marks
//! when the frame asks, the extensions' own sheet items, and the model's
//! dimensions and leader texts when the frame opts in. Everything is
//! projected through the frame's own camera (sheets::anchors `Projector`),
//! and what the frame's view hides stays out: the live viewport is never
//! pointed at the frame and put back.
//!
//! The overlay is a list of the composer's tuples, frame-local mm:
//! `("line", x0, y0, x1, y1)`, `("arrow", x, y, dx, dy)`,
//! `("poly", [(x, y), …])`, `("text", x, y, deg, text, size)`,
//! `("secmark", x0, y0, x1, y1, ax, ay, label, size)`,
//! `("xpoly", pts, width_mm, rgb)` and `("xtext", x, y, deg, text, size,
//! bold, rgb, bg_rgb, bg_alpha, left, w, h)`.

use super::anchors::Projector;
use super::metrics::face_for;
use super::paint::{align, Cap, Painter, Pen, Rgba};
use super::visibility::{Tags, ViewState};
use crate::doc;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};
use std::f64::consts::PI;

/// One overlay item (see the module's tuples).
#[derive(Clone, Debug, PartialEq)]
pub enum Annot {
    Line([f64; 4]),
    Arrow([f64; 4]),
    Poly(Vec<[f64; 2]>),
    Text { at: [f64; 2], deg: f64, text: String, size: f64 },
    SecMark { line: [f64; 4], dir: [f64; 2], label: String, size: f64 },
    XPoly { pts: Vec<[f64; 2]>, width: f64, rgb: [u8; 3] },
    XText { at: [f64; 2], deg: f64, text: String, size: f64, bold: bool, rgb: [u8; 3], bg: Option<[u8; 3]>,
            alpha: f64, left: f64, w: f64, h: f64 },
}

const INK: Rgba = Rgba(45, 55, 75, 255);
const HALO: Rgba = Rgba(255, 255, 255, 230);
/// `_GEO_PATH_INK`: traced georef paths in the viewport's cyan.
const GEO_PATH_INK: Rgba = Rgba(0, 150, 170, 255);
/// The extensions' default ink.
const EXT_INK: [u8; 3] = [45, 55, 75];

fn rgba(c: [u8; 3]) -> Rgba {
    Rgba(c[0], c[1], c[2], 255)
}

// ---- The tuples ----------------------------------------------------------------------------------

fn floats(v: &Bound<'_, PyAny>) -> PyResult<Vec<f64>> {
    v.try_iter()?.map(|x| x?.extract::<f64>()).collect()
}

fn pt2(v: &Bound<'_, PyAny>) -> PyResult<[f64; 2]> {
    let c = floats(v)?;
    Ok([c.first().copied().unwrap_or(0.0), c.get(1).copied().unwrap_or(0.0)])
}

fn pt3(v: &Bound<'_, PyAny>) -> PyResult<[f64; 3]> {
    let c = floats(v)?;
    Ok([c.first().copied().unwrap_or(0.0), c.get(1).copied().unwrap_or(0.0), c.get(2).copied().unwrap_or(0.0)])
}

fn pts2(v: &Bound<'_, PyAny>) -> PyResult<Vec<[f64; 2]>> {
    v.try_iter()?.map(|q| pt2(&q?)).collect()
}

fn rgb_of(v: &Bound<'_, PyAny>) -> PyResult<[u8; 3]> {
    let c = floats(v)?;
    let at = |i: usize| c.get(i).copied().unwrap_or(0.0).round().clamp(0.0, 255.0) as u8;
    Ok([at(0), at(1), at(2)])
}

/// The composer's overlay tuples as items (an unknown kind is skipped).
pub fn parse(list: &Bound<'_, PyAny>) -> PyResult<Vec<Annot>> {
    let mut out = Vec::new();
    for a in list.try_iter()? {
        let a = a?;
        let n = a.len()?;
        let kind: String = a.get_item(0)?.extract()?;
        let f = |i: usize| -> PyResult<f64> { a.get_item(i)?.extract() };
        let s = |i: usize| -> PyResult<String> { Ok(a.get_item(i)?.str()?.to_string()) };
        out.push(match (kind.as_str(), n) {
            ("line", 5) => Annot::Line([f(1)?, f(2)?, f(3)?, f(4)?]),
            ("arrow", 5) => Annot::Arrow([f(1)?, f(2)?, f(3)?, f(4)?]),
            ("poly", 2) => Annot::Poly(pts2(&a.get_item(1)?)?),
            ("text", 6) => Annot::Text { at: [f(1)?, f(2)?], deg: f(3)?, text: s(4)?, size: f(5)? },
            ("secmark", 9) => Annot::SecMark { line: [f(1)?, f(2)?, f(3)?, f(4)?], dir: [f(5)?, f(6)?], label: s(7)?,
                                               size: f(8)? },
            ("xpoly", 4) => Annot::XPoly { pts: pts2(&a.get_item(1)?)?, width: f(2)?, rgb: rgb_of(&a.get_item(3)?)? },
            ("xtext", 13) => {
                let bg = a.get_item(8)?;
                Annot::XText { at: [f(1)?, f(2)?], deg: f(3)?, text: s(4)?, size: f(5)?,
                               bold: a.get_item(6)?.is_truthy()?, rgb: rgb_of(&a.get_item(7)?)?,
                               bg: if bg.is_none() { None } else { Some(rgb_of(&bg)?) }, alpha: f(9)?, left: f(10)?,
                               w: f(11)?, h: f(12)? }
            }
            _ => continue,
        });
    }
    Ok(out)
}

fn rgb_tuple<'py>(py: Python<'py>, c: [u8; 3]) -> PyResult<Bound<'py, PyTuple>> {
    PyTuple::new(py, c.map(i64::from))
}

/// An item as the composer's tuple.
pub fn to_tuple<'py>(py: Python<'py>, a: &Annot) -> PyResult<Bound<'py, PyTuple>> {
    let pts = |p: &[[f64; 2]]| PyList::new(py, p.iter().map(|q| (q[0], q[1])));
    Ok(match a {
        Annot::Line(l) => PyTuple::new(py, [("line").into_pyobject(py)?.into_any(), l[0].into_pyobject(py)?.into_any(),
                                            l[1].into_pyobject(py)?.into_any(), l[2].into_pyobject(py)?.into_any(),
                                            l[3].into_pyobject(py)?.into_any()])?,
        Annot::Arrow(l) => ("arrow", l[0], l[1], l[2], l[3]).into_pyobject(py)?,
        Annot::Poly(p) => ("poly", pts(p)?).into_pyobject(py)?,
        Annot::Text { at, deg, text, size } => ("text", at[0], at[1], *deg, text.as_str(), *size).into_pyobject(py)?,
        Annot::SecMark { line, dir, label, size } => {
            ("secmark", line[0], line[1], line[2], line[3], dir[0], dir[1], label.as_str(), *size).into_pyobject(py)?
        }
        Annot::XPoly { pts: p, width, rgb } => ("xpoly", pts(p)?, *width, rgb_tuple(py, *rgb)?).into_pyobject(py)?,
        Annot::XText { at, deg, text, size, bold, rgb, bg, alpha, left, w, h } => {
            let bg: Bound<'py, PyAny> = match bg {
                Some(c) => rgb_tuple(py, *c)?.into_any(),
                None => py.None().into_bound(py),
            };
            let items: Vec<Bound<'py, PyAny>> = vec![
                "xtext".into_pyobject(py)?.into_any(), at[0].into_pyobject(py)?.into_any(),
                at[1].into_pyobject(py)?.into_any(), deg.into_pyobject(py)?.into_any(),
                text.as_str().into_pyobject(py)?.into_any(), size.into_pyobject(py)?.into_any(),
                pyo3::types::PyBool::new(py, *bold).to_owned().into_any(), rgb_tuple(py, *rgb)?.into_any(), bg,
                alpha.into_pyobject(py)?.into_any(), left.into_pyobject(py)?.into_any(),
                w.into_pyobject(py)?.into_any(), h.into_pyobject(py)?.into_any()];
            PyTuple::new(py, items)?
        }
    })
}

// ---- Painting ------------------------------------------------------------------------------------

/// `_paint_section_mark_mm`: the cut line dash-dotted with a heavy stub at
/// each end, an arrow toward the side the section looks at, and the letter
/// in a bubble past the arrow — «A … A» across the plan.
fn paint_section_mark(p: &mut Painter, line: [f64; 4], dir: [f64; 2], label: &str, size: f64) {
    let [x0, y0, x1, y1] = line;
    let [ax, ay] = dir;
    p.set_pen(Pen::new(INK, 0.35).dashed(&[8.0, 2.0, 1.0, 2.0]).cap(Cap::Flat));
    p.line(x0, y0, x1, y1);
    let (dx, dy) = (x1 - x0, y1 - y0);
    let ln = dx.hypot(dy);
    let ln = if ln != 0.0 { ln } else { 1.0 };
    let (ux, uy) = (dx / ln, dy / ln);
    let stub = (size * 1.2).max(3.0);
    let arrow = (size * 1.0).max(2.5);
    let r = size * 0.95;
    for (ex, ey, sx, sy) in [(x0, y0, ux, uy), (x1, y1, -ux, -uy)] {
        p.set_pen(Pen::new(INK, 0.7).cap(Cap::Flat));
        p.line(ex, ey, ex + sx * stub, ey + sy * stub);
        let (bx, by) = (ex + sx * stub * 0.5, ey + sy * stub * 0.5);
        let tip = [bx + ax * arrow, by + ay * arrow];
        let wing = arrow * 0.45;
        p.save();
        p.set_pen(Pen::none());
        p.set_brush(Some(INK));
        p.polygon(&[tip, [bx + ux * wing, by + uy * wing], [bx - ux * wing, by - uy * wing]]);
        p.restore();
        let (cx, cy) = (bx + ax * (arrow + 0.8 + r), by + ay * (arrow + 0.8 + r));
        p.set_pen(Pen::new(INK, 0.35));
        p.set_brush(Some(Rgba(255, 255, 255, 255)));
        p.ellipse(cx, cy, r, r);
        p.set_brush(None);
        p.text_mm([cx - r, cy - r, 2.0 * r, 2.0 * r], label, size * 0.9, true, align::HCENTER | align::VCENTER, INK);
    }
}

/// `_paint_annots_mm`: the overlay over a `w`×`h` frame, clipped to it.
/// Lines first with a white halo under the ink, so a leader over dark
/// water or stone still reads.
pub fn paint_annots(p: &mut Painter, w: f64, h: f64, annots: &[Annot]) {
    p.save();
    p.clip_rect([0.0, 0.0, w, h]);
    for a in annots {
        match a {
            Annot::Line(l) => {
                p.set_pen(Pen::new(HALO, 0.7));
                p.line(l[0], l[1], l[2], l[3]);
            }
            Annot::SecMark { line: l, .. } => {
                p.set_pen(Pen::new(HALO, 1.0));
                p.line(l[0], l[1], l[2], l[3]);
            }
            Annot::Poly(pts) => {
                p.set_pen(Pen::new(HALO, 0.9).cap(Cap::Round));
                p.polyline(pts);
            }
            _ => {}
        }
    }
    for a in annots {
        match a {
            Annot::Poly(pts) => {
                p.set_pen(Pen::new(GEO_PATH_INK, 0.4).cap(Cap::Round));
                p.set_brush(None);
                p.polyline(pts);
                // Node marks, as the viewport draws them — small on paper.
                p.set_brush(Some(GEO_PATH_INK));
                p.set_pen(Pen::new(HALO, 0.15));
                for q in pts {
                    p.ellipse(q[0], q[1], 0.45, 0.45);
                }
                p.set_brush(None);
            }
            Annot::Line(l) => {
                p.set_pen(Pen::new(INK, 0.25));
                p.line(l[0], l[1], l[2], l[3]);
            }
            Annot::Arrow([x, y, dx, dy]) => {
                // A dimension's arrowhead: tip at (x, y), wings trailing
                // back along (dx, dy) — the sheet cota's own proportions.
                let (l, base, ang) = (1.8, 12f64.to_radians(), dy.atan2(*dx));
                p.save();
                p.set_pen(Pen::none());
                p.set_brush(Some(INK));
                p.polygon(&[[*x, *y], [x + l * (ang + base).cos(), y + l * (ang + base).sin()],
                            [x + l * (ang - base).cos(), y + l * (ang - base).sin()]]);
                p.restore();
            }
            Annot::SecMark { line, dir, label, size } => paint_section_mark(p, *line, *dir, label, *size),
            Annot::XPoly { pts, width, rgb } => {
                p.set_pen(Pen::new(rgba(*rgb), *width).cap(Cap::Round));
                p.set_brush(None);
                p.polyline(pts);
            }
            Annot::XText { at, deg, text, size, bold, rgb, bg, alpha, left, w, h } => {
                let rect = [*left, -h / 2.0, *w, *h];
                p.save();
                p.translate(at[0], at[1]);
                p.rotate(*deg);
                if let Some(bg) = bg {
                    p.set_pen(Pen::none());
                    p.set_brush(Some(rgba(*bg).alpha_f(*alpha)));
                    p.rounded_rect(rect, h * 0.15, h * 0.15);
                    p.set_brush(None);
                }
                p.text_mm(rect, text, *size, *bold, align::CENTER, rgba(*rgb));
                p.restore();
            }
            Annot::Text { at, deg, text, size } => {
                let rect = [-60.0, -size * 1.3, 120.0, size * 1.3];
                let flags = align::HCENTER | align::BOTTOM;
                p.save();
                p.translate(at[0], at[1]);
                p.rotate(*deg);
                let d = (size * 0.06).max(0.12);
                for (dx, dy) in [(-d, 0.0), (d, 0.0), (0.0, -d), (0.0, d), (-d, -d), (d, d), (-d, d), (d, -d)] {
                    p.text_mm([rect[0] + dx, rect[1] + dy, rect[2], rect[3]], text, *size, true, flags, HALO);
                }
                p.text_mm(rect, text, *size, true, flags, INK);
                p.restore();
            }
        }
    }
    p.restore();
}

// ---- Gathering -----------------------------------------------------------------------------------

/// `str.splitlines()`: the lines of a label, with no empty last one.
fn split_lines(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(std::mem::take(&mut cur));
            }
            '\n' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}' => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `point_at_station`: the (x, y) at horizontal chainage `s`.
fn point_at_station(pts: &[[f64; 3]], s: f64) -> [f64; 2] {
    let Some(first) = pts.first() else { return [0.0, 0.0] };
    if s <= 0.0 {
        return [first[0], first[1]];
    }
    let mut acc = 0.0;
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let seg = (b[0] - a[0]).hypot(b[1] - a[1]);
        if acc + seg >= s {
            let t = if seg > 1e-12 { (s - acc) / seg } else { 0.0 };
            return [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        }
        acc += seg;
    }
    let last = pts[pts.len() - 1];
    [last[0], last[1]]
}

fn polyline_length(pts: &[[f64; 3]]) -> f64 {
    pts.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum()
}

/// What an extension's sheet provider is told about the frame
/// (`ExtensionApp.add_sheet_annotations`): its scale (0 for a
/// perspective), the annotation text size, and `paper(points)`, world
/// points to frame paper mm.
#[pyclass(module = "ingetrazo_core")]
pub struct SheetContext {
    #[pyo3(get)]
    scale_n: f64,
    #[pyo3(get)]
    text_mm: f64,
    proj: Projector,
}

#[pymethods]
impl SheetContext {
    fn paper(&self, points: &Bound<'_, PyAny>) -> PyResult<Vec<(f64, f64)>> {
        points.try_iter()?.map(|q| {
            let [x, y] = self.proj.page(pt3(&q?)?);
            Ok((x, y))
        }).collect()
    }
}

/// `_extension_marks`: the extensions' own sheet items, projected. Texts
/// with a `priority` give way where they would overlap; a provider that
/// raises is logged and taken out of `providers`.
fn extension_marks(py: Python<'_>, proj: &Projector, text_mm: f64, scale_n: f64, scene: &Bound<'_, PyAny>,
                   providers: &Bound<'_, PyList>) -> PyResult<Vec<Annot>> {
    if providers.is_empty() {
        return Ok(Vec::new());
    }
    let sheet = Bound::new(py, SheetContext { scale_n, text_mm, proj: proj.clone() })?;
    let page = |v: &Bound<'_, PyAny>| -> PyResult<[f64; 2]> { Ok(proj.page(pt3(v)?)) };
    let mut polys = Vec::new();
    let mut texts: Vec<Bound<'_, PyDict>> = Vec::new();
    for fun in providers.iter().collect::<Vec<_>>() {
        let items = match fun.call1((scene, &sheet)).and_then(|got| {
            if got.is_none() { Ok(Vec::new()) } else { got.try_iter()?.collect::<PyResult<Vec<_>>>() }
        }) {
            Ok(items) => items,
            Err(e) => {
                // A plugin never breaks a sheet.
                let log = py.import("logging")?.call_method1("getLogger", ("ingetrazo.plugins",))?;
                let kw = PyDict::new(py);
                kw.set_item("exc_info", e.into_value(py))?;
                log.call_method("error", ("extension sheet annotations %r failed; removed", &fun), Some(&kw))?;
                let _ = providers.call_method1("remove", (&fun,));
                continue;
            }
        };
        for it in items {
            let Ok(it) = it.cast_into::<PyDict>() else { continue };
            let get = |k: &str| -> PyResult<Option<Bound<'_, PyAny>>> { it.get_item(k) };
            if let Some(line) = get("line")?.filter(|l| l.is_truthy().unwrap_or(false)) {
                let pts: Vec<[f64; 2]> = line.try_iter()?.map(|q| page(&q?)).collect::<PyResult<_>>()?;
                if pts.len() >= 2 {
                    let width = match get("width_mm")? { Some(v) => v.extract()?, None => 0.25 };
                    let rgb = match get("color")?.filter(|c| c.is_truthy().unwrap_or(false)) {
                        Some(c) => rgb_of(&c)?,
                        None => EXT_INK,
                    };
                    polys.push(Annot::XPoly { pts, width, rgb });
                }
            } else if get("text")?.is_some_and(|t| t.is_truthy().unwrap_or(false))
                && get("at")?.is_some_and(|a| !a.is_none()) {
                texts.push(it);
            }
        }
    }
    // (priority, order, box, item)
    let mut placed: Vec<(Option<f64>, usize, [f64; 4], Annot)> = Vec::new();
    for it in &texts {
        let get = |k: &str| -> PyResult<Option<Bound<'_, PyAny>>> { Ok(it.get_item(k)?.filter(|v| !v.is_none())) };
        let text = get("text")?.map(|t| t.str().map(|s| s.to_string())).transpose()?.unwrap_or_default();
        let size = match get("size_mm")? {
            Some(v) if v.is_truthy()? => v.extract()?,
            _ => text_mm,
        };
        let bold = match it.get_item("bold")? { Some(v) => v.is_truthy()?, None => true };
        // `_draw_text_mm`'s scale on a 100 px type.
        let face = face_for(bold);
        let s = size / 100.0 * 0.75;
        let pad = size * 0.3;
        let w = face.advance(&text) * s + 2.0 * pad;
        let h = (face.ascent + face.descent) * s + pad;
        let at = get("at")?.expect("checked above");
        let [x, y] = page(&at)?;
        let (mut deg, mut left) = (0.0, -w / 2.0);
        if let Some(toward) = get("toward")? {
            let [qx, qy] = page(&toward)?;
            if (qx - x).hypot(qy - y) > 1e-6 {
                deg = (qy - y).atan2(qx - x).to_degrees();
                let flipped = deg.abs() > 90.0;          // never upside down
                if flipped {
                    deg += if deg < 0.0 { 180.0 } else { -180.0 };
                }
                if get("anchor")?.map(|a| a.str().map(|s| s.to_string())).transpose()?.as_deref() == Some("start") {
                    left = if flipped { -w } else { 0.0 };
                }
            }
        }
        let (c, sn) = (deg.to_radians().cos(), deg.to_radians().sin());
        let mut bx = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for u in [left, left + w] {
            for v in [-h / 2.0, h / 2.0] {
                let q = [x + u * c - v * sn, y + u * sn + v * c];
                bx = [bx[0].min(q[0]), bx[1].min(q[1]), bx[2].max(q[0]), bx[3].max(q[1])];
            }
        }
        let bg = get("background")?.filter(|b| b.is_truthy().unwrap_or(false)).map(|b| rgb_of(&b)).transpose()?;
        let opacity: f64 = match get("background_opacity")? { Some(v) => v.extract()?, None => 100.0 };
        let rgb = match get("color")?.filter(|c| c.is_truthy().unwrap_or(false)) {
            Some(c) => rgb_of(&c)?,
            None => EXT_INK,
        };
        let priority: Option<f64> = get("priority")?.map(|p| p.extract()).transpose()?;
        let n = placed.len();
        placed.push((priority, n, bx, Annot::XText { at: [x, y], deg, text, size, bold, rgb, bg,
                                                      alpha: (opacity / 100.0).clamp(0.0, 1.0), left, w, h }));
    }
    let mut out = polys;
    // The free ones first, then the ranked ones from the lowest number.
    placed.sort_by(|a, b| (a.0.is_some(), a.0.unwrap_or(0.0), a.1)
        .partial_cmp(&(b.0.is_some(), b.0.unwrap_or(0.0), b.1)).unwrap_or(std::cmp::Ordering::Equal));
    let mut taken: Vec<[f64; 4]> = Vec::new();
    for (prio, _n, b, item) in placed {
        if prio.is_some() && taken.iter().any(|t| b[0] < t[2] && t[0] < b[2] && b[1] < t[3] && t[1] < b[3]) {
            continue;
        }
        taken.push(b);
        out.push(item);
    }
    Ok(out)
}

/// `_section_marks`: one mark per section plane that cuts ACROSS the
/// frame's view (a plane face-on to the view has no trace): its trace on
/// paper, clipped to the model's extent plus a margin and kept inside the
/// frame, the paper direction the section looks along, its letter.
fn section_marks(proj: &Projector, w: f64, h: f64, size: f64, extent: ([f64; 3], [f64; 3]),
                 planes: &[([f64; 3], [f64; 3], String)]) -> Vec<Annot> {
    let mut out = Vec::new();
    let (lo, hi) = extent;
    let mut pc = Vec::new();
    for x in [lo[0], hi[0]] {
        for y in [lo[1], hi[1]] {
            for z in [lo[2], hi[2]] {
                pc.push(proj.page([x, y, z]));
            }
        }
    }
    let inset = 1.0;
    let (fx0, fy0, fx1, fy1) = (inset, inset, w - inset, h - inset);
    let min = |i: usize| pc.iter().map(|q| q[i]).fold(f64::INFINITY, f64::min);
    let max = |i: usize| pc.iter().map(|q| q[i]).fold(f64::NEG_INFINITY, f64::max);
    let (bx0, bx1) = (min(0).max(fx0), max(0).min(fx1));
    let (by0, by1) = (min(1).max(fy0), max(1).min(fy1));
    if bx1 <= bx0 || by1 <= by0 {
        return out;
    }
    let margin = 6.0;
    let fwd = proj.forward();
    for (i, (o, n, symbol)) in planes.iter().enumerate() {
        let nv = glam::DVec3::from_array(*n);
        if nv.dot(fwd).abs() > 0.999 {
            continue;                                   // face-on: no trace
        }
        let d = nv.cross(fwd);
        let d = d / d.length().max(1e-12);
        let p0 = proj.page(*o);
        let p1 = proj.page([o[0] + d.x, o[1] + d.y, o[2] + d.z]);
        let (dx, dy) = (p1[0] - p0[0], p1[1] - p0[1]);
        let ln = dx.hypot(dy);
        if ln < 1e-9 {
            continue;
        }
        let (dx, dy) = (dx / ln, dy / ln);
        // Clip the infinite trace to the extent box (Liang–Barsky).
        let (mut t0, mut t1) = (-1e9, 1e9);
        for (pdir, q, lo, hi) in [(dx, p0[0], bx0, bx1), (dy, p0[1], by0, by1)] {
            if pdir.abs() < 1e-12 {
                if q < lo || q > hi {
                    t0 = 1.0;
                    t1 = 0.0;
                    break;
                }
                continue;
            }
            let (ta, tb) = ((lo - q) / pdir, (hi - q) / pdir);
            t0 = f64::max(t0, ta.min(tb));
            t1 = f64::min(t1, ta.max(tb));
        }
        if t1 <= t0 {
            continue;                                   // misses the model on paper
        }
        let (mut ta, mut tb) = (t0 - margin, t1 + margin);
        for (pdir, q, lo, hi) in [(dx, p0[0], fx0, fx1), (dy, p0[1], fy0, fy1)] {
            if pdir.abs() < 1e-12 {
                continue;
            }
            let (u, v) = ((lo - q) / pdir, (hi - q) / pdir);
            let (u, v) = if u <= v { (u, v) } else { (v, u) };
            ta = ta.max(u);
            tb = tb.min(v);
        }
        let a = [p0[0] + dx * ta, p0[1] + dy * ta];
        let b = [p0[0] + dx * tb, p0[1] + dy * tb];
        // The section looks along −n: that way point the arrows.
        let q = proj.page([o[0] - n[0], o[1] - n[1], o[2] - n[2]]);
        let (ax, ay) = (q[0] - p0[0], q[1] - p0[1]);
        let la = ax.hypot(ay);
        if la < 1e-9 {
            continue;
        }
        let label = match symbol.trim() {
            "" => char::from(b'A' + (i % 26) as u8).to_string(),
            s => s.to_string(),
        };
        out.push(Annot::SecMark { line: [a[0], a[1], b[0], b[1]], dir: [ax / la, ay / la], label, size });
    }
    out
}

/// `_chainage_marks`: a tick and «0+020» at every chainage step along each
/// traced path, labels standing perpendicular to it on the right of travel.
fn chainage_marks(proj: &Projector, size: f64, step_m: f64, paths: &[Vec<[f64; 3]>],
                  drape: &dyn Fn(f64, f64) -> PyResult<[f64; 3]>) -> PyResult<Vec<Annot>> {
    let mut out = Vec::new();
    let tick = 0.9;
    let pt = |x: f64, y: f64| -> PyResult<[f64; 2]> { Ok(proj.page(drape(x, y)?)) };
    for nodes in paths {
        let length = polyline_length(nodes);
        if nodes.len() < 2 || length <= 1e-9 {
            continue;
        }
        let step = super::items_b::chainage_step(length, step_m);
        let mut stations = Vec::new();
        let mut s = 0.0;
        while s <= length + 1e-6 {
            stations.push(s.min(length));
            s += step;
        }
        if length - stations[stations.len() - 1] > 0.3 * step {
            stations.push(length);
        }
        for s in stations {
            let [x, y] = point_at_station(nodes, s);
            let p = pt(x, y)?;
            let (dx, dy) = if s + 0.5 <= length {
                let [xq, yq] = point_at_station(nodes, s + 0.5);
                let q = pt(xq, yq)?;
                (q[0] - p[0], q[1] - p[1])
            } else {
                let [xq, yq] = point_at_station(nodes, (s - 0.5).max(0.0));
                let q = pt(xq, yq)?;
                (p[0] - q[0], p[1] - q[1])
            };
            let norm = dx.hypot(dy);
            let (dx, dy) = if norm < 1e-9 { (1.0, 0.0) } else { (dx / norm, dy / norm) };
            let (nx, ny) = (-dy, dx);
            out.push(Annot::Line([p[0] - nx * tick, p[1] - ny * tick, p[0] + nx * tick, p[1] + ny * tick]));
            let deg = super::items_b::py_mod(ny.atan2(nx).to_degrees() + 90.0, 180.0) - 90.0;
            let text = super::items_b::chainage(s, step);
            let width = (text.chars().count() as f64 * size * 0.55).max(1.0);
            let off = tick + 0.6 + width / 2.0;
            out.push(Annot::Text { at: [p[0] + nx * off, p[1] + ny * off], deg, text, size });
        }
    }
    Ok(out)
}

/// The box of what the frame's view shows of the model (its triangles,
/// sheets::geometry), from ``data``'s ``root``, ``groups`` and ``loose``.
fn model_extent(py: Python<'_>, camera: &Bound<'_, PyDict>, data: &Bound<'_, PyDict>)
    -> PyResult<Option<([f64; 3], [f64; 3])>> {
    let Some(root) = data.get_item("root")?.filter(|r| !r.is_none()) else { return Ok(None) };
    let state = match camera.get_item("view")?.filter(|v| !v.is_none()) {
        Some(v) => ViewState::of_view(&super::model::read_fields(doc::py::handle_key(&v)?)?),
        None => ViewState::default(),
    };
    let roots: Vec<doc::Key> = list_of(data, "groups")?.iter().map(doc::py::handle_key).collect::<PyResult<_>>()?;
    let loose = data.get_item("loose")?.filter(|l| !l.is_none()).map(Bound::unbind);
    let c = super::geometry::collect(py, doc::py::handle_key(&root)?, &roots, loose, &state,
                                     crate::hypot_of("libc++-fma")?)?;
    if c.tris.is_empty() {
        return Ok(None);
    }
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for t in &c.tris {
        for v in t {
            for i in 0..3 {
                lo[i] = lo[i].min(v[i]);
                hi[i] = hi[i].max(v[i]);
            }
        }
    }
    Ok(Some((lo, hi)))
}

fn list_of<'py>(d: &Bound<'py, PyDict>, key: &str) -> PyResult<Vec<Bound<'py, PyAny>>> {
    match d.get_item(key)?.filter(|v| !v.is_none()) {
        Some(v) => v.try_iter()?.collect(),
        None => Ok(Vec::new()),
    }
}

/// A frame's annotations (`compute_annotations`) as the composer's tuples.
///
/// ``camera``: the frame camera's inputs ``{view, base, bounds}`` (see
/// ``sheet_frame_camera``). ``data``, read off the scene by the host:
/// ``root`` (the document, for the tags the frame's view shows),
/// ``paths`` ``[(draped points, closed, profile nodes)]``, ``drape(x, y)``
/// → the ground point, ``dims`` ``[(a, b, line a, line b, text, tag,
/// hidden)]`` and ``ends`` (the dimension style's), ``labels`` ``[(anchor,
/// position, text, tag, hidden)]``, ``planes`` ``[(point, normal,
/// symbol)]`` with the model's ``extent`` ``(lo, hi)`` (else worked out
/// from ``groups`` and ``loose``, the scene's), and the
/// extensions' ``providers`` (a list a failing one is taken out of) with
/// the ``scene`` they are given.
#[pyfunction]
pub fn sheet_frame_annotations<'py>(py: Python<'py>, frame: &Bound<'py, PyAny>, camera: &Bound<'py, PyDict>,
                                    data: &Bound<'py, PyDict>) -> PyResult<Bound<'py, PyList>> {
    py.import("core.doc_model")?.call_method0("flush")?;
    let (f, proj) = Projector::of(frame, camera, true)?;
    let (w, h) = (f.num("w_mm", 100.0), f.num("h_mm", 100.0));
    let size = match f.num("annot_text_mm", 2.8) { s if s != 0.0 => s, _ => 2.8 };
    let tags = match data.get_item("root")?.filter(|r| !r.is_none()) {
        Some(root) => {
            let state = match camera.get_item("view")?.filter(|v| !v.is_none()) {
                Some(v) => ViewState::of_view(&super::model::read_fields(doc::py::handle_key(&v)?)?),
                None => ViewState::default(),
            };
            let root = doc::py::handle_key(&root)?;
            Some(Tags::read(&doc::lock(), root, &state)?)
        }
        None => None,
    };
    let shown = |tag: &Bound<'_, PyAny>, hidden: &Bound<'_, PyAny>| -> PyResult<bool> {
        if hidden.is_truthy()? {
            return Ok(false);
        }
        let key = if tag.is_none() { super::visibility::DEFAULT_LAYER.to_string() } else { tag.str()?.to_string() };
        let key = if key.is_empty() { super::visibility::DEFAULT_LAYER.to_string() } else { key };
        Ok(tags.as_ref().is_none_or(|t| t.shown(&key)))
    };
    let mut out: Vec<Annot> = Vec::new();
    let mut nodes_of = Vec::new();
    for path in list_of(data, "paths")? {
        let pts: Vec<[f64; 2]> = path.get_item(0)?.try_iter()?.map(|q| Ok(proj.page(pt3(&q?)?))).collect::<PyResult<_>>()?;
        let nodes: Vec<[f64; 3]> = path.get_item(2)?.try_iter()?.map(|q| pt3(&q?)).collect::<PyResult<_>>()?;
        nodes_of.push(nodes);
        if pts.len() < 2 {
            continue;
        }
        let mut pts = pts;
        if path.get_item(1)?.is_truthy()? && pts.len() > 2 {
            pts.push(pts[0]);
        }
        out.push(Annot::Poly(pts));
    }
    if f.flag("km_marks", false) {
        let drape_fn = data.get_item("drape")?.filter(|d| !d.is_none());
        let drape = |x: f64, y: f64| -> PyResult<[f64; 3]> {
            match &drape_fn {
                Some(d) => pt3(&d.call1((x, y))?),
                None => Ok([x, y, 0.0]),
            }
        };
        out.extend(chainage_marks(&proj, size, f.num("km_step_m", 0.0), &nodes_of, &drape)?);
    }
    if f.flag("section_marks", false) {
        let planes: Vec<([f64; 3], [f64; 3], String)> = list_of(data, "planes")?.iter().map(|sp| {
            let symbol = sp.get_item(2)?;
            Ok((pt3(&sp.get_item(0)?)?, pt3(&sp.get_item(1)?)?,
                if symbol.is_none() { String::new() } else { symbol.str()?.to_string() }))
        }).collect::<PyResult<_>>()?;
        let extent = match data.get_item("extent")?.filter(|e| !e.is_none()) {
            Some(e) => Some((pt3(&e.get_item(0)?)?, pt3(&e.get_item(1)?)?)),
            None if !planes.is_empty() => model_extent(py, camera, data)?,
            None => None,
        };
        if let Some(extent) = extent.filter(|_| !planes.is_empty()) {
            out.extend(section_marks(&proj, w, h, size, extent, &planes));
        }
    }
    if let Some(providers) = data.get_item("providers")?.filter(|p| !p.is_none()) {
        let providers = providers.cast_into::<PyList>()?;
        let scene = data.get_item("scene")?.unwrap_or_else(|| py.None().into_bound(py));
        let scale_n = if proj.perspective() { 0.0 } else { f.num("scale_n", 100.0) };
        out.extend(extension_marks(py, &proj, size, scale_n, &scene, &providers)?);
    }
    if f.flag("annotations", false) {
        let ends = match data.get_item("ends")?.filter(|e| !e.is_none()) {
            Some(e) => match e.str()?.to_string() { s if s.is_empty() => "arrow".to_string(), s => s },
            None => "arrow".to_string(),
        };
        for dim in list_of(data, "dims")? {
            if !shown(&dim.get_item(5)?, &dim.get_item(6)?)? {
                continue;
            }
            let at = |i: usize| -> PyResult<[f64; 2]> { Ok(proj.page(pt3(&dim.get_item(i)?)?)) };
            let (a, b, a2, b2) = (at(0)?, at(1)?, at(2)?, at(3)?);
            out.push(Annot::Line([a[0], a[1], a2[0], a2[1]]));
            out.push(Annot::Line([b[0], b[1], b2[0], b2[1]]));
            out.push(Annot::Line([a2[0], a2[1], b2[0], b2[1]]));
            let ang = (b2[1] - a2[1]).atan2(b2[0] - a2[0]);
            for (q, heading) in [(a2, ang), (b2, ang + PI)] {
                if ends == "tick" {                      // oblique ticks
                    let (t, k) = (1.6, super::items_b::oblique_tick_angle(ang));
                    out.push(Annot::Line([q[0] - t * k.cos(), q[1] - t * k.sin(), q[0] + t * k.cos(), q[1] + t * k.sin()]));
                } else if ends != "none" {               // inside arrows
                    out.push(Annot::Arrow([q[0], q[1], heading.cos(), heading.sin()]));
                }
            }
            let text = dim.get_item(4)?.str()?.to_string();
            let deg = super::items_b::readable_deg(b2[0] - a2[0], b2[1] - a2[1]);
            out.push(Annot::Text { at: [(a2[0] + b2[0]) / 2.0, (a2[1] + b2[1]) / 2.0], deg, text, size });
        }
        for lab in list_of(data, "labels")? {
            if !shown(&lab.get_item(3)?, &lab.get_item(4)?)? {
                continue;
            }
            let a = proj.page(pt3(&lab.get_item(0)?)?);
            let p = proj.page(pt3(&lab.get_item(1)?)?);
            out.push(Annot::Line([a[0], a[1], p[0], p[1]]));
            let mut lines = split_lines(&lab.get_item(2)?.str()?.to_string());
            if lines.is_empty() {
                lines.push(String::new());
            }
            let side = if p[0] >= a[0] { 1.0 } else { -1.0 };  // away from the anchor
            for (i, line) in lines.into_iter().enumerate() {
                let width = (line.chars().count() as f64 * size * 0.55).max(1.0);
                let x = p[0] + side * (1.5 + width / 2.0);
                out.push(Annot::Text { at: [x, p[1] + i as f64 * size * 1.3], deg: 0.0, text: line, size });
            }
        }
    }
    PyList::new(py, out.iter().map(|a| to_tuple(py, a)).collect::<PyResult<Vec<_>>>()?)
}

/// A frame's annotations as the page draws them over its content: a
/// display list in frame-local mm.
#[pyfunction]
pub fn sheet_frame_annots(frame: &Bound<'_, PyAny>, annots: &Bound<'_, PyAny>) -> PyResult<String> {
    let f = super::model::read_fields(doc::py::handle_key(frame)?)?;
    let items = parse(annots)?;
    let mut p = Painter::default();
    if !items.is_empty() {
        paint_annots(&mut p, f.num("w_mm", 100.0), f.num("h_mm", 100.0), &items);
    }
    Ok(super::paint::ops_json(&p.ops).to_string())
}
