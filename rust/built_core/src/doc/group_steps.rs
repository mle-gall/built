// SPDX-License-Identifier: GPL-3.0-or-later
//! Native group and component steps: replace,
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
    if !s.arg(py, 1).is_none() {
        scene.getattr("materials")?.call_method1("update", (s.arg(py, 1),))?;
        scene.getattr("layers")?.call_method1("extend", (s.arg(py, 2),))?;
    }
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
    crate::doc::history_support::dirty_placements(&groups)?;
    s.remember(vec![groups]);
    version(scene)
}

fn after_reparent(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    let groups = moved(s, py)?;
    if phase == Some("undo") {
        scene.getattr("selection")?.call_method0("clear")?;
    } else {
        crate::doc::history_support::unselect(scene, &groups)?;
    }
    crate::doc::history_support::dirty_placements(&groups)?;
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
    crate::hosted_openings::regenerate(scene,&group)?;
    crate::structure::file_created_placement(scene, &group)?;
    if !attachment.is_none() {
        let anchor = py
            .import("built_core.annotation_anchor")?
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
    crate::hosted_openings::detach(scene,&group)?;
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

// ------------------------------------------------------------ making groups

/// What a Make Group execute keeps: `[group, mesh, before, after]`.
fn made<'py>(s: &RecordStep, py: Python<'py>) -> Option<(Obj<'py>, Obj<'py>, Obj<'py>, Obj<'py>)> {
    Some((s.remembered(py, 0)?, s.remembered(py, 1)?, s.remembered(py, 2)?, s.remembered(py, 3)?))
}

fn run_make_group(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let mesh = scene.getattr("mesh")?;
    let geometry = s.arg(py, 0);
    // Store snapshots are O(1): they undo the mesh when History does not
    // journal it (an object mesh) and for a direct undo.
    let before = mesh.call_method0("capture_state")?;
    // Built once: a redo puts back the same group the next commands hold.
    let group = match s.remembered(py, 0) {
        Some(g) => g,
        None => geometry.call_method1("build", (scene, s.arg(py, 1), s.arg(py, 2)))?,
    };
    geometry.call_method1("take_off", (&mesh, discard(scene)?))?;
    let after = mesh.call_method0("capture_state")?;
    s.remember(vec![group.clone(), mesh, before.clone(), after]);
    scene.getattr("groups")?.call_method1("append", (&group,))?;
    crate::structure::file_created_placement(scene, &group)?;
    s.publish("group", group.clone());
    s.publish("snapshot", before);
    select_only(scene, &group)?;
    version(scene)
}

fn after_make_group(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    if let Some((group, mesh, before, after)) = made(s, py) {
        if phase == Some("undo") {
            if !s.journalled() || s.is_direct() {
                mesh.call_method1("restore_state", (before,))?;
            }
            discard(scene)?.call1((&group,))?;
        } else {
            if !s.journalled() {
                mesh.call_method1("restore_state", (after,))?;
            }
            select_only(scene, &group)?;
        }
    }
    version(scene)
}

fn run_make_nested(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let inner = s.arg(py, 0);
    // The loose part of the selection becomes the container's own mesh.
    let container = if !inner.is_none() {
        inner.call_method1("do", (scene,))?;
        inner.getattr("group")?
    } else {
        match s.remembered(py, 0) {
            Some(c) => c,
            None => {
                let mesh = py.import("built_core.mesh")?.getattr("Mesh")?.call0()?;
                let kw = pyo3::types::PyDict::new(py);
                kw.set_item("name", s.arg(py, 2))?;
                py.import("built_core.group")?.getattr("Group")?.call((mesh,), Some(&kw))?
            }
        }
    };
    s.remember(vec![container.clone()]);
    places::nest(&scene.getattr("groups")?, &container, s.items(py, 1)?, &discard(scene)?)?;
    if inner.is_none() {
        crate::structure::file_created_placement(scene, &container)?;
    }
    s.publish("container", container.clone());
    select_only(scene, &container)?;
    version(scene)
}

fn after_make_nested(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    let inner = s.arg(py, 0);
    if !inner.is_none() {
        inner.call_method1("doc_after", (scene, phase))?;
    }
    let sel = scene.getattr("selection")?;
    sel.call_method0("clear")?;
    if phase == Some("redo") {
        if let Some(container) = s.remembered(py, 0) {
            sel.call_method1("add", (container,))?;
        }
    }
    version(scene)
}

fn run_component_of(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let nest = s.arg(py, 0);
    nest.call_method1("do", (scene,))?;
    let container = nest.getattr("container")?;
    container.setattr("component", true)?;
    s.publish("container", container);
    version(scene)
}

fn after_component_of(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    s.arg(py, 0).call_method1("doc_after", (scene, phase)).map(|_| ())
}

fn run_in_context(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let (inner, ctx) = (s.arg(py, 0), s.arg(py, 1));
    if ctx.getattr("children")?.is_none() {
        ctx.setattr("children", PyList::empty(py))?;
    }
    // The scene reads its top level from the open group meanwhile: the
    // document's lists cannot be swapped like Python lists.
    let d = scene.getattr("__dict__")?;
    let prev = d.call_method1("get", ("_groups_owner",))?;
    d.set_item("_groups_owner", &ctx)?;
    let ran = inner.call_method1("do", (scene,));
    d.set_item("_groups_owner", prev)?;
    ran?;
    if ctx.getattr("children")?.is_truthy()? && ctx.getattr("xform")?.is_none() {
        ctx.setattr("xform", py.import("built_core")?.getattr("Matrix4")?.call0()?)?;
        ctx.setattr("component", false)?;
    }
    let group = match inner.getattr_opt("group")?.filter(|g| !g.is_none()) {
        Some(g) => g,
        None => inner.getattr_opt("container")?.unwrap_or_else(|| none(py)),
    };
    s.publish("group", group);
    version(scene)
}

fn after_in_context(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    s.arg(py, 0).call_method1("doc_after", (scene, phase))?;
    version(scene)
}

fn run_explode_view(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    // The parts' matrices, `exploded` and `explode_offset` are document fields.
    crate::placement_transform::apply_explode(&s.arg(py, 0), s.arg(py, 1).extract()?, &s.arg(py, 2).extract::<String>()?, &none(py))?;
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
    MakeGroup = "MakeGroupCommand";
    MakeNestedGroup = "MakeNestedGroupCommand";
    MakeComponentOf = "MakeComponentOfCommand";
    GroupInContext = "GroupInContextCommand";
    ExplodeView = "ExplodeViewCommand";
}

#[pymethods]
impl ExplodeView {
    /// Pull a component's parts apart to `factor` along `mode`, or put
    /// them back with `factor=0` (see `core.explode`).
    #[new]
    #[pyo3(signature = (container, factor, mode = "outward"))]
    fn new(py: Python<'_>, container: Obj<'_>, factor: f64, mode: &str) -> PyResult<PyClassInitializer<Self>> {
        let args = vec![
            ("container", container),
            ("factor", factor.into_pyobject(py)?.into_any()),
            ("mode", pyo3::types::PyString::new(py, mode).into_any()),
        ];
        Ok(PyClassInitializer::from(RecordStep::new("ExplodeViewCommand", run_explode_view, args)).add_subclass(Self))
    }
}

#[pymethods]
impl MakeGroup {
    #[new]
    #[pyo3(signature = (faces, edges, component = false, name = None))]
    fn new(py: Python<'_>, faces: Obj<'_>, edges: Obj<'_>, component: bool, name: Option<Obj<'_>>) -> PyResult<PyClassInitializer<Self>> {
        // `component` builds the mesh in local coordinates and hands back an instance.
        let geometry = py.import("built_core")?.getattr("GroupGeometry")?.call1((faces, edges))?;
        let component = pyo3::types::PyBool::new(py, component).to_owned().into_any();
        let args = vec![("_geometry", geometry), ("_component", component), ("_name", name.unwrap_or_else(|| none(py)))];
        let step = RecordStep::new("MakeGroupCommand", run_make_group, args)
            .publishes(&["group", "snapshot"])
            .hook(After(after_make_group));
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}

#[pymethods]
impl MakeNestedGroup {
    #[new]
    #[pyo3(signature = (faces, edges, groups, name = None))]
    fn new(py: Python<'_>, faces: Obj<'_>, edges: Obj<'_>, groups: Obj<'_>, name: Option<Obj<'_>>) -> PyResult<PyClassInitializer<Self>> {
        let (faces, edges, name) = (list(&faces)?, list(&edges)?, name.unwrap_or_else(|| none(py)));
        // Reusing Make Group for the loose half keeps one road off the loose mesh.
        let inner = if faces.is_truthy()? || edges.is_truthy()? {
            let kw = pyo3::types::PyDict::new(py);
            kw.set_item("name", &name)?;
            py.get_type::<MakeGroup>().call((faces, edges), Some(&kw))?
        } else {
            none(py)
        };
        let args = vec![("_inner", inner), ("groups", list(&groups)?), ("name", name)];
        let step = RecordStep::new("MakeNestedGroupCommand", run_make_nested, args)
            .publishes(&["container"])
            .hook(After(after_make_nested));
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}

#[pymethods]
impl MakeComponentOf {
    #[new]
    #[pyo3(signature = (faces, edges, groups, name = None))]
    fn new(py: Python<'_>, faces: Obj<'_>, edges: Obj<'_>, groups: Obj<'_>, name: Option<Obj<'_>>) -> PyResult<PyClassInitializer<Self>> {
        // Make Group's container, made a component: one component holding the groups.
        let kw = pyo3::types::PyDict::new(py);
        kw.set_item("name", name.unwrap_or_else(|| none(py)))?;
        let nest = py.get_type::<MakeNestedGroup>().call((faces, edges, groups), Some(&kw))?;
        let step = RecordStep::new("MakeComponentOfCommand", run_component_of, vec![("_nest", nest)])
            .publishes(&["container"])
            .hook(After(after_component_of));
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}

#[pymethods]
impl GroupInContext {
    #[new]
    fn new(inner: Obj<'_>, context: Obj<'_>) -> PyClassInitializer<Self> {
        let step = RecordStep::new("GroupInContextCommand", run_in_context, vec![("inner", inner), ("context", context)])
            .publishes(&["group"])
            .hook(After(after_in_context));
        PyClassInitializer::from(step).add_subclass(Self)
    }
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
    #[pyo3(signature=(templates, materials=None, layers=None))]
    fn new<'py>(py: Python<'py>, templates: Obj<'py>, materials: Option<Obj<'py>>, layers: Option<Obj<'py>>) -> PyResult<PyClassInitializer<Self>> {
        let step = RecordStep::new("AddKeptComponentsCommand", run_add_kept, vec![
            ("templates", list(&templates)?), ("materials", materials.unwrap_or_else(|| none(py))),
            ("layers", layers.unwrap_or_else(|| PyList::empty(py).into_any())),
        ]);
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
        let copies = PyList::empty(py);
        for piece in pieces.try_iter()? {
            let mesh = piece?.getattr("mesh")?;
            if let (Some(source), Some(keys)) = crate::kernel::store_views::provenance::origin_of(&mesh)? {
                copies.append(PyTuple::new(py, [source, mesh, keys])?)?;
            }
        }
        let step = RecordStep::new("SplitIntoPiecesCommand", run_split, vec![("group", group), ("pieces", pieces), ("copies", copies.into_any())]);
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}
