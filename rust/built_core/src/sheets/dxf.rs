// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! A sheet view's lines as DXF: an ASCII DXF R12
//! with LINE, SOLID and TEXT entities, the layers with their line types and
//! the nearest standard colour, in model units. Ported from
//! formats/dxf_out.py `save_dxf_layers` byte for byte
//! (tests/test_sheet_dxf_parity.py).

use pyo3::prelude::*;
use std::fmt::Write as _;

/// The AutoCAD Color Index palette (ezdxf `DXF_DEFAULT_COLORS`).
const ACI: [u32; 256] = [0x000000, 0xff0000, 0xffff00, 0x00ff00, 0x00ffff, 0x0000ff, 0xff00ff, 0xffffff, 0x808080, 0xc0c0c0, 0xff0000, 0xff7f7f, 0xa50000, 0xa55252, 0x7f0000, 0x7f3f3f, 0x4c0000, 0x4c2626, 0x260000, 0x261313, 0xff3f00, 0xff9f7f, 0xa52900, 0xa56752, 0x7f1f00, 0x7f4f3f, 0x4c1300, 0x4c2f26, 0x260900, 0x261713, 0xff7f00, 0xffbf7f, 0xa55200, 0xa57c52, 0x7f3f00, 0x7f5f3f, 0x4c2600, 0x4c3926, 0x261300, 0x261c13, 0xffbf00, 0xffdf7f, 0xa57c00, 0xa59152, 0x7f5f00, 0x7f6f3f, 0x4c3900, 0x4c4226, 0x261c00, 0x262113, 0xffff00, 0xffff7f, 0xa5a500, 0xa5a552, 0x7f7f00, 0x7f7f3f, 0x4c4c00, 0x4c4c26, 0x262600, 0x262613, 0xbfff00, 0xdfff7f, 0x7ca500, 0x91a552, 0x5f7f00, 0x6f7f3f, 0x394c00, 0x424c26, 0x1c2600, 0x212613, 0x7fff00, 0xbfff7f, 0x52a500, 0x7ca552, 0x3f7f00, 0x5f7f3f, 0x264c00, 0x394c26, 0x132600, 0x1c2613, 0x3fff00, 0x9fff7f, 0x29a500, 0x67a552, 0x1f7f00, 0x4f7f3f, 0x134c00, 0x2f4c26, 0x092600, 0x172613, 0x00ff00, 0x7fff7f, 0x00a500, 0x52a552, 0x007f00, 0x3f7f3f, 0x004c00, 0x264c26, 0x002600, 0x132613, 0x00ff3f, 0x7fff9f, 0x00a529, 0x52a567, 0x007f1f, 0x3f7f4f, 0x004c13, 0x264c2f, 0x002609, 0x135817, 0x00ff7f, 0x7fffbf, 0x00a552, 0x52a57c, 0x007f3f, 0x3f7f5f, 0x004c26, 0x264c39, 0x002613, 0x13581c, 0x00ffbf, 0x7fffdf, 0x00a57c, 0x52a591, 0x007f5f, 0x3f7f6f, 0x004c39, 0x264c42, 0x00261c, 0x135858, 0x00ffff, 0x7fffff, 0x00a5a5, 0x52a5a5, 0x007f7f, 0x3f7f7f, 0x004c4c, 0x264c4c, 0x002626, 0x135858, 0x00bfff, 0x7fdfff, 0x007ca5, 0x5291a5, 0x005f7f, 0x3f6f7f, 0x00394c, 0x26427e, 0x001c26, 0x135858, 0x007fff, 0x7fbfff, 0x0052a5, 0x527ca5, 0x003f7f, 0x3f5f7f, 0x00264c, 0x26397e, 0x001326, 0x131c58, 0x003fff, 0x7f9fff, 0x0029a5, 0x5267a5, 0x001f7f, 0x3f4f7f, 0x00134c, 0x262f7e, 0x000926, 0x131758, 0x0000ff, 0x7f7fff, 0x0000a5, 0x5252a5, 0x00007f, 0x3f3f7f, 0x00004c, 0x26267e, 0x000026, 0x131358, 0x3f00ff, 0x9f7fff, 0x2900a5, 0x6752a5, 0x1f007f, 0x4f3f7f, 0x13004c, 0x2f267e, 0x090026, 0x171358, 0x7f00ff, 0xbf7fff, 0x5200a5, 0x7c52a5, 0x3f007f, 0x5f3f7f, 0x26004c, 0x39267e, 0x130026, 0x1c1358, 0xbf00ff, 0xdf7fff, 0x7c00a5, 0x9152a5, 0x5f007f, 0x6f3f7f, 0x39004c, 0x42264c, 0x1c0026, 0x581358, 0xff00ff, 0xff7fff, 0xa500a5, 0xa552a5, 0x7f007f, 0x7f3f7f, 0x4c004c, 0x4c264c, 0x260026, 0x581358, 0xff00bf, 0xff7fdf, 0xa5007c, 0xa55291, 0x7f005f, 0x7f3f6f, 0x4c0039, 0x4c2642, 0x26001c, 0x581358, 0xff007f, 0xff7fbf, 0xa50052, 0xa5527c, 0x7f003f, 0x7f3f5f, 0x4c0026, 0x4c2639, 0x260013, 0x58131c, 0xff003f, 0xff7f9f, 0xa50029, 0xa55267, 0x7f001f, 0x7f3f4f, 0x4c0013, 0x4c262f, 0x260009, 0x581317, 0x000000, 0x656565, 0x666666, 0x999999, 0xcccccc, 0xffffff];

/// Line types a view export may declare: description and pattern in
/// drawing units (dash > 0, gap < 0, dot 0).
const LTYPES: [(&str, &str, &[f64]); 9] = [
    ("DASHED", "__ __ __ __", &[0.5, -0.25]),
    ("HIDDEN", "_ _ _ _ _ _", &[0.25, -0.125]),
    ("DASHEDX2", "____  ____  ____", &[1.0, -0.5]),
    ("DOT", ". . . . . .", &[0.0, -0.25]),
    ("DASHDOT", "__ . __ . __", &[0.5, -0.25, 0.0, -0.25]),
    ("DIVIDE", "__ . . __ . . __", &[0.5, -0.25, 0.0, -0.25, 0.0, -0.25]),
    ("CENTER", "____ _ ____ _ ____", &[1.25, -0.25, 0.25, -0.25]),
    ("PHANTOM", "____ _ _ ____ _ _", &[1.25, -0.25, 0.25, -0.25, 0.25, -0.25]),
    ("BORDER", "__ __ . __ __ .", &[0.5, -0.25, 0.5, -0.25, 0.0, -0.25]),
];

/// Python's `format(x, ".{p}g")`.
pub fn fmt_gp(x: f64, p: usize) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if x == 0.0 {
        return if x.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    let p = p.max(1);
    let sci = format!("{:.*e}", p - 1, x);
    let (mant, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let strip = |s: &str| -> String {
        if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s.to_string() }
    };
    if exp >= -4 && exp < p as i32 {
        strip(&format!("{:.*}", (p as i32 - 1 - exp) as usize, x))
    } else {
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{}e{}{:02}", strip(mant), sign, exp.abs())
    }
}

fn layer_name(name: &str) -> String {
    let n = name.trim().to_uppercase().replace(' ', "_");
    if n.is_empty() { "VISTA".into() } else { n }
}

/// `_nearest_aci`: the standard colour (1–255, 7 left out) closest to `rgb`.
pub fn nearest_aci(rgb: [f64; 3]) -> u8 {
    let (r, g, b) = (rgb[0] * 255.0, rgb[1] * 255.0, rgb[2] * 255.0);
    let (mut best, mut best_d) = (7u8, f64::INFINITY);
    for (aci, &c) in ACI.iter().enumerate().skip(1) {
        if aci == 7 {
            continue;
        }
        let (cr, cg, cb) = (f64::from((c >> 16) & 255), f64::from((c >> 8) & 255), f64::from(c & 255));
        let d = (cr - r).powi(2) + (cg - g).powi(2) + (cb - b).powi(2);
        if d < best_d {
            best = aci as u8;
            best_d = d;
        }
    }
    best
}

/// A group of lines: layer, segments, line type, colour, own pattern.
pub struct Group {
    pub layer: String,
    pub segs: Vec<[f64; 4]>,
    pub ltype: Option<String>,
    pub rgb: Option<[f64; 3]>,
    pub pattern: Option<Vec<f64>>,
}

pub struct Solids {
    pub layer: String,
    pub quads: Vec<[[f64; 2]; 4]>,
    pub rgb: Option<[f64; 3]>,
}

pub struct Text {
    pub layer: String,
    pub x: f64,
    pub y: f64,
    pub height: f64,
    pub angle: f64,
    pub text: String,
}

/// The file's text and the number of lines written.
pub fn write(groups: &[Group], solids: &[Solids], texts: &[Text]) -> (String, usize) {
    let mut names: Vec<(String, String)> = Vec::new();
    let mut colors: Vec<(String, u8)> = Vec::new();
    let mut own: Vec<(String, String, Vec<f64>)> = Vec::new();
    let set = |v: &mut Vec<(String, String)>, k: String, val: String| {
        if !v.iter().any(|(n, _)| *n == k) {
            v.push((k, val));
        }
    };
    let set_color = |v: &mut Vec<(String, u8)>, k: String, c: u8| {
        if !v.iter().any(|(n, _)| *n == k) {
            v.push((k, c));
        }
    };
    for s in solids {
        let name = layer_name(&s.layer);
        set(&mut names, name.clone(), "CONTINUOUS".into());
        if let Some(rgb) = s.rgb {
            set_color(&mut colors, name, nearest_aci(rgb));
        }
    }
    for g in groups {
        if let (Some(lt), Some(p)) = (g.ltype.as_ref().filter(|l| !l.is_empty()), g.pattern.as_ref().filter(|p| !p.is_empty())) {
            if !own.iter().any(|(n, _, _)| n == lt) {
                let desc: String = p.iter().map(|v| if *v > 0.0 { '_' } else if *v < 0.0 { ' ' } else { '.' }).collect();
                own.push((lt.clone(), desc, p.clone()));
            }
        }
        let name = layer_name(&g.layer);
        let ltype = g.ltype.clone().filter(|l| !l.is_empty()).unwrap_or_else(|| "CONTINUOUS".into());
        set(&mut names, name.clone(), ltype);
        if let Some(rgb) = g.rgb {
            set_color(&mut colors, name, nearest_aci(rgb));
        }
    }
    // The built-in types (an own one of the same name replaces it in place),
    // then the document's own.
    let mut table: Vec<(String, String, Vec<f64>)> =
        LTYPES.iter().map(|(n, d, p)| (n.to_string(), d.to_string(), p.to_vec())).collect();
    for (n, d, p) in own {
        match table.iter_mut().find(|t| t.0 == n) {
            Some(t) => *t = (n, d, p),
            None => table.push((n, d, p)),
        }
    }
    let used: Vec<&(String, String, Vec<f64>)> = table.iter().filter(|t| names.iter().any(|(_, lt)| *lt == t.0)).collect();
    let g6 = |v: f64| fmt_gp(v, 6);
    let g9 = |v: f64| fmt_gp(v, 9);
    let mut w = String::new();
    w += "0\nSECTION\n2\nTABLES\n";
    if !used.is_empty() {
        let _ = write!(w, "0\nTABLE\n2\nLTYPE\n70\n{}\n", used.len());
        for (lt, desc, pattern) in &used {
            let total: f64 = pattern.iter().map(|d| d.abs()).sum();
            let _ = write!(w, "0\nLTYPE\n2\n{lt}\n70\n0\n3\n{desc}\n72\n65\n73\n{}\n40\n{}\n", pattern.len(), g6(total));
            for d in pattern {
                let _ = write!(w, "49\n{}\n", g6(*d));
            }
        }
        w += "0\nENDTAB\n";
    }
    let _ = write!(w, "0\nTABLE\n2\nLAYER\n70\n{}\n", names.len());
    for (name, ltype) in &names {
        let c = colors.iter().find(|(n, _)| n == name).map_or(7, |(_, c)| *c);
        let _ = write!(w, "0\nLAYER\n2\n{name}\n70\n0\n62\n{c}\n6\n{ltype}\n");
    }
    w += "0\nENDTAB\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n";
    for s in solids {
        let name = layer_name(&s.layer);
        for [a, b, c, d] in &s.quads {
            let _ = write!(w, "0\nSOLID\n8\n{name}\n10\n{}\n20\n{}\n30\n0\n11\n{}\n21\n{}\n31\n0\n12\n{}\n22\n{}\n32\n0\n13\n{}\n23\n{}\n33\n0\n",
                           g9(a[0]), g9(a[1]), g9(b[0]), g9(b[1]), g9(c[0]), g9(c[1]), g9(d[0]), g9(d[1]));
        }
    }
    for t in texts {
        let name = layer_name(&t.layer);
        let words = t.text.replace('\n', " ");
        let (x, y) = (g9(t.x), g9(t.y));
        let _ = write!(w, "0\nTEXT\n8\n{name}\n10\n{x}\n20\n{y}\n30\n0\n40\n{}\n1\n{words}\n50\n{}\n72\n1\n11\n{x}\n21\n{y}\n31\n0\n73\n2\n",
                       g9(t.height), fmt_gp(t.angle, 6));
    }
    let mut n = 0;
    for g in groups {
        let name = layer_name(&g.layer);
        for s in &g.segs {
            let _ = write!(w, "0\nLINE\n8\n{name}\n10\n{}\n20\n{}\n30\n0\n11\n{}\n21\n{}\n31\n0\n",
                           g9(s[0]), g9(s[1]), g9(s[2]), g9(s[3]));
            n += 1;
        }
    }
    w += "0\nENDSEC\n0\nEOF\n";
    // Written as ASCII, a character it lacks as "?" (Python's errors="replace").
    let w: String = w.chars().map(|c| if c.is_ascii() { c } else { '?' }).collect();
    (w, n)
}

fn rgb_of(v: Option<Bound<'_, PyAny>>) -> PyResult<Option<[f64; 3]>> {
    match v.filter(|v| !v.is_none()) {
        None => Ok(None),
        Some(v) => {
            let c: Vec<f64> = v.extract()?;
            Ok(Some([c[0], c[1], c[2]]))
        }
    }
}

fn rows4(v: &Bound<'_, PyAny>) -> PyResult<Vec<[f64; 4]>> {
    let rows: Vec<Vec<f64>> = match v.call_method0("tolist") {
        Ok(l) => l.extract()?,
        Err(_) => v.try_iter()?.map(|r| r?.extract()).collect::<PyResult<_>>()?,
    };
    Ok(rows.into_iter().map(|r| [r[0], r[1], r[2], r[3]]).collect())
}

/// The line types every file can declare, by name.
#[pyfunction]
pub fn sheet_dxf_ltypes() -> Vec<&'static str> {
    LTYPES.iter().map(|t| t.0).collect()
}

/// ``formats.dxf_out.save_dxf_layers``: ``groups`` ``(layer, segments[,
/// ltype[, rgb[, pattern]]])``, ``solids`` ``(layer, quads, rgb)``,
/// ``texts`` ``(layer, x, y, height, degrees, text)``. Returns the lines.
#[pyfunction]
#[pyo3(signature = (path, groups, solids=None, texts=None))]
pub fn sheet_dxf(path: std::path::PathBuf, groups: Vec<Bound<'_, PyAny>>, solids: Option<Vec<Bound<'_, PyAny>>>,
                 texts: Option<Vec<Bound<'_, PyAny>>>) -> PyResult<usize> {
    let mut gs = Vec::new();
    for g in groups {
        let n = g.len()?;
        let item = |i: usize| -> PyResult<Option<Bound<'_, PyAny>>> {
            if i < n { Ok(Some(g.get_item(i)?).filter(|v| !v.is_none())) } else { Ok(None) }
        };
        let layer = item(0)?.map(|v| v.extract::<String>()).transpose()?.unwrap_or_default();
        let segs = match item(1)? { Some(s) => rows4(&s)?, None => Vec::new() };
        let ltype = match item(2)? {
            Some(v) if v.is_truthy()? => Some(v.extract::<String>()?),
            _ => None,
        };
        let pattern = match item(4)? {
            Some(v) if v.is_truthy()? => Some(v.extract::<Vec<f64>>()?),
            _ => None,
        };
        gs.push(Group { layer, segs, ltype, rgb: rgb_of(item(3)?)?, pattern });
    }
    let mut ss = Vec::new();
    for s in solids.unwrap_or_default() {
        let layer: String = s.get_item(0)?.extract().unwrap_or_default();
        let quads: Vec<[[f64; 2]; 4]> = s.get_item(1)?.try_iter()?.map(|q| {
            let q: Vec<(f64, f64)> = q?.extract()?;
            Ok([[q[0].0, q[0].1], [q[1].0, q[1].1], [q[2].0, q[2].1], [q[3].0, q[3].1]])
        }).collect::<PyResult<_>>()?;
        ss.push(Solids { layer, quads, rgb: rgb_of(Some(s.get_item(2)?))? });
    }
    let mut ts = Vec::new();
    for t in texts.unwrap_or_default() {
        let (layer, x, y, height, angle, text): (String, f64, f64, f64, f64, Bound<'_, PyAny>) = t.extract()?;
        ts.push(Text { layer, x, y, height, angle, text: text.str()?.to_string() });
    }
    let (text, n) = write(&gs, &ss, &ts);
    std::fs::write(&path, text).map_err(|e| pyo3::exceptions::PyOSError::new_err(format!("{}: {e}", path.display())))?;
    Ok(n)
}
