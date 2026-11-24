// SPDX-License-Identifier: GPL-3.0-or-later
//! Native group and component steps (docs/commands-port.md): replace,
//! keep and convert components, re-edit 3D text, reparent, merge, make
//! unique, insert, delete, file into the structure and split into pieces.
//! Each is a [`RecordStep`] subclass under the retired Python class's name,
//! running the `definition_edits`, `placement_edits`, `group_geometry` and
//! `structure` kernels. Undo replays the step's transaction.
use super::steps::{version, After, Obj, RecordStep};
use crate::definition_edits as defs;
use crate::placement_edits as places;
use pyo3::prelude::*;
use pyo3::types::{PyList, PyTuple};
use pyo3::PyClassInitializer;

fn list<'py>(items: &Obj<'py>) -> PyResult<Obj<'py>> {
    Ok(PyList::new(items.py(), items.try_iter()?.collect::<PyResult<Vec<_>>>()?)?.into_any())
}

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}

fn host<'py>(py: Python<'py>, name: &str) -> PyResult<Obj<'py>> {
    py.import("core.history_commands")?.getattr(name)
}

fn discard<'py>(scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    scene.getattr("selection")?.getattr("discard")
}

fn select_only(scene: &Obj<'_>, item: &Obj<'_>) -> PyResult<()> {
    let sel = scene.getattr("selection")?;
    sel.call_method0("clear")?;
    sel.call_method1("add", (item,))?;
    Ok(())
}

// ------------------------------------------------------------ operations

fn run_replace(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let fields = ["mesh", "children", "name", "axes", "component", "text3d", "ifc", "ext", "billboard", "exploded"];
    defs::replace_definitions(s.items(py, 1)?, &s.arg(py, 0), fields.iter().map(|f| f.to_string()).collect())?;
    version(scene)
}

fn run_keep(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    // A repeated execute keeps the very template the first one made.
    let previous = s.remembered(py, 0).unwrap_or_else(|| none(py));
    let (template, _) = defs::keep_template(scene, &s.arg(py, 0), s.arg(py, 1).extract()?, &previous)?;
    s.remember(vec![template]);
    version(scene)
}

fn run_add_kept(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    defs::append_members(&scene.getattr("kept_components")?, &s.arg(py, 0))?;
    version(scene)
}

fn run_to_component(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    places::convert_component(&s.arg(py, 0), &s.arg(py, 1))?;
    version(scene)
}

fn run_text3d(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    // Redo lays out again the very letters the first run made.
    let before = s.remembered(py, 0).unwrap_or_else(|| none(py));
    let old = defs::rebuild_letter_placement(&s.arg(py, 0), &s.arg(py, 1), &before)?;
    s.remember(vec![old]);
    version(scene)
}

fn moved<'py>(s: &RecordStep, py: Python<'py>) -> PyResult<Obj<'py>> {
    Ok(s.remembered(py, 0).unwrap_or_else(|| PyList::empty(py).into_any()))
}

fn run_reparent(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let (_, moves) = places::reparent(&scene.getattr("groups")?, &s.arg(py, 0), &s.arg(py, 1), &discard(scene)?)?;
    let groups = PyList::new(py, moves.into_iter().map(|m| m.0))?.into_any();
    host(py, "_dirty_placements")?.call1((&groups,))?;
    s.remember(vec![groups]);
    version(scene)
}

fn after_reparent(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    let groups = moved(s, py)?;
    if phase == Some("undo") {
        scene.getattr("selection")?.call_method0("clear")?;
    } else {
        host(py, "_unselect")?.call1((scene, &groups))?;
    }
    host(py, "_dirty_placements")?.call1((&groups,))?;
    version(scene)
}

fn merged<'py>(s: &RecordStep, py: Python<'py>) -> PyResult<Obj<'py>> {
    match s.remembered(py, 0) {
        Some(m) => Ok(m),
        None => {
            // Built once: a redo puts back the same merged group.
            let m = defs::merge_group_geometry(&s.arg(py, 0))?;
            s.remember(vec![m.clone()]);
            Ok(m)
        }
    }
}

fn run_merge(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let group = merged(s, py)?;
    defs::replace_roots(&scene.getattr("groups")?, s.items(py, 0)?, &group, &discard(scene)?)?;
    select_only(scene, &group)?;
    version(scene)
}

fn after_merge(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    if let Some(group) = s.remembered(py, 0) {
        if phase == Some("undo") {
            discard(scene)?.call1((&group,))?;
        } else {
            select_only(scene, &group)?;
        }
    }
    version(scene)
}

fn run_unique(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let prior = crate::group_geometry::make_placement_unique(scene, &s.arg(py, 0), s.arg(py, 1).extract()?, true)?;
    if prior.is_some() {
        version(scene)?;
    }
    Ok(())
}

fn run_insert(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let (group, attachment) = (s.arg(py, 0), s.arg(py, 1));
    // Inside an open container it goes into the container.
    let ctx = scene.getattr("edit_group")?;
    let ctx = if !ctx.is_none() && !ctx.getattr("children")?.is_none() { ctx } else { none(py) };
    let (owner, _) = places::insert_placement(&scene.getattr("groups")?, &ctx, &scene.getattr("mesh")?, &group)?;
    s.publish("_owner", owner);
    crate::structure::file_created_placement(scene, &group)?;
    if !attachment.is_none() {
        let anchor = py
            .import("core.annotation_anchor")?
            .getattr("TextPlacementAnchor")?
            .call_method1("create", (scene, scene.call_method1("occurrence_path", (&group,))?, &attachment))?;
        if !anchor.is_none() {
            scene.getattr("text_placement_anchors")?.call_method1("append", (anchor,))?;
        }
    }
    select_only(scene, &group)?;
    version(scene)
}

fn after_insert(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    let group = s.arg(py, 0);
    if phase == Some("undo") {
        discard(scene)?.call1((&group,))?;
    } else {
        select_only(scene, &group)?;
    }
    version(scene)
}

fn run_delete(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let group = s.arg(py, 0);
    let (owner, index) = defs::delete_placement(scene, &group)?;
    s.publish("_owner", owner);
    s.publish("index", index.into_pyobject(py)?.into_any());
    discard(scene)?.call1((&group,))?;
    scene.getattr("mesh")?.call_method0("purge_debris")?;
    version(scene)
}

fn after_delete(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    if phase == Some("redo") {
        discard(scene)?.call1((s.arg(py, 0),))?;
        scene.getattr("mesh")?.call_method0("purge_debris")?;
    }
    version(scene)
}

fn run_structure(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    crate::structure::set_structure_nodes(&s.arg(py, 0), &s.arg(py, 1))?;
    version(scene)
}

fn run_split(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let emptied = s.remembered(py, 0).unwrap_or_else(|| none(py));
    let (_, emptied) = places::split_placement(&s.arg(py, 0), &s.arg(py, 1), &emptied)?;
    s.remember(vec![emptied]);
    version(scene)
}

fn run_finish(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    // How a named material answers light in a render; the viewport draws
    // the same. A material no longer in the registry is left alone.
    let material = scene.getattr("materials")?.call_method1("get", (s.arg(py, 0),))?;
    if material.is_none() {
        return Ok(());
    }
    material.setattr("finish", s.arg(py, 1))?;
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
    ReplaceComponent = "ReplaceComponentCommand";
    KeepComponent = "KeepComponentCommand";
    AddKeptComponents = "AddKeptComponentsCommand";
    GroupToComponent = "GroupToComponentCommand";
    EditText3D = "EditText3DCommand";
    ReparentGroups = "ReparentGroupsCommand";
    MergeGroups = "MergeGroupsCommand";
    MakeUnique = "MakeUniqueCommand";
    InsertGroup = "InsertGroupCommand";
    DeleteGroup = "DeleteGroupCommand";
    SetStructureNode = "SetStructureNodeCommand";
    SplitIntoPieces = "SplitIntoPiecesCommand";
    SetMaterialFinish = "SetMaterialFinishCommand";
}

#[pymethods]
impl SetMaterialFinish {
    #[new]
    fn new(name: Obj<'_>, finish: Obj<'_>) -> PyClassInitializer<Self> {
        let step = RecordStep::new("SetMaterialFinishCommand", run_finish, vec![("_name", name), ("_finish", finish)]);
        PyClassInitializer::from(step).add_subclass(Self)
    }
}

#[pymethods]
impl ReplaceComponent {
    #[new]
    fn new(py: Python<'_>, targets: Obj<'_>, source: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        // Instances already sharing the source, and non-instances, are left alone.
        let targets = PyList::new(py, defs::replacement_targets(&targets, &source)?)?.into_any();
        let step = RecordStep::new("ReplaceComponentCommand", run_replace, vec![("source", source), ("targets", targets)]);
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}

#[pymethods]
impl KeepComponent {
    #[new]
    #[pyo3(signature = (instance, keep = true))]
    fn new(py: Python<'_>, instance: Obj<'_>, keep: bool) -> PyClassInitializer<Self> {
        let keep = pyo3::types::PyBool::new(py, keep).to_owned().into_any();
        let step = RecordStep::new("KeepComponentCommand", run_keep, vec![("instance", instance), ("keep", keep)]);
        PyClassInitializer::from(step).add_subclass(Self)
    }
}

#[pymethods]
impl AddKeptComponents {
    #[new]
    fn new(templates: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let step = RecordStep::new("AddKeptComponentsCommand", run_add_kept, vec![("templates", list(&templates)?)]);
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}

#[pymethods]
impl GroupToComponent {
    #[new]
    #[pyo3(signature = (group, name = None))]
    fn new(py: Python<'_>, group: Obj<'_>, name: Option<Obj<'_>>) -> PyClassInitializer<Self> {
        let name = name.unwrap_or_else(|| none(py));
        let step = RecordStep::new("GroupToComponentCommand", run_to_component, vec![("group", group), ("_name", name)]);
        PyClassInitializer::from(step).add_subclass(Self)
    }
}

#[pymethods]
impl EditText3D {
    #[new]
    fn new(py: Python<'_>, group: Obj<'_>, params: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let params = py.import("builtins")?.getattr("dict")?.call1((params,))?;
        let step = RecordStep::new("EditText3DCommand", run_text3d, vec![("group", group), ("params", params)]);
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}

#[pymethods]
impl ReparentGroups {
    #[new]
    #[pyo3(signature = (groups, target = None))]
    fn new(py: Python<'_>, groups: Obj<'_>, target: Option<Obj<'_>>) -> PyResult<PyClassInitializer<Self>> {
        let target = target.unwrap_or_else(|| none(py));
        let step = RecordStep::new("ReparentGroupsCommand", run_reparent, vec![("groups", list(&groups)?), ("target", target)])
            .hook(After(after_reparent));
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}

#[pymethods]
impl MergeGroups {
    #[new]
    fn new(groups: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let step = RecordStep::new("MergeGroupsCommand", run_merge, vec![("_groups", list(&groups)?)]).hook(After(after_merge));
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}

#[pymethods]
impl MakeUnique {
    #[new]
    #[pyo3(signature = (group, rename = false))]
    fn new(py: Python<'_>, group: Obj<'_>, rename: bool) -> PyClassInitializer<Self> {
        let rename = pyo3::types::PyBool::new(py, rename).to_owned().into_any();
        let step = RecordStep::new("MakeUniqueCommand", run_unique, vec![("group", group), ("_rename", rename)]);
        PyClassInitializer::from(step).add_subclass(Self)
    }
}

#[pymethods]
impl InsertGroup {
    #[new]
    #[pyo3(signature = (group, attachment = None))]
    fn new(py: Python<'_>, group: Obj<'_>, attachment: Option<Obj<'_>>) -> PyClassInitializer<Self> {
        let attachment = attachment.unwrap_or_else(|| none(py));
        let step = RecordStep::new("InsertGroupCommand", run_insert, vec![("group", group), ("attachment", attachment)])
            .groups()
            .publishes(&["_owner"])
            .hook(After(after_insert));
        PyClassInitializer::from(step).add_subclass(Self)
    }
}

#[pymethods]
impl DeleteGroup {
    #[new]
    fn new(group: Obj<'_>) -> PyClassInitializer<Self> {
        let step = RecordStep::new("DeleteGroupCommand", run_delete, vec![("group", group)])
            .groups()
            .publishes(&["_owner", "index"])
            .hook(After(after_delete));
        PyClassInitializer::from(step).add_subclass(Self)
    }
}

#[pymethods]
impl SetStructureNode {
    #[new]
    fn new(targets: Obj<'_>, key: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let step = RecordStep::new("SetStructureNodeCommand", run_structure, vec![("targets", list(&targets)?), ("key", key)]);
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}

#[pymethods]
impl SplitIntoPieces {
    #[new]
    fn new(py: Python<'_>, group: Obj<'_>, pieces: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let pieces = list(&pieces)?;
        // Where the group's faces went, for the change feed: `[(source
        // mesh, piece mesh, {"face": [(key, key)]})]` for the pieces the
        // store cut (`core.store_parts.copy_part`).
        let origin = py.import("core.store_parts")?.getattr("origin_of")?;
        let copies = PyList::empty(py);
        for piece in pieces.try_iter()? {
            let mesh = piece?.getattr("mesh")?;
            let found = origin.call1((&mesh,))?;
            let (source, keys) = (found.get_item(0)?, found.get_item(1)?);
            if !source.is_none() {
                copies.append(PyTuple::new(py, [source, mesh, keys])?)?;
            }
        }
        let step = RecordStep::new("SplitIntoPiecesCommand", run_split, vec![("group", group), ("pieces", pieces), ("copies", copies.into_any())]);
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}
