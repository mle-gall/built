// SPDX-License-Identifier: GPL-3.0-or-later
//! The per-user session file and the open project of a window
//! (docs/builtproject-format.md §3, §6).
//!
//! `ProjectSession` plans the open (tabs in order, roles, cameras, scenes,
//! which exist), takes each member's completion or failure, answers which
//! file is editable, gives the Project tree its file rows and Problems their
//! entries, and writes the session file when what it holds changes.
use super::manifest::{self, Converted, Result};
use crate::project_tree::{row, Row};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const SESSION_VERSION: u64 = 1;
/// Suffixes a tab can edit (formats/built.py DOCUMENT_SUFFIXES).
pub const DOCUMENT_SUFFIXES: [&str; 2] = ["built", "igz"];

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|s| !s.is_empty()).map(PathBuf::from)
}

/// §3: where this person's sessions live.
pub fn session_dir(override_dir: Option<&Path>) -> Result<PathBuf> {
    if let Some(d) = override_dir {
        return Ok(d.join("projects"));
    }
    if let Some(d) = env_dir("BUILT_SETTINGS_DIR") {
        return Ok(d.join("projects"));
    }
    #[cfg(target_os = "macos")]
    {
        let home = env_dir("HOME").ok_or("No home directory")?;
        Ok(home.join("Library/Application Support/Built/projects"))
    }
    #[cfg(windows)]
    {
        let base = env_dir("LOCALAPPDATA").or_else(|| env_dir("APPDATA")).ok_or("No AppData directory")?;
        Ok(base.join("Built").join("projects"))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let base = env_dir("XDG_STATE_HOME")
            .or_else(|| env_dir("HOME").map(|h| h.join(".local/state")))
            .ok_or("No home directory")?;
        Ok(base.join("built/projects"))
    }
}

pub fn session_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

/// The session file's value, or None when there is none (or it belongs to
/// another project, or does not parse: a session is never worth an error).
pub fn read_session(path: &Path, id: &str) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let ok = value.get("built_session").and_then(Value::as_u64) == Some(SESSION_VERSION)
        && value.get("project").and_then(Value::as_str) == Some(id)
        && value.get("tabs").is_some_and(Value::is_array);
    ok.then_some(value)
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// Planned and not open yet (or opening).
    Pending,
    Open,
    /// Listed but not on disk.
    Missing,
    Failed(String),
    /// A project file that is not a tab (another host, a reference).
    Listed,
}

impl State {
    fn word(&self) -> &str {
        match self {
            State::Pending => "opening",
            State::Open => "open",
            State::Missing => "missing",
            State::Failed(_) => "failed",
            State::Listed => "listed",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Member {
    pub rel: String,
    pub abs: PathBuf,
    pub role: String,
    pub state: State,
    pub tab: bool,
    pub camera: Value,
    pub scene: Value,
}

#[pyo3::pyclass]
pub struct ProjectSession {
    pub manifest_path: PathBuf,
    pub dir: PathBuf,
    pub id: String,
    pub name: String,
    pub manifest: Value,
    pub session_file: PathBuf,
    pub session: Value,
    pub members: Vec<Member>,
    pub editable: Option<String>,
    pub converted: Option<(PathBuf, Converted)>,
    written: String,
}

fn is_document(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| DOCUMENT_SUFFIXES.iter().any(|d| s.eq_ignore_ascii_case(d)))
}

impl ProjectSession {
    pub fn open(path: &Path, settings: Option<&Path>) -> Result<Self> {
        let given = manifest::normalize(path);
        let (manifest_path, converted) = manifest::open(&given)?;
        let manifest_path = manifest::normalize(&manifest_path);
        let value = manifest::load(&manifest_path)?;
        let dir = manifest_path.parent().unwrap_or(Path::new("")).to_owned();
        let id = value["id"].as_str().unwrap_or_default().to_owned();
        let session_file = session_path(&session_dir(settings)?, &id);
        let name = manifest_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut me = Self {
            manifest_path,
            dir,
            id,
            name,
            manifest: value,
            session_file,
            session: Value::Null,
            members: vec![],
            editable: None,
            converted: converted.map(|c| (given, c)),
            written: String::new(),
        };
        let session = read_session(&me.session_file, &me.id);
        me.written = session.as_ref().map(manifest::to_text).unwrap_or_default();
        me.session = session.unwrap_or_else(|| me.seed());
        me.build_members();
        Ok(me)
    }

    /// No session yet: a converted `.igzproj`'s `worksession`, else the
    /// host files in manifest order with the first one editable.
    fn seed(&self) -> Value {
        let mut tabs = Vec::new();
        let mut editable = Value::Null;
        if let Some(ws) = self.manifest.get("worksession").filter(|w| w.is_object()) {
            for f in ws["files"].as_array().into_iter().flatten() {
                tabs.push(json!({"path": f["path"], "camera": f.get("camera").cloned().unwrap_or(Value::Null), "scene": null}));
            }
            editable = ws.get("active_file").cloned().unwrap_or(Value::Null);
        } else {
            for f in self.files() {
                if f.1 == "host" {
                    tabs.push(json!({"path": f.0, "camera": null, "scene": null}));
                }
            }
        }
        if editable.is_null() {
            editable = tabs.first().map_or(Value::Null, |t| t["path"].clone());
        }
        json!({"built_session": SESSION_VERSION, "project": self.id,
               "manifest": self.manifest_path.to_string_lossy(),
               "tabs": tabs, "editable": editable, "window": {}})
    }

    fn files(&self) -> Vec<(String, String)> {
        self.manifest["files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| Some((f["path"].as_str()?.to_owned(), f["role"].as_str()?.to_owned())))
            .collect()
    }

    pub fn abs(&self, rel: &str) -> PathBuf {
        manifest::resolve(&self.dir, rel)
    }

    pub fn rel(&self, abs: &Path) -> String {
        manifest::relative(abs, &self.dir)
    }

    /// Members: the session's tabs in order, then the other project files.
    fn build_members(&mut self) {
        let roles: Vec<(String, String)> = self.files();
        let role_of = |abs: &Path| {
            roles
                .iter()
                .find(|(p, _)| manifest::resolve(&self.dir, p) == abs)
                .map_or("host".to_owned(), |(_, r)| r.clone())
        };
        let mut members = Vec::new();
        let mut seen = HashSet::new();
        for tab in self.session["tabs"].as_array().cloned().unwrap_or_default() {
            let Some(rel) = tab["path"].as_str() else { continue };
            let abs = self.abs(rel);
            if !seen.insert(abs.clone()) {
                continue;
            }
            let role = role_of(&abs);
            if role == "reference" || !is_document(rel) {
                continue; // a reference is listed, never a modelling tab
            }
            let listed = roles.iter().any(|(p, _)| manifest::resolve(&self.dir, p) == abs);
            if !listed && !abs.is_file() {
                continue; // neither in the project nor on disk (relocated)
            }
            let state = if abs.is_file() { State::Pending } else { State::Missing };
            members.push(Member {
                rel: self.rel(&abs),
                abs,
                role,
                state,
                tab: true,
                camera: tab.get("camera").cloned().unwrap_or(Value::Null),
                scene: tab.get("scene").cloned().unwrap_or(Value::Null),
            });
        }
        // Every tab was closed (or is gone): open the hosts, as a project
        // without a session does.
        let fallback = !members.iter().any(|m: &Member| m.state != State::Missing);
        for (rel, role) in roles.iter() {
            let abs = self.abs(rel);
            if !seen.insert(abs.clone()) {
                continue;
            }
            if fallback && role == "host" && is_document(rel) {
                let state = if abs.is_file() { State::Pending } else { State::Missing };
                members.push(Member {
                    rel: self.rel(&abs),
                    abs,
                    role: role.clone(),
                    state,
                    tab: true,
                    camera: Value::Null,
                    scene: Value::Null,
                });
                continue;
            }
            let state = if abs.exists() { State::Listed } else { State::Missing };
            members.push(Member {
                rel: self.rel(&abs),
                abs,
                role: role.clone(),
                state,
                tab: false,
                camera: Value::Null,
                scene: Value::Null,
            });
        }
        let editable = self.session["editable"].as_str().map(|e| self.rel(&self.abs(e)));
        self.editable = editable.filter(|e| members.iter().any(|m| m.tab && &m.rel == e && m.state != State::Missing));
        if self.editable.is_none() {
            self.editable = members
                .iter()
                .find(|m| m.tab && m.state != State::Missing)
                .map(|m| m.rel.clone());
        }
        self.members = members;
    }

    pub fn member(&self, abs: &Path) -> Option<&Member> {
        let abs = manifest::normalize(abs);
        self.members.iter().find(|m| m.abs == abs)
    }

    fn member_mut(&mut self, abs: &Path) -> Option<&mut Member> {
        let abs = manifest::normalize(abs);
        self.members.iter_mut().find(|m| m.abs == abs)
    }

    pub fn set_state(&mut self, abs: &Path, state: State) {
        let failed = matches!(state, State::Failed(_) | State::Missing);
        if let Some(m) = self.member_mut(abs) {
            m.state = state;
        }
        // The editable file did not come in: the next open tab is.
        if failed && self.editable.as_deref() == Some(&self.rel(abs)) {
            self.editable = self
                .members
                .iter()
                .find(|m| m.tab && matches!(m.state, State::Open | State::Pending))
                .map(|m| m.rel.clone());
        }
    }

    pub fn set_editable(&mut self, abs: Option<&Path>) -> Result<()> {
        let Some(abs) = abs else {
            self.editable = None;
            return Ok(());
        };
        let rel = self.rel(abs);
        let role = self.member(abs).map(|m| m.role.clone());
        if !is_document(&rel) || role.as_deref() == Some("reference") {
            return Err("The editable file must be a project document".into());
        }
        if !manifest::normalize(abs).is_file() {
            return Err(format!("{} is not there", abs.display()));
        }
        if self.member(abs).is_none() {
            self.members.push(Member {
                rel: rel.clone(),
                abs: manifest::normalize(abs),
                role: "host".into(),
                state: State::Open,
                tab: true,
                camera: Value::Null,
                scene: Value::Null,
            });
        }
        self.editable = Some(rel);
        Ok(())
    }

    pub fn is_read_only(&self, abs: &Path) -> bool {
        self.editable.as_deref() != Some(&self.rel(abs))
    }

    /// The window's tabs now (absolute path, camera, scene), the editable
    /// file and the window layout; writes the session when it changed.
    pub fn record(&mut self, tabs: Vec<(PathBuf, Value, Value)>, window: Option<Value>, keep_planned: bool) -> Result<bool> {
        let mut rows = Vec::new();
        let mut open = HashSet::new();
        for (abs, camera, scene) in tabs {
            let rel = self.rel(&abs);
            if !open.insert(rel.clone()) {
                continue;
            }
            rows.push(json!({"path": rel, "camera": camera, "scene": scene}));
            if let Some(m) = self.member_mut(&abs) {
                m.tab = true;
            }
        }
        // Planned tabs that never came in (missing, failed, still opening)
        // stay in the session so the next open finds them again.
        for m in self.members.iter().filter(|_| keep_planned) {
            if m.tab && !open.contains(&m.rel) && !matches!(m.state, State::Open | State::Listed) {
                rows.push(json!({"path": m.rel, "camera": m.camera, "scene": m.scene}));
            }
        }
        let mut value = self.session.as_object().cloned().unwrap_or_default();
        value.insert("built_session".into(), Value::from(SESSION_VERSION));
        value.insert("project".into(), Value::from(self.id.clone()));
        value.insert("manifest".into(), Value::from(self.manifest_path.to_string_lossy().into_owned()));
        value.insert("tabs".into(), Value::Array(rows));
        value.insert("editable".into(), self.editable.clone().map_or(Value::Null, Value::from));
        if let Some(w) = window {
            value.insert("window".into(), w);
        }
        self.session = Value::Object(value);
        let text = manifest::to_text(&self.session);
        if text == self.written {
            return Ok(false);
        }
        if let Some(dir) = self.session_file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        manifest::write_atomic(&self.session_file, &text)?;
        self.written = text;
        Ok(true)
    }

    /// Re-read the manifest (another person saved): roles and new files.
    pub fn reload(&mut self) -> Result<()> {
        self.manifest = manifest::load(&self.manifest_path)?;
        let states: Vec<(PathBuf, State, bool)> =
            self.members.iter().map(|m| (m.abs.clone(), m.state.clone(), m.tab)).collect();
        let editable = self.editable.clone();
        self.session["editable"] = editable.clone().map_or(Value::Null, Value::from);
        self.build_members();
        for (abs, state, tab) in states {
            if let Some(m) = self.member_mut(&abs) {
                if tab {
                    m.tab = true;
                    m.state = state;
                }
            }
        }
        self.editable = editable.or(self.editable.take());
        Ok(())
    }

    pub fn problems(&self) -> Vec<(String, String)> {
        self.members
            .iter()
            .filter_map(|m| match &m.state {
                State::Missing => Some((m.abs.to_string_lossy().into_owned(), format!("{} is missing", m.rel))),
                State::Failed(why) => Some((m.abs.to_string_lossy().into_owned(), format!("{} did not open: {why}", m.rel))),
                _ => None,
            })
            .collect()
    }

    /// The Project tree's project branch: one folder row, then one file row
    /// per member, with the file-row icon states the link rows use.
    pub fn tree_rows(&self, query: &str, expanded: &HashSet<String>) -> Vec<Row> {
        let query = query.trim().to_lowercase();
        let hits: Vec<&Member> = self
            .members
            .iter()
            .filter(|m| query.is_empty() || m.rel.to_lowercase().contains(&query))
            .collect();
        if hits.is_empty() {
            return vec![];
        }
        let key = "project-files:all".to_owned();
        let open = !query.is_empty() || !expanded.contains(&key);
        let problems = self.problems().len();
        let mut root = row(key, format!("Project {}", self.name), "", "folder", 0, 0, hits.len(), 0);
        root.1[2] = problems as i64;
        root.1[7] = 1;
        root.1[8] = i64::from(open);
        let mut rows = vec![root];
        if !open {
            return rows;
        }
        for m in hits {
            let dot = match m.state {
                State::Missing | State::Failed(_) => "missing",
                _ => "resolved",
            };
            let editable = self.editable.as_deref() == Some(&m.rel);
            let what = match &m.state {
                State::Missing => "Not found — this project file is missing".to_owned(),
                State::Failed(why) => format!("Did not open: {why}"),
                State::Pending => "Opening…".to_owned(),
                State::Open if editable => "Open — the file you edit".to_owned(),
                State::Open => "Open — read-only here".to_owned(),
                State::Listed => "In the project, not open".to_owned(),
            };
            let label = format!("{}\x1e{}\n\n{}", m.role, m.rel, what);
            let name = Path::new(&m.rel)
                .file_name()
                .map_or(m.rel.clone(), |s| s.to_string_lossy().into_owned());
            let mut r = row(
                format!("project-file:{}", m.abs.to_string_lossy()),
                name,
                &label,
                &format!("file:{dot}"),
                1,
                0,
                0,
                0,
            );
            r.0[3] = if editable { "editing".into() } else { m.role.clone() };
            r.1[2] = i64::from(matches!(m.state, State::Missing | State::Failed(_)));
            r.1[3] = i64::from(editable);
            rows.push(r);
        }
        rows
    }

    pub fn query(&self) -> Value {
        let editable = self.editable.as_ref().map(|e| self.abs(e).to_string_lossy().into_owned());
        let files: Vec<Value> = self
            .members
            .iter()
            .map(|m| {
                let mut row = Map::new();
                row.insert("path".into(), Value::from(m.abs.to_string_lossy().into_owned()));
                row.insert("role".into(), Value::from(m.role.clone()));
                row.insert("state".into(), Value::from(m.state.word()));
                row.insert("tab".into(), Value::from(m.tab));
                row.insert("read_only".into(), Value::from(self.editable.as_deref() != Some(&m.rel)));
                if let State::Failed(why) = &m.state {
                    row.insert("error".into(), Value::from(why.clone()));
                }
                Value::Object(row)
            })
            .collect();
        json!({"project": self.manifest_path.to_string_lossy(), "id": self.id,
               "session_file": self.session_file.to_string_lossy(),
               "active_file": editable, "files": files,
               "problems": self.problems().into_iter().map(|(p, t)| json!({"path": p, "text": t})).collect::<Vec<_>>()})
    }

    pub fn state_word(&self, abs: &Path) -> Option<&str> {
        self.member(abs).map(|m| m.state.word())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn project(dir: &Path, files: &[(&str, &str)]) -> PathBuf {
        let path = dir.join("Job.builtproject");
        let list: Vec<Value> = files.iter().map(|(p, r)| json!({"path": p, "role": r})).collect();
        let value = json!({"built_project": 1, "id": manifest::new_id(), "files": list, "links": [],
                           "bim_registry": {}, "ifc_identity": {"namespace": manifest::new_id()}});
        manifest::save(&path, &value).unwrap();
        for (p, _) in files {
            if !p.starts_with("gone") {
                fs::create_dir_all(dir.join(p).parent().unwrap()).unwrap();
                fs::write(dir.join(p), b"x").unwrap();
            }
        }
        path
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bpsession-{name}-{}", manifest::new_id()));
        fs::create_dir_all(&d).unwrap();
        manifest::normalize(&d)
    }

    #[test]
    fn two_people_keep_their_own_tabs() {
        let dir = tmp("two");
        let path = project(&dir, &[("a.built", "host"), ("b.built", "host"), ("gone.built", "host")]);
        let (alice, bob) = (dir.join("alice"), dir.join("bob"));
        let mut a = ProjectSession::open(&path, Some(&alice)).unwrap();
        assert_eq!(a.members.iter().filter(|m| m.state == State::Missing).count(), 1);
        assert_eq!(a.editable.as_deref(), Some("a.built"));
        a.set_state(&dir.join("a.built"), State::Open);
        a.set_state(&dir.join("b.built"), State::Open);
        a.set_editable(Some(&dir.join("b.built"))).unwrap();
        assert!(a.record(vec![(dir.join("b.built"), json!({"distance": 3}), Value::Null),
                              (dir.join("a.built"), Value::Null, json!("Entrance"))], None, true).unwrap());
        assert!(!a.record(vec![(dir.join("b.built"), json!({"distance": 3}), Value::Null),
                               (dir.join("a.built"), Value::Null, json!("Entrance"))], None, true).unwrap());
        let b = ProjectSession::open(&path, Some(&bob)).unwrap();
        assert_eq!(b.editable.as_deref(), Some("a.built"));
        let again = ProjectSession::open(&path, Some(&alice)).unwrap();
        let order: Vec<&str> = again.members.iter().filter(|m| m.tab).map(|m| m.rel.as_str()).collect();
        assert_eq!(order, ["b.built", "a.built", "gone.built"]);
        assert_eq!(again.editable.as_deref(), Some("b.built"));
        assert_eq!(again.members[0].camera, json!({"distance": 3}));
        assert_eq!(again.members[1].scene, json!("Entrance"));
        assert!(again.is_read_only(&dir.join("a.built")));
    }
}
