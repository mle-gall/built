// SPDX-License-Identifier: GPL-3.0-or-later
//! Native mesh-maintenance steps (docs/commands-port.md): curve tags, the
//! push/pull clean-ups (orphan edges, coplanar seams, stitching), Simplify,
//! Reverse Faces and tag-as-you-draw.
//!
//! Each runs on the store's own native call when History journals the
//! mesh (the transaction replays it) and keeps its real inverse otherwise:
//! a snapshot, the swept edges, the merges or the curve ids it took.
use super::steps::{running, version, After, Meshes, Obj, RecordStep};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PySet, PyTuple};
use pyo3::PyClassInitializer;
use std::collections::HashSet;

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}

fn host<'py>(py: Python<'py>, name: &str) -> PyResult<Obj<'py>> {
    py.import("core.history")?.getattr(name)
}

fn optional<'py>(o: &Obj<'py>, name: &str) -> PyResult<Option<Obj<'py>>> {
    Ok(o.getattr_opt(name)?.filter(|v| !v.is_none()))
}

fn vectors<'py>(py: Python<'py>, points: Obj<'py>) -> PyResult<Obj<'py>> {
    let items = points.try_iter()?.map(|p| super::steps::vector(py, &p?)).collect::<PyResult<Vec<_>>>()?;
    Ok(PyList::new(py, items)?.into_any())
}

fn keys_of<'py>(py: Python<'py>, points: &Obj<'py>) -> PyResult<Bound<'py, PySet>> {
    let key = py.import("core.topology")?.getattr("_key")?;
    PySet::new(py, points.try_iter()?.map(|p| key.call1((p?,))).collect::<PyResult<Vec<_>>>()?)
}

fn touch(mesh: &Obj<'_>) -> PyResult<()> {
    mesh.setattr("_chunk_dirty", true)?;
    let serial: i64 = mesh.getattr("_mut_serial")?.extract()?;
    mesh.setattr("_mut_serial", serial + 1)
}

fn snapshot<'py>(s: &RecordStep, py: Python<'py>, mesh: &Obj<'py>) -> PyResult<Obj<'py>> {
    if s.journalled() { Ok(none(py)) } else { mesh.call_method0("capture_state") }
}

/// Undone by its enclosing snapshot when direct (nothing to do), a
/// version bump when a transaction replays it.
fn after_quiet(s: &RecordStep, _py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    if phase == Some("undo") && s.is_direct() {
        return Ok(());
    }
    version(scene)
}

/// A direct undo restores the `snapshot` the run kept.
fn after_snapshot(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    if phase == Some("undo") && s.is_direct() {
        let me = running(py).expect("a running step");
        if let Some(before) = optional(&me, "snapshot")? {
            scene.getattr("mesh")?.call_method1("restore_state", (before,))?;
            return version(scene);
        }
        return Ok(());
    }
    version(scene)
}

// ------------------------------------------------------------ curve tags

fn run_tag_curve(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let mesh = scene.getattr("mesh")?;
    let (points, closed) = (s.arg(py, 0), s.arg(py, 1));
    mesh.call_method1("tag_curve", (points, closed))?;
    Ok(())
}

fn run_explode_curves(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let me = running(py).expect("a running step");
    let mesh = scene.getattr("mesh")?;
    let edges = s.arg(py, 0);
    let ids = PySet::empty(py)?;
    for edge in edges.try_iter()? {
        if let Some(id) = optional(&edge?, "curve")? { ids.add(id)?; }
    }
    let store = mesh.getattr("_store")?;
    let prior = store.call_method1("explode_curves", (&ids, !s.journalled()))?;
    me.setattr("curves", ids.len())?;
    me.setattr("_ids", prior)?;
    touch(&mesh)?;
    version(scene)
}

fn after_explode_curves(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    if phase == Some("undo") && s.is_direct() {
        let me = running(py).expect("a running step");
        let mesh = scene.getattr("mesh")?;
        let store = mesh.getattr("_store")?;
        for row in me.getattr("_ids")?.try_iter()? {
            let row = row?;
            store.call_method1("set_edge_curve", (row.get_item(0)?, row.get_item(1)?))?;
        }
        touch(&mesh)?;
    }
    version(scene)
}

// ------------------------------------------------------------ tag as you draw

fn run_auto_tag(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let tag = py.get_type::<PyDict>().call1((s.arg(py, 1),))?;
    tag.set_item("id", py.import("core.bim")?.getattr("next_object_id")?.call1((scene,))?)?;
    let mesh = scene.getattr("mesh")?;
    let live = mesh.getattr("faces")?;
    let mut changed = false;
    for fc in s.arg(py, 0).try_iter()? {
        let Some(face) = optional(&fc?, "face")? else { continue };
        let attrs = face.getattr("attrs")?;
        if live.contains(&face)? && !attrs.call_method1("get", ("ifc",))?.is_truthy()? {
            attrs.set_item("ifc", py.get_type::<PyDict>().call1((&tag,))?)?;
            changed = true;
        }
    }
    if changed {
        mesh.call_method0("mark_modified")?;
    }
    Ok(())
}

// ------------------------------------------------------------ push/pull clean-ups

/// The keys of `items`, or None for None (no scan: every element).
fn key_list<'py>(py: Python<'py>, items: &Obj<'py>) -> PyResult<Obj<'py>> {
    if items.is_none() {
        return Ok(none(py));
    }
    Ok(PyList::new(py, items.try_iter()?.map(|i| i?.getattr("key")).collect::<PyResult<Vec<_>>>()?)?.into_any())
}

fn run_prune_orphans(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let me = running(py).expect("a running step");
    let mesh = scene.getattr("mesh")?;
    let vertices = s.arg(py, 0);
    let key = py.import("core.topology")?.getattr("_key")?;
    let keys = PyList::new(py, vertices.try_iter()?.map(|v| key.call1((v?,))).collect::<PyResult<Vec<_>>>()?)?;
    // Only an edge with an end near the vertices can be swept, and only a
    // face with a corner there can still hold it.
    let scans = py.import("core.push_scans")?;
    let edges = scans.call_method1("edges_near_points", (&mesh, &keys))?;
    let faces = scans.call_method1("faces_near_points_rings", (&mesh, &keys))?;
    let selection = scene.getattr("selection")?;
    let removed: Vec<Obj<'_>> = {
        let store = mesh.getattr("_store")?;
        let prune = if s.journalled() { "prune_orphan_edges" } else { "orphaned_edges_at" };
        let rows = PyList::new(py, vertices.try_iter()?.map(|v| v?.call_method0("toTuple")).collect::<PyResult<Vec<_>>>()?)?;
        let found: Vec<u128> = store.call_method1(prune, (rows, key_list(py, &edges)?, key_list(py, &faces)?))?.extract()?;
        let view = py.import("core.store_mesh")?.getattr("EdgeView")?;
        let removed = found.iter().map(|k| view.call1((&mesh, *k))).collect::<PyResult<Vec<_>>>()?;
        if s.journalled() {
            me.setattr("removed", PyList::new(py, &removed)?)?;
            if !found.is_empty() {
                mesh.call_method1("_dirty", (found.len(),))?;
                for edge in &removed {
                    selection.call_method1("discard", (edge,))?;
                }
                version(scene)?;
            }
            return Ok(());
        }
        removed
    };
    for edge in &removed {
        mesh.call_method1("remove_edge", (edge,))?;
        selection.call_method1("discard", (edge,))?;
    }
    me.setattr("removed", PyList::new(py, &removed)?)?;
    if !removed.is_empty() {
        version(scene)?;
    }
    Ok(())
}

fn after_prune_orphans(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    if phase == Some("undo") && s.is_direct() {
        let me = running(py).expect("a running step");
        let removed = me.getattr("removed")?;
        let mesh = scene.getattr("mesh")?;
        let any = removed.is_truthy()?;
        for edge in removed.try_iter()? {
            mesh.call_method1("relink_edge", (edge?,))?;
        }
        me.setattr("removed", PyList::empty(py))?;
        return if any { version(scene) } else { Ok(()) };
    }
    version(scene)
}

fn run_coplanar_merge(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let me = running(py).expect("a running step");
    let mesh = scene.getattr("mesh")?;
    let seeds = PyList::new(py, keys_of(py, &s.arg(py, 0))?.iter())?;
    let merges = match me.getattr("merges")? {
        m if m.is_instance_of::<PyList>() => m,
        _ => {
            let m = PyList::empty(py).into_any();
            me.setattr("merges", &m)?;
            m
        }
    };
    let api = py.import("core.store_mesh")?;
    let record = if s.journalled() { none(py) } else { merges };
    let merged = py
        .import("ingetrazo_core")?
        .getattr("merge_seeded_edges")?
        .call1((&mesh, seeds, record, api.getattr("_store_of")?, api.getattr("EdgeView")?))?;
    if merged.is_truthy()? {
        version(scene)?;
    }
    Ok(())
}

fn after_coplanar_merge(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    if phase == Some("undo") && s.is_direct() {
        let me = running(py).expect("a running step");
        let merges = me.getattr("merges")?;
        let any = merges.is_truthy()?;
        py.import("ingetrazo_core")?.getattr("revert_seeded_merges")?.call1((scene.getattr("mesh")?, &merges))?;
        me.setattr("merges", PyList::empty(py))?;
        return if any { version(scene) } else { Ok(()) };
    }
    version(scene)
}

fn run_stitch(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let me = running(py).expect("a running step");
    let mesh = scene.getattr("mesh")?;
    me.setattr("snapshot", snapshot(s, py, &mesh)?)?;
    host(py, "run_stitch")?.call1((&mesh, keys_of(py, &s.arg(py, 0))?))?;
    version(scene)
}

// ------------------------------------------------------------ simplify

fn run_simplify(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let me = running(py).expect("a running step");
    let mesh = scene.getattr("mesh")?;
    match optional(&me, "after")? {
        Some(after) => {
            mesh.call_method1("restore_state", (after,))?;
        }
        None => {
            me.setattr("before", snapshot(s, py, &mesh)?)?;
            let element_set = py.import("core.element_set")?.getattr("ElementSet")?;
            me.setattr("before_selection", element_set.call1((scene.getattr("selection")?,))?)?;
            let removed = py.import("formats.fuse")?.getattr("simplify_mesh")?.call1((&mesh, s.arg(py, 0)))?;
            me.setattr("faces_removed", removed)?;
            me.setattr("after", snapshot(s, py, &mesh)?)?;
        }
    }
    scene.getattr("selection")?.call_method0("clear")?;
    version(scene)
}

fn after_simplify(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    let me = running(py).expect("a running step");
    let set = py.get_type::<PySet>();
    let undo = phase == Some("undo");
    if undo && s.is_direct() {
        let Some(before) = optional(&me, "before")? else { return Ok(()) };
        scene.getattr("mesh")?.call_method1("restore_state", (before,))?;
    }
    let selection = if undo { set.call1((me.getattr("before_selection")?,))? } else { set.call0()? };
    scene.setattr("selection", selection)?;
    version(scene)
}

// ------------------------------------------------------------ reverse faces

fn face_meshes<'py>(s: &RecordStep, py: Python<'py>, _scene: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for face in s.items(py, 0)? {
        let mesh = face.getattr_opt("_m")?.unwrap_or_else(|| none(py));
        if seen.insert(mesh.as_ptr() as usize) {
            out.push(mesh);
        }
    }
    Ok(out)
}

/// Reverse the faces' windings (an involution: undo flips again). A
/// reversed loop changes no count, coordinate or paint, so every mesh
/// holding a flipped face says so for the group chunks drawing it (#197).
fn flip(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let owners = host(py, "_reverse_faces")?.call1((s.arg(py, 0),))?;
    for mesh in owners.try_iter()? {
        let mesh = mesh?;
        let rev: i64 = mesh.getattr_opt("_winding_rev")?.map_or(Ok(0), |v| v.extract())?;
        mesh.setattr("_winding_rev", rev + 1)?;
    }
    host(py, "_dirty_group_chunks")?.call1((scene,))?;
    version(scene)
}

fn run_flip_faces(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    flip(s, py, scene)
}

fn after_flip_faces(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    if phase == Some("undo") && s.is_direct() {
        return flip(s, py, scene);
    }
    let view = py.import("core.store_mesh")?.getattr("FaceView")?;
    let mut seen = HashSet::new();
    for f in s.items(py, 0)? {
        if f.is_instance(&view)? {
            let mesh = f.getattr("_m")?;
            if seen.insert(mesh.as_ptr() as usize) {
                mesh.call_method0("note_rewound")?;
            }
        }
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
    TagCurve = "TagCurveCommand";
    ExplodeCurves = "ExplodeCurvesCommand";
    AutoTagDrawnFaces = "AutoTagDrawnFacesCommand";
    PruneOrphanEdges = "PruneOrphanEdgesCommand";
    CoplanarMerge = "CoplanarMergeCommand";
    StitchSolid = "StitchSolidCommand";
    SimplifyMesh = "SimplifyMeshCommand";
    FlipFaces = "FlipFacesCommand";
}

fn step(name: &'static str, run: fn(&RecordStep, Python<'_>, &Obj<'_>) -> PyResult<()>, args: Vec<(&'static str, Obj<'_>)>, after: fn(&RecordStep, Python<'_>, &Obj<'_>, Option<&str>) -> PyResult<()>) -> RecordStep {
    RecordStep::new(name, run, args).own_inverse().hook(After(after))
}

fn class_none(py: Python<'_>) -> Py<PyAny> {
    py.None()
}

#[pymethods]
impl TagCurve {
    /// Mark a drawn circle's or arc's edges as one curve (a shared id), so
    /// picking a segment picks the whole curve. Runs last in the draw:
    /// undo is the enclosing snapshot's.
    #[new]
    #[pyo3(signature = (loop_points, closed = true))]
    fn new(py: Python<'_>, loop_points: Obj<'_>, closed: bool) -> PyResult<PyClassInitializer<Self>> {
        let args = vec![("pts", vectors(py, loop_points)?), ("closed", pyo3::types::PyBool::new(py, closed).to_owned().into_any())];
        Ok(PyClassInitializer::from(step("TagCurveCommand", run_tag_curve, args, after_quiet)).add_subclass(Self))
    }
}

#[pymethods]
#[allow(non_upper_case_globals)]
impl ExplodeCurves {
    #[classattr]
    const curves: i64 = 0;
    #[classattr]
    fn _ids(py: Python<'_>) -> Py<PyAny> {
        PyTuple::empty(py).into_any().unbind()
    }

    /// Explode Curve: the curves these edges belong to become plain
    /// segments; the geometry stays as it is. Undo gives the ids back.
    #[new]
    fn new(py: Python<'_>, edges: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let edges = PyList::new(py, edges.try_iter()?.collect::<PyResult<Vec<_>>>()?)?.into_any();
        Ok(PyClassInitializer::from(step("ExplodeCurvesCommand", run_explode_curves, vec![("edges", edges)], after_explode_curves)).add_subclass(Self))
    }
}

#[pymethods]
impl AutoTagDrawnFaces {
    /// Tag as you draw: stamp the active IFC tag, with a fresh object id,
    /// on the faces the draw's own AddFace steps made (a face that
    /// inherited a tag keeps it). Undo is the enclosing snapshot's.
    #[new]
    fn new(py: Python<'_>, face_commands: Obj<'_>, tag: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let cmds = PyList::new(py, face_commands.try_iter()?.collect::<PyResult<Vec<_>>>()?)?.into_any();
        let tag = py.get_type::<PyDict>().call1((tag,))?;
        let args = vec![("_face_cmds", cmds), ("_tag", tag)];
        Ok(PyClassInitializer::from(step("AutoTagDrawnFacesCommand", run_auto_tag, args, after_quiet)).add_subclass(Self))
    }
}

#[pymethods]
impl PruneOrphanEdges {
    #[classattr]
    fn removed(py: Python<'_>) -> Py<PyAny> {
        PyTuple::empty(py).into_any().unbind()
    }

    /// Remove the edges at `vertices` that, once the rest of a compound
    /// ran, border no face: the dangling lines a push/pull carve leaves.
    #[new]
    fn new(py: Python<'_>, vertices: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        Ok(PyClassInitializer::from(step("PruneOrphanEdgesCommand", run_prune_orphans, vec![("vertices", vectors(py, vertices)?)], after_prune_orphans)).add_subclass(Self))
    }
}

#[pymethods]
impl CoplanarMerge {
    #[classattr]
    fn merges(py: Python<'_>) -> Py<PyAny> {
        class_none(py)
    }

    /// Dissolve the coplanar seams a push/pull left around its own
    /// vertices; a line drawn on purpose elsewhere stays.
    #[new]
    fn new(py: Python<'_>, seed_positions: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        Ok(PyClassInitializer::from(step("CoplanarMergeCommand", run_coplanar_merge, vec![("seed", vectors(py, seed_positions)?)], after_coplanar_merge)).add_subclass(Self))
    }
}

#[pymethods]
impl StitchSolid {
    #[classattr]
    fn snapshot(py: Python<'_>) -> Py<PyAny> {
        class_none(py)
    }

    /// Make a solid watertight again after push/pull: T-junctions split,
    /// collinear vertices collapsed, coplanar faces around it fused.
    #[new]
    fn new(py: Python<'_>, seed_positions: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        Ok(PyClassInitializer::from(step("StitchSolidCommand", run_stitch, vec![("seed", vectors(py, seed_positions)?)], after_snapshot)).add_subclass(Self))
    }
}

#[pymethods]
#[allow(non_upper_case_globals)]
impl SimplifyMesh {
    #[classattr]
    const faces_removed: i64 = 0;
    #[classattr]
    fn before(py: Python<'_>) -> Py<PyAny> {
        class_none(py)
    }
    #[classattr]
    fn after(py: Python<'_>) -> Py<PyAny> {
        class_none(py)
    }
    #[classattr]
    fn before_selection(py: Python<'_>) -> Py<PyAny> {
        PyTuple::empty(py).into_any().unbind()
    }

    /// Merge the active mesh's near-coplanar faces (within
    /// `max_angle_degrees`).
    #[new]
    #[pyo3(signature = (max_angle_degrees = 0.0))]
    fn new(py: Python<'_>, max_angle_degrees: f64) -> PyResult<PyClassInitializer<Self>> {
        let args = vec![("max_angle_degrees", max_angle_degrees.into_pyobject(py)?.into_any())];
        Ok(PyClassInitializer::from(step("SimplifyMeshCommand", run_simplify, args, after_simplify)).add_subclass(Self))
    }
}

#[pymethods]
impl FlipFaces {
    /// Reverse Faces: flip the winding of these faces, the same face
    /// objects kept (an involution, so undo flips again). Faces in the
    /// loose mesh or inside a group's mesh.
    #[new]
    fn new(py: Python<'_>, faces: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let face_type = py.import("core.mesh")?.getattr("Face")?;
        let mut kept = Vec::new();
        for f in faces.try_iter()? {
            let f = f?;
            if f.is_instance(&face_type)? || f.hasattr("loop")? {
                kept.push(f);
            }
        }
        let step = step("FlipFacesCommand", run_flip_faces, vec![("_faces", PyList::new(py, kept)?.into_any())], after_flip_faces)
            .rows()
            .on_meshes(Meshes(face_meshes));
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }

    /// The store meshes holding the faces.
    fn meshes<'py>(slf: PyRef<'py, Self>) -> PyResult<Bound<'py, PySet>> {
        let py = slf.py();
        let s: &RecordStep = &*slf.as_super();
        let view = py.import("core.store_mesh")?.getattr("FaceView")?;
        let mut out = Vec::new();
        for f in s.items(py, 0)? {
            if f.is_instance(&view)? {
                out.push(f.getattr("_m")?);
            }
        }
        PySet::new(py, out)
    }
}
