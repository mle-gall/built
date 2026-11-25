// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded, depth-first link previews for the Start sheet.
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use regex::bytes::Regex;
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek};
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

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

fn records(py: Python<'_>, p: &Path) -> PyResult<Vec<(String, String)>> {
    let mut file = File::open(p)?;
    let mut magic = [0u8; 4];
    if file.read(&mut magic)? == 4 && &magic == b"PK\x03\x04" {
        let archive = py
            .import("built_core")?
            .call_method1("DocumentArchive", (p,))?;
        let rows = (|| {
            let info = archive.call_method1("getinfo", ("document.json",))?;
            if info.getattr("file_size")?.extract::<u64>()? > MAX_DOC as u64 {
                return Ok(vec![]);
            }
            let bytes = archive
                .call_method1("read", (info,))?
                .cast_into::<PyBytes>()?;
            Ok(linked_files(bytes.as_bytes()))
        })();
        archive.call_method0("close")?;
        rows
    } else {
        file.rewind()?;
        let mut json = Vec::new();
        file.take((MAX_DOC + 1) as u64).read_to_end(&mut json)?;
        Ok(if json.len() <= MAX_DOC {
            linked_files(&json)
        } else {
            vec![]
        })
    }
}

fn walk<'py>(
    out: &Bound<'py, PyList>,
    seen: &mut HashSet<PathBuf>,
    file: &Path,
    level: usize,
) -> PyResult<()> {
    if level >= 3 || out.len() >= MAX_LINKS {
        return Ok(());
    }
    let py = out.py();
    for (path, rel) in records(py, file).unwrap_or_default() {
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
        out.append(row)?;
        if let Some(next) = found {
            if seen.insert(path_key(&next)) {
                walk(out, seen, &next, level + 1)?;
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
    walk(&out, &mut HashSet::from([path_key(&file)]), &file, 0)?;
    Ok(out)
}
