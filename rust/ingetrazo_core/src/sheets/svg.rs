// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! The SVG writer of a view's line groups (docs/export-svg.md §3, docs/rust-sheets.md §5).
//!
//! Byte for byte what `formats/svg_out.py`'s `svg_text` wrote for the
//! lines: one Inkscape layer per non-empty group, its segments chained into
//! paths, the layer's colour, line type and pen, the drawing at 1:N or
//! fitted on a sheet, cut to a window. Face and picture layers are still
//! drawn in Python and come in as `under` (they go below the lines).

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};
use std::collections::{HashMap, HashSet};

/// The pieces of a line group (`(layer, segments[, linetype[, rgb[,
/// dashes[, weight_mm]]]])`, segments in metres).
#[derive(Clone, Debug, Default)]
pub struct Group {
    pub name: String,
    pub segs: Vec<[f64; 4]>,
    pub linetype: Option<String>,
    pub rgb: Option<[f64; 3]>,
    pub dashes: Option<Vec<f64>>,
    pub weight: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub scale_n: f64,
    /// cut / profile / edge pens in paper mm.
    pub pens: [f64; 3],
    pub line_colours: bool,
    pub paper: Option<(f64, f64)>,
    /// The part of the drawing the page shows; the layers are clipped to it.
    pub window: Option<[f64; 4]>,
    /// The page's window when there is no `window` (a picture's).
    pub fixed: Option<[f64; 4]>,
    /// Other things on the page (faces) the bounds must hold.
    pub extra: Option<[f64; 4]>,
    pub margin: Option<f64>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            scale_n: 100.0, pens: DEFAULT_PENS, line_colours: true, paper: None,
            window: None, fixed: None, extra: None, margin: None,
        }
    }
}

/// formats.svg_out.DEFAULT_PENS: cut, profile, edge.
pub const DEFAULT_PENS: [f64; 3] = [0.5, 0.35, 0.18];
const MARGIN: f64 = 10.0;
const INKSCAPE: &str = "http://www.inkscape.org/namespaces/inkscape";
/// formats.svg_out.MAX_TURN_DEG.
pub const MAX_TURN_DEG: f64 = 30.0;

/// Where the drawing lands on the page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub min_x: f64,
    pub max_y: f64,
    pub k: f64,
    pub off_x: f64,
    pub off_y: f64,
    pub width: f64,
    pub height: f64,
    pub scale_n: f64,
}

impl Layout {
    pub fn px(&self, x: f64) -> f64 {
        (x - self.min_x) * self.k + self.off_x
    }
    pub fn py(&self, y: f64) -> f64 {
        (self.max_y - y) * self.k + self.off_y
    }
}

#[inline]
fn py_max(a: f64, b: f64) -> f64 {
    if b > a { b } else { a }
}

#[inline]
fn py_min(a: f64, b: f64) -> f64 {
    if b < a { b } else { a }
}

/// Python's `format(x, "g")`.
pub fn fmt_g(x: f64) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if x == 0.0 {
        return if x.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    let sci = format!("{:.5e}", x);
    let (mant, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let strip = |s: &str| -> String {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s.to_string()
        }
    };
    if (-4..6).contains(&exp) {
        strip(&format!("{:.*}", (5 - exp) as usize, x))
    } else {
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{}e{}{:02}", strip(mant), sign, exp.abs())
    }
}

/// Python's `round(x)` on a float (halves to even).
fn py_round(x: f64) -> f64 {
    x.round_ties_even()
}

/// formats.svg_out._scale_label.
pub fn scale_label(scale_n: f64) -> String {
    let n = if scale_n >= 10.0 {
        py_round(scale_n)
    } else {
        format!("{:.1}", scale_n).parse().unwrap_or(scale_n)
    };
    format!("1:{}", fmt_g(n))
}

fn line_class(name: &str) -> usize {
    let upper = name.to_uppercase();
    if upper.ends_with("-CORTE") {
        0
    } else if upper.ends_with("-PERFIL") {
        1
    } else {
        2
    }
}

fn hex(rgb: Option<[f64; 3]>) -> String {
    match rgb {
        None => "#000000".into(),
        Some(c) => {
            let v = c.map(|x| py_round(x * 255.0).clamp(0.0, 255.0) as u8);
            format!("#{:02x}{:02x}{:02x}", v[0], v[1], v[2])
        }
    }
}

/// formats.svg_out._xml_id.
pub fn xml_id(name: &str, taken: &mut HashSet<String>) -> String {
    let mut base: String = name
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') { c } else { '_' })
        .collect();
    if base.is_empty() {
        base = "capa".into();
    }
    if !base.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        base = format!("_{base}");
    }
    let mut out = base.clone();
    let mut i = 2;
    while taken.contains(&out) {
        out = format!("{base}_{i}");
        i += 1;
    }
    taken.insert(out.clone());
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// xml.sax.saxutils.quoteattr.
pub fn quoteattr(text: &str) -> String {
    let data = escape(text).replace('\n', "&#10;").replace('\r', "&#13;").replace('\t', "&#9;");
    if data.contains('"') {
        if data.contains('\'') {
            format!("\"{}\"", data.replace('"', "&quot;"))
        } else {
            format!("'{data}'")
        }
    } else {
        format!("\"{data}\"")
    }
}

fn dash_of(linetype: &str) -> Option<&'static str> {
    Some(match linetype.to_uppercase().as_str() {
        "DASHED" => "1.5 0.8",
        "HIDDEN" => "0.8 0.4",
        "DASHEDX2" => "3 1.5",
        "DOT" => "0 0.6",
        "DASHDOT" => "1.5 0.6 0 0.6",
        "DIVIDE" => "1.5 0.6 0 0.6 0 0.6",
        "CENTER" => "4 0.8 0.8 0.8",
        "PHANTOM" => "4 0.8 0.8 0.8 0.8 0.8",
        "BORDER" => "1.5 0.6 1.5 0.6 0 0.6",
        _ => return None,
    })
}

/// A chained polyline and whether it closes on itself.
pub type Chain = (Vec<[f64; 2]>, bool);

/// formats.svg_out.chain_segments: join segments that continue one another
/// (exactly two meet, turning at most `MAX_TURN_DEG`) into polylines.
pub fn chain_segments(segs: &[[f64; 4]], tol: f64) -> Vec<Chain> {
    let inv = 1.0 / tol;
    let key = |x: f64, y: f64| ((x * inv).round_ties_even() as i64, (y * inv).round_ties_even() as i64);
    let cos_max = MAX_TURN_DEG.to_radians().cos();
    let mut at: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, s) in segs.iter().enumerate() {
        let (k0, k1) = (key(s[0], s[1]), key(s[2], s[3]));
        if k0 == k1 {
            continue;
        }
        at.entry(k0).or_default().push(i);
        at.entry(k1).or_default().push(i);
    }
    let mut used = vec![false; segs.len()];
    let walk = |pts: &mut Vec<[f64; 2]>, used: &mut Vec<bool>| loop {
        let last = pts[pts.len() - 1];
        let Some(here) = at.get(&key(last[0], last[1])) else { return };
        if here.len() != 2 {
            return;
        }
        let Some(&nxt) = here.iter().find(|&&j| !used[j]) else { return };
        let s = segs[nxt];
        let far = if key(s[0], s[1]) == key(last[0], last[1]) { [s[2], s[3]] } else { [s[0], s[1]] };
        let prev = pts[pts.len() - 2];
        let (ax, ay) = (last[0] - prev[0], last[1] - prev[1]);
        let (bx, by) = (far[0] - last[0], far[1] - last[1]);
        let (la, lb) = (super::pymath::hypot(ax, ay), super::pymath::hypot(bx, by));
        if la * lb == 0.0 || (ax * bx + ay * by) / (la * lb) < cos_max {
            return;
        }
        used[nxt] = true;
        pts.push(far);
    };
    let mut chains = Vec::new();
    for (i, s) in segs.iter().enumerate() {
        if used[i] || key(s[0], s[1]) == key(s[2], s[3]) {
            continue;
        }
        used[i] = true;
        let mut fwd = vec![[s[0], s[1]], [s[2], s[3]]];
        walk(&mut fwd, &mut used);
        let mut back = vec![[s[2], s[3]], [s[0], s[1]]];
        walk(&mut back, &mut used);
        let mut pts: Vec<[f64; 2]> = back[2..].iter().rev().copied().collect();
        pts.extend(fwd);
        let closed = pts.len() > 3 && {
            let (a, b) = (pts[0], pts[pts.len() - 1]);
            key(a[0], a[1]) == key(b[0], b[1])
        };
        if closed {
            pts.pop();
        }
        chains.push((pts, closed));
    }
    chains
}

/// Where the drawing goes on the page (the first half of `svg_text`).
pub fn layout(groups: &[Group], o: &Options) -> Layout {
    let margin = o.margin.unwrap_or(MARGIN);
    let [min_x, min_y, max_x, max_y] = if let Some(w) = o.window.or(o.fixed) {
        w
    } else {
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        let mut any = false;
        let mut take = |x: f64, y: f64| {
            any = true;
            b = [py_min(b[0], x), py_min(b[1], y), py_max(b[2], x), py_max(b[3], y)];
        };
        for g in groups {
            for s in &g.segs {
                take(s[0], s[1]);
                take(s[2], s[3]);
            }
        }
        if let Some(e) = o.extra {
            take(e[0], e[1]);
            take(e[2], e[3]);
        }
        if any { b } else { [0.0; 4] }
    };
    let bw = py_max(max_x - min_x, 1e-9);
    let bh = py_max(max_y - min_y, 1e-9);
    if let Some((width, height)) = o.paper {
        let k = py_max(py_min((width - 2.0 * margin) / bw, (height - 2.0 * margin) / bh), 1e-9);
        Layout {
            min_x, max_y, k,
            off_x: (width - bw * k) / 2.0,
            off_y: (height - bh * k) / 2.0,
            width, height,
            scale_n: 1000.0 / k,
        }
    } else {
        let k = 1000.0 / py_max(1e-9, o.scale_n);
        Layout {
            min_x, max_y, k, off_x: margin, off_y: margin,
            width: (max_x - min_x) * k + 2.0 * margin,
            height: (max_y - min_y) * k + 2.0 * margin,
            scale_n: o.scale_n,
        }
    }
}

/// The SVG of a view's line groups as `(text, line segments)`. `under`
/// gets the layout and the ids taken so far and returns the layers that go
/// below the lines (faces, the picture of the view).
pub fn svg_text(
    groups: &[Group],
    o: &Options,
    under: Option<&mut dyn FnMut(&Layout, &mut HashSet<String>) -> String>,
) -> (String, usize) {
    let groups: Vec<&Group> = groups.iter().filter(|g| !g.segs.is_empty()).collect();
    let owned: Vec<Group> = groups.iter().map(|g| (*g).clone()).collect();
    let l = layout(&owned, o);
    let (width, height) = (l.width, l.height);
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
         xmlns:inkscape=\"{INKSCAPE}\" version=\"1.1\" width=\"{width:.3}mm\" height=\"{height:.3}mm\" \
         viewBox=\"0 0 {width:.3} {height:.3}\">\n"
    ));
    out.push_str(&format!("<desc>{}</desc>\n", escape(&scale_label(l.scale_n))));
    let mut taken: HashSet<String> = HashSet::from(["pagina".to_string()]);
    let mut clip = "";
    if o.window.is_some() {
        out.push_str(&format!(
            "<defs><clipPath id=\"pagina\"><rect x=\"0\" y=\"0\" width=\"{width:.3}\" height=\"{height:.3}\"/></clipPath></defs>\n"
        ));
        clip = " clip-path=\"url(#pagina)\"";
    }
    if let Some(under) = under {
        out.push_str(&under(&l, &mut taken));
    }
    let mut n = 0;
    for g in groups {
        let name = if g.name.is_empty() { "VISTA" } else { g.name.as_str() };
        let rgb = if o.line_colours { g.rgb } else { None };
        let weight = g.weight.filter(|w| *w != 0.0).unwrap_or(o.pens[line_class(name)]);
        let mut attrs = format!(
            " fill=\"none\" stroke=\"{}\" stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"",
            hex(rgb),
            fmt_g(weight)
        );
        let linetype = g.linetype.as_deref().filter(|s| !s.is_empty()).unwrap_or("CONTINUOUS");
        let dash = match dash_of(linetype) {
            Some(d) => Some(d.to_string()),
            None => g
                .dashes
                .as_ref()
                .filter(|d| !d.is_empty())
                .map(|d| d.iter().map(|v| fmt_g(v.abs() * 10.0)).collect::<Vec<_>>().join(" ")),
        };
        if let Some(d) = dash {
            attrs.push_str(&format!(" stroke-dasharray=\"{d}\""));
        }
        out.push_str(&format!(
            "<g id=\"{}\" inkscape:groupmode=\"layer\" inkscape:label={}{attrs}{clip}>\n",
            xml_id(name, &mut taken),
            quoteattr(name)
        ));
        for (pts, closed) in chain_segments(&g.segs, 1e-6) {
            let d: Vec<String> = pts.iter().map(|p| format!("{:.3},{:.3}", l.px(p[0]), l.py(p[1]))).collect();
            out.push_str(&format!("<path d=\"M{}{}\"/>\n", d.join(" L"), if closed { " Z" } else { "" }));
        }
        n += g.segs.len();
        out.push_str("</g>\n");
    }
    out.push_str("</svg>\n");
    (out, n)
}

// ---------------------------------------------------------------- Python

fn segs_of(obj: &Bound<'_, PyAny>) -> PyResult<Vec<[f64; 4]>> {
    if let Ok(a) = obj.extract::<numpy::PyReadonlyArray2<'_, f64>>() {
        let a = a.as_array();
        if a.nrows() == 0 {
            return Ok(Vec::new());
        }
        if a.ncols() != 4 {
            return Err(PyValueError::new_err("segments are (N,4)"));
        }
        return Ok((0..a.nrows()).map(|i| [a[[i, 0]], a[[i, 1]], a[[i, 2]], a[[i, 3]]]).collect());
    }
    let mut out = Vec::new();
    for s in obj.try_iter()? {
        out.push(s?.extract::<[f64; 4]>()?);
    }
    Ok(out)
}

fn truthy<'py>(g: &Bound<'py, PyAny>, i: usize, len: usize) -> PyResult<Option<Bound<'py, PyAny>>> {
    if i >= len {
        return Ok(None);
    }
    let v = g.get_item(i)?;
    Ok(if v.is_truthy()? { Some(v) } else { None })
}

pub fn group_of(g: &Bound<'_, PyAny>) -> PyResult<Group> {
    let len = g.len()?;
    let name = match truthy(g, 0, len)? {
        Some(v) => v.str()?.to_string(),
        None => String::new(),
    };
    Ok(Group {
        name,
        segs: segs_of(&g.get_item(1)?)?,
        linetype: truthy(g, 2, len)?.map(|v| v.str().map(|s| s.to_string())).transpose()?,
        rgb: match (len > 3).then(|| g.get_item(3)).transpose()? {
            Some(v) if !v.is_none() => {
                let c: Vec<f64> = v.extract()?;
                (c.len() >= 3).then(|| [c[0], c[1], c[2]])
            }
            _ => None,
        },
        dashes: truthy(g, 4, len)?.map(|v| v.extract::<Vec<f64>>()).transpose()?,
        weight: truthy(g, 5, len)?.map(|v| v.extract::<f64>()).transpose()?,
    })
}

fn rect(v: Option<Bound<'_, PyAny>>) -> PyResult<Option<[f64; 4]>> {
    v.filter(|v| !v.is_none()).map(|v| v.extract::<[f64; 4]>()).transpose()
}

/// `formats.svg_out.svg_text` for the lines (see the module): ``groups``
/// as the DXF export takes them; ``window`` clips; ``fixed`` and ``extra``
/// are the picture's window and the faces' bounds; ``under(layout,
/// taken)`` returns the layers below the lines (``layout`` a dict with
/// ``min_x``, ``max_y``, ``k``, ``off_x``, ``off_y``, ``width``,
/// ``height``, ``clip``; ``taken`` the layer ids so far, a set it adds
/// to).
#[pyfunction]
#[pyo3(signature = (groups, scale_n=100.0, pens=None, line_colours=true, paper=None, window=None, margin=None, fixed=None, extra=None, under=None))]
#[allow(clippy::too_many_arguments)]
fn svg_lines(
    py: Python<'_>,
    groups: &Bound<'_, PyAny>,
    scale_n: f64,
    pens: Option<&Bound<'_, PyDict>>,
    line_colours: bool,
    paper: Option<(f64, f64)>,
    window: Option<Bound<'_, PyAny>>,
    margin: Option<f64>,
    fixed: Option<Bound<'_, PyAny>>,
    extra: Option<Bound<'_, PyAny>>,
    under: Option<Bound<'_, PyAny>>,
) -> PyResult<(String, usize)> {
    let mut gs = Vec::new();
    for g in groups.try_iter()? {
        gs.push(group_of(&g?)?);
    }
    let mut p = DEFAULT_PENS;
    if let Some(pens) = pens {
        for (i, key) in ["cut", "profile", "edge"].iter().enumerate() {
            if let Some(v) = pens.get_item(key)? {
                p[i] = v.extract()?;
            }
        }
    }
    let o = Options {
        scale_n, pens: p, line_colours, paper, window: rect(window)?, margin,
        fixed: rect(fixed)?, extra: rect(extra)?,
    };
    let mut err = None;
    let mut call = |l: &Layout, taken: &mut HashSet<String>| -> String {
        let Some(f) = &under else { return String::new() };
        let r = (|| -> PyResult<String> {
            let d = PyDict::new(py);
            d.set_item("min_x", l.min_x)?;
            d.set_item("max_y", l.max_y)?;
            d.set_item("k", l.k)?;
            d.set_item("off_x", l.off_x)?;
            d.set_item("off_y", l.off_y)?;
            d.set_item("width", l.width)?;
            d.set_item("height", l.height)?;
            d.set_item("clip", if o.window.is_some() { " clip-path=\"url(#pagina)\"" } else { "" })?;
            let ids = pyo3::types::PySet::new(py, taken.iter())?;
            let text: String = f.call1((d, &ids))?.extract()?;
            taken.clear();
            for id in ids.iter() {
                taken.insert(id.extract()?);
            }
            Ok(text)
        })();
        r.unwrap_or_else(|e| {
            err = Some(e);
            String::new()
        })
    };
    let (text, n) = svg_text(&gs, &o, Some(&mut call));
    if let Some(e) = err {
        return Err(e);
    }
    Ok((text, n))
}

/// `formats.svg_out.chain_segments`: ``[(points, closed)]``.
#[pyfunction]
#[pyo3(signature = (segments, tol=1e-6))]
fn chain_segments_py<'py>(py: Python<'py>, segments: &Bound<'_, PyAny>, tol: f64) -> PyResult<Bound<'py, PyList>> {
    let segs = segs_of(segments)?;
    let out = PyList::empty(py);
    for (pts, closed) in chain_segments(&segs, tol) {
        let pts: Vec<(f64, f64)> = pts.into_iter().map(|p| (p[0], p[1])).collect();
        out.append(PyTuple::new(py, [PyList::new(py, pts)?.into_any(), closed.into_pyobject(py)?.to_owned().into_any()])?)?;
    }
    Ok(out)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(svg_lines, m)?)?;
    m.add("chain_segments", wrap_pyfunction!(chain_segments_py, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn g_formats_as_python() {
        for (x, want) in [(0.18, "0.18"), (100.0, "100"), (1e6, "1e+06"), (0.0001, "0.0001"),
                          (0.00001, "1e-05"), (123456.0, "123456"), (1234567.0, "1.23457e+06"),
                          (0.5, "0.5"), (15.0, "15"), (2.0, "2")] {
            assert_eq!(fmt_g(x), want, "{x}");
        }
        assert_eq!(scale_label(100.0), "1:100");
        assert_eq!(scale_label(2.46), "1:2.5");
        assert_eq!(scale_label(12.5), "1:12");
    }

    #[test]
    fn ids_and_labels() {
        let mut t = HashSet::from(["pagina".to_string()]);
        assert_eq!(xml_id("Muros", &mut t), "Muros");
        assert_eq!(xml_id("Muros", &mut t), "Muros_2");
        assert_eq!(xml_id("1 nivel", &mut t), "_1_nivel");
        assert_eq!(xml_id("", &mut t), "capa");
        assert_eq!(quoteattr("a \"b\""), "'a \"b\"'");
        assert_eq!(quoteattr("a & <b>"), "\"a &amp; &lt;b&gt;\"");
    }

    #[test]
    fn a_square_closes_and_a_corner_ends_a_line() {
        let sq = [[0.0, 0.0, 1.0, 0.0], [1.0, 0.0, 1.0, 1.0], [1.0, 1.0, 0.0, 1.0], [0.0, 1.0, 0.0, 0.0]];
        let c = chain_segments(&sq, 1e-6);
        assert_eq!(c.len(), 4); // every corner is 90°: four lines
        let line = [[0.0, 0.0, 1.0, 0.0], [1.0, 0.0, 2.0, 0.01]];
        let c = chain_segments(&line, 1e-6);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].0.len(), 3);
    }
}
