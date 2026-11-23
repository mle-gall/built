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

use pyo3::{
    exceptions::{PyKeyError, PyValueError},
    prelude::*,
};

/// Prepared before History starts, so invalid input cannot partly edit a
/// multi-tag selection. Each handle is a view of a native document tag.
#[pyclass(frozen)]
pub(super) struct DrawingEdit {
    field: &'static str,
    value: Value,
    tags: Vec<(Py<PyAny>, Value)>,
}

#[pymethods]
impl DrawingEdit {
    fn apply(&self, py: Python<'_>, forward: bool) -> PyResult<()> {
        for (tag, before) in &self.tags {
            let value = if forward { &self.value } else { before };
            tag.bind(py).setattr(self.field, value.to_py(py)?)?;
        }
        Ok(())
    }
}

pub(super) fn drawing_edit(
    py: Python<'_>,
    root: &Bound<'_, PyAny>,
    tags: Vec<String>,
    field: &str,
    input: &str,
) -> PyResult<DrawingEdit> {
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
    let mut seen = HashSet::new();
    let mut targets = Vec::new();
    for key in tags {
        if !seen.insert(key.clone()) {
            continue;
        }
        let tag = root.call_method1("_tag_by_key", (&key,))?;
        if tag.is_none() {
            return Err(PyKeyError::new_err(format!("no tag with key {key:?}")));
        }
        let old = Value::from_py(&tag.getattr(field)?)?;
        if old != value {
            targets.push((tag.unbind(), old));
        }
    }
    Ok(DrawingEdit {
        field,
        value,
        tags: targets,
    })
}
