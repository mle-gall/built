// SPDX-License-Identifier: GPL-3.0-or-later
//! The `.builtproject` manifest (docs/builtproject-format.md §1, §2, §4, §5):
//! read, validate, write atomically, merge on write, convert an `.igzproj`
//! in place, and find a model's project from its pointer.
//!
//! The registry is opaque here: it is stored and merged as JSON, and
//! `core.bim_registry.validate` (Python, by design) checks its contents.
use serde_json::{Map, Value};
use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SUFFIX: &str = "builtproject";
pub const LEGACY_SUFFIX: &str = "igzproj";
pub const VERSION: u64 = 1;
pub const ROLES: [&str; 3] = ["host", "linked", "reference"];

pub type Result<T> = std::result::Result<T, String>;

fn has_suffix(path: &Path, suffix: &str) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case(suffix))
}

pub fn is_manifest(path: &Path) -> bool {
    has_suffix(path, SUFFIX)
}

pub fn is_legacy(path: &Path) -> bool {
    has_suffix(path, LEGACY_SUFFIX)
}

fn is_hex_id(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_uuid(s: &str) -> bool {
    let plain: String = s.chars().filter(|c| *c != '-').collect();
    let hyphens = s.len() - plain.len();
    (hyphens == 0 || (hyphens == 4 && s.len() == 36))
        && plain.len() == 32
        && plain.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A fresh project id: 128 bits from the standard library's per-process
/// random hash keys, the clock and the process id (no extra dependency).
pub fn new_id() -> String {
    let mut out = String::with_capacity(32);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    for round in 0..2u64 {
        let mut h = RandomState::new().build_hasher();
        h.write_u128(nanos);
        h.write_u32(std::process::id());
        h.write_u64(round);
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

fn obj<'a>(o: &'a Map<String, Value>, key: &str) -> Result<&'a Map<String, Value>> {
    o.get(key)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{key} must be an object"))
}

fn arr<'a>(o: &'a Map<String, Value>, key: &str) -> Result<&'a Vec<Value>> {
    o.get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{key} must be an array"))
}

fn check_select(select: Option<&Value>) -> Result<()> {
    let Some(select) = select else { return Ok(()) };
    if select.is_null() {
        return Ok(());
    }
    let s = select
        .as_object()
        .filter(|s| s.len() == 1 && (s.contains_key("uids") || s.contains_key("filter")))
        .ok_or("select requires exactly uids or filter")?;
    if let Some(uids) = s.get("uids") {
        let ok = uids.as_array().is_some_and(|u| {
            u.iter().all(|u| {
                u.is_string()
                    || u.as_array()
                        .is_some_and(|p| !p.is_empty() && p.iter().all(Value::is_string))
            })
        });
        if !ok {
            return Err("uids must be a list of strings or occurrence UID paths".into());
        }
    }
    Ok(())
}

/// What both formats share: files, links, registry and identity.
fn check_common(o: &Map<String, Value>) -> Result<()> {
    obj(o, "bim_registry")?;
    let identity = obj(o, "ifc_identity")?;
    let namespace = identity.get("namespace").and_then(Value::as_str).unwrap_or("");
    if !is_uuid(namespace) {
        return Err("ifc_identity.namespace must be a UUID".into());
    }
    let mut seen = std::collections::HashSet::new();
    for entry in arr(o, "files")? {
        let path = entry.get("path").and_then(Value::as_str);
        let role = entry.get("role").and_then(Value::as_str);
        match (path, role) {
            (Some(p), Some(r)) if ROLES.contains(&r) => {
                if !seen.insert(p.to_owned()) {
                    return Err(format!("files lists {p} twice"));
                }
            }
            _ => return Err("files require path and host, linked or reference role".into()),
        }
    }
    for edge in arr(o, "links")? {
        if ["host", "source", "placement_uid"]
            .iter()
            .any(|k| !edge.get(*k).is_some_and(Value::is_string))
        {
            return Err("links require host, source and placement_uid".into());
        }
        check_select(edge.get("select"))?;
    }
    Ok(())
}

/// The manifest's envelope rules (§1).
pub fn validate(value: &Value) -> Result<()> {
    let o = value
        .as_object()
        .ok_or("A Built project must be a JSON object")?;
    match o.get("built_project") {
        Some(v) if v.as_u64() == Some(VERSION) => {}
        Some(_) => return Err("Unsupported Built project version".into()),
        None => return Err("Not a Built project: built_project is missing".into()),
    }
    if o.contains_key("igz_format") || o.contains_key("built_format") {
        return Err("A Built project cannot also be a document".into());
    }
    if !o.get("id").and_then(Value::as_str).is_some_and(is_hex_id) {
        return Err("id must be 32 lowercase hexadecimal characters".into());
    }
    check_common(o)
}

/// The `.igzproj` rules (docs/project-file.md), as formats/project.py had them.
pub fn validate_legacy(value: &Value) -> Result<()> {
    let o = value.as_object().ok_or("A project must be a JSON object")?;
    if o.get("version").and_then(Value::as_u64) != Some(1) {
        return Err("Unsupported project version".into());
    }
    check_common(o)?;
    if let Some(session) = o.get("worksession").filter(|s| !s.is_null()) {
        let files = session
            .get("files")
            .and_then(Value::as_array)
            .ok_or("worksession requires a files array")?;
        let mut paths = Vec::new();
        for entry in files {
            let path = entry
                .get("path")
                .and_then(Value::as_str)
                .ok_or("worksession files require paths")?;
            if entry.get("camera").is_some_and(|c| !c.is_null() && !c.is_object()) {
                return Err("worksession camera must be an object".into());
            }
            paths.push(path);
        }
        let mut unique = paths.clone();
        unique.sort();
        unique.dedup();
        let active = session.get("active_file").and_then(Value::as_str);
        if unique.len() != paths.len() || active.is_some_and(|a| !paths.contains(&a)) {
            return Err("worksession requires unique files and an open active file".into());
        }
    }
    Ok(())
}

fn read_json(path: &Path) -> Result<Value> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn load(path: &Path) -> Result<Value> {
    let value = read_json(path)?;
    validate(&value)?;
    Ok(value)
}

pub fn load_legacy(path: &Path) -> Result<Value> {
    let value = read_json(path)?;
    validate_legacy(&value)?;
    Ok(value)
}

/// Manifest or `.igzproj`, by suffix, without converting anything.
pub fn load_any(path: &Path) -> Result<Value> {
    if is_legacy(path) {
        load_legacy(path)
    } else {
        load(path)
    }
}

/// Sorted keys, two spaces, a final newline; numbers keep their text.
pub fn to_text(value: &Value) -> String {
    let mut text = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into());
    text.push('\n');
    text
}

fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Temporary file beside `path`, flushed and synced, renamed over it.
pub fn write_atomic(path: &Path, text: &str) -> Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("project");
    let tmp = dir.join(format!(".{name}.{}.{}.tmp", std::process::id(), &new_id()[..8]));
    let result = (|| -> std::io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.flush()?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(|e| format!("{}: {e}", path.display()))?;
    sync_dir(dir);
    Ok(())
}

pub fn save(path: &Path, value: &Value) -> Result<()> {
    validate(value)?;
    write_atomic(path, &to_text(value))
}

/// (mtime in ns, size): what a write checks has not moved under it.
pub fn stamp(path: &Path) -> Option<(u128, u64)> {
    let meta = fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos();
    Some((mtime, meta.len()))
}

/// Read, change, write, retrying when the file moved between the read and
/// the rename (§4.3). `change` returns what the caller reports back.
pub fn update<T>(path: &Path, mut change: impl FnMut(&mut Value) -> Result<T>) -> Result<T> {
    let mut last = Err(String::new());
    for _ in 0..3 {
        let before = stamp(path);
        let mut value = load(path)?;
        let out = change(&mut value)?;
        validate(&value)?;
        let text = to_text(&value);
        if stamp(path) != before {
            last = Err(format!("{} kept changing while being written", path.display()));
            continue;
        }
        write_atomic(path, &text)?;
        return Ok(out);
    }
    last
}

/// A new manifest; refuses to replace an existing file.
pub fn create(path: &Path, registry: Value, identity: Value, id: Option<String>) -> Result<Value> {
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    let mut o = Map::new();
    o.insert("built_project".into(), Value::from(VERSION));
    o.insert("id".into(), Value::from(id.unwrap_or_else(new_id)));
    o.insert("files".into(), Value::Array(vec![]));
    o.insert("links".into(), Value::Array(vec![]));
    o.insert("bim_registry".into(), registry);
    o.insert("ifc_identity".into(), identity);
    let value = Value::Object(o);
    save(path, &value)?;
    Ok(value)
}

// ---- owner-scoped files and links (§4.1) ------------------------------------

/// Saving host `host`: its edges are replaced by `links`; `files` entries
/// missing from the manifest are added (an existing entry keeps its role).
pub fn sync_host(value: &mut Value, host: &str, files: &[Value], links: Vec<Value>) {
    let o = value.as_object_mut().expect("validated");
    let list = o.entry("files").or_insert_with(|| Value::Array(vec![]));
    let list = list.as_array_mut().expect("validated");
    for entry in files {
        let path = entry.get("path").and_then(Value::as_str);
        if path.is_some() && !list.iter().any(|e| e.get("path").and_then(Value::as_str) == path) {
            list.push(entry.clone());
        }
    }
    let edges = o.entry("links").or_insert_with(|| Value::Array(vec![]));
    let edges = edges.as_array_mut().expect("validated");
    edges.retain(|e| e.get("host").and_then(Value::as_str) != Some(host));
    edges.extend(links);
}

/// A project file found elsewhere: its entry and every link naming it
/// take the new path. False when no entry had the old one.
pub fn relocate(value: &mut Value, old: &str, new: &str) -> bool {
    let mut found = false;
    for entry in value["files"].as_array_mut().into_iter().flatten() {
        if entry["path"].as_str() == Some(old) {
            entry["path"] = Value::from(new);
            found = true;
        }
    }
    for edge in value["links"].as_array_mut().into_iter().flatten() {
        for key in ["host", "source"] {
            if edge[key].as_str() == Some(old) {
                edge[key] = Value::from(new);
            }
        }
    }
    found
}

// ---- registry merge (§4.2) ---------------------------------------------------

/// Three-way merge of `ours` into `theirs` against `base`, by section and
/// then by key of an object section. Conflicting entries take ours and are
/// returned as `section` or `section.key`.
pub fn merge_registry(base: &Value, ours: &Value, theirs: &Value) -> (Value, Vec<String>) {
    let empty = Map::new();
    let b = base.as_object().unwrap_or(&empty);
    let o = ours.as_object().unwrap_or(&empty);
    let t = theirs.as_object().unwrap_or(&empty);
    let mut out = t.clone();
    let mut conflicts = Vec::new();
    let mut sections: Vec<&String> = b.keys().chain(o.keys()).chain(t.keys()).collect();
    sections.sort();
    sections.dedup();
    for section in sections {
        let (bs, os, ts) = (b.get(section), o.get(section), t.get(section));
        if os == bs {
            continue; // we did not touch it: theirs stands
        }
        if ts == bs || ts == os {
            set(&mut out, section, os.cloned());
            continue;
        }
        // Both changed the section: go one level down when all are objects.
        let as_map = |v: Option<&Value>| match v {
            None => Some(empty.clone()),
            Some(Value::Object(m)) => Some(m.clone()),
            _ => None,
        };
        match (as_map(bs), as_map(os), as_map(ts)) {
            (Some(bm), Some(om), Some(tm)) if os.is_some() && ts.is_some() => {
                let mut merged = tm.clone();
                let mut keys: Vec<&String> = bm.keys().chain(om.keys()).chain(tm.keys()).collect();
                keys.sort();
                keys.dedup();
                for key in keys {
                    let (bk, ok, tk) = (bm.get(key), om.get(key), tm.get(key));
                    if ok == bk {
                        continue;
                    }
                    if tk != bk && tk != ok {
                        conflicts.push(format!("{section}.{key}"));
                    }
                    set(&mut merged, key, ok.cloned());
                }
                out.insert(section.clone(), Value::Object(merged));
            }
            _ => {
                conflicts.push(section.clone());
                set(&mut out, section, os.cloned());
            }
        }
    }
    (Value::Object(out), conflicts)
}

fn set(map: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    match value {
        Some(v) => {
            map.insert(key.to_owned(), v);
        }
        None => {
            map.remove(key);
        }
    }
}

/// Merge and write the registry; the conflicts, if any.
pub fn write_registry(path: &Path, base: &Value, ours: &Value) -> Result<Vec<String>> {
    update(path, |value| {
        let theirs = value.get("bim_registry").cloned().unwrap_or(Value::Object(Map::new()));
        let (merged, conflicts) = merge_registry(base, ours, &theirs);
        value["bim_registry"] = merged;
        Ok(conflicts)
    })
}

// ---- conversion (§5) ---------------------------------------------------------

#[derive(Debug, PartialEq)]
pub enum Converted {
    /// The `.igzproj` became this manifest and is gone.
    Converted(PathBuf),
    /// A manifest beside it already existed; nothing was written.
    AlreadyConverted(PathBuf),
}

impl Converted {
    pub fn path(&self) -> &Path {
        match self {
            Converted::Converted(p) | Converted::AlreadyConverted(p) => p,
        }
    }
}

pub fn sibling(path: &Path) -> PathBuf {
    path.with_extension(SUFFIX)
}

/// The manifest an `.igzproj` value becomes: every field kept except
/// `version`, plus `built_project` and `id`.
pub fn from_legacy(old: &Value) -> Result<Value> {
    validate_legacy(old)?;
    let mut o = old.as_object().cloned().unwrap_or_default();
    o.remove("version");
    let namespace = o["ifc_identity"]["namespace"].as_str().unwrap_or("").to_ascii_lowercase();
    let id = if is_hex_id(&namespace) { namespace } else { new_id() };
    o.insert("built_project".into(), Value::from(VERSION));
    o.insert("id".into(), Value::from(id));
    Ok(Value::Object(o))
}

/// The `.igzproj` value of a manifest (the round trip, and an export for an
/// old build).
pub fn to_legacy(manifest: &Value) -> Value {
    let mut o = manifest.as_object().cloned().unwrap_or_default();
    o.remove("built_project");
    o.remove("id");
    o.insert("version".into(), Value::from(1u64));
    Value::Object(o)
}

/// §5: write the sibling manifest, read it back, and only then remove the
/// old file.
pub fn convert(old_path: &Path) -> Result<Converted> {
    let target = sibling(old_path);
    if target.exists() {
        return Ok(Converted::AlreadyConverted(target));
    }
    let old = load_legacy(old_path)?;
    let manifest = from_legacy(&old)?;
    let text = to_text(&manifest);
    if let Err(e) = write_atomic(&target, &text) {
        return Err(e);
    }
    let check = load(&target);
    match check {
        Ok(back) if back == manifest && to_legacy(&back) == old => {}
        Ok(_) => {
            let _ = fs::remove_file(&target);
            return Err(format!("{}: the converted project did not read back the same", target.display()));
        }
        Err(e) => {
            let _ = fs::remove_file(&target);
            return Err(e);
        }
    }
    fs::remove_file(old_path).map_err(|e| {
        format!("{} was converted but could not be removed: {e}", old_path.display())
    })?;
    if let Some(dir) = old_path.parent() {
        sync_dir(dir);
    }
    Ok(Converted::Converted(target))
}

/// The manifest to use for `path`: itself, or an `.igzproj` converted (or
/// its existing sibling).
pub fn open(path: &Path) -> Result<(PathBuf, Option<Converted>)> {
    if is_legacy(path) {
        let done = convert(path)?;
        return Ok((done.path().to_owned(), Some(done)));
    }
    Ok((path.to_owned(), None))
}

// ---- pointers (§2) -------------------------------------------------------------

/// The project a model's `plugin_data.project` points at, or None. An
/// `.igzproj` pointer prefers its `.builtproject` sibling. The result may be
/// an `.igzproj` that still has to be converted.
pub fn resolve_pointer(host: Option<&Path>, rel: Option<&str>, abs: Option<&str>) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let (Some(host), Some(rel)) = (host, rel.filter(|r| !r.is_empty())) {
        let dir = host.parent().unwrap_or(Path::new(""));
        candidates.push(dir.join(rel));
    }
    if let Some(abs) = abs.filter(|a| !a.is_empty()) {
        candidates.push(PathBuf::from(abs));
    }
    for c in candidates {
        if is_legacy(&c) {
            let s = sibling(&c);
            if s.is_file() {
                return Some(normalize(&s));
            }
        }
        if c.is_file() {
            return Some(normalize(&c));
        }
    }
    None
}

/// Absolute, with `.` and `..` folded (the file need not exist).
pub fn normalize(path: &Path) -> PathBuf {
    if let Ok(p) = fs::canonicalize(path) {
        return p;
    }
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// `path` relative to `dir`, with `/` and `..` as needed (§1).
pub fn relative(path: &Path, dir: &Path) -> String {
    let path = normalize(path);
    let dir = normalize(dir);
    let p: Vec<_> = path.components().collect();
    let d: Vec<_> = dir.components().collect();
    let common = p.iter().zip(&d).take_while(|(a, b)| a == b).count();
    if common == 0 {
        return path.to_string_lossy().replace('\\', "/");
    }
    let mut parts: Vec<String> = std::iter::repeat("..".to_owned()).take(d.len() - common).collect();
    parts.extend(p[common..].iter().map(|c| c.as_os_str().to_string_lossy().into_owned()));
    if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    }
}

/// A manifest path resolved against the manifest's directory.
pub fn resolve(dir: &Path, rel: &str) -> PathBuf {
    let p = Path::new(rel);
    normalize(&if p.is_absolute() { p.to_owned() } else { dir.join(rel) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("builtproject-{name}-{}", new_id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn legacy() -> Value {
        json!({"version": 1, "bim_registry": {"project": {"name": "Ride"}},
               "ifc_identity": {"namespace": "ca2c12df23ac48f9a103455660d950e5", "project": "2gB0"},
               "files": [{"path": "a.igz", "role": "host"}], "links": [],
               "worksession": {"files": [{"path": "a.igz", "camera": {"distance": 7.25}}], "active_file": "a.igz"},
               "extra": {"kept": [1, 2.50, "x"]}})
    }

    #[test]
    fn conversion_round_trips_and_removes_old_file() {
        let dir = tmp("convert");
        let old = dir.join("Job.igzproj");
        let text = serde_json::to_string(&legacy()).unwrap().replace("\"extra\":{", "\"precise\":2.50,\"extra\":{");
        assert!(text.contains("2.50"));
        fs::write(&old, &text).unwrap();
        let done = convert(&old).unwrap();
        assert_eq!(done, Converted::Converted(dir.join("Job.builtproject")));
        assert!(!old.exists());
        let back = load(done.path()).unwrap();
        assert_eq!(back["id"], "ca2c12df23ac48f9a103455660d950e5");
        assert_eq!(to_legacy(&back), serde_json::from_str::<Value>(&text).unwrap());
        // Numbers keep their text.
        assert!(fs::read_to_string(done.path()).unwrap().contains("2.50"));
    }

    #[test]
    fn existing_manifest_wins_and_old_file_stays() {
        let dir = tmp("both");
        let old = dir.join("Job.igzproj");
        fs::write(&old, serde_json::to_string(&legacy()).unwrap()).unwrap();
        let manifest = from_legacy(&legacy()).unwrap();
        save(&sibling(&old), &manifest).unwrap();
        assert!(matches!(convert(&old).unwrap(), Converted::AlreadyConverted(_)));
        assert!(old.exists());
    }

    #[test]
    fn invalid_old_file_is_left_alone() {
        let dir = tmp("bad");
        let old = dir.join("Job.igzproj");
        fs::write(&old, "{\"version\": 2}").unwrap();
        assert!(convert(&old).is_err());
        assert!(old.exists() && !sibling(&old).exists());
    }

    #[test]
    fn relocate_renames_the_entry_and_its_links() {
        let mut v = json!({"files": [{"path": "a.built", "role": "host"}, {"path": "s.built", "role": "linked"}],
                           "links": [{"host": "a.built", "source": "s.built", "placement_uid": "1"}]});
        assert!(relocate(&mut v, "s.built", "parts/s.built"));
        assert_eq!(v["files"][1]["path"], "parts/s.built");
        assert_eq!(v["links"][0]["source"], "parts/s.built");
        assert!(!relocate(&mut v, "gone.built", "x.built"));
    }

    #[test]
    fn merge_takes_each_side_and_reports_both() {
        let base = json!({"types": {"a": 1, "b": 1}, "project": {"name": "x"}});
        let ours = json!({"types": {"a": 2, "b": 1}, "project": {"name": "x"}});
        let theirs = json!({"types": {"a": 1, "b": 3}, "project": {"name": "y"}, "storeys": {}});
        let (m, c) = merge_registry(&base, &ours, &theirs);
        assert_eq!(m, json!({"types": {"a": 2, "b": 3}, "project": {"name": "y"}, "storeys": {}}));
        assert!(c.is_empty());
        let theirs = json!({"types": {"a": 5, "b": 1}});
        let (m, c) = merge_registry(&base, &ours, &theirs);
        assert_eq!(m["types"]["a"], 2);
        assert_eq!(c, vec!["types.a"]);
    }

    #[test]
    fn pointer_prefers_converted_sibling() {
        let dir = tmp("pointer");
        let host = dir.join("models/a.built");
        fs::create_dir_all(host.parent().unwrap()).unwrap();
        let manifest = dir.join("Job.builtproject");
        fs::write(&manifest, "{}").unwrap();
        let found = resolve_pointer(Some(&host), Some("../Job.igzproj"), Some("/nowhere/Job.igzproj"));
        assert_eq!(found, Some(normalize(&manifest)));
        assert_eq!(resolve_pointer(Some(&host), Some("../Other.igzproj"), None), None);
        assert_eq!(relative(&manifest, host.parent().unwrap()), "../Job.builtproject");
    }
}
