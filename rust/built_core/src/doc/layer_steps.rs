// SPDX-License-Identifier: GPL-3.0-or-later
//! Native tag-list steps: bring a template's
//! missing tags in, delete or merge a tag, and purge what nothing uses.
//!
//! The tag list, folders, the active tag, placements' tags and saved views
//! are document state, replayed by the step's transaction. Faces and edges
//! keep their tag in mesh attributes: Delete Layer writes those back itself
//! (its real inverse) when it relabelled any.
use super::purge::{purge_layer_carriers, purge_unused_layers, purge_unused_materials};
use super::steps::{version, After, Obj, RecordStep};
use super::tag_steps::{all_placements, assign_layer, carriers, layer_of};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyList, PyString, PyTuple};
use pyo3::PyClassInitializer;

const DEFAULT_LAYER: &str = "Layer 0";

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}

fn rejected(py: Python<'_>, why: &str) -> PyErr {
    match py
        .import("built_core.history")
        .and_then(|m| m.getattr("CommandRejected"))
        .and_then(|c| c.call1((why,)))
    {
        Ok(e) => PyErr::from_value(e),
        Err(e) => e,
    }
}



// ------------------------------------------------------------ ensure layers

/// Whether a template run added any tag or folder.
fn added_any(s: &RecordStep, py: Python<'_>) -> PyResult<bool> {
    match s.remembered(py, 0) {
        Some(added) => Ok(added.get_item(0)?.is_truthy()? || added.get_item(1)?.is_truthy()?),
        None => Ok(false),
    }
}

fn ensure(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    // Tags the document has keep their look, visibility and folder.
    let (layers, folders) = crate::structure::add_template_layers(scene, &s.arg(py, 0))?;
    let added = PyTuple::new(py, [PyList::new(py, layers)?.into_any(), PyList::new(py, folders)?.into_any()])?;
    s.remember(vec![added.clone().into_any()]);
    s.publish("_added", added.into_any());
    if added_any(s, py)? {
        version(scene)?;
    }
    Ok(())
}

fn run_ensure(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    ensure(s, py, scene)
}

fn run_ensure_structure(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    ensure(s, py, scene)?;
    // The structure's folders keep their nesting from the template.
    crate::structure::apply_folder_parents(scene, &s.arg(py, 0))?;
    version(scene)
}

fn after_ensure(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, _phase: Option<&str>) -> PyResult<()> {
    if added_any(s, py)? {
        version(scene)?;
    }
    Ok(())
}

// ------------------------------------------------------------ delete layer

fn run_delete_layer(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let tag = scene.call_method1("layer", (s.arg(py, 0),))?;
    if tag.is_none() || tag.getattr("key")?.eq(DEFAULT_LAYER)? {
        return Err(rejected(py, "layer cannot be deleted"));
    }
    // Entities carry the tag's key; the argument may be its label.
    let key = tag.getattr("key")?;
    let into_name = s.arg(py, 1);
    let into_name = if into_name.is_truthy()? { into_name } else { PyString::new(py, DEFAULT_LAYER).into_any() };
    let into = scene.call_method1("layer", (into_name,))?;
    if into.is_none() || into.is(&tag) {
        return Err(rejected(py, "no layer to merge into"));
    }
    let target = into.getattr("key")?;
    let mut carried = Vec::new();
    for e in purge_layer_carriers(py, scene)?.try_iter()? {
        let e = e?;
        if layer_of(&e)?.eq(&key)? {
            carried.push(e);
        }
    }
    // The scenes forget the tag.
    if let Some(views) = scene.getattr_opt("saved_views")?.filter(|v| !v.is_none()) {
        for view in views.try_iter()? {
            let view = view?;
            let hidden = view.getattr_opt("hidden_layers")?.filter(|h| !h.is_none());
            if let Some(hidden) = hidden {
                if hidden.is_truthy()? && hidden.contains(&key)? {
                    let kept: Vec<Obj<'_>> = hidden.try_iter()?.filter_map(|n| n.ok()).filter(|n| !n.eq(&key).unwrap_or(false)).collect();
                    view.setattr("hidden_layers", PyList::new(py, kept)?)?;
                }
            }
        }
    }
    for e in &carried {
        assign_layer(e, &target)?;
    }
    if scene.getattr_opt("active_layer")?.is_some_and(|a| a.eq(&key).unwrap_or(false)) {
        let active = if target.eq(DEFAULT_LAYER)? { none(py) } else { target.clone() };
        scene.setattr("active_layer", active)?;
    }
    scene.getattr("layers")?.call_method1("remove", (&tag,))?;
    // Faces and edges keep the tag in their meshes, outside the document.
    let own = !carriers(&carried, "layer")?.is_empty();
    s.set_undo_by_transaction(!own);
    s.publish("name", key.clone());
    s.remember(vec![key, PyList::new(py, &carried)?.into_any()]);
    if !all_placements(&carried) {
        crate::doc::history_support::dirty_group_chunks(scene)?;
    }
    version(scene)
}

fn after_delete_layer(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    if phase == Some("undo") && s.is_direct() {
        if let (Some(key), Some(carried)) = (s.remembered(py, 0), s.remembered(py, 1)) {
            for e in carried.try_iter()? {
                assign_layer(&e?, &key)?;
            }
        }
        crate::doc::history_support::dirty_group_chunks(scene)?;
    }
    version(scene)
}

// ------------------------------------------------------------ purge

/// What goes is decided once, on the first execute: a redo removes exactly
/// what the first run removed, not a new survey of a model drawn on since.
fn surveyed<'py>(s: &RecordStep, py: Python<'py>, scene: &Obj<'py>) -> PyResult<(Obj<'py>, Obj<'py>)> {
    if let (Some(layers), Some(materials)) = (s.remembered(py, 0), s.remembered(py, 1)) {
        return Ok((layers, materials));
    }
    let tags = PyList::empty(py);
    if s.arg(py, 0).is_truthy()? {
        let current = scene.getattr("layers")?;
        for tag in purge_unused_layers(py, scene)?.iter() {
            let index = current.call_method1("index", (&tag,))?;
            tags.append(PyTuple::new(py, [index, tag])?)?;
        }
    }
    let mats = PyList::empty(py);
    if s.arg(py, 1).is_truthy()? {
        let registry = scene.getattr("materials")?;
        for name in purge_unused_materials(py, scene)?.iter() {
            let material = registry.get_item(&name)?;
            mats.append(PyTuple::new(py, [name, material])?)?;
        }
    }
    s.remember(vec![tags.clone().into_any(), mats.clone().into_any()]);
    Ok((tags.into_any(), mats.into_any()))
}

fn run_purge(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let (tags, mats) = surveyed(s, py, scene)?;
    let layers = scene.getattr("layers")?;
    for pair in tags.try_iter()? {
        let tag = pair?.get_item(1)?;
        if layers.contains(&tag)? {
            layers.call_method1("remove", (tag,))?;
        }
    }
    let registry = scene.getattr("materials")?;
    for pair in mats.try_iter()? {
        registry.call_method1("pop", (pair?.get_item(0)?, none(py)))?;
    }
    version(scene)
}

// ------------------------------------------------------------ the classes

macro_rules! steps {
    ($($ty:ident = $name:literal;)*) => {
        $(
            #[pyclass(module = "built_core", name = $name, extends = RecordStep, frozen)]
            pub struct $ty;
        )*
        pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
            $(m.add_class::<$ty>()?;)*
            Ok(())
        }
    };
}

steps! {
    EnsureLayers = "EnsureLayersCommand";
    EnsureStructureLayers = "EnsureStructureLayersCommand";
    DeleteLayer = "DeleteLayerCommand";
    PurgeUnused = "PurgeUnusedCommand";
}

#[pymethods]
impl EnsureLayers {
    /// Give a document the tags and folders pasted geometry carries and it
    /// lacks, as a step of the paste (`built_core.layers.add_missing`).
    #[new]
    fn new(template: Obj<'_>) -> PyClassInitializer<Self> {
        let step = RecordStep::new("EnsureLayersCommand", run_ensure, vec![("_template", template)])
            .publishes(&["_added"])
            .hook(After(after_ensure));
        PyClassInitializer::from(step).add_subclass(Self)
    }
}

#[pymethods]
impl EnsureStructureLayers {
    /// The structure tree's tags and folders, with the folders' nesting.
    #[new]
    fn new(template: Obj<'_>) -> PyClassInitializer<Self> {
        let step = RecordStep::new("EnsureStructureLayersCommand", run_ensure_structure, vec![("_template", template)])
            .publishes(&["_added"])
            .hook(After(after_ensure));
        PyClassInitializer::from(step).add_subclass(Self)
    }
}

#[pymethods]
impl DeleteLayer {
    /// Delete a tag: what it carried goes to the default tag, or `into`
    /// (Merge into…), and the scenes forget it.
    #[new]
    #[pyo3(signature = (name, into = None))]
    fn new(py: Python<'_>, name: Obj<'_>, into: Option<Obj<'_>>) -> PyClassInitializer<Self> {
        let into = into.unwrap_or_else(|| none(py));
        let step = RecordStep::new("DeleteLayerCommand", run_delete_layer, vec![("_requested", name), ("into", into)])
            .publishes(&["name"])
            .hook(After(after_delete_layer));
        PyClassInitializer::from(step).add_subclass(Self)
    }
}

#[pymethods]
impl PurgeUnused {
    /// Purge Unused, for tags and/or materials.
    #[new]
    #[pyo3(signature = (layers = true, materials = true))]
    fn new(py: Python<'_>, layers: bool, materials: bool) -> PyClassInitializer<Self> {
        let args = vec![
            ("_do_layers", PyBool::new(py, layers).to_owned().into_any()),
            ("_do_materials", PyBool::new(py, materials).to_owned().into_any()),
        ];
        PyClassInitializer::from(RecordStep::new("PurgeUnusedCommand", run_purge, args)).add_subclass(Self)
    }

    /// What a run would remove, for the confirmation the panel shows.
    fn counts(slf: PyRef<'_, Self>, scene: Obj<'_>) -> PyResult<(usize, usize)> {
        let py = slf.py();
        let step: &RecordStep = &*slf.as_super();
        let (tags, mats) = surveyed(step, py, &scene)?;
        Ok((tags.len()?, mats.len()?))
    }
}
