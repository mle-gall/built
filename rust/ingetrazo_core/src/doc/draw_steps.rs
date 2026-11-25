// SPDX-License-Identifier: GPL-3.0-or-later
//! Native drawing steps (docs/commands-port.md): add an edge, add a face.
//!
//! On a store mesh History journals, the kernel's command entry points
//! (`store::command_facade`) draw in Rust and the transaction replays the
//! store. Otherwise (the object-mesh oracle, planned points, a direct call)
//! the object kernels draw and the step keeps its real inverse: the edge it
//! owns, or the face's before and after snapshots.
use super::steps::{running, version, After, Obj, RecordStep};
use crate::store::command_facade::{command_add_face, command_draw_edge};
use pyo3::types::{PyDict, PyList};
use pyo3::prelude::*;
use pyo3::PyClassInitializer;

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}

fn point(v: &Obj<'_>) -> PyResult<[f64; 3]> {
    Ok(crate::math::xyz(v)?.into())
}

fn view<'py>(py: Python<'py>, class: &str, mesh: &Obj<'py>, key: u128) -> PyResult<Obj<'py>> {
    py.import("core.store_mesh")?.getattr(class)?.call1((mesh, key))
}

fn host<'py>(py: Python<'py>, name: &str) -> PyResult<Obj<'py>> {
    py.import("core.history")?.getattr(name)
}

// ------------------------------------------------------------ edge

fn run_edge(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let me = running(py).expect("a running step");
    let mesh = scene.getattr("mesh")?;
    let (soft, curve, layer) = (s.arg(py, 3), s.arg(py, 4), s.arg(py, 5));
    let a = s.arg(py, 0);
    let b = s.arg(py, 1);
    let direct = !s.journalled();
    if direct && me.getattr("_owned")?.is_truthy()? {
        let edge = me.getattr("edge")?;
        mesh.call_method1("relink_vertices", (me.getattr("_made_vertices")?,))?;
        mesh.call_method1("relink_edge", (edge,))?;
        return version(scene);
    }
    let missing = if direct {
        [mesh.call_method1("vertex_at", (&a,))?.is_none(),
         mesh.call_method1("vertex_at", (&b,))?.is_none()]
    } else { [false; 2] };
    let layer_name: Option<String> = layer.extract()?;
    let drawn = command_draw_edge(&mesh, point(&a)?, point(&b)?,
        soft.extract()?, curve.extract()?, layer_name.as_deref())?
        .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("drawing requires a store mesh"))?;
    let edge = match drawn.key {
        Some(key) => view(py, "EdgeView", &mesh, key)?,
        None => none(py),
    };
    if direct && drawn.key.is_some() {
        let made = PyList::empty(py);
        for (absent, attr) in missing.into_iter().zip(["v0", "v1"]) {
            if absent { made.append(edge.getattr(attr)?)?; }
        }
        me.setattr("_made_vertices", made)?;
    }
    me.setattr("_owned", drawn.key.is_some())?;
    me.setattr("edge", edge)?;
    version(scene)
}

fn after_edge(_s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    let me = running(py).expect("a running step");
    let owned = me.getattr("_owned")?.is_truthy()?;
    if phase == Some("undo") {
        if _s.is_direct() && !_s.journalled() && owned {
            let mesh = scene.getattr("mesh")?;
            mesh.call_method1("remove_edge", (me.getattr("edge")?,))?;
            mesh.call_method1("drop_orphan_vertices", (me.getattr("_made_vertices")?,))?;
        }
        if owned {
            scene.getattr("selection")?.call_method1("discard", (me.getattr("edge")?,))?;
        }
    }
    version(scene)
}

// ------------------------------------------------------------ face

/// Memory: `[before, after]` snapshots of the object way (None when journalled).
fn run_face(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>) -> PyResult<()> {
    let me = running(py).expect("a running step");
    let mesh = scene.getattr("mesh")?;
    // A direct redo of the object way puts back what the first run made.
    if let Some(after) = s.remembered(py, 1).filter(|a| !a.is_none()) {
        mesh.call_method1("restore_state", (after,))?;
        return version(scene);
    }
    let (outer, holes, auto, attrs, interior) = (s.arg(py, 0), s.arg(py, 1), s.arg(py, 2), s.arg(py, 3), s.arg(py, 4));
    let before = if s.journalled() { none(py) } else { mesh.call_method0("capture_state")? };
    let outer_pts = outer.try_iter()?.map(|p| point(&p?)).collect::<PyResult<Vec<_>>>()?;
    let hole_pts = if holes.is_none() { None } else {
        Some(holes.try_iter()?.map(|r| r?.try_iter()?.map(|p| point(&p?)).collect::<PyResult<Vec<_>>>()).collect::<PyResult<Vec<_>>>()?)
    };
    let attrs = if attrs.is_none() { None } else { Some(attrs.cast::<PyDict>()?) };
    let added = command_add_face(&mesh, outer_pts, hole_pts, attrs, interior.is_truthy()?, auto.is_truthy()?)?
        .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("face division would leave inconsistent incidence"))?;
    me.setattr("face", view(py, "FaceView", &mesh, added.face)?)?;
    me.setattr("_hole_swaps", added.hole_swaps)?;
    let after = if s.journalled() { none(py) } else { mesh.call_method0("capture_state")? };
    s.remember(vec![before, after]);
    version(scene)
}

fn after_face(s: &RecordStep, py: Python<'_>, scene: &Obj<'_>, phase: Option<&str>) -> PyResult<()> {
    if phase == Some("undo") && s.is_direct() {
        if let Some(before) = s.remembered(py, 0).filter(|b| !b.is_none()) {
            let mesh = scene.getattr("mesh")?;
            let edges = mesh.getattr("edges")?;
            let edge_type = py.import("core.mesh")?.getattr("Edge")?;
            let selected: Vec<Obj<'_>> = scene.getattr("selection")?.try_iter()?.filter_map(|e| e.ok())
                .filter(|e| e.is_instance(&edge_type).unwrap_or(false) && edges.contains(e).unwrap_or(false))
                .collect();
            mesh.call_method1("restore_state", (before,))?;
            let edges = mesh.getattr("edges")?;
            for edge in selected {
                if !edges.contains(&edge)? {
                    scene.getattr("selection")?.call_method1("discard", (edge,))?;
                }
            }
        }
    }
    host(py, "_discard_removed_selection")?.call1((scene,))?;
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
    AddEdge = "AddEdgeCommand";
    AddFace = "AddFaceCommand";
}

#[pymethods]
impl AddEdge {
    /// Add a single edge. The mesh welds and dedups: a coincident edge is
    /// reused, and only an edge this step made goes again on undo.
    #[new]
    #[pyo3(signature = (a, b, merge = true, soft = None, curve = None, layer = None))]
    fn new(py: Python<'_>, a: Obj<'_>, b: Obj<'_>, merge: bool, soft: Option<Obj<'_>>, curve: Option<Obj<'_>>, layer: Option<Obj<'_>>) -> PyResult<PyClassInitializer<Self>> {
        let args = vec![
            ("a", super::steps::vector(py, &a)?),
            ("b", super::steps::vector(py, &b)?),
            ("merge", pyo3::types::PyBool::new(py, merge).to_owned().into_any()),
            ("soft", soft.unwrap_or_else(|| none(py))),
            ("curve", curve.unwrap_or_else(|| none(py))),
            ("layer", layer.unwrap_or_else(|| none(py))),
        ];
        let step = RecordStep::new("AddEdgeCommand", run_edge, args)
            .rows()
            .own_inverse()
            .publishes(&["edge", "_owned"])
            .hook(After(after_edge));
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}

#[pymethods]
impl AddFace {
    /// Add a face, dividing any coplanar face it lands strictly inside.
    #[new]
    #[pyo3(signature = (vertices, auto = true, holes = None, attrs = None, *, interior = false))]
    fn new(py: Python<'_>, vertices: Obj<'_>, auto: bool, holes: Option<Obj<'_>>, attrs: Option<Obj<'_>>, interior: bool) -> PyResult<PyClassInitializer<Self>> {
        let outer = PyList::new(py, vertices.try_iter()?.map(|p| super::steps::vector(py, &p?)).collect::<PyResult<Vec<_>>>()?)?.into_any();
        let holes = match holes.filter(|h| h.is_truthy().unwrap_or(false)) {
            Some(h) => {
                let rings = h.try_iter()?.map(|ring| Ok(PyList::new(py, ring?.try_iter()?.map(|p| super::steps::vector(py, &p?)).collect::<PyResult<Vec<_>>>()?)?.into_any())).collect::<PyResult<Vec<_>>>()?;
                PyList::new(py, rings)?.into_any()
            }
            None => none(py),
        };
        let attrs = match attrs.filter(|a| a.is_truthy().unwrap_or(false)) {
            Some(a) => py.get_type::<PyDict>().call1((a,))?,
            None => none(py),
        };
        let args = vec![
            ("vertices", outer),
            ("preset_holes", holes),
            ("auto", pyo3::types::PyBool::new(py, auto).to_owned().into_any()),
            ("attrs", attrs),
            ("interior", pyo3::types::PyBool::new(py, interior).to_owned().into_any()),
        ];
        let step = RecordStep::new("AddFaceCommand", run_face, args)
            .rows()
            .own_inverse()
            .publishes(&["face", "_hole_swaps"])
            .hook(After(after_face));
        Ok(PyClassInitializer::from(step).add_subclass(Self))
    }
}
