// SPDX-License-Identifier: GPL-3.0-or-later
//! Prepare the new definition off-document. History swaps only this occurrence;
//! undo and redo retain the original objects and never reopen the source.
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList},
};

struct Definition {
    mesh: Py<PyAny>,
    children: Py<PyList>,
    subset: Py<PyAny>,
}

impl Definition {
    fn read(group: &Bound<'_, PyAny>, subset: Bound<'_, PyAny>) -> PyResult<Self> {
        let py = group.py();
        let children = py
            .get_type::<PyList>()
            .call1((group.getattr("children")?,))?;
        Ok(Self {
            mesh: group.getattr("mesh")?.unbind(),
            children: children.cast_into::<PyList>()?.unbind(),
            subset: subset.unbind(),
        })
    }
}

#[pyclass]
pub(crate) struct LinkSubset {
    placement: Py<PyAny>,
    path: String,
    before: Definition,
    after: Definition,
    tags: Py<PyAny>,
    materials: Py<PyAny>,
    tag_states: Option<(Py<PyAny>, Py<PyAny>)>,
}

#[pymethods]
impl LinkSubset {
    #[staticmethod]
    fn prepare(
        group: &Bound<'_, PyAny>,
        scene: &Bound<'_, PyAny>,
        text: &str,
    ) -> PyResult<Option<Self>> {
        let py = group.py();
        let record = crate::model::links::link_of(group)?.unwrap_or_else(|| py.None().into_bound(py));
        if record.is_none() || crate::model::links::linked_ancestor(&group.getattr("owner")?)?.is_some() {
            return Err(PyValueError::new_err(
                "Choose a linked placement in this document",
            ));
        }
        let subset = py.import("json")?.call_method1("loads", (text,))?;
        crate::model::links::validate_select(&subset)?;
        let previous = record.call_method1("get", ("select",))?;
        if previous.eq(&subset)? {
            return Ok(None);
        }
        let path = record.get_item("path")?.extract::<String>()?;
        let nested = record.call_method1("get", ("include_nested", true))?;
        let materials = py
            .get_type::<PyDict>()
            .call1((scene.getattr("materials")?,))?;
        let source = py.import("core.links")?.call_method1(
            "_read_definition",
            (&materials, &path, &subset, nested, group),
        )?;
        let temporary = crate::model::links::clone_shell_of(group)?;
        crate::model::links::populate(&temporary, &source.get_item(0)?, false, None)?;
        let after = Definition::read(&temporary, subset)?;
        // Native placement lists have a single owner.
        temporary.setattr("children", PyList::empty(py))?;
        Ok(Some(Self {
            placement: group.clone().unbind(),
            path,
            before: Definition::read(group, previous)?,
            after,
            materials: materials.unbind(),
            tags: source.get_item(1)?.unbind(),
            tag_states: None,
        }))
    }

    fn apply(&mut self, scene: &Bound<'_, PyAny>, forward: bool) -> PyResult<()> {
        let py = scene.py();
        let group = self.placement.bind(py);
        let record = crate::model::links::link_of(group)?.unwrap_or_else(|| py.None().into_bound(py));
        let new = if forward { &self.after } else { &self.before };
        if let Some((before, after)) = &self.tag_states {
            let (from, to) = if forward {
                (before, after)
            } else {
                (after, before)
            };
            from.call_method1(py, "go", (scene, to))?;
        } else {
            let before = crate::doc::unlink::tags::TagState::capture(scene)?.into_any();
            scene
                .getattr("materials")?
                .call_method1("update", (&self.materials,))?;
            crate::model::links::sync_tags(scene, &pyo3::types::PyString::new(py, &self.path), self.tags.bind(py))?;
            self.tag_states = Some((before.unbind(), crate::doc::unlink::tags::TagState::capture(scene)?.into_any().unbind()));
        }
        let selection = scene.getattr("selection")?;
        let selected = selection.try_iter()?.collect::<PyResult<Vec<_>>>()?;
        for part in selected {
            let mut cursor = part.getattr("owner").ok();
            while let Some(owner) = cursor.filter(|o| !o.is_none()) {
                if owner.is(group) {
                    selection.call_method1("discard", (&part,))?;
                    break;
                }
                cursor = owner.getattr("owner").ok();
            }
        }
        group.setattr("mesh", &new.mesh)?;
        group.call_method1("adopt", (&new.children,))?;
        for child in new.children.bind(py) {
            child.setattr("owner", group)?;
        }
        if new.subset.bind(py).is_none() {
            record.call_method1("pop", ("select", py.None()))?;
        } else {
            record.set_item("select", &new.subset)?;
        }
        Ok(())
    }
}

#[pyfunction]
pub(crate) fn link_selection_sources(
    window: &Bound<'_, PyAny>,
) -> PyResult<std::collections::BTreeMap<String, Vec<Vec<String>>>> {
    let scene = window.getattr("viewport")?.getattr("scene")?;
    let ty = window.py().import("core.group")?.getattr("Group")?;
    let mut sources = std::collections::BTreeMap::new();
    for doc in window.call_method0("documents")?.try_iter()? {
        let doc = doc?;
        let path = doc.getattr("path")?;
        let source = doc.getattr("viewport")?.getattr("scene")?;
        if path.is_none() || source.is(&scene) {
            continue;
        }
        let mut uids = Vec::new();
        for group in source.getattr("selection")?.try_iter()? {
            let group = group?;
            if group.is_instance(&ty)? {
                let path: Vec<String> = source
                    .call_method1("occurrence_path", (&group,))?
                    .extract()?;
                // The linked subset accepts an occurrence address for repeated UIDs.
                uids.push(path);
            }
        }
        if !uids.is_empty() {
            sources.insert(path.str()?.to_string(), uids);
        }
    }
    Ok(sources)
}

#[pyfunction]
pub(crate) fn link_subset_rows(group: &Bound<'_, PyAny>) -> PyResult<Vec<(String, String)>> {
    let py = group.py();
    let record = crate::model::links::link_of(group)?.unwrap_or_else(|| py.None().into_bound(py));
    let subset = record.call_method1("get", ("select",))?;
    let summary = if subset.is_none() {
        "Full document".to_owned()
    } else if subset.contains("uids")? {
        let n = subset.get_item("uids")?.len()?;
        format!("{n} {}", if n == 1 { "item" } else { "items" })
    } else {
        "Filtered".to_owned()
    };
    let id = group.getattr("_doc_key")?.extract()?;
    let doc = crate::doc::lock();
    let mut pending = vec![id];
    let mut count = 0;
    while let Some(id) = pending.pop() {
        let children = &doc.need(id).map_err(PyValueError::new_err)?.children;
        pending.extend_from_slice(children);
        count += children.len();
    }
    Ok(vec![
        ("Subset".into(), summary),
        ("Stable UIDs".into(), count.to_string()),
    ])
}
