// SPDX-License-Identifier: GPL-3.0-or-later
//! Drawing styles and the read-only projection of unstructured host tags.
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

/// Display cells shared by filed nodes and previews of legacy tags.
pub(super) fn cells(node: &Node) -> Result<Vec<String>, String> {
    let Data::Tag(tag) = &node.data else {
        return Err("Expected a document tag".into());
    };
    let number = |v: &Value| match v {
        Value::Float(n) => Some(*n),
        Value::Int(n) => Some(*n as f64),
        _ => None,
    };
    let color = match &tag.color {
        Some(Value::List(rgb) | Value::Tuple(rgb)) if rgb.len() >= 3 => {
            let channel =
                |i: usize| (number(&rgb[i]).unwrap_or(0.0).clamp(0.0, 1.0) * 255.0).round() as u8;
            format!("#{:02x}{:02x}{:02x}", channel(0), channel(1), channel(2))
        }
        _ => String::new(),
    };
    let string = |key, default: &str| -> Result<String, String> {
        Ok(match node.attr(attr_id(key)?) {
            Some(Value::Str(s)) => s.clone(),
            _ => default.into(),
        })
    };
    let weight = node
        .attr(attr_id("lineweight")?)
        .and_then(number)
        .map_or_else(|| "auto".into(), |mm| format!("{mm:.2}"));
    let print = !matches!(node.attr(attr_id("printable")?), Some(Value::Bool(false)));
    Ok(vec![
        color,
        string("linetype", "continuous")?,
        weight,
        u8::from(print).to_string(),
        string("hatch", "")?,
    ])
}

/// Preorder display rows for the existing Project tree. Keep the document untouched:
/// older imports can refer to folders that predate the explicit folder list.
pub(super) fn preview(
    doc: &Doc,
    owner: Key,
    expanded: HashSet<String>,
    filter: &str,
    has_links: bool,
) -> Result<Vec<(Vec<String>, Vec<i64>)>, String> {
    let root = doc.need(owner)?;
    let names = labels(root, "layer_folder_names")?;
    let parents = labels(root, "folder_parents")?;
    let visibility = match root.attr(attr_id("folder_visibility")?) {
        Some(Value::Dict(entries)) => entries
            .iter()
            .filter_map(|(k, v)| match (k, v) {
                (Value::Str(key), Value::Bool(false)) => Some(key.as_str()),
                _ => None,
            })
            .collect::<HashSet<_>>(),
        _ => HashSet::new(),
    };
    let shown = |first: &str| {
        if visibility.contains(first) {
            return false;
        }
        let mut folder = parents.get(first).copied().unwrap_or("");
        let mut seen = HashSet::new();
        while !folder.is_empty() && seen.insert(folder) {
            if visibility.contains(folder) {
                return false;
            }
            folder = parents.get(folder).copied().unwrap_or("");
        }
        true
    };
    let mut folders = explicit_folders(root)?;
    let mut leaves = HashMap::<&str, Vec<(&str, &str, Key)>>::new();
    for &id in &root.tags {
        let Data::Tag(tag) = &doc.need(id)?.data else {
            continue;
        };
        let Some(key) = tag.key.as_deref() else {
            continue;
        };
        if key == "Layer 0" {
            continue;
        }
        let folder = tag.folder.as_deref().unwrap_or("");
        if !folder.is_empty() {
            folders.push(folder);
        }
        leaves
            .entry(folder)
            .or_default()
            .push((key, tag.name.as_deref().unwrap_or(key), id));
    }
    let mut known = HashSet::new();
    folders.retain(|f| known.insert(*f));
    let mut children = HashMap::<&str, Vec<&str>>::new();
    for folder in folders {
        if has_links && folder == "Linked files" {
            continue;
        }
        let mut visited = HashSet::from([folder]);
        let mut parent = parents
            .get(folder)
            .copied()
            .filter(|f| known.contains(f))
            .unwrap_or("");
        let mut cursor = parent;
        while !cursor.is_empty() {
            if !visited.insert(cursor) {
                parent = "";
                break;
            }
            cursor = parents.get(cursor).copied().unwrap_or("");
        }
        children.entry(parent).or_default().push(folder);
    }
    let query = filter.trim().to_lowercase();
    let mut result = vec![];
    // Each folder's closing event drops its row if neither it nor any descendant matched.
    enum Visit<'a> {
        Children(&'a str, usize, bool),
        Folder(&'a str, usize, bool),
        End(usize),
    }
    let mut stack = vec![Visit::Children("", 0, false)];
    while let Some(visit) = stack.pop() {
        match visit {
            Visit::End(start) => {
                if result.len() == start + 1 {
                    result.pop();
                }
            }
            Visit::Folder(f, depth, inherited) => {
                let id = format!("preview-folder:{f}");
                let name = names.get(f).copied().unwrap_or(f);
                let matches = inherited || name.to_lowercase().contains(&query);
                let open = !query.is_empty() || expanded.contains(&id);
                if !matches {
                    stack.push(Visit::End(result.len()));
                }
                result.push((
                    vec![
                        id,
                        name.into(),
                        String::new(),
                        String::new(),
                        "folder".into(),
                    ],
                    vec![
                        depth as i64,
                        0,
                        0,
                        0,
                        6,
                        i64::from(shown(f)),
                        -1,
                        1,
                        i64::from(open),
                        0,
                        0,
                    ],
                ));
                if open {
                    stack.push(Visit::Children(f, depth + 1, matches));
                }
            }
            Visit::Children(f, depth, inherited) => {
                for &(key, name, id) in leaves.get(f).into_iter().flatten() {
                    let node = doc.need(id)?;
                    let Data::Tag(tag) = &node.data else { continue };
                    let visible = tag.visible && shown(f);
                    let matches = query.split_whitespace().all(|word| {
                        let (neg, token) =
                            word.strip_prefix('-').map_or((false, word), |t| (true, t));
                        let hit = match token {
                            "is:hidden" => !visible,
                            "is:locked" => tag.locked,
                            _ if token.contains(':') => false,
                            _ => {
                                inherited
                                    || name.to_lowercase().contains(token)
                                    || "drawing only".contains(token)
                            }
                        };
                        hit != neg
                    });
                    if matches {
                        let mut text = vec![
                            format!("preview-tag:{key}"),
                            name.into(),
                            "Drawing only".into(),
                            String::new(),
                            String::new(),
                        ];
                        text.extend(cells(node)?);
                        result.push((
                            text,
                            vec![
                                depth as i64,
                                0,
                                0,
                                0,
                                6,
                                i64::from(tag.visible && shown(f)),
                                i64::from(tag.locked),
                                0,
                                0,
                                0,
                                0,
                            ],
                        ));
                    }
                }
                for child in children.get(f).into_iter().flatten().rev() {
                    stack.push(Visit::Folder(child, depth, inherited));
                }
            }
        }
    }
    Ok(result)
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
        "hatch" if input.is_empty() => ("hatch", Value::None),
        "hatch"
            if py
                .import("core.hatches")?
                .call_method1("known", (input,))?
                .is_truthy()? =>
        {
            ("hatch", Value::Str(input.into()))
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
    let (prop, value) = drawing_value(py, field, input)?;
    let hatch_kinds = if field == "hatch" {
        let module = py.import("core.hatches")?;
        let kinds = module
            .getattr("HATCH_LABELS")?
            .try_iter()?
            .map(|entry| {
                let key: String = entry?.extract()?;
                let kind: Option<String> = module.call_method1("piece_kind", (&key,))?.extract()?;
                Ok((key, kind))
            })
            .collect::<PyResult<HashMap<_, _>>>()?;
        Some(kinds)
    } else {
        None
    };
    let doc = lock();
    let root = doc.need(owner).map_err(PyValueError::new_err)?;
    let mut needed: HashSet<_> = tags.into_iter().collect();
    let mut edit = TagEdit { changes: vec![] };
    for &id in &root.tags {
        let Data::Tag(tag) = &doc.need(id).map_err(PyValueError::new_err)?.data else {
            continue;
        };
        let Some(key) = &tag.key else { continue };
        if !needed.remove(key) {
            continue;
        }
        if let Some(kinds) = &hatch_kinds {
            let old = doc
                .need(id)
                .map_err(PyValueError::new_err)?
                .get(prop)
                .map_err(PyValueError::new_err)?;
            let previous = match &old {
                FVal::Val(Some(Value::Str(key))) => kinds.get(key).and_then(Option::as_ref),
                _ => None,
            };
            if previous != kinds.get(input).and_then(Option::as_ref) {
                edit.stage(
                    &doc,
                    id,
                    Field::Attr(attr_id("brick").map_err(PyValueError::new_err)?),
                    FVal::Val(None),
                )
                .map_err(PyValueError::new_err)?;
            }
        }
        edit.stage(&doc, id, prop, value.clone())
            .map_err(PyValueError::new_err)?;
    }
    if let Some(key) = needed.iter().next() {
        return Err(PyKeyError::new_err(format!("no tag with key {key:?}")));
    }
    Ok((!edit.changes.is_empty()).then_some(edit))
}
