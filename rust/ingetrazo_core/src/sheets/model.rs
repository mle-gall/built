// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! Typed views of the sheet records (docs/rust-sheets.md §1).
//!
//! The records live in the document arena (`DocRecord` + `DocAttr`,
//! `core/composition.py` declares their fields); this reads them as Rust
//! values. Paper millimetres, origin top-left, y down; model metres.

use crate::doc::{self, value::Value, Key};
use pyo3::prelude::*;
use std::collections::HashMap;

/// core.composition.PAPER_SIZES_MM (portrait width, height).
pub const PAPER_SIZES_MM: [(&str, f64, f64); 5] = [
    ("A4", 210.0, 297.0),
    ("A3", 297.0, 420.0),
    ("A2", 420.0, 594.0),
    ("A1", 594.0, 841.0),
    ("A0", 841.0, 1189.0),
];

/// The page in mm, as `Composicion.page_size_mm`.
pub fn page_size_mm(paper: &str, landscape: bool) -> (f64, f64) {
    let (w, h) = PAPER_SIZES_MM.iter().find(|p| p.0 == paper).map(|p| (p.1, p.2)).unwrap_or((210.0, 297.0));
    if landscape { (h, w) } else { (w, h) }
}

/// core.composition.format_scale: `1:50`, `10:1`, `1:1`.
pub fn format_scale(n: f64) -> String {
    if !(n > 0.0) {
        return String::new();
    }
    if n < 1.0 {
        format!("{}:1", super::svg::fmt_g(1.0 / n))
    } else {
        format!("1:{}", super::svg::fmt_g(n))
    }
}

/// Metres of model a frame `h_mm` tall shows at 1:`n`.
pub fn model_height_for_frame(h_mm: f64, n: f64) -> f64 {
    h_mm * n / 1000.0
}

/// Camera-plane metres (x right, y up, origin at the view's centre) to
/// frame-local paper mm, for a parallel frame (`frame_page_projector`).
pub fn frame_projector(w_mm: f64, h_mm: f64, n: f64) -> impl Fn(f64, f64) -> (f64, f64) {
    let half_h = model_height_for_frame(h_mm, n) / 2.0;
    let half_w = half_h * if h_mm != 0.0 { w_mm / h_mm } else { 1.0 };
    let k = if half_h != 0.0 { h_mm / (2.0 * half_h) } else { 0.0 };
    move |cx, cy| ((cx + half_w) * k, (half_h - cy) * k)
}

/// One record's fields, read out of the arena.
#[derive(Clone, Debug, Default)]
pub struct Fields(pub HashMap<String, Value>);

impl Fields {
    pub fn num(&self, name: &str, default: f64) -> f64 {
        match self.0.get(name) {
            Some(Value::Float(f)) => *f,
            Some(Value::Int(i)) => *i as f64,
            Some(Value::Bool(b)) => f64::from(u8::from(*b)),
            _ => default,
        }
    }
    pub fn opt_num(&self, name: &str) -> Option<f64> {
        match self.0.get(name) {
            Some(Value::Float(f)) => Some(*f),
            Some(Value::Int(i)) => Some(*i as f64),
            _ => None,
        }
    }
    pub fn flag(&self, name: &str, default: bool) -> bool {
        match self.0.get(name) {
            Some(Value::Bool(b)) => *b,
            Some(Value::Int(i)) => *i != 0,
            Some(Value::None) | None => default,
            Some(_) => default,
        }
    }
    pub fn text(&self, name: &str, default: &str) -> String {
        match self.0.get(name) {
            Some(Value::Str(s)) => s.clone(),
            _ => default.to_string(),
        }
    }
    pub fn list(&self, name: &str) -> &[Value] {
        match self.0.get(name) {
            Some(Value::List(v)) | Some(Value::Tuple(v)) => v,
            _ => &[],
        }
    }
}

/// The record kinds a sheet holds, by their `core.composition` class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Frame,
    Text,
    TitleBlock,
    Image,
    ScaleBar,
    North,
    Legend,
    Shape,
    Dimension,
    AngularDimension,
    RadialDimension,
    Label,
    Profile,
    Level,
    Callout,
}

impl ItemKind {
    /// The kind of a `core.composition` record class, and the sheet list
    /// that holds it (`Composicion.all_items` order).
    pub const LISTS: [(&'static str, ItemKind); 14] = [
        ("frames", ItemKind::Frame),
        ("texts", ItemKind::Text),
        ("images", ItemKind::Image),
        ("scalebars", ItemKind::ScaleBar),
        ("nortes", ItemKind::North),
        ("leyendas", ItemKind::Legend),
        ("shapes", ItemKind::Shape),
        ("cotas", ItemKind::Dimension),
        ("cotas_ang", ItemKind::AngularDimension),
        ("cotas_rad", ItemKind::RadialDimension),
        ("etiquetas", ItemKind::Label),
        ("perfiles", ItemKind::Profile),
        ("niveles", ItemKind::Level),
        ("llamadas", ItemKind::Callout),
    ];

    pub fn name(self) -> &'static str {
        match self {
            ItemKind::Frame => "frame",
            ItemKind::Text => "text",
            ItemKind::TitleBlock => "titleblock",
            ItemKind::Image => "image",
            ItemKind::ScaleBar => "scalebar",
            ItemKind::North => "north",
            ItemKind::Legend => "legend",
            ItemKind::Shape => "shape",
            ItemKind::Dimension => "dimension",
            ItemKind::AngularDimension => "angular",
            ItemKind::RadialDimension => "radial",
            ItemKind::Label => "label",
            ItemKind::Profile => "profile",
            ItemKind::Level => "level",
            ItemKind::Callout => "callout",
        }
    }

    /// Whether the native page moves and resizes it today (the rest keep
    /// their own geometry rules in the classic editor).
    pub fn native_edit(self) -> bool {
        matches!(self, ItemKind::Frame | ItemKind::Text | ItemKind::TitleBlock | ItemKind::Image)
    }
}

#[derive(Clone, Debug)]
pub struct Item {
    pub key: Key,
    pub kind: ItemKind,
    /// x, y, w, h in paper mm.
    pub rect: [f64; 4],
    pub z: f64,
    pub locked: bool,
    pub hidden: bool,
    pub fields: Fields,
}

#[derive(Clone, Debug)]
pub struct Sheet {
    pub key: Key,
    pub fields: Fields,
    pub page: (f64, f64),
    pub margin: f64,
    /// Every item, `Composicion.all_items` order, then stacked by z.
    pub items: Vec<Item>,
}

fn read_fields(key: Key) -> PyResult<Fields> {
    let d = doc::lock();
    let node = d.need(key).map_err(pyo3::exceptions::PyRuntimeError::new_err)?;
    Ok(Fields(node.attrs.iter().map(|(a, v)| (doc::attr_name(*a).to_string(), v.clone())).collect()))
}

fn handle_of<'py>(py: Python<'py>, v: &Value) -> Option<Bound<'py, PyAny>> {
    match v {
        Value::Opaque(o) => Some(o.0.clone_ref(py).into_bound(py)),
        _ => None,
    }
}

/// The size of a record whose class computes it (a scale bar's from its
/// segments, a legend's from its rows): asked of the Python handle until
/// those kinds are drawn natively (docs/python-glue.md).
fn computed(handle: &Bound<'_, PyAny>, name: &str) -> Option<f64> {
    handle.getattr(name).ok().and_then(|v| v.extract::<f64>().ok())
}

fn read_item(handle: &Bound<'_, PyAny>, kind: ItemKind) -> PyResult<Item> {
    let key = doc::py::handle_key(handle)?;
    let f = read_fields(key)?;
    let x = f.num("x_mm", 0.0);
    let y = f.num("y_mm", 0.0);
    let w = f.opt_num("w_mm").or_else(|| computed(handle, "w_mm")).unwrap_or(20.0);
    let h = match kind {
        // Text grows with its lines; the page lays it out (ingetrazo_render::page).
        ItemKind::Text => {
            let lines = f.text("text", "").lines().count().max(1) as f64;
            lines * f.num("size_pt", 14.0) * 25.4 / 72.0 * 1.25
        }
        _ => f.opt_num("h_mm").or_else(|| computed(handle, "h_mm")).unwrap_or(10.0),
    };
    Ok(Item {
        key,
        kind,
        rect: [x, y, w, h],
        z: f.num("z", 0.0),
        locked: f.flag("locked", false),
        hidden: f.flag("hidden", false),
        fields: f,
    })
}

impl Sheet {
    /// The sheet record `comp` (a `Composicion`), read from the document.
    pub fn read(comp: &Bound<'_, PyAny>) -> PyResult<Sheet> {
        let py = comp.py();
        // In-place list edits (`comp.frames.append(f)`) reach the arena on flush.
        py.import("core.doc_model")?.call_method0("flush")?;
        let key = doc::py::handle_key(comp)?;
        let fields = read_fields(key)?;
        let mut items = Vec::new();
        for (list, kind) in ItemKind::LISTS {
            for v in fields.list(list) {
                if let Some(h) = handle_of(py, v) {
                    items.push(read_item(&h, kind)?);
                }
            }
        }
        if let Some(h) = fields.0.get("cajetin").and_then(|v| handle_of(py, v)) {
            items.push(read_item(&h, ItemKind::TitleBlock)?);
        }
        // Stable: equal z keeps the list order, as the canvas stacks them.
        items.sort_by(|a, b| a.z.partial_cmp(&b.z).unwrap_or(std::cmp::Ordering::Equal));
        let page = page_size_mm(&fields.text("paper", "A4"), fields.flag("landscape", true));
        Ok(Sheet { key, margin: fields.num("margin_mm", 10.0), page, fields, items })
    }
}

/// The page size of a sheet record and its margin: ``(w, h, margin)``.
#[pyfunction]
fn sheet_page_size(comp: &Bound<'_, PyAny>) -> PyResult<(f64, f64, f64)> {
    let s = Sheet::read(comp)?;
    Ok((s.page.0, s.page.1, s.margin))
}

/// core.composition.format_scale in Rust (the native page's labels).
#[pyfunction]
#[pyo3(name = "sheet_format_scale")]
fn format_scale_py(n: f64) -> String {
    format_scale(n)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(sheet_page_size, m)?)?;
    m.add_function(wrap_pyfunction!(format_scale_py, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paper_and_scale() {
        assert_eq!(page_size_mm("A3", true), (420.0, 297.0));
        assert_eq!(page_size_mm("A4", false), (210.0, 297.0));
        assert_eq!(format_scale(100.0), "1:100");
        assert_eq!(format_scale(0.1), "10:1");
        assert_eq!(format_scale(1.0), "1:1");
        assert_eq!(format_scale(0.0), "");
        let p = frame_projector(200.0, 100.0, 100.0);
        // 100 mm at 1:100 shows 10 m: the centre lands mid-frame.
        assert_eq!(p(0.0, 0.0), (100.0, 50.0));
        assert_eq!(p(-10.0, 5.0), (0.0, 0.0));
    }
}
