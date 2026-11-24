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
    old_back: Option<Py<PyAny>>,
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
            old_back: None,
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
    #[pyo3(name = "keeps_rows")]
    const KEEPS_ROWS: bool = false;
    #[classattr]
    #[pyo3(name = "keeps_groups")]
    const KEEPS_GROUPS: bool = true;
    #[classattr]
    #[pyo3(name = "keeps_paint")]
    const KEEPS_PAINT: bool = false;
    #[classattr]
    #[pyo3(name = "doc_undo")]
    const DOC_UNDO: bool = true;
    #[classattr]
    #[pyo3(name = "_journal_geometry")]
    const _JOURNAL_GEOMETRY: bool = false;
    #[pyo3(name = "do")]
    fn attach(&mut self, scene: &Bound<'_, PyAny>) -> PyResult<()> {
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
            self.old_back = Some(scene.getattr("back_face_color")?.unbind());
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
        slf.setattr("_doc_txn", txn)?;
        let mut command = slf.borrow_mut();
        command.meshes.clear();
        command.placements.clear();
        command.materials.bind(slf.py()).clear();
        command.layers.bind(slf.py()).clear();
        Ok(())
    }
    fn doc_after(&self, scene: &Bound<'_, PyAny>, phase: &str) -> PyResult<()> {
        if let Some(old) = &self.old_back {
            scene.setattr(
                "back_face_color",
                if phase == "undo" { old } else { &self.back }.bind(scene.py()),
            )?;
        }
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

/// The first step in a newly opened, untitled drawing. The private worker
/// cannot reserve the UI thread's document writer. Capture just the root
/// collections touched by the import and retain their existing store handles.
#[pyclass(dict, module = "ingetrazo_core")]
pub(crate) struct SkpOpenUndo {
    before: Py<PyDict>,
    after: Py<PyDict>,
}

impl SkpOpenUndo {
    pub(crate) fn capture<'py>(scene: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyDict>> {
        let py = scene.py();
        let snapshot = PyDict::new(py);
        let seq = py.get_type::<PyList>();
        let map = py.get_type::<PyDict>();
        for key in [
            "groups",
            "layers",
            "saved_views",
            "dimensions",
            "text_labels",
            "layer_folders",
        ] {
            snapshot.set_item(key, seq.call1((scene.getattr(key)?,))?)?;
        }
        for key in [
            "materials",
            "folder_visibility",
            "folder_parents",
            "layer_folder_uids",
            "layer_folder_names",
        ] {
            snapshot.set_item(key, map.call1((scene.getattr(key)?,))?)?;
        }
        snapshot.set_item("back_face_color", scene.getattr("back_face_color")?)?;
        Ok(snapshot)
    }
    pub(crate) fn new(before: Bound<'_, PyDict>, after: Bound<'_, PyDict>) -> Self {
        Self {
            before: before.unbind(),
            after: after.unbind(),
        }
    }
    fn restore(&self, scene: &Bound<'_, PyAny>, undo: bool) -> PyResult<()> {
        let py = scene.py();
        let snapshot = if undo { &self.before } else { &self.after };
        if undo {
            let selection = scene.getattr("selection")?;
            if selection.is_truthy()? {
                for g in self
                    .after
                    .bind(py)
                    .as_any()
                    .get_item("groups")?
                    .try_iter()?
                {
                    selection.call_method1("discard", (g?,))?;
                }
            }
        }
        for (key, v) in snapshot.bind(py) {
            scene.setattr(key.cast::<pyo3::types::PyString>()?, v)?;
        }
        bump(scene)
    }
}

#[pymethods]
impl SkpOpenUndo {
    #[classattr]
    #[pyo3(name = "keeps_rows")]
    const KEEPS_ROWS: bool = false;
    #[classattr]
    #[pyo3(name = "keeps_groups")]
    const KEEPS_GROUPS: bool = true;
    #[classattr]
    #[pyo3(name = "keeps_paint")]
    const KEEPS_PAINT: bool = false;
    fn undo(&self, scene: &Bound<'_, PyAny>) -> PyResult<()> {
        self.restore(scene, true)
    }
    #[pyo3(name = "do")]
    fn redo(&self, scene: &Bound<'_, PyAny>) -> PyResult<()> {
        self.restore(scene, false)
    }
}
