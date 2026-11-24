// SPDX-License-Identifier: GPL-3.0-or-later
//! The recent documents and projects: one list per kind of pinnable rows
//! (architecture review 15: they were written twice, as rich rows and as a
//! legacy path list, and clearing the native menu cleared only the latter).
//!
//! The settings store holds the rows as JSON (`home/recent_files`,
//! `home/recent_projects`, core/recent.py). Before that list existed the
//! app kept a plain path list (`recent_files`); a store without the rich
//! list reads it once, and core/recent.py drops it after writing the rich
//! one, so the user's recents survive and nothing brings them back later.
use pyo3::prelude::*;
use serde::{Deserialize, Serialize};

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
        Some(values) => values.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect(),
        None => legacy.iter().filter(|p| !p.trim().is_empty())
            .map(|p| Recent { path: p.clone(), pinned: false }).collect(),
    };
    rows.into_iter().take(LIMIT).collect()
}

/// `path` to the top (pinned rows first), keeping its pin unless `pinned`
/// says otherwise; `replace` is the path it was opened as before (a moved
/// file found again), which goes.
pub fn remember(rows: &[Recent], path: &str, pinned: Option<bool>, replace: Option<&str>) -> Vec<Recent> {
    let gone = |p: &str| p == path || Some(p) == replace;
    let before = rows.iter().find(|r| gone(&r.path)).map_or(false, |r| r.pinned);
    let mut out = vec![Recent { path: path.to_owned(), pinned: pinned.unwrap_or(before) }];
    out.extend(rows.iter().filter(|r| !gone(&r.path)).cloned());
    // Stable: pinned rows first, each group in its order.
    out.sort_by_key(|r| !r.pinned);
    out.truncate(LIMIT);
    out
}

/// The rows without `gone`.
pub fn forget(rows: &[Recent], gone: &[String]) -> Vec<Recent> {
    rows.iter().filter(|r| !gone.contains(&r.path)).cloned().collect()
}

fn parse(json: &str) -> PyResult<Vec<Recent>> {
    serde_json::from_str(json).map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
}
fn dump(rows: &[Recent]) -> String {
    serde_json::to_string(rows).unwrap_or_else(|_| "[]".into())
}

/// The list as JSON from the stored value (None when the store has none)
/// and the legacy path list.
#[pyfunction]
#[pyo3(signature = (stored, legacy=Vec::new()))]
pub fn recent_rows(stored: Option<String>, legacy: Vec<String>) -> String {
    dump(&rows(stored.as_deref(), &legacy))
}

#[pyfunction]
#[pyo3(signature = (rows_json, path, pinned=None, replace=None))]
pub fn recent_remember(rows_json: &str, path: &str, pinned: Option<bool>, replace: Option<String>) -> PyResult<String> {
    Ok(dump(&remember(&parse(rows_json)?, path, pinned, replace.as_deref())))
}

#[pyfunction]
pub fn recent_forget(rows_json: &str, gone: Vec<String>) -> PyResult<String> {
    Ok(dump(&forget(&parse(rows_json)?, &gone)))
}

/// File ▸ Open Recent: the paths of the rows whose file is still there,
/// at most `limit`.
#[pyfunction]
#[pyo3(signature = (rows_json, limit=10))]
pub fn recent_menu(rows_json: &str, limit: usize) -> PyResult<Vec<String>> {
    Ok(parse(rows_json)?.into_iter().filter(|r| std::path::Path::new(&r.path).exists())
        .take(limit).map(|r| r.path).collect())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(recent_rows, m)?)?;
    m.add_function(wrap_pyfunction!(recent_remember, m)?)?;
    m.add_function(wrap_pyfunction!(recent_forget, m)?)?;
    m.add_function(wrap_pyfunction!(recent_menu, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(path: &str, pinned: bool) -> Recent {
        Recent { path: path.into(), pinned }
    }

    #[test]
    fn the_legacy_list_is_read_only_while_there_is_no_rich_one() {
        let legacy = vec!["/a.built".to_owned(), String::new(), "/b.built".to_owned()];
        assert_eq!(rows(None, &legacy), [r("/a.built", false), r("/b.built", false)]);
        assert!(rows(Some("[]"), &legacy).is_empty());                // cleared stays cleared
        assert_eq!(rows(Some(r#"[{"path":"/c.built","pinned":true},{"x":1}]"#), &legacy), [r("/c.built", true)]);
        assert_eq!(rows(Some("not json"), &legacy).len(), 2);            // unreadable: as if absent
    }

    #[test]
    fn remembering_keeps_pins_first_and_the_limit() {
        let list = vec![r("/p", true), r("/a", false), r("/b", false)];
        assert_eq!(remember(&list, "/b", None, None), [r("/p", true), r("/b", false), r("/a", false)]);
        assert_eq!(remember(&list, "/a", Some(true), None)[..2], [r("/a", true), r("/p", true)]);
        assert_eq!(remember(&list, "/moved", None, Some("/a")), [r("/p", true), r("/moved", false), r("/b", false)]);
        let many: Vec<Recent> = (0..30).map(|i| r(&format!("/{i}"), false)).collect();
        assert_eq!(remember(&many, "/new", None, None).len(), LIMIT);
        assert_eq!(forget(&list, &["/a".into()]), [r("/p", true), r("/b", false)]);
    }
}
