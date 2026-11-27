// SPDX-License-Identifier: GPL-3.0-or-later
//! A delivery's rules (formats/delivery, built_render deliveries_host.rs): the
//! formats, the scopes each format takes, which fields of the form a
//! format and a scope show, how the form's values become a record (the
//! format's own options), and what makes a record valid. The form, the
//! protocol's create_delivery / edit_delivery and the writers read this.
use pyo3::prelude::*;
use serde_json::{json, Map, Value};
use std::path::Path;

pub const FORMATS: [&str; 9] = ["ifc", "dxf", "glb", "obj", "dae", "svg", "pdf", "stl", "step"];
pub const PACKAGE_OUTPUTS: [(&str, &str, &str); 7] = [
    ("ifc", "Model + IDS report", "model.ifc"),
    ("step", "STEP geometry", "model.step"),
    ("pdf", "Sheet set", "sheets.pdf"),
    ("dxf", "Drawings", "drawings.dxf"),
    ("bcf", "Task topics", "topics.bcfzip"),
    ("csv", "Quantity schedule", "schedule.csv"),
    ("manifest", "File inventory", "manifest.json"),
];

pub const STL_UNITS: [&str; 5] = ["mm", "cm", "m", "in", "ft"];

/// The scopes a format takes, (id, label); "node" only with a project tree.
pub fn scopes(fmt: &str, nodes: bool) -> Vec<(&'static str, &'static str)> {
    if fmt == "pdf" {
        return vec![("sheets", "Sheets")];
    }
    let mut out = vec![("whole", "Whole model"), ("visible", "Visible"), ("selection", "Selection")];
    if nodes {
        out.push(("node", "Project tree node"));
    }
    if fmt == "dxf" || fmt == "svg" {
        out.push(("scenes", "Scenes"));
    }
    out
}

/// Whether the form shows field ``fid`` for this format and scope.
pub fn visible(fid: &str, fmt: &str, scope: &str) -> bool {
    match fid {
        "node" => scope == "node",
        "ids" | "sheet_names" | "drawing_names" | "topics" => fmt == "package",
        id if id.starts_with("include_") => fmt == "package",
        "names" => scope == "sheets" || scope == "scenes",
        "profile" => fmt == "ifc" || fmt == "package",
        "textures" => fmt == "glb",
        "unit" => fmt == "stl",
        _ => true,
    }
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// The record the form's values make: name, format, scope, destination,
/// the node for a node scope, and the format's options (``svg``: the SVG
/// options' own JSON).
pub fn record(values: &Value, svg: &Value) -> Value {
    let fmt = text(values, "format");
    let scope = text(values, "scope");
    let mut opts = Map::new();
    if matches!(fmt, "ifc" | "package") && !text(values, "profile").is_empty() {
        opts.insert("profile".into(), json!(text(values, "profile")));
    }
    if fmt == "glb" {
        opts.insert("textures".into(), values.get("textures").cloned().unwrap_or(json!(true)));
    }
    if fmt == "svg" {
        if let Some(o) = svg.as_object() {
            opts.extend(o.clone());
        }
    }
    if fmt == "stl" {
        opts.insert("unit".into(), json!(text(values, "unit")));
    }
    let names = text(values, "names");
    if (scope == "sheets" || scope == "scenes") && !names.trim().is_empty() {
        opts.insert("names".into(), json!(names.lines().collect::<Vec<_>>()));
    }
    let mut r = json!({"name": text(values, "name"), "format": fmt, "scope": scope,
                       "destination": text(values, "destination")});
    if fmt == "package" {
        let mut outputs = Vec::new();
        for (format, _, _) in PACKAGE_OUTPUTS {
            if !values[format!("include_{format}")].as_bool().unwrap_or(format != "step") {
                continue;
            }
            let custom = text(values, &format!("source_{format}"));
            let source = match format {
                "pdf" => "sheets",
                "bcf" => "topics",
                "manifest" => "files",
                "dxf" if !text(values, "drawing_names").trim().is_empty() => "scenes",
                _ if !custom.is_empty() => custom,
                _ => scope,
            };
            let mut options = json!({});
            if format == "ifc" {
                for k in ["profile", "ids"] {
                    if !text(values, k).is_empty() {
                        options[k] = values[k].clone();
                    }
                }
            }
            let names = match format {
                "pdf" => "sheet_names",
                "dxf" => "drawing_names",
                "bcf" => "topics",
                _ => "",
            };
            if !text(values, names).trim().is_empty() {
                options["names"] = json!(text(values, names)
                    .lines()
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                    .collect::<Vec<_>>());
            }
            outputs.push(json!({"format":format,"scope":source,"options":options}));
        }
        r["outputs"] = json!(outputs);
    }
    if scope == "node" {
        r["node"] = json!(text(values, "node"));
    }
    r["options"] = Value::Object(opts);
    r
}

fn number(v: &Value) -> Option<f64> {
    v.as_f64()
}

/// Why the record is not a delivery, or None.
pub fn validate(record: &Value) -> Option<String> {
    let fmt = record.get("format").and_then(Value::as_str).unwrap_or(
        if record.get("outputs").is_some() {
            "package"
        } else {
            ""
        },
    );
    if fmt != "package" && !FORMATS.contains(&fmt) {
        return Some("Choose an export format".into());
    }
    for field in ["name", "destination"] {
        if record.get(field).and_then(Value::as_str).map_or(true, |s| s.trim().is_empty()) {
            return Some(format!("Enter a {field}"));
        }
    }
    let scope = match record.get("scope") {
        None => "whole",
        Some(v) => v.as_str().unwrap_or("\u{0}"),
    };
    let allowed: Vec<&str> = if fmt == "pdf" { vec!["sheets"] } else {
        let mut a = vec!["whole", "visible", "selection", "node"];
        if fmt == "dxf" || fmt == "svg" {
            a.push("scenes");
        }
        a
    };
    if !allowed.contains(&scope) {
        return Some(format!("{} requires one of {}", fmt.to_uppercase(), allowed.join(", ")));
    }
    if scope == "node" && record.get("node").map_or(true, |n| n.is_null() || n.as_str() == Some("")) {
        return Some("Choose a project node".into());
    }
    let dest = record.get("destination").and_then(Value::as_str).unwrap_or("");
    if fmt == "package" {
        let Some(outputs) = record["outputs"].as_array().filter(|o| !o.is_empty()) else {
            return Some("Select at least one output".into());
        };
        let mut seen = std::collections::HashSet::new();
        for out in outputs {
            let format = text(out, "format");
            if !PACKAGE_OUTPUTS.iter().any(|p| p.0 == format) || !seen.insert(format) {
                return Some("Choose each supported output once".into());
            }
            let source = text(out, "scope");
            let valid = match format {
                "pdf" => source == "sheets",
                "bcf" => source == "topics",
                "manifest" => source == "files",
                "dxf" if source == "scenes" => true,
                _ => matches!(source, "whole" | "visible" | "node" | "selection"),
            };
            if source == "node"
                && out
                    .get("node")
                    .or(record.get("node"))
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
            {
                return Some(format!("Choose a project node for {format}"));
            }
            if !valid || !out["options"].is_object() {
                return Some(format!("Invalid {format} output setup"));
            }
            if let Some(names) = out["options"].get("names") {
                if !names
                    .as_array()
                    .is_some_and(|v| v.iter().all(Value::is_string))
                {
                    return Some("Expected a list of names".into());
                }
            }
        }
        return None;
    }
    let suffix = Path::new(dest).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if suffix != fmt {
        return Some(format!("Choose a destination ending in .{fmt}"));
    }
    let empty = json!({});
    let opts = record.get("options").unwrap_or(&empty);
    let Some(o) = opts.as_object() else { return Some("Options must be an object".into()) };
    if let Some(names) = o.get("names") {
        if !names.as_array().is_some_and(|a| a.iter().all(Value::is_string)) {
            return Some("Scene and sheet names must be a list of names".into());
        }
    }
    if fmt == "svg" {
        if let Some(paper) = o.get("paper").filter(|p| !p.is_null()) {
            let ok = paper.as_str() == Some("screen") || paper.as_array().is_some_and(|a| {
                a.len() == 2 && a.iter().all(|n| number(n).is_some_and(|n| n > 0.0))
            });
            if !ok {
                return Some("Choose screen size or a positive paper width and height".into());
            }
        }
        if let Some(frame) = o.get("frame").filter(|f| !f.is_null()) {
            let ok = frame.as_array().is_some_and(|a| {
                a.len() == 3 && a.iter().all(|n| number(n).is_some()) && number(&a[2]).is_some_and(|z| z > 0.0)
            });
            if !ok {
                return Some("Framing takes horizontal pan, vertical pan, and positive zoom".into());
            }
        }
        let truthy = |v: &Value| match v {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Number(n) => n.as_f64() != Some(0.0),
            Value::String(s) => !s.is_empty(),
            Value::Array(a) => !a.is_empty(),
            Value::Object(m) => !m.is_empty(),
        };
        if o.get("picture").is_some_and(truthy) && scope != "visible" {
            return Some("Choose Visible for a picture of the viewport".into());
        }
        if o.get("faces").is_some_and(|f| !f.is_object()) {
            return Some("SVG face options must be an object".into());
        }
    }
    if fmt == "stl" {
        let unit = o.get("unit").map_or(Some("mm"), Value::as_str);
        if !unit.is_some_and(|u| STL_UNITS.contains(&u)) {
            return Some("Choose a supported STL unit".into());
        }
    }
    None
}

fn parse(text: &str) -> PyResult<Value> {
    serde_json::from_str(text).map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
}

#[pyfunction]
pub fn delivery_formats() -> Vec<&'static str> {
    FORMATS.to_vec()
}

#[pyfunction]
pub fn delivery_scopes(fmt: &str, nodes: bool) -> Vec<(&'static str, &'static str)> {
    scopes(fmt, nodes)
}

#[pyfunction]
pub fn delivery_field_visible(fid: &str, fmt: &str, scope: &str) -> bool {
    visible(fid, fmt, scope)
}

/// The record (JSON) of the form's values (JSON) and the SVG options (JSON).
#[pyfunction]
#[pyo3(signature = (values, svg="{}"))]
pub fn delivery_record(values: &str, svg: &str) -> PyResult<String> {
    Ok(record(&parse(values)?, &parse(svg)?).to_string())
}

/// Why the record (JSON) is not a valid delivery, or "".
#[pyfunction]
pub fn delivery_error(record: &str) -> PyResult<String> {
    Ok(validate(&parse(record)?).unwrap_or_default())
}

/// Initial values for an unsaved delivery opened by a File menu shortcut.
#[pyfunction]
fn delivery_defaults<'a>(format: &str, one_off: bool, last_unit: &'a str) -> (&'static str, &'a str) {
    let scope = if one_off && format == "dxf" { "visible" } else { "whole" };
    let unit = if one_off && STL_UNITS.contains(&last_unit) { last_unit } else { "mm" };
    (scope, unit)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(delivery_defaults, m)?)?;
    m.add_function(wrap_pyfunction!(delivery_formats, m)?)?;
    m.add_function(wrap_pyfunction!(delivery_scopes, m)?)?;
    m.add_function(wrap_pyfunction!(delivery_field_visible, m)?)?;
    m.add_function(wrap_pyfunction!(delivery_record, m)?)?;
    m.add_function(wrap_pyfunction!(delivery_error, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(r: Value) -> Option<String> {
        validate(&r)
    }

    #[test]
    fn scopes_and_fields_follow_the_format() {
        assert_eq!(scopes("pdf", true), [("sheets", "Sheets")]);
        assert_eq!(scopes("ifc", false).len(), 3);
        assert_eq!(scopes("svg", true).last().unwrap().0, "scenes");
        assert!(visible("unit", "stl", "whole") && !visible("unit", "ifc", "whole"));
        assert!(visible("names", "pdf", "sheets") && !visible("node", "ifc", "whole"));
    }

    #[test]
    fn the_form_makes_its_record() {
        let v = json!({"name": "A", "format": "svg", "scope": "scenes", "destination": "a.svg", "names": "One\nTwo",
                       "profile": "x", "unit": "mm", "textures": false, "node": "n"});
        let r = record(&v, &json!({"paper": "screen"}));
        assert_eq!(r["options"], json!({"paper": "screen", "names": ["One", "Two"]}));
        assert!(r.get("node").is_none());
        let r = record(&json!({"format": "ifc", "scope": "node", "profile": "p", "node": "n"}), &json!({}));
        assert_eq!((r["node"].clone(), r["options"].clone()), (json!("n"), json!({"profile": "p"})));
    }

    #[test]
    fn validation_says_what_is_wrong() {
        let base = json!({"name": "A", "format": "ifc", "destination": "out/a.IFC"});
        assert_eq!(ok(base.clone()), None);
        assert_eq!(ok(json!({"format": "zip"})).unwrap(), "Choose an export format");
        assert_eq!(ok(json!({"format": "ifc", "name": " ", "destination": "a.ifc"})).unwrap(), "Enter a name");
        assert_eq!(ok(json!({"name": "A", "format": "pdf", "destination": "a.pdf"})).unwrap(), "PDF requires one of sheets");
        assert_eq!(ok(json!({"name": "A", "format": "ifc", "scope": "node", "destination": "a.ifc"})).unwrap(),
                   "Choose a project node");
        assert_eq!(ok(json!({"name": "A", "format": "ifc", "destination": "a.dxf"})).unwrap(),
                   "Choose a destination ending in .ifc");
        let svg = |o: Value| ok(json!({"name": "A", "format": "svg", "scope": "whole", "destination": "a.svg", "options": o}));
        assert_eq!(svg(json!({"paper": [297, 0]})).unwrap(), "Choose screen size or a positive paper width and height");
        assert_eq!(svg(json!({"frame": [0, 0, 0]})).unwrap(), "Framing takes horizontal pan, vertical pan, and positive zoom");
        assert_eq!(svg(json!({"picture": true})).unwrap(), "Choose Visible for a picture of the viewport");
        assert_eq!(svg(json!({"names": [1]})).unwrap(), "Scene and sheet names must be a list of names");
        assert_eq!(svg(json!({"paper": [297, 210.5], "frame": [1, -1, 2]})), None);
        assert_eq!(ok(json!({"name": "A", "format": "stl", "destination": "a.stl", "options": {"unit": "yd"}})).unwrap(),
                   "Choose a supported STL unit");
    }
}
