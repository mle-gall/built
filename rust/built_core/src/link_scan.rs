// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded, depth-first link previews for the Start sheet.
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use regex::bytes::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek};
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

pub(super) const MAX_LEGACY: usize = 1024 * 1024;
#[derive(Default, Serialize, Deserialize)]
struct Summary {
    units: Option<String>,
    links: Vec<(String, String)>,
}

/// Save a compact index of placement links without visiting mesh data.
pub(super) fn encode_summary(payload: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
    let mut stack = Vec::new();
    fn append<'py>(
        stack: &mut Vec<Bound<'py, PyAny>>,
        row: &Bound<'py, PyAny>,
        key: &str,
    ) -> PyResult<()> {
        if let Some(rows) = row.cast::<PyDict>()?.get_item(key)? {
            stack.extend(
                rows.try_iter()?
                    .collect::<PyResult<Vec<_>>>()?
                    .into_iter()
                    .rev(),
            );
        }
        Ok(())
    }
    append(&mut stack, payload, "groups")?;
    let mut links = Vec::new();
    let mut seen = HashSet::new();
    while let Some(row) = stack.pop() {
        let Ok(d) = row.cast::<PyDict>() else {
            continue;
        };
        if let Some(l) = d
            .get_item("ext")?
            .and_then(|ext| ext.get_item("link").ok())
            .and_then(|v| v.cast_into::<PyDict>().ok())
        {
            if let Some(path) = l.get_item("path")?.and_then(|v| v.extract::<String>().ok()) {
                if seen.insert(path.clone()) {
                    links.push((
                        path,
                        l.get_item("rel")?
                            .and_then(|v| v.extract().ok())
                            .unwrap_or_default(),
                    ));
                    if links.len() == MAX_LINKS {
                        break;
                    }
                }
            }
        }
        append(&mut stack, &row, "children")?;
    }
    let units = payload
        .get_item("units")
        .and_then(|u| u.get_item("length"))
        .and_then(|u| u.extract())
        .unwrap_or_else(|_| "m".to_string());
    Ok(serde_json::to_vec(&Summary {
        units: Some(units),
        links,
    })
    .expect("string metadata"))
}

const MAX_DOC: usize = 64 << 20;
const MAX_LINKS: usize = 64;
static LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r#""link"\s*:\s*\{\s*"path"\s*:\s*("(?:[^"\\]|\\.)*")(?:\s*,\s*"rel"\s*:\s*("(?:[^"\\]|\\.)*"))?"#).unwrap()
});

fn linked_files(data: &[u8]) -> Vec<(String, String)> {
    let mut listed = HashSet::new();
    LINK.captures_iter(data)
        .filter_map(|cap| {
            let path: String = serde_json::from_slice(cap.get(1)?.as_bytes()).ok()?;
            let rel: String = match cap.get(2) {
                Some(v) => serde_json::from_slice(v.as_bytes()).ok()?,
                None => String::new(),
            };
            listed
                .insert(path_key(Path::new(&path)))
                .then_some((path, rel))
        })
        .take(MAX_LINKS)
        .collect()
}

fn path_key(p: &Path) -> PathBuf {
    // Match normcase(abspath(path)) without resolving symlinks. Two recorded
    // paths can point to the same file and still deserve distinct preview rows.
    let absolute = std::path::absolute(p).unwrap_or_else(|_| p.into());
    let mut path = PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::ParentDir => {
                path.pop();
            }
            Component::CurDir => {}
            _ => path.push(part.as_os_str()),
        }
    }
    #[cfg(windows)]
    let path = PathBuf::from(path.to_string_lossy().to_lowercase());
    path
}

fn records(py: Python<'_>, p: &Path, limit: usize) -> PyResult<Summary> {
    let legacy = |data: &[u8]| -> PyResult<Summary> {
        #[derive(Deserialize)]
        struct Document {
            scene: Option<Scene>,
        }
        #[derive(Deserialize)]
        struct Scene {
            units: Option<Units>,
        }
        #[derive(Deserialize)]
        struct Units {
            length: String,
        }
        let doc: Document = serde_json::from_slice(data)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        Ok(Summary {
            units: doc.scene.and_then(|s| s.units).map(|u| u.length).or_else(|| {
                p.extension()?.eq_ignore_ascii_case("built").then(|| "m".into())
            }),
            links: linked_files(data),
        })
    };
    let mut file = File::open(p)?;
    let mut magic = [0u8; 4];
    if file.read(&mut magic)? == 4 && &magic == b"PK\x03\x04" {
        let archive = py
            .import("built_core")?
            .call_method1("DocumentArchive", (p,))?;
        let rows = (|| {
            if archive
                .call_method0("namelist")?
                .contains("start-summary.json")?
            {
                let info = archive.call_method1("getinfo", ("start-summary.json",))?;
                if info.getattr("file_size")?.extract::<usize>()? <= MAX_LEGACY {
                    let data = archive
                        .call_method1("read", (info,))?
                        .cast_into::<PyBytes>()?;
                    if let Ok(summary) = serde_json::from_slice(data.as_bytes()) {
                        return Ok(summary);
                    }
                }
            }
            let info = archive.call_method1("getinfo", ("document.json",))?;
            if info.getattr("file_size")?.extract::<u64>()? > limit as u64 {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "No preview metadata",
                ));
            }
            let bytes = archive
                .call_method1("read", (info,))?
                .cast_into::<PyBytes>()?;
            legacy(bytes.as_bytes())
        })();
        archive.call_method0("close")?;
        rows
    } else {
        file.rewind()?;
        let mut json = Vec::new();
        file.take((limit + 1) as u64).read_to_end(&mut json)?;
        if json.len() > limit {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "No preview metadata",
            ));
        }
        legacy(&json)
    }
}

fn walk<'py>(
    out: &Bound<'py, PyList>,
    seen: &mut HashSet<PathBuf>,
    file: &Path,
    level: usize,
    records_here: Summary,
    limit: usize,
) -> PyResult<()> {
    if level >= 3 || out.len() >= MAX_LINKS {
        return Ok(());
    }
    let py = out.py();
    for (path, rel) in records_here.links {
        if out.len() >= MAX_LINKS {
            break;
        }
        let found = (!rel.is_empty())
            .then(|| file.with_file_name(&rel))
            .filter(|p| p.is_file())
            .and_then(|p| p.canonicalize().ok())
            .or_else(|| Some(PathBuf::from(&path)).filter(|p| p.is_file()));
        let row = PyDict::new(py);
        row.set_item("path", &path)?;
        row.set_item("found", found.as_ref().map(|p| p.to_string_lossy()))?;
        row.set_item("depth", level)?;
        row.set_item("via", file.to_string_lossy())?;
        let modified = |p: &Path| p.metadata().ok()?.modified().ok();
        let status = match found.as_deref() {
            None => "missing",
            Some(p)
                if modified(p)
                    .zip(modified(file))
                    .and_then(|(src, host)| src.duration_since(host).ok())
                    .is_some_and(|age| age.as_secs_f64() > 2.) =>
            {
                "changed"
            }
            Some(_) => "found",
        };
        if limit == MAX_LEGACY {
            row.set_item("status", status)?;
        }
        out.append(row)?;
        if let Some(next) = found {
            if seen.insert(path_key(&next)) {
                walk(
                    out,
                    seen,
                    &next,
                    level + 1,
                    records(py, &next, limit).unwrap_or_default(),
                    limit,
                )?;
            }
        }
    }
    Ok(())
}

#[pyfunction]
pub(crate) fn scan_document_links<'py>(
    py: Python<'py>,
    file: PathBuf,
) -> PyResult<Bound<'py, PyList>> {
    let out = PyList::empty(py);
    walk(
        &out,
        &mut HashSet::from([path_key(&file)]),
        &file,
        0,
        records(py, &file, MAX_DOC).unwrap_or_default(),
        MAX_DOC,
    )?;
    Ok(out)
}

#[pyfunction]
pub(crate) fn start_document_summary(py: Python<'_>, file: PathBuf) -> PyResult<Bound<'_, PyDict>> {
    let summary = records(py, &file, MAX_LEGACY)?;
    let result = PyDict::new(py);
    result.set_item("units", &summary.units)?;
    let links = PyList::empty(py);
    walk(
        &links,
        &mut HashSet::from([path_key(&file)]),
        &file,
        0,
        summary,
        MAX_LEGACY,
    )?;
    result.set_item("links", links)?;
    Ok(result)
}
