// SPDX-License-Identifier: GPL-3.0-or-later
//! Drive Push/Pull and outline tools in the same mesh frame as their viewport.
use super::annotations::vec3;
use super::mesh_tools::one;
use super::transforms::{empty, finite};
use super::*;
use crate::doc::history::in_frame;
fn store<'py>(mesh: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    Ok(mesh
        .getattr("__dict__")?
        .cast::<PyDict>()?
        .get_item("_store")?
        .filter(|s| !s.is_none()))
}
fn origin<'py>(
    mesh: &Obj<'py>,
    face: Option<&Obj<'py>>,
    point: Option<&Obj<'py>>,
) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    let frames = py.import("built_core.coordinate_frame")?;
    if let Some(st) = store(mesh)? {
        let p = if let Some(face) = face {
            st.call_method1("face_positions", (face.getattr("key")?,))?
                .get_item(0)?
                .get_item(0)?
        } else {
            point.unwrap().clone()
        };
        frames.call_method1("choose_origin", (p,))
    } else {
        frames.getattr("ZERO")
    }
}
#[pyfunction]
#[pyo3(signature=(history,face,distance,keep_base=false))]
fn push_pull<'py>(
    history: &Obj<'py>,
    face: &Obj<'py>,
    distance: &Obj<'py>,
    #[pyo3(from_py_with=PyAnyMethods::is_truthy)] keep_base: bool,
) -> Reply<'py> {
    let py = history.py();
    let scene = history.getattr("scene")?;
    let mesh = scene.getattr("mesh")?;
    let f = one(history, "face", face)?;
    let op = py
        .import("built_core.push_pull")?
        .call_method0("PushPullOperation")?;
    op.setattr("base_face", &f)?;
    let frame = py
        .import("built_core.coordinate_frame")?
        .call_method1("mesh_frame", (&mesh, origin(&mesh, Some(&f), None)?))?;
    in_frame(&frame, || {
        op.setattr("dragging", true)?;
        op.setattr("_group", py.None())?;
        let target = op.call_method1("_target_scene", (&scene,))?;
        op.setattr("_anchor", f.call_method0("centroid")?)?;
        op.setattr("_normal", f.call_method0("normal")?)?;
        let base = op.call_method1("_classify_base", (&target,))?;
        op.setattr("_attached", base.get_item(0)?)?;
        op.setattr("_prism_cap", base.get_item(1)?)?;
        op.setattr(
            "_cap_positions",
            op.call_method1("_cap_loop_positions", (&f,))?,
        )?;
        op.setattr("_keep_base", keep_base)?;
        op.call_method1("_compute_inward_limit", (target,))?;
        op.setattr(
            "extrusion",
            py.get_type::<pyo3::types::PyFloat>().call1((distance,))?,
        )?;
        op.call_method0("_clamp_extrusion")?;
        let before = history.getattr("revision")?;
        if op.getattr("extrusion")?.extract::<f64>()?.abs() < 1e-6 {
            op.call_method0("_reset")?;
            return answer(history, &before, false);
        }
        let viewport = Bound::new(py, super::api::Headless::from_history(history.clone())?)?;
        let intent = if keep_base { crate::store::exact_bound::KEEP_BASE } else { 0 };
        crate::store::exact_bound::with_intent(intent, || op.call_method1("_commit", (viewport,)))?;
        answer(history, &before, true)
    })
}
fn failed<'py>(h: &Obj<'py>, before: &Obj<'py>, message: &str) -> Reply<'py> {
    let reply = answer(h, before, false)?;
    reply.set_item("error", message)?;
    Ok(reply)
}
#[pyfunction]
#[pyo3(signature=(history,distance,face=None,points=None,normal=None,closed=false,eye=None))]
fn offset_outline<'py>(
    history: &Obj<'py>,
    distance: f64,
    face: Option<&Obj<'py>>,
    points: Option<&Obj<'py>>,
    normal: Option<&Obj<'py>>,
    closed: bool,
    eye: Option<&Obj<'py>>,
) -> Reply<'py> {
    let py = history.py();
    let scene = history.getattr("scene")?;
    let mesh = scene.getattr("mesh")?;
    let before = history.getattr("revision")?;
    if face.is_none() == points.is_none() {
        return Err(PyValueError::new_err(
            "offset_outline needs one face or point path",
        ));
    }
    if !distance.is_finite() || distance == 0. {
        return Err(PyValueError::new_err(
            "offset distance must be finite and nonzero",
        ));
    }
    let offset = py.import("built_core.offset")?;
    let frames = py.import("built_core.coordinate_frame")?;
    if let Some(face) = face {
        let target = one(history, "face", face)?;
        let frame =
            frames.call_method1("mesh_frame", (&mesh, origin(&mesh, Some(&target), None)?))?;
        return in_frame(&frame, || {
            let cmd = match offset.call_method1("build_offset_face", (&mesh, &target, distance)) {
                Ok(v) => v,
                Err(e)
                    if e.value(py)
                        .is_instance(&offset.getattr("OffsetHoleCollision")?)? =>
                {
                    return failed(history, &before, "hole")
                }
                Err(e) => return Err(e),
            };
            if cmd.is_none() {
                return failed(history, &before, "collapsed_face");
            }
            // An offset arc is concentric: its run fits a circle.
            crate::store::exact_bound::with_intent(crate::store::exact_bound::FIT_CIRCLES, || {
                execute(history, &cmd)
            })
        });
    }
    let points = points.unwrap();
    let normal = normal
        .filter(|_| points.len().is_ok_and(|n| n >= if closed { 3 } else { 2 }))
        .ok_or_else(|| PyValueError::new_err("an offset path needs points and a plane normal"))?;
    let msg = "offset path vectors must contain three finite coordinates";
    let n = finite(normal, msg)?;
    for p in points.try_iter()? {
        finite(&p?, msg)?;
    }
    if n.length_squared() < 1e-18 {
        return Err(PyValueError::new_err("offset path normal must be nonzero"));
    }
    let o = origin(&mesh, None, Some(&points.get_item(0)?))?;
    let frame = frames.call_method1("mesh_frame", (&mesh, &o))?;
    in_frame(&frame, || {
        let local = |p: &Obj<'py>| frames.call_method1("point_in_frame", (p, &o));
        let path = PyList::empty(py);
        for p in points.try_iter()? {
            path.append(local(&p?)?)?;
        }
        let shifted = if closed {
            py.import("built_core.topology")?.getattr("offset_loop")?
        } else {
            offset.getattr("offset_chain")?
        }
        .call1((path, vec3(normal)?, distance))?;
        if shifted.is_none() {
            return failed(history, &before, "collapsed_chain");
        }
        let segs = PyList::empty(py);
        let len = shifted.len()?;
        for i in 0..if closed { len } else { len.saturating_sub(1) } {
            segs.append((shifted.get_item(i)?, shifted.get_item((i + 1) % len)?))?;
        }
        let kw = PyDict::new(py);
        kw.set_item("eye", eye.map(local).transpose()?)?;
        execute(
            history,
            &py.import("built_core.edits")?
                .call_method("build_add_edges", (scene, segs), Some(&kw))?,
        )
    })
}
#[pyclass(frozen, module = "built_core")]
struct ApplyFillet {
    plan: Py<PyAny>,
}
#[pymethods]
impl ApplyFillet {
    fn __traverse__(&self, visit: pyo3::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        visit.call(&self.plan)
    }

    fn __call__(&self, scene: &Obj<'_>) -> PyResult<()> {
        let py = scene.py();
        py.import("built_core")?
            .call_method1("apply_fillet", (scene.getattr("mesh")?, self.plan.bind(py)))?;
        Ok(())
    }
}
#[pyfunction]
#[pyo3(signature=(history,radius,segments=None,profile=0.5,measure="radius",corners="round",shape=None,edges=None,vertices=None,_plan=None))]
fn bevel_selection<'py>(
    history: &Obj<'py>,
    radius: f64,
    segments: Option<&Obj<'py>>,
    profile: f64,
    measure: &str,
    corners: &str,
    shape: Option<&Obj<'py>>,
    edges: Option<&Obj<'py>>,
    vertices: Option<&Obj<'py>>,
    _plan: Option<&Obj<'py>>,
) -> Reply<'py> {
    let py = history.py();
    let edges = empty(py, edges);
    let vertices = empty(py, vertices);
    let corners_only = vertices.is_truthy()?;
    if edges.is_truthy()? == corners_only {
        return Err(PyValueError::new_err("supply edges or vertices"));
    }
    let count = match segments {
        Some(s) if s.is_exact_instance_of::<pyo3::types::PyInt>() => {
            s.extract::<usize>().unwrap_or(0)
        }
        None => 8,
        _ => 0,
    };
    if !radius.is_finite() || radius <= 0. || count == 0 {
        return Err(PyValueError::new_err(
            "bevel needs a positive size and segment count",
        ));
    }
    if !(profile > 0. && profile < 1.)
        || !matches!(measure, "radius" | "offset" | "width" | "depth")
        || !matches!(corners, "round" | "cut")
    {
        return Err(PyValueError::new_err(
            "invalid bevel profile, measure or corner fill",
        ));
    }
    let shape = if let Some(shape) = shape {
        if corners_only {
            return Err(PyValueError::new_err("unknown shape for this bevel"));
        }
        if let Ok(name) = shape.extract::<String>() {
            if !matches!(name.as_str(), "steps" | "supports" | "cornice" | "crown") {
                return Err(PyValueError::new_err("unknown shape for this bevel"));
            }
            Some(shape.clone())
        } else {
            if !py
                .import("built_core.bevel_profiles")?
                .call_method1("valid", (shape,))?
                .is_truthy()?
            {
                return Err(PyValueError::new_err("invalid drawn bevel profile"));
            }
            let tuple = py.get_type::<PyTuple>();
            Some(
                PyTuple::new(
                    py,
                    shape
                        .try_iter()?
                        .map(|p| tuple.call1((p?,)))
                        .collect::<PyResult<Vec<_>>>()?,
                )?
                .into_any(),
            )
        }
    } else {
        None
    };
    let mesh = history.getattr("scene")?.getattr("mesh")?;
    let before = history.getattr("revision")?;
    let plan = match _plan {
        Some(p) => p.clone(),
        None if corners_only => py.import("built_core")?.call_method1(
            "plan_vertex_bevel",
            (
                &mesh,
                need(&mesh, "vertex", &vertices)?,
                radius,
                count,
                profile,
            ),
        )?,
        None => py.import("built_core")?.call_method1(
            "plan_fillet",
            (
                &mesh,
                need(&mesh, "edge", &edges)?,
                radius,
                count,
                profile,
                measure,
                corners,
                shape,
            ),
        )?,
    };
    if let Ok(msg) = plan.extract::<String>() {
        return failed(history, &before, &msg);
    }
    let class = py.import("built_core")?.getattr("resident_face_mutation")?;
    let callback = Bound::new(
        py,
        ApplyFillet {
            plan: plan.unbind(),
        },
    )?;
    execute(history, &class.call1((callback, mesh))?)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(bevel_selection, m)?)?;
    m.add_function(wrap_pyfunction!(push_pull, m)?)?;
    m.add_function(wrap_pyfunction!(offset_outline, m)?)?;
    Ok(())
}
