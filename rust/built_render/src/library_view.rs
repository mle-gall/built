//! The Library sheet's view model (library_host.rs): the scopes and
//! categories on the side, the facet and the search, the sort, how the
//! content groups, each tile's words and menu, the details of a material,
//! and which of the model's materials only linked files wear.
//!
//! Python gathers the items (dicts, `Item`) from the document, the Rust
//! catalogue and My library, counts their uses, sends them with the
//! browsing state (`Browse`), and draws what comes back onto the sheet; it
//! keeps the thumbnails, the online catalogue and what
//! the verbs do.
pub(crate) mod facets;
use pyo3::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};

/// A null where a value is expected reads as the default (items come from
/// Python dicts, the catalogue and settings).
fn lenient<'de, D: serde::Deserializer<'de>, T: Default + Deserialize<'de>>(d: D) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// A count, also when it came as a float.
fn count<'de, D: serde::Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    Ok(Option::<f64>::deserialize(d)?.map_or(0, |f| f as i64))
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Item {
    #[serde(deserialize_with = "lenient")]
    pub key: String,
    #[serde(deserialize_with = "lenient")]
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub source: String,
    #[serde(deserialize_with = "lenient")]
    pub category: String,
    /// colour | image; None: any facet keeps it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(deserialize_with = "count")]
    pub uses: i64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub defaults: Vec<(String, String, String)>,
    pub hex: Option<String>,
    pub thumb: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    pub classes: Vec<String>,
    pub collection: String,
    pub files: Vec<String>,
    pub explicit_class: String,
    #[serde(deserialize_with = "lenient")]
    pub verbs: Vec<String>,
}

impl Item {
    fn shown(&self) -> &str {
        self.label.as_deref().filter(|l| !l.is_empty()).unwrap_or(&self.name)
    }
}

#[derive(Deserialize, Default, Debug)]
#[serde(default)]
pub struct Browse {
    pub link_errors: HashMap<String, String>,
    pub sort: String,
    pub group: String,
    pub facet: String,
    pub filter: String,
    pub scope: String,
    pub fold: Vec<String>,
    pub recent: Vec<String>,
    pub cursor: Option<String>,
}

pub const SORTS: [(&str, &str); 3] = [("name", "Name"), ("used", "Most used"), ("recent", "Recently used")];
/// The side list's scopes, in order (ids "s:<scope>"; a category
/// "c:<scope>:<name>"; a linked file "s:linked:<path>").
pub const SCOPES: [(&str, &str); 4] = [("model", "In this model"), ("recent", "Recently used"),
    ("mine", "My library"), ("bundled", "Bundled")];
/// Browsing by base: the Materials' sort menu.
pub const FACETS: [(&str, &str); 3] = [("facet:all", "Every base"), ("facet:colour", "Colour bases"),
    ("facet:image", "Image bases")];
/// Verb ids and their menu labels (a type relabels "apply").
pub const VERBS: [(&str, &str); 12] = [("apply", "Use"), ("edit", "Edit in the inspector"), ("duplicate", "Duplicate"),
    ("rename", "Rename"), ("keep", "Add to My library"), ("unkeep", "Remove from My library"),
    ("select", "Select uses"), ("replace", "Replace selection with this"), ("keepfile", "Keep when unused"),
    ("unkeepfile", "Stop keeping in file"), ("delete", "Delete…"), ("-", "")];
/// A search lists at most this many items per group (the category shows them all).


/// What differs per resource type: (id, apply label, empty text).
const TYPES: [(&str, &str, &str); 5] = [
    ("presets","Use in Sweep","No presets in this collection."),
    ("materials", "Paint with it", "No materials yet."),
    ("components", "Place a copy", "No components yet."),
    ("families", "Place type", "Import a collection folder to browse its family types."),
    ("profiles", "Use in Sweep", "No profiles in this collection."),
];

fn kind(t: &str) -> (&'static str, &'static str, &'static str) {
    TYPES.iter().copied().find(|k| k.0 == t).unwrap_or(("", "Use", ""))
}

/// The verbs of a type, (id, label).
pub fn verbs_of(t: &str) -> Vec<(String, String)> {
    let apply = kind(t).1;
    let mut out: Vec<(String, String)> = VERBS.iter().filter(|v| v.0 != "-")
        .map(|(k, v)| (k.to_string(), if *k == "apply" { apply } else { v }.to_string())).collect();
    if t == "materials" {
        out.push(("name".into(), "Name these colours".into()));
    }
    out
}

// ---- Natural order ---------------------------------------------------------------------------
#[derive(PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Part {
    Text(String),
    // (digits without leading zeros, by length then text): any size.
    Number(usize, String),
}

/// "a10b" → [a, 10, b], lower case: numbers compare as numbers.
fn natural(text: &str) -> Vec<Part> {
    let lower = text.to_lowercase();
    let mut out = vec![];
    let mut cur = String::new();
    let mut digits = false;
    let flush = |cur: &mut String, digits: bool, out: &mut Vec<Part>| {
        if digits {
            let t = cur.trim_start_matches('0').to_string();
            out.push(Part::Number(t.len(), t));
        } else {
            out.push(Part::Text(std::mem::take(cur)));
        }
        cur.clear();
    };
    for ch in lower.chars() {
        let d = ch.is_ascii_digit();
        if d != digits {
            flush(&mut cur, digits, &mut out);
            digits = d;
        }
        cur.push(ch);
    }
    flush(&mut cur, digits, &mut out);
    out
}

fn by_name(a: &str, b: &str) -> Ordering {
    natural(a).cmp(&natural(b))
}

pub fn sorted<'a>(mut items: Vec<&'a Item>, sort: &str, recent: &[String]) -> Vec<&'a Item> {
    let rank: HashMap<&str, usize> = recent.iter().enumerate().map(|(i, k)| (k.as_str(), i)).collect();
    match sort {
        "used" => items.sort_by(|a, b| b.uses.cmp(&a.uses).then_with(|| by_name(a.shown(), b.shown()))),
        "recent" => items.sort_by(|a, b| {
            let r = |e: &Item| rank.get(e.key.as_str()).copied().unwrap_or(rank.len());
            r(a).cmp(&r(b)).then_with(|| by_name(a.shown(), b.shown()))
        }),
        _ => items.sort_by(|a, b| by_name(a.shown(), b.shown())),
    }
    items
}

// ---- What the sheet lists --------------------------------------------------------------------
/// The items the facet and the search keep (``is:colour`` / ``is:image``
/// tokens browse by base, like the facet).
pub fn filtered<'a>(items: impl IntoIterator<Item = &'a Item>, st: &Browse) -> Vec<&'a Item> {
    let lower = st.filter.to_lowercase();
    let mut base = None;
    let mut words = vec![];
    for w in lower.split_whitespace() {
        match w {
            "is:colour" | "is:color" => base = base.or(Some("colour")),
            "is:image" => base = base.or(Some("image")),
            _ => words.push(w),
        }
    }
    let base = base.or(match st.facet.as_str() { "colour" => Some("colour"), "image" => Some("image"), _ => None });
    items.into_iter().filter(|e| {
        if let (Some(b), Some(eb)) = (base, e.base.as_deref()) {
            if eb != b {
                return false;
            }
        }
        if words.is_empty() {
            return true;
        }
        let hay = format!("{} {} {} {} {}", e.name, e.label.as_deref().unwrap_or(""), e.short.as_deref().unwrap_or(""),
                          e.category, e.file.as_deref().unwrap_or("")).to_lowercase();
        words.iter().all(|w| hay.contains(w))
    }).collect()
}

/// The items of one side row.
pub fn in_scope<'a>(items: &'a [Item], scope: &str, recent: &[String]) -> Vec<&'a Item> {
    if scope == "s:all" { return items.iter().collect(); }
    if scope == "s:mine" {
        return items.iter().filter(|e| e.source == "mine" || e.source.starts_with("collection:")).collect();
    }
    if scope == "s:recent" {
        let by_key: HashMap<&str, &Item> = items.iter().map(|e| (e.key.as_str(), e)).collect();
        return recent.iter().filter_map(|k| by_key.get(k.as_str()).copied()).collect();
    }
    if let Some(path) = scope.strip_prefix("s:linked:") {
        return items.iter().filter(|e| e.source == "linked" && e.file.as_deref() == Some(path)).collect();
    }
    if let Some(rest) = scope.strip_prefix("c:") {
        let (sec, cat) = rest.split_once(':').unwrap_or((rest, ""));
        return items.iter().filter(|e| e.source == sec && e.category == cat).collect();
    }
    let sec = scope.get(2..).unwrap_or("");
    items.iter().filter(|e| e.source == sec).collect()
}

fn base_name(path: &str) -> String {
    path.replace('\\', "/").rsplit('/').next().unwrap_or("").to_owned()
}

/// A side row: (id, label, count, depth, kind, fold); a caption has no id.
pub type SideRow = (String, String, usize, u8, String, Option<bool>);

/// The side list. Bundled folds open on its categories.
pub fn side_rows(items: &[Item], st: &Browse) -> Vec<SideRow> {
    let mut out = vec![];
    for (sec, title) in SCOPES {
        let group = in_scope(items, &format!("s:{sec}"), &st.recent);
        let mut cats: Vec<&str> = vec![];
        if matches!(sec, "model" | "bundled" | "mine") {
            let set: BTreeSet<&str> = group.iter().map(|e| e.category.as_str()).filter(|c| !c.is_empty()).collect();
            cats = set.into_iter().collect();
            cats.sort_by(|a, b| by_name(a, b));
        }
        let fold = (sec == "bundled" && !cats.is_empty()).then(|| !st.fold.iter().any(|f| f == &format!("s:{sec}")));
        out.push((format!("s:{sec}"), title.to_owned(), group.len(), 0, String::new(), fold));
        if !cats.is_empty() && fold != Some(false) {
            for cat in cats {
                let n = group.iter().filter(|e| e.category == cat).count();
                out.push((format!("c:{sec}:{cat}"), cat.to_owned(), n, 1, String::new(), None));
            }
        }
    }
    out.push(("s:all".into(), "All collections".into(), items.len(), 0, String::new(), None));
    let mut files: Vec<(String, usize)> = vec![];
    for e in items.iter().filter(|e| e.source == "linked") {
        let Some(f) = e.file.as_deref().filter(|f| !f.is_empty()) else { continue };
        match files.iter_mut().find(|(p, _)| p == f) {
            Some(row) => row.1 += 1,
            None => files.push((f.to_owned(), 1)),
        }
    }
    if !files.is_empty() {
        out.push((String::new(), "Linked files · read-only".into(), 0, 0, "caption".into(), None));
        files.sort_by(|a, b| by_name(&base_name(&a.0), &base_name(&b.0)));
        for (path, n) in files {
            out.push((format!("s:linked:{path}"), base_name(&path), n, 0, "file".into(), None));
        }
    }
    out
}

pub fn scope_title(scope: &str) -> String {
    if scope == "s:all" { return "All collections".into(); }
    if let Some(path) = scope.strip_prefix("s:linked:") {
        return base_name(path);
    }
    if let Some(rest) = scope.strip_prefix("c:") {
        return rest.split_once(':').map_or("", |x| x.1).to_owned();
    }
    SCOPES.iter().find(|s| Some(s.0) == scope.get(2..)).map_or("", |s| s.1).to_owned()
}

fn searching(st: &Browse) -> bool {
    let f = st.filter.trim();
    !f.is_empty() && !f.starts_with("is:")
}

fn categories<'a>(members: &[&'a Item]) -> Vec<&'a str> {
    let mut cats: Vec<&str> = members.iter().map(|e| e.category.as_str()).collect::<BTreeSet<_>>().into_iter().collect();
    // The uncategorised first, then by name.
    cats.sort_by(|a, b| (!a.is_empty()).cmp(&!b.is_empty()).then_with(|| by_name(a, b)));
    cats
}

/// The content: (caption, sub, items). Browsing shows the side row's items
/// (the model's grouped by category); a search shows every match, grouped
/// by where it comes from.
pub fn groups_for<'a>(items: &'a [Item], st: &Browse) -> Vec<(String, String, Vec<&'a Item>)> {
    let scope = if st.scope.is_empty() { "s:model" } else { st.scope.as_str() };
    if searching(st) && !scope.starts_with("s:collection:") && !items.iter().any(|e| e.source.starts_with("collection:")) {
        let mut out = vec![];
        let found = filtered(items, st);
        for (sec, title) in SCOPES {
            if sec == "recent" {
                continue;
            }
            let group: Vec<&Item> = found.iter().copied().filter(|e| e.source == sec).collect();
            for cat in categories(&group) {
                let members = sorted(group.iter().copied().filter(|e| e.category == cat).collect(), &st.sort, &st.recent);
                let cap = if cat.is_empty() { title.to_owned() } else { format!("{title} · {cat}") };
                out.push((cap, members.len().to_string(), members));
            }
        }
        let linked: Vec<&Item> = found.iter().copied().filter(|e| e.source == "linked").collect();
        let paths: BTreeSet<&str> = linked.iter().map(|e| e.file.as_deref().unwrap_or("")).collect();
        for path in paths {
            let members = sorted(linked.iter().copied().filter(|e| e.file.as_deref().unwrap_or("") == path).collect(),
                                 &st.sort, &st.recent);
            out.push((format!("{} · read-only", path.rsplit('/').next().unwrap_or("")), members.len().to_string(),
                      members));
        }
        return out;
    }
    let mut members = filtered(in_scope(items, scope, &st.recent), st);
    if scope != "s:recent" {
        members = sorted(members, &st.sort, &st.recent);
    }
    if matches!(scope, "s:model" | "s:mine" | "s:bundled") {
        let cats = categories(&members);
        if cats.len() > 1 || cats.first().is_some_and(|c| !c.is_empty()) {
            return cats.into_iter().map(|c| (c.to_owned(), String::new(),
                members.iter().copied().filter(|e| e.category == c).collect())).collect();
        }
    }
    vec![(String::new(), String::new(), members)]
}

/// The registry materials only linked parts wear move out of «In this
/// model» (editing them there would change a file that is not this one),
/// and each linked file lists the ones its parts wear, read-only.
/// ``per_file``: [(file, [(material, objects)])]; ``own``: the model's uses.
pub fn linked_split(model: Vec<Item>, per_file: &[(String, Vec<(String, i64)>)], own: &HashMap<String, i64>)
                    -> (Vec<Item>, Vec<Item>) {
    if per_file.is_empty() {
        return (model, vec![]);
    }
    let mut total: HashMap<&str, i64> = HashMap::new();
    for (_, counts) in per_file {
        for (name, n) in counts {
            *total.entry(name).or_insert(0) += n;
        }
    }
    let by_name: HashMap<&str, &Item> = model.iter().map(|e| (e.name.as_str(), e)).collect();
    let mut linked = vec![];
    for (path, counts) in per_file {
        for (name, n) in counts {
            if let Some(e) = by_name.get(name.as_str()) {
                linked.push(Item { key: format!("lk:{path}|{name}"), source: "linked".into(), file: Some(path.clone()),
                                   uses: *n, category: String::new(), verbs: vec!["apply".into(), "select".into()],
                                   ..(*e).clone() });
            }
        }
    }
    let keep = model.iter().filter(|e| !(e.key.starts_with("m:") && total.get(e.name.as_str())
        .is_some_and(|t| own.get(&e.name).copied().unwrap_or(0) <= *t))).cloned().collect();
    (keep, linked)
}

/// The name of an image as the user gave it (a stored copy is
/// ``<hash>-<name>``).
pub(crate) fn file_stem(path: &str) -> String {
    let base = base_name(path);
    if let Some((head, rest)) = base.split_once('-') {
        if head.len() >= 12 && head.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')) {
            return rest.to_owned();
        }
    }
    base
}

// ---- The sheet's pieces ----------------------------------------------------------------------
fn verb(id: &str, label: &str, key: &str, role: i64) -> Value {
    json!({"id": id, "label": label, "key": key, "role": role, "enabled": true, "tip": ""})
}
const PRIMARY: i64 = 1;
const QUIET: i64 = 3;

fn prune(v: Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(m.into_iter().filter(|(k, v)| {
            k == "id" || k == "label" || !matches!(v, Value::Null | Value::Bool(false))
                && v.as_str() != Some("") && v.as_f64() != Some(0.0) && v.as_array().map_or(true, |a| !a.is_empty())
        }).collect()),
        other => other,
    }
}

fn block(kind: &str, mut fields: Value) -> Value {
    fields["kind"] = json!(kind);
    prune(fields)
}

fn icon(e: &Item, thumbs: &mut Vec<String>) -> String {
    match e.thumb.as_deref().filter(|t| !t.is_empty()) {
        Some(t) if t.starts_with("component:") && e.key.starts_with("cm:") => t.into(),
        Some(t) => {
            if !thumbs.iter().any(|x| x == t) {
                thumbs.push(t.to_owned());
            }
            format!("lib:{t}")
        }
        None => String::new(),
    }
}

fn objects(n: i64) -> String {
    format!("{n} object{}", if n != 1 { "s" } else { "" })
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Input {
    #[serde(rename = "type")]
    pub kind: String,
    pub items: std::sync::Arc<[Item]>,
    pub st: Browse,
    /// The Paint tool's current material.
    pub current: Option<String>,
    /// The model's uses of each material (the details of a kept or bundled one).
    pub model_uses: HashMap<String, i64>,
    /// How many unnamed colours the model has.
    pub unnamed: usize,
    pub usage_pending: bool,
}

type SheetItem = crate::ui::large_sheet::Item;
pub(crate) type RowGroups = Vec<std::sync::Arc<[SheetItem]>>;

fn tile(t: &str, e: &Item, current: Option<&str>, thumbs: &mut Vec<String>, labels: &HashMap<String, String>) -> SheetItem {
    let mut menu: Vec<_> = e.verbs.iter().map(|v| if v == "-" { (v.clone(), String::new()) } else {
        (v.clone(), labels.get(v).cloned().unwrap_or_else(|| v.clone()))
    }).collect();
    while menu.last().is_some_and(|m| m.0 == "-") {
        menu.pop();
    }
    let mut shown = e.shown().to_owned();
    if e.source == "bundled" {
        if let Some(s) = e.short.as_deref().filter(|s| !s.is_empty()) {
            shown = s.to_owned();
        }
    }
    let sub = if t == "materials" {
        let mut s = if e.base.as_deref() == Some("image") { "image".to_owned() } else { "colour".to_owned() };
        if e.uses != 0 {
            s += &format!(" · used by {}", e.uses);
        }
        s
    } else if t == "presets" {
        e.defaults.iter().find(|d|d.0.starts_with("IFC class")).map(|d|d.1.clone()).unwrap_or_default()
    } else if t == "families" {
        e.category.clone()
    } else if e.uses != 0 {
        format!("{} cop{}", e.uses, if e.uses != 1 { "ies" } else { "y" })
    } else {
        String::new()
    };
    let host = e.defaults.iter().find(|r| r.0 == "Host").map(|r| &r.1);
    SheetItem {
        id: e.key.clone(), right: if t=="components" {if matches!(e.source.as_str(),"model"|"linked") {e.uses.to_string()} else {"—".into()}} else {host.cloned().unwrap_or_default()}, label: shown, sub: if t=="components"{facets::class(e).into()}else{sub},
        component_source: if t=="components" {facets::source(e).into()} else {String::new()},
        kind: if matches!(t, "components" | "families") { "placeable".into() } else { String::new() },
        icon: icon(e, thumbs), hex: e.hex.clone().unwrap_or_default(), menu,
        on: current.is_some_and(|c| c == e.name), tip: e.name.clone(), fold: -1, ..Default::default()
    }
}

pub(crate) fn detail(inp: &Input, e: &Item, thumbs: &mut Vec<String>) -> Vec<Value> {
    let mut out = vec![block("picture", json!({"icon": icon(e, thumbs), "hex": e.hex.clone().unwrap_or_default(),
                                               "size": 180.0}))];
    let where_ = match e.source.as_str() {
        "model" => "In this model".to_owned(),
        "mine" => "My library".to_owned(),
        "bundled" => "Bundled · read-only".to_owned(),
        "linked" => format!("{} · read-only", e.file.as_deref().unwrap_or("").rsplit('/').next().unwrap_or("")),
        _ => String::new(),
    };
    out.push(block("heading", json!({"text": e.shown(), "right": where_})));
    if inp.kind=="presets" {
        if e.verbs.iter().any(|v|v=="apply"){out.push(block("buttons",json!({"buttons":[verb("apply","Use in Sweep","↩",PRIMARY)],"align":"fill"})));}
        out.push(block("kv",json!({"rows":e.defaults})));
        let actions:Vec<_>=[("edit","Edit…"),("duplicate","Duplicate"),("export","Export pack…"),("delete","Delete…")].into_iter().filter(|(id,_)|e.verbs.iter().any(|v|v==id)).map(|(id,title)|verb(id,title,"",0)).collect();
        out.push(block("buttons",json!({"buttons":actions})));
        return out;
    }
    if inp.kind=="profiles" {
        out.push(block("buttons",json!({"buttons":[verb("apply","Use in Sweep","↩",PRIMARY)],"align":"fill"})));
        out.push(block("kv",json!({"rows":e.defaults})));
        return out;
    }
    if inp.kind == "families" {
        let category = e.defaults.iter().find(|r| r.0 == "Category").map(|r| r.1.to_ascii_lowercase());
        let action = match category.as_deref() { Some("door"|"doors") => "Place door", Some("window"|"windows") => "Place window", _ => "Place type" };
        out.push(block("buttons", json!({"buttons": [verb("apply", action, "↩", PRIMARY)], "align": "fill"})));
        out.push(block("kv", json!({"rows": e.defaults})));
        return out;
    }
    if inp.kind == "components" {
        out.push(block("kv", json!({"rows":[["IFC class", facets::class(e), ""],["Source", facets::source(e), ""]]})));
        return out;
    }
    let buttons = |list: Vec<Value>, align: &str| block("buttons", json!({"buttons": list, "align": align}));
    match e.key.split_once(':').map_or("", |x| x.0) {
        "uc" => {
            out.push(block("text", json!({"text": "A colour painted before materials had names (an older file). \
                                                   Name these makes each a material."})));
            out.push(block("kv", json!({"rows": [["Colour", e.name, ""], ["Used by", objects(e.uses), ""]]})));
            out.push(buttons(vec![verb("apply", "Paint with it", "↩", PRIMARY), verb("select", "Select uses", "", 0),
                                  verb("name", &format!("Name these ({})", inp.unnamed), "", 0)], "fill"));
        }
        "m" => {
            if inp.usage_pending {
                out.push(buttons(vec![verb("apply", "Paint with it", "↩", PRIMARY)], "fill"));
                return out;
            }
            out.push(block("kv", json!({"rows": [["Used by", objects(e.uses), ""]]})));
            out.push(buttons(vec![verb("apply", "Paint with it", "↩", PRIMARY), verb("edit", "Edit in inspector", "", 0),
                                  verb("select", "Select uses", "", 0)], "fill"));
            out.push(buttons(vec![verb("duplicate", "Duplicate", "", QUIET), verb("keep", "Add to My library", "", QUIET),
                                  verb("delete", "Delete…", "", QUIET)], "left"));
        }
        _ => {
            let base = match (e.base.as_deref(), e.thumb.as_deref()) {
                (Some("image"), Some(t)) if !t.is_empty() => format!("Image · {}", file_stem(t)),
                _ => format!("Colour {}", e.hex.as_deref().unwrap_or("").to_uppercase()),
            };
            let mut rows = vec![json!(["Base", base, ""])];
            if !e.category.is_empty() {
                rows.push(json!(["Category", e.category, ""]));
            }
            let used = inp.model_uses.get(&e.name).copied().unwrap_or(0);
            if !inp.usage_pending { rows.push(json!(["In this model", if used != 0 { format!("used by {used}") } else { "not used".into() }, ""])); }
            out.push(block("kv", json!({"rows": rows})));
            let mut verbs = vec![verb("apply", "Paint with it", "↩", PRIMARY)];
            if e.source == "mine" {
                verbs.push(verb("unkeep", "Remove from My library", "", 0));
            }
            if e.source == "linked" {
                out.push(block("text", json!({"text": "Edit it in its own file: the change comes in when the link reloads.",
                                              "tone": "faint"})));
                verbs.push(verb("opensource", &format!("Open {}", base_name(e.file.as_deref().unwrap_or(""))), "", 0));
            }
            out.push(buttons(verbs, "fill"));
        }
    }
    out
}

fn empty_text(t: &str, scope: &str) -> String {
    match scope {
        "s:mine" => "Keep a material for every project: Add to My library, on a material of this model.".into(),
        "s:recent" => "What you used last shows here.".into(),
        "s:model" => {
            let e = kind(t).2;
            if e.is_empty() { "Nothing of this type in the model yet.".into() } else { e.into() }
        }
        _ => "Nothing here.".into(),
    }
}

/// The sheet's pieces for the Materials or Components tab.
pub fn sheet(inp: &Input) -> Value { sheet_rows(inp, None) }

pub(crate) fn sheet_rows(inp: &Input, mut native: Option<&mut RowGroups>) -> Value {
    let st = &inp.st;
    let t = inp.kind.as_str();
    let mut thumbs = vec![];
    let label = SORTS.iter().find(|s| s.0 == st.sort).map_or("Name", |s| s.1);
    let facet = format!("facet:{}", if st.facet.is_empty() { "all" } else { &st.facet });
    let mut options: Vec<Value> = SORTS.iter().map(|(i, l)| verb(i, l, "", i64::from(*i == st.sort))).collect();
    let show = (t == "materials").then(|| {
        let options: Vec<_> = FACETS.iter().map(|(id, label)| verb(id, label, "", i64::from(*id == facet))).collect();
        let label = FACETS.iter().find(|(id, _)| *id == facet).unwrap_or(&FACETS[0]).1;
        json!({"id":"show", "label":format!("Show: {label}"), "on":facet, "options":options})
    });
    if t=="components" { options[1]["label"]=json!("Copies"); }
    let label=if t=="components" && st.sort=="used" {"Copies"}else{label};
    let sort = json!({"id": "sort", "label": format!("Sort: {label}"), "on": st.sort, "options": options});
    let faceted = t == "components" && st.scope != "s:online";
    let index = faceted.then(|| facets::index(&inp.items, st));
    let mut sides = if faceted {Vec::new()} else {side_rows(&inp.items, st)};
    if t == "families" { sides.retain(|r| matches!(r.0.as_str(), "s:all" | "s:mine")); }
    let mut scope = if st.scope.is_empty() { "s:model".to_owned() } else { st.scope.clone() };
    let online = t == "components" && scope == "s:online";
    if !online && !scope.starts_with("s:collection:") && !sides.iter().any(|r| !r.0.is_empty() && r.0 == scope) {
        scope = if t == "families" { "s:all" } else { "s:model" }.into();
    }
    let search = searching(st);
    let side: Vec<Value> = sides.iter().map(|(id, label, n, depth, kind, fold)| {
        if kind == "caption" {
            return json!({"id": "", "label": label, "kind": "caption"});
        }
        let mut item = prune(json!({"id": id, "label": label, "right": if *n != 0 { n.to_string() } else { String::new() },
            "depth": depth, "kind": kind, "on": *id == scope && !search}));
        if let Some(f) = fold {
            item["fold"] = json!(i64::from(*f));
        }
        item
    }).collect();
    let browse = Browse { link_errors: Default::default(), group: st.group.clone(), scope: scope.clone(), sort: st.sort.clone(), facet: st.facet.clone(), filter: st.filter.clone(),
                          fold: st.fold.clone(), recent: st.recent.clone(), cursor: None };
    let tokens = index.as_ref().map(|i|i.tokens.clone());
    let (side,groups) = index.map(|i|(i.side,i.groups)).unwrap_or_else(||(side,groups_for(&inp.items, &browse)));
    let shown: Vec<&Item> = groups.iter().flat_map(|g| g.2.iter().copied()).collect();
    let content = if faceted {
        json!({"title":format!("{} definitions · {} excluded",shown.len(),inp.items.len()-shown.len()),"empty":"No matching definitions."})
    } else if search {
        json!({"title": format!("“{}”", st.filter.trim()), "sub": format!("{} found", shown.len()), "empty": "Nothing matches."})
    } else {
        let n = in_scope(&inp.items, &scope, &st.recent).len();
        let where_ = if scope.starts_with("c:bundled") { "Bundled" } else { "" };
        let sub = [where_.to_owned(), n.to_string()].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ");
        json!({"title": scope_title(&scope), "sub": sub, "empty": if t == "families" { kind(t).2.into() } else { empty_text(t, &scope) }})
    };
    let current = inp.current.as_deref();
    let labels = verbs_of(t).into_iter().collect();
    let groups: Vec<Value> = groups.iter().filter(|g| !g.2.is_empty()).map(|(cap, sub, members)| {
        let tiles = members.iter().map(|e| tile(t, e, current, &mut thumbs, &labels));
        let wire = if let Some(rows) = native.as_deref_mut() {
            rows.push(tiles.collect());
            Vec::new()
        } else {
            tiles.map(|it| prune(json!({"id": it.id, "right": it.right, "kind": it.kind, "label": it.label,
                "sub": it.sub, "component_source":it.component_source, "icon": it.icon, "hex": it.hex, "menu": it.menu, "on": it.on, "tip": it.tip}))).collect()
        };
        json!({"caption": cap, "right": sub, "tiles": wire})
    }).collect();
    let keys: HashSet<&str> = shown.iter().map(|e| e.key.as_str()).collect();
    let cursor = match &st.cursor {
        Some(c) if keys.contains(c.as_str()) => Some(c.clone()),
        _ => shown.first().map(|e| e.key.clone()),
    };
    let blocks = cursor.as_deref().and_then(|k| inp.items.iter().find(|e| e.key == k))
        .map(|e| detail(inp, e, &mut thumbs)).unwrap_or_default();
    json!({"scope": if faceted {"s:all"} else {&scope}, "tokens":tokens, "matches":faceted.then(||shown.iter().map(|e|&e.key).collect::<Vec<_>>()), "online": online, "searching": search, "search": format!("Search all {t}"), "sort": sort, "show": show,
           "side": side, "content": content, "groups": groups, "type_actions": inp.kind=="presets" || faceted || (!search && scope.starts_with("s:model")),
           "cursor": cursor, "detail": blocks, "thumbs": thumbs})
}

// ---- Python ----------------------------------------------------------------------------------
fn err(e: serde_json::Error) -> PyErr {
    pyo3::exceptions::PyValueError::new_err(e.to_string())
}

fn items_of(json: &str) -> PyResult<Vec<Item>> {
    serde_json::from_str(json).map_err(err)
}

#[pyfunction]
pub fn library_sheet(input: &str) -> PyResult<String> {
    Ok(sheet(&serde_json::from_str(input).map_err(err)?).to_string())
}

/// [(caption, sub, [item keys])]
#[pyfunction]
pub fn library_groups(items: &str, st: &str) -> PyResult<Vec<(String, String, Vec<String>)>> {
    let items = items_of(items)?;
    let st: Browse = serde_json::from_str(st).map_err(err)?;
    Ok(groups_for(&items, &st).into_iter()
        .map(|(c, s, m)| (c, s, m.into_iter().map(|e| e.key.clone()).collect())).collect())
}

/// The keys the facet and the search keep.
#[pyfunction]
pub fn library_filtered(items: &str, st: &str) -> PyResult<Vec<String>> {
    let items = items_of(items)?;
    let st: Browse = serde_json::from_str(st).map_err(err)?;
    Ok(filtered(&items, &st).into_iter().map(|e| e.key.clone()).collect())
}

#[pyfunction]
pub fn library_side(items: &str, st: &str) -> PyResult<Vec<SideRow>> {
    let items = items_of(items)?;
    let st: Browse = serde_json::from_str(st).map_err(err)?;
    Ok(side_rows(&items, &st))
}

/// (the model's items that stay, the linked files' items) as JSON.
#[pyfunction]
pub fn library_linked(model: &str, per_file: &str, own: &str) -> PyResult<(String, String)> {
    let per_file: Vec<(String, Vec<(String, i64)>)> = serde_json::from_str(per_file).map_err(err)?;
    let own: HashMap<String, i64> = serde_json::from_str(own).map_err(err)?;
    let (keep, linked) = linked_split(items_of(model)?, &per_file, &own);
    let dump = |v: &[Item]| serde_json::to_string(v).unwrap_or_else(|_| "[]".into());
    Ok((dump(&keep), dump(&linked)))
}

#[pyfunction]
pub fn library_verbs(kind: &str) -> Vec<(String, String)> {
    verbs_of(kind)
}

/// (sorts, facets, apply label, empty text) of a type.
#[pyfunction]
pub fn library_type(kind: &str) -> (Vec<(String, String)>, Vec<(String, String)>, String, String) {
    let k = self::kind(kind);
    let pairs = |v: &[(&str, &str)]| v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    (pairs(&SORTS), if kind == "materials" { pairs(&FACETS) } else { vec![] }, k.1.into(), k.2.into())
}

/// The order of ``names`` by ``uses``, most first, then by name (the
/// Paint strip).
#[pyfunction]
pub fn library_by_use(names: Vec<String>, uses: Vec<i64>) -> Vec<usize> {
    let mut order: Vec<usize> = (0..names.len()).collect();
    order.sort_by(|&a, &b| uses.get(b).cmp(&uses.get(a)).then_with(|| by_name(&names[a], &names[b])));
    order
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(library_sheet, m)?)?;
    m.add_function(wrap_pyfunction!(library_by_use, m)?)?;
    m.add_function(wrap_pyfunction!(library_groups, m)?)?;
    m.add_function(wrap_pyfunction!(library_filtered, m)?)?;
    m.add_function(wrap_pyfunction!(library_side, m)?)?;
    m.add_function(wrap_pyfunction!(library_linked, m)?)?;
    m.add_function(wrap_pyfunction!(library_verbs, m)?)?;
    m.add_function(wrap_pyfunction!(library_type, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn it(key: &str, name: &str, source: &str, category: &str, uses: i64) -> Item {
        Item { key: key.into(), name: name.into(), source: source.into(), category: category.into(), uses,
               base: Some("colour".into()), ..Default::default() }
    }

    #[test]
    fn natural_order_counts_numbers() {
        let mut names = vec!["Oak 10", "oak 9", "Ash", "Oak 010b"];
        names.sort_by(|a, b| by_name(a, b));
        assert_eq!(names, ["Ash", "oak 9", "Oak 10", "Oak 010b"]);
    }

    #[test]
    fn sort_facet_search_and_scopes() {
        let mut img = it("lib:w:1", "Ash", "bundled", "Wood", 0);
        img.base = Some("image".into());
        let items = vec![it("m:b", "Beta", "model", "", 1), it("m:a", "Alpha", "model", "", 5), img,
                         it("ral:1", "Slate", "bundled", "Stone", 0)];
        let st = Browse { sort: "used".into(), ..Default::default() };
        let keys = |g: Vec<(String, String, Vec<&Item>)>| -> Vec<(String, Vec<String>)> {
            g.into_iter().map(|(c, _, m)| (c, m.into_iter().map(|e| e.name.clone()).collect())).collect()
        };
        assert_eq!(keys(groups_for(&items, &st)), [(String::new(), vec!["Alpha".to_owned(), "Beta".to_owned()])]);
        let st = Browse { scope: "s:bundled".into(), facet: "image".into(), ..Default::default() };
        assert_eq!(keys(groups_for(&items, &st)), [("Wood".to_owned(), vec!["Ash".to_owned()])]);
        let st = Browse { filter: "a".into(), ..Default::default() };
        assert_eq!(keys(groups_for(&items, &st)).iter().map(|g| g.0.as_str()).collect::<Vec<_>>(),
                   ["In this model", "Bundled · Stone", "Bundled · Wood"]);
        let st = Browse { fold: vec!["s:bundled".into()], ..Default::default() };
        let side: Vec<String> = side_rows(&items, &st).into_iter().map(|r| r.0).collect();
        assert_eq!(side, ["s:model", "s:recent", "s:mine", "s:bundled", "s:all"]);
        let st = Browse::default();
        assert!(side_rows(&items, &st).iter().any(|r| r.0 == "c:bundled:Stone"));
    }

    #[test]
    fn materials_only_linked_parts_wear_move_out() {
        let model = vec![it("m:rail", "Rail", "model", "", 3), it("m:deck", "Deck", "model", "", 9)];
        let per_file = vec![("/w/Ride.igz".to_owned(), vec![("Rail".to_owned(), 3), ("Deck".to_owned(), 2)])];
        let own: HashMap<String, i64> = [("Rail".to_owned(), 3), ("Deck".to_owned(), 9)].into();
        let (keep, linked) = linked_split(model, &per_file, &own);
        assert_eq!(keep.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(), ["m:deck"]);
        assert_eq!(linked.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(), ["lk:/w/Ride.igz|Rail", "lk:/w/Ride.igz|Deck"]);
        assert_eq!(linked[1].uses, 2);
    }

    #[test]
    fn the_sheet_parts_read() {
        let inp = Input { kind: "materials".into(), items: vec![it("m:a", "Alpha", "model", "", 2)].into(),
                          current: Some("Alpha".into()), ..Default::default() };
        let v = sheet(&inp);
        assert_eq!(v["cursor"], "m:a");
        assert_eq!(v["groups"][0]["tiles"][0]["sub"], "colour · used by 2");
        assert_eq!(v["groups"][0]["tiles"][0]["on"], true);
        assert_eq!(v["detail"][2]["rows"][0], json!(["Used by", "2 objects", ""]));
        assert_eq!(v["sort"]["options"].as_array().unwrap().len(), SORTS.len());
        assert_eq!(v["show"]["options"].as_array().unwrap().len(), FACETS.len());
        assert_eq!(file_stem("/x/0123456789abcdef-oak.png"), "oak.png");
    }
}
