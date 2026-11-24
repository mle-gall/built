// SPDX-License-Identifier: GPL-3.0-or-later
//! Native Hide and Tag steps (docs/commands-port.md): hide or show
//! objects, faces and edges, and put entities on a tag.
//!
//! Placements are written as one document op over the selection. Faces
//! keep the flag in their attribute dictionaries and other entities in
//! their own fields. When every target is a placement or a journalled
//! store, the transaction replays the step. Otherwise the step keeps the
//! flags it overwrote and puts them back: its real inverse.
use super::py::{doc_get_many, doc_set_many, PyPlacement};
use super::steps::{version, After, Meshes, Obj, RecordStep};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyList};
use pyo3::PyClassInitializer;
use std::collections::HashSet;

const DEFAULT_LAYER: &str = "Layer 0";

fn host<'py>(py: Python<'py>, name: &str) -> PyResult<Obj<'py>> {
    py.import("core.history_commands")?.getattr(name)
}

fn all_placements(items: &[Obj<'_>]) -> bool {
    items.iter().all(|e| e.is_instance_of::<PyPlacement>())
}

/// The meshes owning the targets that keep `field` outside the document:
/// faces and edges (and anything else that is not a placement or record).
fn carriers<'py>(items: &[Obj<'py>], field: &str) -> PyResult<Vec<Obj<'py>>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for e in items {
        if e.is_instance_of::<PyPlacement>() {
            continue;
        }
        let record = match e.getattr_opt("_record_fields")? {
            Some(fields) => fields.contains(field)?,
            None => false,
        };
        if record {
            continue;
        }
        let mesh = e.getattr_opt("_m")?.unwrap_or_else(|| e.py().None().into_bound(e.py()));
        if seen.insert(mesh.as_ptr() as usize) {
            out.push(mesh);
        }
    }
    Ok(out)
}

fn hidden_meshes<'py>(s: &RecordStep, py: Python<'py>, _scene: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    carriers(&s.items(py, 0)?, "hidden")
}

fn layer_meshes<'py>(s: &RecordStep, py: Python<'py>, _scene: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    carriers(&s.items(py, 0)?, "layer")
}

// ------------------------------------------------------------ flags

fn is_hidden(e: &Obj<'_>) -> PyResult<bool> {
    match e.getattr_opt("attrs")?.filter(|a| !a.is_none()) {
        Some(attrs) => attrs.call_method1("get", ("hidden",))?.is_truthy(),
        None => match e.getattr_opt("hidden")? {
            Some(v) => v.is_truthy(),
            None => Ok(false),
        },
    }
}

fn set_hidden(e: &Obj<'_>, hidden: bool) -> PyResult<()> {
    match e.getattr_opt("attrs")?.filter(|a| !a.is_none()) {
        Some(attrs) if hidden => attrs.set_item("hidden", true),
        Some(attrs) => attrs.call_method1("pop", ("hidden", e.py().None())).map(|_| ()),
        None => e.setattr("hidden", hidden),
    }
}

fn layer_of<'py>(e: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = e.py();
    let value = match e.getattr_opt("attrs")?.filter(|a| !a.is_none()) {
        Some(attrs) => attrs.call_method1("get", ("layer",))?,
        None => e.getattr_opt("layer")?.unwrap_or_else(|| py.None().into_bound(py)),
    };
    Ok(if value.is_truthy()? { value } else { pyo3::types::PyString::new(py, DEFAULT_LAYER).into_any() })
}

fn assign_layer(e: &Obj<'_>, name: &Obj<'_>) -> PyResult<()> {
    let py = e.py();
    let value = if name.eq(DEFAULT_LAYER)? { py.None().into_bound(py) } else { name.clone() };
    match e.getattr_opt("attrs")?.filter(|a| !a.is_none()) {
        Some(attrs) if value.is_none() => attrs.call_method1("pop", ("layer", py.None())).map(|_| ()),
        Some(attrs) => attrs.set_item("layer", value),
        None if e.hasattr("layer")? => e.setattr("layer", value),
        None => Ok(()),
    }
}

// ------------------------------------------------------------ hide

fn run_hide(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let list = s.arg(py, 0);
    let items = s.items(py, 0)?;
    let hidden: bool = s.arg(py, 1).extract()?;
    let bulk = !items.is_empty() && all_placements(&items);
    let previous = if bulk {
        // One document op over a whole selection (Hide of 20,000 placements).
        let prev = doc_get_many(py, &list, "hidden")?;
        doc_set_many(py, &list, "hidden", PyBool::new(py, hidden).as_any())?;
        prev.into_any()
    } else {
        let prev = PyList::new(py, items.iter().map(is_hidden).collect::<PyResult<Vec<_>>>()?)?;
        for e in &items {
            set_hidden(e, hidden)?;
        }
        prev.into_any()
    };
    if s.remembered(py, 0).is_none() {
        s.remember(vec![previous]);
    }
    if hidden {
        // An invisible entity must not linger in the selection.
        host(py, "_unselect")?.call1((scene, &list))?;
    }
    if !bulk {
        host(py, "_dirty_group_chunks")?.call1((scene,))?;
    }
    version(scene)
}

fn after_hide(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    if phase == Some("undo") && s.is_direct() {
        if let Some(previous) = s.remembered(py, 0) {
            for (e, p) in s.items(py, 0)?.iter().zip(previous.try_iter()?) {
                set_hidden(e, p?.is_truthy()?)?;
            }
        }
        host(py, "_dirty_group_chunks")?.call1((scene,))?;
    } else if phase == Some("redo") && s.arg(py, 1).is_truthy()? {
        host(py, "_unselect")?.call1((scene, s.arg(py, 0)))?;
    }
    version(scene)
}

fn hide_step(name: &'static str, entities: Obj<'_>, hidden: bool) -> PyResult<RecordStep> {
    let py = entities.py();
    let mut kept = Vec::new();
    for e in entities.try_iter()? {
        let e = e?;
        if e.hasattr("hidden")? || e.hasattr("attrs")? {
            kept.push(e);
        }
    }
    let own = !carriers(&kept, "hidden")?.is_empty();
    let list = PyList::new(py, kept)?.into_any();
    let flag = PyBool::new(py, hidden).to_owned().into_any();
    Ok(RecordStep::new(name, run_hide, vec![("_entities", list.clone()), ("_hidden", flag.clone()), ("entities", list), ("hides", flag)])
        .own_inverse_if(own)
        .on_meshes(Meshes(hidden_meshes))
        .hook(After(after_hide)))
}

// ------------------------------------------------------------ tags

fn run_assign(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let list = s.arg(py, 0);
    let items = s.items(py, 0)?;
    // A tag that does not exist yet is made (Entity Info names a new one).
    let mut tag = scene.call_method1("layer", (s.arg(py, 1),))?;
    if tag.is_none() {
        tag = py.import("core.layers")?.getattr("Layer")?.call1((s.arg(py, 1),))?;
        scene.getattr("layers")?.call_method1("append", (&tag,))?;
        s.publish("_created", tag.clone());
    }
    // Entities carry the tag's key; the argument may be its label.
    let key = tag.getattr("key")?;
    let bulk = !items.is_empty() && all_placements(&items);
    let previous = if bulk {
        let value = if key.eq(DEFAULT_LAYER)? { py.None().into_bound(py) } else { key.clone() };
        let prev = doc_get_many(py, &list, "layer")?;
        doc_set_many(py, &list, "layer", &value)?;
        let named = prev
            .iter()
            .map(|p| Ok(if p.is_truthy()? { p } else { pyo3::types::PyString::new(py, DEFAULT_LAYER).into_any() }))
            .collect::<PyResult<Vec<_>>>()?;
        PyList::new(py, named)?
    } else {
        let prev = PyList::new(py, items.iter().map(layer_of).collect::<PyResult<Vec<_>>>()?)?;
        for e in &items {
            assign_layer(e, &key)?;
        }
        prev
    };
    if s.remembered(py, 0).is_none() {
        s.remember(vec![previous.into_any()]);
    }
    if !bulk {
        host(py, "_dirty_group_chunks")?.call1((scene,))?;
    }
    version(scene)
}

fn after_assign(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    if phase == Some("undo") && s.is_direct() {
        if let Some(previous) = s.remembered(py, 0) {
            for (e, p) in s.items(py, 0)?.iter().zip(previous.try_iter()?) {
                assign_layer(e, &p?)?;
            }
        }
        if let Ok(tag) = s.as_named(py, "_created") {
            let layers = scene.getattr("layers")?;
            if !tag.is_none() && layers.contains(&tag)? {
                layers.call_method1("remove", (tag,))?;
            }
        }
        host(py, "_dirty_group_chunks")?.call1((scene,))?;
    }
    version(scene)
}

// ------------------------------------------------------------ the classes

macro_rules! steps {
    ($($ty:ident = $name:literal;)*) => {
        $(
            #[pyclass(module = "ingetrazo_core", name = $name, extends = RecordStep, frozen)]
            pub struct $ty;
        )*
        pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
            $(m.add_class::<$ty>()?;)*
            Ok(())
        }
    };
}

steps! {
    Hide = "HideCommand";
    HideEdges = "HideEdgesCommand";
    AssignLayer = "AssignLayerCommand";
}

#[pymethods]
impl Hide {
    /// Hide (or show) objects, faces and edges: Edit ▸ Hide and Unhide.
    #[new]
    #[pyo3(signature = (entities, hidden = true))]
    fn new(entities: Obj<'_>, hidden: bool) -> PyResult<PyClassInitializer<Self>> {
        Ok(PyClassInitializer::from(hide_step("HideCommand", entities, hidden)?).add_subclass(Self))
    }
}

#[pymethods]
impl HideEdges {
    /// The edges-only spelling, kept for callers that grew up with it.
    #[new]
    #[pyo3(signature = (edges, hidden = true))]
    fn new(edges: Obj<'_>, hidden: bool) -> PyResult<PyClassInitializer<Self>> {
        Ok(PyClassInitializer::from(hide_step("HideEdgesCommand", edges, hidden)?).add_subclass(Self))
    }
}

#[pymethods]
impl AssignLayer {
    /// Move entities onto a tag (the Tag field in Entity Info).
    #[new]
    fn new(py: Python<'_>, entities: Obj<'_>, layer: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let items: Vec<Obj<'_>> = entities.try_iter()?.collect::<PyResult<_>>()?;
        let own = !carriers(&items, "layer")?.is_empty();
        let list = PyList::new(py, items)?.into_any();
        let step = RecordStep::new("AssignLayerCommand", run_assign, vec![("_entities", list), ("_layer", layer)])
            .own_inverse_if(own)
            .on_meshes(Meshes(layer_meshes))
            .publishes(&["_created"])
            .hook(After(after_assign));
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}
