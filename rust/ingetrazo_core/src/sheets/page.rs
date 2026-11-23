// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! The page display list (docs/rust-sheets.md §3): one sheet as the
//! renderer paints it, in paper millimetres. The renderer
//! (`ingetrazo_render::ui::page`) parses this JSON; the two crates are
//! separate Python extensions, so the snapshot crosses as a string.

use super::model::{format_scale, Item, ItemKind, Sheet};
use crate::doc::value::Value;
use pyo3::prelude::*;
use serde_json::{json, Value as Json};

fn rect(i: &Item) -> Json {
    json!(i.rect)
}

/// The view's name as a frame title says it (`frame_view_name`).
pub fn view_name(view_key: &str) -> String {
    if let Some(name) = view_key.strip_prefix("scene:") {
        return name.to_string();
    }
    match view_key.strip_prefix("std:") {
        Some("top") => "Top (plan)".into(),
        Some("bottom") => "Bottom".into(),
        Some("front") => "Front".into(),
        Some("back") => "Back".into(),
        Some("left") => "Left".into(),
        Some("right") => "Right".into(),
        Some("iso") => "Isometric".into(),
        Some(other) => other.to_string(),
        None => "Current view".into(),
    }
}

fn frame(i: &Item) -> Json {
    let f = &i.fields;
    let persp = f.flag("perspective", false);
    let scale = f.num("scale_n", 100.0);
    let title = {
        let typed = f.text("title_text", "");
        if typed.is_empty() { view_name(&f.text("view_key", "__current__")) } else { typed }
    };
    let style = f.text("style", "sombreado");
    json!({
        "view": f.text("view_key", "__current__"),
        "style": style,
        // What fills it: the exact vector pass, or a render.
        "vector": style == "vectorial" && !persp,
        "scale": if persp { "No scale".to_string() } else { format_scale(scale) },
        "scale_n": scale,
        "title": title,
        "show_title": f.flag("show_title", false),
        "title_scale": f.flag("title_scale", true),
        "title_mm": f.num("title_mm", 4.0),
        "title_pos": f.text("title_pos", "below"),
        "title_align": f.text("title_align", "left"),
        "border": f.flag("border", false),
        "border_mm": f.num("border_mm", 0.3),
        "border_color": f.text("border_color", "#282e36"),
        "pens": [f.num("pen_edge_mm", 0.18), f.num("pen_profile_mm", 0.35),
                 f.num("pen_cut_mm", 0.5), f.num("pen_edge_mm", 0.18)],
        "profiles": f.flag("profiles", true),
        "cut_fill": f.text("cut_fill", "solid"),
        "cut_fill_color": f.text("cut_fill_color", "#595e69"),
        "perspective": persp,
    })
}

fn text(i: &Item) -> Json {
    let f = &i.fields;
    json!({
        "text": f.text("text", ""),
        "size_pt": f.num("size_pt", 14.0),
        "bold": f.flag("bold", false),
        "italic": f.flag("italic", false),
        "underline": f.flag("underline", false),
        "family": f.text("family", "Sans Serif"),
        "color": f.text("color", "#1e242c"),
        "align": f.text("align", "left"),
        "bg_color": f.text("bg_color", ""),
        "bg_opacity": f.num("bg_opacity", 1.0),
    })
}

fn rows_of(v: &[Value]) -> Vec<[String; 2]> {
    let s = |v: Option<&Value>| match v {
        Some(Value::Str(s)) => s.clone(),
        Some(Value::Int(i)) => i.to_string(),
        Some(Value::Float(f)) => f.to_string(),
        _ => String::new(),
    };
    v.iter()
        .filter_map(|r| match r {
            Value::List(c) | Value::Tuple(c) => Some([s(c.first()), s(c.get(1))]),
            _ => None,
        })
        .collect()
}

fn title_block(i: &Item) -> Json {
    let f = &i.fields;
    json!({
        "rows": rows_of(f.list("campos")),
        "columns": f.num("columns", 1.0).max(1.0) as i64,
        "border_mm": f.num("border_mm", 0.5),
        "line_mm": f.num("line_mm", 0.2),
        "layout": f.text("layout", "grid"),
        "fill_color": f.text("fill_color", ""),
        "label_color": f.text("label_color", "#5a626c"),
        "text_color": f.text("text_color", "#1e242c"),
        "line_color": f.text("line_color", "#1e242c"),
        "label_mm": f.num("label_mm", 0.0),
        "double_border": f.flag("double_border", false),
    })
}

fn shape(i: &Item) -> Json {
    let f = &i.fields;
    json!({
        "shape": f.text("kind", "rect"),
        "stroke_mm": f.num("stroke_mm", 0.35),
        "fill": f.flag("fill", false),
        "invert": f.flag("invert", false),
        "color": f.text("color", "#1e242c"),
        "fill_color": f.text("fill_color", "#e2e8ee"),
        "radius_mm": f.num("radius_mm", 0.0),
        "sides": f.num("sides", 6.0) as i64,
    })
}

/// The sheet as the renderer's page snapshot.
pub fn page_json(s: &Sheet) -> Json {
    let f = &s.fields;
    let items: Vec<Json> = s
        .items
        .iter()
        .map(|i| {
            let mut o = json!({
                "key": i.key,
                "kind": i.kind.name(),
                "rect": rect(i),
                "z": i.z,
                "locked": i.locked,
                "hidden": i.hidden,
                "editable": i.kind.native_edit(),
            });
            let extra = match i.kind {
                ItemKind::Frame => Some(("frame", frame(i))),
                ItemKind::Text => Some(("text", text(i))),
                ItemKind::TitleBlock => Some(("titleblock", title_block(i))),
                ItemKind::Shape => Some(("shape", shape(i))),
                _ => None,
            };
            if let Some((k, v)) = extra {
                o[k] = v;
            }
            o
        })
        .collect();
    let num_list = |name: &str| -> Vec<f64> {
        f.list(name).iter().filter_map(|v| match v {
            Value::Float(x) => Some(*x),
            Value::Int(x) => Some(*x as f64),
            _ => None,
        }).collect()
    };
    json!({
        "key": s.key,
        "name": f.text("name", ""),
        "paper": f.text("paper", "A4"),
        "landscape": f.flag("landscape", true),
        "size": [s.page.0, s.page.1],
        "margin": s.margin,
        "border": {
            "on": f.flag("border", false),
            "mm": f.num("border_mm", 0.5),
            "color": f.text("border_color", "#1e242c"),
            "radius_mm": f.num("border_radius_mm", 0.0),
            "style": f.text("border_style", "single"),
        },
        "guides": {"v": num_list("guides_v"), "h": num_list("guides_h")},
        "items": items,
    })
}

/// The page snapshot of sheet record ``comp`` (a ``Composicion``) as JSON:
/// paper, margin, border, guides and every item (key, kind, rect in mm,
/// z, locked, hidden, and the fields its painter needs).
#[pyfunction]
fn sheet_page_json(comp: &Bound<'_, PyAny>) -> PyResult<String> {
    Ok(page_json(&Sheet::read(comp)?).to_string())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(sheet_page_json, m)?)?;
    Ok(())
}
