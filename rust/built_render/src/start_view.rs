//! Start sheet rows, selection, and creation or opening details. File facts
//! come from the host worker cache so building a frame needs no filesystem I/O.
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
    pub status: String,
}

/// What the worker found of one path; every field "not known yet".
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Cache {
    pub exists: Option<bool>,
    pub mtime: Option<f64>,
    pub bytes: Option<u64>,
    pub units: Option<String>,
    /// The picture's texture key ("" without one).
    pub thumb: String,
    /// A project's files, (path, role); None: unreadable or not read.
    pub files: Option<Vec<(String, String)>>,
    pub links: Option<Vec<Link>>,
    pub missing_members: Vec<String>,
    pub autosave_newer: bool,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct State {
    pub filter: String,
    pub focus: Option<String>,
    /// A document is open behind the page.
    pub back: bool,
    /// The document opened or saved last.
    pub last: String,
    pub files: Vec<Recent>,
    pub projects: Vec<Recent>,
    pub cache: HashMap<String, Cache>,
    /// What each document noted about itself (core/recent.py ``meta``).
    pub meta: HashMap<String, Value>,
    /// The opening job's label while one runs.
    pub pending: String,
    pub projects_tab: bool,
    pub missing_open: bool,
    pub draft_units: String,
    pub usual_units: String,
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
    Template { id: "survey", name: "As-built survey", desc: "Existing structure, envelope, services and survey references.",
               nodes: SURVEY, active: "existing.structure", units: Some(("m", 3)) },
    Template { id: "steel", name: "Steel fabrication", desc: "Columns, beams, bracing, plates and fasteners.",
               nodes: STEEL, active: "steel.beams", units: Some(("mm", 0)) },
    Template { id: "arch_mm", name: "Architectural mm", desc: "Walls, slabs, doors, windows and roof.",
               nodes: ARCH, active: "building.walls", units: Some(("mm", 0)) },
    Template { id: "blank", name: "Blank", desc: "Start with an empty model.", nodes: &[], active: "",
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

fn size(bytes: u64) -> String {
    let mut n = bytes as f64;
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

fn problem_label(count: i64) -> String {
    let digits = count.to_string();
    let mut label = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 { label.push(','); }
        label.push(digit);
    }
    label.push_str(if count == 1 { " problem" } else { " problems" });
    label
}

fn joined(parts: &[&str]) -> String {
    parts.iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(" · ")
}

// ---- The spec's pieces (sheet_runtime.rs ``item``, ``verb``, Pane blocks) --------------
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
pub const UNIT_CODES: [&str; 4] = ["m", "cm", "mm", "ft-in-frac"];

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
}

// ---- The list -------------------------------------------------------------------------------
pub enum Kind<'a> {
    File(&'a Recent),
    Proj(&'a Recent),
    Tpl(&'a Template),
}

pub const SECTIONS: [(&str, &str); 5] = [
    ("templates", "Templates"), ("pinned", "Pinned"), ("recent", "Recent"),
    ("missing", "Unavailable"), ("projects", "Recent projects"),
];

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
        let mut out = SECTIONS.map(|(key, _)| (key, Vec::new()));
        let [(_, templates), (_, pinned), (_, recent), (_, missing), (_, projects)] = &mut out;
        if !self.st.projects_tab {
            // History records paths. A moved model may still have an older
            // unavailable path with the same filename as its live entry.
            let live_names: HashSet<_> = self.st.files.iter()
                .filter(|r| self.cache(&r.path).exists == Some(true))
                .filter_map(|r| Path::new(&r.path).file_name()).collect();
            let mut seen_paths = HashSet::new();
            for r in &self.st.files {
                if !seen_paths.insert(&r.path) { continue; }
                if self.missing(&r.path) && Path::new(&r.path).file_name().is_some_and(|n| live_names.contains(n)) {
                    continue;
                }
                if matches(&[&name_of(&r.path), &self.meta(&r.path).text("project"), &r.path]) {
                    let group = if self.missing(&r.path) { &mut *missing } else if r.pinned { &mut *pinned } else { &mut *recent };
                    group.push((format!("file:{}", r.path), Kind::File(r)));
                }
            }
            for t in TEMPLATES.iter().filter(|t| t.id == "blank").chain(TEMPLATES.iter().filter(|t| t.id != "blank")) {
                if matches(&[t.name, t.desc]) {
                    templates.push((format!("tpl:{}", t.id), Kind::Tpl(t)));
                }
            }
            return out.into();
        }
        for r in &self.st.projects {
            let names: Vec<String> = self.cache(&r.path).files.iter().flatten().map(|(f, _)| name_of(f)).collect();
            let mut hay = vec![stem_of(&r.path), r.path.clone()];
            hay.extend(names);
            if !matches(&hay.iter().map(String::as_str).collect::<Vec<_>>()) {
                continue;
            }
            projects.push((format!("proj:{}", r.path), Kind::Proj(r)));
        }
        out.into()
    }

    fn ago(&self, mtime: f64) -> String {
        ago(mtime, self.st.now, self.st.utc_offset)
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
            Kind::Tpl(t) => prune(json!({"id": rid, "label": t.name, "kind": "file"})),
            Kind::Proj(r) | Kind::File(r) => {
                let path = &r.path;
                let c = self.cache(path);
                let missing = c.exists == Some(false);
                let mut menu = json!([["open", "Open"], ["pin", if r.pinned { "Unpin" } else { "Pin" }],
                                      ["finder", "Show in Finder"], ["-", ""], ["forget", "Remove from recents"]]);
                if matches!(kind, Kind::Proj(_)) {
                    let n = c.files.as_ref().map_or(0, Vec::len);
                    let right = if n == 0 { String::new() } else { format!("{n} file{}", if n != 1 { "s" } else { "" }) };
                    return prune(json!({"id": rid, "label": stem_of(path),
                        "sub": if missing { "Not found · Locate…".into() } else { folder(path, home) },
                        "right": right, "dim": missing, "tip": path, "menu": menu, "kind": "file"}));
                }
                let m = self.meta(path);
                let (sub, right) = if missing {
                    menu = json!([["locate", "Locate…"], ["-", ""], ["forget", "Remove from recents"]]);
                    ("File missing".to_owned(), String::new())
                } else {
                    (joined(&[&m.text("project"), &folder(path, home)]), c.mtime.map(|t| self.ago(t)).unwrap_or_default())
                };
                let problems = m.problems();
                prune(json!({"id": rid, "label": name_of(path), "sub": sub, "right": right, "icon": thumb(path),
                    "dim": missing, "badge": if problems != 0 && !missing { problems.to_string() } else { String::new() },
                    "tip": if problems > 0 && !missing { format!("{path}\n{}", problem_label(problems)) } else { path.clone() },
                    "menu": menu, "kind":"file"}))
            }
        }
    }

    fn preview(&self, found: Option<&(String, Kind)>, icons: &mut Vec<String>) -> Value {
        let mut blocks = vec![];
        let detail = |blocks: Vec<Value>| json!({"title": "", "sub": "",
            "empty": "Pick a file to see what it holds before opening it.", "width": 580.0, "blocks": blocks});
        let Some((_, kind)) = found else {
            if self.st.projects_tab {
                blocks.push(block("heading", json!({"text": "Projects"})));
                blocks.push(block("text", json!({"text": "A .builtproject file keeps linked models and their project structure together."})));
            }
            return detail(blocks);
        };
        let buttons = |list: Vec<Value>| block("buttons", json!({"buttons": list, "align": "left"}));
        let (path, name, pinned) = match kind {
            Kind::Tpl(t) => {
                blocks.push(block("heading", json!({"text": t.name})));
                blocks.push(block("text", json!({"text": t.desc})));
                let usual = if self.st.usual_units.is_empty() { "m" } else { &self.st.usual_units };
                let units = if self.st.draft_units.is_empty() { t.units.map_or(usual, |u| u.0) } else { &self.st.draft_units };
                let mut codes = UNIT_CODES.to_vec();
                if !codes.contains(&units) { codes.push(units); }
                let index = codes.iter().position(|code| *code == units).unwrap_or(0);
                blocks.push(block("fields", json!({"id":"creation", "fields":[["units","choice","Units","",index,0,codes.len(),codes,0,""]]})));
                let mut rows = vec![];
                if !t.nodes.is_empty() {
                    let tops: Vec<&str> = t.nodes.iter().filter(|n| n.2.is_empty()).map(|n| n.1).collect();
                    rows.push(json!(["Structure", tops.join(", "), ""]));
                    rows.push(json!(["Nodes", t.nodes.len().to_string(), ""]));
                }
                if !rows.is_empty() { blocks.push(block("kv", json!({"rows": rows}))); }
                return detail(blocks);
            }
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
        }
        blocks.push(block("heading", json!({"text": name, "tone": if pinned { "accent" } else { "" }})));
        blocks.push(block("text", json!({"text": Path::new(&path).parent().unwrap_or(Path::new("")).display().to_string(), "tone":"faint"})));
        if missing {
            blocks.push(block("text", json!({"text": "File missing. Locate it to update its path in recents.",
                                             "tone": "warn"})));
            blocks.push(buttons(vec![verb("forget", "Remove from recents", "", 0)]));
            return detail(blocks);
        }
        if proj {
            match &c.files {
                None if c.exists == Some(true) =>
                    blocks.push(block("text", json!({"text": "Can't read this project.", "tone": "warn"}))),
                Some(files) if !files.is_empty() => {
                    blocks.push(block("caption", json!({"text": "Files", "right": files.len().to_string()})));
                    let items: Vec<Value> = files.iter().map(|(f, role)| {
                        let here = !c.missing_members.contains(f);
                        prune(json!({"id": "", "label": name_of(f), "sub": if role != "host" { role.as_str() } else { "" },
                            "dot": if here { "ok" } else { "error" }, "dot_text": if here { "" } else { "missing" },
                            "kind": "file", "fixed": true}))
                    }).collect();
                    blocks.push(block("rows", json!({"items": items})));
                }
                _ => {}
            }
            blocks.push(buttons(vec![verb("finder", "Show in Finder", "", 0)]));
            return detail(blocks);
        }
        if c.thumb.is_empty() {
            blocks.push(block("text", json!({"text":"No saved preview","tone":"faint"})));
        } else {
            blocks.push(block("picture", json!({"icon":c.thumb, "size":180.0, "tone":"fit"})));
        }
        let units = c.units.clone().unwrap_or_else(|| m.text("units"));
        let rows = json!([
            ["Units", if units.is_empty() {"Unavailable"} else {&units}, ""],
            ["File size", c.bytes.map(size).unwrap_or_else(|| "Unavailable".into()), ""],
            ["Last saved", c.mtime.map(|t| self.ago(t)).unwrap_or_else(|| "Unavailable".into()), ""]
        ]);
        blocks.push(block("kv", json!({"rows": rows})));
        if c.autosave_newer {
            blocks.push(block("text", json!({"text": "Auto-save is newer than the file: opening asks which to keep.",
                                             "tone": "warn"})));
        }
        match &c.links {
            None => {
                blocks.push(block("caption", json!({"text": "Linked files"})));
                blocks.push(block("text", json!({"text": "Not checked", "tone": "faint"})));
            }
            Some(links) if !links.is_empty() => {
                let states: Vec<&str> = links.iter().map(|l| l.status.as_str()).collect();
                let bad = states.iter().filter(|s| **s != "found").count();
                let right = if bad > 0 { format!("{} · {bad} to look at", links.len()) } else { links.len().to_string() };
                blocks.push(block("caption", json!({"text": "Linked files", "right": right})));
                let items: Vec<Value> = links.iter().zip(&states).map(|(l, s)| {
                    let (dot, words) = match *s { "found" => ("ok", "Found"), "changed" => ("warn", "Changed on disk"),
                                                  _ => ("error", "Missing") };
                    let found = l.found.clone().unwrap_or_default();
                    prune(json!({"id": "", "label": name_of(&l.path), "depth": l.depth, "dot": dot, "dot_text": words,
                         "kind": "file", "fixed": true,
                        "tip": if found.is_empty() { l.path.clone() } else { found }}))
                }).collect();
                blocks.push(block("rows", json!({"items": items})));
            }
            _ => blocks.push(block("text", json!({"text":"Linked files · None"}))),
        }

        detail(blocks)
    }

    /// The sheet spec and its referenced textures.
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
        let visible = |key: &str| key != "missing" || st.missing_open;
        let order: Vec<&str> = sections.iter().filter(|(key, _)| visible(key)).flat_map(|(_, rows)| rows.iter().map(|(id, _)| id.as_str())).collect();
        let focus = match &st.focus {
            Some(f) if order.contains(&f.as_str()) => Some(f.clone()),
            _ => sections.iter().filter(|(key, _)| matches!(*key, "pinned" | "recent"))
                .find_map(|(_, rows)| rows.first().map(|(id, _)| id.clone()))
                .or_else(|| order.first().map(|s| s.to_string())),
        };
        let mut blocks = vec![];

        for (key, rows) in &sections {
            if *key == "recent" && !st.projects_tab && st.files.is_empty() && st.filter.is_empty() {
                blocks.push(block("caption", json!({"text":"Recent models"})));
                blocks.push(block("text", json!({"text":"Models you open appear here.", "tone":"faint"})));
            }
            if rows.is_empty() {
                continue;
            }
            let caption = SECTIONS.iter().find(|(k, _)| k == key).map_or("", |s| s.1);
            if *key == "missing" {
                blocks.push(block("disclosure", json!({"text": format!("{caption} ({})", rows.len()),
                    "id": "toggle_missing", "expanded": st.missing_open})));
                if !st.missing_open { continue; }
            } else {
                blocks.push(block("caption", json!({"text": caption})));
            }
            let items: Vec<Value> = rows.iter().map(|(id, kind)| {
                self.row(id, kind, &mut icons)
            }).collect();
            blocks.push(block("rows", json!({"items": items, "id": key,
                                             "tone": "start-list"})));
        }
        let found = sections.iter().flat_map(|(_, rows)| rows.iter()).find(|(id, _)| Some(id) == focus.as_ref());
        let detail = self.preview(found, &mut icons);
        let empty = if st.projects_tab && st.filter.is_empty() { "No recent projects".to_owned() } else if st.filter.is_empty() { "No recent files yet.".to_owned() }
                    else { format!("Nothing matches “{}”.", st.filter) };
        let mut verbs = vec![verb("openfile", if st.projects_tab {"Open project…"} else {"Open file…"}, "", 0)];
        if !st.projects_tab { verbs.insert(0, verb("new", "New model", "", 0)); }
        let primary = match found.map(|(_, k)| k) {
            Some(Kind::Tpl(_)) => Some(("open", "Create model")),
            Some(Kind::File(r) | Kind::Proj(r)) if self.missing(&r.path) => Some(("locate", "Locate…")),
            Some(Kind::File(_)) => Some(("open", "Open model")),
            Some(Kind::Proj(_)) => Some(("open", "Open project")),
            None if st.projects_tab => Some(("project","New project…")),
            None => Some(("new", "Create model")),
        };
        if st.projects_tab && found.is_some() {
            verbs.push(verb("project", "New project…", "", 0));
        }
        let links = [("libraries", "Library"), ("prefs", "Settings"), ("extensions", "Extensions"), ("help", "Help")];
        json!({
            "id": "start", "title": "Start", "closable": st.back, "width": 1180.0, "backdrop_icon": backdrop,
            "default_verb": if st.projects_tab && found.is_none() {"project"} else if found.is_none() {"new"} else {""},
            "tabs": [{"id":"models","label":"Models","on":!st.projects_tab},{"id":"projects","label":"Projects","on":st.projects_tab}],
            "verbs": verbs, "menus": [],
            "side": [], "side_width": 0.0,
            "content": {"title": st.pending, "sub": "", "empty": empty, "width": 0.0, "blocks": blocks},
            "detail": detail,
            "footer": {"links": links.iter().map(|(i, l)| verb(i, l, "", 0)).collect::<Vec<_>>(), "hint": "", "action": primary.map(|(id,label)| verb(id,label,"↩",PRIMARY))},
            "focus": focus, "force_focus": st.focus != focus,
            "search": {"placeholder": if st.projects_tab {"Filter projects"} else {"Filter models and templates"}, "key": "", "text": "", "force": false},
            "icons": icons,
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
    fn selection_owns_the_only_primary_action() {
        let files = json!([{"path":"file.built"},{"path":"lost.built","pinned":true}]);
        let cache = json!({"lost.built":{"exists":false}});
        for (focus, action) in [("tpl:blank","Create model"), ("file:file.built","Open model"), ("file:lost.built","Locate…")] {
            let st = state(json!({"files":files,"cache":cache,"focus":focus,"missing_open":true}));
            let spec = View::new(&st).sheet();
            assert_eq!(spec["focus"], focus);
            assert_eq!(spec["footer"]["action"]["label"], action);
            assert_eq!(spec["footer"]["action"]["role"], 1);
            assert!(spec["verbs"].as_array().unwrap().iter().all(|v| v["role"] != 1));
            serde_json::from_value::<crate::ui::large_sheet::Spec>(spec).unwrap();
        }
        let spec = View::new(&State::default()).sheet();
        assert_eq!(spec["focus"], "tpl:blank");
        assert!(spec["menus"].as_array().unwrap().is_empty());
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
