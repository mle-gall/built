// SPDX-License-Identifier: GPL-3.0-or-later
//! Reversible inspector edits and visibility overrides for linked tags.
use super::steps::{version, After, Obj, RecordStep};
use crate::model::{links, tag_edits};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};

fn scene_edit(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    s.arg(py, 0).call_method1("apply", (scene, true))?;
    version(scene)
}
fn scene_after(
    s: &RecordStep,
    py: Python<'_>,
    scene: &Obj<'_>,
    phase: Option<&str>,
) -> PyResult<()> {
    if s.is_direct() && phase == Some("undo") {
        s.arg(py, 0).call_method1("apply", (scene, false))?;
    }
    version(scene)
}
#[pyclass(extends=RecordStep,module="built_core")]
pub(crate) struct LinkSubsetCommand;
#[pymethods]
impl LinkSubsetCommand {
    #[new]
    fn new(edit: Obj<'_>) -> PyClassInitializer<Self> {
        PyClassInitializer::from(
            RecordStep::new("LinkSubsetCommand", scene_edit, vec![("edit", edit)])
                .explicit_inverse()
                .hook(After(scene_after)),
        )
        .add_subclass(Self)
    }
}
#[pyclass(extends=RecordStep,module="built_core")]
pub(crate) struct SetLocationCommand;
#[pymethods]
impl SetLocationCommand {
    #[new]
    fn new(edit: Obj<'_>) -> PyClassInitializer<Self> {
        PyClassInitializer::from(
            RecordStep::new("Set project location", scene_edit, vec![("edit", edit)])
                .rows()
                .explicit_inverse()
                .hook(After(scene_after)),
        )
        .add_subclass(Self)
    }
}
fn tag_edit(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    s.arg(py, 0).call_method1("apply", (true,))?;
    super::history_support::dirty_group_chunks(scene)?;
    version(scene)
}
fn tag_after(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    if s.is_direct() && phase == Some("undo") {
        s.arg(py, 0).call_method1("apply", (false,))?;
    }
    super::history_support::dirty_group_chunks(scene)?;
    version(scene)
}
#[pyclass(extends=RecordStep,module="built_core")]
pub(crate) struct EditTagCommand;
#[pymethods]
impl EditTagCommand {
    #[new]
    fn new(edit: Obj<'_>) -> PyClassInitializer<Self> {
        let s = RecordStep::new("EditTagCommand", tag_edit, vec![("edit", edit)])
            .rows()
            .explicit_inverse()
            .hook(After(tag_after));
        s.set_undo_by_transaction(true);
        PyClassInitializer::from(s).add_subclass(Self)
    }
}
fn settings_apply(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    s.arg(py, 0).call_method1("apply", (scene,))?;
    version(scene)
}
#[pyclass(extends=RecordStep,module="built_core")]
pub(crate) struct DocumentSettingsCommand;
#[pymethods]
impl DocumentSettingsCommand {
    #[new]
    fn new(name: &str, edit: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let name = match name {
            "Set model units" => "Set model units",
            "Set dimension style" => "Set dimension style",
            "Set annotation font" => "Set annotation font",
            _ => {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "unknown document setting",
                ))
            }
        };
        Ok(PyClassInitializer::from(
            RecordStep::new(name, settings_apply, vec![("edit", edit)]).rows(),
        )
        .add_subclass(Self))
    }
}
fn discard_invisible(scene: &Obj<'_>) -> PyResult<()> {
    let selection = scene.getattr("selection")?;
    for e in scene
        .py()
        .get_type::<PyList>()
        .call1((&selection,))?
        .try_iter()?
    {
        let e = e?;
        if !scene.call_method1("entity_visible", (&e,))?.is_truthy()? {
            selection.call_method1("discard", (e,))?;
        }
    }
    Ok(())
}
fn switch_tags(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    s.publish("before", tag_edits::layer_state(scene)?.into_any());
    let overrides = PyDict::new(py);
    for pair in links::registry_view(scene)?
        .call_method0("items")?
        .try_iter()?
    {
        let pair = pair?;
        let entry = pair.get_item(1)?;
        if entry.is_instance_of::<PyDict>() {
            overrides.set_item(
                pair.get_item(0)?,
                entry.call_method1("get", ("overrides",))?,
            )?;
        }
    }
    s.publish(
        "links_before",
        py.import("copy")?.call_method1("deepcopy", (overrides,))?,
    );
    let tags = s.arg(py, 0);
    let visible = s.arg(py, 2);
    let locked = s.arg(py, 3);
    if !visible.is_none() {
        let on = visible.is_truthy()?;
        for f in s.arg(py, 1).try_iter()? {
            scene.getattr("folder_visibility")?.set_item(f?, on)?;
        }
        tag_edits::set_visible(scene, &tags, &visible)?;
        for t in tags.try_iter()? {
            links::note_visibility(scene, &t?, &visible)?;
        }
        discard_invisible(scene)?;
    }
    if !locked.is_none() {
        tag_edits::set_locked(scene, &tags, &locked)?;
    }
    version(scene)
}
fn tags_after(
    s: &RecordStep,
    py: Python<'_>,
    scene: &Obj<'_>,
    phase: Option<&str>,
) -> PyResult<()> {
    if phase == Some("undo") && s.is_direct() {
        tag_edits::apply_layer_state(scene, &s.as_named(py, "before")?)?;
        let overrides = s.as_named(py, "links_before")?;
        for p in links::registry_view(scene)?
            .call_method0("items")?
            .try_iter()?
        {
            let p = p?;
            let key = p.get_item(0)?;
            let entry = p.get_item(1)?;
            if entry.is_instance_of::<PyDict>() && overrides.contains(&key)? {
                let value = overrides.get_item(key)?;
                let dict = if value.is_truthy()? {
                    py.get_type::<PyDict>().call1((value,))?
                } else {
                    PyDict::new(py).into_any()
                };
                entry.set_item("overrides", dict)?;
            }
        }
    } else if phase == Some("redo") {
        discard_invisible(scene)?;
    }
    version(scene)
}
#[pyclass(extends=RecordStep,module="built_core")]
pub(crate) struct SetTagStateCommand;
#[pymethods]
impl SetTagStateCommand {
    #[new]
    fn new<'py>(
        py: Python<'py>,
        tags: Obj<'py>,
        folders: Obj<'py>,
        visible: Obj<'py>,
        locked: Obj<'py>,
    ) -> PyResult<PyClassInitializer<Self>> {
        let list = py.get_type::<PyList>();
        let step = RecordStep::new(
            "SetTagStateCommand",
            switch_tags,
            vec![
                ("tags", list.call1((tags,))?),
                ("folders", list.call1((folders,))?),
                ("visible", visible),
                ("locked", locked),
            ],
        )
        .rows()
        .paint()
        .explicit_inverse()
        .hook(After(tags_after))
        .publishes(&["before", "links_before"]);
        step.set_undo_by_transaction(true);
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<LinkSubsetCommand>()?;
    m.add_class::<SetLocationCommand>()?;
    m.add_class::<EditTagCommand>()?;
    m.add_class::<DocumentSettingsCommand>()?;
    m.add_class::<SetTagStateCommand>()?;
    Ok(())
}
