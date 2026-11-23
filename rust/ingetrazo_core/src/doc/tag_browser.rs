// SPDX-License-Identifier: GPL-3.0-or-later
//! Document-side snapshot for the chrome Tags tab. Keys address edits;
//! labels (including ancestor folders) are only for display.
use super::{attr_id, lock, value::Value, Data, Doc, FVal, Field, Key, Node, Trash};
use std::collections::{HashMap, HashSet};

fn labels<'a>(root: &'a Node, name: &str) -> Result<HashMap<&'a str, &'a str>, String> {
    let Some(Value::Dict(entries)) = root.attr(attr_id(name)?) else {
        return Ok(HashMap::new());
    };
    Ok(entries
        .iter()
        .filter_map(|(k, v)| match (k, v) {
            (Value::Str(key), Value::Str(value)) if !value.is_empty() => {
                Some((key.as_str(), value.as_str()))
            }
            _ => None,
        })
        .collect())
}

type Row = (String, String, bool, bool, String, String);
pub(super) type Snapshot = (Vec<Row>, Vec<(String, String)>, String);

fn explicit_folders(root: &Node) -> Result<Vec<&str>, String> {
    let mut keys = Vec::new();
    if let Some(Value::List(items) | Value::Tuple(items)) = root.attr(attr_id("layer_folders")?) {
        for item in items {
            if let Value::Str(key) = item {
                if !key.is_empty() {
                    keys.push(key.as_str());
                }
            }
        }
    }
    Ok(keys)
}

pub(super) fn snapshot(doc: &Doc, owner: Key) -> Result<Snapshot, String> {
    let root = doc.need(owner)?;
    let parents = labels(root, "folder_parents")?;
    let names = labels(root, "layer_folder_names")?;
    let mut folders = explicit_folders(root)?;
    for &id in &root.tags {
        if let Data::Tag(tag) = &doc.need(id)?.data {
            if let Some(key) = tag.folder.as_deref() {
                folders.push(key);
            }
        }
    }
    let mut known = HashSet::new();
    folders.retain(|key| known.insert(*key));
    let path = |key: &str| {
        let mut chain = vec![];
        let mut seen = HashSet::new();
        let mut next = Some(key);
        while let Some(folder) = next {
            if !known.contains(folder) || !seen.insert(folder) {
                break;
            }
            chain.push(names.get(folder).copied().unwrap_or(folder));
            next = parents.get(folder).copied();
        }
        chain.reverse();
        chain.join(" / ")
    };
    let folders: Vec<_> = folders
        .into_iter()
        .map(|key| (key.to_owned(), path(key)))
        .collect();
    let paths: HashMap<_, _> = folders.iter().map(|(k, v)| (k.as_str(), v)).collect();
    let mut rows = Vec::with_capacity(root.tags.len());
    for &id in &root.tags {
        let Data::Tag(tag) = &doc.need(id)?.data else {
            continue;
        };
        let Some(key) = &tag.key else { continue };
        let name = tag.name.as_ref().unwrap_or(key);
        let folder = tag.folder.as_deref().unwrap_or("");
        let label = paths
            .get(folder)
            .map_or_else(|| name.clone(), |p| format!("{p} / {name}"));
        rows.push((
            key.clone(),
            label,
            tag.visible,
            tag.locked,
            name.clone(),
            folder.to_owned(),
        ));
    }
    let active = match root.attr(attr_id("active_layer")?) {
        Some(Value::Str(s)) => s.clone(),
        _ => "Layer 0".into(),
    };
    Ok((rows, folders, active))
}

use pyo3::{
    exceptions::{PyKeyError, PyValueError},
    prelude::*,
};

/// Prepared before History starts, so invalid input cannot partly edit a
/// multi-tag selection. Each handle is a view of a native document tag.
#[pyclass(frozen)]
pub(super) struct TagEdit {
    changes: Vec<(Key, Field, FVal, FVal)>,
}

impl TagEdit {
    fn stage(&mut self, doc: &Doc, id: Key, field: Field, after: FVal) -> Result<(), String> {
        let before = doc.need(id)?.get(field)?;
        if before != after {
            self.changes.push((id, field, before, after));
        }
        Ok(())
    }
}

#[pymethods]
impl TagEdit {
    fn apply(&self, py: Python<'_>, forward: bool) -> PyResult<()> {
        let mut trash = Trash::default();
        let mut doc = lock();
        for (key, field, before, after) in &self.changes {
            let value = if forward { after } else { before };
            doc.write(py, *key, *field, value.clone(), &mut trash)
                .map_err(PyValueError::new_err)?;
        }
        Ok(())
    }
}

fn drawing_value(py: Python<'_>, field: &str, input: &str) -> PyResult<(Field, FVal)> {
    let (field, value) = match field {
        "color" if input.is_empty() => ("color", Value::None),
        "color" => {
            let hex = input
                .strip_prefix('#')
                .filter(|s| s.len() == 6 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                .ok_or_else(|| PyValueError::new_err("expected #RRGGBB"))?;
            let rgb =
                u32::from_str_radix(hex, 16).map_err(|e| PyValueError::new_err(e.to_string()))?;
            (
                "color",
                Value::Tuple(
                    [16, 8, 0]
                        .map(|shift| Value::Float(((rgb >> shift) & 255u32) as f64 / 255.0))
                        .into(),
                ),
            )
        }
        "weight" if input == "auto" => ("lineweight", Value::None),
        "weight" => {
            let mm = input
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite() && *n > 0.0)
                .ok_or_else(|| PyValueError::new_err("expected a positive pen width in mm"))?;
            ("lineweight", Value::Float(mm))
        }
        "linetype"
            if py
                .import("core.layers")?
                .call_method1("known_linetype", (input,))?
                .is_truthy()? =>
        {
            ("linetype", Value::Str(input.to_string()))
        }
        "print" if matches!(input, "0" | "1") => ("printable", Value::Bool(input == "1")),
        _ => return Err(PyValueError::new_err("invalid tag drawing style")),
    };
    let field = if field == "color" {
        Field::Color
    } else {
        Field::Attr(attr_id(field).map_err(PyValueError::new_err)?)
    };
    Ok((field, FVal::Val((value != Value::None).then_some(value))))
}

pub(super) fn prepare(
    py: Python<'_>,
    owner: Key,
    tags: Vec<String>,
    field: &str,
    input: &str,
) -> PyResult<Option<TagEdit>> {
    let change = match field {
        "active" => None,
        "name" if input.trim().is_empty() || input == "Layer 0" => {
            return Err(PyValueError::new_err("Choose a tag name"))
        }
        "name" => Some((Field::Name, FVal::Str(Some(input.to_owned())))),
        "folder" => Some((
            Field::Folder,
            FVal::Str((!input.is_empty()).then(|| input.to_owned())),
        )),
        _ => Some(drawing_value(py, field, input)?),
    };
    let doc = lock();
    let root = doc.need(owner).map_err(PyValueError::new_err)?;
    let mut needed: HashSet<_> = tags.into_iter().collect();
    if field == "active" && needed.len() != 1 {
        return Err(PyValueError::new_err("Choose one active tag"));
    }
    let mut known_folder = input.is_empty()
        || field != "folder"
        || explicit_folders(root)
            .map_err(PyValueError::new_err)?
            .contains(&input);
    let mut edit = TagEdit { changes: vec![] };
    for &id in &root.tags {
        let Data::Tag(tag) = &doc.need(id).map_err(PyValueError::new_err)?.data else {
            continue;
        };
        known_folder |= tag.folder.as_deref() == Some(input);
        let Some(key) = &tag.key else { continue };
        if !needed.remove(key) {
            continue;
        }
        if let Some((prop, value)) = &change {
            if key == "Layer 0" && matches!(prop, Field::Name | Field::Folder) {
                return Err(PyValueError::new_err(
                    "The default tag keeps its name and folder",
                ));
            }
            edit.stage(&doc, id, *prop, value.clone())
                .map_err(PyValueError::new_err)?;
        } else {
            let active = (key != "Layer 0").then(|| Value::Str(key.clone()));
            if active.is_some() {
                edit.stage(&doc, id, Field::Visible, FVal::Bool(true))
                    .map_err(PyValueError::new_err)?;
                edit.stage(&doc, id, Field::Locked, FVal::Bool(false))
                    .map_err(PyValueError::new_err)?;
            }
            let attr = attr_id("active_layer").map_err(PyValueError::new_err)?;
            edit.stage(&doc, owner, Field::Attr(attr), FVal::Val(active))
                .map_err(PyValueError::new_err)?;
        }
    }
    if let Some(key) = needed.iter().next() {
        return Err(PyKeyError::new_err(format!("no tag with key {key:?}")));
    }
    if !known_folder {
        return Err(PyKeyError::new_err(format!("no folder with key {input:?}")));
    }
    Ok((!edit.changes.is_empty()).then_some(edit))
}
