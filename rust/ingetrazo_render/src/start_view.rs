//! The start page's view model (views/wgpu_start.py): which rows the list
//! shows after the filter (Continue, Pinned, Recent, Projects, Templates),
//! each row's words, the preview of the focused one (its linked files and
//! their state), the header, the footer and the templates.
//!
//! Python sends what it knows (`State`, JSON): the recent rows
//! (core/recent.py), what each document noted about itself, and what the
//! worker thread found of each file (exists, mtime, the picture's texture
//! key, a project's files, the links). This returns the large sheet's spec
//! (ui/large_sheet.rs `Spec`) with the icons it names and the focused row.
//! Sizes, ages and the links' state are read here (a `stat` each), as the
//! page did on a build.
use pyo3::prelude::*;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub struct Recent {
    pub path: String,
    pub pinned: bool,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Link {
    pub path: String,
    pub found: Option<String>,
    pub depth: i64,
    pub via: Option<String>,
}

/// What the worker found of one path; every field "not known yet".
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Cache {
    pub exists: Option<bool>,
    pub mtime: Option<f64>,
    /// The picture's texture key ("" without one).
    pub thumb: String,
    /// A project's files, (path, role); None: unreadable or not read.
    pub files: Option<Vec<(String, String)>>,
    pub links: Option<Vec<Link>>,
    pub autosave_newer: bool,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct OpenDoc {
    pub path: String,
    pub name: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct State {
    pub filter: String,
    pub focus: Option<String>,
    /// A document is open behind the page.
    pub back: bool,
    pub open_doc: OpenDoc,
    /// The document opened or saved last.
    pub last: String,
    pub files: Vec<Recent>,
    pub projects: Vec<Recent>,
    pub cache: HashMap<String, Cache>,
    /// What each document noted about itself (core/recent.py ``meta``).
    pub meta: HashMap<String, Value>,
    /// The opening job's label while one runs.
    pub pending: String,
    /// The engine and renderer line.
    pub engine: String,
    pub home: String,
    pub now: f64,
    /// Local time minus UTC, seconds (dates of old files).
    pub utc_offset: f64,
}

// ---- Templates ---------------------------------------------------------------------------
/// (key, name, parent, recipe tag, recipe class)
type Node = (&'static str, &'static str, &'static str, &'static str, &'static str);

pub struct Template {
    pub id: &'static str,
    pub name: &'static str,
    pub desc: &'static str,
    pub nodes: &'static [Node],
    pub active: &'static str,
    /// (length unit, precision); None: the user's usual units.
    pub units: Option<(&'static str, i64)>,
}

const SURVEY: &[Node] = &[
    ("existing", "Existing", "", "", ""),
    ("existing.structure", "Structure", "existing", "Existing Structure", ""),
    ("existing.envelope", "Envelope", "existing", "Existing Envelope", ""),
    ("existing.services", "Services", "existing", "Existing Services", ""),
    ("reference", "Reference", "", "", ""),
    ("reference.scan", "Scan and survey points", "reference", "Reference Scan", ""),
    ("reference.context", "Context", "reference", "Reference Context", ""),
];
const STEEL: &[Node] = &[
    ("steel", "Steel", "", "", ""),
    ("steel.columns", "Columns", "steel", "Steel Columns", "IfcColumn"),
    ("steel.beams", "Beams", "steel", "Steel Beams", "IfcBeam"),
    ("steel.bracing", "Bracing", "steel", "Steel Bracing", "IfcMember"),
    ("steel.plates", "Plates", "steel", "Steel Plates", "IfcPlate"),
    ("steel.fasteners", "Fasteners", "steel", "Fasteners", "IfcMechanicalFastener"),
];
const ARCH: &[Node] = &[
    ("building", "Building", "", "", ""),
    ("building.walls", "Walls", "building", "Walls", "IfcWall"),
    ("building.slabs", "Slabs", "building", "Slabs", "IfcSlab"),
    ("building.doors", "Doors", "building", "Doors", "IfcDoor"),
    ("building.windows", "Windows", "building", "Windows", "IfcWindow"),
    ("building.roof", "Roof", "building", "Roof", "IfcRoof"),
];

/// Stubs until Phase 4 of docs/project-structure.md: a new model with a
/// structure preset (core/structure.py nodes with recipes) and units.
pub const TEMPLATES: &[Template] = &[
    Template { id: "survey", name: "As-built survey", desc: "Existing structure, envelope, services; reference · m",
               nodes: SURVEY, active: "existing.structure", units: Some(("m", 3)) },
    Template { id: "steel", name: "Steel fabrication", desc: "Columns, beams, bracing, plates, fasteners · mm",
               nodes: STEEL, active: "steel.beams", units: Some(("mm", 0)) },
    Template { id: "arch_mm", name: "Architectural mm", desc: "Walls, slabs, doors, windows, roof · mm",
               nodes: ARCH, active: "building.walls", units: Some(("mm", 0)) },
    Template { id: "blank", name: "Blank", desc: "An empty model in your usual units", nodes: &[], active: "",
               units: None },
];

/// The structure nodes as core/structure.py stores them.
fn nodes_json(nodes: &[Node]) -> Value {
    let mut out = Map::new();
    let mut order: HashMap<&str, i64> = HashMap::new();
    for (key, name, parent, tag, class) in nodes {
        let n = order.entry(parent).or_insert(0);
        let mut node = json!({"name": name, "parent": if parent.is_empty() { Value::Null } else { json!(parent) },
                              "order": *n});
        *n += 1;
        if !tag.is_empty() {
            let mut recipe = json!({"tag": tag});
            if !class.is_empty() {
                recipe["class"] = json!(class);
            }
            node["recipe"] = recipe;
        }
        out.insert(key.to_string(), node);
    }
    Value::Object(out)
}

// ---- Words --------------------------------------------------------------------------------
fn name_of(path: &str) -> String {
    Path::new(path).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn stem_of(path: &str) -> String {
    Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The folder, short: under the home folder from ~, and only its last two
/// levels when deeper (the full path is the row's tooltip).
pub fn folder(path: &str, home: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let parent = Path::new(path).parent().unwrap_or(Path::new(""));
    let (head, parts): (&str, Vec<String>) = match (!home.is_empty()).then(|| parent.strip_prefix(home).ok()).flatten() {
        Some(rel) => ("~/", rel.iter().map(|s| s.to_string_lossy().into_owned()).collect()),
        None => ("/", parent.iter().skip(1).map(|s| s.to_string_lossy().into_owned()).collect()),
    };
    if parts.len() > 2 {
        return format!("…/{}", parts[parts.len() - 2..].join("/"));
    }
    format!("{head}{}", parts.join("/"))
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// (year, month 1-12, day) of a day count since 1970-01-01.
fn civil(days: i64) -> (i64, usize, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m as usize, d)
}

pub fn ago(mtime: f64, now: f64, utc_offset: f64) -> String {
    let s = (now - mtime).max(0.0);
    if s < 90.0 {
        return "just now".into();
    }
    if s < 3600.0 {
        return format!("{} min ago", (s / 60.0) as i64);
    }
    if s < 86400.0 {
        return format!("{} h ago", (s / 3600.0) as i64);
    }
    if s < 86400.0 * 14.0 {
        return format!("{} d ago", (s / 86400.0) as i64);
    }
    let (y, m, d) = civil(((mtime + utc_offset) / 86400.0).floor() as i64);
    format!("{d:02} {} {y}", MONTHS[m - 1])
}

fn mtime_of(path: &str) -> Option<f64> {
    let t = std::fs::metadata(path).ok()?.modified().ok()?;
    t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs_f64())
}

pub fn size(path: &str) -> String {
    let Ok(meta) = std::fs::metadata(path) else { return String::new() };
    let mut n = meta.len() as f64;
    for unit in ["B", "KB", "MB", "GB"] {
        if n < 1024.0 || unit == "GB" {
            return if unit == "B" || unit == "KB" {
                format!("{n:.0} {unit}")
            } else {
                format!("{n:.1} {unit}").replace(".0 ", " ")
            };
        }
        n /= 1024.0;
    }
    String::new()
}

/// ``missing``, ``changed`` (the source is newer than the file that links
/// it was saved) or ``found``.
pub fn link_state(link: &Link, host_mtime: Option<f64>) -> &'static str {
    let Some(found) = link.found.as_deref().filter(|f| !f.is_empty()) else { return "missing" };
    let Some(src) = mtime_of(found) else { return "missing" };
    let reference = match link.via.as_deref().filter(|v| !v.is_empty()) {
        Some(via) => match mtime_of(via) {
            Some(t) => Some(t),
            None => return "missing",
        },
        None => host_mtime,
    };
    match reference {
        Some(r) if src > r + 2.0 => "changed",
        _ => "found",
    }
}

fn imported_name(code: &str) -> String {
    match code {
        "skp" => ".skp".into(),
        "dwg" => "DWG".into(),
        "dxf" => "DXF".into(),
        "ifc" => "IFC".into(),
        "rfa" => "Revit".into(),
        other => other.to_uppercase(),
    }
}

fn joined(parts: &[&str]) -> String {
    parts.iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(" · ")
}

// ---- The spec's pieces (views/chrome_sheet.py ``item``, ``verb``, Pane blocks) --------------
fn prune(v: Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(m.into_iter().filter(|(k, v)| {
            k == "id" || k == "label" || !matches!(v, Value::Null | Value::Bool(false))
                && v.as_str() != Some("") && v.as_f64() != Some(0.0) && v.as_array().map_or(true, |a| !a.is_empty())
        }).collect()),
        other => other,
    }
}

fn verb(id: &str, label: &str, key: &str, role: i64) -> Value {
    json!({"id": id, "label": label, "key": key, "role": role, "enabled": true, "tip": ""})
}
const PRIMARY: i64 = 1;
const QUIET: i64 = 3;

fn block(kind: &str, fields: Value) -> Value {
    let mut b = fields;
    b["kind"] = json!(kind);
    prune(b)
}

// ---- The meta a document noted ----------------------------------------------------------
struct Meta<'a>(Option<&'a Value>);

impl Meta<'_> {
    fn text(&self, key: &str) -> String {
        self.0.and_then(|m| m.get(key)).and_then(Value::as_str).unwrap_or("").to_owned()
    }
    fn int(&self, key: &str) -> Option<i64> {
        let v = self.0.and_then(|m| m.get(key))?;
        v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)).or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    }
    fn problems(&self) -> i64 {
        self.int("problems").unwrap_or(0)
    }
    fn last_in(&self) -> String {
        let w = self.text("workspace");
        if w.is_empty() { w } else { format!("Last in {w}") }
    }
}

// ---- The list -------------------------------------------------------------------------------
pub enum Kind<'a> {
    Cont(Cont),
    File(&'a Recent),
    Proj(&'a Recent),
    Tpl(&'a Template),
}

pub struct Cont {
    pub path: String,
    pub name: String,
    pub place: String,
    pub last_in: String,
    pub problems: i64,
    pub open: bool,
}

pub const SECTIONS: [(&str, &str); 5] = [("cont", "Continue"), ("pinned", "Pinned"), ("recent", "Recent"),
    ("projects", "Projects"), ("templates", "Templates")];

pub struct View<'a> {
    st: &'a State,
    empty_cache: Cache,
}

impl<'a> View<'a> {
    pub fn new(st: &'a State) -> Self {
        View { st, empty_cache: Cache::default() }
    }

    fn cache(&self, path: &str) -> &Cache {
        self.st.cache.get(path).unwrap_or(&self.empty_cache)
    }

    fn meta(&self, path: &str) -> Meta<'_> {
        Meta(self.st.meta.get(path))
    }

    fn missing(&self, path: &str) -> bool {
        self.cache(path).exists == Some(false)
    }

    /// The document to continue: the one open behind the page, else the
    /// last one opened or saved (when it still exists).
    pub fn continue_info(&self) -> Option<Cont> {
        let st = self.st;
        if st.back {
            let path = st.open_doc.path.clone();
            let m = self.meta(&path);
            let name = if st.open_doc.name.is_empty() { "Untitled".to_owned() } else { st.open_doc.name.clone() };
            let last_in = m.last_in();
            return Some(Cont {
                place: joined(&[&m.text("project"), &folder(&path, &st.home)]),
                last_in: if last_in.is_empty() { "In Model".into() } else { last_in },
                problems: m.problems(), name, path, open: true });
        }
        let path = st.last.clone();
        if path.is_empty() || self.missing(&path) {
            return None;
        }
        let m = self.meta(&path);
        Some(Cont { place: joined(&[&m.text("project"), &folder(&path, &st.home)]), last_in: m.last_in(),
                    problems: m.problems(), name: name_of(&path), path, open: false })
    }

    /// The recent files the page shows: all of them but a missing one whose
    /// name a file higher up the list has, and that one exists — the same
    /// document, moved. Until the worker has looked, every row shows.
    pub fn visible_files(&self) -> Vec<&'a Recent> {
        let mut here = HashSet::new();
        let mut out = vec![];
        for r in &self.st.files {
            let c = self.cache(&r.path);
            let name = name_of(&r.path).to_lowercase();
            if c.exists == Some(false) && here.contains(&name) {
                continue;
            }
            if c.exists == Some(true) {
                here.insert(name);
            }
            out.push(r);
        }
        out
    }

    /// The recent files the worker found gone (hidden duplicates included).
    pub fn missing_files(&self) -> Vec<String> {
        self.st.files.iter().filter(|r| self.missing(&r.path)).map(|r| r.path.clone()).collect()
    }

    /// The list's sections after the filter: (section, [(row id, kind)]).
    pub fn sections(&self) -> Vec<(&'static str, Vec<(String, Kind<'a>)>)> {
        let words: Vec<String> = self.st.filter.to_lowercase().split_whitespace().map(str::to_owned).collect();
        let matches = |hay: &[&str]| {
            let text = hay.iter().filter(|h| !h.is_empty()).copied().collect::<Vec<_>>().join(" ").to_lowercase();
            words.iter().all(|w| text.contains(w.as_str()))
        };
        let mut out: Vec<(&'static str, Vec<(String, Kind<'a>)>)> = SECTIONS.iter().map(|(k, _)| (*k, vec![])).collect();
        let cont = self.continue_info();
        let skip = cont.as_ref().map(|c| c.path.clone()).unwrap_or_default();
        if let Some(c) = cont {
            if matches(&[&c.name, &c.place]) {
                out[0].1.push(("cont".into(), Kind::Cont(c)));
            }
        }
        for r in self.visible_files() {
            if r.path == skip {
                continue;
            }
            if !matches(&[&name_of(&r.path), &self.meta(&r.path).text("project"), &r.path]) {
                continue;
            }
            out[if r.pinned { 1 } else { 2 }].1.push((format!("file:{}", r.path), Kind::File(r)));
        }
        for r in &self.st.projects {
            let names: Vec<String> = self.cache(&r.path).files.iter().flatten().map(|(f, _)| name_of(f)).collect();
            let mut hay = vec![stem_of(&r.path), r.path.clone()];
            hay.extend(names);
            if !matches(&hay.iter().map(String::as_str).collect::<Vec<_>>()) {
                continue;
            }
            out[if r.pinned { 1 } else { 3 }].1.push((format!("proj:{}", r.path), Kind::Proj(r)));
        }
        for t in TEMPLATES {
            if matches(&[t.name, t.desc]) {
                out[4].1.push((format!("tpl:{}", t.id), Kind::Tpl(t)));
            }
        }
        out
    }

    fn ago(&self, mtime: f64) -> String {
        ago(mtime, self.st.now, self.st.utc_offset)
    }

    fn ago_of(&self, path: &str) -> String {
        if path.is_empty() { String::new() } else { mtime_of(path).map(|t| self.ago(t)).unwrap_or_default() }
    }

    fn row(&self, rid: &str, kind: &Kind, icons: &mut Vec<String>) -> Value {
        let home = &self.st.home;
        let mut thumb = |path: &str| {
            let key = self.cache(path).thumb.clone();
            if !key.is_empty() && !icons.contains(&key) {
                icons.push(key.clone());
            }
            key
        };
        match kind {
            Kind::Cont(c) => prune(json!({"id": rid, "label": c.name, "sub": joined(&[&c.place, &c.last_in]),
                "right": if c.open { "open now".into() } else { self.ago_of(&c.path) },
                "icon": thumb(&c.path), "badge": if c.problems != 0 { c.problems.to_string() } else { String::new() },
                "tip": c.path})),
            Kind::Tpl(t) => prune(json!({"id": rid, "label": t.name, "sub": t.desc,
                                         "menu": [["open", "New from this template"]]})),
            Kind::Proj(r) | Kind::File(r) => {
                let path = &r.path;
                let c = self.cache(path);
                let missing = c.exists == Some(false);
                let mut menu = json!([["open", "Open"], ["pin", if r.pinned { "Unpin" } else { "Pin" }],
                                      ["finder", "Show in Finder"], ["-", ""], ["forget", "Remove from list"]]);
                if matches!(kind, Kind::Proj(_)) {
                    let n = c.files.as_ref().map_or(0, Vec::len);
                    let right = if n == 0 { String::new() } else { format!("{n} file{}", if n != 1 { "s" } else { "" }) };
                    return prune(json!({"id": rid, "label": stem_of(path),
                        "sub": if missing { "Not found · Locate…".into() } else { folder(path, home) },
                        "right": right, "dim": missing, "tip": path, "menu": menu, "kind": "file"}));
                }
                let m = self.meta(path);
                let (sub, right) = if missing {
                    menu = json!([["locate", "Locate…"], ["-", ""], ["forget", "Remove from list"]]);
                    ("Not found · Locate…".to_owned(), String::new())
                } else {
                    (joined(&[&m.text("project"), &folder(path, home)]), c.mtime.map(|t| self.ago(t)).unwrap_or_default())
                };
                let problems = m.problems();
                prune(json!({"id": rid, "label": name_of(path), "sub": sub, "right": right, "icon": thumb(path),
                    "dim": missing, "badge": if problems != 0 && !missing { problems.to_string() } else { String::new() },
                    "tip": path, "menu": menu}))
            }
        }
    }

    fn preview(&self, found: Option<&(String, Kind)>, icons: &mut Vec<String>) -> Value {
        let mut blocks = vec![];
        let detail = |blocks: Vec<Value>| json!({"title": "", "sub": "",
            "empty": "Pick a file to see what it holds before opening it.", "width": 520.0, "blocks": blocks});
        let Some((_, kind)) = found else { return detail(blocks) };
        let buttons = |list: Vec<Value>| block("buttons", json!({"buttons": list, "align": "right"}));
        let (path, name, pinned) = match kind {
            Kind::Tpl(t) => {
                blocks.push(block("heading", json!({"text": t.name})));
                blocks.push(block("text", json!({"text": t.desc})));
                let mut rows = vec![json!(["Units", t.units.map_or("your usual units", |u| u.0), ""])];
                if !t.nodes.is_empty() {
                    let tops: Vec<&str> = t.nodes.iter().filter(|n| n.2.is_empty()).map(|n| n.1).collect();
                    rows.push(json!(["Structure", tops.join(", "), ""]));
                    rows.push(json!(["Nodes", t.nodes.len().to_string(), ""]));
                }
                blocks.push(block("kv", json!({"rows": rows})));
                blocks.push(buttons(vec![verb("open", "New from this template", "↩", PRIMARY)]));
                return detail(blocks);
            }
            Kind::Cont(c) => (c.path.clone(), c.name.clone(), false),
            Kind::File(r) => (r.path.clone(), name_of(&r.path), r.pinned),
            Kind::Proj(r) => (r.path.clone(), stem_of(&r.path), r.pinned),
        };
        let c = self.cache(&path);
        let missing = c.exists == Some(false);
        let m = self.meta(&path);
        let proj = matches!(kind, Kind::Proj(_));
        if !proj {
            if !c.thumb.is_empty() && !icons.contains(&c.thumb) {
                icons.push(c.thumb.clone());
            }
            blocks.push(block("picture", json!({"icon": c.thumb, "size": 250.0})));
        }
        let right = joined(&[&folder(&path, &self.st.home), &size(&path)]);
        blocks.push(block("heading", json!({"text": name, "right": right, "tone": if pinned { "accent" } else { "" }})));
        if missing {
            blocks.push(block("text", json!({"text": "This file is not where it was. Locate it, or take it off the list.",
                                             "tone": "warn"})));
            blocks.push(buttons(vec![verb("forget", "Remove from list", "", 0), verb("locate", "Locate…", "↩", PRIMARY)]));
            return detail(blocks);
        }
        if proj {
            match &c.files {
                None if c.exists == Some(true) =>
                    blocks.push(block("text", json!({"text": "Can't read this project.", "tone": "warn"}))),
                Some(files) if !files.is_empty() => {
                    blocks.push(block("caption", json!({"text": "Files", "right": files.len().to_string()})));
                    let items: Vec<Value> = files.iter().map(|(f, role)| {
                        let here = Path::new(f).is_file();
                        prune(json!({"id": "", "label": name_of(f), "sub": if role != "host" { role.as_str() } else { "" },
                            "dot": if here { "ok" } else { "error" }, "dot_text": if here { "" } else { "missing" },
                            "kind": "file", "fixed": true}))
                    }).collect();
                    blocks.push(block("rows", json!({"items": items})));
                }
                _ => {}
            }
            blocks.push(buttons(vec![verb("finder", "Show in Finder", "", 0), verb("open", "Open project", "↩", PRIMARY)]));
            return detail(blocks);
        }
        let mut rows = vec![];
        let project = m.text("project");
        if !project.is_empty() {
            rows.push(json!(["Project", project, ""]));
        }
        let left = joined(&[&m.text("workspace"), &c.mtime.map(|t| self.ago(t)).unwrap_or_default()]);
        if !left.is_empty() {
            rows.push(json!(["Left in", left, ""]));
        }
        let n = m.problems();
        rows.push(json!(["Problems", if n != 0 { format!("{n} to fix") } else { "none".into() }, if n != 0 { "warn" } else { "" }]));
        if let Some(u) = m.int("unfiled") {
            rows.push(json!(["Unfiled", u.to_string(), ""]));
        }
        let imported = m.text("imported");
        if !imported.is_empty() {
            rows.push(json!(["From", imported_name(&imported), ""]));
        }
        blocks.push(block("kv", json!({"rows": rows})));
        if c.autosave_newer {
            blocks.push(block("text", json!({"text": "Auto-save is newer than the file: opening asks which to keep.",
                                             "tone": "warn"})));
        }
        match &c.links {
            None if c.exists == Some(true) => {
                blocks.push(block("caption", json!({"text": "Linked files"})));
                blocks.push(block("text", json!({"text": "Reading links…", "tone": "faint"})));
            }
            Some(links) if !links.is_empty() => {
                let states: Vec<&str> = links.iter().map(|l| link_state(l, c.mtime)).collect();
                let bad = states.iter().filter(|s| **s != "found").count();
                let right = if bad > 0 { format!("{} · {bad} to look at", links.len()) } else { links.len().to_string() };
                blocks.push(block("caption", json!({"text": "Linked files", "right": right})));
                let items: Vec<Value> = links.iter().zip(&states).map(|(l, s)| {
                    let (dot, words) = match *s { "found" => ("ok", ""), "changed" => ("warn", "changed on disk"),
                                                  _ => ("error", "missing") };
                    let found = l.found.clone().unwrap_or_default();
                    prune(json!({"id": "", "label": name_of(&l.path), "depth": l.depth, "dot": dot, "dot_text": words,
                        "right": self.ago_of(&found), "kind": "file", "fixed": true,
                        "tip": if found.is_empty() { l.path.clone() } else { found }}))
                }).collect();
                blocks.push(block("rows", json!({"items": items})));
            }
            _ => {}
        }
        let open = match kind {
            Kind::Cont(c) if c.open => verb("open", "Back to the model", "esc", PRIMARY),
            _ => verb("open", "Open", "↩", PRIMARY),
        };
        blocks.push(buttons(vec![verb("finder", "Show in Finder", "", 0), open]));
        detail(blocks)
    }

    /// The sheet's spec, with the icons it names, the focused row and the
    /// path of the Continue row.
    pub fn sheet(&self) -> Value {
        let st = self.st;
        let mut icons: Vec<String> = vec![];
        let backdrop = if st.back { String::new() } else {
            std::iter::once(&st.last).chain(st.files.iter().map(|r| &r.path)).filter(|p| !p.is_empty())
                .map(|p| self.cache(p).thumb.clone()).find(|k| !k.is_empty()).unwrap_or_default()
        };
        if !backdrop.is_empty() {
            icons.push(backdrop.clone());
        }
        let sections = self.sections();
        let order: Vec<&str> = sections.iter().flat_map(|(_, rows)| rows.iter().map(|(id, _)| id.as_str())).collect();
        let focus = match &st.focus {
            Some(f) if order.contains(&f.as_str()) => Some(f.clone()),
            _ => order.first().map(|s| s.to_string()),
        };
        let mut blocks = vec![];
        let mut cont_path = String::new();
        for (key, rows) in &sections {
            if rows.is_empty() {
                continue;
            }
            let caption = SECTIONS.iter().find(|(k, _)| k == key).map_or("", |s| s.1);
            blocks.push(block("caption", json!({"text": caption})));
            let items: Vec<Value> = rows.iter().map(|(id, kind)| {
                if let Kind::Cont(c) = kind {
                    cont_path = c.path.clone();
                }
                self.row(id, kind, &mut icons)
            }).collect();
            blocks.push(block("rows", json!({"items": items, "id": key,
                                             "tone": if *key != "templates" { "thumbs" } else { "" }})));
            if *key == "recent" {
                let gone = self.missing_files();
                if !gone.is_empty() && st.filter.is_empty() {
                    blocks.push(block("buttons", json!({"buttons": [verb("clearmissing",
                        &format!("Clear missing ({})", gone.len()), "", QUIET)], "align": "left"})));
                }
            }
        }
        let found = sections.iter().flat_map(|(_, rows)| rows.iter()).find(|(id, _)| Some(id) == focus.as_ref());
        let detail = self.preview(found, &mut icons);
        let empty = if st.filter.is_empty() { "No recent files yet.".to_owned() }
                    else { format!("Nothing matches “{}”.", st.filter) };
        let hint = format!("↑↓ choose · ↩ open{}   {}", if st.back { " · esc back to the model" } else { "" }, st.engine);
        let links = [("libraries", "Library"), ("prefs", "Settings"), ("extensions", "Extensions"), ("help", "Help")];
        json!({
            "id": "start", "title": "", "closable": st.back, "width": 1180.0, "backdrop_icon": backdrop,
            "default_verb": "", "tabs": [],
            "verbs": [verb("new", "New model", "⌘N", PRIMARY), verb("openfile", "Open…", "⌘O", 0),
                      verb("project", "New project…", "", 0)],
            "menus": [{"id": "template", "label": "From template", "on": "", "left": true,
                       "options": TEMPLATES.iter().map(|t| verb(t.id, t.name, "", 0)).collect::<Vec<_>>()}],
            "side": [], "side_width": 0.0,
            "content": {"title": st.pending, "sub": "", "empty": empty, "width": 0.0, "blocks": blocks},
            "detail": detail,
            "footer": {"links": links.iter().map(|(i, l)| verb(i, l, "", 0)).collect::<Vec<_>>(), "hint": hint},
            "focus": focus, "force_focus": false,
            "search": {"placeholder": "Type to filter", "key": "", "text": "", "force": false},
            "icons": icons, "cont_path": cont_path,
        })
    }
}

fn parse(state: &str) -> PyResult<State> {
    serde_json::from_str(state).map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
}

/// The Start sheet's spec (JSON) for a ``State`` (JSON).
#[pyfunction]
pub fn start_sheet(state: &str) -> PyResult<String> {
    Ok(View::new(&parse(state)?).sheet().to_string())
}

/// The sections as [(section, [(row id, kind, path)])] (tests, the home
/// worksession).
#[pyfunction]
pub fn start_sections(state: &str) -> PyResult<Vec<(String, Vec<(String, String, String)>)>> {
    let st = parse(state)?;
    let view = View::new(&st);
    Ok(view.sections().into_iter().map(|(key, rows)| (key.to_owned(), rows.into_iter().map(|(id, kind)| {
        let (k, path) = match kind {
            Kind::Cont(c) => ("cont", c.path),
            Kind::File(r) => ("file", r.path.clone()),
            Kind::Proj(r) => ("proj", r.path.clone()),
            Kind::Tpl(t) => ("tpl", t.id.to_owned()),
        };
        (id, k.to_owned(), path)
    }).collect())).collect())
}

/// The recent files the worker found gone ("Clear missing").
#[pyfunction]
pub fn start_missing(state: &str) -> PyResult<Vec<String>> {
    Ok(View::new(&parse(state)?).missing_files())
}

/// The templates: (id, name, description, structure nodes JSON or "",
/// active node, units JSON or "").
#[pyfunction]
pub fn start_templates() -> Vec<(String, String, String, String, String, String)> {
    TEMPLATES.iter().map(|t| (t.id.into(), t.name.into(), t.desc.into(),
        if t.nodes.is_empty() { String::new() } else { nodes_json(t.nodes).to_string() }, t.active.into(),
        t.units.map(|(l, p)| json!({"length": l, "precision": p}).to_string()).unwrap_or_default())).collect()
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(start_sheet, m)?)?;
    m.add_function(wrap_pyfunction!(start_sections, m)?)?;
    m.add_function(wrap_pyfunction!(start_missing, m)?)?;
    m.add_function(wrap_pyfunction!(start_templates, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(v: Value) -> State {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn folders_ages_and_dates() {
        assert_eq!(folder("/Users/me/a/b.igz", "/Users/me"), "~/a");
        assert_eq!(folder("/Users/me/a/b/c/d.igz", "/Users/me"), "…/b/c");
        assert_eq!(folder("/tmp/x.igz", "/Users/me"), "/tmp");
        assert_eq!(ago(100.0, 130.0, 0.0), "just now");
        assert_eq!(ago(0.0, 7200.0, 0.0), "2 h ago");
        assert_eq!(ago(0.0, 86400.0 * 20.0, 0.0), "01 Jan 1970");
        assert_eq!(civil(19_723), (2024, 1, 1));
        assert_eq!(civil(-1), (1969, 12, 31));
    }

    #[test]
    fn sections_filter_pins_and_hide_moved_duplicates() {
        let st = state(json!({
            "last": "/w/a.igz",
            "files": [{"path": "/w/a.igz"}, {"path": "/w/p.igz", "pinned": true},
                      {"path": "/n/ride.igz"}, {"path": "/old/ride.igz"}],
            "projects": [{"path": "/w/job.igzproj"}],
            "cache": {"/n/ride.igz": {"exists": true}, "/old/ride.igz": {"exists": false},
                      "/w/job.igzproj": {"exists": true, "files": [["/w/host.igz", "host"]]}},
            "meta": {"/w/p.igz": {"project": "Ride"}}}));
        let v = View::new(&st);
        let ids = |s: &[(&str, Vec<(String, Kind)>)], k: &str| -> Vec<String> {
            s.iter().find(|x| x.0 == k).unwrap().1.iter().map(|r| r.0.clone()).collect()
        };
        let s = v.sections();
        assert_eq!(ids(&s, "cont"), ["cont"]);
        assert_eq!(ids(&s, "pinned"), ["file:/w/p.igz"]);
        assert_eq!(ids(&s, "recent"), ["file:/n/ride.igz"]);         // a moved one: once
        assert_eq!(ids(&s, "projects"), ["proj:/w/job.igzproj"]);
        assert_eq!(v.missing_files(), ["/old/ride.igz"]);
        let st = State { filter: "ride host".into(), ..st };
        let s = View::new(&st).sections();
        assert!(ids(&s, "pinned").is_empty() && ids(&s, "recent").is_empty());
        let st = State { filter: "host".into(), ..st };
        assert_eq!(ids(&View::new(&st).sections(), "projects"), ["proj:/w/job.igzproj"]);  // by a file it holds
    }

    #[test]
    fn the_sheet_names_its_icons_and_focus() {
        let st = state(json!({"last": "/w/a.igz", "files": [{"path": "/w/a.igz"}],
                              "cache": {"/w/a.igz": {"exists": true, "thumb": "thumb:1", "links": []}}}));
        let sheet = View::new(&st).sheet();
        assert_eq!(sheet["backdrop_icon"], "thumb:1");
        assert_eq!(sheet["focus"], "cont");
        assert_eq!(sheet["cont_path"], "/w/a.igz");
        assert_eq!(sheet["icons"], json!(["thumb:1"]));
        let text = sheet.to_string();
        assert!(serde_json::from_str::<crate::ui::large_sheet::Spec>(&text).is_ok(), "{text}");
        let st = State { focus: Some("tpl:steel".into()), ..st };
        let sheet = View::new(&st).sheet();
        let blocks = sheet["detail"]["blocks"].as_array().unwrap();
        assert_eq!(blocks[0]["text"], "Steel fabrication");
        assert_eq!(blocks[2]["rows"][1], json!(["Structure", "Steel", ""]));
    }

    #[test]
    fn templates_keep_their_structure() {
        let t = start_templates();
        assert_eq!(t.iter().map(|t| t.0.as_str()).collect::<Vec<_>>(), ["survey", "steel", "arch_mm", "blank"]);
        let nodes: Value = serde_json::from_str(&t[0].3).unwrap();
        assert_eq!(nodes["reference"]["order"], 1);
        assert_eq!(nodes["existing.envelope"]["order"], 1);
        assert_eq!(nodes["existing.envelope"]["recipe"], json!({"tag": "Existing Envelope"}));
        assert_eq!(nodes["steel"], Value::Null);
        assert!(t[3].3.is_empty() && t[3].5.is_empty());
    }
}
