// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! The Sheets dock's inspector, built from the page snapshot (the sheet's
//! records as the document holds them; docs/rust-sheets.md step 3): the
//! fields of the selected item, or of the sheet when nothing is selected,
//! as the dock's rows (ui/sheets.rs `sheet_rows`), and the edits typed
//! back into field changes. An edit leaves as ``set␟<key>␟<json changes>``
//! (``sheet`` for the sheet itself); views/wgpu_sheets.py runs it as one
//! step of the document's history.

use crate::sheet_page::{Item, Snapshot};
use serde_json::{json, Map, Value as J};

const ED: char = '\u{1d}';
const SEP: char = '\u{1f}';

/// How a field is shown and typed.
#[derive(Clone, Copy, PartialEq)]
enum F {
    /// A number with its unit (shown after the value).
    Num(&'static str),
    /// A whole number.
    Int,
    Text,
    Multi,
    Check,
    /// One of these keys, labelled.
    Pick(&'static [(&'static str, &'static str)]),
    /// One of a list the document gives (`choices.<name>`).
    Choices(&'static str),
    /// A 1:N scale, typed as 1:50, 10:1 or 50.
    Scale,
    /// A colour as #rrggbb.
    Color,
    /// An action, not a field.
    Button,
}

type Row = (&'static str, &'static str, F);

const UNITS: &[(&str, &str)] = &[("m", "m"), ("cm", "cm"), ("mm", "mm"), ("in", "in"), ("ft", "ft"),
    ("ft-in", "ft-in"), ("in-frac", "in (fractions)"), ("ft-in-frac", "ft-in (fractions)")];
const ENDS: &[(&str, &str)] = &[("tick", "Tick"), ("arrow", "Arrow"), ("none", "None")];
const ALIGN: &[(&str, &str)] = &[("left", "Left"), ("center", "Centre"), ("right", "Right")];
const PAPERS: &[(&str, &str)] = &[("A4", "A4"), ("A3", "A3"), ("A2", "A2"), ("A1", "A1"), ("A0", "A0")];

fn catalogue(kind: &str) -> &'static [Row] {
    match kind {
        "frame" => &[("view_key", "View", F::Choices("views")), ("scale_n", "Scale", F::Scale),
            ("style", "Style", F::Choices("styles")), ("w_mm", "Width", F::Num("mm")),
            ("h_mm", "Height", F::Num("mm")), ("rot_deg", "Rotation", F::Num("°")),
            ("perspective", "Perspective", F::Check), ("show_title", "View title", F::Check),
            ("paper_bg", "Paper background", F::Check), ("hidden_lines", "Hidden lines", F::Check),
            ("profiles", "Profiles", F::Check), ("border", "Border", F::Check),
            ("locked", "Locked", F::Check), ("refresh", "Update view", F::Button)],
        "text" => &[("text", "Text", F::Multi), ("size_pt", "Size", F::Num("pt")), ("bold", "Bold", F::Check),
            ("italic", "Italic", F::Check), ("underline", "Underline", F::Check),
            ("align", "Alignment", F::Pick(ALIGN)), ("color", "Colour", F::Color), ("w_mm", "Width", F::Num("mm")),
            ("locked", "Locked", F::Check), ("font", "Font…", F::Button)],
        "titleblock" => &[("w_mm", "Width", F::Num("mm")), ("h_mm", "Height", F::Num("mm")),
            ("columns", "Columns", F::Int), ("locked", "Locked", F::Check)],
        "image" => &[("path", "File", F::Text), ("opacity", "Opacity", F::Num("")),
            ("shape", "Shape", F::Pick(&[("rect", "Rectangle"), ("rounded", "Rounded"), ("ellipse", "Ellipse")])),
            ("fit", "Fit", F::Pick(&[("stretch", "Stretch"), ("cover", "Cover"), ("contain", "Contain")])),
            ("border", "Border", F::Check), ("w_mm", "Width", F::Num("mm")), ("h_mm", "Height", F::Num("mm")),
            ("locked", "Locked", F::Check)],
        "scalebar" => &[("scale_n", "Scale", F::Scale), ("segments", "Segments", F::Int), ("locked", "Locked", F::Check)],
        "north" => &[("size_mm", "Size", F::Num("mm")), ("angle_deg", "Angle", F::Num("°")), ("locked", "Locked", F::Check)],
        "legend" => &[("title", "Title", F::Text), ("w_mm", "Width", F::Num("mm")), ("locked", "Locked", F::Check)],
        "shape" => &[("stroke_mm", "Pen", F::Num("mm")), ("color", "Colour", F::Color), ("fill", "Fill", F::Check),
            ("fill_color", "Fill colour", F::Color), ("radius_mm", "Corner radius", F::Num("mm")),
            ("sides", "Sides", F::Int), ("w_mm", "Width", F::Num("mm")), ("h_mm", "Height", F::Num("mm")),
            ("locked", "Locked", F::Check)],
        "dimension" => &[("text", "Text (<> = value)", F::Text), ("text_mm", "Text height", F::Num("mm")),
            ("decimals", "Decimals", F::Int), ("units", "Units", F::Pick(UNITS)), ("show_unit", "Show unit", F::Check),
            ("ends", "Ends", F::Pick(ENDS)),
            ("text_pos", "Text position", F::Pick(&[("above", "Above"), ("below", "Below"), ("centered", "On the line"),
                ("aside", "Beside"), ("aside_below", "Beside, below")])),
            ("text_align", "Text", F::Pick(&[("aligned", "Along the line"), ("horizontal", "Horizontal")])),
            ("sep_mm", "Line offset", F::Num("mm")), ("offset_mm", "Text gap", F::Num("mm")),
            ("scale_n", "Scale", F::Scale), ("color", "Colour", F::Color), ("locked", "Locked", F::Check)],
        "angular" => &[("text", "Text (<> = value)", F::Text), ("text_mm", "Text height", F::Num("mm")),
            ("decimals", "Decimals", F::Int), ("ends", "Ends", F::Pick(ENDS)), ("radius_mm", "Arc radius", F::Num("mm")),
            ("offset_mm", "Text gap", F::Num("mm")), ("color", "Colour", F::Color), ("locked", "Locked", F::Check)],
        "radial" => &[("kind", "Measures", F::Pick(&[("radius", "Radius"), ("diameter", "Diameter")])),
            ("text", "Text (<> = value)", F::Text), ("text_mm", "Text height", F::Num("mm")),
            ("decimals", "Decimals", F::Int), ("units", "Units", F::Pick(UNITS)), ("ends", "Ends", F::Pick(ENDS)),
            ("placement", "Text", F::Pick(&[("auto", "Automatic"), ("in", "Inside"), ("out", "Outside")])),
            ("centre_mark", "Centre mark", F::Check), ("radius_mm", "Radius", F::Num("mm")),
            ("angle_deg", "Angle", F::Num("°")), ("scale_n", "Scale", F::Scale), ("color", "Colour", F::Color),
            ("locked", "Locked", F::Check)],
        "label" => &[("text", "Text", F::Multi), ("size_pt", "Size", F::Num("pt")), ("w_mm", "Width", F::Num("mm")),
            ("bold", "Bold", F::Check), ("italic", "Italic", F::Check), ("arrow", "Arrow", F::Check),
            ("dot", "Dot", F::Check), ("color", "Colour", F::Color), ("bg_color", "Background", F::Color),
            ("locked", "Locked", F::Check)],
        "level" => &[("text", "Text ({z} = level)", F::Text), ("z_m", "Level", F::Num("m")),
            ("symbol", "Symbol", F::Pick(&[("triangle", "Section"), ("circle", "Plan")])),
            ("mirror", "Text to the left", F::Check), ("size_mm", "Text height", F::Num("mm")),
            ("line_mm", "Line", F::Num("mm")), ("decimals", "Decimals", F::Int), ("color", "Colour", F::Color),
            ("locked", "Locked", F::Check)],
        "callout" => &[("number", "Number", F::Text), ("sheet", "Sheet", F::Text),
            ("shape", "Shape", F::Pick(&[("rect", "Rectangle"), ("circle", "Circle")])),
            ("size_mm", "Bubble text", F::Num("mm")), ("w_mm", "Width", F::Num("mm")), ("h_mm", "Height", F::Num("mm")),
            ("color", "Colour", F::Color), ("locked", "Locked", F::Check)],
        "profile" => &[("title", "Title", F::Text), ("scale_n", "Scale 1:N (0 fits)", F::Num("")),
            ("exag", "Exaggeration (0 fits)", F::Num("×")), ("grid", "Grid", F::Check), ("fill", "Fill", F::Check),
            ("text_mm", "Text height", F::Num("mm")), ("w_mm", "Width", F::Num("mm")), ("h_mm", "Height", F::Num("mm")),
            ("locked", "Locked", F::Check)],
        _ => &[],
    }
}

const SHEET: &[Row] = &[("name", "Name", F::Text), ("paper", "Paper", F::Pick(PAPERS)),
    ("landscape", "Landscape", F::Check), ("margin_mm", "Margin", F::Num("mm")),
    ("border", "Sheet border", F::Check), ("border_mm", "Border pen", F::Num("mm"))];

fn title_of(kind: &str) -> &'static str {
    match kind {
        "frame" => "Model view",
        "text" => "Text",
        "titleblock" => "Title block",
        "image" => "Image",
        "scalebar" => "Scale bar",
        "north" => "North arrow",
        "legend" => "Legend",
        "shape" => "Shape",
        "dimension" => "Dimension",
        "angular" => "Angular dimension",
        "radial" => "Radius dimension",
        "label" => "Label",
        "level" => "Level",
        "callout" => "Detail callout",
        "profile" => "Terrain profile",
        _ => "Item",
    }
}

/// A number the way the field shows it: up to four decimals, no trailing zeros.
fn num_text(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// core.composition.format_scale.
pub fn format_scale(n: f64) -> String {
    if !(n > 0.0) {
        return String::new();
    }
    if n < 1.0 { format!("{}:1", num_text(1.0 / n)) } else { format!("1:{}", num_text(n)) }
}

/// core.composition.parse_scale: ``1:50`` → 50, ``10:1`` → 0.1, ``50`` → 50.
pub fn parse_scale(text: &str) -> Option<f64> {
    let s = text.trim().replace(',', ".");
    if let Some((a, b)) = s.split_once(':') {
        let (a, b) = (a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?);
        return (a > 0.0 && b > 0.0).then_some(b / a);
    }
    s.parse::<f64>().ok().filter(|n| *n > 0.0)
}

/// The first number typed («123,5 mm» → 123.5).
fn parse_num(text: &str) -> Option<f64> {
    let t = text.replace(',', ".");
    let start = t.find(|c: char| c.is_ascii_digit() || c == '-' || c == '+' || c == '.')?;
    let rest = &t[start..];
    let end = rest.char_indices().skip(1).find(|(_, c)| !(c.is_ascii_digit() || *c == '.')).map_or(rest.len(), |(i, _)| i);
    rest[..end].parse().ok()
}

fn clean(s: &str) -> String {
    s.replace([ED, SEP, '|'], " ")
}

fn choices<'a>(doc: &'a Snapshot, name: &str) -> Vec<(String, String)> {
    doc.choices[name].as_array().map(|a| a.iter().filter_map(|p| {
        Some((p[0].as_str()?.to_string(), p[1].as_str().unwrap_or("").to_string()))
    }).collect()).unwrap_or_default()
}

fn row(doc: &Snapshot, owner: &str, fields: &J, (name, label, kind): &Row) -> Option<(String, String)> {
    let fid = format!("{owner}:{name}");
    let v = &fields[*name];
    let text = |v: &J| match v {
        J::String(s) => clean(s),
        J::Number(n) => num_text(n.as_f64().unwrap_or(0.0)),
        _ => String::new(),
    };
    Some(((*label).to_string(), match kind {
        F::Num(unit) => {
            let t = text(v);
            format!("{ED}edit{ED}{fid}{ED}{}", if unit.is_empty() { t } else { format!("{t} {unit}") })
        }
        F::Int | F::Text | F::Color => format!("{ED}edit{ED}{fid}{ED}{}", text(v)),
        F::Scale => format!("{ED}edit{ED}{fid}{ED}{}", format_scale(v.as_f64().unwrap_or(100.0))),
        F::Multi => format!("{ED}text{ED}{fid}{ED}{}", v.as_str().unwrap_or("").replace([ED, SEP], " ")),
        F::Check => format!("{ED}check{ED}{fid}{ED}{}", u8::from(v.as_bool().unwrap_or(false))),
        F::Pick(opts) => {
            let at = opts.iter().position(|(k, _)| Some(*k) == v.as_str()).unwrap_or(0);
            let labels: Vec<&str> = opts.iter().map(|(_, l)| *l).collect();
            format!("{ED}pick{ED}{fid}{ED}{}{ED}{at}", labels.join("|"))
        }
        F::Choices(list) => {
            let opts = choices(doc, list);
            if opts.is_empty() {
                return None;
            }
            let at = opts.iter().position(|(k, _)| Some(k.as_str()) == v.as_str()).unwrap_or(0);
            let labels: Vec<String> = opts.iter().map(|(_, l)| clean(l)).collect();
            format!("{ED}pick{ED}{fid}{ED}{}{ED}{at}", labels.join("|"))
        }
        F::Button => format!("{ED}button{ED}{fid}"),
    }))
}

/// The dock's title and rows for the current selection.
pub fn sections(doc: &Snapshot, selection: &[u64]) -> (String, Vec<(String, Vec<(String, String)>)>) {
    let items: Vec<&Item> = selection.iter().filter_map(|k| doc.items.iter().find(|i| i.key == *k)).collect();
    match items.as_slice() {
        [] => {
            let rows = SHEET.iter().filter_map(|r| row(doc, "sheet", &doc.fields, r)).collect();
            ("Sheet".into(), vec![(String::new(), rows)])
        }
        [item] => {
            let fields = &item.data["fields"];
            let owner = item.key.to_string();
            let mut rows: Vec<(String, String)> = catalogue(&item.kind).iter()
                .filter_map(|r| row(doc, &owner, fields, r)).collect();
            if item.kind == "titleblock" {
                // The rows of the block: each value typed in place.
                let mut cells: Vec<(String, String)> = Vec::new();
                for (i, cell) in fields["campos"].as_array().into_iter().flatten().enumerate() {
                    let label = clean(cell[0].as_str().unwrap_or(""));
                    let value = clean(cell[1].as_str().unwrap_or(""));
                    cells.push((if label.is_empty() { format!("Row {}", i + 1) } else { label },
                                format!("{ED}edit{ED}{owner}:campos.{i}{ED}{value}")));
                }
                cells.append(&mut rows);
                rows = cells;
            }
            (title_of(&item.kind).into(), vec![(String::new(), rows)])
        }
        many => {
            let all_locked = many.iter().all(|i| i.locked);
            let ids: Vec<String> = many.iter().map(|i| i.key.to_string()).collect();
            let rows = vec![("Locked".to_string(), format!("{ED}check{ED}{}:locked{ED}{}", ids.join(","), u8::from(all_locked)))];
            (format!("{} items", many.len()), vec![(String::new(), rows)])
        }
    }
}

fn kind_of<'a>(doc: &'a Snapshot, owner: &str) -> Option<(&'a str, &'a J)> {
    if owner == "sheet" {
        return Some(("sheet", &doc.fields));
    }
    let key: u64 = owner.parse().ok()?;
    doc.items.iter().find(|i| i.key == key).map(|i| (i.kind.as_str(), &i.data["fields"]))
}

/// An edit of the dock's row ``fid`` (``owner:field``) with the raw text
/// or index the row gives, as the action that changes the document.
pub fn edit(doc: &Snapshot, fid: &str, raw: &str) -> Option<String> {
    let (owner, field) = fid.split_once(':')?;
    // Several items at once (the Locked switch of a multiple selection).
    if owner.contains(',') {
        let on = raw == "1";
        let parts: Vec<String> = owner.split(',').map(|k| format!("{k}{SEP}{}", json!({field: on}))).collect();
        return Some(format!("set_many{SEP}{}", parts.join(&SEP.to_string())));
    }
    let (kind, fields) = kind_of(doc, owner)?;
    if let Some(i) = field.strip_prefix("campos.") {
        let i: usize = i.parse().ok()?;
        let mut rows = fields["campos"].as_array()?.clone();
        let cell = rows.get_mut(i)?.as_array_mut()?;
        if cell.len() < 2 {
            return None;
        }
        cell[1] = J::String(raw.to_string());
        return Some(format!("set{SEP}{owner}{SEP}{}", json!({"campos": rows})));
    }
    let table = if kind == "sheet" { SHEET } else { catalogue(kind) };
    let (_, _, f) = table.iter().find(|(n, _, _)| *n == field)?;
    let value = match f {
        F::Num(_) => json!(parse_num(raw)?),
        F::Int => json!(parse_num(raw)?.round() as i64),
        F::Text | F::Multi => json!(raw),
        F::Color => {
            let t = raw.trim();
            let ok = t.is_empty() || (t.starts_with('#') && t.len() == 7 && t[1..].chars().all(|c| c.is_ascii_hexdigit()));
            if !ok {
                return None;
            }
            json!(t)
        }
        F::Scale => json!(parse_scale(raw)?),
        F::Check => json!(raw == "1"),
        F::Pick(opts) => json!(opts.get(raw.parse::<usize>().ok()?)?.0),
        F::Choices(list) => json!(choices(doc, list).get(raw.parse::<usize>().ok()?)?.0),
        F::Button => return Some(format!("{field}{SEP}{owner}")),
    };
    if fields[field] == value {
        return None;
    }
    let mut changes = Map::new();
    changes.insert(field.to_string(), value);
    if kind == "frame" && field == "view_key" {
        // A new source is a new camera: the in-place view edits belonged
        // to the old one (the composer's rule).
        for k in ["cam_target", "cam_yaw", "cam_pitch", "cam_distance"] {
            changes.insert(k.into(), J::Null);
        }
    }
    let changes = J::Object(changes);
    Some(if kind == "sheet" { format!("sheet{SEP}{changes}") } else { format!("set{SEP}{owner}{SEP}{changes}") })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Snapshot {
        Snapshot::parse(r#"{"key":7,"size":[420,297],"margin":10,"border":{"on":false},"guides":{"v":[],"h":[]},
            "fields":{"name":"Plans","paper":"A3","landscape":true,"margin_mm":10.0},
            "choices":{"views":[["__current__","Current view"],["std:top","Top (plan)"],["scene:Iso","Scene: Iso"]],
                       "styles":[["sombreado","Model style"],["vectorial","Vector"]]},
            "items":[{"key":1,"kind":"frame","rect":[20,20,190,150],"locked":false,"hidden":false,"editable":true,
                      "fields":{"view_key":"std:top","scale_n":100.0,"style":"vectorial","w_mm":190.0,"locked":false}},
                     {"key":2,"kind":"titleblock","rect":[0,0,180,33],"locked":false,"hidden":false,"editable":true,
                      "fields":{"campos":[["PROYECTO","A"],["LÁMINA","L-01"]],"w_mm":180.0}}]}"#).unwrap()
    }

    #[test]
    fn the_sheet_when_nothing_is_selected_and_its_edits() {
        let d = doc();
        let (title, secs) = sections(&d, &[]);
        assert_eq!(title, "Sheet");
        assert!(secs[0].1.iter().any(|(l, v)| l == "Paper" && v.ends_with("A4|A3|A2|A1|A0\u{1d}1")));
        assert_eq!(edit(&d, "sheet:paper", "0").unwrap(), "sheet\u{1f}{\"paper\":\"A4\"}");
        assert_eq!(edit(&d, "sheet:name", "Plans"), None);              // unchanged: nothing
        assert_eq!(edit(&d, "sheet:margin_mm", "12,5 mm").unwrap(), "sheet\u{1f}{\"margin_mm\":12.5}");
    }

    #[test]
    fn a_frame_s_fields_scale_and_source() {
        let d = doc();
        let (title, secs) = sections(&d, &[1]);
        assert_eq!(title, "Model view");
        let scale = &secs[0].1.iter().find(|(l, _)| l == "Scale").unwrap().1;
        assert!(scale.ends_with("1:100"));
        assert_eq!(edit(&d, "1:scale_n", "1:50").unwrap(), "set\u{1f}1\u{1f}{\"scale_n\":50.0}");
        assert_eq!(edit(&d, "1:scale_n", "10:1").unwrap(), "set\u{1f}1\u{1f}{\"scale_n\":0.1}");
        let src = edit(&d, "1:view_key", "2").unwrap();
        assert!(src.contains("\"view_key\":\"scene:Iso\"") && src.contains("\"cam_yaw\":null"), "{src}");
        assert_eq!(edit(&d, "1:refresh", "").unwrap(), "refresh\u{1f}1");
        assert_eq!(edit(&d, "1:w_mm", "abc"), None);
    }

    #[test]
    fn title_block_rows_and_several_items() {
        let d = doc();
        let (_, secs) = sections(&d, &[2]);
        assert_eq!(secs[0].1[0].0, "PROYECTO");
        let e = edit(&d, "2:campos.1", "L-02").unwrap();
        assert!(e.contains("[\"LÁMINA\",\"L-02\"]"), "{e}");
        let (title, secs) = sections(&d, &[1, 2]);
        assert_eq!(title, "2 items");
        assert_eq!(edit(&d, &secs[0].1[0].1.split('\u{1d}').nth(2).unwrap().to_string(), "1").unwrap(),
                   "set_many\u{1f}1\u{1f}{\"locked\":true}\u{1f}2\u{1f}{\"locked\":true}");
        assert_eq!(format_scale(0.5), "2:1");
    }
}
