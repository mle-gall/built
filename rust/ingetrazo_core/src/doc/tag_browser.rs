// SPDX-License-Identifier: GPL-3.0-or-later
//! Document-side snapshot for the chrome Tags tab. Keys address edits;
//! labels (including ancestor folders) are only for display.
use super::{attr_id, value::Value, Data, Doc, Key, Node};
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

pub(super) fn rows(doc: &Doc, owner: Key) -> Result<Vec<(String, String, bool, bool)>, String> {
    let root = doc.need(owner)?;
    let parents = labels(root, "folder_parents")?;
    let names = labels(root, "layer_folder_names")?;
    let mut rows = Vec::with_capacity(root.tags.len());
    for handle in &root.tags {
        let Data::Tag(tag) = &doc.need(*handle)?.data else {
            continue;
        };
        let Some(key) = tag.key.as_ref() else {
            continue;
        };
        let mut path = vec![tag.name.as_deref().unwrap_or(key)];
        let mut seen = HashSet::new();
        let mut next = tag.folder.as_deref();
        while let Some(folder) = next {
            if !seen.insert(folder) {
                break;
            }
            path.push(names.get(folder).copied().unwrap_or(folder));
            next = parents.get(folder).copied();
        }
        path.reverse();
        rows.push((key.clone(), path.join(" / "), tag.visible, tag.locked));
    }
    Ok(rows)
}
