// SPDX-License-Identifier: GPL-3.0-or-later
//! Native vertex edits (docs/commands-port.md): move, flip, rotate and
//! scale shared vertices, then fold the faces they warped.
//!
//! On a store mesh the kernel's entry points (`command_move_vertices`,
//! `command_transform_vertices`) move, weld, carry textures and fold in one
//! atomic edit; a move that would leave a topology defect writes nothing and
//! the step refuses it. Otherwise the object kernels edit and the step keeps
//! the mesh's before and after snapshots (when History does not journal it),
//! as the Python commands did.
use super::steps::{running, version, After, Obj, RecordStep};
use crate::store::command_facade::{command_move_vertices, command_transform_vertices, MovedVertices};
use pyo3::prelude::*;
use pyo3::types::{PyFloat, PyList, PySet, PyTuple};
use pyo3::PyClassInitializer;

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}

fn host<'py>(py: Python<'py>, name: &str) -> PyResult<Obj<'py>> {
    py.import("core.history")?.getattr(name)
}

fn topology<'py>(py: Python<'py>, name: &str) -> PyResult<Obj<'py>> {
    py.import("core.topology")?.getattr(name)
}

fn is_store(py: Python<'_>, mesh: &Obj<'_>) -> PyResult<bool> {
    Ok(mesh.get_type().is(&py.import("core.store_mesh")?.getattr("StoreMesh")?))
}

/// A position as given: a Vec3 is copied, a tuple kept (its integer
/// coordinates decide the store path, as `_key` keeps them integers).
fn position<'py>(py: Python<'py>, p: Obj<'py>) -> PyResult<Obj<'py>> {
    if p.hasattr("toTuple")? {
        super::steps::vector(py, &p)
    } else {
        Ok(PyTuple::new(py, p.try_iter()?.collect::<PyResult<Vec<_>>>()?)?.into_any())
    }
}

fn positions<'py>(py: Python<'py>, src: Obj<'py>) -> PyResult<Obj<'py>> {
    let items = src.try_iter()?.map(|p| position(py, p?)).collect::<PyResult<Vec<_>>>()?;
    Ok(PyList::new(py, items)?.into_any())
}

/// The raw points, or None when one has a coordinate that is not a float:
/// the store facade reads integer cells as already scaled, so those edits
/// keep the object way.
fn float_points(src: &Obj<'_>) -> PyResult<Option<Vec<[f64; 3]>>> {
    let mut out = Vec::new();
    for p in src.try_iter()? {
        let p = p?;
        let row = match p.getattr_opt("toTuple")? {
            Some(f) => f.call0()?,
            None => p,
        };
        let coords: Vec<Obj<'_>> = row.extract()?;
        if coords.iter().any(|n| !n.is_exact_instance_of::<PyFloat>()) {
            return Ok(None);
        }
        out.push(row.extract()?);
    }
    Ok(Some(out))
}

fn keys_of<'py>(py: Python<'py>, points: &[Obj<'py>]) -> PyResult<Bound<'py, PySet>> {
    let key = topology(py, "_key")?;
    PySet::new(py, points.iter().map(|p| key.call1((p,))).collect::<PyResult<Vec<_>>>()?)
}

/// The mesh before (None when journalled), remembered at memory slot 0.
fn snapshot_before(s: &RecordStep, py: Python<'_>, mesh: &Obj<'_>) -> PyResult<()> {
    let before = if s.journalled() { none(py) } else { mesh.call_method0("capture_state")? };
    s.remember(vec![before, none(py)]);
    Ok(())
}

fn snapshot_after(s: &RecordStep, py: Python<'_>, mesh: &Obj<'_>) -> PyResult<()> {
    let before = s.remembered(py, 0).unwrap_or_else(|| none(py));
    let after = if s.journalled() { none(py) } else { mesh.call_method0("capture_state")? };
    s.remember(vec![before, after]);
    Ok(())
}

/// A direct redo of the object way puts back what the first run made.
fn redo_direct(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<bool> {
    match s.remembered(py, 1).filter(|a| !a.is_none()) {
        Some(after) => {
            scene.getattr("mesh")?.call_method1("restore_state", (after,))?;
            version(scene)?;
            Ok(true)
        }
        None => Ok(false),
    }
}

fn after_vertices(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    if phase == Some("undo") && s.is_direct() {
        if let Some(before) = s.remembered(py, 0).filter(|b| !b.is_none()) {
            scene.getattr("mesh")?.call_method1("restore_state", (before,))?;
            return version(scene);
        }
        return Ok(());
    }
    version(scene)
}

/// Refuse the edit: what it would leave, in plain words.
fn reject(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, reason: &Obj<'_>) -> PyResult<()> {
    if let Some(before) = s.remembered(py, 0).filter(|b| !b.is_none()) {
        scene.getattr("mesh")?.call_method1("restore_state", (before,))?;
    }
    version(scene)?;
    running(py).expect("a running step").setattr("welded", false)?;
    let defect = host(py, "defect_names")?.call0()?.get_item(reason)?;
    let kw = pyo3::types::PyDict::new(py);
    kw.set_item("defect", defect)?;
    let message = py
        .import("core.i18n")?
        .getattr("tr")?
        .call(("Not joined: it would leave {defect}. The move was undone.",), Some(&kw))?;
    Err(PyErr::from_value(host(py, "CommandRejected")?.call1((message,))?))
}

// ------------------------------------------------------------ move

fn run_move(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    if redo_direct(s, py, scene)? {
        return Ok(());
    }
    let me = running(py).expect("a running step");
    let mesh = scene.getattr("mesh")?;
    let (src, delta, carry) = (s.arg(py, 0), s.arg(py, 1), s.arg(py, 2).is_truthy()?);
    snapshot_before(s, py, &mesh)?;
    me.setattr("welded", false)?;
    let d: [f64; 3] = crate::math::xyz(&super::steps::vector(py, &delta)?)?.into();
    let moved = match float_points(&src)? {
        Some(points) => command_move_vertices(&mesh, points, d, carry)?,
        None => None,
    };
    match moved {
        Some(MovedVertices { landed, defects, .. }) => {
            version(scene)?;
            if let Some(reason) = defects.first() {
                return reject(s, py, scene, &pyo3::types::PyString::new(py, reason).into_any());
            }
            me.setattr("welded", !landed.is_empty())?;
        }
        None => move_objects(s, py, scene, &mesh, &src, &delta, carry)?,
    }
    snapshot_after(s, py, &mesh)
}

/// The object way, as the Python command ran it.
fn move_objects(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, mesh: &Obj<'_>, src: &Obj<'_>, delta: &Obj<'_>, carry: bool) -> PyResult<()> {
    let me = running(py).expect("a running step");
    let delta = super::steps::vector(py, delta)?;
    let points: Vec<Obj<'_>> = src.try_iter()?.map(|p| super::steps::vector(py, &p?)).collect::<PyResult<_>>()?;
    let landing_points = points.iter().map(|p| p.add(&delta)).collect::<PyResult<Vec<_>>>()?;
    let keys = keys_of(py, &points)?;
    let landing = keys_of(py, &landing_points)?;
    let at = host(py, "_vertices_at")?;
    let vertices = at.call1((mesh, &keys))?;
    let moving = PySet::new(py, vertices.try_iter()?.map(|v| v?.getattr("key")).collect::<PyResult<Vec<_>>>()?)?;
    let mut receiving = false;
    for v in at.call1((mesh, &landing))?.try_iter()? {
        if !moving.contains(v?.getattr("key")?)? {
            receiving = true;
            break;
        }
    }
    let defects_near = topology(py, "defects_near")?;
    let before = if receiving { defects_near.call1((at.call1((mesh, keys.call_method1("union", (&landing,))?))?,))? } else { none(py) };
    for v in vertices.try_iter()? {
        mesh.call_method1("move_vertex", (v?, &delta))?;
    }
    version(scene)?;
    if carry && moving.len() > 0 {
        let carried = host(py, "_carried_faces")?.call1((mesh, &moving))?;
        if carried.is_truthy()? {
            let transform = py.import("ingetrazo_core")?.getattr("Matrix4")?.call0()?;
            transform.call_method1("translate", (&delta,))?;
            py.import("core.group")?.getattr("_remap_uvws")?.call1((mesh, transform, carried))?;
        }
    }
    let welded = host(py, "_weld_landed")?.call1((mesh, moving.call_method0("copy")?))?.is_truthy()?;
    me.setattr("welded", welded)?;
    let kw = pyo3::types::PyDict::new(py);
    kw.set_item("moved", &landing)?;
    topology(py, "fold_nonplanar_faces")?.call((mesh,), Some(&kw))?;
    if welded && !before.is_none() {
        let worse = topology(py, "new_defects")?.call1((&before, defects_near.call1((at.call1((mesh, &landing))?,))?))?;
        if worse.is_truthy()? {
            return reject(s, py, scene, &worse.get_item(0)?);
        }
    }
    Ok(())
}

// ------------------------------------------------------------ transforms

/// Memory beyond the snapshots is not needed: the matrix is a pure
/// function of the arguments (`matrix()`), asked of the step itself.
fn run_transform(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    if redo_direct(s, py, scene)? {
        return Ok(());
    }
    let me = running(py).expect("a running step");
    let mesh = scene.getattr("mesh")?;
    snapshot_before(s, py, &mesh)?;
    let src = s.arg(py, 0);
    let carry = me.getattr("carry_textures")?.is_truthy()?;
    let matrix = me.call_method0("matrix")?;
    let matrix4 = py.import("ingetrazo_core")?.getattr("Matrix4")?;
    let mut done = false;
    if is_store(py, &mesh)? && matrix.get_type().is(&matrix4) {
        if let Some(points) = float_points(&src)? {
            let data: [f64; 16] = matrix.call_method0("data")?.extract()?;
            done = command_transform_vertices(&mesh, points, data, carry)?.is_some();
            if done {
                version(scene)?;
            }
        }
    }
    if !done {
        let points: Vec<Obj<'_>> = src.try_iter()?.map(|p| p).collect::<PyResult<_>>()?;
        host(py, "_map_selected_vertices")?.call1((scene, keys_of(py, &points)?, &matrix, carry))?;
    }
    if let Some(faces) = me.getattr_opt("_faces")?.filter(|f| f.is_truthy().unwrap_or(false)) {
        let face_type = py.import("core.mesh")?.getattr("Face")?;
        let mut picked = Vec::new();
        for f in faces.try_iter()? {
            let f = f?;
            if f.is_instance(&face_type)? || f.hasattr("loop")? {
                picked.push(f);
            }
        }
        host(py, "_reverse_faces")?.call1((PyList::new(py, picked)?,))?;
    }
    topology(py, "fold_nonplanar_faces")?.call1((&mesh,))?;
    snapshot_after(s, py, &mesh)
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
    MoveVertices = "MoveVerticesCommand";
    FlipVertices = "FlipVerticesCommand";
    RotateVertices = "RotateVerticesCommand";
    ScaleVertices = "ScaleVerticesCommand";
}

fn vertex_step(name: &'static str, run: fn(&RecordStep, Python<'_>, &Obj<'_>) -> PyResult<()>, args: Vec<(&'static str, Obj<'_>)>) -> RecordStep {
    RecordStep::new(name, run, args).own_inverse().hook(After(after_vertices))
}

fn bool_obj(py: Python<'_>, b: bool) -> Obj<'_> {
    pyo3::types::PyBool::new(py, b).to_owned().into_any()
}

#[pymethods]
impl MoveVertices {
    /// Move shared vertices, carry whole-face textures and fold warped
    /// faces. A snap that introduces a local topology defect is refused.
    #[new]
    #[pyo3(signature = (positions, delta, carry_textures = false))]
    fn new(py: Python<'_>, positions: Obj<'_>, delta: Obj<'_>, carry_textures: bool) -> PyResult<PyClassInitializer<Self>> {
        let delta = position(py, delta)?;
        let args = vec![("src", self::positions(py, positions)?), ("delta", delta), ("carry_textures", bool_obj(py, carry_textures))];
        let step = vertex_step("MoveVerticesCommand", run_move, args).rows();
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}

#[pymethods]
impl FlipVertices {
    /// Mirror loose positions about an axis plane and re-reverse the fully
    /// selected faces' windings.
    #[new]
    #[pyo3(signature = (positions, centre, axis, faces = None))]
    fn new(py: Python<'_>, positions: Obj<'_>, centre: Obj<'_>, axis: Obj<'_>, faces: Option<Obj<'_>>) -> PyResult<PyClassInitializer<Self>> {
        let faces = match faces {
            Some(f) => PyList::new(py, f.try_iter()?.collect::<PyResult<Vec<_>>>()?)?.into_any(),
            None => PyList::empty(py).into_any(),
        };
        let args = vec![
            ("src", self::positions(py, positions)?),
            ("centre", super::steps::vector(py, &centre)?),
            ("axis", super::steps::vector(py, &axis)?),
            ("_faces", faces),
            ("carry_textures", bool_obj(py, false)),
        ];
        Ok(PyClassInitializer::from(vertex_step("FlipVerticesCommand", run_transform, args)).add_subclass(Self))
    }

    fn matrix<'py>(slf: PyRef<'py, Self>) -> PyResult<Obj<'py>> {
        let py = slf.py();
        let step: &RecordStep = &*slf.as_super();
        host(py, "mirror_matrix")?.call1((step.arg(py, 1), step.arg(py, 2)))
    }
}

#[pymethods]
impl RotateVertices {
    /// Rotate shared vertices around `axis` through `center` by `degrees`,
    /// then fold the faces it warped.
    #[new]
    #[pyo3(signature = (positions, center, axis, degrees, carry_textures = false))]
    fn new(py: Python<'_>, positions: Obj<'_>, center: Obj<'_>, axis: Obj<'_>, degrees: Obj<'_>, carry_textures: bool) -> PyResult<PyClassInitializer<Self>> {
        let args = vec![
            ("src", self::positions(py, positions)?),
            ("center", super::steps::vector(py, &center)?),
            ("axis", super::steps::vector(py, &axis)?),
            ("degrees", degrees),
            ("carry_textures", bool_obj(py, carry_textures)),
        ];
        Ok(PyClassInitializer::from(vertex_step("RotateVerticesCommand", run_transform, args)).add_subclass(Self))
    }

    fn matrix<'py>(slf: PyRef<'py, Self>) -> PyResult<Obj<'py>> {
        let py = slf.py();
        let step: &RecordStep = &*slf.as_super();
        host(py, "rotation_matrix")?.call1((step.arg(py, 1), step.arg(py, 2), step.arg(py, 3)))
    }
}

#[pymethods]
impl ScaleVertices {
    /// Scale shared vertices about `center` by `factor` (a float, or one
    /// per axis along `axes`), then fold the faces it warped.
    #[new]
    #[pyo3(signature = (positions, center, factor, axes = None))]
    fn new(py: Python<'_>, positions: Obj<'_>, center: Obj<'_>, factor: Obj<'_>, axes: Option<Obj<'_>>) -> PyResult<PyClassInitializer<Self>> {
        let args = vec![
            ("src", self::positions(py, positions)?),
            ("center", super::steps::vector(py, &center)?),
            ("factor", factor),
            ("axes", axes.unwrap_or_else(|| none(py))),
            ("carry_textures", bool_obj(py, false)),
        ];
        Ok(PyClassInitializer::from(vertex_step("ScaleVerticesCommand", run_transform, args)).add_subclass(Self))
    }

    fn matrix<'py>(slf: PyRef<'py, Self>) -> PyResult<Obj<'py>> {
        let py = slf.py();
        let step: &RecordStep = &*slf.as_super();
        host(py, "scale_matrix")?.call1((step.arg(py, 1), step.arg(py, 2), step.arg(py, 3)))
    }
}
