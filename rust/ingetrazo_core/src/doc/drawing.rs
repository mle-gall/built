// SPDX-License-Identifier: GPL-3.0-or-later
//! Drawing styles and the read-only projection of unstructured host tags.
use super::{attr_id, lock, value::Value, Data, Doc, FVal, Field, Key, Node, Trash};
use crate::project_tree::text_matches;
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
    let colour = |value: Option<&Value>| match value {
        Some(Value::List(rgb) | Value::Tuple(rgb)) if rgb.len() >= 3 => {
            let channel =
                |i: usize| (number(&rgb[i]).unwrap_or(0.0).clamp(0.0, 1.0) * 255.0).round() as u8;
            format!("#{:02x}{:02x}{:02x}", channel(0), channel(1), channel(2))
        }
        Some(Value::Str(s)) if s == "layer" => s.clone(),
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
    let switch = |key, default: &str| -> Result<String,String> {
        Ok(match node.attr(attr_id(key)?) {
            Some(Value::Bool(on)) => u8::from(*on).to_string(),
            _ => default.into(),
        })
    };
    let measures = |key| -> Result<String,String> {
        Ok(match node.attr(attr_id(key)?) {
            Some(Value::List(v) | Value::Tuple(v)) => v.iter().filter_map(number).map(|n| n.to_string()).collect::<Vec<_>>().join(", "),
            _ => String::new(),
        })
    };
    Ok(vec![
        colour(tag.color.as_ref()),
        string("linetype", "continuous")?,
        weight,
        u8::from(print).to_string(),
        string("hatch", "")?,
        tag.key.clone().unwrap_or_default(),
        node.attr(attr_id("cut_weight")?).and_then(number).map_or_else(|| "auto".into(), |mm| format!("{mm:.2}")),
        u8::from(tag.locked).to_string(),
        node.attr(attr_id("hatch_angle")?).and_then(number).unwrap_or(0.).to_string(),
        node.attr(attr_id("hatch_scale")?).and_then(number).unwrap_or(1.).to_string(),
        String::new(), // imported line families are supplied by project_sidebar
        colour(node.attr(attr_id("hatch_ground")?)),
        colour(node.attr(attr_id("hatch_color")?)),
        switch("hatch_on_paper", "")?, measures("hatch_offset")?,
        switch("hatch_cross", "0")?, string("joints", "")?, switch("mortar", "0")?,
        measures("brick")?, node.attr(attr_id("opacity")?).and_then(number).unwrap_or(1.).to_string(),
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
    used: &HashSet<String>,
    held: &HashSet<String>,
    selected: &HashSet<String>,
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

        let folder = tag.folder.as_deref().unwrap_or("");
        if !folder.is_empty() {
            folders.push(folder);
        }
        leaves
            .entry(folder)
            .or_default()
            .push((key, if key == "Layer 0" { "Untagged" } else { tag.name.as_deref().unwrap_or(key) }, id));
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
    let (inverted, query) = query.strip_prefix('!').filter(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
        .map_or((false, query.as_str()), |rest| (true, rest.trim_start()));
    let filtering = inverted || !query.is_empty();
    let print_attr = attr_id("printable")?;
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
                let matches = inherited
                    || (!inverted && !query.contains(':') && text_matches(&name.to_lowercase(), query));
                let open = filtering || expanded.contains(&id);
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
                            "is:visible" => visible,
                            "is:unprinted" => matches!(node.attr(print_attr), Some(Value::Bool(false))),
                            "is:locked" => tag.locked,
                            "is:selected" => selected.contains(key),
                            "is:used" => used.contains(key),
                            "is:unused" => !used.contains(key) && !held.contains(key),
                            _ if token.contains(':') => false,
                            _ => {
                                inherited
                                    || text_matches(&name.to_lowercase(), token)
                                    || text_matches("tag", token)
                            }
                        };
                        hit != neg
                    });
                    if matches != inverted {
                        let mut text = vec![
                            format!("preview-tag:{key}"),
                            name.into(),
                            String::new(),
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
    if field == "lock" && matches!(input, "0" | "1") { return Ok((Field::Locked, FVal::Bool(input == "1"))); }
    let (field, value) = match field {
        "color" | "hatch_ground" | "hatch_color" if input.is_empty() => (field, Value::None),
        "hatch_ground" | "hatch_color" if input == "layer" => (field, Value::Str("layer".into())),
        "color" | "hatch_ground" | "hatch_color" => {
            let hex = input
                .strip_prefix('#')
                .filter(|s| s.len() == 6 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                .ok_or_else(|| PyValueError::new_err("expected #RRGGBB"))?;
            let rgb =
                u32::from_str_radix(hex, 16).map_err(|e| PyValueError::new_err(e.to_string()))?;
            (
                field,
                Value::Tuple(
                    [16, 8, 0]
                        .map(|shift| Value::Float(((rgb >> shift) & 255u32) as f64 / 255.0))
                        .into(),
                ),
            )
        }
        "weight" | "cut_weight" if input == "auto" => (if field=="weight" {"lineweight"} else {"cut_weight"}, Value::None),
        "weight" | "cut_weight" => {
            let mm = input
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite() && *n > 0.0)
                .ok_or_else(|| PyValueError::new_err("expected a positive pen width in mm"))?;
            (if field=="weight" {"lineweight"} else {"cut_weight"}, Value::Float(mm))
        }
        "linetype"
            if py
                .import("core.layers")?
                .call_method1("known_linetype", (input,))?
                .is_truthy()? =>
        {
            ("linetype", Value::Str(input.to_string()))
        }
        "hatch_on_paper" | "brick" | "hatch_offset" | "joints" if input.is_empty() => (field, Value::None),
        "hatch_on_paper" | "mortar" | "hatch_cross" if matches!(input, "0" | "1") => (field, Value::Bool(input == "1")),
        "joints" if matches!(input, "single" | "double" | "none") => (field, Value::Str(input.into())),
        "opacity" => {
            let n = input.parse::<f64>().ok().filter(|n| (0.1..=1.0).contains(n))
                .ok_or_else(|| PyValueError::new_err("Opacity must be between 0.1 and 1"))?;
            (field, Value::Float(n))
        }
        "brick" | "hatch_offset" => {
            let nums: Vec<f64> = input.split(',').map(|token| token.trim().parse::<f64>()
                .ok().filter(|v| v.is_finite()).ok_or_else(|| PyValueError::new_err("Expected millimetres separated by commas")))
                .collect::<PyResult<_>>()?;
            if field == "brick" {
                let clean = py.import("core.hatches")?.call_method1("clean_brick", (nums,))?;
                if clean.is_none() { return Err(PyValueError::new_err("Invalid piece dimensions or joint widths")); }
                (field, Value::from_py(&clean)?)
            } else {
                if nums.len() != 2 || nums.iter().any(|v| v.abs() > 10000.) { return Err(PyValueError::new_err("Start X and Y must be within 10000 mm")); }
                let value=py.import("core.layers")?.call_method1("clean_offset",(nums,))?;
                (field, Value::from_py(&value)?)
            }
        }
        "hatch_angle" | "hatch_scale" => {
            let n = input.parse::<f64>().ok().filter(|v| v.is_finite())
                .ok_or_else(|| PyValueError::new_err("Expected a finite hatch angle or scale"))?;
            if field == "hatch_angle" {
                ("hatch_angle", Value::Float(n.rem_euclid(360.)))
            } else if (0.05..=20.).contains(&n) {
                ("hatch_scale", Value::Float(n))
            } else {
                return Err(PyValueError::new_err("Hatch scale must be between 0.05 and 20"));
            }
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
    if field == "style" {
        let patch: std::collections::BTreeMap<String,String> = serde_json::from_str(input).map_err(|e| PyValueError::new_err(e.to_string()))?;
        let mut combined = TagEdit {changes: Vec::new()};
        let explicit_piece = patch.contains_key("brick");
        for (name, v) in patch {
            if name == "style" { return Err(PyValueError::new_err("Nested style patch")); }
            if let Some(mut edit) = prepare(py,owner,tags.clone(),&name,&v)? {
                if name == "hatch" && explicit_piece {
                    let brick = Field::Attr(attr_id("brick").map_err(PyValueError::new_err)?);
                    edit.changes.retain(|(_, f, _, _)| *f != brick);
                }
                combined.changes.extend(edit.changes);
            }
        }
        return Ok((!combined.changes.is_empty()).then_some(combined));
    }
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
