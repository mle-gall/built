// SPDX-License-Identifier: GPL-3.0-or-later
//! Transfer staged occurrences into an existing drawing as one transaction.
//! A staged root releases ownership before publication. Its handles retain
//! the definition stores, including those shared by several occurrences.
use super::apply::{material_names, skp_apply_metadata};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use std::collections::{HashMap, HashSet};

#[pyclass(dict, module = "ingetrazo_core")]
pub struct SkpInsertion {
    groups: Py<PyList>,
    views: Py<PyList>,
    dimensions: Py<PyList>,
    texts: Py<PyList>,
    layers: Py<PyDict>,
    materials: Py<PyDict>,
    back: Py<PyAny>,
    placements: Vec<(String, Py<PyAny>)>,
    meshes: Vec<Py<PyAny>>,
    #[pyo3(get)]
    empty: bool,
}

impl SkpInsertion {
    pub(crate) fn stage(
        scene: &Bound<'_, PyAny>,
        payload: &Bound<'_, PyDict>,
        empty: bool,
    ) -> PyResult<Self> {
        let py = scene.py();
        let select = |fields: &[&str]| -> PyResult<Py<PyDict>> {
            let metadata = PyDict::new(py);
            for &field in fields {
                if let Some(value) = payload.get_item(field)? {
                    metadata.set_item(field, value)?;
                }
            }
            Ok(metadata.unbind())
        };
        let mut seen = HashSet::new();
        let mut meshes = Vec::new();
        let mut placements = Vec::new();
        let all = scene.call_method0("groups_by_uid")?.cast_into::<PyDict>()?;
        for (uid, group) in all {
            let mesh = group.getattr("mesh")?;
            if seen.insert(mesh.as_ptr()) {
                meshes.push(mesh.unbind());
            }
            placements.push((uid.extract()?, group.unbind()));
        }
        let detach = |field| -> PyResult<Py<PyList>> {
            let members = scene.getattr(field)?;
            let items = py
                .get_type::<PyList>()
                .call1((&members,))?
                .cast_into::<PyList>()?;
            members.call_method0("clear")?;
            Ok(items.unbind())
        };
        Ok(Self {
            groups: detach("groups")?,
            views: detach("saved_views")?,
            dimensions: detach("dimensions")?,
            texts: detach("text_labels")?,
            materials: select(&["materials"])?,
            layers: select(&["layers", "layer_folders"])?,
            back: scene.getattr("back_face_color")?.unbind(),
            placements,
            meshes,
            empty,
        })
    }
}

fn remap_row(row: &Bound<'_, PyAny>, names: &HashMap<String, String>) -> PyResult<()> {
    if let Some(old) = row
        .call_method1("get", ("mat",))?
        .extract::<Option<String>>()?
    {
        if let Some(new) = names.get(&old) {
            row.set_item("mat", new)?;
        }
    }
    Ok(())
}

#[pymethods]
impl SkpInsertion {
    #[classattr]
    const keeps_rows: bool = false;
    #[classattr]
    const keeps_groups: bool = true;
    #[classattr]
    const keeps_paint: bool = false;
    #[classattr]
    const doc_undo: bool = true;
    #[classattr]
    const _journal_geometry: bool = false;
    #[pyo3(name = "do")]
    fn attach(&self, scene: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = scene.py();
        let remap = material_names(py, scene, self.materials.bind(py))?;
        if !remap.is_empty() {
            for mesh in &self.meshes {
                let mesh = mesh.bind(py);
                if let Some(table) = mesh.getattr_opt("_attr_table")? {
                    for row in table.try_iter()? {
                        remap_row(&row?, &remap)?;
                    }
                    for row in mesh
                        .getattr("_attr_own")?
                        .call_method0("values")?
                        .try_iter()?
                    {
                        remap_row(&row?, &remap)?;
                    }
                } else {
                    for face in mesh.getattr("faces")?.try_iter()? {
                        remap_row(&face?.getattr("attrs")?, &remap)?;
                    }
                }
            }
        }
        skp_apply_metadata(scene, self.layers.bind(py))?;
        let existing = scene.call_method0("groups_by_uid")?.cast_into::<PyDict>()?;
        let fresh = py.import("core.group")?.getattr("new_uid")?;
        if !existing.is_empty() {
            let mut used = self
                .placements
                .iter()
                .map(|(uid, _)| uid.clone())
                .collect::<HashSet<_>>();
            for (uid, group) in &self.placements {
                if existing.contains(uid)? {
                    loop {
                        let new = fresh.call0()?.extract::<String>()?;
                        if !existing.contains(&new)? && used.insert(new.clone()) {
                            group.bind(py).setattr("uid", new)?;
                            break;
                        }
                    }
                }
            }
        }
        scene
            .getattr("groups")?
            .call_method1("extend", (self.groups.bind(py),))?;
        let dest = scene.getattr("saved_views")?;
        let mut names = HashSet::new();
        for v in dest.try_iter()? {
            names.insert(v?.getattr("name")?.extract::<String>()?);
        }
        for view in self.views.bind(py) {
            if names.insert(view.getattr("name")?.extract()?) {
                dest.call_method1("append", (view,))?;
            }
        }
        scene
            .getattr("dimensions")?
            .call_method1("extend", (self.dimensions.bind(py),))?;
        scene
            .getattr("text_labels")?
            .call_method1("extend", (self.texts.bind(py),))?;
        if scene.getattr("back_face_color")?.is_none() {
            scene.setattr("back_face_color", self.back.bind(py))?;
        }
        for dim in scene.getattr("dimensions")?.try_iter()? {
            let dim = dim?;
            if dim.hasattr("bind")? {
                dim.call_method1("bind", (scene,))?;
            }
        }
        bump(scene)
    }
    fn bind_transaction(slf: &Bound<'_, Self>, txn: &Bound<'_, PyAny>) -> PyResult<()> {
        slf.setattr("_doc_txn", txn)
    }
    fn doc_after(&self, scene: &Bound<'_, PyAny>, phase: &str) -> PyResult<()> {
        if phase == "undo" {
            let selection = scene.getattr("selection")?;
            if selection.is_truthy()? {
                for group in self.groups.bind(scene.py()) {
                    selection.call_method1("discard", (group,))?;
                }
            }
        }
        bump(scene)
    }
}
fn bump(scene: &Bound<'_, PyAny>) -> PyResult<()> {
    scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)
}
