// SPDX-License-Identifier: GPL-3.0-or-later
//! Mesh tools invoked by document protocol clients.
use super::placements::resolve_placements;
use super::transforms::{apply, empty, records};
use super::*;
use pyo3::types::{PyInt, PyTuple};
fn mesh<'py>(h: &Obj<'py>) -> PyResult<Obj<'py>> {
    h.getattr("scene")?.getattr("mesh")
}
pub(super) fn one<'py>(h: &Obj<'py>, kind: &str, key: &Obj<'py>) -> PyResult<Obj<'py>> {
    need(&mesh(h)?, kind, PyList::new(h.py(), [key])?.as_any())?.get_item(0)
}
fn int<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    v.py().get_type::<PyInt>().call1((v,))
}
#[pyfunction]
#[pyo3(signature=(history,edges=None,faces=None))]
fn erase<'py>(
    history: &Obj<'py>,
    edges: Option<&Obj<'py>>,
    faces: Option<&Obj<'py>>,
) -> Reply<'py> {
    let py = history.py();
    let m = mesh(history)?;
    let es = need(&m, "edge", &empty(py, edges))?;
    let fs = need(&m, "face", &empty(py, faces))?;
    execute(
        history,
        &history_command(py, "EraseSelectionCommand", (es, fs))?,
    )
}
#[pyfunction]
#[pyo3(signature=(history,hide=false,edges=None,placements=None,guides=None,dimensions=None,geo_paths=None,text_labels=None))]
fn erase_stroke<'py>(
    history: &Obj<'py>,
    hide: bool,
    edges: Option<&Obj<'py>>,
    placements: Option<&Obj<'py>>,
    guides: Option<&Obj<'py>>,
    dimensions: Option<&Obj<'py>>,
    geo_paths: Option<&Obj<'py>>,
    text_labels: Option<&Obj<'py>>,
) -> Reply<'py> {
    let py = history.py();
    let scene = history.getattr("scene")?;
    let es = need(&scene.getattr("mesh")?, "edge", &empty(py, edges))?;
    let gs = resolve_placements(history, &empty(py, placements))?;
    let parts = PyList::empty(py);
    if hide {
        for (items, command) in [(es, "HideEdgesCommand"), (gs, "HideCommand")] {
            let visible = PyList::empty(py);
            for x in items.iter() {
                if !x.getattr("hidden")?.is_truthy()? {
                    visible.append(x)?;
                }
            }
            if !visible.is_empty() {
                parts.append(history_command(py, command, (visible, true))?)?;
            }
        }
    } else {
        if !es.is_empty() {
            parts.append(history_command(
                py,
                "EraseSelectionCommand",
                (es, PyList::empty(py)),
            )?)?;
        }
        for g in gs.iter() {
            parts.append(history_command(py, "DeleteGroupCommand", (g,))?)?;
        }
        for (ids, field, cmd) in [
            (guides, "guides", "DeleteGuidesCommand"),
            (dimensions, "dimensions", "DeleteDimensionsCommand"),
            (geo_paths, "geo_paths", "DeleteGeoPathsCommand"),
            (text_labels, "text_labels", "DeleteTextLabelsCommand"),
        ] {
            let selected = records(
                &scene.getattr(field)?,
                &empty(py, ids),
                "unknown stroke target",
            )?;
            if !selected.is_empty() {
                parts.append(history_command(py, cmd, (selected,))?)?;
            }
        }
    }
    apply(history, &parts)
}
#[pyfunction]
fn reverse_faces<'py>(history: &Obj<'py>, faces: &Obj<'py>) -> Reply<'py> {
    flip(history, need(&mesh(history)?, "face", faces)?.as_any())
}
fn flip<'py>(h: &Obj<'py>, faces: &Obj<'py>) -> Reply<'py> {
    if faces.is_truthy()? {
        execute(h, &history_command(h.py(), "FlipFacesCommand", (faces,))?)
    } else {
        answer(h, &h.getattr("revision")?, false)
    }
}
#[pyfunction]
fn orient_faces<'py>(history: &Obj<'py>, face: &Obj<'py>) -> Reply<'py> {
    let seed = one(history, "face", face)?;
    let targets = crate::seed_orientation::faces_to_flip(&seed)?;
    flip(history, PyList::new(history.py(), targets)?.as_any())
}
#[pyfunction]
fn resegment_curve<'py>(history: &Obj<'py>, edge: &Obj<'py>, segments: &Obj<'py>) -> Reply<'py> {
    let seed = one(history, "edge", edge)?;
    let cmd = history
        .py()
        .import("built_core.curve_segments")?
        .call_method1("ResegmentCurveCommand", (seed, int(segments)?))?;
    // An exact curve keeps its circle: only its divisions change.
    crate::store::exact_bound::with_intent(crate::store::exact_bound::FIT_CIRCLES, || {
        execute(history, &cmd)
    })
}
#[pyfunction]
#[pyo3(signature=(history,face,path,closed=false))]
fn follow_me<'py>(
    history: &Obj<'py>,
    face: &Obj<'py>,
    path: &Obj<'py>,
    closed: bool,
) -> Reply<'py> {
    let py = history.py();
    let points = py.get_type::<PyList>().call1((path,))?;
    let message = py.import("built_core.i18n")?.call_method1(
        "tr",
        ("Follow Me: could not sweep along that path (reversal or degenerate segment)",),
    )?;
    let key = py
        .import("built_core.command_protocol")?
        .call_method1("from_json_key", (face,))?;
    let cmd = py.import("built_core.sweep")?.call_method1(
        "build_sweep_face",
        (mesh(history)?, key, &points, closed, message),
    )?;
    // Swept runs along an arc path become exact circles about its axis.
    let (axis, joints) = path_axis(&points)?;
    crate::store::exact_bound::with_joints(joints, || {
        crate::store::exact_bound::with_axis(axis, || {
            crate::store::exact_bound::with_intent(crate::store::exact_bound::FIT_CIRCLES, || {
                execute(history, &cmd)
            })
        })
    })
}

/// The axis of a Follow Me path whose points all lie on one circle.
type Joint = (glam::DVec3, glam::DVec3);
fn path_axis(points: &Obj<'_>) -> PyResult<(Option<Joint>, Vec<Joint>)> {
    let mut pts = Vec::new();
    for p in points.try_iter()? {
        let p = p?;
        let v = if let Ok(v) = p.extract::<[f64; 3]>() {
            v
        } else {
            p.call_method0("toTuple")?.extract::<[f64; 3]>()?
        };
        pts.push(glam::DVec3::from(v));
    }
    // Mitre planes bisect the directions meeting at each interior joint.
    let mut joints = Vec::new();
    for w in pts.windows(3) {
        if let (Some(a), Some(b)) = ((w[1] - w[0]).try_normalize(), (w[2] - w[1]).try_normalize()) {
            if let Some(n) = (a + b).try_normalize() {
                joints.push((w[1], n));
            }
        }
    }
    // Any three points lie on a circle: an axis needs a fourth to verify.
    if pts.len() < 4 {
        return Ok((None, joints));
    }
    let scale = pts.iter().fold(0f64, |m, p| m.max(p.abs().max_element()));
    let axis = match crate::exact::bound::fit_circle(&pts, crate::exact::bound::tolerance(scale)) {
        Some(crate::exact::Curve::Circle { frame, .. }) => Some((frame.origin, frame.axis)),
        _ => None,
    };
    Ok((axis, joints))
}

enum Edit {
    Divide,
    Join,
    Cap,
    Debris,
}
#[pyclass(frozen, module = "built_core")]
struct MeshEdit {
    op: Edit,
    args: Py<PyTuple>,
}
#[pymethods]
impl MeshEdit {
    fn __traverse__(&self, visit: pyo3::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        visit.call(&self.args)
    }

    fn __call__(&self, scene: &Obj<'_>) -> PyResult<()> {
        let py = scene.py();
        let m = scene.getattr("mesh")?;
        let args = self.args.bind(py);
        match self.op {
            Edit::Debris => {
                m.call_method0("purge_debris")?;
            }
            Edit::Cap => {
                if !m
                    .call_method("rebuild_cap_plane", args, None)?
                    .is_truthy()?
                {
                    return Err(PyErr::from_value(py.import("built_core.history")?.call_method1(
                        "CommandRejected",
                        ("Cap already matches its plane",),
                    )?));
                }
            }
            Edit::Join | Edit::Divide => {
                let call = PyTuple::new(
                    py,
                    std::iter::once(m).chain(args.iter()).collect::<Vec<_>>(),
                )?;
                py.import("built_core.edits")?.call_method(
                    if matches!(self.op, Edit::Join) {
                        "join_edges"
                    } else {
                        "divide_edges"
                    },
                    call,
                    None,
                )?;
            }
        }
        Ok(())
    }
}
fn mutation<'py>(h: &Obj<'py>, op: Edit, args: Bound<'py, PyTuple>) -> Reply<'py> {
    let callback = Bound::new(
        h.py(),
        MeshEdit {
            op,
            args: args.unbind(),
        },
    )?;
    execute(
        h,
        &history_command(h.py(), "SnapshotMutation", (callback,))?,
    )
}
#[pyfunction]
fn rebuild_cap<'py>(history: &Obj<'py>, face: &Obj<'py>) -> Reply<'py> {
    let f = one(history, "face", face)?;
    mutation(
        history,
        Edit::Cap,
        PyTuple::new(
            history.py(),
            [f.call_method0("centroid")?, f.call_method0("normal")?],
        )?,
    )
}
#[pyfunction]
fn divide_edges<'py>(history: &Obj<'py>, edges: &Obj<'py>, n: &Obj<'py>) -> Reply<'py> {
    let args = PyTuple::new(
        history.py(),
        [need(&mesh(history)?, "edge", edges)?.into_any(), int(n)?],
    )?;
    // Dividing an exact edge splits its parameter interval: the new
    // vertices go onto the curve (succession, `exact_bound::project`).
    crate::store::exact_bound::with_intent(crate::store::exact_bound::PROJECT, || {
        mutation(history, Edit::Divide, args)
    })
}
#[pyfunction]
fn join_edges<'py>(history: &Obj<'py>, edges: &Obj<'py>) -> Reply<'py> {
    mutation(
        history,
        Edit::Join,
        PyTuple::new(history.py(), [need(&mesh(history)?, "edge", edges)?])?,
    )
}
#[pyfunction]
fn purge_debris<'py>(history: &Obj<'py>) -> Reply<'py> {
    mutation(history, Edit::Debris, PyTuple::empty(history.py()))
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(erase, m)?)?;
    m.add_function(wrap_pyfunction!(erase_stroke, m)?)?;
    m.add_function(wrap_pyfunction!(reverse_faces, m)?)?;
    m.add_function(wrap_pyfunction!(orient_faces, m)?)?;
    m.add_function(wrap_pyfunction!(resegment_curve, m)?)?;
    m.add_function(wrap_pyfunction!(follow_me, m)?)?;
    m.add_function(wrap_pyfunction!(rebuild_cap, m)?)?;
    m.add_function(wrap_pyfunction!(divide_edges, m)?)?;
    m.add_function(wrap_pyfunction!(join_edges, m)?)?;
    m.add_function(wrap_pyfunction!(purge_debris, m)?)?;
    Ok(())
}
