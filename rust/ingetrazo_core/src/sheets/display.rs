// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! One item's display list: its fields from the document, the few strings
//! that still come from Python (`items::Extra`), its painter.

use super::items::{self, Extra};
use super::items_b;
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

/// A terrain profile's samples, as ``ComposerWindow.profile_for`` gives
/// them: ``(profile, path_name, message)``.
fn profile_extra(py: Python<'_>, f: &Fields, given: Option<&Bound<'_, PyAny>>, x: &mut Extra) -> PyResult<()> {
    let (profile, name, message) = match given {
        Some(t) if !t.is_none() => {
            let (a, b, c): (Bound<'_, PyAny>, Option<String>, Option<String>) = t.extract()?;
            (Some(a).filter(|a| !a.is_none()), b.unwrap_or_default(), c.unwrap_or_default())
        }
        _ => (None, String::new(), String::new()),
    };
    let title = match f.text("title", "") {
        t if !t.is_empty() => t,
        _ => {
            let n = if name.is_empty() { tr(py, "path", &[])? } else { name.clone() };
            tr(py, "Longitudinal profile — {name}", &[("name", n)])?
        }
    };
    x.0.insert("title".into(), Value::Str(title));
    let waiting = if message.is_empty() { tr(py, "Loading terrain…", &[])? } else { message.clone() };
    x.0.insert("waiting".into(), Value::Str(waiting));
    x.0.insert("message".into(), Value::Str(message));
    x.0.insert("caption".into(), Value::Str(tr(py, "Scale H 1:{h} · V 1:{v} · vert. exag. ×{k}", &[])?));
    x.0.insert("fitted".into(), Value::Str(tr(py, "(fitted to the width)", &[])?));
    if let Some(pr) = profile {
        // sheets::profile's sampling: {"samples": [(station, x, y, elevation)], "length"}.
        let rows: Vec<(f64, f64, f64, Option<f64>)> = pr.get_item("samples")?.extract()?;
        let samples = rows.into_iter()
            .map(|(st, _, _, el)| Value::Tuple(vec![Value::Float(st), el.map_or(Value::None, Value::Float)]))
            .collect();
        x.0.insert("samples".into(), Value::List(samples));
        x.0.insert("length".into(), Value::Float(pr.get_item("length")?.extract()?));
    }
    Ok(())
}

/// What the item's painter takes from Python.
pub fn extra(py: Python<'_>, handle: &Bound<'_, PyAny>, kind: ItemKind, f: &Fields,
             profile: Option<&Bound<'_, PyAny>>) -> PyResult<Extra> {
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
        ItemKind::Level | ItemKind::Dimension | ItemKind::AngularDimension | ItemKind::RadialDimension => {
            put("label".into(), Value::Str(handle.call_method0("label")?.extract()?));
        }
        ItemKind::Label => {
            let expand = py.import("core.composition")?.getattr("expand_fields")?;
            put("text".into(), Value::Str(expand.call1((f.text("text", ""),))?.extract()?));
        }
        ItemKind::Profile => {
            profile_extra(py, f, profile, &mut x)?;
        }
        ItemKind::Text => {
            let expand = py.import("core.composition")?.getattr("expand_fields")?;
            let s: String = expand.call1((f.text("text", ""), f.text("frame_uid", "")))?.extract()?;
            put("text".into(), Value::Str(s));
        }
        ItemKind::TitleBlock => {
            // The rows (its own, else the class's labelled fields), expanded;
            // an empty scale row reads the sheet's main frame.
            let expand = py.import("core.composition")?.getattr("expand_fields")?;
            // `str(value or "")`, as Python reads them.
            let as_str = |v: Bound<'_, PyAny>| -> PyResult<String> {
                Ok(if v.is_truthy()? { v.str()?.to_string() } else { String::new() })
            };
            let mut rows: Vec<(String, String)> = Vec::new();
            let campos = handle.getattr("campos")?;
            if campos.is_truthy()? {
                for row in campos.try_iter()? {
                    let row = row?;
                    rows.push((row.get_item(0)?.str()?.to_string(), as_str(row.get_item(1)?)?));
                }
            } else {
                for pair in handle.getattr("FIELDS")?.try_iter()? {
                    let (label, attr): (String, String) = pair?.extract()?;
                    rows.push((label, as_str(handle.getattr(attr.as_str())?)?));
                }
            }
            let mut out = Vec::new();
            for (label, value) in rows {
                let mut v: String = expand.call1((value,))?.extract()?;
                if v.is_empty() && matches!(label.trim().to_lowercase().as_str(), "escala" | "scale") {
                    v = expand.call1(("{escala}",))?.extract()?;
                }
                out.push(Value::List(vec![Value::Str(label), Value::Str(v)]));
            }
            put("rows".into(), Value::List(out));
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
        ItemKind::Dimension => items_b::paint_dimension(&mut p, f, x),
        ItemKind::AngularDimension => items_b::paint_angular(&mut p, f, x),
        ItemKind::RadialDimension => items_b::paint_radial(&mut p, f, x),
        ItemKind::Label => items_b::paint_label(&mut p, f, x),
        ItemKind::Profile => items_b::paint_profile(&mut p, f, x),
        ItemKind::Text => super::items_c::paint_text(&mut p, f, x),
        ItemKind::TitleBlock => super::items_c::paint_titleblock(&mut p, f, x),
        ItemKind::Image => {
            let path = f.text("path", "");
            items::paint_image(&mut p, f, !path.is_empty() && std::path::Path::new(&path).is_file());
        }
        _ => return None,
    }
    Some(p.ops)
}

/// The item's box in paper mm, as its record class states it (the
/// ``w_mm``/``h_mm`` fields, or the properties core.composition derives).
pub fn size(kind: ItemKind, f: &Fields) -> (f64, f64) {
    let field = |w: f64, h: f64| (f.num("w_mm", w), f.num("h_mm", h));
    match kind {
        ItemKind::ScaleBar => items::scalebar_size(f),
        ItemKind::North => (f.num("size_mm", 18.0), f.num("size_mm", 18.0)),
        ItemKind::Legend => (f.num("w_mm", 55.0), items::legend_height(f)),
        ItemKind::Dimension => items_b::dimension_size(f),
        ItemKind::AngularDimension => items_b::angular_size(f),
        ItemKind::RadialDimension => items_b::radial_size(f),
        ItemKind::Label => items_b::label_size(f),
        ItemKind::Text => {
            // TextItem.size_mm: the lines at 1.4 × the size, 6 mm at least.
            let lines = f.text("text", "").matches('\n').count() as f64 + 1.0;
            (f.num("w_mm", 80.0), (f.num("size_pt", 14.0) * 25.4 / 72.0 * 1.4 * lines).max(6.0))
        }
        ItemKind::Level => (1.0, 1.0),
        ItemKind::Frame => field(170.0, 170.0),
        ItemKind::TitleBlock => field(180.0, 33.0),
        ItemKind::Image => field(60.0, 40.0),
        ItemKind::Shape => field(40.0, 25.0),
        ItemKind::Callout => field(30.0, 20.0),
        ItemKind::Profile => field(180.0, 70.0),
    }
}

/// The primitives of sheet item ``item`` (a record of core.composition), as
/// tests/sheet_paint_recorder.py records the Python painter's: a JSON list.
/// ``profile``: a terrain profile item's ``(profile, path_name, message)``.
#[pyfunction]
#[pyo3(signature = (item, profile=None))]
pub fn sheet_item_display(py: Python<'_>, item: &Bound<'_, PyAny>, profile: Option<Bound<'_, PyAny>>) -> PyResult<String> {
    py.import("core.doc_model")?.call_method0("flush")?;
    let class = item.get_type().name()?.to_string();
    if class == "Composicion" {
        // The sheet itself: its border on the margin rectangle.
        let (pw, ph): (f64, f64) = item.call_method0("page_size_mm")?.extract()?;
        let f = super::model::read_fields(crate::doc::py::handle_key(item)?)?;
        let mut p = super::paint::Painter::default();
        super::items_c::paint_sheet_border(&mut p, &f, pw, ph);
        return Ok(ops_json(&p.ops).to_string());
    }
    let kind = ItemKind::of_class(&class)
        .ok_or_else(|| pyo3::exceptions::PyValueError::new_err(format!("not a sheet item: {class}")))?;
    let key = crate::doc::py::handle_key(item)?;
    let f = super::model::read_fields(key)?;
    let x = extra(py, item, kind, &f, profile.as_ref())?;
    let ops = paint(kind, &f, &x).unwrap_or_default();
    Ok(ops_json(&ops).to_string())
}

/// The box of sheet item ``item`` in paper mm: ``(w, h)``.
#[pyfunction]
pub fn sheet_item_size(py: Python<'_>, item: &Bound<'_, PyAny>) -> PyResult<(f64, f64)> {
    py.import("core.doc_model")?.call_method0("flush")?;
    let class = item.get_type().name()?.to_string();
    let kind = ItemKind::of_class(&class)
        .ok_or_else(|| pyo3::exceptions::PyValueError::new_err(format!("not a sheet item: {class}")))?;
    let f = super::model::read_fields(crate::doc::py::handle_key(item)?)?;
    Ok(size(kind, &f))
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(sheet_item_display, m)?)?;
    m.add_function(wrap_pyfunction!(sheet_item_size, m)?)?;
    m.add_function(wrap_pyfunction!(sheet_duplicates, m)?)?;
    m.add_function(wrap_pyfunction!(sheet_next_z, m)?)?;
    Ok(())
}

/// Copies of sheet items for Duplicate (views/composer.py
/// `duplicate_selected`): 5 mm down and right, unlocked, on top of the
/// sheet in their order, grouped again under fresh group ids, a frame with
/// a fresh uid, anchors let go. The title block is not copied. Returns the
/// new records; adding them to the sheet is the caller's history step.
#[pyfunction]
pub fn sheet_duplicates<'py>(py: Python<'py>, sheet: &Bound<'py, PyAny>, items: Vec<Bound<'py, PyAny>>)
    -> PyResult<Vec<Bound<'py, PyAny>>> {
    let s = super::model::Sheet::read(sheet, None)?;
    let next_z = s.items.iter().map(|i| i.z).fold(0.0f64, f64::max) + 1.0;
    let uuid = py.import("uuid")?;
    let fresh = || -> PyResult<String> { uuid.call_method0("uuid4")?.getattr("hex")?.extract() };
    let mut groups: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut out = Vec::new();
    for item in items {
        let class = item.get_type();
        if class.name()?.to_string() == "Cajetin" {
            continue;
        }
        let copy = class.call0()?;
        let names: Vec<String> = item.getattr("_record_fields")?.extract()?;
        let deep = py.import("copy")?.getattr("deepcopy")?;
        for n in &names {
            if let Ok(v) = item.getattr(n.as_str()) {
                copy.setattr(n.as_str(), deep.call1((v,))?)?;
            }
        }
        for k in ["x_mm", "y_mm"] {
            let v: f64 = copy.getattr(k)?.extract()?;
            copy.setattr(k, v + 5.0)?;
        }
        copy.setattr("locked", false)?;
        let gid: String = copy.getattr("group_id").and_then(|g| g.extract()).unwrap_or_default();
        if !gid.is_empty() {
            let new = match groups.get(&gid) {
                Some(g) => g.clone(),
                None => {
                    let g = fresh()?;
                    groups.insert(gid, g.clone());
                    g
                }
            };
            copy.setattr("group_id", new)?;
        }
        if names.iter().any(|n| n == "uid") {
            copy.setattr("uid", if class.name()?.to_string() == "MarcoVista" { fresh()? } else { String::new() })?;
        }
        if names.iter().any(|n| n == "anchor_uid") {
            copy.setattr("anchor_uid", "")?;
            copy.setattr("a_world", py.None())?;
            if names.iter().any(|n| n == "b_world") {
                copy.setattr("b_world", py.None())?;
            }
        }
        copy.setattr("z", next_z + out.len() as f64)?;
        out.push(copy);
    }
    Ok(out)
}

/// The z an item put on top of sheet ``sheet`` takes (the composer's
/// ``_next_z``): above every item on it.
#[pyfunction]
pub fn sheet_next_z(sheet: &Bound<'_, PyAny>) -> PyResult<f64> {
    let s = super::model::Sheet::read(sheet, None)?;
    Ok(s.items.iter().map(|i| i.z).fold(0.0f64, f64::max) + 1.0)
}
