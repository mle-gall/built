// SPDX-License-Identifier: GPL-3.0-or-later
//! Native group transform steps: move, rotate,
//! scale and flip whole groups, and change a group's axes.
//!
//! An instance moves by its matrix, a document field: the step's
//! transaction replays it exactly. A classic group moves its own mesh:
//! History journals the store, else the step's own inverse applies (the
//! negated delta, the `GroupTransform` / `AxesEdit` snapshots, the second
//! reflection). A direct undo first replays the step's transaction, so
//! the explicit inverse only touches what the transaction does not hold.
use super::steps::{rotation, vector, version, After, Meshes, Obj, RecordStep};
use glam::DMat4;
use pyo3::prelude::*;
use pyo3::types::{PyCFunction, PyList};
use pyo3::PyClassInitializer;

fn is_instance(g: &Obj<'_>) -> PyResult<bool> {
    Ok(!g.getattr("xform")?.is_none())
}


fn kernel<'py>(py: Python<'py>, class: &str) -> PyResult<Obj<'py>> {
    py.import("built_core")?.getattr("placement_transform")?.getattr(class)?.call0()
}

/// The classic group's own mesh, for History to enlist; none for an instance.
fn group_meshes<'py>(s: &RecordStep, py: Python<'py>, _scene: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    let g = s.arg(py, 0);
    Ok(if is_instance(&g)? { Vec::new() } else { vec![g.getattr("mesh")?] })
}

/// A Python callable `make(reverse)` over two fixed matrices, as
/// `GroupTransform.apply` takes it.
fn matrices<'py>(py: Python<'py>, forward: Obj<'py>, backward: Obj<'py>) -> PyResult<Obj<'py>> {
    let (f, b) = (forward.unbind(), backward.unbind());
    Ok(PyCFunction::new_closure(py, None, None, move |args, _| -> PyResult<Py<PyAny>> {
        let py = args.py();
        let reverse: bool = args.get_item(0)?.extract()?;
        Ok(if reverse { b.clone_ref(py) } else { f.clone_ref(py) })
    })?
    .into_any())
}

// ------------------------------------------------------------ move

fn run_move(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let g = s.arg(py, 0);
    s.remember(vec![g.getattr("axes")?]);
    crate::placement_transform::shift_group(scene, &g, &s.arg(py, 1))?;
    s.set_undo_by_transaction(is_instance(&g)?);
    version(scene)
}

fn after_move(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    let g = s.arg(py, 0);
    if phase == Some("undo") && s.is_direct() && !is_instance(&g)? {
        let back = s.arg(py, 1).call_method0("__neg__")?;
        crate::placement_transform::shift_group(scene, &g, &back)?;
        if let Some(axes) = s.remembered(py, 0) {
            g.setattr("axes", axes)?;
        }
    }
    version(scene)
}

// ------------------------------------------------------------ rotate, scale

fn run_pose(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let g = s.arg(py, 0);
    let transform = s.arg(py, 2);
    if transform.call_method1("apply", (scene, &g, s.arg(py, 1), false, s.journalled()))?.is_truthy()? {
        version(scene)?;
    }
    s.set_undo_by_transaction(is_instance(&g)?);
    Ok(())
}

fn after_pose(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    let g = s.arg(py, 0);
    if phase == Some("undo") && s.is_direct() && !is_instance(&g)? {
        let transform = s.arg(py, 2);
        if transform.call_method1("apply", (scene, &g, s.arg(py, 1), true, s.journalled()))?.is_truthy()? {
            return version(scene);
        }
        return Ok(());
    }
    version(scene)
}

fn pose_step(name: &'static str, group: Obj<'_>, maker: Obj<'_>, extra: Vec<(&'static str, Obj<'_>)>) -> PyResult<RecordStep> {
    let py = group.py();
    let undo_by_transaction = is_instance(&group)?;
    let mut args = vec![("group", group), ("_matrix", maker), ("_transform", kernel(py, "GroupTransform")?)];
    args.extend(extra);
    Ok(RecordStep::new(name, run_pose, args)
        .paint()
        .own_inverse_if(!undo_by_transaction)
        .on_meshes(Meshes(group_meshes))
        .hook(After(after_pose)))
}

// ------------------------------------------------------------ flip

/// The reflection about the plane through `centre` with normal `axis`.
fn mirror<'py>(py: Python<'py>, centre: &Obj<'py>, axis: &Obj<'py>) -> PyResult<Obj<'py>> {
    let c = crate::math::xyz(centre)?;
    let n = crate::math::xyz(axis)?.normalize_or_zero();
    let r = DMat4::from_cols_array(&[
        1. - 2. * n.x * n.x, -2. * n.y * n.x, -2. * n.z * n.x, 0.,
        -2. * n.x * n.y, 1. - 2. * n.y * n.y, -2. * n.z * n.y, 0.,
        -2. * n.x * n.z, -2. * n.y * n.z, 1. - 2. * n.z * n.z, 0.,
        0., 0., 0., 1.,
    ]);
    let m = DMat4::from_translation(c) * r * DMat4::from_translation(-c);
    Ok(Bound::new(py, crate::math::Matrix4::from(m))?.into_any())
}

fn classic<'py>(groups: &[Obj<'py>]) -> PyResult<Vec<Obj<'py>>> {
    let mut out = Vec::new();
    for g in groups {
        if !is_instance(g)? {
            out.push(g.clone());
        }
    }
    Ok(out)
}

fn flip_meshes<'py>(s: &RecordStep, py: Python<'py>, _scene: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    classic(&s.items(py, 0)?)?.iter().map(|g| g.getattr("mesh")).collect()
}

fn run_flip(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let groups = s.items(py, 0)?;
    crate::placement_transform::flip_groups(scene, groups.clone(), &s.arg(py, 1))?;
    let instances = classic(&groups)?.is_empty();
    s.set_undo_by_transaction(instances);
    if instances {
        crate::doc::history_support::dirty_placements(&s.arg(py, 0))?;
    } else {
        crate::doc::history_support::dirty_group_chunks(scene)?;
    }
    version(scene)
}

fn after_flip(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    if phase == Some("undo") && s.is_direct() {
        // A reflection is an involution; the transaction gave the
        // instances their matrices back already.
        let back = classic(&s.items(py, 0)?)?;
        if !back.is_empty() {
            crate::placement_transform::flip_groups(scene, back, &s.arg(py, 1))?;
            crate::doc::history_support::dirty_group_chunks(scene)?;
        }
    }
    crate::doc::history_support::dirty_placements(&s.arg(py, 0))?;
    version(scene)
}

// ------------------------------------------------------------ axes

fn axes_meshes<'py>(s: &RecordStep, py: Python<'py>, _scene: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    let g = s.arg(py, 0);
    Ok(if is_instance(&g)? { vec![g.getattr("mesh")?] } else { Vec::new() })
}

/// Rebuild the live occurrence view after its definition basis changed.
fn refresh_open_axes(scene: &Obj<'_>, target: &Obj<'_>) -> PyResult<()> {
    let open = scene.getattr("edit_group")?;
    if open.is_none() || !open.getattr("mesh")?.is(&target.getattr("mesh")?) {
        return Ok(());
    }
    let level = scene.getattr("_edit_stack")?.get_item(-1)?;
    let py = scene.py();
    let mut frame = py
        .import("built_core.group")?
        .call_method1("group_frame", (&open,))?;
    if level.get_item("affine")?.is_truthy()? {
        let mut world = DMat4::IDENTITY;
        for row in scene.getattr("_edit_stack")?.try_iter()? {
            let pose = row?.get_item("group")?.getattr("xform")?;
            if !pose.is_none() {
                world *= crate::math::mat(&pose)?;
            }
        }
        let origin = scene
            .getattr("mesh")?
            .getattr("_store")?
            .getattr("frame_origin")?;
        let view = py.import("built_core.store_mesh")?.call_method1(
            "instance_view",
            (open.getattr("mesh")?, world.to_cols_array(), origin, true),
        )?;
        frame = view.getattr("_store")?.call_method0("tool_placement")?;
        let axes = open.getattr("axes")?;
        if !axes.is_none() {
            frame = frame.mul(axes)?;
        }
        scene.setattr("mesh", view)?;
    }
    level.set_item("frame", frame)
}

fn run_axes(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let edit = s.arg(py, 2);
    if edit.call_method1("apply", (scene, s.arg(py, 0), s.arg(py, 1)))?.is_truthy()? {
        refresh_open_axes(scene, &s.arg(py, 0))?;
        version(scene)?;
    }
    // A classic group swaps its axes (a field); an instance re-expresses
    // its shared definition's mesh: its own undo.
    s.set_undo_by_transaction(edit.call_method0("fields_only")?.is_truthy()?);
    Ok(())
}

fn after_axes(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    let edit = s.arg(py, 2);
    if phase == Some("undo") && s.is_direct() && !edit.call_method0("fields_only")?.is_truthy()? {
        if edit.call_method1("undo", (s.arg(py, 0),))?.is_truthy()? {
            refresh_open_axes(scene, &s.arg(py, 0))?;
            return version(scene);
        }
        return Ok(());
    }
    refresh_open_axes(scene, &s.arg(py, 0))?;
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
    MoveGroup = "MoveGroupCommand";
    RotateGroup = "RotateGroupCommand";
    ScaleGroup = "ScaleGroupCommand";
    FlipGroups = "FlipGroupsCommand";
    DrawingAxes = "DrawingAxesCommand";
    ChangeAxes = "ChangeAxesCommand";
}

#[pymethods]
impl MoveGroup {
    /// Translate a whole group by `delta`: an instance by its matrix, a
    /// classic group by every vertex of its isolated mesh.
    #[new]
    fn new(py: Python<'_>, group: Obj<'_>, delta: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let undo_by_transaction = is_instance(&group)?;
        let step = RecordStep::new("MoveGroupCommand", run_move, vec![("group", group), ("delta", vector(py, &delta)?)])
            .paint()
            .own_inverse_if(!undo_by_transaction)
            .on_meshes(Meshes(group_meshes))
            .hook(After(after_move));
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}

#[pymethods]
impl RotateGroup {
    /// Rotate a whole group about `center` / `axis` (rigid).
    #[new]
    fn new(py: Python<'_>, group: Obj<'_>, center: Obj<'_>, axis: Obj<'_>, degrees: f64) -> PyResult<PyClassInitializer<Self>> {
        let (center, axis) = (vector(py, &center)?, vector(py, &axis)?);
        let maker = matrices(py, rotation(py, &center, &axis, degrees)?, rotation(py, &center, &axis, -degrees)?)?;
        let extra = vec![("center", center), ("axis", axis), ("degrees", degrees.into_pyobject(py)?.into_any())];
        Ok(PyClassInitializer::from(pose_step("RotateGroupCommand", group, maker, extra)?).add_subclass(Self))
    }
}

#[pymethods]
impl ScaleGroup {
    /// Scale a whole group about `center` (per axis along `axes` when given).
    #[new]
    #[pyo3(signature = (group, center, factor, axes = None))]
    fn new(py: Python<'_>, group: Obj<'_>, center: Obj<'_>, factor: Obj<'_>, axes: Option<Obj<'_>>) -> PyResult<PyClassInitializer<Self>> {
        let center = vector(py, &center)?;
        let axes = axes.unwrap_or_else(|| py.None().into_bound(py));
        use super::transform_math::{invert_scale_factor, scale_matrix};
        let forward = scale_matrix(py, &center, &factor, Some(&axes))?;
        let back = scale_matrix(py, &center, &invert_scale_factor(py, &factor)?, Some(&axes))?;
        let extra = vec![("center", center), ("factor", factor), ("axes", axes)];
        Ok(PyClassInitializer::from(pose_step("ScaleGroupCommand", group, matrices(py, forward, back)?, extra)?).add_subclass(Self))
    }
}

#[pymethods]
impl FlipGroups {
    /// Flip whole groups about an axis plane (the Flip tool).
    #[new]
    fn new(py: Python<'_>, groups: Obj<'_>, centre: Obj<'_>, axis: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let groups = PyList::new(py, groups.try_iter()?.collect::<PyResult<Vec<_>>>()?)?;
        let (centre, axis) = (vector(py, &centre)?, vector(py, &axis)?);
        let m = mirror(py, &centre, &axis)?;
        let instances = classic(&groups.iter().collect::<Vec<_>>())?.is_empty();
        let args = vec![("_groups", groups.into_any()), ("_mirror", m), ("centre", centre), ("axis", axis)];
        let step = RecordStep::new("FlipGroupsCommand", run_flip, args)
            .paint()
            .own_inverse_if(!instances)
            .on_meshes(Meshes(flip_meshes))
            .hook(After(after_flip));
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }

    /// The classic groups' meshes the flip maps (what the feed reads).
    fn meshes<'py>(slf: PyRef<'py, Self>) -> PyResult<Bound<'py, PyList>> {
        let py = slf.py();
        let step: &RecordStep = &*slf.as_super();
        PyList::new(py, flip_meshes(step, py, &py.None().into_bound(py))?)
    }
}

fn set_world_axes(step: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    scene.setattr("world_drawing_frame", step.arg(py, 1))?;
    version(scene)
}
fn replay_world_axes(
    step: &RecordStep,
    py: Python<'_>,
    scene: &Obj<'_>,
    phase: Option<&str>,
) -> PyResult<()> {
    scene.setattr(
        "world_drawing_frame",
        step.arg(py, usize::from(phase != Some("undo"))),
    )?;
    version(scene)
}
#[pymethods]
impl DrawingAxes {
    #[new]
    fn new(before: Obj<'_>, after: Obj<'_>) -> PyClassInitializer<Self> {
        let step = RecordStep::new(
            "DrawingAxesCommand",
            set_world_axes,
            vec![("before", before), ("after", after)],
        )
        .rows()
        .explicit_inverse()
        .hook(After(replay_world_axes));
        PyClassInitializer::from(step).add_subclass(Self)
    }
}

#[pymethods]
impl ChangeAxes {
    /// Change Axes: a new frame for `group` while nothing moves in the world.
    #[new]
    fn new(py: Python<'_>, group: Obj<'_>, frame: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let frame = py.import("built_core")?.getattr("Matrix4")?.call1((frame,))?;
        let args = vec![("group", group), ("frame", frame), ("_axes_edit", kernel(py, "AxesEdit")?)];
        let step = RecordStep::new("ChangeAxesCommand", run_axes, args)
            .on_meshes(Meshes(axes_meshes))
            .hook(After(after_axes));
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}

