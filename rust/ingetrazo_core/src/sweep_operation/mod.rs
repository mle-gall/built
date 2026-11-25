// SPDX-License-Identifier: GPL-3.0-or-later
//! Follow Me uses one projection kernel for the live preview, editable meshes
//! and store transactions. Retain the owning face's paint and curve runs.
use crate::{
    math::{xyz, Vec3},
    store::sweep::{emit_sweep_faces, projected_stations, Station},
};
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet},
    IntoPyObjectExt,
};
type Obj<'p> = Bound<'p, PyAny>;
mod edit;
mod path;
const FACET_COS: f64 = 0.85;
fn points(seq: &Obj) -> PyResult<Vec<DVec3>> {
    seq.try_iter()?.map(|p| xyz(&p?)).collect()
}
fn coords(seq: &Obj) -> PyResult<Vec<[f64; 3]>> {
    Ok(points(seq)?.iter().map(DVec3::to_array).collect())
}
fn hypot(py: Python) -> PyResult<String> {
    py.import("core.store_mesh")?
        .call_method0("_hypot")?
        .extract()
}
fn compensated(py: Python) -> bool {
    py.version_info() >= (3, 12)
}
fn stations(
    face: &Obj,
    path: &Obj,
    closed: bool,
) -> PyResult<Option<(Vec<Station>, Vec<[f64; 3]>)>> {
    let mut profile = vec![coords(&face.getattr("vertices")?)?];
    for ring in face.getattr("holes")?.try_iter()? {
        profile.push(coords(&ring?)?);
    }
    let py = face.py();
    let h = crate::hypot_of(&hypot(py)?)?;
    Ok(projected_stations(
        profile,
        &coords(path)?,
        closed,
        h,
        compensated(py),
    ))
}
fn vectors(rows: &Station) -> Vec<Vec<Vec3>> {
    rows.iter()
        .map(|r| r.iter().map(|&p| DVec3::from_array(p).into()).collect())
        .collect()
}
fn polygons(py: Python, stations: &[Station], closed: bool, cap: bool) -> PyResult<Vec<Station>> {
    let mut out = vec![];
    emit_sweep_faces(
        stations,
        closed,
        crate::hypot_of(&hypot(py)?)?,
        None,
        cap,
        |face| {
            out.push(face.to_vec());
            Ok(())
        },
    )
    .map_err(crate::store::py::op_err)?;
    Ok(out)
}
#[pyfunction]
fn _wall(quad: &Obj) -> PyResult<Option<Vec<Vec3>>> {
    let mut out: Vec<DVec3> = Vec::new();
    for p in points(quad)? {
        if out.last().is_none_or(|v| p.distance(*v) >= 1e-9) {
            out.push(p);
        }
    }
    if out.len() > 1 && out[0].distance(out[out.len() - 1]) < 1e-9 {
        out.pop();
    }
    Ok((out.len() >= 3).then(|| out.into_iter().map(Vec3::from).collect()))
}
#[pyfunction]
fn sweep_rings<'p>(
    face: &Obj<'p>,
    path: &Obj,
    closed: bool,
) -> PyResult<Option<(Vec<Vec<Vec<Vec3>>>, Vec<Vec3>, usize)>> {
    Ok(stations(face, path, closed)?.map(|(ss, ds)| {
        let n = ds.len();
        (
            ss.iter().map(vectors).collect(),
            ds.into_iter()
                .map(|d| DVec3::from_array(d).into())
                .collect(),
            n,
        )
    }))
}
#[pyfunction]
fn sweep_preview_faces<'p>(face: &Obj<'p>, path: &Obj, closed: bool) -> PyResult<Vec<Obj<'p>>> {
    let Some((ss, _)) = stations(face, path, closed)? else {
        return Ok(vec![]);
    };
    let py = face.py();
    let class = py.import("core.geometry")?.getattr("Face")?;
    let attrs = face.getattr("attrs")?;
    let kw = PyDict::new(py);
    if attrs.is_truthy()? {
        kw.set_item("attrs", py.get_type::<PyDict>().call1((attrs,))?)?;
    }
    polygons(py, &ss, closed, false)?
        .into_iter()
        .map(|p| {
            let mut r = vectors(&p);
            let boundary = r.remove(0);
            let holes = (!r.is_empty()).then_some(r);
            class.call((boundary, holes), Some(&kw))
        })
        .collect()
}
#[pyclass(frozen)]
struct SweepFace {
    profile: Py<PyAny>,
    path: Py<PyAny>,
    closed: bool,
    refusal: String,
}
#[pymethods]
impl SweepFace {
    fn __call__(&self, scene: &Obj) -> PyResult<()> {
        let py = scene.py();
        if !sweep_profile(
            &scene.getattr("mesh")?,
            self.profile.bind(py),
            self.path.bind(py),
            self.closed,
        )? {
            return Err(PyErr::from_value(
                py.import("core.history")?
                    .call_method1("CommandRejected", (&self.refusal,))?,
            ));
        }
        Ok(())
    }
}
#[pyfunction(signature=(mesh,key,path,closed,rejection_message="degenerate sweep path"))]
fn build_sweep_face<'p>(
    mesh: &Obj<'p>,
    key: &Obj,
    path: &Obj,
    closed: bool,
    rejection_message: &str,
) -> PyResult<Obj<'p>> {
    let py = mesh.py();
    let profile = py
        .import("core.protocol.keys")?
        .call_method1("elements", (mesh, "face", PyList::new(py, [key])?))?
        .get_item(key)?;
    let cmd = if mesh.hasattr("_store")? {
        py.import("core.store_face_edit")?
            .getattr("StoreFaceMutation")?
    } else {
        py.import("core.history")?.getattr("SnapshotMutation")?
    };
    cmd.call1((SweepFace {
        profile: profile.unbind(),
        path: path.clone().unbind(),
        closed,
        refusal: rejection_message.into(),
    },))
}
fn stitch(mesh: &Obj, ss: &[Station]) -> PyResult<()> {
    let py = mesh.py();
    let keys = PySet::empty(py)?;
    for p in ss.iter().flatten().flatten() {
        let [x, y, z] = crate::store::commands::cell(*p).map(f64::from_bits);
        keys.add((x, y, z))?;
    }
    let kw = PyDict::new(py);
    kw.set_item("coplanar_merge", false)?;
    py.import("ingetrazo_core")?
        .getattr("run_stitch")?
        .call((mesh, keys, py.None()), Some(&kw))?;
    Ok(())
}
fn sweep_store(mesh: &Obj, face: &Obj, path: &Obj, closed: bool, h: &Obj) -> PyResult<bool> {
    let py = mesh.py();
    let store = mesh.getattr("_store")?;
    let key = face.getattr("key")?;
    let path = coords(path)?.into_bound_py_any(py)?;
    let ss = store.call_method1("sweep_stations", (&key, &path, closed, h, compensated(py)))?;
    if ss.is_none() {
        return Ok(false);
    }
    let typed = ss.extract::<Vec<Station>>()?;
    let before = store.call_method0("snapshot")?;
    let adapter = py.import("core.store_mesh")?;
    let cap = if !closed && typed.first().is_some_and(|s| s.len() > 2) {
        let mut first = typed[0][1..].to_vec();
        for ring in &mut first {
            ring.reverse();
        }
        let last = &typed[typed.len() - 1][1..];
        Some([
            adapter.call_method1("_hole_filter", (first,))?,
            adapter.call_method1("_hole_filter", (last.to_vec(),))?,
        ])
    } else {
        None
    };
    let attrs = face.getattr("attrs")?;
    let change = store.call_method1("sweep_face", (&key, &ss, closed, h, cap))?;
    if attrs.is_truthy()? {
        let copy = py.import("copy")?.getattr("deepcopy")?;
        let face_view = adapter.getattr("FaceView")?;
        for k in change.getattr("created")?.get_item("face")?.try_iter()? {
            face_view
                .call1((mesh, k?))?
                .setattr("attrs", copy.call1((&attrs,))?)?;
        }
    }
    mesh.call_method0("_dirty")?;
    stitch(mesh, &typed)?;
    let finished = store.call_method1("finish_sweep", (before, key, path, ss, closed, h))?;
    let mesh_class = py.import("core.mesh")?.getattr("Mesh")?;
    for run in finished.get_item(0)?.try_iter()? {
        store.call_method1(
            "set_curve_run",
            (run?, mesh_class.call_method0("next_curve_id")?),
        )?;
    }
    mesh.call_method0("_dirty")?;
    Ok(true)
}
#[pyfunction]
fn sweep_profile(mesh: &Obj, face: &Obj, path: &Obj, closed: bool) -> PyResult<bool> {
    let py = mesh.py();
    let adapter = py.import("core.store_mesh")?;
    if !adapter.call_method1("_store_of", (mesh,))?.is_none() {
        let h = adapter.call_method0("_hypot")?;
        if h.is_truthy()? {
            if mesh
                .getattr("__dict__")?
                .call_method1("get", ("_resident_edit",))?
                .is_truthy()?
            {
                return sweep_store(mesh, face, path, closed, &h);
            }
            let before = mesh.call_method0("capture_state")?;
            let ctx = py
                .import("core.store_face_edit")?
                .call_method1("resident_edit", (mesh,))?;
            ctx.call_method0("__enter__")?;
            let result = sweep_store(mesh, face, path, closed, &h);
            let restore = if result.is_err() {
                mesh.call_method1("restore_state", (before,)).map(|_| ())
            } else {
                Ok(())
            };
            ctx.call_method1("__exit__", (py.None(), py.None(), py.None()))?;
            restore?;
            return result;
        }
    }
    edit::object_sweep(mesh, face, path, closed)
}
pub(crate) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    m.add("_CURVE_FACET_COS", FACET_COS)?;
    m.add(
        "order_path_edges",
        m.py()
            .import("ingetrazo_core")?
            .getattr("ordered_edge_path")?,
    )?;
    for f in [
        wrap_pyfunction!(_wall, m)?,
        wrap_pyfunction!(sweep_rings, m)?,
        wrap_pyfunction!(sweep_preview_faces, m)?,
        wrap_pyfunction!(sweep_profile, m)?,
        wrap_pyfunction!(build_sweep_face, m)?,
        wrap_pyfunction!(path::orient_closed_path, m)?,
        wrap_pyfunction!(path::manual_path_start, m)?,
        wrap_pyfunction!(path::manual_path_extend, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
