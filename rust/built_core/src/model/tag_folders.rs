//! The tag-folder tree's queries (`built_core.layers`): a folder's parent, the
//! folders inside one, and the order the Layers tab lists tags in, folders
//! unfolded.
//!
//! The tree is the document's `folder_parents` table over its folder list.
//! A parent that is not a folder, or one that would make a loop, reads as
//! the top, so every walk here ends. Answers match the Python ones
//! (`tests/test_tag_folder_parity.py`).

use std::collections::{HashMap, HashSet};

use pyo3::prelude::*;
use pyo3::types::PyDict;

type Parents = HashMap<String, Option<String>>;

/// `folder_parents` read leniently: a value that is not a string is none.
fn parents_of(table: &Bound<'_, PyAny>) -> PyResult<Parents> {
    let mut out = Parents::new();
    if let Ok(d) = table.cast::<PyDict>() {
        for (k, v) in d.iter() {
            if let Ok(k) = k.extract::<String>() {
                out.insert(k, v.extract::<String>().ok());
            }
        }
    }
    Ok(out)
}

/// The folder `name` sits in, or `None` at the top.
pub fn parent_in(parents: &Parents, known: &HashSet<String>, name: &str) -> Option<String> {
    let parent = parents.get(name).cloned().flatten()?;
    if parent.is_empty() || !known.contains(&parent) {
        return None;
    }
    let mut seen: HashSet<&str> = HashSet::from([name]);
    let mut up: Option<&str> = Some(parent.as_str());
    while let Some(u) = up {
        if !seen.insert(u) {
            return None;
        }
        up = parents.get(u).and_then(|p| p.as_deref()).filter(|p| known.contains(*p));
    }
    Some(parent)
}

fn below(folders: &[String], parents: &Parents) -> HashMap<Option<String>, Vec<String>> {
    let known: HashSet<String> = folders.iter().cloned().collect();
    let mut out: HashMap<Option<String>, Vec<String>> = HashMap::new();
    for f in folders {
        out.entry(parent_in(parents, &known, f)).or_default().push(f.clone());
    }
    out
}

fn walk(below: &HashMap<Option<String>, Vec<String>>, folder: &Option<String>, deep: bool) -> Vec<String> {
    let mut out = below.get(folder).cloned().unwrap_or_default();
    if deep {
        let children = out.clone();
        for f in children {
            out.extend(walk(below, &Some(f), deep));
        }
    }
    out
}

#[pyfunction]
pub fn tag_folder_parent(parents: &Bound<'_, PyAny>, known: Vec<String>, name: &str) -> PyResult<Option<String>> {
    let known: HashSet<String> = known.into_iter().collect();
    Ok(parent_in(&parents_of(parents)?, &known, name))
}

/// The folders directly in `name` (`None`: the top), or at any depth.
#[pyfunction]
#[pyo3(signature = (folders, parents, name, deep=false))]
pub fn tag_subfolders(folders: Vec<String>, parents: &Bound<'_, PyAny>, name: Option<String>, deep: bool)
    -> PyResult<Vec<String>> {
    let b = below(&folders, &parents_of(parents)?);
    Ok(walk(&b, &name, deep))
}

/// The tag keys in the Layers tab's order ("Sort by order created"):
/// `default` first, then each folder and each tag out of one as made —
/// what a folder holds inside it, its subfolders first. `tags`: `(key,
/// folder)` in document order.
#[pyfunction]
pub fn tag_ordered_keys(tags: Vec<(String, Option<String>)>, folders: Vec<String>, parents: &Bound<'_, PyAny>,
                        default: &str) -> PyResult<Vec<String>> {
    let parents = parents_of(parents)?;
    let place: HashMap<&str, usize> = tags.iter().enumerate().map(|(i, (k, _))| (k.as_str(), i)).collect();
    let folder_place: HashMap<&str, usize> = folders.iter().enumerate().map(|(i, f)| (f.as_str(), i)).collect();
    let mut held: HashMap<Option<&str>, Vec<&str>> = HashMap::new();
    for (key, folder) in &tags {
        if key != default {
            held.entry(folder.as_deref()).or_default().push(key);
        }
    }
    let below = below(&folders, &parents);
    fn block<'a>(folder: Option<&'a str>, below: &'a HashMap<Option<String>, Vec<String>>,
                 held: &HashMap<Option<&'a str>, Vec<&'a str>>, place: &HashMap<&str, usize>,
                 folder_place: &HashMap<&str, usize>, out: &mut Vec<String>) {
        let mut subs: Vec<&String> = below.get(&folder.map(str::to_string)).map(|v| v.iter().collect()).unwrap_or_default();
        subs.sort_by_key(|f| folder_place[f.as_str()]);
        for f in subs {
            block(Some(f.as_str()), below, held, place, folder_place, out);
        }
        let mut mine: Vec<&str> = held.get(&folder).cloned().unwrap_or_default();
        mine.sort_by_key(|k| place[k]);
        out.extend(mine.into_iter().map(str::to_string));
    }
    let mut out = Vec::new();
    if place.contains_key(default) {
        out.push(default.to_string());
    }
    block(None, &below, &held, &place, &folder_place, &mut out);
    Ok(out)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(tag_folder_parent, m)?)?;
    m.add_function(wrap_pyfunction!(tag_subfolders, m)?)?;
    m.add_function(wrap_pyfunction!(tag_ordered_keys, m)?)?;
    Ok(())
}
