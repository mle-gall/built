// SPDX-License-Identifier: GPL-3.0-or-later
//! Built projects: the `.builtproject` manifest and the per-user session
//! (docs/builtproject-format.md). JSON crosses to Python as text.
pub mod manifest;
pub mod session;

use crate::project_tree::Row;
use manifest::Converted;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use serde_json::Value;
use session::{ProjectSession, State};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn err(e: String) -> PyErr {
    PyValueError::new_err(e)
}

fn parse(text: &str) -> PyResult<Value> {
    serde_json::from_str(text).map_err(|e| PyValueError::new_err(e.to_string()))
}

fn dump(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".into())
}

/// The validated manifest as JSON (the registry's contents are Python's).
#[pyfunction]
pub fn project_manifest_load(path: PathBuf) -> PyResult<String> {
    manifest::load(&path).map(|v| dump(&v)).map_err(err)
}

/// A manifest or an `.igzproj`, read-only (never converts).
#[pyfunction]
pub fn project_manifest_peek(path: PathBuf) -> PyResult<String> {
    manifest::load_any(&path).map(|v| dump(&v)).map_err(err)
}

#[pyfunction]
pub fn project_manifest_save(path: PathBuf, json: &str) -> PyResult<()> {
    manifest::save(&path, &parse(json)?).map_err(err)
}

#[pyfunction]
#[pyo3(signature = (path, registry_json, identity_json, id=None))]
pub fn project_manifest_new(path: PathBuf, registry_json: &str, identity_json: &str, id: Option<String>) -> PyResult<String> {
    manifest::create(&path, parse(registry_json)?, parse(identity_json)?, id)
        .map(|v| dump(&v))
        .map_err(err)
}

/// Saving host `host_rel`: its file entries and outgoing links (§4.1).
#[pyfunction]
pub fn project_sync_host(path: PathBuf, host_rel: &str, files_json: &str, links_json: &str) -> PyResult<()> {
    let files = parse(files_json)?.as_array().cloned().unwrap_or_default();
    let links = parse(links_json)?.as_array().cloned().unwrap_or_default();
    manifest::update(&path, |value| {
        manifest::sync_host(value, host_rel, &files, links.clone());
        Ok(())
    })
    .map_err(err)
}

/// A missing project file found at `new_path`: its entry and links follow.
#[pyfunction]
pub fn project_relocate(path: PathBuf, old_path: PathBuf, new_path: PathBuf) -> PyResult<()> {
    let dir = manifest::normalize(path.parent().unwrap_or(Path::new("")));
    let (old, new) = (manifest::relative(&old_path, &dir), manifest::relative(&new_path, &dir));
    manifest::update(&path, |value| {
        if manifest::relocate(value, &old, &new) {
            Ok(())
        } else {
            Err(format!("{old} is not a file of this project"))
        }
    })
    .map_err(err)
}

/// Three-way registry write (§4.2); the conflicting entries.
#[pyfunction]
pub fn project_write_registry(path: PathBuf, base_json: &str, ours_json: &str) -> PyResult<Vec<String>> {
    manifest::write_registry(&path, &parse(base_json)?, &parse(ours_json)?).map_err(err)
}

/// §5. (manifest path, "converted" | "already" | "none").
#[pyfunction]
pub fn project_convert(path: PathBuf) -> PyResult<(PathBuf, &'static str)> {
    let (p, done) = manifest::open(&path).map_err(err)?;
    Ok((p, match done {
        Some(Converted::Converted(_)) => "converted",
        Some(Converted::AlreadyConverted(_)) => "already",
        None => "none",
    }))
}

#[pyfunction]
pub fn project_legacy_json(path: PathBuf) -> PyResult<String> {
    manifest::load(&path).map(|v| dump(&manifest::to_legacy(&v))).map_err(err)
}

/// §2. The project a model's pointer finds, or None.
#[pyfunction]
#[pyo3(signature = (host_path, pointer_json))]
pub fn project_resolve_pointer(host_path: Option<PathBuf>, pointer_json: &str) -> PyResult<Option<PathBuf>> {
    let pointer = parse(pointer_json)?;
    Ok(manifest::resolve_pointer(
        host_path.as_deref(),
        pointer.get("rel").and_then(Value::as_str),
        pointer.get("path").and_then(Value::as_str),
    ))
}

/// A path relative to `dir` in manifest spelling.
#[pyfunction]
pub fn project_relative(path: PathBuf, dir: PathBuf) -> String {
    manifest::relative(&path, &dir)
}

/// (absolute path, role) of each project file, without converting.
#[pyfunction]
pub fn project_members(path: PathBuf) -> PyResult<Vec<(String, String)>> {
    let value = manifest::load_any(&path).map_err(err)?;
    let dir = manifest::normalize(path.parent().unwrap_or(Path::new("")));
    Ok(value["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| {
            let rel = f["path"].as_str()?;
            Some((manifest::resolve(&dir, rel).to_string_lossy().into_owned(), f["role"].as_str()?.to_owned()))
        })
        .collect())
}

#[pyfunction]
#[pyo3(signature = (override_dir=None))]
pub fn project_session_dir(override_dir: Option<PathBuf>) -> PyResult<PathBuf> {
    session::session_dir(override_dir.as_deref()).map_err(err)
}

fn opt_json(value: Option<&str>) -> PyResult<Value> {
    value.map_or(Ok(Value::Null), parse)
}

#[pymethods]
impl ProjectSession {
    /// Open the project at `path` (converting an `.igzproj`) with this
    /// person's session from `settings_dir` (default §3).
    #[new]
    #[pyo3(signature = (path, settings_dir=None))]
    fn py_new(path: PathBuf, settings_dir: Option<PathBuf>) -> PyResult<Self> {
        ProjectSession::open(&path, settings_dir.as_deref()).map_err(err)
    }

    #[getter]
    fn path(&self) -> PathBuf {
        self.manifest_path.clone()
    }
    #[getter(id)]
    fn py_id(&self) -> String {
        self.id.clone()
    }
    #[getter(session_file)]
    fn py_session_file(&self) -> PathBuf {
        self.session_file.clone()
    }
    /// (what was opened, how): "converted" or "already", or None.
    #[getter]
    fn conversion(&self) -> Option<(PathBuf, &'static str)> {
        self.converted.as_ref().map(|(p, c)| {
            (p.clone(), if matches!(c, Converted::Converted(_)) { "converted" } else { "already" })
        })
    }

    /// The tabs to open, in order: (absolute path, role, camera JSON or
    /// None, scene or None, exists).
    fn plan(&self) -> Vec<(String, String, Option<String>, Option<String>, bool)> {
        self.members
            .iter()
            .filter(|m| m.tab)
            .map(|m| {
                (
                    m.abs.to_string_lossy().into_owned(),
                    m.role.clone(),
                    (!m.camera.is_null()).then(|| dump(&m.camera)),
                    m.scene.as_str().map(str::to_owned),
                    m.state != State::Missing,
                )
            })
            .collect()
    }

    fn loaded(&mut self, path: PathBuf) {
        self.set_state(&path, State::Open);
    }
    fn failed(&mut self, path: PathBuf, reason: String) {
        self.set_state(&path, State::Failed(reason));
    }
    /// A tab closed: the member stays in the project, no longer open.
    fn closed(&mut self, path: PathBuf) {
        if let Some(m) = self.members.iter_mut().find(|m| m.abs == manifest::normalize(&path)) {
            m.tab = false;
            if m.state == State::Open {
                m.state = State::Listed;
            }
        }
    }
    fn state_of(&self, path: PathBuf) -> Option<String> {
        self.state_word(&path).map(str::to_owned)
    }
    fn pending(&self) -> Vec<String> {
        self.members
            .iter()
            .filter(|m| m.tab && m.state == State::Pending)
            .map(|m| m.abs.to_string_lossy().into_owned())
            .collect()
    }
    fn is_member(&self, path: PathBuf) -> bool {
        self.member(&path).is_some()
    }

    #[getter(editable)]
    fn py_editable(&self) -> Option<String> {
        self.editable.as_ref().map(|e| self.abs(e).to_string_lossy().into_owned())
    }
    #[pyo3(name = "set_editable", signature = (path))]
    fn py_set_editable(&mut self, path: Option<PathBuf>) -> PyResult<()> {
        self.set_editable(path.as_deref()).map_err(err)
    }
    fn read_only(&self, path: PathBuf) -> bool {
        self.is_read_only(&path)
    }

    /// Write the session when it changed: `tabs` as (absolute path, camera
    /// JSON or None, scene or None), `window` as JSON or None. True if
    /// written. `keep_planned` keeps planned tabs that never came in
    /// (missing, failed, still opening); False makes `tabs` the whole list.
    #[pyo3(name = "record", signature = (tabs, window=None, keep_planned=true))]
    fn py_record(&mut self, tabs: Vec<(PathBuf, Option<String>, Option<String>)>, window: Option<String>, keep_planned: bool) -> PyResult<bool> {
        let mut rows = Vec::new();
        for (p, camera, scene) in tabs {
            rows.push((p, opt_json(camera.as_deref())?, scene.map_or(Value::Null, Value::from)));
        }
        let window = window.as_deref().map(parse).transpose()?;
        self.record(rows, window, keep_planned).map_err(err)
    }

    /// The `window` object of the session (JSON).
    fn window(&self) -> String {
        dump(self.session.get("window").unwrap_or(&Value::Null))
    }

    #[pyo3(name = "reload")]
    fn py_reload(&mut self) -> PyResult<()> {
        self.reload().map_err(err)
    }

    /// (absolute path, text) of each missing or failed member.
    #[pyo3(name = "problems")]
    fn py_problems(&self) -> Vec<(String, String)> {
        self.problems()
    }

    #[pyo3(name = "tree_rows", signature = (query, expanded))]
    fn py_tree_rows(&self, query: &str, expanded: HashSet<String>) -> Vec<Row> {
        self.tree_rows(query, &expanded)
    }

    /// The agent tools' `query_worksession` answer, as JSON.
    #[pyo3(name = "query")]
    fn py_query(&self) -> String {
        dump(&self.query())
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<ProjectSession>()?;
    m.add_function(wrap_pyfunction!(project_manifest_load, m)?)?;
    m.add_function(wrap_pyfunction!(project_manifest_peek, m)?)?;
    m.add_function(wrap_pyfunction!(project_manifest_save, m)?)?;
    m.add_function(wrap_pyfunction!(project_manifest_new, m)?)?;
    m.add_function(wrap_pyfunction!(project_sync_host, m)?)?;
    m.add_function(wrap_pyfunction!(project_write_registry, m)?)?;
    m.add_function(wrap_pyfunction!(project_relocate, m)?)?;
    m.add_function(wrap_pyfunction!(project_convert, m)?)?;
    m.add_function(wrap_pyfunction!(project_legacy_json, m)?)?;
    m.add_function(wrap_pyfunction!(project_resolve_pointer, m)?)?;
    m.add_function(wrap_pyfunction!(project_relative, m)?)?;
    m.add_function(wrap_pyfunction!(project_members, m)?)?;
    m.add_function(wrap_pyfunction!(project_session_dir, m)?)?;
    Ok(())
}
