// SPDX-License-Identifier: GPL-3.0-or-later
//! Dimension and label file records. Attachment rebinding needs the live
//! occurrence tree and its freshly allocated vertex keys.
use crate::math::Vec3;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PySet, PyTuple};

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
    let attachment = py.import("core.annotation_anchor")?.getattr("Attachment")?;
    let (module, class) = if labels {
        ("core.textlabel", "TextLabel")
    } else {
        ("core.dimension", "Dimension")
    };
    let class = py.import(module)?.getattr(class)?;
    let seen = PySet::empty(py)?;
    let json = py.import("json")?;
    let options = PyDict::new(py);
    options.set_item("sort_keys", true)?;
    options.set_item("default", py.get_type::<pyo3::types::PyString>())?;
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
            let key = json.call_method("dumps", (&raw,), Some(&options))?;
            if seen.contains(&key)? {
                continue;
            }
            seen.add(key)?;
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
