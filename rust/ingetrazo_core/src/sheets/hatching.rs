// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! A vector frame's cut filled layer by layer (views/composer.py
//! `_paint_cut_fills_mm`, `_fill_cut_rings`; views/hatch_paint.py
//! `paint_hatch`, `paint_fitted`, `paint_bond`): each ring's look — its
//! layer's fill colour, hatch pattern, ground, ink and form (angle, scale,
//! an imported pattern, masonry joints, timber grain) — grouped, and each
//! group filled the frame's way or with its pattern. The pattern geometry
//! (where each mark goes) is the model's hatch engine, `core.hatches`,
//! asked here; what is drawn of it, with which pen and clip, is ported.

use super::paint::{Cap, Painter, Pen, Rgba};
use super::model::Fields;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

pub type Ring = Vec<[f64; 2]>;

/// One ring's look: colour, pattern, ground, ink (rgb 0–1) and form.
pub struct Look {
    pub colour: Option<[f64; 3]>,
    pub hatch: Option<String>,
    pub ground: Option<[f64; 3]>,
    pub ink: Option<[f64; 3]>,
    pub form: Option<Py<PyDict>>,
}

fn rgb_of(c: [f64; 3]) -> Rgba {
    Rgba::from_f(c[0], c[1], c[2])
}

fn ring_py<'py>(py: Python<'py>, r: &Ring) -> PyResult<Bound<'py, PyAny>> {
    Ok(numpy::PyArray2::from_vec2(py, &r.iter().map(|p| p.to_vec()).collect::<Vec<_>>())?.into_any())
}

fn bbox(rings: &[Ring]) -> [f64; 4] {
    let (mut l, mut t, mut r, mut b) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for q in rings.iter().flatten() {
        l = l.min(q[0]);
        t = t.min(q[1]);
        r = r.max(q[0]);
        b = b.max(q[1]);
    }
    [l, t, r, b]
}

fn pts_of(v: &Bound<'_, PyAny>) -> PyResult<Vec<[f64; 2]>> {
    v.try_iter()?.map(|q| {
        let q = q?;
        Ok([q.get_item(0)?.extract()?, q.get_item(1)?.extract()?])
    }).collect()
}

/// `paint_hatch`: pattern `key` inside `rings` (even-odd), over `ground`.
#[allow(clippy::too_many_arguments)]
fn paint_hatch(py: Python<'_>, p: &mut Painter, rings: &[Ring], key: &str, ink: Rgba, width: f64, scale: f64,
               ground: Option<Rgba>, angle: f64, families: &Bound<'_, PyAny>, offset: &Bound<'_, PyAny>) -> PyResult<()> {
    if rings.is_empty() {
        return Ok(());
    }
    p.save();
    if key == "solid" {
        p.set_pen(Pen::none());
        p.set_brush(Some(ink));
        p.path(rings);
        p.restore();
        return Ok(());
    }
    if let Some(g) = ground {
        p.set_pen(Pen::none());
        p.set_brush(Some(g));
        p.path(rings);
    }
    p.clip_path(rings);
    let [l, t, r, b] = bbox(rings);
    let marks = py.import("core.hatches")?.getattr("hatch_marks")?
        .call1((key, (l, t, r, b), scale, angle, families, offset))?;
    let w = width.max(0.03);
    p.set_pen(Pen::new(ink, w).cap(Cap::Round));
    p.set_brush(None);
    for seg in marks.get_item("lines")?.try_iter()? {
        let s: (f64, f64, f64, f64) = seg?.extract()?;
        p.line(s.0, s.1, s.2, s.3);
    }
    for poly in marks.get_item("polys")?.try_iter()? {
        p.polyline(&pts_of(&poly?)?);
    }
    for poly in marks.get_item("closed")?.try_iter()? {
        p.polygon(&pts_of(&poly?)?);
    }
    for c in marks.get_item("circles")?.try_iter()? {
        let (x, y, r): (f64, f64, f64) = c?.extract()?;
        p.ellipse(x, y, r, r);
    }
    let dots: Vec<[f64; 2]> = pts_of(&marks.get_item("dots")?)?;
    if !dots.is_empty() {
        // A round dot as wide as the pen's 1.8.
        p.set_pen(Pen::none());
        p.set_brush(Some(ink));
        for d in dots {
            p.ellipse(d[0], d[1], w * 0.9, w * 0.9);
        }
    }
    p.restore();
    Ok(())
}

/// `paint_fitted`: a symbol fitted to each piece's thickness.
#[allow(clippy::too_many_arguments)]
fn paint_fitted(py: Python<'_>, p: &mut Painter, rings: &[Ring], key: &str, ink: Rgba, width: f64, density: f64,
                ground: Option<Rgba>) -> PyResult<()> {
    if rings.is_empty() {
        return Ok(());
    }
    let h = py.import("core.hatches")?;
    p.save();
    if let Some(g) = ground {
        p.set_pen(Pen::none());
        p.set_brush(Some(g));
        p.path(rings);
    }
    p.clip_path(rings);
    p.set_brush(None);
    for ring in rings {
        let band = h.getattr("fit_band")?.call1((ring_py(py, ring)?,))?;
        if band.is_none() {
            continue;
        }
        // A thin piece thins its line: a twentieth of its thickness at most.
        let thick: f64 = band.get_item(4)?.extract()?;
        p.set_pen(Pen::new(ink, width.min(thick / 20.0).max(0.03)).cap(Cap::Round));
        let kw = PyDict::new(py);
        kw.set_item("density", density)?;
        let mut args = vec![key.into_pyobject(py)?.into_any()];
        args.extend(band.try_iter()?.collect::<PyResult<Vec<_>>>()?);
        for poly in h.getattr("fitted_marks")?.call(PyTuple::new(py, args)?, Some(&kw))?.try_iter()? {
            p.polyline(&pts_of(&poly?)?);
        }
    }
    p.restore();
    Ok(())
}

/// `paint_bond`: masonry as it is laid in each ring; the rings it cannot
/// lay that way come back.
#[allow(clippy::too_many_arguments)]
fn paint_bond<'py>(py: Python<'py>, p: &mut Painter, rings: &[Ring], key: &str, ink: Rgba, width: f64,
                   ground: Option<Rgba>, form: &Bound<'py, PyDict>, scale: f64, joints: &str) -> PyResult<Vec<Ring>> {
    let h = py.import("core.hatches")?;
    let get = |k: &str| -> PyResult<Option<Bound<'py, PyAny>>> { Ok(form.get_item(k)?.filter(|v| !v.is_none())) };
    let cut: String = get("cut")?.map(|v| v.extract()).transpose()?.unwrap_or_default();
    let mortar = get("mortar")?.map(|v| v.is_truthy()).transpose()?.unwrap_or(false) && joints == "double";
    let mortar_c = Rgba(205, 205, 205, 255);
    let mut rest = Vec::new();
    p.save();
    if let Some(u) = if mortar { Some(mortar_c) } else { ground } {
        p.set_pen(Pen::none());
        p.set_brush(Some(u));
        p.path(rings);
    }
    p.clip_path(rings);
    p.set_brush(None);
    let scale_n: f64 = get("scale_n")?.map(|v| v.extract()).transpose()?.unwrap_or(100.0);
    for ring in rings {
        if ring.len() < 3 {
            continue;
        }
        let band = h.getattr(if cut == "plan" { "fit_band" } else { "vertical_band" })?.call1((ring_py(py, ring)?,))?;
        let marks = h.getattr("bond_marks")?.call1((key, &band, &cut, scale, scale_n, get("brick")?, joints, get("offset")?))?;
        if marks.is_none() {
            rest.push(ring.clone());
            continue;
        }
        p.save();
        p.clip_path(std::slice::from_ref(ring));
        // A thin wall thins its joints, or they run together.
        let thick: f64 = band.get_item(4)?.extract()?;
        p.set_pen(Pen::new(ink, width.min(thick / 16.0).max(0.03)).cap(Cap::Flat));
        for seg in marks.get_item("lines")?.try_iter()? {
            let s: (f64, f64, f64, f64) = seg?.extract()?;
            p.line(s.0, s.1, s.2, s.3);
        }
        let bricks = marks.get_item("bricks")?;
        if bricks.is_truthy()? {
            if mortar {
                p.set_brush(Some(ground.unwrap_or(Rgba(255, 255, 255, 255))));
            }
            for b in bricks.try_iter()? {
                p.polygon(&pts_of(&b?)?);
            }
            p.set_brush(None);
        }
        for poly in marks.get_item("polys")?.try_iter()? {
            p.polyline(&pts_of(&poly?)?);
        }
        p.restore();
    }
    p.restore();
    Ok(rest)
}

fn hatch_pen(f: &Fields) -> f64 {
    super::items_d::hatch_pen_mm(f)
}

/// `_fill_cut_rings` for one look's rings.
fn fill_rings(py: Python<'_>, p: &mut Painter, f: &Fields, rings: &[Ring], look: &Look) -> PyResult<()> {
    let mode = { let m = f.text("cut_fill", "solid"); if m.is_empty() { "solid".to_string() } else { m } };
    let rings: Vec<Ring> = rings.iter().filter(|r| r.len() >= 3).cloned().collect();
    if rings.is_empty() {
        return Ok(());
    }
    let frame_colour = { let c = f.text("cut_fill_color", ""); Rgba::parse(if c.is_empty() { "#595e69" } else { &c }) };
    let color = look.colour.map(rgb_of).unwrap_or(frame_colour);
    let h = py.import("core.hatches")?;
    if let Some(hatch) = look.hatch.as_deref().filter(|s| !s.is_empty()) {
        // The layer's colour as QColor.getRgbF gives it back (not its 8 bits).
        let [cr, cg, cb] = look.colour.unwrap_or([0.0; 3]);
        let (ink_frame, pale) = if look.colour.is_some() && hatch != "solid" {
            (Rgba::from_f(cr * 0.7, cg * 0.7, cb * 0.7),
             Some(Rgba::from_f(cr + (1.0 - cr) * 0.82, cg + (1.0 - cg) * 0.82, cb + (1.0 - cb) * 0.82)))
        } else {
            (color, None)
        };
        let form = match &look.form { Some(d) => d.bind(py).clone(), None => PyDict::new(py) };
        let fget = |k: &str| -> PyResult<Option<Bound<'_, PyAny>>> { Ok(form.get_item(k)?.filter(|v| !v.is_none())) };
        let fnum = |k: &str, d: f64| -> PyResult<f64> {
            Ok(match fget(k)? { Some(v) if v.is_truthy()? => v.extract()?, _ => d })
        };
        let frame_scale = (f.num("cut_hatch_mm", 1.5) / 1.5).max(0.2);
        let scale = frame_scale * fnum("scale", 1.0)?;
        let width = hatch_pen(f);
        let ink = look.ink.map(rgb_of).unwrap_or(ink_frame);
        let under = look.ground.map(rgb_of).or(pale);
        let isin = |set: &str| -> PyResult<bool> { h.getattr(set)?.contains(hatch) };
        let angle = fnum("angle", 0.0)?;
        let families = fget("families")?.map_or_else(|| py.None().into_bound(py), |v| v);
        let mut rings = rings;
        if isin("WOOD")? && fget("cross")?.is_some_and(|v| v.is_truthy().unwrap_or(false)) && fget("cut")?.is_some() {
            // Timber: a cross on each piece cut across its grain.
            let n = fnum("scale_n", 100.0)?;
            let depth = fget("depth")?;
            let mut across = Vec::new();
            let mut rest = Vec::new();
            for r in rings {
                let band = h.getattr("fit_band")?.call1((ring_py(py, &r)?,))?;
                if h.getattr("cut_across")?.call1((band, n, depth.as_ref()))?.is_truthy()? {
                    across.push(r);
                } else {
                    rest.push(r);
                }
            }
            if !across.is_empty() {
                paint_fitted(py, p, &across, "wood_cross", ink, width, 1.0, under)?;
                if rest.is_empty() {
                    return Ok(());
                }
            }
            rings = rest;
        }
        let none = py.None().into_bound(py);
        let offset = h.getattr("paper_offset")?.call1((&form,))?;
        if isin("COURSED")? && fget("cut")?.map(|c| c.extract::<String>()).transpose()?.is_some_and(|c| c == "plan" || c == "vertical") {
            let jm: String = h.getattr("joints_mode")?.call1((fget("joints")?, fnum("scale_n", 100.0)?))?.extract()?;
            if jm == "hatch" {
                // Too small a scale for bricks: the 45° hatch.
                return paint_hatch(py, p, &rings, "diagonal", ink, width, frame_scale * fnum("hatch_scale", 1.0)?, under, 0.0,
                                   &none, &none);
            }
            if jm == "none" {
                if let Some(u) = under {
                    p.save();
                    p.set_pen(Pen::none());
                    p.set_brush(Some(u));
                    p.path(&rings);
                    p.restore();
                }
                return Ok(());
            }
            let rest = paint_bond(py, p, &rings, hatch, ink, width, under, &form, scale, &jm)?;
            if !rest.is_empty() {
                paint_hatch(py, p, &rest, hatch, ink, width, scale, None, angle, &families, &offset)?;
            }
            return Ok(());
        }
        if h.getattr("FITTED")?.contains(hatch)? {
            return paint_fitted(py, p, &rings, hatch, ink, width, 1.0 / fnum("scale", 1.0)?.max(0.2), under);
        }
        let off = if isin("PLACED")? { offset } else { none };
        return paint_hatch(py, p, &rings, hatch, ink, width, scale, under, angle, &families, &off);
    }
    p.save();
    if mode == "hatch" {
        p.clip_path(&rings);
        let step = f.num("cut_hatch_mm", 1.5).max(0.3);
        p.set_pen(Pen::new(color, hatch_pen(f)));
        let [l, t, r, b] = bbox(&rings);
        let mut c = ((l + t) / step).floor() * step;
        while c <= r + b {
            let (xa, xb) = (l.max(c - b), r.min(c - t));
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
    Ok(())
}

/// `_paint_cut_fills_mm` with a look per ring: one pass per look, in the
/// order the looks first appear (each even-odd on its own).
pub fn paint_fills(py: Python<'_>, p: &mut Painter, f: &Fields, rings: &[Ring], looks: &[Look]) -> PyResult<()> {
    let mode = f.text("cut_fill", "solid");
    if rings.is_empty() || mode == "none" {
        return Ok(());
    }
    let key = |l: &Look| -> PyResult<String> {
        let form = match &l.form { Some(d) => d.bind(py).repr()?.to_string(), None => String::new() };
        Ok(format!("{:?}|{:?}|{:?}|{:?}|{form}", l.colour.map(rgb_of), l.hatch, l.ground.map(rgb_of), l.ink.map(rgb_of)))
    };
    let mut groups: Vec<(String, usize, Vec<Ring>)> = Vec::new();
    for (i, (ring, look)) in rings.iter().zip(looks).enumerate() {
        let k = key(look)?;
        match groups.iter_mut().find(|g| g.0 == k) {
            Some(g) => g.2.push(ring.clone()),
            None => groups.push((k, i, vec![ring.clone()])),
        }
    }
    for (_, first, group) in groups {
        fill_rings(py, p, f, &group, &looks[first])?;
    }
    Ok(())
}
