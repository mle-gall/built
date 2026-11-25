// SPDX-License-Identifier: GPL-3.0-or-later
//! Map texture and PBR references between cache paths and archive members.
use pyo3::exceptions::{PyKeyError, PyOSError, PyRecursionError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList, PyString};

fn slot<'p>(
    packing: bool,
    resolve: &mut impl FnMut(Bound<'p, PyAny>) -> PyResult<Bound<'p, PyAny>>,
    parent: &Bound<'p, PyDict>,
    key: &Bound<'p, PyAny>,
    image: &Bound<'p, PyDict>,
) -> PyResult<()> {
    let image = if packing {
        let copy = image.copy()?;
        parent.set_item(key, &copy)?;
        copy
    } else {
        image.clone()
    };
    let pairs = if packing {
        [("path", "embed"), ("base", "base_embed")]
    } else {
        [("base_embed", "base"), ("embed", "path")]
    };
    for (from, to) in pairs {
        let Some(value) = image.get_item(from)? else {
            continue;
        };
        if !packing {
            image.del_item(from)?;
            if let Some(existing) = image.get_item(to)? {
                if existing.is_truthy()? {
                    continue;
                }
            }
        }
        if value.is_truthy()? {
            let mapped = resolve(value)?;
            if mapped.is_truthy()? {
                if packing {
                    image.del_item(from)?;
                }
                image.set_item(to, mapped)?;
            }
        }
    }
    Ok(())
}

fn walk<'p>(
    root: &Bound<'p, PyAny>,
    mut visit: impl FnMut(&Bound<'p, PyDict>, &Bound<'p, PyAny>, &Bound<'p, PyDict>) -> PyResult<()>,
) -> PyResult<()> {
    enum Task<'py> {
        Enter(Bound<'py, PyAny>),
        Leave(usize),
        Slot(Bound<'py, PyDict>, Bound<'py, PyAny>, Bound<'py, PyDict>),
    }
    let mut pending = vec![Task::Enter(root.clone())];
    let mut ancestors = std::collections::HashSet::new();
    while let Some(task) = pending.pop() {
        let node = match task {
            Task::Leave(id) => {
                ancestors.remove(&id);
                continue;
            }
            Task::Slot(parent, key, image) => {
                visit(&parent, &key, &image)?;
                continue;
            }
            Task::Enter(node) => node,
        };
        let id = node.as_ptr() as usize;
        if !ancestors.insert(id) {
            return Err(PyRecursionError::new_err("cyclic texture references"));
        }
        pending.push(Task::Leave(id));
        let start = pending.len();
        if let Ok(dict) = node.cast::<PyDict>() {
            for (key, value) in dict {
                let text = key.cast::<PyString>().ok().and_then(|s| s.to_str().ok());
                if let Ok(image) = value.cast::<PyDict>() {
                    if text == Some("texture") {
                        pending.push(Task::Slot(dict.clone(), key, image.clone()));
                        continue;
                    }
                    if text == Some("pbr") {
                        let recipe = image.copy()?;
                        dict.set_item(key, &recipe)?;
                        for (slot, value) in &recipe {
                            if slot.cast::<PyString>()?.to_str()?.ends_with("_map") {
                                if let Ok(image) = value.cast::<PyDict>() {
                                    pending.push(Task::Slot(recipe.clone(), slot, image.clone()));
                                }
                            }
                        }
                        continue;
                    }
                }
                if !matches!(
                    text,
                    Some("vertices" | "holes" | "a" | "b" | "edges" | "arrays")
                ) && (value.is_instance_of::<PyDict>() || value.is_instance_of::<PyList>())
                {
                    pending.push(Task::Enter(value));
                }
            }
        } else if let Ok(list) = node.cast::<PyList>() {
            if !list.is_empty() {
                let first = list.get_item(0)?;
                if first.is_instance_of::<PyDict>() || first.is_instance_of::<PyList>() {
                    pending.extend(list.iter().map(Task::Enter));
                }
            }
        }
        // Preserve the insertion order of content-addressed archive members.
        pending[start..].reverse();
    }
    Ok(())
}

#[pyfunction]
pub fn igz_texture_slots<'p>(node: &Bound<'p, PyAny>) -> PyResult<Bound<'p, PyAny>> {
    let slots = PyList::empty(node.py());
    walk(node, |parent, key, _| slots.append((parent, key)))?;
    slots.call_method0("__iter__")
}
#[pyfunction]
pub fn igz_texture_entries<'p>(node: &Bound<'p, PyAny>) -> PyResult<Bound<'p, PyAny>> {
    let entries = PyList::empty(node.py());
    walk(node, |_, _, img| entries.append(img))?;
    entries.call_method0("__iter__")
}
#[pyfunction]
#[pyo3(signature=(payload,*,blobs=None,resolved=None))]
pub fn igz_pack_textures<'p>(
    payload: &Bound<'p, PyAny>,
    blobs: Option<Bound<'p, PyDict>>,
    resolved: Option<Bound<'p, PyDict>>,
) -> PyResult<(Bound<'p, PyDict>, usize)> {
    let py = payload.py();
    let blobs = blobs.unwrap_or_else(|| PyDict::new(py));
    let resolved = resolved.unwrap_or_else(|| PyDict::new(py));
    let texture = py.import("core.texture")?;
    let mut missing = 0;
    let mut member = |source: Bound<'p, PyAny>| -> PyResult<Bound<'p, PyAny>> {
        if let Some(known) = resolved.get_item(&source)?.filter(|v| !v.is_none()) {
            return Ok(known);
        }
        let path = source.extract::<std::path::PathBuf>()?;
        let bytes = if texture
            .call_method1("is_image_file", (&source,))?
            .is_truthy()?
        {
            py.detach(|| std::fs::read(&path)).ok()
        } else {
            None
        };
        if let Some(data) = bytes {
            let data = PyBytes::new(py, &data);
            let hash = py
                .import("hashlib")?
                .call_method1("sha1", (&data,))?
                .call_method0("hexdigest")?
                .extract::<String>()?;
            let name = texture
                .call_method1(
                    "texture_file_name",
                    (path.file_name().unwrap_or_default().to_string_lossy(),),
                )?
                .extract::<String>()?;
            let member = PyString::new(py, &format!("textures/{}-{name}", &hash[..16]));
            resolved.set_item(&source, &member)?;
            blobs.set_item(&member, data)?;
            Ok(member.into_any())
        } else {
            missing += 1;
            resolved.set_item(source, "")?;
            Ok(PyString::new(py, "").into_any())
        }
    };
    let api = py.import("ingetrazo_core.formats.igz")?;
    if api.call_method0("native_io")?.is_none() {
        for pair in api.call_method1("_texture_slots", (payload,))?.try_iter()? {
            let (parent, key): (Bound<'p, PyDict>, Bound<'p, PyAny>) = pair?.extract()?;
            let image = parent
                .get_item(&key)?
                .ok_or_else(|| PyKeyError::new_err("missing texture slot"))?
                .cast_into::<PyDict>()?;
            slot(true, &mut member, &parent, &key, &image)?;
        }
    } else {
        walk(payload, |p, k, i| slot(true, &mut member, p, k, i))?;
    }
    Ok((blobs, missing))
}
#[pyfunction]
pub fn igz_unpack_textures<'p>(
    payload: &Bound<'p, PyAny>,
    archive: &Bound<'p, PyAny>,
) -> PyResult<()> {
    let py = payload.py();
    let cached = PyDict::new(py);
    let texture = py.import("core.texture")?;
    let mut restore = |member: Bound<'p, PyAny>| -> PyResult<Bound<'p, PyAny>> {
        if let Some(out) = cached.get_item(&member)? {
            return Ok(out);
        }
        let data = match archive.call_method1("read", (&member,)) {
            Err(e) if e.is_instance_of::<PyKeyError>(py) => return Ok(py.None().into_bound(py)),
            v => v?,
        };
        let name = member.extract::<String>()?;
        let name = name.rsplit('/').next().unwrap_or("");
        let path = match texture.call_method1("cache_image", (data, name, "embedded")) {
            Ok(p) => p.str()?.into_any(),
            Err(e) if e.is_instance_of::<PyOSError>(py) => py.None().into_bound(py),
            Err(e) => return Err(e),
        };
        cached.set_item(member, &path)?;
        Ok(path)
    };
    walk(payload, |p, k, i| slot(false, &mut restore, p, k, i))
}
