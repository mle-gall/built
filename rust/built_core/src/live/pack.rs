// SPDX-License-Identifier: GPL-3.0-or-later
//! Portable detail packs (format version 2): identity, provenance, licence,
//! units, dependencies and content hashes, so a junction detail opens, lists
//! and applies in a model that has never seen its source. Packs are data
//! only: nothing here runs, and nothing reaches the network.
use super::*;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

/// What this build reads. A pack that requires anything else is refused.
pub const FEATURES: &[&str] = &[
    "junction.v1",
    "hole.v1",
    "block.v1",
    "fastener.v1",
    "weld.v1",
    "glue.v1",
    "hosted.v1",
];
const MAX_FILES: usize = 4096;
const MAX_FILE: u64 = 16 << 20;
const MAX_TOTAL: u64 = 256 << 20;

fn err(s: impl ToString) -> PyErr {
    PyValueError::new_err(s.to_string())
}

// ---- canonical JSON and hashes ---------------------------------------------------------------

fn canonical_into(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => {
            let keys: BTreeSet<&String> = m.keys().collect();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                canonical_into(&m[k], out);
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical_into(x, out);
            }
            out.push(']');
        }
        Value::Number(n) => {
            let f = n.as_f64().unwrap_or(0.);
            if f.fract() == 0. && f.abs() < 1e15 {
                out.push_str(&format!("{}", f as i64));
            } else {
                out.push_str(&format!("{f}"));
            }
        }
        other => out.push_str(&other.to_string()),
    }
}

/// Sorted keys, no spaces, numbers in their shortest form: the same row
/// gives the same text whoever wrote it.
pub fn canonical(v: &Value) -> String {
    let mut out = String::new();
    canonical_into(v, &mut out);
    out
}

pub fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::from("sha256:");
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// The hash of a detail row: its canonical JSON without the `hash` field
/// itself and without where it was stored (`source`).
pub fn row_hash(row: &Value) -> String {
    let mut row = row.clone();
    if let Some(m) = row.as_object_mut() {
        m.remove("hash");
        m.remove("source");
    }
    sha256(canonical(&row).as_bytes())
}

fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            s.push(c);
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    let s = s.trim_end_matches('-').to_owned();
    if s.is_empty() {
        "detail".into()
    } else {
        s.chars().take(48).collect()
    }
}

// ---- dependencies ----------------------------------------------------------------------------

/// One thing a detail needs besides itself.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Dep {
    pub kind: &'static str,
    pub key: String,
}

fn template_items(p: &Value) -> impl Iterator<Item = &Value> {
    p["values"]["contents"].as_array().into_iter().flatten()
}

/// What a preset row names directly: part presets, child details,
/// fastener tables, components and (for a sweep preset) its profile.
pub fn direct_deps(p: &Value) -> Vec<Dep> {
    let mut out = BTreeSet::new();
    match p["primitive"].as_str() {
        Some("junction") => {
            for item in template_items(p) {
                let part = &item["part"];
                if let Some(uid) = part["preset"].as_str() {
                    out.insert(Dep { kind: "preset", key: uid.into() });
                }
                if let Some(key) = part["sweep"]["profile"]["library"].as_str() {
                    out.insert(Dep { kind: "profile", key: key.into() });
                }
                if let Some(uid) = part["component"].as_str() {
                    out.insert(Dep { kind: "component", key: uid.into() });
                }
                if part.get("fastener").is_some() {
                    out.insert(Dep { kind: "fastener", key: part["fastener"]["table"].as_str().unwrap_or("metric").into() });
                }
                if let Some(uid) = item["junction"]["detail"].as_str() {
                    out.insert(Dep { kind: "detail", key: uid.into() });
                }
            }
        }
        Some("sweep") => {
            let profile = &p["values"]["profile"];
            if let Some(key) = profile["library"].as_str().filter(|k| !k.is_empty()) {
                if profile.get("drawing").is_none() {
                    out.insert(Dep { kind: "profile", key: key.into() });
                }
            }
        }
        _ => {}
    }
    out.into_iter().collect()
}

fn builtin_preset(uid: &str) -> Option<presets::Preset> {
    presets::builtin(uid)
}

/// Whether a dependency is part of every Built: a bundled preset or detail,
/// a catalogue profile, the metric fastener table.
pub fn built_in(dep: &Dep) -> bool {
    match dep.kind {
        "preset" | "detail" => builtin_preset(&dep.key).is_some(),
        "profile" => profile::built_in(&dep.key),
        "fastener" => matches!(dep.key.as_str(), "metric" | "inch"),
        _ => false,
    }
}

// ---- template validation ---------------------------------------------------------------------

fn symbol_names(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::String(s) => {
            if let Some(name) = s.strip_prefix('@') {
                out.insert(name.split('.').next().unwrap_or("").to_owned());
            }
        }
        Value::Array(a) => a.iter().for_each(|x| symbol_names(x, out)),
        Value::Object(m) => m.values().for_each(|x| symbol_names(x, out)),
        _ => {}
    }
}

fn expression_names(v: &Value, out: &mut BTreeSet<String>) -> Result<(), String> {
    match v {
        Value::String(s) if !s.starts_with('@') => {
            if s == "through" { return Ok(()); }
            for n in expr::names(v)? {
                out.insert(n);
            }
            let _ = s;
        }
        Value::Array(a) => {
            for x in a {
                expression_names(x, out)?;
            }
        }
        Value::Object(m) => {
            for (k, x) in m {
                // Free text and keys are not expressions.
                if matches!(
                    k.as_str(),
                    "key" | "on" | "name" | "prompt" | "group" | "unit" | "lock" | "kind"
                        | "operation" | "end" | "face" | "frame" | "type" | "standard"
                        | "grade" | "preset" | "component" | "detail" | "run" | "material"
                        | "table" | "library" | "family" | "catalogue" | "class" | "axis" | "process" | "contact" | "arrangement" | "ref" | "meeting"
                ) {
                    continue;
                }
                expression_names(x, out)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// A detail template refers only to its own roles, frames, content keys and
/// declared dependencies, and its parameters parse without a cycle.
pub fn validate_template(p: &Value, declared: &BTreeSet<Dep>) -> Result<(), String> {
    if p["primitive"] != "junction" {
        return Ok(());
    }
    let values = &p["values"];
    let roles: BTreeSet<String> = values["roles"]
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    let params = values["params"].as_array().cloned().unwrap_or_default();
    expr::validate(&params)?;
    let keys: BTreeSet<String> = template_items(p)
        .filter_map(|i| i["key"].as_str().map(str::to_owned))
        .collect();
    let param_keys: BTreeSet<String> = params
        .iter()
        .filter_map(|d| d["key"].as_str().map(str::to_owned))
        .collect();
    let mut symbols = BTreeSet::new();
    symbol_names(&values["contents"], &mut symbols);
    for s in &symbols {
        if s != "meet" && !roles.contains(s) && !keys.contains(s) {
            return Err(format!("The template refers to {s}, which it does not define"));
        }
    }
    let mut names = BTreeSet::new();
    for item in template_items(p) {
        expression_names(item, &mut names)?;
    }
    for d in &params {
        expression_names(&d["expr"], &mut names)?;
    }
    for n in &names {
        if let Some((role, _)) = n.split_once('.') {
            if role != "ply" && !roles.contains(role) {
                return Err(format!("The template measures {role}, which is not one of its roles"));
            }
        } else if !param_keys.contains(n) && !matches!(n.as_str(), "length" | "area") {
            return Err(format!("Unknown parameter {n}"));
        }
    }
    for dep in direct_deps(p) {
        if !built_in(&dep) && !declared.contains(&dep) {
            return Err(format!("The template needs the undeclared {} {}", dep.kind, dep.key));
        }
    }
    Ok(())
}

// ---- rules in words --------------------------------------------------------------------------

fn profile_words(p: &Value) -> String {
    if let Some(key) = p["library"].as_str() {
        if let Some(label) = profile::label(key) {
            return label.into();
        }
        if let Some(label) = p["label"].as_str() { return label.into(); }
        return key.rsplit('/').next().unwrap_or(key).to_uppercase();
    }
    match p["family"].as_str() {
        Some("i") => "I".into(),
        Some("rhs") => "RHS".into(),
        Some("chs") => "CHS".into(),
        Some(f) => f.into(),
        None => String::new(),
    }
}

fn range(v: &Value) -> Option<String> {
    let a = v.as_array()?;
    let lo = a.first()?.as_f64()?;
    let hi = a.get(1)?.as_f64()?;
    Some(format!("{}–{}", trim(lo), trim(hi)))
}

fn trim(n: f64) -> String {
    if n.fract() == 0. {
        format!("{}", n as i64)
    } else {
        format!("{n:.1}")
    }
}

fn class_words(c: &str) -> String {
    let c = c.strip_prefix("Ifc").unwrap_or(c);
    let c = c.strip_suffix("StandardCase").unwrap_or(c);
    c.to_lowercase()
}

/// Role order: the main or host role first, then the joining or hosted one,
/// then the rest by name.
pub fn role_order(roles: &Value) -> Vec<String> {
    let mut names: Vec<String> = roles.as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
    let rank = |n: &str| match n {
        "main" | "host" | "a" | "ply_1" => 0,
        "joining" | "hosted" | "b" | "ply_2" => 1,
        _ => 2,
    };
    names.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.cmp(b)));
    names
}

/// "HE 200 A 152–240 column flange · IPE 200–300 rafter end · 75° ± 2° · mirrored"
pub fn rule_words(p: &Value) -> String {
    let values = &p["values"];
    let rule = &values["rule"];
    let meeting = values["meeting"].as_str().or(rule["meeting"].as_str()).unwrap_or("");
    let mut parts = Vec::new();
    for role in role_order(&values["roles"]) {
        let r = &rule["roles"][&role];
        let mut words = Vec::new();
        let profiles: Vec<String> = r["profile"]
            .as_array()
            .into_iter()
            .flatten()
            .map(profile_words)
            .filter(|s| !s.is_empty())
            .collect();
        if !profiles.is_empty() {
            words.push(profiles.join(" or "));
        }
        if let Some(size) = r["size"].as_object() {
            for (dim, v) in size {
                if let Some(text) = range(v) {
                    words.push(if size.len() > 1 || !matches!(dim.as_str(), "height" | "thickness") {
                        format!("{dim} {text}")
                    } else {
                        text
                    });
                }
            }
        }
        let name = values["roles"][&role]["name"]
            .as_str()
            .map(str::to_lowercase)
            .or_else(|| {
                r["class"]
                    .as_array()
                    .and_then(|c| c.first())
                    .and_then(Value::as_str)
                    .map(class_words)
            })
            .unwrap_or_else(|| role.replace('_', " "));
        words.push(name);
        if let Some(face) = r["face"].as_array().and_then(|f| f.first()).and_then(Value::as_str) {
            words.push(match face {
                "flange_outer" => "flange".into(),
                "web_left" | "web_right" => "web".into(),
                f => f.replace('_', " "),
            });
        } else if role != "main" && matches!(meeting, "end_face" | "end_end" | "base") {
            words.push("end".into());
        }
        parts.push(words.join(" "));
    }
    let arrow = match meeting {
        "end_face" => " → ",
        "end_end" | "corner" => " ↔ ",
        "base" => " on ",
        "hosted" => " in ",
        "crossing" => " × ",
        _ => " · ",
    };
    let mut text = if meeting == "end_face" || meeting == "base" || meeting == "hosted" {
        parts.reverse();
        parts.join(arrow)
    } else {
        parts.join(arrow)
    };
    if let Some(angle) = rule["angle"].as_object() {
        let value = angle.get("value").and_then(Value::as_f64);
        let tol = angle.get("tol").and_then(Value::as_f64).unwrap_or(2.);
        if let Some(v) = value {
            text.push_str(&format!(" · {}° ± {}°", trim(v), trim(tol)));
        }
    } else if !text.is_empty() {
        text.push_str(match meeting {
            "corner" | "end_end" => " · any angle",
            _ => "",
        });
    }
    if rule["mirror"] != "exact" && rule.is_object() && meeting != "hosted" {
        text.push_str(" · mirrored");
    }
    text
}

/// What a detail adds, in a few words: "plate, 4 bolts, weld".
pub fn adds_words(p: &Value) -> String {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut order = Vec::new();
    let mut bump = |k: &'static str, n: usize| {
        if !counts.contains_key(k) {
            order.push(k);
        }
        *counts.entry(k).or_default() += n;
    };
    for item in template_items(p) {
        let part = &item["part"];
        if item.get("row").is_some() || item["hosted_opening"] == true {
            bump(if item["hosted_opening"] == true { "opening" } else { "cut" }, 1);
        } else if let Some(f) = part.get("fastener") {
            bump(match f["kind"].as_str().unwrap_or("bolt") {
                "screw" => "screw",
                "nail" => "nail",
                "dowel" => "dowel",
                "anchor" => "anchor",
                _ => "bolt",
            }, 1);
        } else if part.get("weld").is_some() {
            bump("weld", 1);
        } else if part.get("glue").is_some() {
            bump("glue", 1);
        } else if part.get("component").is_some() {
            bump("component", 1);
        } else if part.get("preset").is_some() {
            bump("part", 1);
        } else if let Some(j) = item.get("junction") {
            let n = j["repeat"]["rule"]["count"].as_u64().unwrap_or(1) as usize
                * j["repeat"]["nest"]["count"].as_u64().unwrap_or(1) as usize;
            let uid = j["detail"].as_str().unwrap_or("");
            let child = builtin_preset(uid);
            let label = child
                .as_ref()
                .and_then(|c| {
                    let v = json!(c);
                    let w = adds_words(&v);
                    (!w.is_empty()).then_some(w)
                })
                .or_else(|| p["source"]["child_kinds"][uid].as_str().map(str::to_owned))
                .unwrap_or_default();
            let kind: &'static str = if label.contains("screw") {
                "screw"
            } else if label.contains("nail") {
                "nail"
            } else if label.contains("dowel") {
                "dowel"
            } else if label.contains("anchor") {
                "anchor"
            } else if label.contains("weld") {
                "weld"
            } else {
                "bolt"
            };
            bump(kind, n);
        }
    }
    order
        .into_iter()
        .map(|k| {
            let n = counts[k];
            let plural = match k {
                "glue" => "glue".to_owned(),
                _ if n == 1 => k.to_owned(),
                _ => format!("{k}s"),
            };
            if n == 1 {
                plural
            } else {
                format!("{n} {plural}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

// ---- thumbnails ------------------------------------------------------------------------------

/// A detail's thumbnail key: its meeting and what it adds, so details that
/// look alike share one drawing.
pub fn thumb_key(p: &Value) -> String {
    let meeting = p["values"]["meeting"].as_str().unwrap_or("parts");
    let mut flags = BTreeSet::new();
    for item in template_items(p) {
        let part = &item["part"];
        if item["hosted_opening"] == true {
            flags.insert('h');
        } else if let Some(row) = item.get("row") {
            flags.insert(if row["cutter"].get("block").is_some() || row["cutter"].get("body").is_some() {
                'k'
            } else {
                'c'
            });
        } else if let Some(child) = item["junction"]["detail"].as_str() {
            let welds = p["source"]["child_kinds"][child] == "weld" || builtin_preset(child).is_some_and(|c| {
                c.values
                    .get("contents")
                    .and_then(Value::as_array)
                    .is_some_and(|items| items.iter().any(|i| i["part"].get("weld").is_some()))
            });
            flags.insert(if welds { 'w' } else { 'b' });
        } else if part.get("fastener").is_some() {
            flags.insert('b');
        } else if part.get("weld").is_some() {
            flags.insert('w');
        } else if part.get("preset").is_some() || part.get("component").is_some() {
            flags.insert('p');
        }
    }
    format!("junction:{meeting}:{}", flags.into_iter().collect::<String>())
}

/// A schematic drawing of a meeting: grey members, blue parts, dark
/// fasteners, an orange weld. RGBA, premultiplied, `px` square.
pub fn thumbnail(key: &str, px: u32) -> Result<Vec<u8>, &'static str> {
    use tiny_skia::{Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};
    if !(16..=512).contains(&px) {
        return Err("invalid thumbnail size");
    }
    let mut it = key.strip_prefix("junction:").ok_or("not a junction thumbnail")?.split(':');
    let meeting = it.next().unwrap_or("");
    let flags = it.next().unwrap_or("");
    let s = px as f32 / 100.;
    let mut pix = Pixmap::new(px, px).ok_or("invalid thumbnail size")?;
    let fill = |pix: &mut Pixmap, rgb: [u8; 3], x: f32, y: f32, w: f32, h: f32| {
        let mut paint = Paint::default();
        paint.set_color_rgba8(rgb[0], rgb[1], rgb[2], 255);
        paint.anti_alias = true;
        if let Some(r) = Rect::from_xywh(x * s, y * s, w * s, h * s) {
            pix.fill_rect(r, &paint, Transform::identity(), None);
        }
    };
    let member = [150, 156, 162];
    let joining = [196, 200, 203];
    let part = [96, 150, 220];
    let fastener = [40, 44, 52];
    // The members, by meeting.
    let (meet_x, meet_y) = match meeting {
        // A weld: a member standing on a plate; fasteners: two plies.
        "parts" if flags.contains('w') && !flags.contains('b') => {
            fill(&mut pix, member, 10., 72., 80., 12.);
            fill(&mut pix, joining, 38., 10., 24., 62.);
            (50., 72.)
        }
        "parts" => {
            fill(&mut pix, member, 10., 40., 80., 12.);
            fill(&mut pix, joining, 10., 54., 80., 12.);
            (50., 53.)
        }
        "end_face" => {
            fill(&mut pix, member, 14., 8., 16., 84.);
            fill(&mut pix, joining, 34., 40., 58., 20.);
            (32., 50.)
        }
        "end_end" | "corner" => {
            let mut pb = PathBuilder::new();
            pb.move_to(8. * s, 70. * s);
            pb.line_to(50. * s, 28. * s);
            pb.line_to(92. * s, 70. * s);
            if let Some(path) = pb.finish() {
                let mut paint = Paint::default();
                paint.set_color_rgba8(joining[0], joining[1], joining[2], 255);
                paint.anti_alias = true;
                let stroke = Stroke { width: 16. * s, ..Default::default() };
                pix.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
            }
            (50., 28.)
        }
        "crossing" => {
            fill(&mut pix, member, 8., 56., 84., 16.);
            fill(&mut pix, joining, 42., 10., 16., 62.);
            (50., 56.)
        }
        "base" => {
            fill(&mut pix, member, 8., 76., 84., 16.);
            fill(&mut pix, joining, 40., 8., 20., 64.);
            (50., 72.)
        }
        "hosted" => {
            fill(&mut pix, member, 8., 10., 84., 80.);
            fill(&mut pix, [36, 40, 46], 34., 30., 32., 60.);
            (50., 60.)
        }
        _ => {
            fill(&mut pix, member, 10., 40., 80., 20.);
            (50., 50.)
        }
    };
    for flag in flags.chars() {
        match flag {
            'p' => match meeting {
                "hosted" => {
                    fill(&mut pix, part, 30., 26., 40., 4.);
                    fill(&mut pix, part, 30., 26., 4., 64.);
                    fill(&mut pix, part, 66., 26., 4., 64.);
                }
                "base" => fill(&mut pix, part, 26., 70., 48., 6.),
                "crossing" => fill(&mut pix, part, 58., 44., 10., 12.),
                "end_end" | "corner" => fill(&mut pix, part, meet_x - 3., meet_y - 2., 6., 30.),
                _ => fill(&mut pix, part, meet_x - 2., 26., 6., 48.),
            },
            'b' if meeting == "parts" => {
                fill(&mut pix, fastener, 44., 34., 12., 6.);
                fill(&mut pix, fastener, 47., 34., 6., 38.);
                fill(&mut pix, fastener, 44., 66., 12., 6.);
            }
            'w' if meeting == "parts" => {
                fill(&mut pix, [230, 140, 50], 33., 66., 5., 6.);
                fill(&mut pix, [230, 140, 50], 62., 66., 5., 6.);
            }
            'b' => {
                let spots: &[(f32, f32)] = match meeting {
                    "base" => &[(32., 73.), (68., 73.)],
                    "crossing" => &[(63., 48.)],
                    "end_end" | "corner" => &[(meet_x, meet_y + 8.), (meet_x, meet_y + 20.)],
                    _ => &[(meet_x + 1., 32.), (meet_x + 1., 68.)],
                };
                for (x, y) in spots {
                    fill(&mut pix, fastener, x - 3., y - 3., 6., 6.);
                }
            }
            'w' => fill(&mut pix, [230, 140, 50], meet_x + 3., meet_y - 12., 3., 24.),
            'k' => fill(&mut pix, [24, 26, 30], meet_x - 6., meet_y - 4., 12., 8.),
            'c' => fill(&mut pix, [24, 26, 30], meet_x - 1., meet_y - 14., 2., 28.),
            'h' => fill(&mut pix, [24, 26, 30], 36., 32., 28., 58.),
            _ => {}
        }
    }
    Ok(pix.data().to_vec())
}

fn png(rgba: &[u8], px: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut straight = rgba.to_vec();
    for p in straight.chunks_mut(4) {
        let a = p[3] as f32 / 255.;
        if a > 0. {
            for c in &mut p[..3] {
                *c = ((*c as f32 / a).round()).min(255.) as u8;
            }
        }
    }
    if let Some(img) = image::RgbaImage::from_raw(px, px, straight) {
        let _ = img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png);
    }
    out
}

// ---- binding a selection ---------------------------------------------------------------------

fn axis_ends(scene: &Obj<'_>, t: &LiveTable, uid: &str) -> PyResult<Option<(DVec3, DVec3)>> {
    let Some(row) = t.owners.get(uid).and_then(|id| t.rows.get(id)) else {
        return Ok(None);
    };
    let Some(spec) = row.sweep.as_deref() else {
        return Ok(None);
    };
    let Some(path) = reference::sweep_path(scene, t, row, &spec.path)? else {
        return Ok(None);
    };
    let (Some(a), Some(b)) = (path.first(), path.last()) else {
        return Ok(None);
    };
    let m = if spec.path.definition_space {
        reference::frame(t.groups[uid])
    } else {
        DMat4::IDENTITY
    };
    Ok(Some((m.transform_point3((*a).into()), m.transform_point3((*b).into()))))
}

fn segment_distance(p: DVec3, a: DVec3, b: DVec3) -> f64 {
    let ab = b - a;
    let t = if ab.length_squared() > 0. {
        ((p - a).dot(ab) / ab.length_squared()).clamp(0., 1.)
    } else {
        0.
    };
    p.distance(a + ab * t)
}

fn met_face(
    t: &LiveTable,
    main: &str,
    joining: Option<&String>,
    joining_axis: Option<(DVec3, DVec3)>,
    main_axis: Option<(DVec3, DVec3)>,
) -> Option<&'static str> {
    let spec = t.owners.get(main).and_then(|id| t.rows.get(id))?.sweep.as_deref()?;
    let family = spec.profile.outline(spec.segments).ok()?.family;
    if !matches!(family.as_str(), "i" | "channel" | "tee" | "zed") {
        return None;
    }
    let (a, z) = main_axis?;
    let (s, e) = joining_axis?;
    let _ = joining?;
    let dir = (z - a).try_normalize()?;
    // The joining member's run, from the end nearest the main axis.
    let run = if segment_distance(s, a, z) <= segment_distance(e, a, z) { e - s } else { s - e };
    let across = DVec3::Z.cross(dir).try_normalize().unwrap_or(DVec3::X);
    let up = dir.cross(across);
    let (mut u, mut v) = (run.dot(up).abs(), run.dot(across).abs());
    let turned = (spec.rotation.rem_euclid(180.) - 90.).abs() < 45.;
    if turned {
        std::mem::swap(&mut u, &mut v);
    }
    Some(if u >= v { "flange_outer" } else { "web_left" })
}

/// Bind a detail's roles to picked members, in role order (the first pick
/// is the main or host role). Ends are the ones nearest the other member.
pub(super) fn bind(scene: &Obj<'_>, t: &LiveTable, p: &presets::Preset, picks: &[String]) -> PyResult<Value> {
    let roles = role_order(p.values.get("roles").unwrap_or(&Value::Null));
    if roles.len() < 2 {
        return Err(err("This detail has no roles to bind"));
    }
    if picks.len() != roles.len() {
        return Err(err(format!(
            "Select {} members for {} ({} first)",
            roles.len(),
            p.name,
            roles[0].replace('_', " ")
        )));
    }
    let meeting = p.values.get("meeting").and_then(Value::as_str).unwrap_or("");
    let rule = p.values.get("rule").cloned().unwrap_or(Value::Null);
    let mut out = Map::new();
    // Two picks in either order: the joining member is the one whose end
    // meets the other (a base has no axis of its own).
    let mut picks = picks.to_vec();
    if picks.len() == 2 && matches!(meeting, "end_face" | "base") {
        let a = axis_ends(scene, t, &picks[0])?;
        let b = axis_ends(scene, t, &picks[1])?;
        let allowed = rule["roles"][&roles[0]]["class"].as_array();
        let main_class_fits = |uid: &str| -> bool {
            let class = group(scene.py(), t, uid).and_then(|g| g.getattr("ifc").ok())
                .and_then(|r| r.get_item("class").ok()).and_then(|v| v.extract::<String>().ok());
            allowed.is_some_and(|classes| class.is_some_and(|s| classes.contains(&json!(s))))
        };
        let fits = (main_class_fits(&picks[0]), main_class_fits(&picks[1]));
        let swap = if fits.0 != fits.1 { fits.1 } else { match (a, b) {
            (Some(_), None) => meeting == "base",
            (Some((a0, a1)), Some((b0, b1))) => {
                let joins = segment_distance(b0, a0, a1).min(segment_distance(b1, a0, a1));
                let reversed = segment_distance(a0, b0, b1).min(segment_distance(a1, b0, b1));
                reversed + 1e-6 < joins
            }
            _ => false,
        }};
        if swap {
            picks.swap(0, 1);
        }
    }
    let picks = &picks;
    let axes: Vec<_> = picks
        .iter()
        .map(|uid| axis_ends(scene, t, uid))
        .collect::<PyResult<_>>()?;
    for (i, (role, uid)) in roles.iter().zip(picks).enumerate() {
        if !t.groups.contains_key(uid) {
            return Err(err("Unknown member"));
        }
        let mut b = json!({"placement": uid});
        let other = if i == 0 { axes.get(1) } else { axes.first() }.cloned().flatten();
        let needs_end = match meeting {
            "end_face" | "base" => i > 0,
            "end_end" | "corner" => true,
            _ => false,
        };
        // A base or plain element has no axis: its placement stands for it.
        let other = other.or_else(|| {
            let k = picks.get(usize::from(i == 0)).and_then(|uid| t.groups.get(uid))?;
            let p = reference::frame(*k).w_axis.truncate();
            Some((p, p))
        });
        if let (true, Some((a, z)), Some((oa, oz))) = (needs_end, axes[i], other) {
            let da = segment_distance(a, oa, oz);
            let dz = segment_distance(z, oa, oz);
            b["end"] = json!(if da < dz { "start" } else { "end" });
        }
        if let Some(face) = rule["roles"][role]["face"]
            .as_array()
            .and_then(|f| f.first())
            .and_then(Value::as_str)
        {
            if i == 0 && matches!(meeting, "end_face" | "crossing") {
                b["face"] = json!(face);
            }
        }
        // An I or channel main member is met on a flange or on its web: the
        // side the joining member runs off to (bolt grips read that face).
        if i == 0 && meeting == "end_face" && b.get("face").is_none() {
            if let Some(face) = met_face(t, uid, picks.get(1), axes.get(1).copied().flatten(), axes[0]) {
                b["face"] = json!(face);
            }
        }
        // A base is met on its top, whichever end of the member stands on it.
        if i == 0 && meeting == "base" && b.get("face").is_none() {
            b["face"] = json!("top");
        }
        out.insert(role.clone(), b);
    }
    Ok(Value::Object(out))
}

// ---- writing a pack --------------------------------------------------------------------------

fn zip_stored(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    // 2026-01-01 00:00: a fixed stamp keeps the archive reproducible.
    let (time, date) = (0u16, ((2026 - 1980) << 9 | 1 << 5 | 1) as u16);
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let crc = crc32fast::hash(data);
        let offset = out.len() as u32;
        let mut local = Vec::new();
        local.extend_from_slice(&0x04034b50u32.to_le_bytes());
        local.extend_from_slice(&20u16.to_le_bytes());
        local.extend_from_slice(&0x0800u16.to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(&time.to_le_bytes());
        local.extend_from_slice(&date.to_le_bytes());
        local.extend_from_slice(&crc.to_le_bytes());
        local.extend_from_slice(&(data.len() as u32).to_le_bytes());
        local.extend_from_slice(&(data.len() as u32).to_le_bytes());
        local.extend_from_slice(&(name.len() as u16).to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&local);
        out.extend_from_slice(data);
        central.extend_from_slice(&0x02014b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&local[4..30]);
        central.extend_from_slice(&[0; 6]);
        central.extend_from_slice(&(0o100644u32 << 16).to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let start = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x06054b50u32.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// The pack's meta: id, name, version, author, licence, units, requires.
fn pack_meta(meta: &Value, features: &BTreeSet<&'static str>) -> Result<Value, String> {
    let name = meta["name"].as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or("Name the pack")?;
    let id = match meta["id"].as_str() {
        Some(id) if id.len() == 32 && id.bytes().all(|c| c.is_ascii_hexdigit()) => id.to_lowercase(),
        Some(_) => return Err("A pack id is 32 hex digits".into()),
        None => eid::new_uid() + &eid::new_uid(),
    };
    let version = meta["version"].as_str().unwrap_or("1.0.0");
    if version.is_empty() || version.len() > 32 {
        return Err("Invalid pack version".into());
    }
    let mut out = json!({"id": id, "name": name, "version": version,
                         "units": {"length": "mm", "angle": "deg"},
                         "requires": {"built": ">= 1.0", "features": features.iter().collect::<Vec<_>>()}});
    let author = &meta["author"];
    if let Some(a) = author.as_str().filter(|s| !s.trim().is_empty()) {
        out["author"] = json!({"name": a.trim()});
    } else if author["name"].as_str().is_some_and(|s| !s.trim().is_empty()) {
        out["author"] = json!({"name": author["name"].as_str().unwrap().trim()});
        if let Some(url) = author["url"].as_str().filter(|u| !u.trim().is_empty()) {
            out["author"]["url"] = json!(url.trim());
        }
    }
    if let Some(l) = meta["licence"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
        if !l.chars().all(|c| c.is_ascii_alphanumeric() || "-.+() ".contains(c)) {
            return Err("Give the licence as an SPDX id, such as CC-BY-4.0".into());
        }
        out["licence"] = json!(l);
    } else if meta["publish"] == true {
        return Err("Choose a licence to publish the pack".into());
    }
    if let Some(d) = meta["description"].as_str().filter(|s| !s.is_empty()) {
        out["description"] = json!(d);
    }
    Ok(out)
}

fn features_of(p: &Value, out: &mut BTreeSet<&'static str>) {
    if p["primitive"] == "junction" {
        out.insert("junction.v1");
    }
    let text = p["values"].to_string();
    for (needle, feature) in [
        ("\"hole\"", "hole.v1"),
        ("\"block\"", "block.v1"),
        ("\"fastener\"", "fastener.v1"),
        ("\"weld\"", "weld.v1"),
        ("\"glue\"", "glue.v1"),
        ("\"hosted_opening\"", "hosted.v1"),
    ] {
        if text.contains(needle) {
            out.insert(feature);
        }
    }
}

/// What `export` needs from its caller: the presets it can look up besides
/// the bundled ones, and how to write a component's file.
pub struct Sources<'a> {
    pub rows: &'a [Value],
    pub component: &'a dyn Fn(&str) -> PyResult<Option<Vec<u8>>>,
}

fn find_row(src: &Sources<'_>, uid: &str) -> Option<Value> {
    src.rows.iter().find(|r| r["uid"] == uid).cloned()
}

fn part_revision(row: &Value, key: &str) -> Value {
    row["source"]["components"][key].get("rev").cloned()
        .or_else(|| row["depends"].as_array()?.iter()
            .find(|d| d["kind"] == "component" && d["key"] == key)?.get("rev").cloned())
        .unwrap_or(json!(1))
}

/// Write a pack of `details` (preset rows) and everything they need that is
/// not built in. Returns pack.json as written and the files.
pub fn export(details: &[Value], meta: &Value, src: &Sources<'_>, created_with: &str) -> PyResult<(Value, Vec<(String, Vec<u8>)>)> {
    if details.is_empty() {
        return Err(err("Choose at least one detail"));
    }
    let mut rows: Vec<(String, Value)> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut queue: Vec<(Value, bool)> = details.iter().map(|d| (d.clone(), true)).collect();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut used_paths = BTreeSet::new();
    let mut features = BTreeSet::new();
    while let Some((row, top)) = queue.pop() {
        let uid = row["uid"].as_str().unwrap_or("").to_lowercase();
        if uid.len() != 16 || !uid.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(err("A detail has no valid id"));
        }
        if !seen.insert(uid.clone()) {
            continue;
        }
        features_of(&row, &mut features);
        let mut depends = Vec::new();
        for dep in direct_deps(&row) {
            let mut entry = json!({"kind": dep.kind, "key": dep.key, "bundled": null});
            if built_in(&dep) {
                if let Some(p) = builtin_preset(&dep.key) {
                    entry["rev"] = json!(p.rev);
                }
                depends.push(entry);
                continue;
            }
            match dep.kind {
                "preset" | "detail" => {
                    let child = find_row(src, &dep.key)
                        .ok_or_else(|| err(format!("{} needs {} {}, which is not available", row["name"].as_str().unwrap_or("A detail"), dep.kind, dep.key)))?;
                    entry["rev"] = child["rev"].clone();
                    queue.push((child, false));
                    entry["bundled"] = json!(format!("@{}", dep.key));
                }
                "profile" => {
                    // A size outside the catalogue travels as a one-size table.
                    let profile = &row["values"]["profile"];
                    if !profile["family"].is_string() {
                        return Err(err(format!("Profile {} is not in this Built's catalogue and has no family", dep.key)));
                    }
                    let path = format!("profiles/{}.builtprofile", slug(&dep.key));
                    let table = json!({"format":"built.profile","v":1,"use":"profile","name":dep.key,
                        "table":{"sizes":[{"key":dep.key,"label":dep.key,"family":profile["family"],"params":profile["params"]}]}});
                    let bytes = serde_json::to_vec_pretty(&table).map_err(err)?;
                    if used_paths.insert(path.clone()) {
                        files.push((path.clone(), bytes));
                    }
                    entry["bundled"] = json!(path);
                }
                "component" => {
                    let bytes = (src.component)(&dep.key)?
                        .ok_or_else(|| err(format!("Component {} is not in this model", dep.key)))?;
                    let path = format!("components/{}.built", dep.key);
                    entry["hash"] = json!(sha256(&bytes));
                    entry["rev"] = part_revision(&row, &dep.key);
                    if used_paths.insert(path.clone()) {
                        files.push((path.clone(), bytes));
                    }
                    entry["bundled"] = json!(path);
                }
                _ => {}
            }
            depends.push(entry);
        }
        let declared: BTreeSet<Dep> = direct_deps(&row).into_iter().collect();
        validate_template(&row, &declared)
            .map_err(|e| err(format!("{}: {e}", row["name"].as_str().unwrap_or("A detail"))))?;
        let folder = if row["primitive"] == "junction" { "details" } else { "parts" };
        let mut path = format!("{folder}/{}.builtpreset", slug(row["name"].as_str().unwrap_or("detail")));
        let mut n = 2;
        while !used_paths.insert(path.clone()) {
            path = format!("{folder}/{}-{n}.builtpreset", slug(row["name"].as_str().unwrap_or("detail")));
            n += 1;
        }
        let mut out = row.clone();
        let obj = out.as_object_mut().ok_or_else(|| err("A detail is not an object"))?;
        for k in ["source", "needs", "hash"] {
            obj.remove(k);
        }
        obj.insert("uid".into(), json!(uid));
        obj.insert("rev".into(), json!(row["rev"].as_u64().unwrap_or(1).max(1)));
        obj.insert("depends".into(), json!(depends));
        if !obj.contains_key("provenance") {
            let derived = row["source"]["pack"].as_str().map(|pack| {
                json!({"pack": pack, "uid": uid, "rev": row["source"]["rev"]})
            });
            obj.insert(
                "provenance".into(),
                json!({"created": created_with.split('|').nth(1).unwrap_or(""), "created_with": created_with.split('|').next().unwrap_or("Built"), "derived_from": derived}),
            );
        }
        if top || row["primitive"] == "junction" {
            let thumb = format!("thumbnails/{}.png", slug(row["name"].as_str().unwrap_or("detail")));
            if row["primitive"] == "junction" && used_paths.insert(thumb.clone()) {
                if let Ok(rgba) = thumbnail(&thumb_key(&row), 128) {
                    files.push((thumb.clone(), png(&rgba, 128)));
                    obj.insert("thumbnail".into(), json!(thumb));
                }
            }
        }
        let hash = row_hash(&out);
        out["hash"] = json!(hash);
        rows.push((path, out));
    }
    // Replace "@uid" bundled markers by the rows' paths.
    let paths: BTreeMap<String, String> = rows
        .iter()
        .map(|(p, r)| (r["uid"].as_str().unwrap_or("").to_owned(), p.clone()))
        .collect();
    for (_, row) in &mut rows {
        let mut changed = false;
        for d in row["depends"].as_array_mut().into_iter().flatten() {
            if let Some(uid) = d["bundled"].as_str().and_then(|s| s.strip_prefix('@')).map(str::to_owned) {
                d["bundled"] = json!(paths.get(&uid));
                changed = true;
            }
        }
        if changed {
            let hash = row_hash(row);
            row["hash"] = json!(hash);
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let mut meta = pack_meta(meta, &features).map_err(err)?;
    for (path, row) in &rows {
        files.push((path.clone(), serde_json::to_vec_pretty(row).map_err(err)?));
    }
    let mut hashes = Map::new();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    for (path, bytes) in &files {
        hashes.insert(path.clone(), json!(sha256(bytes)));
    }
    meta["files"] = Value::Object(hashes);
    let manifest = json!({"format": "built.presets", "v": 2, "pack": meta,
                          "rows": rows.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>()});
    Ok((manifest, files))
}

/// Write the pack: a folder (pack.json beside its files) or, for a
/// `.builtpack` destination, the same folder zipped.
pub fn write(dest: &Path, manifest: &Value, files: &[(String, Vec<u8>)]) -> std::io::Result<PathBuf> {
    let json = serde_json::to_vec_pretty(manifest).map_err(std::io::Error::other)?;
    if dest.extension().is_some_and(|e| e == "builtpack") {
        let mut entries = vec![("pack.json".to_owned(), json)];
        entries.extend(files.iter().cloned());
        write_atomic(dest, &zip_stored(&entries))?;
        return Ok(dest.to_owned());
    }
    if dest.join("pack.json").exists() {
        return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "a pack is already there"));
    }
    for (path, bytes) in files {
        write_atomic(&dest.join(path), bytes)?;
    }
    write_atomic(&dest.join("pack.json"), &json)?;
    Ok(dest.join("pack.json"))
}

// ---- reading a pack --------------------------------------------------------------------------

fn safe_path(p: &str) -> bool {
    !p.is_empty()
        && p.len() < 256
        && !p.starts_with('/')
        && !p.contains('\\')
        && Path::new(p).components().all(|c| matches!(c, Component::Normal(_)))
}

/// The files of a pack, by path, read from a folder or a zip.
pub trait Files {
    fn read(&self, path: &str) -> PyResult<Option<Vec<u8>>>;
}

pub struct Folder(pub PathBuf);
impl Files for Folder {
    fn read(&self, path: &str) -> PyResult<Option<Vec<u8>>> {
        let p = self.0.join(path);
        match std::fs::metadata(&p) {
            Ok(m) if m.len() > MAX_FILE => Err(err(format!("{path} is too large"))),
            Ok(_) => Ok(Some(std::fs::read(p)?)),
            Err(_) => Ok(None),
        }
    }
}

/// The plan for importing a pack: every row checked, its dependencies
/// resolved, nothing written yet.
pub struct Plan {
    pub manifest: Value,
    pub rows: Vec<Value>,
    pub files: BTreeMap<String, Vec<u8>>,
}

/// What the Library already holds: preset uid → hash (or "" when unknown).
pub type Known = BTreeMap<String, String>;

pub fn read(files: &dyn Files, known: &Known) -> PyResult<Plan> {
    let bytes = files.read("pack.json")?.ok_or_else(|| err("This folder has no pack.json"))?;
    let manifest: Value = serde_json::from_slice(&bytes).map_err(|e| err(format!("pack.json: {e}")))?;
    if manifest["format"] != "built.presets" {
        return Err(err("Not a Built preset pack"));
    }
    let v = manifest["v"].as_u64().unwrap_or(0);
    if v == 1 {
        return read_v1(&manifest, known);
    }
    if v != 2 {
        return Err(err(format!("Pack format version {v} needs a newer Built")));
    }
    let pack = &manifest["pack"];
    for f in pack["requires"]["features"].as_array().into_iter().flatten() {
        let f = f.as_str().unwrap_or("");
        if !FEATURES.contains(&f) {
            return Err(err(format!("This pack needs a newer Built ({f})")));
        }
    }
    let length = pack["units"]["length"].as_str().unwrap_or("mm");
    if length != "mm" {
        return Err(err(format!("Packs in {length} are not read yet: lengths must be millimetres")));
    }
    let id = pack["id"].as_str().unwrap_or("");
    if id.len() != 32 || !id.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(err("The pack has no valid id"));
    }
    let empty = Map::new();
    let inline = manifest["rows"].as_array().is_some_and(|r| r.iter().all(Value::is_object));
    let listed = match pack["files"].as_object() {
        Some(files) => files,
        None if inline => &empty,
        None => return Err(err("The pack lists no files")),
    };
    if listed.len() > MAX_FILES {
        return Err(err("The pack has too many files"));
    }
    let mut contents = BTreeMap::new();
    let mut total = 0u64;
    for (path, hash) in listed {
        if !safe_path(path) {
            return Err(err(format!("Unsafe path in the pack: {path}")));
        }
        let data = files.read(path)?.ok_or_else(|| err(format!("{path} is missing from the pack")))?;
        total += data.len() as u64;
        if total > MAX_TOTAL {
            return Err(err("The pack is too large"));
        }
        if hash.as_str() != Some(sha256(&data).as_str()) {
            return Err(err(format!("{path} does not match its hash: the pack was changed after it was written")));
        }
        contents.insert(path.clone(), data);
    }
    let mut rows = Vec::new();
    for entry in manifest["rows"].as_array().into_iter().flatten() {
        let (path, mut row) = if entry.is_object() {
            ("pack.json", entry.clone())
        } else {
            let path = entry.as_str().ok_or_else(|| err("A row is not a path"))?;
            let data = contents.get(path).ok_or_else(|| err(format!("{path} is not listed with a hash")))?;
            (path, serde_json::from_slice::<Value>(data).map_err(|e| err(format!("{path}: {e}")))?)
        };
        if row["hash"].as_str() != Some(row_hash(&row).as_str()) {
            return Err(err(format!("{path} does not match its hash")));
        }
        presets_row_check(&row).map_err(|e| err(format!("{path}: {e}")))?;
        row["source"] = json!({"pack": id, "name": pack["name"], "version": pack["version"],
                               "rev": row["rev"], "hash": row["hash"], "path": path});
        if row["profile_size"] == true {
            row["source"]["profile_size"] = json!(true);
        }
        for key in ["author", "licence"] {
            if row.get(key).is_none() {
                if let Some(v) = pack.get(key) {
                    row[key] = v.clone();
                }
            }
        }
        rows.push(row);
    }
    if rows.is_empty() {
        return Err(err("The pack has no details"));
    }
    // Bundled profile sizes are carried into the presets that use them.
    for row in &mut rows {
        for d in row["depends"].clone().as_array().into_iter().flatten() {
            if d["kind"] == "profile" {
                if let Some(path) = d["bundled"].as_str() {
                    let data = contents.get(path).ok_or_else(|| err(format!("{path} is missing")))?;
                    let table: Value = serde_json::from_slice(data).map_err(|e| err(format!("{path}: {e}")))?;
                    let size = table["table"]["sizes"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|s| s["key"] == d["key"])
                        .ok_or_else(|| err(format!("{path} has no size {}", d["key"])))?;
                    if !size["family"].as_str().is_some_and(profile::is_family) {
                        return Err(err(format!("{path}: unknown profile family")));
                    }
                    let profile = &mut row["values"]["profile"];
                    profile["family"] = size["family"].clone();
                    let mut params = size["params"].clone();
                    for (k, v) in profile["params"].as_object().cloned().unwrap_or_default() {
                        params[&k] = v;
                    }
                    profile["params"] = params;
                }
            }
        }
    }
    let child_kinds: BTreeMap<_, _> = rows.iter().filter_map(|r| {
        let part = &template_items(r).next()?["part"];
        let kind = if part.get("weld").is_some() { "weld" }
            else { part.get("fastener")?.get("kind").and_then(Value::as_str).unwrap_or("bolt") };
        Some((r["uid"].as_str()?.to_owned(), kind.to_owned()))
    }).collect();
    for row in &mut rows {
        let children: Map<_, _> = template_items(row).filter_map(|item| {
            let uid = item["junction"]["detail"].as_str()?;
            Some((uid.to_owned(), json!(child_kinds.get(uid)?)))
        }).collect();
        if !children.is_empty() {
            row["source"]["child_kinds"] = Value::Object(children);
        }
    }
    resolve(&mut rows, &contents, known)?;
    Ok(Plan { manifest, rows, files: contents })
}

fn read_v1(manifest: &Value, known: &Known) -> PyResult<Plan> {
    let mut rows: Vec<Value> = manifest["rows"].as_array().cloned().unwrap_or_default();
    for row in &mut rows {
        presets_row_check(row).map_err(err)?;
        let deps: Vec<Value> = direct_deps(row)
            .into_iter()
            .map(|d| json!({"kind": d.kind, "key": d.key, "bundled": null}))
            .collect();
        row["depends"] = json!(deps);
    }
    resolve(&mut rows, &BTreeMap::new(), known)?;
    Ok(Plan { manifest: manifest.clone(), rows, files: BTreeMap::new() })
}

fn presets_row_check(row: &Value) -> Result<(), String> {
    let p: presets::Preset = serde_json::from_value(row.clone()).map_err(|e| e.to_string())?;
    p.check().map_err(|e| e.to_string())?;
    if p.primitive == "sweep" {
        let profile = &row["values"]["profile"];
        if let Some(key) = profile["library"].as_str().filter(|k| !k.is_empty()) {
            if !profile::built_in(key) && profile.get("drawing").is_none() && !profile["family"].is_string() {
                // Resolved from the pack's bundled table, or reported missing.
                return Ok(());
            }
        }
    }
    Ok(())
}

/// Mark each row ok or with what it needs; validate templates. A row whose
/// dependency is missing, or depends on such a row, needs it too.
fn resolve(rows: &mut [Value], contents: &BTreeMap<String, Vec<u8>>, known: &Known) -> PyResult<()> {
    let in_pack: BTreeSet<String> = rows.iter().filter_map(|r| r["uid"].as_str().map(str::to_lowercase)).collect();
    let mut needs: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows.iter() {
        let uid = row["uid"].as_str().unwrap_or("").to_lowercase();
        let declared: BTreeSet<Dep> = row["depends"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|d| {
                let kind = match d["kind"].as_str()? {
                    "profile" => "profile",
                    "fastener" => "fastener",
                    "component" => "component",
                    "detail" => "detail",
                    "preset" => "preset",
                    "material" => "material",
                    _ => return None,
                };
                Some(Dep { kind, key: d["key"].as_str()?.to_owned() })
            })
            .collect();
        validate_template(row, &declared).map_err(|e| {
            err(format!("{}: {e}", row["name"].as_str().unwrap_or("A detail")))
        })?;
        let mut missing = Vec::new();
        for dep in &declared {
            let bundled = row["depends"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|d| d["kind"] == dep.kind && d["key"] == dep.key.as_str())
                .and_then(|d| d["bundled"].as_str());
            let ok = built_in(dep)
                || match dep.kind {
                    "preset" | "detail" => in_pack.contains(&dep.key.to_lowercase()) || known.contains_key(&dep.key.to_lowercase()),
                    "profile" => row["values"]["profile"]["family"].is_string(),
                    "component" => bundled.is_some_and(|p| contents.contains_key(p)) || known.contains_key(&format!("component:{}", dep.key)),
                    _ => false,
                };
            if !ok {
                missing.push(match dep.kind {
                    "profile" => format!("profile {}", dep.key),
                    "fastener" => format!("fastener table {}", dep.key),
                    "component" => format!("component {}", dep.key),
                    kind => format!("{kind} {}", dep.key),
                });
            }
        }
        needs.insert(uid, missing);
    }
    // A row that needs a row that needs something, needs it too.
    loop {
        let mut changed = false;
        for row in rows.iter() {
            let uid = row["uid"].as_str().unwrap_or("").to_lowercase();
            for d in row["depends"].as_array().into_iter().flatten() {
                let key = d["key"].as_str().unwrap_or("").to_lowercase();
                if matches!(d["kind"].as_str(), Some("preset" | "detail")) {
                    let inherited = needs.get(&key).cloned().unwrap_or_default();
                    let mine = needs.entry(uid.clone()).or_default();
                    for n in inherited {
                        if !mine.contains(&n) {
                            mine.push(n);
                            changed = true;
                        }
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    for row in rows.iter_mut() {
        let uid = row["uid"].as_str().unwrap_or("").to_lowercase();
        let n = needs.remove(&uid).unwrap_or_default();
        if n.is_empty() {
            if let Some(m) = row.as_object_mut() {
                m.remove("needs");
            }
        } else {
            row["needs"] = json!(n);
        }
    }
    Ok(())
}

// ---- Python ----------------------------------------------------------------------------------

fn to_value(v: &Obj<'_>) -> PyResult<Value> {
    let raw: String = v.py().import("json")?.call_method1("dumps", (v,))?.extract()?;
    serde_json::from_str(&raw).map_err(err)
}

fn zip_files(py: Python<'_>, path: &Path) -> PyResult<BTreeMap<String, Vec<u8>>> {
    let zipfile = py.import("zipfile")?;
    let archive = zipfile.call_method1("ZipFile", (path,))?;
    crate::archive_limits::check_archive(&archive, "the pack")?;
    let mut out = BTreeMap::new();
    let mut total = 0u64;
    for info in archive.call_method0("infolist")?.try_iter()? {
        let info = info?;
        let name: String = info.getattr("filename")?.extract()?;
        if name.ends_with('/') {
            continue;
        }
        if !safe_path(&name) {
            return Err(err(format!("Unsafe path in the pack: {name}")));
        }
        let size: u64 = info.getattr("file_size")?.extract()?;
        total += size;
        if size > MAX_FILE || total > MAX_TOTAL || out.len() >= MAX_FILES {
            return Err(err("The pack is too large"));
        }
        let data: Vec<u8> = crate::archive_limits::read_member(&archive, &name)?.extract()?;
        out.insert(name, data);
    }
    archive.call_method0("close")?;
    Ok(out)
}

struct Zipped(BTreeMap<String, Vec<u8>>);
impl Files for Zipped {
    fn read(&self, path: &str) -> PyResult<Option<Vec<u8>>> {
        Ok(self.0.get(path).cloned())
    }
}

/// Read and check a pack (folder, pack.json or .builtpack) without writing
/// anything: `{"pack", "rows": [row with "needs"?], "components": {uid: bytes}}`.
#[pyfunction]
#[pyo3(signature=(path, known=None))]
pub(super) fn pack_read<'p>(py: Python<'p>, path: PathBuf, known: Option<&Obj<'p>>) -> PyResult<Obj<'p>> {
    let known: Known = match known {
        Some(k) if !k.is_none() => serde_json::from_value(to_value(k)?).map_err(err)?,
        _ => Known::new(),
    };
    let plan = if path.extension().is_some_and(|e| e == "builtpack") {
        read(&Zipped(zip_files(py, &path)?), &known)?
    } else {
        let folder = if path.is_dir() { path.clone() } else { path.parent().map(Path::to_owned).unwrap_or_default() };
        if !path.is_dir() && path.file_name().is_some_and(|n| n != "pack.json") {
            // A single .builtpreset row, as before.
            let row: Value = serde_json::from_slice(&std::fs::read(&path)?).map_err(|e| err(e.to_string()))?;
            let manifest = if row["format"] == "built.presets" { row } else { json!({"format":"built.presets","v":1,"rows":[row]}) };
            if manifest["v"] == 2 {
                // An inline format 2 pack has the same validation as pack.json.
                let files = Zipped(BTreeMap::from([("pack.json".into(),
                    serde_json::to_vec(&manifest).map_err(err)?)]));
                read(&files, &known)?
            } else {
                read_v1(&manifest, &known)?
            }
        } else {
            read(&Folder(folder), &known)?
        }
    };
    let components: BTreeMap<String, Value> = plan
        .rows
        .iter()
        .flat_map(|r| r["depends"].as_array().cloned().unwrap_or_default())
        .filter(|d| d["kind"] == "component")
        .filter_map(|d| {
            let path = d["bundled"].as_str()?;
            Some((d["key"].as_str()?.to_owned(), json!(path)))
        })
        .collect();
    let out = pyo3::types::PyDict::new(py);
    out.set_item("pack", to_py(py, &plan.manifest["pack"])?)?;
    out.set_item("v", plan.manifest["v"].as_u64().unwrap_or(1))?;
    let mut rows = plan.rows;
    // Keep the verified bundle with the library row until its first application,
    // including packs imported directly into a model.
    for row in &mut rows {
        for dep in row["depends"].as_array().cloned().unwrap_or_default() {
            if dep["kind"] != "component" { continue; }
            let Some((uid, bytes)) = dep["key"].as_str().zip(
                dep["bundled"].as_str().and_then(|p| plan.files.get(p))) else { continue };
            let encoded: String = py.import("base64")?.call_method1("b64encode",
                (pyo3::types::PyBytes::new(py, bytes),))?.call_method1("decode", ("ascii",))?.extract()?;
            row["source"]["components"][uid] = json!({"rev":dep["rev"], "hash":sha256(bytes), "data":encoded});
        }
    }
    out.set_item("rows", to_py(py, &rows)?)?;
    let bundled = pyo3::types::PyDict::new(py);
    for (uid, path) in components {
        if let Some(bytes) = path.as_str().and_then(|p| plan.files.get(p)) {
            bundled.set_item(uid, pyo3::types::PyBytes::new(py, bytes))?;
        }
    }
    out.set_item("components", bundled)?;
    Ok(out.into_any())
}

/// Write a pack of `details` to `dest` (a folder, or a `.builtpack`).
/// `rows` are the other presets the details may need; `scene` provides
/// component definitions.
#[pyfunction]
#[pyo3(signature=(details, meta, dest, rows=None, scene=None))]
pub(super) fn pack_export<'p>(
    py: Python<'p>,
    details: &Obj<'p>,
    meta: &Obj<'p>,
    dest: PathBuf,
    rows: Option<&Obj<'p>>,
    scene: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let details: Vec<Value> = serde_json::from_value(to_value(details)?).map_err(err)?;
    let meta = to_value(meta)?;
    let mut lookup: Vec<Value> = match rows {
        Some(r) if !r.is_none() => serde_json::from_value(to_value(r)?).map_err(err)?,
        _ => Vec::new(),
    };
    if let Some(scene) = scene {
        if let Some(t) = current(py, root(scene)?) {
            lookup.extend(t.presets.values().map(|p| json!(p)));
        }
    }
    lookup.extend(details.iter().cloned());
    let component = |uid: &str| -> PyResult<Option<Vec<u8>>> {
        for row in &lookup {
            let bundle = &row["source"]["components"][uid];
            if let Some(data) = bundle["data"].as_str() {
                let bytes: Vec<u8> = py.import("base64")?.call_method1("b64decode", (data,))?.extract()?;
                if bundle["hash"] != sha256(&bytes) {
                    return Err(err("Bundled component hash mismatch"));
                }
                return Ok(Some(bytes));
            }
        }
        let Some(scene) = scene else { return Ok(None) };
        let definition = match current(py, root(scene)?).and_then(|t| group(py, &t, uid)) {
            Some(g) => Some(g),
            None => match cut::source(scene, uid)? {
                Some(definition) => Some(definition),
                None => scene.call_method0("groups_by_uid")?.cast_into::<PyDict>()?.get_item(uid)?,
            }
        };
        let Some(definition) = definition else { return Ok(None) };
        let tmp = py.import("tempfile")?.call_method0("mkdtemp")?.extract::<PathBuf>()?;
        let file = tmp.join("component.built");
        let kc = py.import("built_core.kept_components")?;
        let template = kc.call_method1("make_template", (&definition,))?;
        let copy = kc.call_method1("place_copy", (template,))?;
        copy.setattr("uid", uid)?;
        let doc = py.import("built_core.scene")?.call_method0("Scene")?;
        doc.getattr("groups")?.call_method1("append", (copy,))?;
        let kw = pyo3::types::PyDict::new(py);
        kw.set_item("backup", false)?;
        py.import("built_core.formats.igz")?
            .getattr("save_scene")?
            .call((doc, py.import("pathlib")?.call_method1("Path", (&file,))?), Some(&kw))?;
        let bytes = std::fs::read(&file)?;
        let _ = std::fs::remove_dir_all(&tmp);
        Ok(Some(bytes))
    };
    let src = Sources { rows: &lookup, component: &component };
    let stamp: String = py
        .import("time")?
        .call_method1("strftime", ("%Y-%m-%d",))?
        .extract()?;
    let (manifest, files) = export(&details, &meta, &src, &format!("Built|{stamp}"))?;
    let written = write(&dest, &manifest, &files)?;
    let out = pyo3::types::PyDict::new(py);
    out.set_item("path", written)?;
    out.set_item("pack", to_py(py, &manifest["pack"])?)?;
    out.set_item("rows", manifest["rows"].as_array().map_or(0, Vec::len))?;
    Ok(out.into_any())
}

/// The detail and every preset or child detail it needs that is not built
/// in, found among `rows`: what `junction_apply` embeds with it.
pub fn closure(detail: &Value, rows: &[Value]) -> Result<Vec<Value>, String> {
    let mut out: Vec<Value> = Vec::new();
    let mut queue = vec![detail.clone()];
    while let Some(row) = queue.pop() {
        if out.iter().any(|r| r["uid"] == row["uid"]) {
            continue;
        }
        if let Some(needs) = row["needs"].as_array().filter(|n| !n.is_empty()) {
            let words: Vec<&str> = needs.iter().filter_map(Value::as_str).collect();
            return Err(format!("{} needs {}", row["name"].as_str().unwrap_or("This detail"), words.join(", ")));
        }
        for dep in direct_deps(&row) {
            if built_in(&dep) || !matches!(dep.kind, "preset" | "detail") {
                continue;
            }
            let child = rows
                .iter()
                .find(|r| r["uid"] == dep.key.as_str())
                .ok_or_else(|| format!("{} needs {} {}", row["name"].as_str().unwrap_or("This detail"), dep.kind, dep.key))?;
            queue.push(child.clone());
        }
        out.push(row);
    }
    Ok(out)
}

#[pyfunction]
pub(super) fn junction_definitions<'p>(py: Python<'p>, detail: &Obj<'p>, rows: &Obj<'p>) -> PyResult<Obj<'p>> {
    let rows: Vec<Value> = serde_json::from_value(to_value(rows)?).map_err(err)?;
    to_py(py, &closure(&to_value(detail)?, &rows).map_err(err)?)
}

#[pyfunction]
pub(super) fn pack_row_hash(row: &Obj<'_>) -> PyResult<String> {
    Ok(row_hash(&to_value(row)?))
}

/// "HE 200 A column flange ← IPE rafter end · 75° ± 2°", and what it adds.
#[pyfunction]
pub(super) fn junction_words<'p>(py: Python<'p>, preset: &Obj<'p>) -> PyResult<Obj<'p>> {
    let p = to_value(preset)?;
    to_py(py, &json!({"rule": rule_words(&p), "adds": adds_words(&p), "thumb": thumb_key(&p),
                      "roles": role_order(&p["values"]["roles"])}))
}

#[pyfunction]
pub(super) fn junction_thumbnail<'p>(py: Python<'p>, key: &str, pixels: u32) -> PyResult<Bound<'p, pyo3::types::PyBytes>> {
    let rgba = thumbnail(key, pixels).map_err(err)?;
    Ok(pyo3::types::PyBytes::new(py, &rgba))
}

/// Bind a detail's roles to picked members (main first), for Apply….
#[pyfunction]
#[pyo3(signature=(scene, detail, placements))]
pub(super) fn junction_bind<'p>(scene: &Obj<'p>, detail: &Obj<'p>, placements: Vec<String>) -> PyResult<Obj<'p>> {
    let t = current(scene.py(), root(scene)?).unwrap_or_default();
    let p: presets::Preset = if let Ok(uid) = detail.extract::<String>() {
        presets::lookup(&t, &uid).ok_or_else(|| err("Unknown detail"))?
    } else {
        serde_json::from_value(to_value(detail)?).map_err(err)?
    };
    if p.primitive != "junction" {
        return Err(err("Not a junction detail"));
    }
    to_py(scene.py(), &bind(scene, &t, &p, &placements)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn knee() -> Value {
        json!({"uid":"c081000000000014","name":"End plate","primitive":"junction","pack":"Steel","rev":1,
               "locks":{},"record":{},
               "values":{"meeting":"end_face","roles":{"main":{"name":"Column"},"joining":{"name":"Rafter"}},
                 "params":[{"key":"plate_width","expr":"joining.width + 150","unit":"mm"}],
                 "rule":{"roles":{"main":{"profile":[{"library":"en/he200a","label":"HE 200 A"}],"size":{"height":[152,240]},"face":["flange_outer"]},
                                  "joining":{"profile":[{"library":"en/ipe"}]}},"angle":{"value":90,"tol":2}},
                 "contents":[{"key":"end_cut","on":"joining","row":{"kind":"cut","operation":"trim","end":"@joining.end",
                   "cutter":{"plane":{}},"anchor":{"ref":"@meet","offset":12}}},
                   {"key":"plate","part":{"preset":"732730a077d0b9f2","fields":{"profile.params.width":"plate_width"},"at":{"frame":"@meet"}}}]}})
    }

    #[test]
    fn canonical_hash_ignores_key_order_and_number_spelling() {
        let a = json!({"b": 1.0, "a": [1, {"y": 2, "x": "s"}]});
        let b: Value = serde_json::from_str(r#"{"a":[1.0,{"x":"s","y":2.0}],"b":1}"#).unwrap();
        assert_eq!(canonical(&a), canonical(&b));
        assert_eq!(row_hash(&a), row_hash(&b));
        let mut c = a.clone();
        c["hash"] = json!("sha256:x");
        assert_eq!(row_hash(&a), row_hash(&c));
    }

    #[test]
    fn template_refs_and_dependencies() {
        let k = knee();
        assert_eq!(direct_deps(&k), vec![Dep { kind: "preset", key: "732730a077d0b9f2".into() }]);
        assert!(validate_template(&k, &BTreeSet::new()).is_ok());
        let mut bad = k.clone();
        bad["values"]["contents"][0]["row"]["anchor"]["ref"] = json!("@brace.contact");
        assert!(validate_template(&bad, &BTreeSet::new()).unwrap_err().contains("brace"));
        let mut bad = k.clone();
        bad["values"]["params"][0]["expr"] = json!("brace.width + 1");
        assert!(validate_template(&bad, &BTreeSet::new()).unwrap_err().contains("brace"));
        let mut bad = k.clone();
        bad["values"]["params"][0]["expr"] = json!("plate_width + 1");
        assert!(validate_template(&bad, &BTreeSet::new()).unwrap_err().contains("cycle"));
        let mut bad = k;
        bad["values"]["contents"][1]["part"]["preset"] = json!("00000000000000aa");
        assert!(validate_template(&bad, &BTreeSet::new()).unwrap_err().contains("undeclared"));
    }

    #[test]
    fn rule_in_words() {
        let words = rule_words(&knee());
        assert_eq!(words, "IPE rafter end → HE 200 A 152–240 column flange · 90° ± 2° · mirrored");
        assert_eq!(adds_words(&knee()), "cut, part");
        assert_eq!(thumb_key(&knee()), "junction:end_face:cp");
        assert_eq!(thumbnail("junction:end_face:cpbw", 64).unwrap().len(), 64 * 64 * 4);
    }

    struct Mem(BTreeMap<String, Vec<u8>>);
    impl Files for Mem {
        fn read(&self, path: &str) -> PyResult<Option<Vec<u8>>> {
            Ok(self.0.get(path).cloned())
        }
    }

    #[test]
    fn export_then_read_round_trip_and_tamper() {
        Python::initialize();
        let none = |_: &str| -> PyResult<Option<Vec<u8>>> { Ok(None) };
        let src = Sources { rows: &[], component: &none };
        let (manifest, files) = export(&[knee()], &json!({"name":"Test","licence":"CC-BY-4.0","author":"A"}), &src, "Built|2026-11-28").unwrap();
        assert_eq!(manifest["v"], 2);
        assert!(manifest["pack"]["requires"]["features"].as_array().unwrap().contains(&json!("junction.v1")));
        let mut mem: BTreeMap<String, Vec<u8>> = files.into_iter().collect();
        mem.insert("pack.json".into(), serde_json::to_vec(&manifest).unwrap());
        let plan = read(&Mem(mem.clone()), &Known::new()).unwrap();
        assert_eq!(plan.rows.len(), 1);
        assert!(plan.rows[0].get("needs").is_none());
        assert_eq!(plan.rows[0]["licence"], "CC-BY-4.0");
        let path = "details/end-plate.builtpreset";
        let mut text = String::from_utf8(mem[path].clone()).unwrap();
        text = text.replace("150", "160");
        let mut tampered = mem.clone();
        tampered.insert(path.into(), text.into_bytes());
        let e = read(&Mem(tampered), &Known::new()).err().unwrap();
        assert!(e.to_string().contains("hash"));
    }

    #[test]
    fn missing_dependency_is_reported_and_inherited() {
        let mut k = knee();
        k["values"]["contents"][1]["part"]["preset"] = json!("00000000000000aa");
        k["depends"] = json!([{"kind":"preset","key":"00000000000000aa","bundled":null}]);
        let mut rows = vec![k];
        resolve(&mut rows, &BTreeMap::new(), &Known::new()).unwrap();
        assert_eq!(rows[0]["needs"], json!(["preset 00000000000000aa"]));
        let mut rows = vec![rows[0].clone()];
        let known = Known::from([("00000000000000aa".into(), String::new())]);
        resolve(&mut rows, &BTreeMap::new(), &known).unwrap();
        assert!(rows[0].get("needs").is_none());
    }
}
