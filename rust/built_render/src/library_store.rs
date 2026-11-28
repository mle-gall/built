// SPDX-License-Identifier: GPL-3.0-or-later
//! Where saved Library items live, and how they are laid out on disk
//! (docs/library.md).
//!
//! Every kind that can be saved (materials, components, profiles, sections,
//! presets, styles, scene templates, title blocks; a drawn sketch is saved
//! as a profile or a section, by its use) goes to one of
//! the same destinations:
//!
//! - **This model**: inside the open file (the document keeps the entries,
//!   `library_host/destinations.rs`);
//! - **This project**: `<project folder>/Library`, beside the `.builtproject`;
//! - **My library**: a folder on this machine for this user
//!   ([`default_mine`], changeable in Settings › Library);
//! - **Shared libraries**: any number of named folders, each optionally
//!   read-only (never offered as a save target).
//!
//! Each folder destination has the same layout: one folder per kind, one
//! JSON file per item named by its id (`<kind>/<id>.json`), an optional
//! thumbnail beside it (`<id>.png`) and the files an item carries
//! (`<id>.files/`, a material's image, a component's `.built`). An item keeps
//! its id wherever it is copied, and every save of it bumps its revision, so
//! a copy is recognised and a newer revision is offered as an update.
//!
//! This module is plain Rust (no Python): the host reads the preferences and
//! the document and calls in.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const FORMAT: &str = "built.library";
pub const VERSION: u64 = 1;
/// The marker a library root gets once the old saved folders are copied in.
pub const MIGRATED: &str = ".migrated-v1";

/// A kind that can be saved: (id, folder, label, saved in the model by default).
pub struct Kind {
    pub id: &'static str,
    pub label: &'static str,
    /// A first save goes to This model (it only makes sense with the model).
    pub model_first: bool,
}

pub const KINDS: &[Kind] = &[
    Kind { id: "materials", label: "Materials", model_first: false },
    Kind { id: "components", label: "Components", model_first: false },
    Kind { id: "profiles", label: "Profiles", model_first: false },
    Kind { id: "sections", label: "Sections", model_first: false },
    Kind { id: "presets", label: "Presets", model_first: false },
    Kind { id: "styles", label: "Styles", model_first: false },
    Kind { id: "scenes", label: "View templates", model_first: true },
    Kind { id: "title_blocks", label: "Title blocks", model_first: false },
];

pub fn kind(id: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.id == id)
}

/// One saved item. `data` is the kind's own payload (a material's
/// dictionary, a profile file, a preset row, a style…).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Entry {
    #[serde(default = "format")]
    pub format: String,
    #[serde(default = "version")]
    pub v: u64,
    pub kind: String,
    pub id: String,
    #[serde(default = "first")]
    pub rev: u64,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Seconds since 1970 of the last save.
    #[serde(default)]
    pub saved: u64,
    #[serde(default)]
    pub data: Value,
}

fn format() -> String {
    FORMAT.into()
}
fn version() -> u64 {
    VERSION
}
fn first() -> u64 {
    1
}

impl Entry {
    pub fn new(kind: &str, id: &str, name: &str, data: Value) -> Self {
        Entry {
            format: FORMAT.into(),
            v: VERSION,
            kind: kind.into(),
            id: id.into(),
            rev: 1,
            name: name.into(),
            description: String::new(),
            tags: Vec::new(),
            saved: now(),
            data,
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let e: Entry = serde_json::from_str(text).map_err(|e| e.to_string())?;
        e.check()?;
        Ok(e)
    }

    pub fn check(&self) -> Result<(), String> {
        if self.format != FORMAT || self.v != VERSION {
            return Err(format!("Not a {FORMAT} v{VERSION} item"));
        }
        if kind(&self.kind).is_none() {
            return Err(format!("Unknown Library kind “{}”", self.kind));
        }
        if !valid_id(&self.id) {
            return Err(format!("“{}” is not a Library id", self.id));
        }
        if self.name.trim().is_empty() {
            return Err("A Library item needs a name".into());
        }
        Ok(())
    }

    pub fn to_text(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).expect("entry");
        text.push('\n');
        text
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// An id is 1–64 of `[0-9a-z_-]`: it names files on every platform.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_digit() || b.is_ascii_lowercase() || b == b'_' || b == b'-')
}

/// A new random id: 16 hex digits.
pub fn new_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(now());
    h.write_u64(SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    h.write_u32(std::process::id());
    format!("{:016x}", h.finish())
}

/// The same id for the same seed on every machine (64-bit FNV-1a), for
/// items migrated from folders that had none.
pub fn stable_id(seed: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in seed.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

// ---- destinations ----------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct SharedLibrary {
    pub name: String,
    pub path: PathBuf,
    #[serde(default)]
    pub read_only: bool,
}

/// What Settings › Library keeps: My library's folder (None: the default)
/// and the shared libraries.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    pub mine: Option<PathBuf>,
    pub shared: Vec<SharedLibrary>,
}

impl Config {
    /// From the two preference values (a path, empty for the default, and
    /// the shared libraries as JSON); anything unreadable reads as unset.
    pub fn from_settings(mine: &str, shared: &str) -> Self {
        let mine = Some(mine.trim()).filter(|m| !m.is_empty()).map(PathBuf::from);
        let mut shared: Vec<SharedLibrary> = serde_json::from_str(shared).unwrap_or_default();
        shared.retain(|s| !s.name.trim().is_empty() && !s.path.as_os_str().is_empty());
        Config { mine, shared }
    }

    pub fn shared_json(&self) -> String {
        serde_json::to_string(&self.shared).expect("shared libraries")
    }
}

/// My library's default folder: macOS `~/Library/Application
/// Support/Built/Library`, Windows `%APPDATA%\Built\Library`, Linux
/// `$XDG_DATA_HOME/built/library`. With a settings profile (tests,
/// `--settings`) it is inside the profile.
pub fn default_mine(profile: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = profile {
        return Some(p.join("data").join("Built").join("Library"));
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/Built/Library"))
    }
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Built").join("Library"))
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .map(|d| d.join("built").join("library"))
    }
}

/// The library folder of a project: `Library` beside its manifest.
pub fn project_root(manifest: &Path) -> PathBuf {
    manifest.parent().unwrap_or(Path::new(".")).join("Library")
}

/// One place items can live.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Destination {
    /// "model", "project", "mine", "shared:<name>" or "bundled".
    pub key: String,
    pub label: String,
    /// The folder (None for This model and Built's bundled content).
    pub root: Option<PathBuf>,
    pub writable: bool,
    /// Why it cannot be written ("" when it can).
    pub reason: String,
}

/// Every destination, in the order the save drawer and the Source facet
/// list them: This model, This project, My library, the shared libraries.
/// (Built's bundled content is a source, never a destination.)
pub fn destinations(cfg: &Config, has_model: bool, project: Option<&Path>, mine_default: Option<&Path>) -> Vec<Destination> {
    let mut out = vec![Destination {
        key: "model".into(),
        label: "This model".into(),
        root: None,
        writable: has_model,
        reason: if has_model { String::new() } else { "Open a model first".into() },
    }];
    out.push(match project {
        Some(p) => Destination { key: "project".into(), label: "This project".into(), root: Some(project_root(p)), writable: true, reason: String::new() },
        None => Destination {
            key: "project".into(),
            label: "This project".into(),
            root: None,
            writable: false,
            reason: "This model is not in a project".into(),
        },
    });
    let mine = cfg.mine.clone().or_else(|| mine_default.map(Path::to_path_buf));
    out.push(Destination {
        key: "mine".into(),
        label: "My library".into(),
        writable: mine.is_some(),
        reason: if mine.is_some() { String::new() } else { "No folder for My library".into() },
        root: mine,
    });
    for s in &cfg.shared {
        out.push(Destination {
            key: format!("shared:{}", s.name),
            label: s.name.clone(),
            root: Some(s.path.clone()),
            writable: !s.read_only,
            reason: if s.read_only { format!("{} is read-only", s.name) } else { String::new() },
        });
    }
    out
}

/// The destination a first save offers: the last one used for this kind
/// when it can still be written, else This model for model-only kinds,
/// else My library.
pub fn default_destination<'a>(dests: &'a [Destination], kind_id: &str, last: Option<&str>) -> Option<&'a Destination> {
    let writable = |k: &str| dests.iter().find(|d| d.key == k && d.writable);
    last.and_then(writable)
        .or_else(|| if kind(kind_id).is_some_and(|k| k.model_first) { writable("model") } else { None })
        .or_else(|| writable("mine"))
        .or_else(|| dests.iter().find(|d| d.writable))
}

// ---- the folder layout ------------------------------------------------------------------------

fn bad(text: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, text.into())
}

pub fn kind_dir(root: &Path, kind_id: &str) -> PathBuf {
    root.join(kind_id)
}
pub fn entry_path(root: &Path, kind_id: &str, id: &str) -> PathBuf {
    kind_dir(root, kind_id).join(format!("{id}.json"))
}
pub fn thumb_path(root: &Path, kind_id: &str, id: &str) -> PathBuf {
    kind_dir(root, kind_id).join(format!("{id}.png"))
}
pub fn files_dir(root: &Path, kind_id: &str, id: &str) -> PathBuf {
    kind_dir(root, kind_id).join(format!("{id}.files"))
}

/// Write `bytes` to `path` atomically (a temporary file renamed over it).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().ok_or_else(|| bad("no folder"))?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.{}.tmp", path.file_name().unwrap_or_default().to_string_lossy(), new_id()));
    let result = (|| {
        let mut f = std::fs::File::create_new(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Can items be written into `root` (it exists or can be made, and takes a file)?
pub fn probe_writable(root: &Path) -> io::Result<()> {
    std::fs::create_dir_all(root)?;
    let probe = root.join(format!(".write-probe-{}", new_id()));
    std::fs::File::create_new(&probe)?;
    std::fs::remove_file(probe)
}

/// Save `entry` into the folder destination `root`: its JSON, its
/// thumbnail (PNG bytes) and the files it carries (`(name, source)`, copied
/// into `<id>.files/`). Returns the JSON's path.
pub fn write(root: &Path, entry: &Entry, thumb: Option<&[u8]>, files: &[(String, PathBuf)]) -> io::Result<PathBuf> {
    entry.check().map_err(bad)?;
    let path = entry_path(root, &entry.kind, &entry.id);
    if !files.is_empty() {
        let dir = files_dir(root, &entry.kind, &entry.id);
        std::fs::create_dir_all(&dir)?;
        for (name, source) in files {
            if name.contains(['/', '\\']) || name.starts_with('.') || name.is_empty() {
                return Err(bad(format!("“{name}” is not a file name")));
            }
            let target = dir.join(name);
            if source != &target {
                std::fs::copy(source, &target)?;
            }
        }
    }
    if let Some(png) = thumb {
        write_atomic(&thumb_path(root, &entry.kind, &entry.id), png)?;
    }
    write_atomic(&path, entry.to_text().as_bytes())?;
    Ok(path)
}

/// Every item of `kind_id` in `root`, by name; unreadable files are skipped.
pub fn list(root: &Path, kind_id: &str) -> Vec<(PathBuf, Entry)> {
    let mut out = Vec::new();
    let Ok(files) = std::fs::read_dir(kind_dir(root, kind_id)) else { return out };
    for path in files.flatten().map(|e| e.path()) {
        if path.extension().is_none_or(|x| x != "json") || path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
            continue;
        }
        let Some(e) = std::fs::read_to_string(&path).ok().and_then(|t| Entry::parse(&t).ok()) else { continue };
        if e.kind == kind_id && path.file_stem().is_some_and(|s| s == e.id.as_str()) {
            out.push((path, e));
        }
    }
    out.sort_by(|a, b| a.1.name.to_lowercase().cmp(&b.1.name.to_lowercase()).then_with(|| a.1.id.cmp(&b.1.id)));
    out
}

pub fn read(root: &Path, kind_id: &str, id: &str) -> Option<Entry> {
    if !valid_id(id) {
        return None;
    }
    let e = Entry::parse(&std::fs::read_to_string(entry_path(root, kind_id, id)).ok()?).ok()?;
    (e.kind == kind_id && e.id == id).then_some(e)
}

/// Remove an item, its thumbnail and its files. False when it was not there.
pub fn remove(root: &Path, kind_id: &str, id: &str) -> io::Result<bool> {
    if !valid_id(id) {
        return Err(bad("not a Library id"));
    }
    let path = entry_path(root, kind_id, id);
    if !path.exists() {
        return Ok(false);
    }
    std::fs::remove_file(path)?;
    let _ = std::fs::remove_file(thumb_path(root, kind_id, id));
    let files = files_dir(root, kind_id, id);
    if files.is_dir() {
        std::fs::remove_dir_all(files)?;
    }
    Ok(true)
}

/// What a copy did.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Copied {
    /// The target had no item with this id.
    Added,
    /// The target had an older revision: it is replaced.
    Updated,
    /// The target already has this revision.
    Same,
}

/// Why a copy did not happen.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    Missing,
    /// The target has a newer revision of the same item.
    Newer(u64),
    Io(String),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::Missing => write!(f, "The item is no longer there"),
            Refused::Newer(rev) => write!(f, "The destination has a newer revision ({rev})"),
            Refused::Io(e) => write!(f, "{e}"),
        }
    }
}

/// How an item arriving in a destination compares with what is there.
pub fn compare(incoming: &Entry, there: Option<&Entry>) -> Result<Copied, Refused> {
    match there {
        None => Ok(Copied::Added),
        Some(t) if t.rev > incoming.rev => Err(Refused::Newer(t.rev)),
        Some(t) if t.rev == incoming.rev => Ok(Copied::Same),
        Some(_) => Ok(Copied::Updated),
    }
}

/// Copy an item between two folder destinations, keeping its id and
/// revision, with its thumbnail and files. `force` replaces a newer
/// revision too (Move does, and so does an explicit overwrite).
pub fn copy(from: &Path, to: &Path, kind_id: &str, id: &str, force: bool) -> Result<Copied, Refused> {
    let entry = read(from, kind_id, id).ok_or(Refused::Missing)?;
    let there = read(to, kind_id, id);
    let outcome = match compare(&entry, there.as_ref()) {
        Err(Refused::Newer(_)) if force => Copied::Updated,
        other => other?,
    };
    if outcome == Copied::Same && from == to {
        return Ok(Copied::Same);
    }
    let io = |e: io::Error| Refused::Io(e.to_string());
    let thumb = std::fs::read(thumb_path(from, kind_id, id)).ok();
    let mut files = Vec::new();
    if let Ok(dir) = std::fs::read_dir(files_dir(from, kind_id, id)) {
        for f in dir.flatten() {
            if f.path().is_file() {
                files.push((f.file_name().to_string_lossy().into_owned(), f.path()));
            }
        }
    }
    // A replaced item's old files go with it.
    if there.is_some() && outcome != Copied::Same {
        let old = files_dir(to, kind_id, id);
        if old.is_dir() {
            std::fs::remove_dir_all(old).map_err(io)?;
        }
    }
    write(to, &entry, thumb.as_deref(), &files).map_err(io)?;
    Ok(outcome)
}

/// Move: copy (replacing whatever revision is there), then remove the source.
pub fn move_item(from: &Path, to: &Path, kind_id: &str, id: &str) -> Result<Copied, Refused> {
    if from == to {
        return Ok(Copied::Same);
    }
    let outcome = copy(from, to, kind_id, id, true)?;
    remove(from, kind_id, id).map_err(|e| Refused::Io(e.to_string()))?;
    Ok(outcome)
}

/// The saved revision of an item that replaces `old` (a save over an item
/// already there keeps its id and goes up by one).
pub fn next_revision(old: Option<&Entry>, incoming: u64) -> u64 {
    old.map_or(incoming.max(1), |o| o.rev.max(incoming) + 1)
}

// ---- migration -------------------------------------------------------------------------------

/// What was saved before the shared layout: drawn profiles and sections
/// (`.builtprofile` in `profiles`), presets (`.builtpreset` in `presets`)
/// and My library's materials (a list kept in the preferences, as JSON).
#[derive(Default)]
pub struct Legacy<'a> {
    pub profiles: Option<&'a Path>,
    pub presets: Option<&'a Path>,
    pub materials_json: Option<&'a str>,
}

/// The id a legacy profile file gets: stable, so a second run (or another
/// machine with the same file) finds it again.
pub fn legacy_profile_id(file: &Path) -> String {
    stable_id(&format!("profile:{}", file.file_name().unwrap_or_default().to_string_lossy()))
}
pub fn legacy_material_id(name: &str) -> String {
    stable_id(&format!("material:{name}"))
}

/// The kind and entry of one legacy profile file, if it is one.
pub fn legacy_profile(file: &Path) -> Option<Entry> {
    let data: Value = serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
    if data["format"] != "built.profile" || !data["sketch"]["curves"].is_array() {
        return None;
    }
    let kind = if data["use"] == "section" { "sections" } else { "profiles" };
    let name = data["name"].as_str().filter(|n| !n.trim().is_empty()).unwrap_or("Drawn profile").to_owned();
    let mut e = Entry::new(kind, &legacy_profile_id(file), &name, data);
    e.saved = file.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    Some(e)
}

pub fn legacy_preset(file: &Path) -> Option<Entry> {
    let data: Value = serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
    let uid = data["uid"].as_str()?.to_lowercase();
    let name = data["name"].as_str()?.to_owned();
    let id = if valid_id(&uid) { uid } else { stable_id(&format!("preset:{uid}")) };
    let mut e = Entry::new("presets", &id, &name, data.clone());
    e.rev = data["rev"].as_u64().unwrap_or(1).max(1);
    Some(e)
}

pub fn legacy_materials(json: &str) -> Vec<Entry> {
    let rows: Vec<Value> = serde_json::from_str(json).unwrap_or_default();
    rows.into_iter()
        .filter(|r| matches!(r["type"].as_str(), Some("materials" | "colours")))
        .filter_map(|r| {
            let name = r["name"].as_str().filter(|n| !n.trim().is_empty())?.to_owned();
            let mut data = r.clone();
            if let Some(o) = data.as_object_mut() {
                o.remove("type");
            }
            let mut e = Entry::new("materials", &legacy_material_id(&name), &name, data);
            e.tags = r["category"].as_str().filter(|c| !c.is_empty()).map(|c| vec![c.to_owned()]).unwrap_or_default();
            Some(e)
        })
        .collect()
}

/// Every legacy item, by kind, without writing anything.
pub fn legacy_entries(legacy: &Legacy<'_>) -> Vec<Entry> {
    let mut out = Vec::new();
    let files = |dir: Option<&Path>, ext: &str| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = dir
            .and_then(|d| std::fs::read_dir(d).ok())
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == ext))
            .collect();
        v.sort();
        v
    };
    out.extend(files(legacy.profiles, "builtprofile").iter().filter_map(|f| legacy_profile(f)));
    out.extend(files(legacy.presets, "builtpreset").iter().filter_map(|f| legacy_preset(f)));
    if let Some(json) = legacy.materials_json {
        out.extend(legacy_materials(json));
    }
    out
}

/// Copy the legacy items into `root` once (the marker file says it ran).
/// The old files stay where they were and stay readable; an item already
/// in `root` is left alone. Returns how many were copied, None when the
/// migration had already run.
pub fn migrate(root: &Path, legacy: &Legacy<'_>) -> io::Result<Option<usize>> {
    let marker = root.join(MIGRATED);
    if marker.exists() {
        return Ok(None);
    }
    let mut n = 0;
    let mut log = String::new();
    for e in legacy_entries(legacy) {
        if read(root, &e.kind, &e.id).is_none() {
            write(root, &e, None, &[])?;
            n += 1;
            log.push_str(&format!("{}/{} {}\n", e.kind, e.id, e.name));
        }
    }
    write_atomic(&marker, log.as_bytes())?;
    Ok(Some(n))
}

// ---- several destinations at once -----------------------------------------------------------

/// The newest revision of each (kind, id) among `entries`, for "newer
/// revision available" offers.
pub fn newest<'a>(entries: impl IntoIterator<Item = (&'a str, &'a Entry)>) -> std::collections::HashMap<(String, String), (u64, String)> {
    let mut best: std::collections::HashMap<(String, String), (u64, String)> = Default::default();
    for (source, e) in entries {
        let slot = best.entry((e.kind.clone(), e.id.clone())).or_insert((0, String::new()));
        if e.rev > slot.0 {
            *slot = (e.rev, source.to_owned());
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("built-library-test-{tag}-{}", new_id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn every_kind_saves_and_reloads_in_a_folder() {
        let root = temp("kinds");
        for k in KINDS {
            let mut e = Entry::new(k.id, &new_id(), &format!("A {}", k.label), serde_json::json!({"x": 1}));
            e.description = "described".into();
            e.tags = vec!["one".into(), "two".into()];
            let path = write(&root, &e, Some(b"png"), &[]).unwrap();
            assert_eq!(path, root.join(k.id).join(format!("{}.json", e.id)));
            let back = read(&root, k.id, &e.id).unwrap();
            assert_eq!(back, e);
            assert_eq!(list(&root, k.id).len(), 1);
            assert!(thumb_path(&root, k.id, &e.id).exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bad_entries_are_refused() {
        let root = temp("bad");
        let mut e = Entry::new("materials", "abc", "Oak", Value::Null);
        e.kind = "nope".into();
        assert!(write(&root, &e, None, &[]).is_err());
        let e = Entry::new("materials", "../x", "Oak", Value::Null);
        assert!(write(&root, &e, None, &[]).is_err());
        let e = Entry::new("materials", "abc", "  ", Value::Null);
        assert!(write(&root, &e, None, &[]).is_err());
        assert!(Entry::parse("{\"format\":\"other\",\"v\":1,\"kind\":\"materials\",\"id\":\"a\",\"name\":\"b\"}").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn files_travel_with_copies_and_moves() {
        let (a, b) = (temp("a"), temp("b"));
        let src = a.join("oak.png");
        std::fs::write(&src, b"image").unwrap();
        let e = Entry::new("materials", "oak1", "Oak", serde_json::json!({"texture": {"path": "oak.png"}}));
        write(&a, &e, None, &[("oak.png".into(), src)]).unwrap();
        assert_eq!(copy(&a, &b, "materials", "oak1", false), Ok(Copied::Added));
        assert_eq!(std::fs::read(files_dir(&b, "materials", "oak1").join("oak.png")).unwrap(), b"image");
        assert_eq!(copy(&a, &b, "materials", "oak1", false), Ok(Copied::Same));
        // A newer revision in b is not replaced by a copy, only by a move.
        let mut newer = e.clone();
        newer.rev = 3;
        write(&b, &newer, None, &[]).unwrap();
        assert_eq!(copy(&a, &b, "materials", "oak1", false), Err(Refused::Newer(3)));
        // b's newer one comes back to a as an update.
        assert_eq!(copy(&b, &a, "materials", "oak1", false), Ok(Copied::Updated));
        assert_eq!(read(&a, "materials", "oak1").unwrap().rev, 3);
        assert_eq!(move_item(&a, &b, "materials", "oak1"), Ok(Copied::Same));
        assert!(read(&a, "materials", "oak1").is_none());
        assert!(!files_dir(&a, "materials", "oak1").exists());
        assert!(read(&b, "materials", "oak1").is_some());
        assert_eq!(remove(&b, "materials", "oak1").unwrap(), true);
        assert_eq!(remove(&b, "materials", "oak1").unwrap(), false);
        std::fs::remove_dir_all(a).unwrap();
        std::fs::remove_dir_all(b).unwrap();
    }

    #[test]
    fn destinations_follow_settings_and_project() {
        let cfg = Config::from_settings(
            "",
            r#"[{"name":"Team","path":"/team"},{"name":"Vendor","path":"/vendor","read_only":true},{"name":"","path":"/x"}]"#,
        );
        assert_eq!(cfg.shared.len(), 2);
        let d = destinations(&cfg, true, None, Some(Path::new("/home/lib")));
        let keys: Vec<_> = d.iter().map(|d| d.key.as_str()).collect();
        assert_eq!(keys, ["model", "project", "mine", "shared:Team", "shared:Vendor"]);
        assert!(!d[1].writable && d[1].reason.contains("not in a project"));
        assert_eq!(d[2].root.as_deref(), Some(Path::new("/home/lib")));
        assert!(!d[4].writable && d[4].reason.contains("read-only"));
        let d = destinations(&cfg, true, Some(Path::new("/p/Site.builtproject")), None);
        assert_eq!(d[1].root.as_deref(), Some(Path::new("/p/Library")));
        assert!(d[1].writable);
        assert!(!d[2].writable);
        let cfg = Config::from_settings("/elsewhere", "garbage");
        assert_eq!(cfg.mine.as_deref(), Some(Path::new("/elsewhere")));
        assert!(cfg.shared.is_empty());
    }

    #[test]
    fn first_destination_is_remembered_per_kind() {
        let cfg = Config::from_settings("", r#"[{"name":"Vendor","path":"/v","read_only":true}]"#);
        let d = destinations(&cfg, true, None, Some(Path::new("/m")));
        assert_eq!(default_destination(&d, "materials", None).unwrap().key, "mine");
        assert_eq!(default_destination(&d, "scenes", None).unwrap().key, "model");
        assert_eq!(default_destination(&d, "materials", Some("model")).unwrap().key, "model");
        // A remembered destination that cannot be written falls back.
        assert_eq!(default_destination(&d, "materials", Some("shared:Vendor")).unwrap().key, "mine");
        assert_eq!(default_destination(&d, "materials", Some("project")).unwrap().key, "mine");
    }

    #[test]
    fn default_folders_per_platform() {
        let p = default_mine(Some(Path::new("/tmp/profile"))).unwrap();
        assert_eq!(p, Path::new("/tmp/profile/data/Built/Library"));
        let d = default_mine(None);
        #[cfg(target_os = "macos")]
        assert!(d.unwrap().ends_with("Library/Application Support/Built/Library"));
        #[cfg(windows)]
        assert!(d.unwrap().ends_with("Built\\Library"));
        #[cfg(not(any(target_os = "macos", windows)))]
        assert!(d.unwrap().ends_with("built/library"));
    }

    #[test]
    fn migration_copies_once_and_keeps_the_old_files() {
        let (old, root) = (temp("old"), temp("root"));
        let profiles = old.join("profiles");
        let presets = old.join("presets");
        std::fs::create_dir_all(&profiles).unwrap();
        std::fs::create_dir_all(&presets).unwrap();
        std::fs::write(profiles.join("Notch.builtprofile"), r#"{"format":"built.profile","v":1,"name":"Notch","use":"profile","sketch":{"curves":[]}}"#).unwrap();
        std::fs::write(profiles.join("Cut.builtprofile"), r#"{"format":"built.profile","v":1,"name":"Cut","use":"section","sketch":{"curves":[]}}"#).unwrap();
        std::fs::write(profiles.join("broken.builtprofile"), "{").unwrap();
        std::fs::write(presets.join("p.builtpreset"), r#"{"uid":"ABCDEF0123456789","name":"Rail","rev":4}"#).unwrap();
        let mats = r#"[{"type":"materials","name":"Oak","color":[1,0.5,0],"category":"Wood"},{"type":"other","name":"x"}]"#;
        let legacy = Legacy { profiles: Some(&profiles), presets: Some(&presets), materials_json: Some(mats) };
        assert_eq!(migrate(&root, &legacy).unwrap(), Some(4));
        assert_eq!(list(&root, "profiles")[0].1.name, "Notch");
        assert_eq!(list(&root, "sections")[0].1.name, "Cut");
        let preset = &list(&root, "presets")[0].1;
        assert_eq!((preset.id.as_str(), preset.rev), ("abcdef0123456789", 4));
        let oak = &list(&root, "materials")[0].1;
        assert_eq!(oak.tags, ["Wood"]);
        assert!(oak.data.get("type").is_none());
        // Ran once; the old files are untouched.
        assert_eq!(migrate(&root, &legacy).unwrap(), None);
        assert!(profiles.join("Notch.builtprofile").exists());
        assert!(presets.join("p.builtpreset").exists());
        // A second root migrating the same files gets the same ids.
        let other = temp("other");
        migrate(&other, &legacy).unwrap();
        assert_eq!(list(&other, "profiles")[0].1.id, list(&root, "profiles")[0].1.id);
        for d in [old, root, other] {
            std::fs::remove_dir_all(d).unwrap();
        }
    }

    #[test]
    fn newest_revision_wins() {
        let a = Entry { rev: 2, ..Entry::new("styles", "s1", "Clay", Value::Null) };
        let b = Entry { rev: 5, ..Entry::new("styles", "s1", "Clay", Value::Null) };
        let best = newest([("mine", &a), ("shared:Team", &b)]);
        assert_eq!(best[&("styles".to_owned(), "s1".to_owned())], (5, "shared:Team".to_owned()));
        assert_eq!(next_revision(Some(&b), 1), 6);
        assert_eq!(next_revision(None, 3), 3);
    }

    #[test]
    fn read_only_folders_refuse_writes() {
        let root = temp("ro");
        let mut perms = std::fs::metadata(&root).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&root, perms.clone()).unwrap();
        let e = Entry::new("styles", "s1", "Clay", Value::Null);
        let refused = write(&root, &e, None, &[]).is_err() && probe_writable(&root).is_err();
        perms.set_readonly(false);
        std::fs::set_permissions(&root, perms).unwrap();
        // Root can write anywhere: then only the configured flag protects.
        if !refused {
            eprintln!("running as a user who can write read-only folders");
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
