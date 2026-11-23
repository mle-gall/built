// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! One item's display list: its fields from the document, the few strings
//! that still come from Python (`items::Extra`), its painter.

use super::items::{self, Extra};
use super::model::{Fields, ItemKind};
use super::paint::{ops_json, Op, Painter};
use crate::doc::value::Value;
use pyo3::prelude::*;
use pyo3::types::PyDict;

/// `core.i18n.tr(text, **kw)`.
fn tr(py: Python<'_>, text: &str, kw: &[(&str, String)]) -> PyResult<String> {
    let d = PyDict::new(py);
    for (k, v) in kw {
        d.set_item(k, v)?;
    }
    py.import("core.i18n")?.getattr("tr")?.call((text,), Some(&d))?.extract()
}

fn swatch_pattern(py: Python<'_>, f: &Fields, name: &str) -> PyResult<Vec<f64>> {
    let Some(Value::Dict(d)) = f.0.get("swatches") else { return Ok(Vec::new()) };
    let Some((_, Value::Dict(sw))) = d.iter().find(|(k, _)| matches!(k, Value::Str(s) if s == name)) else {
        return Ok(Vec::new());
    };
    let get = |key: &str| sw.iter().find(|(k, _)| matches!(k, Value::Str(s) if s == key)).map(|(_, v)| v.clone());
    let nums = |v: &Value| -> Vec<f64> {
        match v {
            Value::List(l) | Value::Tuple(l) => l.iter().filter_map(|x| match x {
                Value::Float(f) => Some(*f),
                Value::Int(i) => Some(*i as f64),
                _ => None,
            }).collect(),
            _ => Vec::new(),
        }
    };
    if let Some(p) = get("pattern").map(|v| nums(&v)).filter(|p| !p.is_empty()) {
        return Ok(p);
    }
    let Some(Value::Str(lt)) = get("linetype") else { return Ok(Vec::new()) };
    let table = py.import("core.layers")?.getattr("LINETYPES")?;
    match table.call_method1("get", (lt,))? {
        v if v.is_none() => Ok(Vec::new()),
        v => v.extract(),
    }
}

/// What the item's painter takes from Python.
pub fn extra(py: Python<'_>, handle: &Bound<'_, PyAny>, kind: ItemKind, f: &Fields) -> PyResult<Extra> {
    let mut x = Extra::default();
    let mut put = |k: String, v: Value| {
        x.0.insert(k, v);
    };
    match kind {
        ItemKind::ScaleBar => {
            let n = super::svg::fmt_g(f.num("scale_n", 100.0));
            put("caption".into(), Value::Str(tr(py, "metres — scale 1:{n}", &[("n", n)])?));
        }
        ItemKind::Legend => {
            put("empty".into(), Value::Str(tr(py, "(no layers)", &[])?));
            let rows: Vec<String> = f.list("rows").iter().filter_map(|v| match v {
                Value::Str(s) => Some(s.clone()),
                _ => None,
            }).collect();
            for (i, name) in rows.iter().enumerate() {
                let p = swatch_pattern(py, f, name)?;
                put(format!("pattern.{i}"), Value::List(p.into_iter().map(Value::Float).collect()));
            }
        }
        ItemKind::Level => {
            put("label".into(), Value::Str(handle.call_method0("label")?.extract()?));
        }
        ItemKind::Callout => {
            let expand = py.import("core.composition")?.getattr("expand_fields")?;
            let uid = f.text("frame_uid", "");
            for k in ["number", "sheet"] {
                let s: String = expand.call1((f.text(k, ""), uid.clone()))?.extract()?;
                put(k.into(), Value::Str(s));
            }
        }
        _ => {}
    }
    Ok(x)
}

/// The item's primitives (item-local paper mm), or None for the kinds the
/// page still draws its own way (frames, texts, title blocks) or not yet.
pub fn paint(kind: ItemKind, f: &Fields, x: &Extra) -> Option<Vec<Op>> {
    let mut p = Painter::default();
    match kind {
        ItemKind::ScaleBar => items::paint_scalebar(&mut p, f, x),
        ItemKind::North => items::paint_north(&mut p, f),
        ItemKind::Legend => items::paint_legend(&mut p, f, x),
        ItemKind::Shape => items::paint_shape(&mut p, f),
        ItemKind::Callout => items::paint_callout(&mut p, f, x),
        ItemKind::Level => items::paint_level(&mut p, f, x),
        ItemKind::Image => {
            let path = f.text("path", "");
            items::paint_image(&mut p, f, !path.is_empty() && std::path::Path::new(&path).is_file());
        }
        _ => return None,
    }
    Some(p.ops)
}

/// The primitives of sheet item ``item`` (a record of core.composition), as
/// tests/sheet_paint_recorder.py records the Python painter's: a JSON list.
#[pyfunction]
pub fn sheet_item_display(py: Python<'_>, item: &Bound<'_, PyAny>) -> PyResult<String> {
    py.import("core.doc_model")?.call_method0("flush")?;
    let class = item.get_type().name()?.to_string();
    let kind = ItemKind::of_class(&class)
        .ok_or_else(|| pyo3::exceptions::PyValueError::new_err(format!("not a sheet item: {class}")))?;
    let key = crate::doc::py::handle_key(item)?;
    let f = super::model::read_fields(key)?;
    let x = extra(py, item, kind, &f)?;
    let ops = paint(kind, &f, &x).unwrap_or_default();
    Ok(ops_json(&ops).to_string())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(sheet_item_display, m)?)?;
    Ok(())
}
