// SPDX-License-Identifier: GPL-3.0-or-later
//! The tag (layer) edits of `core.layers`: the folder tree's edits (add,
//! move, rename, remove, file tags in a folder), labels and paths, the
//! display state (visibility, lock, isolate, the active tag, scenes hiding
//! a tag), the templates an office carries from one project to the next,
//! and the document's own line types brought in.
//!
//! Every step reads and writes the scene's root tables (`layer_folders`,
//! `folder_parents`, `layer_folder_names`, `layer_folder_uids`,
//! `folder_visibility`, `active_layer`) and its tags, in the Python's order,
//! so a journalled edit records the same writes. What stays Python is
//! called back: the uid pool (`core.layers.new_uid`), the tag class, and the
//! cleaners of line type entries, colours and bricks.
//! `tests/test_tag_edits_parity.py`.

use std::collections::HashSet;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyString, PyTuple};

use super::tag_folders::{tag_folder_parent, tag_subfolders};

pub const DEFAULT_LAYER: &str = "Layer 0";
pub const TEMPLATE_KEY: &str = "ingetrazo_layers";
const CUSTOM_LINETYPE: &str = "lin:";

type Obj<'py> = Bound<'py, PyAny>;

fn attr<'py>(o: &Obj<'py>, name: &str) -> PyResult<Option<Obj<'py>>> {
    Ok(o.getattr_opt(name)?.filter(|v| !v.is_none()))
}

/// `getattr(o, name, None) or {}` as a plain dict copy.
fn dict_of<'py>(o: &Obj<'py>, name: &str) -> PyResult<Bound<'py, PyDict>> {
    let py = o.py();
    match attr(o, name)? {
        Some(v) if v.is_truthy()? => Ok(py.get_type::<PyDict>().call1((v,))?.cast_into()?),
        _ => Ok(PyDict::new(py)),
    }
}

fn layers<'py>(scene: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    match attr(scene, "layers")? {
        Some(l) => l.try_iter()?.collect(),
        None => Ok(Vec::new()),
    }
}

fn s(v: &Obj<'_>) -> Option<String> {
    v.cast::<PyString>().ok().and_then(|s| s.to_str().ok().map(str::to_string))
}

fn truthy_str(v: &Obj<'_>) -> Option<String> {
    s(v).filter(|x| !x.is_empty())
}

fn key_of<'py>(ly: &Obj<'py>) -> PyResult<Obj<'py>> {
    ly.getattr("key")
}

fn layer_by<'py>(scene: &Obj<'py>, name: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    if !name.is_truthy()? {
        return Ok(None);
    }
    Ok(Some(scene.call_method1("layer", (name,))?).filter(|v| !v.is_none()))
}

/// `core.layers.new_uid`, looked up on every call as the Python did.
fn new_uid<'py>(py: Python<'py>) -> PyResult<Obj<'py>> {
    py.import("core.layers")?.call_method0("new_uid")
}

fn layers_module<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyModule>> {
    py.import("core.layers")
}

fn deepcopy<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    v.py().import("copy")?.call_method1("deepcopy", (v,))
}

fn get<'py>(d: &Obj<'py>, key: &str) -> PyResult<Option<Obj<'py>>> {
    Ok(Some(d.call_method1("get", (key,))?).filter(|v| !v.is_none()))
}

fn get_or_empty<'py>(d: &Obj<'py>, key: &str) -> PyResult<Obj<'py>> {
    match get(d, key)? {
        Some(v) if v.is_truthy()? => Ok(v),
        _ => Ok(PyDict::new(d.py()).into_any()),
    }
}

// ---- Folders -----------------------------------------------------------------

/// `folders_of(scene)`: the folders the document lists (empty ones
/// included), then any a tag names that the list misses; every one given a
/// uid.
#[pyfunction]
pub fn folders_of(scene: &Obj<'_>) -> PyResult<Vec<String>> {
    let py = scene.py();
    let mut out: Vec<String> = Vec::new();
    if let Some(list) = attr(scene, "layer_folders")? {
        for f in list.try_iter()? {
            if let Some(f) = truthy_str(&f?) {
                out.push(f);
            }
        }
    }
    let mut seen: HashSet<String> = out.iter().cloned().collect();
    for ly in layers(scene)? {
        if let Some(f) = truthy_str(&ly.getattr("folder")?) {
            if seen.insert(f.clone()) {
                out.push(f);
            }
        }
    }
    let uids = scene.getattr_opt("layer_folder_uids")?.unwrap_or_else(|| PyDict::new(py).into_any());
    let fresh = PyDict::new(py);
    for f in &out {
        let u = uids.call_method1("get", (f,))?;
        fresh.set_item(f, if u.is_truthy()? { u } else { new_uid(py)? })?;
    }
    scene.setattr("layer_folder_uids", fresh)?;
    Ok(out)
}

fn parents_table<'py>(scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    Ok(dict_of(scene, "folder_parents")?.into_any())
}

/// `folder_parent(scene, name, known=None)`.
#[pyfunction]
#[pyo3(signature = (scene, name, *, known=None))]
pub fn folder_parent(scene: &Obj<'_>, name: &str, known: Option<&Obj<'_>>) -> PyResult<Option<String>> {
    let known = match known.filter(|k| !k.is_none()) {
        Some(k) => k.try_iter()?.filter_map(|f| f.ok().and_then(|f| s(&f))).collect(),
        None => folders_of(scene)?,
    };
    tag_folder_parent(&parents_table(scene)?, known, name)
}

/// The folders directly in `name` (`None`: the top), or at any depth.
fn subfolders(scene: &Obj<'_>, name: Option<&str>, deep: bool) -> PyResult<Vec<String>> {
    let folders = folders_of(scene)?;
    tag_subfolders(folders, &parents_table(scene)?, name.map(str::to_string), deep)
}

/// `folder_visible(scene, key, ancestors=True)`: a folder switched off, or
/// one of the folders it sits in; loops tolerated.
#[pyfunction]
#[pyo3(signature = (scene, key, *, ancestors=true))]
pub fn folder_visible(scene: &Obj<'_>, key: &Obj<'_>, ancestors: bool) -> PyResult<bool> {
    let py = scene.py();
    let flags = scene.getattr_opt("folder_visibility")?.unwrap_or_else(|| PyDict::new(py).into_any());
    let parents = scene.getattr_opt("folder_parents")?.unwrap_or_else(|| PyDict::new(py).into_any());
    let seen = pyo3::types::PySet::empty(py)?;
    let mut key = key.clone();
    while key.is_truthy()? && !seen.contains(&key)? {
        seen.add(&key)?;
        if !flags.call_method1("get", (&key, true))?.is_truthy()? {
            return Ok(false);
        }
        key = if ancestors { parents.call_method1("get", (&key,))? } else { py.None().into_bound(py) };
    }
    Ok(true)
}

#[pyfunction]
pub fn effective_visible(scene: &Obj<'_>, layer: &Obj<'_>) -> PyResult<bool> {
    Ok(layer.getattr("visible")?.is_truthy()? && folder_visible(scene, &layer.getattr("folder")?, true)?)
}

#[pyfunction]
pub fn folder_label<'py>(scene: &Obj<'py>, key: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = scene.py();
    if !key.is_truthy()? {
        return Ok(PyString::new(py, "").into_any());
    }
    let label = dict_of(scene, "layer_folder_names")?.call_method1("get", (key,))?;
    Ok(if label.is_truthy()? { label } else { key.clone() })
}

#[pyfunction]
pub fn layer_label<'py>(scene: &Obj<'py>, r#ref: &Obj<'py>) -> PyResult<Obj<'py>> {
    match layer_by(scene, r#ref)? {
        Some(ly) => ly.getattr("name"),
        None if r#ref.is_truthy()? => Ok(r#ref.clone()),
        None => Ok(PyString::new(scene.py(), DEFAULT_LAYER).into_any()),
    }
}

/// `path_label(scene, ref, sep="/")`: the tag's label with its folders' in
/// front.
#[pyfunction]
#[pyo3(signature = (scene, r#ref, sep="/"))]
pub fn path_label<'py>(scene: &Obj<'py>, r#ref: &Obj<'py>, sep: &str) -> PyResult<Obj<'py>> {
    let py = scene.py();
    let Some(ly) = layer_by(scene, r#ref)? else {
        return Ok(if r#ref.is_truthy()? { r#ref.clone() } else { PyString::new(py, DEFAULT_LAYER).into_any() });
    };
    let mut parts: Vec<String> = vec![ly.getattr("name")?.str()?.to_string()];
    let mut f: Option<String> =
        if key_of(&ly)?.eq(DEFAULT_LAYER)? { None } else { truthy_str(&ly.getattr("folder")?) };
    let mut seen: HashSet<String> = HashSet::new();
    while let Some(folder) = f.take() {
        if !seen.insert(folder.clone()) {
            break;
        }
        parts.push(folder_label(scene, &PyString::new(py, &folder).into_any())?.str()?.to_string());
        f = folder_parent(scene, &folder, None)?;
    }
    parts.reverse();
    Ok(PyString::new(py, &parts.join(sep)).into_any())
}

#[pyfunction]
#[pyo3(signature = (scene, sep="/"))]
pub fn export_labels<'py>(scene: &Obj<'py>, sep: &str) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(scene.py());
    for ly in layers(scene)? {
        let k = key_of(&ly)?;
        out.set_item(&k, path_label(scene, &k, sep)?)?;
    }
    Ok(out)
}

/// `base 1`, `base 2`… — the first one not in `taken`.
#[pyfunction]
pub fn unique_name(taken: &Obj<'_>, base: &str) -> PyResult<String> {
    let taken: HashSet<String> = taken.try_iter()?.filter_map(|t| t.ok().and_then(|t| s(&t))).collect();
    let mut n = 1;
    while taken.contains(&format!("{base} {n}")) {
        n += 1;
    }
    Ok(format!("{base} {n}"))
}

#[pyfunction]
pub fn layers_in_folder<'py>(scene: &Obj<'py>, name: &str) -> PyResult<Vec<Obj<'py>>> {
    let mut folders: HashSet<String> = subfolders(scene, Some(name), true)?.into_iter().collect();
    folders.insert(name.to_string());
    let mut out = Vec::new();
    for ly in layers(scene)? {
        if s(&ly.getattr("folder")?).is_some_and(|f| folders.contains(&f)) {
            out.push(key_of(&ly)?);
        }
    }
    Ok(out)
}

fn put_folders(scene: &Obj<'_>, folders: Vec<String>) -> PyResult<()> {
    scene.setattr("layer_folders", PyList::new(scene.py(), folders)?)
}

#[pyfunction]
#[pyo3(signature = (scene, base, parent=None))]
pub fn add_folder(scene: &Obj<'_>, base: &str, parent: Option<&Obj<'_>>) -> PyResult<String> {
    let py = scene.py();
    let name = unique_name(PyList::new(py, folders_of(scene)?)?.as_any(), base)?;
    let mut list = folders_of(scene)?;
    list.push(name.clone());
    put_folders(scene, list)?;
    if let Some(p) = parent.and_then(truthy_str) {
        if folders_of(scene)?.contains(&p) {
            let parents = dict_of(scene, "folder_parents")?;
            parents.set_item(&name, p)?;
            scene.setattr("folder_parents", parents)?;
        }
    }
    folders_of(scene)?;
    Ok(name)
}

/// `move_folder(scene, name, parent)`: refused into itself or into a
/// folder inside it.
#[pyfunction]
pub fn move_folder(scene: &Obj<'_>, name: &Obj<'_>, parent: &Obj<'_>) -> PyResult<bool> {
    let Some(name) = s(name).filter(|n| folders_of(scene).is_ok_and(|f| f.contains(n))) else {
        return Ok(false);
    };
    if !parent.is_none() {
        let p = s(parent);
        let refused = match &p {
            None => true,
            Some(p) => p == &name || !folders_of(scene)?.contains(p) || subfolders(scene, Some(&name), true)?.contains(p),
        };
        if refused {
            return Ok(false);
        }
    }
    let parents = dict_of(scene, "folder_parents")?;
    if parent.is_truthy()? {
        parents.set_item(&name, parent)?;
    } else if parents.contains(&name)? {
        parents.del_item(&name)?;
    }
    scene.setattr("folder_parents", parents)?;
    Ok(true)
}

#[pyfunction]
pub fn rename_folder(scene: &Obj<'_>, old: &Obj<'_>, new: &Obj<'_>) -> PyResult<bool> {
    let folders = folders_of(scene)?;
    let Some(old) = s(old).filter(|o| folders.contains(o)) else { return Ok(false) };
    let Some(new) = s(new).filter(|n| !n.is_empty() && !n.trim().is_empty()) else { return Ok(false) };
    let names = dict_of(scene, "layer_folder_names")?;
    if new == old {
        if names.contains(&old)? {
            names.del_item(&old)?;
        }
    } else {
        names.set_item(&old, new)?;
    }
    scene.setattr("layer_folder_names", names)?;
    Ok(true)
}

/// `remove_folder(scene, name)`: what it held moves up into the folder it
/// sat in.
#[pyfunction]
pub fn remove_folder(scene: &Obj<'_>, name: &str) -> PyResult<()> {
    let py = scene.py();
    let up = folder_parent(scene, name, None)?;
    for ly in layers(scene)? {
        if s(&ly.getattr("folder")?).as_deref() == Some(name) {
            ly.setattr("folder", up.clone())?;
        }
    }
    let parents = dict_of(scene, "folder_parents")?;
    for (f, p) in parents.items().extract::<Vec<(Obj, Obj)>>()? {
        if p.eq(name)? {
            match &up {
                Some(u) if !u.is_empty() => parents.set_item(&f, u)?,
                _ => parents.del_item(&f)?,
            }
        }
    }
    if parents.contains(name)? {
        parents.del_item(name)?;
    }
    scene.setattr("folder_parents", parents)?;
    let rest: Vec<String> = folders_of(scene)?.into_iter().filter(|f| f != name).collect();
    put_folders(scene, rest)?;
    scene.getattr("layer_folder_uids")?.call_method1("pop", (name, py.None()))?;
    scene.getattr("folder_visibility")?.call_method1("pop", (name, py.None()))?;
    if let Some(labels) = attr(scene, "layer_folder_names")? {
        if labels.is_instance_of::<PyDict>() {
            labels.call_method1("pop", (name, py.None()))?;
        }
    }
    Ok(())
}

/// `move_to_folder(scene, names, folder)`: the default tag stays out.
#[pyfunction]
pub fn move_to_folder(scene: &Obj<'_>, names: &Obj<'_>, folder: &Obj<'_>) -> PyResult<()> {
    if let Some(f) = truthy_str(folder) {
        let mut list = folders_of(scene)?;
        if !list.contains(&f) {
            list.push(f);
            put_folders(scene, list)?;
        }
    }
    let value = if folder.is_truthy()? { folder.clone() } else { scene.py().None().into_bound(scene.py()) };
    for name in names.try_iter()? {
        if let Some(ly) = layer_by(scene, &name?)? {
            if !key_of(&ly)?.eq(DEFAULT_LAYER)? {
                ly.setattr("folder", &value)?;
            }
        }
    }
    folders_of(scene)?;
    Ok(())
}

// ---- Display state -------------------------------------------------------------

/// `active_layer(scene)`: where new objects go.
#[pyfunction]
pub fn active_layer<'py>(scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = scene.py();
    let default = PyString::new(py, DEFAULT_LAYER).into_any();
    let Some(name) = attr(scene, "active_layer")?.filter(|n| n.is_truthy().unwrap_or(false)) else {
        return Ok(default);
    };
    if name.eq(DEFAULT_LAYER)? {
        return Ok(default);
    }
    match layer_by(scene, &name)? {
        Some(ly) if effective_visible(scene, &ly)? && !ly.getattr("locked")?.is_truthy()? => key_of(&ly),
        _ => Ok(default),
    }
}

fn release_active(scene: &Obj<'_>) -> PyResult<bool> {
    let name = attr(scene, "active_layer")?;
    if name.is_some_and(|n| n.is_truthy().unwrap_or(false)) && active_layer(scene)?.eq(DEFAULT_LAYER)? {
        scene.setattr("active_layer", scene.py().None())?;
        return Ok(true);
    }
    Ok(false)
}

fn set_flag(scene: &Obj<'_>, names: &Obj<'_>, flag: &str, on: bool) -> PyResult<bool> {
    for name in names.try_iter()? {
        if let Some(ly) = layer_by(scene, &name?)? {
            ly.setattr(flag, on)?;
        }
    }
    release_active(scene)
}

#[pyfunction]
pub fn set_visible(scene: &Obj<'_>, names: &Obj<'_>, on: &Obj<'_>) -> PyResult<bool> {
    set_flag(scene, names, "visible", on.is_truthy()?)
}

#[pyfunction]
pub fn set_locked(scene: &Obj<'_>, names: &Obj<'_>, on: &Obj<'_>) -> PyResult<bool> {
    set_flag(scene, names, "locked", on.is_truthy()?)
}

/// `isolate(scene, names)`: only those shown, and the default tag.
#[pyfunction]
pub fn isolate(scene: &Obj<'_>, names: &Obj<'_>) -> PyResult<bool> {
    let keep = pyo3::types::PySet::empty(scene.py())?;
    for n in names.try_iter()? {
        if let Some(ly) = layer_by(scene, &n?)? {
            keep.add(key_of(&ly)?)?;
        }
    }
    keep.add(DEFAULT_LAYER)?;
    for ly in layers(scene)? {
        ly.setattr("visible", keep.contains(key_of(&ly)?)?)?;
    }
    release_active(scene)
}

#[pyfunction]
pub fn is_isolated(scene: &Obj<'_>, name: &Obj<'_>) -> PyResult<bool> {
    let Some(ly) = layer_by(scene, name)? else { return Ok(false) };
    let key = key_of(&ly)?;
    if !ly.getattr("visible")?.is_truthy()? || key.eq(DEFAULT_LAYER)? {
        return Ok(false);
    }
    let mut others = false;
    for o in layers(scene)? {
        let k = key_of(&o)?;
        if k.eq(&key)? || k.eq(DEFAULT_LAYER)? {
            continue;
        }
        others = true;
        if o.getattr("visible")?.is_truthy()? {
            return Ok(false);
        }
    }
    Ok(others)
}

#[pyfunction]
pub fn set_active(scene: &Obj<'_>, name: &Obj<'_>) -> PyResult<()> {
    match layer_by(scene, name)? {
        Some(ly) if !key_of(&ly)?.eq(DEFAULT_LAYER)? => {
            ly.setattr("visible", true)?;
            ly.setattr("locked", false)?;
            scene.setattr("active_layer", key_of(&ly)?)
        }
        _ => scene.setattr("active_layer", scene.py().None()),
    }
}

#[pyfunction]
pub fn layer_state<'py>(scene: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let py = scene.py();
    let out = PyDict::new(py);
    out.set_item("folders", py.get_type::<PyDict>().call1((scene.getattr("folder_visibility")?,))?)?;
    let tags = PyDict::new(py);
    for ly in layers(scene)? {
        tags.set_item(key_of(&ly)?, (ly.getattr("visible")?, ly.getattr("locked")?))?;
    }
    out.set_item("layers", tags)?;
    out.set_item("active", scene.getattr_opt("active_layer")?.unwrap_or_else(|| py.None().into_bound(py)))?;
    Ok(out)
}

#[pyfunction]
pub fn apply_layer_state(scene: &Obj<'_>, state: &Obj<'_>) -> PyResult<()> {
    let py = scene.py();
    let saved = state.call_method1("get", ("layers", PyDict::new(py)))?;
    for ly in layers(scene)? {
        let v = saved.call_method1("get", (key_of(&ly)?,))?;
        if !v.is_none() {
            let (vis, lock): (Obj, Obj) = v.extract()?;
            ly.setattr("visible", vis)?;
            ly.setattr("locked", lock)?;
        }
    }
    let folders = state.call_method1("get", ("folders", PyDict::new(py)))?;
    scene.setattr("folder_visibility", py.get_type::<PyDict>().call1((folders,))?)?;
    let active = state.call_method1("get", ("active",))?;
    let keep = active.is_truthy()? && layer_by(scene, &active)?.is_some();
    scene.setattr("active_layer", if keep { active } else { py.None().into_bound(py) })?;
    release_active(scene)?;
    Ok(())
}

/// `hide_in_scenes(scene, names, hidden=True)`: how many scenes changed.
#[pyfunction]
#[pyo3(signature = (scene, names, hidden=true))]
pub fn hide_in_scenes(scene: &Obj<'_>, names: &Obj<'_>, hidden: bool) -> PyResult<usize> {
    let py = scene.py();
    let mut keys: Vec<Obj> = Vec::new();
    for n in names.try_iter()? {
        if let Some(ly) = layer_by(scene, &n?)? {
            keys.push(key_of(&ly)?);
        }
    }
    let keys = PyList::new(py, keys)?;
    let mut changed = 0;
    if let Some(views) = attr(scene, "saved_views")? {
        for view in views.try_iter()? {
            let view = view?;
            let before = match attr(&view, "hidden_layers")? {
                Some(h) => PyList::new(py, h.try_iter()?.collect::<PyResult<Vec<_>>>()?)?,
                None => PyList::empty(py),
            };
            let after = PyList::empty(py);
            if hidden {
                for b in before.iter() {
                    after.append(b)?;
                }
                for n in keys.iter() {
                    if !before.contains(&n)? {
                        after.append(n)?;
                    }
                }
            } else {
                for b in before.iter() {
                    if !keys.contains(&b)? {
                        after.append(b)?;
                    }
                }
            }
            if !after.eq(&before)? {
                view.setattr("hidden_layers", after)?;
                changed += 1;
            }
        }
    }
    Ok(changed)
}

// ---- Templates -----------------------------------------------------------------

fn archive<'py>(ly: &Obj<'py>) -> PyResult<Obj<'py>> {
    Ok(crate::file_styles::igz_archive_layer(ly)?.into_any())
}

fn parents_of_folders<'py>(scene: &Obj<'py>, folders: &[String]) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(scene.py());
    for f in folders {
        if folder_parent(scene, f, None)?.is_some_and(|p| !p.is_empty()) {
            out.set_item(f, folder_parent(scene, f, None)?)?;
        }
    }
    Ok(out)
}

/// `template_of(scene)`: the document's tags as a template.
#[pyfunction]
pub fn template_of<'py>(scene: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let py = scene.py();
    let out = PyDict::new(py);
    out.set_item(TEMPLATE_KEY, 1)?;
    out.set_item("folders", folders_of(scene)?)?;
    let tags = layers(scene)?;
    out.set_item("layers", PyList::new(py, tags.iter().map(archive).collect::<PyResult<Vec<_>>>()?)?)?;
    out.set_item("folder_uids", py.get_type::<PyDict>().call1((scene.getattr("layer_folder_uids")?,))?)?;
    let labels = dict_of(scene, "layer_folder_names")?;
    if labels.len() > 0 {
        out.set_item("folder_names", labels)?;
    }
    let parents = parents_of_folders(scene, &folders_of(scene)?)?;
    if parents.len() > 0 {
        out.set_item("folder_parents", parents)?;
    }
    let mut used = HashSet::new();
    for ly in &tags {
        if let Some(lt) = s(&ly.getattr("linetype")?) {
            if let Some(n) = lt.strip_prefix(CUSTOM_LINETYPE) {
                used.insert(n.to_string());
            }
        }
    }
    let own = PyDict::new(py);
    for (n, e) in dict_of(scene, "line_types")?.iter() {
        if s(&n).is_some_and(|n| used.contains(&n)) {
            own.set_item(n, e)?;
        }
    }
    if own.len() > 0 {
        out.set_item("line_types", own)?;
    }
    if let Some(colors) = attr(scene, "linetype_colors")?.filter(|c| c.is_truthy().unwrap_or(false)) {
        let d = PyDict::new(py);
        for item in colors.call_method0("items")?.try_iter()? {
            let (k, v): (Obj, Obj) = item?.extract()?;
            d.set_item(k, py.get_type::<PyList>().call1((v,))?)?;
        }
        out.set_item("linetype_colors", d)?;
    }
    if let Some(bricks) = attr(scene, "bricks")?.filter(|c| c.is_truthy().unwrap_or(false)) {
        let round = py.import("builtins")?.getattr("round")?;
        let d = PyDict::new(py);
        for item in bricks.call_method0("items")?.try_iter()? {
            let (k, size): (Obj, Obj) = item?.extract()?;
            let l = PyList::empty(py);
            for v in size.try_iter()? {
                l.append(round.call1((v?.extract::<f64>()?, 2))?)?;
            }
            d.set_item(k, l)?;
        }
        out.set_item("bricks", d)?;
    }
    Ok(out)
}

/// `matched_template(scene, data)`: identities resolved to local names; a
/// name collision keeps the local uid.
#[pyfunction]
pub fn matched_template<'py>(scene: &Obj<'py>, data: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = scene.py();
    let data = deepcopy(data)?;
    folders_of(scene)?;
    let names = PyDict::new(py);
    let incoming = get_or_empty(&data, "folder_uids")?;
    let local = scene.getattr("layer_folder_uids")?;
    if let Some(folders) = get(&data, "folders")?.filter(|f| f.is_truthy().unwrap_or(false)) {
        for name in folders.try_iter()? {
            let name = name?;
            if truthy_str(&name).is_none() {
                continue;
            }
            let uid = incoming.call_method1("get", (&name,))?;
            let mut found = name.clone();
            if uid.is_truthy()? {
                for item in local.call_method0("items")?.try_iter()? {
                    let (n, u): (Obj, Obj) = item?.extract()?;
                    if u.eq(&uid)? {
                        found = n;
                        break;
                    }
                }
            }
            names.set_item(&name, found)?;
        }
    }
    let folders = PyList::empty(py);
    for (_, v) in names.iter() {
        folders.append(v)?;
    }
    data.set_item("folders", folders)?;
    let uids = PyDict::new(py);
    for item in incoming.call_method0("items")?.try_iter()? {
        let (n, u): (Obj, Obj) = item?.extract()?;
        if names.contains(&n)? {
            uids.set_item(names.get_item(&n)?.unwrap(), u)?;
        }
    }
    data.set_item("folder_uids", uids)?;
    let parents = PyDict::new(py);
    for item in get_or_empty(&data, "folder_parents")?.call_method0("items")?.try_iter()? {
        let (n, p): (Obj, Obj) = item?.extract()?;
        let rn = names.call_method1("get", (&n, &n))?;
        let rp = names.call_method1("get", (&p, &p))?;
        parents.set_item(rn, rp)?;
    }
    data.set_item("folder_parents", parents)?;
    let tags = layers(scene)?;
    for raw in data.call_method1("get", ("layers", PyTuple::empty(py)))?.try_iter()? {
        let raw = raw?;
        if !raw.is_instance_of::<PyDict>() {
            continue;
        }
        let uid = raw.call_method1("get", ("uid",))?;
        if uid.is_truthy()? {
            for ly in &tags {
                if ly.getattr("uid")?.eq(&uid)? {
                    raw.set_item("name", ly.getattr("name")?)?;
                    raw.set_item("key", key_of(ly)?)?;
                    break;
                }
            }
        }
        let folder = raw.call_method1("get", ("folder",))?;
        if folder.is_truthy()? {
            raw.set_item("folder", names.call_method1("get", (&folder, &folder))?)?;
        }
    }
    Ok(data)
}

fn raw_key<'py>(raw: &Obj<'py>) -> PyResult<Obj<'py>> {
    let k = raw.call_method1("get", ("key",))?;
    if k.is_truthy()? { Ok(k) } else { raw.call_method1("get", ("name",)) }
}

#[pyfunction]
pub fn template_name_map<'py>(scene: &Obj<'py>, data: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let py = scene.py();
    let resolved = matched_template(scene, data)?;
    let out = PyDict::new(py);
    let a = data.call_method1("get", ("layers", PyTuple::empty(py)))?;
    let b = resolved.call_method1("get", ("layers", PyTuple::empty(py)))?;
    for (x, y) in a.try_iter()?.zip(b.try_iter()?) {
        let (x, y) = (x?, y?);
        if x.is_instance_of::<PyDict>() && y.is_instance_of::<PyDict>() && raw_key(&x)?.is_truthy()? {
            out.set_item(raw_key(&x)?, raw_key(&y)?)?;
        }
    }
    Ok(out)
}

fn folder_identities(scene: &Obj<'_>, data: &Obj<'_>, previous: &HashSet<String>) -> PyResult<()> {
    folders_of(scene)?;
    let local = scene.getattr("layer_folder_uids")?;
    for item in get_or_empty(data, "folder_uids")?.call_method0("items")?.try_iter()? {
        let (name, uid): (Obj, Obj) = item?.extract()?;
        let fresh = s(&name).is_none_or(|n| !previous.contains(&n));
        if fresh && local.contains(&name)? && uid.is_truthy()? {
            local.set_item(&name, uid)?;
        }
    }
    Ok(())
}

fn folder_labels(scene: &Obj<'_>, data: &Obj<'_>, previous: &HashSet<String>) -> PyResult<()> {
    let incoming = get_or_empty(data, "folder_names")?;
    if !incoming.is_instance_of::<PyDict>() {
        return Ok(());
    }
    let names = dict_of(scene, "layer_folder_names")?;
    for item in incoming.call_method0("items")?.try_iter()? {
        let (key, label): (Obj, Obj) = item?.extract()?;
        let fresh = s(&key).is_some_and(|k| !previous.contains(&k) && folders_of(scene).is_ok_and(|f| f.contains(&k)));
        if fresh && truthy_str(&label).is_some() {
            names.set_item(key, label)?;
        }
    }
    scene.setattr("layer_folder_names", names)
}

const STYLE_COPY: [&str; 16] = ["color", "linetype", "lineweight", "cut_weight", "hatch", "hatch_ground",
                                "hatch_color", "hatch_angle", "hatch_scale", "hatch_offset", "hatch_on_paper",
                                "brick", "hatch_cross", "joints", "mortar", "opacity"];

fn tag_from<'py>(raw: &Obj<'py>) -> PyResult<Obj<'py>> {
    let class = layers_module(raw.py())?.getattr("Layer")?;
    crate::file_styles::igz_load_layer(raw.cast::<PyDict>()?, &class)
}

fn named_tag(raw: &Obj<'_>) -> PyResult<bool> {
    Ok(raw.is_instance_of::<PyDict>() && truthy_str(&raw.call_method1("get", ("name",))?).is_some())
}

/// `apply_template(scene, data)`: `(added, updated)`.
#[pyfunction]
pub fn apply_template(scene: &Obj<'_>, data: &Obj<'_>) -> PyResult<(usize, usize)> {
    let py = scene.py();
    let ok = data.is_instance_of::<PyDict>()
        && data.contains(TEMPLATE_KEY)?
        && data.call_method1("get", ("layers",))?.is_instance_of::<PyList>();
    if !ok {
        return Err(PyValueError::new_err("not a layer template"));
    }
    let data = matched_template(scene, data)?;
    let previous: HashSet<String> = folders_of(scene)?.into_iter().collect();
    if let Some(folders) = get(&data, "folders")?.filter(|f| f.is_truthy().unwrap_or(false)) {
        for f in folders.try_iter()? {
            if let Some(f) = truthy_str(&f?) {
                let mut list = folders_of(scene)?;
                if !list.contains(&f) {
                    list.push(f);
                    put_folders(scene, list)?;
                }
            }
        }
    }
    if let Some(raw) = get(&data, "folder_parents")?.filter(|p| p.is_instance_of::<PyDict>()) {
        for item in raw.call_method0("items")?.try_iter()? {
            let (f, p): (Obj, Obj) = item?.extract()?;
            if s(&f).is_some() && s(&p).is_some() {
                move_folder(scene, &f, &p)?;
            }
        }
    }
    folder_identities(scene, &data, &previous)?;
    folder_labels(scene, &data, &previous)?;
    let module = layers_module(py)?;
    module.call_method1("load_line_types", (scene, data.call_method1("get", ("line_types",))?))?;
    module.call_method1("load_linetype_colors", (scene, data.call_method1("get", ("linetype_colors",))?))?;
    module.call_method1("load_bricks", (scene, data.call_method1("get", ("bricks",))?))?;
    let (mut added, mut updated) = (0, 0);
    for raw in data.get_item("layers")?.try_iter()? {
        let raw = raw?;
        if !named_tag(&raw)? {
            continue;
        }
        let new = tag_from(&raw)?;
        let key = key_of(&new)?;
        let Some(ly) = Some(scene.call_method1("layer", (&key,))?).filter(|l| !l.is_none()) else {
            if key.eq(DEFAULT_LAYER)? {
                continue;
            }
            scene.getattr("layers")?.call_method1("append", (new,))?;
            added += 1;
            continue;
        };
        for f in STYLE_COPY {
            ly.setattr(f, new.getattr(f)?)?;
        }
        if !key_of(&ly)?.eq(DEFAULT_LAYER)? {
            ly.setattr("folder", new.getattr("folder")?)?;
        }
        updated += 1;
    }
    release_active(scene)?;
    Ok((added, updated))
}

/// `template_for(scene, names)`: narrowed to those tags and their folders.
#[pyfunction]
pub fn template_for<'py>(scene: &Obj<'py>, names: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let py = scene.py();
    let wanted = pyo3::types::PySet::empty(py)?;
    for n in names.try_iter()? {
        let n = n?;
        if n.is_truthy()? && !n.eq(DEFAULT_LAYER)? {
            if let Some(ly) = layer_by(scene, &n)? {
                wanted.add(key_of(&ly)?)?;
            }
        }
    }
    let mut tags = Vec::new();
    for ly in layers(scene)? {
        if wanted.contains(key_of(&ly)?)? {
            tags.push(ly);
        }
    }
    let mut folders: Vec<String> = Vec::new();
    for ly in &tags {
        let mut f = truthy_str(&ly.getattr("folder")?);
        while let Some(x) = f.take() {
            if folders.contains(&x) {
                break;
            }
            folders.push(x.clone());
            f = folder_parent(scene, &x, None)?;
        }
    }
    let mine: Vec<String> = folders_of(scene)?.into_iter().filter(|f| folders.contains(f)).collect();
    let out = PyDict::new(py);
    out.set_item(TEMPLATE_KEY, 1)?;
    out.set_item("folders", mine.clone())?;
    out.set_item("layers", PyList::new(py, tags.iter().map(archive).collect::<PyResult<Vec<_>>>()?)?)?;
    let uids = scene.getattr("layer_folder_uids")?;
    let fu = PyDict::new(py);
    for f in &mine {
        fu.set_item(f, uids.get_item(f)?)?;
    }
    out.set_item("folder_uids", fu)?;
    let labels = dict_of(scene, "layer_folder_names")?;
    let named = PyDict::new(py);
    for f in &mine {
        if let Some(l) = labels.get_item(f)? {
            named.set_item(f, l)?;
        }
    }
    if named.len() > 0 {
        out.set_item("folder_names", named)?;
    }
    let parents = parents_of_folders(scene, &mine)?;
    if parents.len() > 0 {
        out.set_item("folder_parents", parents)?;
    }
    Ok(out)
}

/// `add_missing(scene, data)`: only the tags and folders the document
/// lacks, visible and unlocked: `(tag keys added, folders added)`.
#[pyfunction]
pub fn add_missing<'py>(scene: &Obj<'py>, data: &Obj<'py>) -> PyResult<(Vec<Obj<'py>>, Vec<String>)> {
    let ok = data.is_instance_of::<PyDict>() && data.call_method1("get", ("layers",))?.is_instance_of::<PyList>();
    if !ok {
        return Ok((Vec::new(), Vec::new()));
    }
    let data = matched_template(scene, data)?;
    let have: HashSet<String> = folders_of(scene)?.into_iter().collect();
    let mut new_folders = Vec::new();
    if let Some(folders) = get(&data, "folders")?.filter(|f| f.is_truthy().unwrap_or(false)) {
        for f in folders.try_iter()? {
            if let Some(f) = truthy_str(&f?).filter(|f| !have.contains(f)) {
                new_folders.push(f);
            }
        }
    }
    if !new_folders.is_empty() {
        let mut list = folders_of(scene)?;
        list.extend(new_folders.iter().cloned());
        put_folders(scene, list)?;
    }
    if let Some(raw) = get(&data, "folder_parents")?.filter(|p| p.is_instance_of::<PyDict>()) {
        for f in &new_folders {
            let p = raw.call_method1("get", (f,))?;
            if truthy_str(&p).is_some() {
                move_folder(scene, &PyString::new(scene.py(), f).into_any(), &p)?;
            }
        }
    }
    folder_identities(scene, &data, &have)?;
    let mut added = Vec::new();
    for raw in data.get_item("layers")?.try_iter()? {
        let raw = raw?;
        if !named_tag(&raw)? {
            continue;
        }
        let key = raw_key(&raw)?;
        if key.eq(DEFAULT_LAYER)? || !scene.call_method1("layer_by_key", (&key,))?.is_none() {
            continue;
        }
        let new = tag_from(&raw)?;
        new.setattr("visible", true)?;
        new.setattr("locked", false)?;
        scene.getattr("layers")?.call_method1("append", (&new,))?;
        added.push(key_of(&new)?);
    }
    folder_labels(scene, &data, &have)?;
    Ok((added, new_folders))
}

/// `remove_added(scene, layers, folders)`: undo `add_missing`.
#[pyfunction]
pub fn remove_added(scene: &Obj<'_>, layers_added: &Obj<'_>, folders: &Obj<'_>) -> PyResult<()> {
    let py = scene.py();
    let tags = layers(scene)?;
    let gone = pyo3::types::PySet::empty(py)?;
    for token in layers_added.try_iter()? {
        let token = token?;
        let mut hit = token.clone();
        for ly in &tags {
            if ly.getattr("uid")?.eq(&token)? {
                hit = key_of(ly)?;
                break;
            }
        }
        gone.add(hit)?;
    }
    let mut keep = Vec::new();
    for ly in &tags {
        if !gone.contains(key_of(ly)?)? {
            keep.push(ly.clone());
        }
    }
    scene.getattr("layers")?.set_item(pyo3::types::PySlice::full(py), PyList::new(py, keep)?)?;
    folders_of(scene)?;
    let uids = scene.getattr("layer_folder_uids")?;
    let mut drop: HashSet<String> = HashSet::new();
    for token in folders.try_iter()? {
        let token = token?;
        let mut hit = s(&token);
        for item in uids.call_method0("items")?.try_iter()? {
            let (n, u): (Obj, Obj) = item?.extract()?;
            if u.eq(&token)? {
                hit = s(&n);
                break;
            }
        }
        if let Some(h) = hit {
            drop.insert(h);
        }
    }
    if !drop.is_empty() {
        let rest: Vec<String> = folders_of(scene)?.into_iter().filter(|f| !drop.contains(f)).collect();
        put_folders(scene, rest)?;
        if let Some(parents) = attr(scene, "folder_parents")?.filter(|p| p.is_instance_of::<PyDict>()) {
            for f in &drop {
                parents.call_method1("pop", (f, py.None()))?;
            }
        }
        folders_of(scene)?;
    }
    release_active(scene)?;
    Ok(())
}

/// `load_line_types(scene, raw)`: line types stored as `{name: {"label",
/// "pattern"}}` brought into the document; how many.
#[pyfunction]
pub fn load_line_types(scene: &Obj<'_>, raw: &Obj<'_>) -> PyResult<usize> {
    let py = scene.py();
    if !raw.is_instance_of::<PyDict>() {
        return Ok(0);
    }
    let module = layers_module(py)?;
    let own = dict_of(scene, "line_types")?;
    let mut n = 0;
    let items: Vec<(Obj, Obj)> = raw.call_method0("items")?.try_iter()?.take(500)
        .map(|i| i?.extract()).collect::<PyResult<_>>()?;
    for (name, entry) in items {
        let Some(name) = truthy_str(&name) else { continue };
        if !entry.is_instance_of::<PyDict>() {
            continue;
        }
        let pattern = module.call_method1("clean_pattern", (entry.call_method1("get", ("pattern",))?,))?;
        if pattern.is_none() {
            continue;
        }
        let short: String = name.chars().take(64).collect();
        let label = entry.call_method1("get", ("label",))?;
        let label: Obj = match s(&label) {
            Some(l) => PyString::new(py, &l.chars().take(120).collect::<String>()).into_any(),
            None => PyString::new(py, &short).into_any(),
        };
        let e = PyDict::new(py);
        e.set_item("label", label)?;
        e.set_item("pattern", &pattern)?;
        let text = module.call_method1("clean_line_text", (&entry, &pattern))?;
        if text.is_truthy()? {
            e.call_method1("update", (text,))?;
        }
        let pieces = module.call_method1("clean_pattern", (entry.call_method1("get", ("pieces",))?,))?;
        if !pieces.is_none() {
            e.set_item("pieces", pieces)?;
        }
        let colour = module.call_method1("clean_line_colour", (entry.call_method1("get", ("color",))?,))?;
        if !colour.is_none() {
            e.set_item("color", colour)?;
        }
        own.set_item(&short, e)?;
        n += 1;
    }
    scene.setattr("line_types", own)?;
    Ok(n)
}

/// `release_active(scene)`: an active tag hidden or locked gives way to
/// the default one (True when it did).
#[pyfunction]
fn tag_release_active(scene: &Obj<'_>) -> PyResult<bool> {
    release_active(scene)
}

/// `subfolders(scene, name, deep=False)` over the scene.
#[pyfunction]
#[pyo3(signature = (scene, name, deep=false))]
fn tag_scene_subfolders(scene: &Obj<'_>, name: Option<&str>, deep: bool) -> PyResult<Vec<String>> {
    subfolders(scene, name, deep)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "tag_edits")?;
    for f in [
        wrap_pyfunction!(folders_of, &sub)?,
        wrap_pyfunction!(folder_parent, &sub)?,
        wrap_pyfunction!(folder_visible, &sub)?,
        wrap_pyfunction!(effective_visible, &sub)?,
        wrap_pyfunction!(folder_label, &sub)?,
        wrap_pyfunction!(layer_label, &sub)?,
        wrap_pyfunction!(path_label, &sub)?,
        wrap_pyfunction!(export_labels, &sub)?,
        wrap_pyfunction!(unique_name, &sub)?,
        wrap_pyfunction!(layers_in_folder, &sub)?,
        wrap_pyfunction!(add_folder, &sub)?,
        wrap_pyfunction!(move_folder, &sub)?,
        wrap_pyfunction!(rename_folder, &sub)?,
        wrap_pyfunction!(remove_folder, &sub)?,
        wrap_pyfunction!(move_to_folder, &sub)?,
        wrap_pyfunction!(active_layer, &sub)?,
        wrap_pyfunction!(set_visible, &sub)?,
        wrap_pyfunction!(set_locked, &sub)?,
        wrap_pyfunction!(isolate, &sub)?,
        wrap_pyfunction!(is_isolated, &sub)?,
        wrap_pyfunction!(set_active, &sub)?,
        wrap_pyfunction!(layer_state, &sub)?,
        wrap_pyfunction!(apply_layer_state, &sub)?,
        wrap_pyfunction!(hide_in_scenes, &sub)?,
        wrap_pyfunction!(template_of, &sub)?,
        wrap_pyfunction!(matched_template, &sub)?,
        wrap_pyfunction!(template_name_map, &sub)?,
        wrap_pyfunction!(apply_template, &sub)?,
        wrap_pyfunction!(template_for, &sub)?,
        wrap_pyfunction!(add_missing, &sub)?,
        wrap_pyfunction!(remove_added, &sub)?,
        wrap_pyfunction!(load_line_types, &sub)?,
        wrap_pyfunction!(tag_release_active, &sub)?,
        wrap_pyfunction!(tag_scene_subfolders, &sub)?,
    ] {
        sub.add_function(f)?;
    }
    m.add_submodule(&sub)?;
    Ok(())
}
