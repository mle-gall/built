// SPDX-License-Identifier: GPL-3.0-or-later
//! Native Explode and Unlink (docs/commands-port.md). Explode dissolves one
//! level of a group; Unlink makes a linked file's placement the host's own.
//! Its own geometry merges into the loose mesh and the groups nested in it
//! come out whole, one level up.
//!
//! Undone by the step's document transaction. On a store mesh History
//! journals, the store is replayed too. Otherwise the step keeps the mesh's
//! before and after states and restores them around the replay, as the
//! Python command did. The run's records stay on the step as attributes:
//! `copies` (where the group's elements went, for the change feed),
//! `_lifted` (the children lifted), `index`, `snapshot`.
use super::unlink::{self,tags::TagState};
use super::steps::{running, version, After, Obj, RecordStep};
use crate::store::command_facade::command_explode_geometry;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PySet, PyTuple};
use pyo3::PyClassInitializer;

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}

fn host<'py>(py: Python<'py>, name: &str) -> PyResult<Obj<'py>> {
    py.import("built_core.history")?.getattr(name)
}

fn optional<'py>(o: &Obj<'py>, name: &str) -> PyResult<Option<Obj<'py>>> {
    Ok(o.getattr_opt(name)?.filter(|v| !v.is_none()))
}

fn store_state<'py>(py: Python<'py>) -> PyResult<Obj<'py>> {
    py.import("built_core.store_mesh")?.getattr("StoreState")
}

fn kids<'py>(py: Python<'py>, lifted: &Obj<'py>) -> PyResult<Bound<'py, PySet>> {
    PySet::new(py, lifted.try_iter()?.map(|row| row?.get_item(0)).collect::<PyResult<Vec<_>>>()?)
}

/// Redo on the store: the mesh goes to the state the first run made
/// (O(chunks), the same keys) when it stands where that run started.
fn replay(py: Python<'_>, me: &Obj<'_>, scene: &Obj<'_>, mesh: &Obj<'_>) -> PyResult<bool> {
    let state = store_state(py)?;
    let (Some(after), Some(snapshot), Some(before0)) = (optional(me, "_after")?, optional(me, "snapshot")?, optional(me, "_before0")?) else {
        return Ok(false);
    };
    if !snapshot.is_instance(&state)? || !before0.is_instance(&state)? || snapshot.ne(&before0)? {
        return Ok(false);
    }
    mesh.call_method1("restore_state", (after,))?;
    crate::doc::history_support::drop_dead_elements(scene, mesh)?;
    Ok(true)
}

fn geometry(py: Python<'_>, me: &Obj<'_>, mesh: &Obj<'_>, group: &Obj<'_>, transform: &Obj<'_>, paint: &Obj<'_>) -> PyResult<()> {
    let group_mesh = group.getattr("mesh")?;
    let data: Option<[f64; 16]> = if transform.is_none() { None } else { Some(transform.call_method0("data")?.extract()?) };
    let paint_arg = (!paint.is_none()).then_some(paint);
    let exploded = command_explode_geometry(mesh, &group_mesh, data, paint_arg)?
        .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("group geometry could not be transferred"))?;
    let row = PyTuple::new(py, [group_mesh, mesh.clone(), exploded.copies.into_bound(py)])?;
    me.setattr("copies", PyList::new(py, [row])?)?;
    if let Some(snapshot) = optional(me, "snapshot")? {
        me.setattr("_before0", snapshot)?;
        me.setattr("_after", mesh.call_method0("capture_state")?)?;
    }
    Ok(())
}

fn run_explode(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let me = running(py).expect("a running step");
    let mesh = scene.getattr("mesh")?;
    let group = s.arg(py, 0);
    let snapshot = if s.journalled() { none(py) } else { mesh.call_method0("capture_state")? };
    me.setattr("snapshot", &snapshot)?;
    let groups = scene.getattr("groups")?;
    let index = groups.call_method1("index", (&group,))?;
    me.setattr("index", &index)?;
    let element_set = py.import("built_core")?.getattr("ElementSet")?;
    me.setattr("_selection", element_set.call1((scene.getattr("selection")?,))?)?;
    let placement = group.getattr_opt("xform")?.unwrap_or_else(|| none(py));
    let paint = group.getattr_opt("material")?.unwrap_or_else(|| none(py));
    if !replay(py, &me, scene, &mesh)? {
        let mut transform = placement.clone();
        let defining = mesh.getattr("__dict__")?.get_item("_instance_definition").ok().filter(|d| !d.is_none());
        if defining.is_some() {
            let parent = mesh.getattr("_store")?.call_method0("tool_placement")?;
            transform = if placement.is_none() { parent } else { parent.mul(&placement)? };
        }
        geometry(py, &me, &mesh, &group, &transform, &paint)?;
        if !s.journalled() && optional(&me, "_after")?.is_none() {
            me.setattr("_after", mesh.call_method0("capture_state")?)?;
        }
    }
    // The nested objects come out one level up, whole: their placement
    // composes with the parent's and an unpainted child takes its paint.
    let lifted = py.import("built_core")?.getattr("lift_placements")?.call1((&groups, &group, &index))?;
    me.setattr("_lifted", &lifted)?;
    let selection = scene.getattr("selection")?;
    selection.call_method1("discard", (&group,))?;
    let kids = kids(py, &lifted)?;
    if kids.len() > 0 {
        scene.setattr("selection", kids)?; // select what came out
    }
    version(scene)
}

/// The undo of a scene with no document (a stand-in): put everything back
/// by hand, as the Python command's own inverse did.
fn undo_by_hand(py: Python<'_>, me: &Obj<'_>, scene: &Obj<'_>, group: &Obj<'_>) -> PyResult<()> {
    let lifted = me.getattr("_lifted")?;
    let groups = scene.getattr("groups")?;
    py.import("built_core")?.getattr("lower_placements")?.call1((&groups, &lifted))?;
    if let Some(snapshot) = optional(me, "snapshot")? {
        scene.getattr("mesh")?.call_method1("restore_state", (snapshot,))?;
    }
    let children = lifted.try_iter()?.map(|row| row?.get_item(0)).collect::<PyResult<Vec<_>>>()?;
    group.setattr("children", PyList::new(py, children)?)?;
    groups.call_method1("insert", (me.getattr("index")?, group))?;
    Ok(())
}

fn after_explode(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    let me = running(py).expect("a running step");
    let mesh = scene.getattr("mesh")?;
    let group = s.arg(py, 0);
    let set = py.get_type::<PySet>();
    if phase == Some("undo") {
        if s.is_direct() && !scene.hasattr("_root")? {
            undo_by_hand(py, &me, scene, &group)?;
        } else if !s.journalled() {
            if let Some(snapshot) = optional(&me, "snapshot")? {
                mesh.call_method1("restore_state", (snapshot,))?;
            }
        }
        if let Some(selection) = optional(&me, "_selection")? {
            scene.setattr("selection", set.call1((selection,))?)?;
        }
    } else {
        if !s.journalled() {
            if let Some(after) = optional(&me, "_after")? {
                mesh.call_method1("restore_state", (after,))?;
            }
        }
        scene.getattr("selection")?.call_method1("discard", (&group,))?;
        let lifted = me.getattr("_lifted")?;
        if lifted.is_truthy()? {
            scene.setattr("selection", kids(py, &lifted)?)?;
        }
        crate::doc::history_support::drop_dead_elements(scene, &mesh)?;
    }
    version(scene)
}

#[pyclass(module = "built_core", name = "ExplodeGroupCommand", extends = RecordStep, frozen)]
pub struct ExplodeGroup;

#[pymethods]
impl ExplodeGroup {
    /// Dissolve one level of `group`: its geometry welds into the loose
    /// mesh, and the groups and components nested in it come out whole as
    /// top-level objects in its place.
    #[new]
    fn new(group: Obj<'_>) -> PyClassInitializer<Self> {
        let step = RecordStep::new("ExplodeGroupCommand", run_explode, vec![("group", group)]).hook(After(after_explode));
        PyClassInitializer::from(step).add_subclass(Self)
    }

    // What the step answers before it ran (the run sets its own).
    #[classattr]
    fn snapshot(py: Python<'_>) -> Py<PyAny> {
        py.None()
    }
    #[classattr]
    fn index(py: Python<'_>) -> Py<PyAny> {
        py.None()
    }
    #[classattr]
    fn _selection(py: Python<'_>) -> Py<PyAny> {
        py.None()
    }
    #[classattr]
    fn _before0(py: Python<'_>) -> Py<PyAny> {
        py.None()
    }
    #[classattr]
    fn _after(py: Python<'_>) -> Py<PyAny> {
        py.None()
    }
    #[classattr]
    fn copies(py: Python<'_>) -> Py<PyAny> {
        PyTuple::empty(py).into_any().unbind()
    }
    #[classattr]
    fn _lifted(py: Python<'_>) -> Py<PyAny> {
        PyTuple::empty(py).into_any().unbind()
    }
}

// ------------------------------------------------------------ unlink

/// `{id(a): b}` for the pairs copied (`forward`), or `{id(b): a}`.
fn moved_map<'py>(py: Python<'py>, pairs: &Obj<'py>, forward: bool) -> PyResult<Bound<'py, PyDict>> {
    let map = PyDict::new(py);
    for pair in pairs.try_iter()? {
        let pair = pair?;
        let (a, b) = (pair.get_item(0)?, pair.get_item(1)?);
        let (from, to) = if forward { (a, b) } else { (b, a) };
        map.set_item(from.as_ptr() as usize, to)?;
    }
    Ok(map)
}

/// The first run: copy, then fill the files linked inside.
fn unlink_first(py: Python<'_>, me: &Obj<'_>, scene: &Obj<'_>, groups: &Obj<'_>) -> PyResult<()> {
    let pairs = PyList::empty(py);
    let fields = PyTuple::new(py, ["mesh", "children", "ext", "name", "component"])?;
    let result = py
        .import("built_core")?
        .getattr("definition_edits")?
        .getattr("unlink_placements")?
        .call1((scene, groups, fields, &pairs))?;
    me.setattr("_before", result.get_item(0)?)?;
    me.setattr("_after", result.get_item(1)?)?;
    let host_path = scene.getattr_opt("_project_host_path")?.unwrap_or_else(|| none(py));
    for row in result.get_item(2)?.try_iter()? {
        let row = row?;
        let outer = row.get_item(0)?;
        for inner in row.get_item(1)?.try_iter()? {
            let inner = inner?;
            let (old, new, part_pairs) = (inner.get_item(0)?, inner.get_item(1)?, inner.get_item(2)?);
            if unlink::fill(scene,&new,Some(&outer),Some(&host_path))?.is_some_and(|message|!message.is_empty()) {
                pairs.call_method1("extend", (part_pairs,))?; // kept as it showed
            } else {
                pairs.call_method1("extend", (crate::model::links::nested_pairs(&old, &new)?,))?;
            }
        }
    }
    me.setattr("_pairs", pairs)
}

/// The groups and meshes the copies hold, the files linked inside them
/// aside (theirs are their links').
fn copied<'py>(py: Python<'py>, after: &Obj<'py>) -> PyResult<(Bound<'py, PyList>, Bound<'py, PyList>)> {
    let groups = PyList::empty(py);
    let meshes = PyList::empty(py);
    let mut stack = Vec::new();
    for a in after.try_iter()? {
        let a = a?;
        meshes.append(a.get_item("mesh")?)?;
        let children = a.get_item("children")?;
        if !children.is_none() {
            stack.extend(children.try_iter()?.collect::<PyResult<Vec<_>>>()?);
        }
    }
    while let Some(g) = stack.pop() {
        if crate::model::links::is_linked(&g)? {
            continue;
        }
        groups.append(&g)?;
        meshes.append(g.getattr("mesh")?)?;
        let children = g.getattr("children")?;
        if !children.is_none() {
            stack.extend(children.try_iter()?.collect::<PyResult<Vec<_>>>()?);
        }
    }
    Ok((groups, meshes))
}

/// No placement links the files `gone` any more: their tags are the
/// host's own now, at the top of the tree, the copies relabelled where one
/// merged into a host tag of the same label.
fn release(py: Python<'_>, me: &Obj<'_>, scene: &Obj<'_>, gone: &Bound<'_, PySet>) -> PyResult<()> {
    let counts = py.import("built_core.purge")?.getattr("layer_counts")?;
    let used = counts.call1((scene,))?.cast_into::<PyDict>()?;
    let namespace = py.import("types")?.getattr("SimpleNamespace")?;
    let after = me.getattr("_after")?;
    let mut parts = Vec::new();
    for a in after.try_iter()? {
        let a = a?;
        let kw = PyDict::new(py);
        kw.set_item("mesh", a.get_item("mesh")?)?;
        kw.set_item("children", a.get_item("children")?)?;
        kw.set_item("layer", py.None())?;
        parts.push(namespace.call((), Some(&kw))?);
    }
    let kw = PyDict::new(py);
    kw.set_item("loose_mesh", py.None())?;
    kw.set_item("groups", PyList::new(py, parts)?)?;
    let copies = counts.call1((namespace.call((), Some(&kw))?,))?.cast_into::<PyDict>()?;
    let outside = PyDict::new(py);
    for (k, n) in used.iter() {
        let c: i64 = copies.get_item(&k)?.map_or(Ok(0), |v| v.extract())?;
        outside.set_item(k, n.extract::<i64>()? - c)?;
    }
    let renames = PyDict::new(py);
    for path in gone.iter() {
        renames.update(crate::model::links::release_tags(scene, &path, used.as_any(), outside.as_any())?.as_mapping())?;
    }
    if renames.len() > 0 {
        let (groups, meshes) = copied(py, &after)?;
        crate::model::links::relabel(groups.as_any(), meshes.as_any(), &renames)?;
    }
    Ok(())
}

fn sources<'py>(py: Python<'py>, scene: &Obj<'py>) -> PyResult<Bound<'py, PySet>> {
    PySet::new(py, crate::model::links::sources(scene)?)
}

fn run_unlink(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let me = running(py).expect("a running step");
    let groups = s.arg(py, 0);
    if !groups.is_truthy()? {
        return Err(PyErr::from_value(host(py, "CommandRejected")?.call1(("Nothing linked is selected",))?));
    }
    if optional(&me, "tops")?.is_none() {
        let iter_groups = py.import("built_core")?.getattr("purge_iter_groups")?;
        let want: std::collections::HashSet<usize> = groups.try_iter()?.map(|g| Ok(g?.as_ptr() as usize)).collect::<PyResult<_>>()?;
        let mut tops = Vec::new();
        for root in scene.getattr("groups")?.try_iter()? {
            let root = root?;
            let mut hit = false;
            for g in iter_groups.call1((PyList::new(py, [&root])?,))?.try_iter()? {
                if want.contains(&(g?.as_ptr() as usize)) {
                    hit = true;
                    break;
                }
            }
            if hit {
                tops.push(root);
            }
        }
        me.setattr("tops", PyList::new(py, tops)?)?;
    }
    let live = sources(py, scene)?;
    if optional(&me, "_before")?.is_none() {
        me.setattr("_tags_before", TagState::capture(scene)?)?;
        unlink_first(py, &me, scene, &groups)?;
    } else {
        py.import("built_core")?.getattr("definition_edits")?.getattr("restore_definitions")?.call1((&groups, me.getattr("_after")?))?;
        me.getattr("_tags_before")?.call_method1("go", (scene, me.getattr("_tags_after")?))?;
    }
    unlink::move_holds(scene, &groups, &moved_map(py,&me.getattr("_pairs")?,true)?)?;
    let gone = live.call_method1("difference", (sources(py, scene)?,))?.cast_into::<PySet>()?;
    if optional(&me, "_tags_after")?.is_none() {
        if gone.len() > 0 {
            release(py, &me, scene, &gone)?;
        }
        let registry = crate::model::links::registry(scene)?;
        let cache = scene.getattr("__dict__")?.get_item("_link_tag_tables").ok().filter(|c| c.is_truthy().unwrap_or(false));
        for p in gone.iter() {
            registry.call_method1("pop", (&p, py.None()))?;
            if let Some(cache) = &cache {
                cache.call_method1("pop", (&p, py.None()))?;
            }
        }
        // Nothing projects from their definitions any more.
        let paths: Vec<String> = gone.iter().filter_map(|p| p.extract().ok()).collect();
        crate::model::links::release_definitions(scene, &paths)?;
        me.setattr("_tags_after", TagState::capture(scene)?)?;
    }
    if let Some(errors) = scene.getattr_opt("link_errors")?.filter(|e| e.is_instance_of::<PyDict>()) {
        let kept = PyDict::new(py);
        for p in errors.call_method0("keys")?.try_iter()?.collect::<PyResult<Vec<_>>>()? {
            if gone.contains(crate::model::links::norm(&p)?)? {
                kept.set_item(&p, errors.call_method1("pop", (&p,))?)?;
            }
        }
        me.setattr("_errors", kept)?;
    }
    version(scene)
}

/// The undo of a scene with no document: put everything back by hand.
fn unlink_by_hand(py: Python<'_>, me: &Obj<'_>, scene: &Obj<'_>, groups: &Obj<'_>) -> PyResult<()> {
    let Some(before) = optional(me, "_before")? else {
        return Ok(());
    };
    py.import("built_core")?.getattr("definition_edits")?.getattr("restore_definitions")?.call1((groups, before))?;
    unlink::move_holds(scene, groups, &moved_map(py,&me.getattr("_pairs")?,false)?)?;
    me.getattr("_tags_after")?.call_method1("go", (scene, me.getattr("_tags_before")?))?;
    let errors = me.getattr("_errors")?;
    if errors.is_truthy()? {
        if let Some(now) = scene.getattr_opt("link_errors")?.filter(|e| e.is_instance_of::<PyDict>()) {
            now.call_method1("update", (errors,))?;
        }
    }
    version(scene)
}

fn after_unlink(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    let me = running(py).expect("a running step");
    let groups = s.arg(py, 0);
    let undo = phase == Some("undo");
    if undo && s.is_direct() && !scene.hasattr("_root")? {
        return unlink_by_hand(py, &me, scene, &groups);
    }
    if optional(&me, "_tags_before")?.is_none() {
        return version(scene); // the run refused: nothing to replay
    }
    unlink::move_holds(scene, &groups, &moved_map(py,&me.getattr("_pairs")?,!undo)?)?;
    let before = me.getattr("_tags_before")?.getattr("cache")?;
    let after = me.getattr("_tags_after")?.getattr("cache")?;
    let dict = scene.getattr("__dict__")?;
    let cache = dict.call_method1("setdefault", ("_link_tag_tables", PyDict::new(py)))?;
    let (from, to) = if undo { (after, before) } else { (before, after) };
    unlink::tags::sync(&cache,from.cast::<PyDict>()?,to.cast::<PyDict>()?,false)?;
    if let Some(errors) = scene.getattr_opt("link_errors")?.filter(|e| e.is_instance_of::<PyDict>()) {
        let mine = me.getattr("_errors")?;
        if undo {
            errors.call_method1("update", (mine,))?;
        } else {
            for path in mine.try_iter()? {
                errors.call_method1("pop", (path?, py.None()))?;
            }
        }
    }
    version(scene)
}

#[pyclass(module = "built_core", name = "UnlinkCommand", extends = RecordStep, frozen)]
pub struct Unlink;

#[pymethods]
impl Unlink {
    /// Make Unique of a linked file's placement: the link breaks and the
    /// geometry stays, each placement the host's own component, placed,
    /// painted and tagged as it was. A file linked inside the source stays
    /// a live link; a source no placement links any more leaves the
    /// registry and its tags become the host's. One undo brings the live
    /// links back exactly, and redo the very same copies.
    #[new]
    fn new(py: Python<'_>, groups: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        use crate::model::links;
        let mut kept = Vec::new();
        for g in groups.try_iter()? {
            let g = g?;
            let owner = g.getattr_opt("owner")?.unwrap_or_else(|| none(py));
            if links::is_linked(&g)? && links::linked_ancestor(&owner)?.is_none() {
                kept.push(g);
            }
        }
        let mut paths: Vec<Obj<'_>> = Vec::new();
        for g in &kept {
            let path = links::link_of(g)?.expect("a linked placement").get_item("path")?;
            if !paths.iter().any(|p| p.eq(&path).unwrap_or(false)) {
                paths.push(path);
            }
        }
        let args = vec![("groups", PyList::new(py, kept)?.into_any()), ("paths", PyList::new(py, paths)?.into_any())];
        let step = RecordStep::new("UnlinkCommand", run_unlink, args).hook(After(after_unlink));
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }

    #[classattr]
    fn tops(py: Python<'_>) -> Py<PyAny> {
        py.None()
    }
    #[classattr]
    fn _before(py: Python<'_>) -> Py<PyAny> {
        py.None()
    }
    #[classattr]
    fn _after(py: Python<'_>) -> Py<PyAny> {
        py.None()
    }
    #[classattr]
    fn _tags_before(py: Python<'_>) -> Py<PyAny> {
        py.None()
    }
    #[classattr]
    fn _tags_after(py: Python<'_>) -> Py<PyAny> {
        py.None()
    }
    #[classattr]
    fn _errors(py: Python<'_>) -> Py<PyAny> {
        PyDict::new(py).into_any().unbind()
    }
    #[classattr]
    fn _pairs(py: Python<'_>) -> Py<PyAny> {
        PyTuple::empty(py).into_any().unbind()
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<ExplodeGroup>()?;
    m.add_class::<Unlink>()
}
