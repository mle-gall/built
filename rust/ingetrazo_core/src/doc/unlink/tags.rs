// SPDX-License-Identifier: GPL-3.0-or-later
//! A pair of Unlink snapshots owns only the differences it made to the tree.
use super::Obj;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySlice},
};
use std::collections::HashSet;
const MAPS: [&str; 4] = [
    "folder_parents",
    "folder_visibility",
    "layer_folder_names",
    "layer_folder_uids",
];

fn dict<'py>(py: Python<'py>, value: Option<Obj<'py>>) -> PyResult<Bound<'py, PyDict>> {
    match value {
        Some(v) if v.is_truthy()? => Ok(py.get_type::<PyDict>().call1((v,))?.cast_into()?),
        _ => Ok(PyDict::new(py)),
    }
}
pub(crate) fn sync(
    now: &Obj<'_>,
    old: &Bound<'_, PyDict>,
    new: &Bound<'_, PyDict>,
    deep: bool,
) -> PyResult<()> {
    let py = now.py();
    let keys = old
        .call_method0("keys")?
        .call_method1("__or__", (new.call_method0("keys")?,))?;
    for k in keys.try_iter()? {
        let k = k?;
        let a = old.get_item(&k)?;
        let b = new.get_item(&k)?;
        if let (Some(a), Some(b)) = (&a, &b) {
            if a.is(b) || a.eq(b)? {
                continue;
            }
        }
        if let Some(b) = b {
            let value = if deep {
                py.import("copy")?.call_method1("deepcopy", (b,))?
            } else {
                b
            };
            now.set_item(k, value)?;
        } else {
            now.call_method1("pop", (k, py.None()))?;
        }
    }
    Ok(())
}

#[pyclass(frozen)]
pub(crate) struct TagState {
    layers: Py<PyList>,
    values: Py<PyDict>,
    folders: Vec<String>,
    maps: Py<PyDict>,
    registry: Py<PyDict>,
    #[pyo3(get)]
    cache: Py<PyDict>,
    materials: Py<PyDict>,
    active: Py<PyAny>,
}
impl TagState {
    pub(crate) fn capture<'py>(scene: &Obj<'py>) -> PyResult<Bound<'py, Self>> {
        let py = scene.py();
        let layers = py
            .get_type::<PyList>()
            .call1((scene.getattr("layers")?,))?
            .cast_into::<PyList>()?;
        let read = py.import("core.layers")?.getattr("layer_values")?;
        let values = PyDict::new(py);
        for layer in &layers {
            values.set_item(layer.as_ptr() as usize, read.call1((layer,))?)?;
        }
        let maps = PyDict::new(py);
        for name in MAPS {
            maps.set_item(name, dict(py, scene.getattr_opt(name)?)?)?;
        }
        Bound::new(
            py,
            Self {
                layers: layers.unbind(),
                values: values.unbind(),
                maps: maps.unbind(),
                folders: crate::model::tag_edits::folders_of(scene)?,
                registry: py
                    .import("copy")?
                    .call_method1("deepcopy", (crate::model::links::registry(scene)?,))?
                    .cast_into::<PyDict>()?
                    .unbind(),
                cache: dict(
                    py,
                    scene
                        .getattr("__dict__")?
                        .cast::<PyDict>()?
                        .get_item("_link_tag_tables")?,
                )?
                .unbind(),
                materials: dict(py, Some(scene.getattr("materials")?))?.unbind(),
                active: scene
                    .getattr_opt("active_layer")?
                    .map_or_else(|| py.None(), Bound::unbind),
            },
        )
    }
}
#[pymethods]
impl TagState {
    pub(crate) fn go(&self, scene: &Obj<'_>, to: &TagState) -> PyResult<()> {
        let py = scene.py();
        let ids = |layers: &Bound<'_, PyList>| {
            layers
                .iter()
                .map(|v| v.as_ptr() as usize)
                .collect::<HashSet<_>>()
        };
        let old = ids(self.layers.bind(py));
        let new = ids(to.layers.bind(py));
        let layers = scene.getattr("layers")?;
        let keep = PyList::empty(py);
        let mut have = HashSet::new();
        for layer in layers.try_iter()? {
            let layer = layer?;
            let id = layer.as_ptr() as usize;
            if new.contains(&id) || !old.contains(&id) {
                keep.append(layer)?;
                have.insert(id);
            }
        }
        layers.set_item(PySlice::full(py), keep)?;
        for (i, layer) in to.layers.bind(py).iter().enumerate() {
            let id = layer.as_ptr() as usize;
            if !have.contains(&id) {
                layers.call_method1("insert", (i.min(layers.len()?), &layer))?;
            }
        }
        for layer in to.layers.bind(py) {
            let id = layer.as_ptr() as usize;
            let old = self
                .values
                .bind(py)
                .get_item(id)?
                .map(|a| a.cast_into::<PyDict>())
                .transpose()?;
            let new = to
                .values
                .bind(py)
                .as_any()
                .get_item(id)?
                .cast_into::<PyDict>()?;
            for (key, v) in &new {
                let old = old
                    .as_ref()
                    .map(|a| a.get_item(&key))
                    .transpose()?
                    .flatten();
                if let Some(a) = old {
                    if a.eq(&v)? {
                        continue;
                    }
                }
                layer.setattr(key.cast_into::<pyo3::types::PyString>()?, v)?;
            }
        }
        let names = scene.getattr("layer_folders")?;
        let mut folders = if names.is_truthy()? {
            names.extract::<Vec<String>>()?
        } else {
            Vec::new()
        };
        folders.retain(|n| to.folders.contains(n) || !self.folders.contains(n));
        for (i, name) in to.folders.iter().enumerate() {
            if !folders.contains(name) {
                folders.insert(i.min(folders.len()), name.clone());
            }
        }
        let maps = PyDict::new(py);
        for name in MAPS {
            let now = dict(py, scene.getattr_opt(name)?)?;
            sync(
                now.as_any(),
                self.maps
                    .bind(py)
                    .as_any()
                    .get_item(name)?
                    .cast::<PyDict>()?,
                to.maps.bind(py).as_any().get_item(name)?.cast::<PyDict>()?,
                false,
            )?;
            maps.set_item(name, now)?;
        }
        for name in MAPS {
            scene.setattr(name, maps.as_any().get_item(name)?)?;
        }
        scene.setattr("layer_folders", folders)?;
        sync(
            &crate::model::links::registry(scene)?,
            self.registry.bind(py),
            to.registry.bind(py),
            true,
        )?;
        let cache = scene
            .getattr("__dict__")?
            .call_method1("setdefault", ("_link_tag_tables", PyDict::new(py)))?;
        sync(&cache, self.cache.bind(py), to.cache.bind(py), false)?;
        sync(
            &scene.getattr("materials")?,
            self.materials.bind(py),
            to.materials.bind(py),
            false,
        )?;
        if self.active.bind(py).ne(to.active.bind(py))? {
            scene.setattr("active_layer", to.active.bind(py))?;
        }
        Ok(())
    }
    fn __traverse__(&self, visit: pyo3::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        visit.call(&self.layers)?;
        visit.call(&self.values)?;
        visit.call(&self.maps)?;
        visit.call(&self.cache)?;
        visit.call(&self.registry)?;
        visit.call(&self.materials)?;
        visit.call(&self.active)
    }
}
