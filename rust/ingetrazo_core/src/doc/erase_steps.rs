// SPDX-License-Identifier: GPL-3.0-or-later
//! Native erase steps (docs/commands-port.md): delete edges, delete a
//! face, erase a selection.
//!
//! On a journalled store mesh the kernel's command entry points
//! (`store::command_facade`) delete in Rust and the transaction replays the
//! store. Otherwise the object kernels delete and the step keeps a snapshot
//! of the mesh from before: its real inverse.
use super::steps::{running, version, After, Obj, RecordStep};
use crate::store::command_facade::{command_delete_edges, command_delete_face, command_erase_cleanup, command_erase_selection};
use pyo3::prelude::*;
use pyo3::types::{PyList, PyTuple};
use pyo3::PyClassInitializer;

fn point(v: &Obj<'_>) -> PyResult<[f64; 3]> {
    Ok(crate::math::xyz(v)?.into())
}

fn views<'py>(py: Python<'py>, class: &str, mesh: &Obj<'py>, keys: &[u128]) -> PyResult<Obj<'py>> {
    let view = py.import("core.store_mesh")?.getattr(class)?;
    Ok(PyList::new(py, keys.iter().map(|k| view.call1((mesh, *k))).collect::<PyResult<Vec<_>>>()?)?.into_any())
}

fn discard_all(scene: &Obj<'_>, items: &Obj<'_>) -> PyResult<()> {
    let selection = scene.getattr("selection")?;
    for item in items.try_iter()? {
        selection.call_method1("discard", (item?,))?;
    }
    Ok(())
}

/// The object way keeps the mesh as it was (None when journalled).
fn snapshot<'py>(s: &RecordStep, py: Python<'py>, mesh: &Obj<'py>) -> PyResult<Obj<'py>> {
    let before = if s.journalled() { py.None().into_bound(py) } else { mesh.call_method0("capture_state")? };
    s.remember(vec![before.clone()]);
    Ok(before)
}

/// A direct undo of the object way puts the mesh back; every replay drops
/// what no longer exists from the selection.
fn after_removal(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    if phase == Some("undo") && s.is_direct() {
        if let Some(before) = s.remembered(py, 0).filter(|b| !b.is_none()) {
            scene.getattr("mesh")?.call_method1("restore_state", (before,))?;
            return version(scene);
        }
        return Ok(());
    }
    py.import("core.history")?.getattr("_discard_removed_selection")?.call1((scene,))?;
    version(scene)
}

fn run_delete_edges(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let me = running(py).expect("a running step");
    let mesh = scene.getattr("mesh")?;
    let endpoints = s.arg(py, 0);
    let cascade: bool = s.arg(py, 1).extract()?;
    if s.journalled() {
        let segments = endpoints
            .try_iter()?
            .map(|pair| {
                let pair = pair?;
                Ok([point(&pair.get_item(0)?)?, point(&pair.get_item(1)?)?])
            })
            .collect::<PyResult<Vec<_>>>()?;
        if let Some(removed) = command_delete_edges(&mesh, segments, cascade)? {
            let edges = views(py, "EdgeView", &mesh, &removed.edges)?;
            let faces = views(py, "FaceView", &mesh, &removed.faces)?;
            discard_all(scene, &faces)?;
            discard_all(scene, &edges)?;
            me.setattr("removed_edges", edges)?;
            me.setattr("removed_faces", faces)?;
            return version(scene);
        }
    }
    snapshot(s, py, &mesh)?;
    let removed = py
        .import("ingetrazo_core")?
        .getattr("object_drawing")?
        .getattr("delete_edges")?
        .call1((&mesh, &endpoints, cascade, scene.getattr("selection")?))?;
    me.setattr("removed_edges", removed.get_item(0)?)?;
    me.setattr("removed_faces", removed.get_item(1)?)?;
    version(scene)
}

fn run_delete_face(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let me = running(py).expect("a running step");
    let mesh = scene.getattr("mesh")?;
    let ring = s.arg(py, 0);
    if s.journalled() {
        let outer = ring.try_iter()?.map(|p| point(&p?)).collect::<PyResult<Vec<_>>>()?;
        if let Some(removed) = command_delete_face(&mesh, outer)? {
            let face = match removed.face {
                Some(key) => py.import("core.store_mesh")?.getattr("FaceView")?.call1((&mesh, key))?,
                None => py.None().into_bound(py),
            };
            if !face.is_none() {
                scene.getattr("selection")?.call_method1("discard", (&face,))?;
            }
            me.setattr("face", face)?;
            return version(scene);
        }
    }
    snapshot(s, py, &mesh)?;
    let face = py
        .import("ingetrazo_core")?
        .getattr("object_drawing")?
        .getattr("delete_face")?
        .call1((&mesh, &ring, scene.getattr("selection")?))?;
    me.setattr("face", face)?;
    version(scene)
}

// ------------------------------------------------------------ erase selection

fn dot(a: glam::DVec3, b: glam::DVec3) -> f64 {
    a.dot(b)
}

/// Rebuild one plane's faces (`RebuildPlaneFacesCommand`, its own step).
fn rebuild_plane<'py>(py: Python<'py>, scene: &Obj<'py>, origin: glam::DVec3, normal: glam::DVec3, faces: Option<Obj<'py>>) -> PyResult<()> {
    let vec = |v: glam::DVec3| -> PyResult<Obj<'py>> { Ok(Bound::new(py, crate::math::Vec3::from(v))?.into_any()) };
    let class = py.import("core.history")?.getattr("RebuildPlaneFacesCommand")?;
    let kw = pyo3::types::PyDict::new(py);
    if let Some(faces) = faces {
        kw.set_item("faces", faces)?;
    }
    class.call((vec(origin)?, vec(normal)?), Some(&kw))?.call_method1("do", (scene,))?;
    Ok(())
}

/// The slit planes, each distinct plane once, then the faces across a
/// reunited line, each coplanar group from its own faces' remaining edges.
fn rebuild_after_erase<'py>(py: Python<'py>, scene: &Obj<'py>, slits: Vec<(glam::DVec3, glam::DVec3)>, across: Vec<Obj<'py>>) -> PyResult<()> {
    let mut done: Vec<(glam::DVec3, glam::DVec3)> = Vec::new();
    for (origin, normal) in slits {
        let nn = normal.normalize_or_zero();
        if done.iter().any(|&(o, n)| dot(nn, n).abs() > 0.999 && dot(origin - o, n).abs() < 1e-4) {
            continue;
        }
        done.push((origin, nn));
        rebuild_plane(py, scene, origin, nn, None)?;
    }
    let mut across = across;
    while let Some(f) = across.first().cloned() {
        let o = crate::math::xyz(&f.getattr("vertices")?.get_item(0)?)?;
        let n = crate::math::xyz(&f.call_method0("normal")?)?;
        let mut same = Vec::new();
        let mut rest = Vec::new();
        for g in across {
            let gn = crate::math::xyz(&g.call_method0("normal")?)?;
            let go = crate::math::xyz(&g.getattr("vertices")?.get_item(0)?)?;
            if dot(n, gn).abs() > 0.999 && dot(go - o, n).abs() < 1e-4 {
                same.push(g);
            } else {
                rest.push(g);
            }
        }
        across = rest;
        rebuild_plane(py, scene, o, n, Some(PyList::new(py, same)?.into_any()))?;
    }
    Ok(())
}

fn run_erase(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let mesh = scene.getattr("mesh")?;
    let (segments_obj, loops_obj) = (s.arg(py, 0), s.arg(py, 1));
    snapshot(s, py, &mesh)?;
    let loops = loops_obj
        .try_iter()?
        .map(|ring| ring?.try_iter()?.map(|p| point(&p?)).collect::<PyResult<Vec<_>>>())
        .collect::<PyResult<Vec<_>>>()?;
    let segments = segments_obj
        .try_iter()?
        .map(|pair| {
            let pair = pair?;
            Ok([point(&pair.get_item(0)?)?, point(&pair.get_item(1)?)?])
        })
        .collect::<PyResult<Vec<_>>>()?;
    if let Some(erased) = command_erase_selection(&mesh, loops, segments)? {
        discard_all(scene, &views(py, "FaceView", &mesh, &erased.faces)?)?;
        discard_all(scene, &views(py, "EdgeView", &mesh, &erased.edges)?)?;
        let slits = erased.slit_planes.iter().map(|(o, n)| (glam::DVec3::from_array(*o), glam::DVec3::from_array(*n))).collect();
        let across = views(py, "FaceView", &mesh, &erased.across)?.try_iter()?.collect::<PyResult<Vec<_>>>()?;
        rebuild_after_erase(py, scene, slits, across)?;
        if let Some(cleanup) = command_erase_cleanup(&mesh, erased.footprint.bind(py))? {
            discard_all(scene, &views(py, "FaceView", &mesh, &cleanup.healed_faces)?)?;
        }
        return version(scene);
    }
    // The object way, as the Python command ran it.
    let topology = py.import("core.topology")?;
    let footprint = topology.call_method1("winding_footprint", (&mesh,))?;
    let drawing = py.import("ingetrazo_core")?.getattr("object_drawing")?;
    let result = drawing.getattr("erase_selection")?.call1((&mesh, &loops_obj, &segments_obj, scene.getattr("selection")?))?;
    let slits = result
        .get_item(0)?
        .try_iter()?
        .map(|pair| {
            let pair = pair?;
            Ok((crate::math::xyz(&pair.get_item(0)?)?, crate::math::xyz(&pair.get_item(1)?)?))
        })
        .collect::<PyResult<Vec<_>>>()?;
    let across = result.get_item(1)?.try_iter()?.collect::<PyResult<Vec<_>>>()?;
    rebuild_after_erase(py, scene, slits, across)?;
    drawing.getattr("prune_soft_edges")?.call1((&mesh,))?;
    let kw = pyo3::types::PyDict::new(py);
    kw.set_item("footprint", footprint)?;
    let healed = topology.getattr("heal_overlapping_faces")?.call((&mesh,), Some(&kw))?;
    discard_all(scene, &healed)?;
    mesh.call_method0("prune_orphan_vertices")?;
    version(scene)
}

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
    DeleteEdges = "DeleteEdgesCommand";
    DeleteFace = "DeleteFaceCommand";
    EraseSelection = "EraseSelectionCommand";
}

#[pymethods]
impl DeleteEdges {
    /// Erase edges (resolved by endpoint position); a face bounded by one on
    /// its outer loop goes with it, unless `cascade_faces` is false.
    #[new]
    #[pyo3(signature = (edges, cascade_faces = true))]
    fn new(py: Python<'_>, edges: Obj<'_>, cascade_faces: bool) -> PyResult<PyClassInitializer<Self>> {
        let mut pairs = Vec::new();
        for e in edges.try_iter()? {
            let e = e?;
            pairs.push(PyTuple::new(py, [super::steps::vector(py, &e.getattr("a")?)?, super::steps::vector(py, &e.getattr("b")?)?])?);
        }
        let args = vec![
            ("_endpoints", PyList::new(py, pairs)?.into_any()),
            ("cascade_faces", pyo3::types::PyBool::new(py, cascade_faces).to_owned().into_any()),
        ];
        let step = RecordStep::new("DeleteEdgesCommand", run_delete_edges, args)
            .rows()
            .own_inverse()
            .publishes(&["removed_edges", "removed_faces"])
            .hook(After(after_removal));
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}

#[pymethods]
impl DeleteFace {
    /// Remove a face (resolved by its outer loop); its holes go with it.
    #[new]
    fn new(py: Python<'_>, face: Obj<'_>) -> PyResult<PyClassInitializer<Self>> {
        let ring = face
            .getattr("vertices")?
            .try_iter()?
            .map(|v| super::steps::vector(py, &v?))
            .collect::<PyResult<Vec<_>>>()?;
        let step = RecordStep::new("DeleteFaceCommand", run_delete_face, vec![("_loop", PyList::new(py, ring)?.into_any())])
            .rows()
            .own_inverse()
            .publishes(&["face"])
            .hook(After(after_removal));
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}

#[pymethods]
impl EraseSelection {
    /// Erase selected edges and faces, the usual way: a line between two
    /// coplanar faces dissolves and they become one; any other erased edge
    /// takes the faces it bounds, except through a hole.
    #[new]
    #[pyo3(signature = (edges, faces = None))]
    fn new(py: Python<'_>, edges: Obj<'_>, faces: Option<Obj<'_>>) -> PyResult<PyClassInitializer<Self>> {
        let mut pairs = Vec::new();
        for e in edges.try_iter()? {
            let e = e?;
            pairs.push(PyTuple::new(py, [super::steps::vector(py, &e.getattr("a")?)?, super::steps::vector(py, &e.getattr("b")?)?])?);
        }
        let mut loops = Vec::new();
        if let Some(faces) = faces.filter(|f| !f.is_none()) {
            for f in faces.try_iter()? {
                let ring = f?.getattr("vertices")?.try_iter()?.map(|p| super::steps::vector(py, &p?)).collect::<PyResult<Vec<_>>>()?;
                loops.push(PyList::new(py, ring)?);
            }
        }
        let args = vec![
            ("_edge_endpoints", PyList::new(py, pairs)?.into_any()),
            ("_face_loops", PyList::new(py, loops)?.into_any()),
        ];
        let step = RecordStep::new("EraseSelectionCommand", run_erase, args)
            .own_inverse()
            .hook(After(after_removal));
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}
