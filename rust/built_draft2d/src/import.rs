// SPDX-License-Identifier: GPL-3.0-or-later
//! Importing a drawing into a new sketch: read the file, keep the chosen
//! layers, convert to millimetres, list what needs cleaning.
use crate::cleanup::{issues, Issue};
use crate::geom::Shape;
use crate::sketch::{Sketch, Use};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize)]
pub struct Layer {
    pub name: String,
    pub shapes: Vec<Shape>,
}

/// What a reader found, in drawing units.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Drawing {
    pub layers: Vec<Layer>,
    /// The declared unit name, when the file states one.
    pub unit: Option<String>,
    /// Millimetres per drawing unit, when known.
    pub to_mm: Option<f64>,
    /// Entities not read, by kind.
    pub skipped: BTreeMap<String, usize>,
}

/// A sketch made from a drawing, with its layer list and problems.
#[derive(Clone, Debug, Serialize)]
pub struct Imported {
    pub sketch: Sketch,
    /// (name, entity count, kept).
    pub layers: Vec<(String, usize, bool)>,
    pub unit: Option<String>,
    pub issues: Vec<Issue>,
    pub skipped: BTreeMap<String, usize>,
}

/// Layers that look like annotation, kept off by default.
fn annotation(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["dim", "text", "note", "anno", "hatch", "title", "border", "frame", "centre", "center", "axis", "defpoints"].iter().any(|k| n.contains(k))
}

pub fn read(path_ext: &str, bytes: &[u8]) -> Result<Drawing, String> {
    let text = String::from_utf8_lossy(bytes);
    match path_ext.to_ascii_lowercase().as_str() {
        "dxf" => {
            if bytes.starts_with(b"AutoCAD Binary DXF") {
                return Err("binary DXF is not read: save it as ASCII DXF".into());
            }
            crate::dxf::read(&text)
        }
        "svg" => crate::svg::read(&text),
        other => Err(format!("{other} files are not drawings this can read")),
    }
}

/// Build the sketch from the kept layers (None keeps every layer that is
/// not annotation, or all when that leaves nothing).
pub fn sketch(d: &Drawing, keep: Option<&[String]>, usage: Use, tol: f64) -> Imported {
    let default_keep: Vec<String> = {
        let k: Vec<String> = d.layers.iter().filter(|l| !annotation(&l.name)).map(|l| l.name.clone()).collect();
        if k.is_empty() {
            d.layers.iter().map(|l| l.name.clone()).collect()
        } else {
            k
        }
    };
    let keep: Vec<String> = keep.map(<[String]>::to_vec).unwrap_or(default_keep);
    let mut s = Sketch::new(usage);
    let k = d.to_mm.unwrap_or(1.0);
    for l in &d.layers {
        if keep.contains(&l.name) {
            for sh in &l.shapes {
                s.add(sh.scaled(glam::DVec2::ZERO, k));
            }
        }
    }
    // Put the drawing's lower-left at the origin: profiles are drawn about it.
    if let Some((lo, _)) = s.bounds() {
        s.translate(-lo);
    }
    let unit = if d.to_mm.is_some() { d.unit.clone() } else { None };
    let list = issues(&s, tol, unit.as_deref().or(if d.to_mm.is_some() { Some("mm") } else { None }));
    Imported {
        sketch: s,
        layers: d.layers.iter().map(|l| (l.name.clone(), l.shapes.len(), keep.contains(&l.name))).collect(),
        unit,
        issues: list,
        skipped: d.skipped.clone(),
    }
}
