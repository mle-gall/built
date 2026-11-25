// SPDX-License-Identifier: GPL-3.0-or-later
//! Dimension and label file records. Attachment rebinding needs the live
//! occurrence tree and its freshly allocated vertex keys.
use crate::math::Vec3;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

fn point<'py>(
    py: Python<'py>,
    raw: &Bound<'py, PyDict>,
    name: &str,
    required: bool,
) -> PyResult<Bound<'py, PyAny>> {
    let vec = py.get_type::<Vec3>();
    match raw.get_item(name)? {
        Some(v) => vec.call(
            py.get_type::<PyTuple>()
                .call1((v,))?
                .cast_into::<PyTuple>()?,
            None,
        ),
        None if required => Err(pyo3::exceptions::PyKeyError::new_err(name.to_owned())),
        None => vec.call0(),
    }
}
fn xyz<'py>(item: &Bound<'py, PyAny>, name: &str) -> PyResult<Bound<'py, PyList>> {
    let point = item.getattr(name)?;
    PyList::new(item.py(), point.cast::<Vec3>()?.borrow().coords())
}

/// ``obj`` written out with dict keys sorted at every level (two records
/// that are equal as JSON give the same text); what is not JSON is its str.
fn canonical(obj: &Bound<'_, PyAny>) -> PyResult<String> {
    use pyo3::types::{PyBool, PyFloat, PyInt, PyString};
    let mut out = String::new();
    fn put(obj: &Bound<'_, PyAny>, out: &mut String) -> PyResult<()> {
        if obj.is_none() {
            out.push_str("null");
        } else if let Ok(b) = obj.cast::<PyBool>() {
            out.push_str(if b.is_true() { "true" } else { "false" });
        } else if obj.is_instance_of::<PyInt>() || obj.is_instance_of::<PyFloat>() {
            out.push_str(&obj.repr()?.to_cow()?);
        } else if let Ok(s) = obj.cast::<PyString>() {
            out.push_str(&format!("{:?}", s.to_cow()?));
        } else if let Ok(d) = obj.cast::<PyDict>() {
            let mut items = d.iter().map(|(k, v)| Ok((k.str()?.to_cow()?.into_owned(), v)))
                .collect::<PyResult<Vec<_>>>()?;
            items.sort_by(|a, b| a.0.cmp(&b.0));
            out.push('{');
            for (i, (k, v)) in items.iter().enumerate() {
                if i > 0 { out.push(','); }
                out.push_str(&format!("{k:?}:"));
                put(v, out)?;
            }
            out.push('}');
        } else if obj.is_instance_of::<PyList>() || obj.is_instance_of::<PyTuple>() {
            out.push('[');
            for (i, v) in obj.try_iter()?.enumerate() {
                if i > 0 { out.push(','); }
                put(&v?, out)?;
            }
            out.push(']');
        } else {
            out.push_str(&format!("{:?}", obj.str()?.to_cow()?));
        }
        Ok(())
    }
    put(obj, &mut out)?;
    Ok(out)
}

#[pyfunction]
pub fn igz_load_annotations(
    py: Python<'_>,
    scene: &Bound<'_, PyAny>,
    payload: &Bound<'_, PyDict>,
    labels: bool,
) -> PyResult<()> {
    let table = if labels { "text_labels" } else { "dimensions" };
    let Some(items) = payload.get_item(table)? else {
        return Ok(());
    };
    let destination = scene.getattr(table)?;
    let attachment = py.import("built_core.annotation_anchor")?.getattr("Attachment")?;
    let (module, class) = if labels {
        ("built_core.textlabel", "TextLabel")
    } else {
        ("built_core.dimension", "Dimension")
    };
    let class = py.import(module)?.getattr(class)?;
    // A dimension's record as a canonical key (keys sorted at every
    // level), read in Rust: it was json.dumps per dimension.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for item in items.try_iter()? {
        let raw = item?.cast_into::<PyDict>()?;
        let optional = |name| -> PyResult<Bound<'_, PyAny>> {
            match raw.get_item(name)? {
                Some(v) if v.is_truthy()? => Ok(v),
                _ => Ok(py.None().into_bound(py)),
            }
        };
        let record = if labels {
            let anchor = point(py, &raw, "anchor", false)?;
            let offset = point(py, &raw, "offset", false)?;
            let binding = attachment
                .call_method1("restore", (scene, raw.get_item("attachment")?, &anchor))?;
            let text = raw
                .get_item("text")?
                .unwrap_or_else(|| pyo3::types::PyString::new(py, "").into_any());
            class.call1((anchor, offset, text, optional("layer")?, binding))?
        } else {
            // Documents from before #765 can repeat the identical dimension.
            if !seen.insert(canonical(raw.as_any())?) {
                continue;
            }
            let record = class.call1((
                point(py, &raw, "a", true)?,
                point(py, &raw, "b", true)?,
                point(py, &raw, "offset", true)?,
                optional("layer")?,
                optional("text")?,
                optional("axis")?,
            ))?;
            for (name, anchor) in [("attachment_a", "a"), ("attachment_b", "b")] {
                let binding = attachment.call_method1(
                    "restore",
                    (scene, raw.get_item(name)?, record.getattr(anchor)?),
                )?;
                record.setattr(name, binding)?;
            }
            record.call_method1("bind", (scene,))?;
            record
        };
        destination.call_method1("append", (record,))?;
    }
    Ok(())
}

#[pyfunction]
pub fn igz_archive_annotations<'py>(
    py: Python<'py>,
    items: &Bound<'py, PyAny>,
    labels: bool,
) -> PyResult<Bound<'py, PyList>> {
    let list = PyList::empty(py);
    for item in items.try_iter()? {
        let item = item?;
        let row = PyDict::new(py);
        if labels {
            let attachment = item.getattr("attachment")?;
            let binding = if attachment.is_none() {
                None
            } else {
                Some(attachment.call_method0("archive")?)
            };
            row.set_item("anchor", xyz(&item, "anchor")?)?;
            row.set_item("offset", xyz(&item, "offset")?)?;
            row.set_item("text", item.getattr("text")?)?;
            if let Some(binding) = binding {
                if binding.is_truthy()? {
                    row.set_item("attachment", binding)?;
                }
            }
        } else {
            for name in ["a", "b", "offset"] {
                row.set_item(name, xyz(&item, name)?)?;
            }
            for name in ["attachment_a", "attachment_b"] {
                if let Some(attachment) = item.getattr(name).ok() {
                    if attachment.is_truthy()? {
                        let value = attachment.call_method0("archive")?;
                        if value.is_truthy()? {
                            row.set_item(name, value)?;
                        }
                    }
                }
            }
        }
        let names: &[&str] = if labels {
            &["layer"]
        } else {
            &["axis", "layer", "text"]
        };
        for &name in names {
            if let Ok(v) = item.getattr(name) {
                if v.is_truthy()? {
                    row.set_item(name, v)?;
                }
            }
        }
        list.append(row)?;
    }
    Ok(list)
}

/// The largest N among the automatic "Group N" names (0 without one), as
/// its digits (any size).
#[pyfunction]
pub fn highest_group_number(names: &Bound<'_, PyAny>) -> PyResult<String> {
    let mut best = String::from("0");
    for name in names.try_iter()? {
        let name = name?;
        let Ok(s) = name.cast::<pyo3::types::PyString>() else { continue };
        let s = s.to_cow()?;
        // As ``^Group (\d+)$``: ``$`` also takes a final newline.
        let Some(digits) = s.strip_prefix("Group ").map(|d| d.strip_suffix('\n').unwrap_or(d)) else { continue };
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let d = digits.trim_start_matches('0');
        let d = if d.is_empty() { "0" } else { d };
        if (d.len(), d) > (best.len(), best.as_str()) {
            best = d.to_owned();
        }
    }
    Ok(best)
}
