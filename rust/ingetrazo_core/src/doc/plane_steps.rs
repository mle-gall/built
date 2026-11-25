// SPDX-License-Identifier: GPL-3.0-or-later
//! Native plane rebuilds (docs/commands-port.md): one plane's faces from
//! its edges, a whole flat drawing, and the overlap heal.
//!
//! On a store mesh the kernel's entry points (`command_rebuild_plane`,
//! `command_rebuild_planar`, `command_heal_overlaps`) rebuild in Rust and a
//! journalling History replays the store. Otherwise the object kernels
//! rebuild, and the step keeps a snapshot of the mesh from before: its
//! real inverse.
use super::steps::{running, version, After, Obj, RecordStep};
use crate::store::command_facade::{command_heal_overlaps, command_rebuild_plane, command_rebuild_planar};
use pyo3::prelude::*;
use pyo3::types::{PyList, PyTuple};
use pyo3::PyClassInitializer;

/// What `RebuildPlaneFacesCommand._on_plane` accepted (and the explode
/// merge reads as the class's `_TOL`).
const TOL: f64 = 1e-4;

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}

fn planar<'py>(py: Python<'py>, name: &str) -> PyResult<Obj<'py>> {
    py.import("ingetrazo_core")?.getattr("planar_edits")?.getattr(name)
}

fn topology<'py>(py: Python<'py>, name: &str) -> PyResult<Obj<'py>> {
    py.import("core.topology")?.getattr(name)
}

fn is_store(py: Python<'_>, mesh: &Obj<'_>) -> PyResult<bool> {
    Ok(mesh.get_type().is(&py.import("core.store_mesh")?.getattr("StoreMesh")?))
}

fn face_views<'py>(py: Python<'py>, mesh: &Obj<'py>, keys: &[u128]) -> PyResult<Vec<Obj<'py>>> {
    let view = py.import("core.store_mesh")?.getattr("FaceView")?;
    keys.iter().map(|k| view.call1((mesh, *k))).collect()
}

/// The mesh before, at memory slot 0 (None when History journals it or
/// the caller holds its own).
fn keep_before(s: &RecordStep, py: Python<'_>, mesh: &Obj<'_>, wanted: bool) -> PyResult<()> {
    let before = if wanted && !s.journalled() { mesh.call_method0("capture_state")? } else { none(py) };
    s.remember(vec![before]);
    Ok(())
}

/// A direct undo puts the mesh back; a transaction replay only bumps the
/// version.
fn after_snapshot(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    if phase == Some("undo") && s.is_direct() {
        if let Some(before) = s.remembered(py, 0).filter(|b| !b.is_none()) {
            scene.getattr("mesh")?.call_method1("restore_state", (before,))?;
            return version(scene);
        }
        return Ok(());
    }
    version(scene)
}

fn clear_selection(scene: &Obj<'_>) -> PyResult<()> {
    scene.getattr("selection")?.call_method0("clear")?;
    Ok(())
}

// ------------------------------------------------------------ one plane

fn run_plane(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let me = running(py).expect("a running step");
    let mesh = scene.getattr("mesh")?;
    keep_before(s, py, &mesh, me.getattr("take_snapshot")?.is_truthy()?)?;
    let (origin, normal, faces) = (s.arg(py, 0), s.arg(py, 1), s.arg(py, 2));
    let n = crate::math::xyz(&normal)?;
    if n.length_squared() < 0.5 {
        // No plane, nothing to rebuild: an on-plane test of a zero normal
        // says yes to every point, and a 3 mm² sliver erased 447 faces.
        return Ok(());
    }
    let keys = if faces.is_none() {
        None
    } else {
        Some(faces.try_iter()?.map(|f| f?.getattr("key")?.extract::<u128>()).collect::<PyResult<Vec<_>>>())
    };
    // Object faces have no store key: the object way rebuilds them.
    let rebuilt = match keys {
        None => command_rebuild_plane(&mesh, crate::math::xyz(&origin)?.into(), n.into(), None)?.map(|r| r.ran.then_some(r.rebuilt)),
        Some(Ok(keys)) => command_rebuild_plane(&mesh, crate::math::xyz(&origin)?.into(), n.into(), Some(keys))?.map(|r| r.ran.then_some(r.rebuilt)),
        Some(Err(_)) => None,
    };
    let count = match rebuilt {
        Some(count) => count,
        None => planar(py, "rebuild_scoped")?.call1((&me, &mesh))?.extract::<Option<usize>>()?,
    };
    if let Some(count) = count {
        me.setattr("rebuilt", count)?;
        clear_selection(scene)?;
        version(scene)?;
    }
    Ok(())
}

// ------------------------------------------------------------ a flat drawing

fn run_planar(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let me = running(py).expect("a running step");
    let mesh = scene.getattr("mesh")?;
    keep_before(s, py, &mesh, true)?;
    me.setattr("rebuilt", 0)?;
    me.setattr("flat", true)?;
    if let Some(r) = command_rebuild_planar(&mesh)? {
        me.setattr("flat", r.flat)?;
        if r.flat {
            me.setattr("rebuilt", r.rebuilt)?;
            clear_selection(scene)?;
            version(scene)?;
        }
        return Ok(());
    }
    // The object way, as the Python command ran it.
    let footprint = topology(py, "winding_footprint")?.call1((&mesh,))?;
    if !mesh.getattr("edges")?.is_truthy()? {
        return me.setattr("flat", false);
    }
    let positions = mesh.getattr("vertices")?.try_iter()?.map(|v| v?.getattr("position")).collect::<PyResult<Vec<_>>>()?;
    let plane = py.import("core.arrangement")?.getattr("coplanar_plane")?.call1((PyList::new(py, positions)?,))?;
    if plane.is_none() {
        return me.setattr("flat", false);
    }
    let (mut origin, mut normal) = (plane.get_item(0)?, plane.get_item(1)?);
    let faces = mesh.getattr("faces")?;
    if faces.is_truthy()? {
        // A real face's normal gives a stable orientation.
        let first = faces.get_item(0)?;
        normal = first.call_method0("normal")?;
        origin = first.getattr("vertices")?.get_item(0)?;
    }
    let mesh_api = py.import("core.mesh")?;
    let vec3 = py.import("ingetrazo_core")?.getattr("Vec3")?;
    let mut segments = Vec::new();
    let mut flagged = Vec::new();
    for e in mesh.getattr("edges")?.try_iter()? {
        let e = e?;
        segments.push(PyTuple::new(py, [e.getattr("v0")?.getattr("position")?, e.getattr("v1")?.getattr("position")?])?);
        if !mesh_api.call_method1("edge_is_plain", (&e,))?.is_truthy()? {
            flagged.push(PyTuple::new(py, [vec3.call1((e.getattr("a")?,))?, vec3.call1((e.getattr("b")?,))?, mesh_api.call_method1("edge_flags", (&e,))?])?);
        }
    }
    let mut paint = Vec::new();
    for f in faces.try_iter()? {
        let f = f?;
        let attrs = f.getattr("attrs")?;
        if attrs.is_truthy()? {
            let tris = f.call_method0("triangulate")?.try_iter()?.map(|t| Ok(PyTuple::new(py, t?.try_iter()?.collect::<PyResult<Vec<_>>>()?)?.into_any())).collect::<PyResult<Vec<_>>>()?;
            paint.push(PyTuple::new(py, [PyList::new(py, tris)?.into_any(), py.get_type::<pyo3::types::PyDict>().call1((attrs,))?])?);
        }
    }
    let rebuilt = py.import("core.arrangement")?.getattr("planar_rebuild")?.call1((PyList::new(py, segments)?, origin, normal))?;
    let (edges, regions) = (rebuilt.get_item(0)?, rebuilt.get_item(1)?);
    clear_selection(scene)?;
    planar(py, "install_flat")?.call1((&mesh, is_store(py, &mesh)?, &edges, &regions, PyList::new(py, flagged)?, PyList::new(py, paint)?))?;
    if !footprint.is_none() {
        topology(py, "orient_coplanar_faces")?.call1((&mesh, &footprint))?;
    }
    mesh.call_method0("resplit_curves")?;
    me.setattr("rebuilt", regions.len()?)?;
    version(scene)
}

// ------------------------------------------------------------ heal

fn run_heal(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let me = running(py).expect("a running step");
    let mesh = scene.getattr("mesh")?;
    keep_before(s, py, &mesh, true)?;
    let removed: Vec<Obj<'_>> = match command_heal_overlaps(&mesh)? {
        Some(healed) => face_views(py, &mesh, &healed.faces)?,
        None => {
            let kw = pyo3::types::PyDict::new(py);
            kw.set_item("footprint", topology(py, "winding_footprint")?.call1((&mesh,))?)?;
            topology(py, "heal_overlapping_faces")?.call((&mesh,), Some(&kw))?.try_iter()?.collect::<PyResult<_>>()?
        }
    };
    me.setattr("healed", removed.len())?;
    let selection = scene.getattr("selection")?;
    for f in removed {
        selection.call_method1("discard", (f,))?;
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
    RebuildPlaneFaces = "RebuildPlaneFacesCommand";
    RebuildPlanarFaces = "RebuildPlanarFacesCommand";
    HealOverlaps = "HealOverlapsCommand";
}

fn snapshot_step(name: &'static str, run: fn(&RecordStep, Python<'_>, &Obj<'_>) -> PyResult<()>, args: Vec<(&'static str, Obj<'_>)>) -> RecordStep {
    RecordStep::new(name, run, args).own_inverse().hook(After(after_snapshot))
}

#[pymethods]
#[allow(non_upper_case_globals)]
impl RebuildPlaneFaces {
    #[classattr]
    const _TOL: f64 = TOL;
    /// False when run inside a step that already holds a snapshot of the
    /// mesh (Explode's merge of overlapping faces, #721).
    #[classattr]
    const take_snapshot: bool = true;
    /// How many faces the last run made (0 before it ran).
    #[classattr]
    const rebuilt: usize = 0;

    /// Whether `p` lies on the plane (within `_TOL`).
    fn _on_plane(slf: PyRef<'_, Self>, p: Obj<'_>) -> PyResult<bool> {
        let py = slf.py();
        let step: &RecordStep = &*slf.as_super();
        let o = crate::math::xyz(&step.arg(py, 0))?;
        let n = crate::math::xyz(&step.arg(py, 1))?;
        Ok((crate::math::xyz(&p)? - o).dot(n).abs() < TOL)
    }

    /// Rebuild the faces of one plane of a (possibly 3D) mesh from that
    /// plane's edges, or, given `faces`, only those from their own
    /// remaining edges. A region keeps a face only where an on-plane face
    /// covered it; winding, paint and edge flags carry over.
    #[new]
    #[pyo3(signature = (origin, normal, faces = None))]
    fn new(py: Python<'_>, origin: Obj<'_>, normal: Obj<'_>, faces: Option<Obj<'_>>) -> PyResult<PyClassInitializer<Self>> {
        // Normalised by hand: a zero normal must stay zero (it would mean
        // "every point is on the plane").
        let n = crate::math::xyz(&normal)?;
        let length = n.length();
        let unit = if length > 1e-9 { n / length } else { glam::DVec3::ZERO };
        let faces = match faces.filter(|f| !f.is_none()) {
            Some(f) => PyList::new(py, f.try_iter()?.collect::<PyResult<Vec<_>>>()?)?.into_any(),
            None => none(py),
        };
        let args = vec![
            ("origin", super::steps::vector(py, &origin)?),
            ("normal", Bound::new(py, crate::math::Vec3::from(unit))?.into_any()),
            ("faces", faces),
        ];
        let step = snapshot_step("RebuildPlaneFacesCommand", run_plane, args);
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }

    /// The edges (both ends) or faces (outer corners) near enough the
    /// plane, in list order: a superset from the mesh scan, every one
    /// without it. The rebuild's own test decides.
    fn _near_plane<'py>(slf: PyRef<'py, Self>, mesh: Obj<'py>, kind: &str) -> PyResult<Obj<'py>> {
        let py = slf.py();
        let step: &RecordStep = &*slf.as_super();
        let scan = py.import("core.scan_index")?.call_method1("scan", (&mesh,))?;
        if scan.is_none() {
            return mesh.getattr(if kind == "edge" { "edges" } else { "faces" });
        }
        let o: (f64, f64, f64) = crate::math::xyz(&step.arg(py, 0))?.into();
        let n: (f64, f64, f64) = crate::math::xyz(&step.arg(py, 1))?.into();
        let native = scan.getattr("native")?;
        if kind == "edge" {
            scan.call_method1("edges", (native.call_method1("edges_on_plane", (o, n, TOL, false))?,))
        } else {
            scan.call_method1("faces", (native.call_method1("faces_on_plane", (o, n, TOL))?,))
        }
    }
}

#[pymethods]
#[allow(non_upper_case_globals)]
impl RebuildPlanarFaces {
    #[classattr]
    const rebuilt: usize = 0;
    #[classattr]
    const flat: bool = true;

    /// Rebuild the minimal faces of a flat drawing from its edge graph:
    /// every crossing split, dangling spurs dropped, rooms and holes exact.
    /// A mesh not on one plane is left alone (`flat` False).
    #[new]
    fn new() -> PyClassInitializer<Self> {
        let step = snapshot_step("RebuildPlanarFacesCommand", run_planar, Vec::new());
        PyClassInitializer::from(step).add_subclass(Self)
    }
}

#[pymethods]
#[allow(non_upper_case_globals)]
impl HealOverlaps {
    #[classattr]
    const healed: usize = 0;

    /// Remove redundant mother faces left overlapping their own
    /// subdivisions (`healed` says how many).
    #[new]
    fn new() -> PyClassInitializer<Self> {
        let step = snapshot_step("HealOverlapsCommand", run_heal, Vec::new());
        PyClassInitializer::from(step).add_subclass(Self)
    }
}
