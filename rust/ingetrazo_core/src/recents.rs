// SPDX-License-Identifier: GPL-3.0-or-later
//! The recent documents and projects: one list per kind of pinnable rows
//! (architecture review 15: they were written twice, as rich rows and as a
//! legacy path list, and clearing the native menu cleared only the latter).
//!
//! The settings store holds the rows as JSON (`home/recent_files`,
//! `home/recent_projects`). Before that list existed the
//! app kept a plain path list (`recent_files`); a store without the rich
//! list reads it once, and the first save drops it after writing the rich
//! one, so the user's recents survive and nothing brings them back later.
use pyo3::{
    exceptions::{PyTypeError, PyValueError},
    prelude::*,
    types::{PyDict, PyList, PyString},
};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// How many rows a list keeps.
pub const LIMIT: usize = 20;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Recent {
    pub path: String,
    #[serde(default)]
    pub pinned: bool,
}

/// The rows of a list: the stored JSON when there is one (even empty: a
/// cleared list stays cleared), else the legacy path list.
pub fn rows(stored: Option<&str>, legacy: &[String]) -> Vec<Recent> {
    let parsed = stored.and_then(|s| serde_json::from_str::<Vec<serde_json::Value>>(s).ok());
    let rows: Vec<Recent> = match parsed {
        Some(values) => values
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect(),
        None => legacy
            .iter()
            .filter(|p| !p.trim().is_empty())
            .map(|p| Recent {
                path: p.clone(),
                pinned: false,
            })
            .collect(),
    };
    rows.into_iter().take(LIMIT).collect()
}

/// `path` to the top (pinned rows first), keeping its pin unless `pinned`
/// says otherwise; `replace` is the path it was opened as before (a moved
/// file found again), which goes.
pub fn remember(
    rows: &[Recent],
    path: &str,
    pinned: Option<bool>,
    replace: Option<&str>,
) -> Vec<Recent> {
    let gone = |p: &str| p == path || Some(p) == replace;
    let before = rows
        .iter()
        .find(|r| gone(&r.path))
        .map_or(false, |r| r.pinned);
    let mut out = vec![Recent {
        path: path.to_owned(),
        pinned: pinned.unwrap_or(before),
    }];
    out.extend(rows.iter().filter(|r| !gone(&r.path)).cloned());
    // Stable: pinned rows first, each group in its order.
    out.sort_by_key(|r| !r.pinned);
    out.truncate(LIMIT);
    out
}

/// The rows without `gone`.
pub fn forget(rows: &[Recent], gone: &[String]) -> Vec<Recent> {
    rows.iter()
        .filter(|r| !gone.contains(&r.path))
        .cloned()
        .collect()
}

type Obj<'py> = Bound<'py, PyAny>;

fn store<'py>(py: Python<'py>, settings: Option<&Obj<'py>>) -> PyResult<Obj<'py>> {
    match settings {
        Some(st) => Ok(st.clone()),
        None => py.import("core.settings")?.call_method0("settings"),
    }
}

fn load(st: &Obj<'_>, kind: &str) -> PyResult<Vec<Recent>> {
    let stored = st.call_method1("value", (format!("home/recent_{kind}"),))?;
    let mut legacy = Vec::new();
    if kind == "files" {
        let old = st.call_method1("value", ("recent_files",))?;
        if let Ok(path) = old.extract::<String>() {
            legacy.push(path);
        } else if let Ok(paths) = old.try_iter() {
            for path in paths {
                legacy.push(path?.str()?.to_string());
            }
        }
    }
    Ok(rows(stored.extract::<String>().ok().as_deref(), &legacy))
}

fn save(st: &Obj<'_>, kind: &str, rows: &[Recent]) -> PyResult<()> {
    let data = serde_json::to_string(rows).map_err(|e| PyValueError::new_err(e.to_string()))?;
    st.call_method1("setValue", (format!("home/recent_{kind}"), data))?;
    if kind == "files" {
        st.call_method1("remove", ("recent_files",))?;
    }
    Ok(())
}

fn resolved(path: &Obj<'_>) -> PyResult<String> {
    // The leaf (or several of its parents) may have been moved or deleted.
    path.py()
        .import("os.path")?
        .call_method1("realpath", (path,))?
        .extract()
}

#[pyfunction]
#[pyo3(signature=(kind="files", settings=None))]
fn entries<'py>(
    py: Python<'py>,
    kind: &str,
    settings: Option<&Obj<'py>>,
) -> PyResult<Bound<'py, PyList>> {
    let result = PyList::empty(py);
    for row in load(&store(py, settings)?, kind)? {
        let item = PyDict::new(py);
        item.set_item("path", row.path)?;
        item.set_item("pinned", row.pinned)?;
        result.append(item)?;
    }
    Ok(result)
}

#[pyfunction]
#[pyo3(signature=(settings=None, limit=10))]
fn menu(py: Python<'_>, settings: Option<&Obj<'_>>, limit: usize) -> PyResult<Vec<String>> {
    Ok(load(&store(py, settings)?, "files")?
        .into_iter()
        .filter(|r| Path::new(&r.path).exists())
        .take(limit)
        .map(|r| r.path)
        .collect())
}

#[pyfunction(name = "remember")]
#[pyo3(signature=(path, pinned=None, settings=None, replace=None))]
fn record(
    path: &Obj<'_>,
    pinned: Option<bool>,
    settings: Option<&Obj<'_>>,
    replace: Option<&Obj<'_>>,
) -> PyResult<()> {
    let st = store(path.py(), settings)?;
    let path = resolved(path)?;
    let kind = match Path::new(&path).extension().and_then(|s| s.to_str()) {
        Some(ext)
            if ext.eq_ignore_ascii_case("builtproject") || ext.eq_ignore_ascii_case("igzproj") =>
        {
            "projects"
        }
        _ => "files",
    };
    let replaced = replace
        .map(|p| p.str().map(|s| s.to_string()))
        .transpose()?;
    let updated = remember(&load(&st, kind)?, &path, pinned, replaced.as_deref());
    save(&st, kind, &updated)?;
    if kind == "files" && pinned.is_none() {
        st.call_method1("setValue", ("home/last_file", path))?;
    }
    Ok(())
}

// JSON objects keep insertion order: an updated document moves to the
// end, so eviction always removes the least recently noted document.
fn metadata<'py>(st: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let text = st.call_method1("value", ("home/meta", "{}"))?;
    let json = st.py().import("json")?;
    match json.call_method1("loads", (text,)) {
        Ok(v) => Ok(v
            .cast_into::<PyDict>()
            .unwrap_or_else(|_| PyDict::new(st.py()))),
        Err(e)
            if e.is_instance_of::<PyTypeError>(st.py())
                || e.is_instance_of::<PyValueError>(st.py()) =>
        {
            Ok(PyDict::new(st.py()))
        }
        Err(e) => Err(e),
    }
}

fn save_meta(st: &Obj<'_>, table: &Bound<'_, PyDict>) -> PyResult<()> {
    let text = st.py().import("json")?.call_method1("dumps", (table,))?;
    st.call_method1("setValue", ("home/meta", text))?;
    Ok(())
}

fn forget_paths(st: &Obj<'_>, paths: &[String]) -> PyResult<()> {
    for kind in ["files", "projects"] {
        let rows = load(st, kind)?;
        let kept = forget(&rows, paths);
        if kept.len() != rows.len() {
            save(st, kind, &kept)?;
        }
    }
    if let Ok(last) = st
        .call_method1("value", ("home/last_file", ""))?
        .extract::<String>()
    {
        if paths.contains(&last) {
            st.call_method1("remove", ("home/last_file",))?;
        }
    }
    let table = metadata(st)?;
    let old_len = table.len();
    for key in paths {
        if table.contains(key)? {
            table.del_item(key)?;
        }
    }
    if old_len != table.len() {
        save_meta(st, &table)?;
    }
    Ok(())
}

#[pyfunction(name = "forget")]
#[pyo3(signature=(paths, settings=None))]
fn remove_paths(paths: &Obj<'_>, settings: Option<&Obj<'_>>) -> PyResult<()> {
    let mut names = Vec::new();
    let mut add = |p: &Obj<'_>| -> PyResult<()> {
        names.push(p.str()?.to_string());
        names.push(resolved(p)?);
        Ok(())
    };
    if paths.is_instance_of::<PyString>() || paths.hasattr("__fspath__")? {
        add(paths)?;
    } else {
        for p in paths.try_iter()? {
            add(&p?)?;
        }
    }
    forget_paths(&store(paths.py(), settings)?, &names)
}

#[pyfunction]
#[pyo3(signature=(settings=None))]
fn clear(py: Python<'_>, settings: Option<&Obj<'_>>) -> PyResult<()> {
    let st = store(py, settings)?;
    let unpinned = PyList::new(
        py,
        load(&st, "files")?
            .into_iter()
            .filter_map(|r| (!r.pinned).then_some(r.path))
            .collect::<Vec<_>>(),
    )?;
    remove_paths(unpinned.as_any(), Some(&st))?;
    save(&st, "files", &load(&st, "files")?)
}

#[pyfunction]
#[pyo3(signature=(settings=None))]
fn query<'py>(py: Python<'py>, settings: Option<&Obj<'py>>) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    let st = store(py, settings)?;
    for kind in ["files", "projects"] {
        out.set_item(kind, entries(py, kind, Some(&st))?)?;
    }
    Ok(out)
}

#[pyfunction]
#[pyo3(signature=(path, settings=None))]
fn meta<'py>(path: &Obj<'py>, settings: Option<&Obj<'py>>) -> PyResult<Bound<'py, PyDict>> {
    let table = metadata(&store(path.py(), settings)?)?;
    Ok(table
        .get_item(resolved(path)?)?
        .and_then(|v| v.cast_into::<PyDict>().ok())
        .unwrap_or_else(|| PyDict::new(path.py())))
}

#[pyfunction]
#[pyo3(signature=(path, data, settings=None))]
fn note(path: &Obj<'_>, data: &Bound<'_, PyDict>, settings: Option<&Obj<'_>>) -> PyResult<()> {
    let st = store(path.py(), settings)?;
    let table = metadata(&st)?;
    let key = resolved(path)?;
    let row = table
        .call_method1("pop", (&key, path.py().None()))?
        .cast_into::<PyDict>()
        .unwrap_or_else(|_| PyDict::new(path.py()));
    for (k, v) in data.iter() {
        if !v.is_none() {
            row.set_item(k, v)?;
        }
    }
    table.set_item(key, row)?;
    let excess = table.len().saturating_sub(LIMIT * 2);
    if excess > 0 {
        for key in table.keys().iter().take(excess) {
            table.del_item(key)?;
        }
    }
    save_meta(&st, &table)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let name = "ingetrazo_core.recent";
    let sub = PyModule::new(m.py(), name)?;
    sub.add("LIMIT", LIMIT)?;
    sub.add_function(wrap_pyfunction!(entries, &sub)?)?;
    sub.add_function(wrap_pyfunction!(menu, &sub)?)?;
    sub.add_function(wrap_pyfunction!(record, &sub)?)?;
    sub.add_function(wrap_pyfunction!(remove_paths, &sub)?)?;
    sub.add_function(wrap_pyfunction!(clear, &sub)?)?;
    sub.add_function(wrap_pyfunction!(query, &sub)?)?;
    sub.add_function(wrap_pyfunction!(meta, &sub)?)?;
    sub.add_function(wrap_pyfunction!(note, &sub)?)?;
    m.py()
        .import("sys")?
        .getattr("modules")?
        .set_item(name, &sub)?;
    m.add("recent", sub)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(path: &str, pinned: bool) -> Recent {
        Recent {
            path: path.into(),
            pinned,
        }
    }

    #[test]
    fn the_legacy_list_is_read_only_while_there_is_no_rich_one() {
        let legacy = vec!["/a.built".to_owned(), String::new(), "/b.built".to_owned()];
        assert_eq!(
            rows(None, &legacy),
            [r("/a.built", false), r("/b.built", false)]
        );
        assert!(rows(Some("[]"), &legacy).is_empty()); // cleared stays cleared
        assert_eq!(
            rows(
                Some(r#"[{"path":"/c.built","pinned":true},{"x":1}]"#),
                &legacy
            ),
            [r("/c.built", true)]
        );
        assert_eq!(rows(Some("not json"), &legacy).len(), 2); // unreadable: as if absent
    }

    #[test]
    fn remembering_keeps_pins_first_and_the_limit() {
        let list = vec![r("/p", true), r("/a", false), r("/b", false)];
        assert_eq!(
            remember(&list, "/b", None, None),
            [r("/p", true), r("/b", false), r("/a", false)]
        );
        assert_eq!(
            remember(&list, "/a", Some(true), None)[..2],
            [r("/a", true), r("/p", true)]
        );
        assert_eq!(
            remember(&list, "/moved", None, Some("/a")),
            [r("/p", true), r("/moved", false), r("/b", false)]
        );
        let many: Vec<Recent> = (0..30).map(|i| r(&format!("/{i}"), false)).collect();
        assert_eq!(remember(&many, "/new", None, None).len(), LIMIT);
        assert_eq!(
            forget(&list, &["/a".into()]),
            [r("/p", true), r("/b", false)]
        );
    }
}
