// SPDX-License-Identifier: GPL-3.0-or-later
//! Choose the store or detached repair planner and preserve keyed views through replay.
use super::{
    faces::{exact_mesh, hypot},
    store_api, Obj,
};
use crate::{
    kernel::{object_repair, push_scans},
    object_outline,
    store::{landing, py::PyMeshStore},
};
use pyo3::{
    exceptions::PyKeyError,
    prelude::*,
    types::{PyDict, PyList, PyTuple},
};

const HEAL_CAP: usize = 3000;
const PAD: f64 = 1e-2;
fn list<'py>(py: Python<'py>, it: Option<&Obj<'py>>) -> PyResult<Obj<'py>> {
    match it {
        Some(v) if v.is_truthy()? => py.get_type::<PyList>().call1((v,)),
        _ => Ok(PyList::empty(py).into_any()),
    }
}
#[pyfunction]
fn _mesh_is_flat(mesh: &Obj<'_>) -> PyResult<bool> {
    let faces = mesh.getattr("faces")?;
    if faces.len()? < 2 {
        return Ok(true);
    }
    match push_scans::mesh_is_flat(mesh, 1e-3)? {
        Some(f) => Ok(f),
        None => object_outline::flat_faces(&faces, 1e-3),
    }
}
#[pyfunction]
fn winding_footprint<'py>(mesh: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    let faces = mesh.getattr("faces")?;
    if faces.len()? > HEAL_CAP || !_mesh_is_flat(mesh)? {
        return Ok(None);
    }
    if exact_mesh(mesh)? && hypot(mesh.py())?.is_some() {
        return Ok(Some(mesh.getattr("_store")?.call_method0("snapshot")?));
    }
    Ok(Some(object_outline::winding_footprint(&faces)?.into_any()))
}
#[pyfunction]
#[pyo3(signature=(mesh,footprint=None))]
fn orient_coplanar_faces<'py>(
    mesh: &Obj<'py>,
    footprint: Option<&Obj<'py>>,
) -> PyResult<Vec<Obj<'py>>> {
    let py = mesh.py();
    let api = store_api(py)?;
    let ns = footprint
        .map(|f| f.getattr_opt("namespace"))
        .transpose()?
        .flatten();
    let mut flips = None;
    if exact_mesh(mesh)? && (footprint.is_none() || ns.is_some()) {
        if let Some(h) = hypot(py)? {
            let keys = mesh
                .getattr("_store")?
                .call_method1(
                    "winding_flips",
                    (h, py.version_info().minor >= 12, footprint),
                )?
                .extract::<Vec<u128>>()?;
            let view = api.getattr("FaceView")?;
            flips = Some(
                keys.into_iter()
                    .map(|k| view.call1((mesh, k)))
                    .collect::<PyResult<Vec<_>>>()?,
            );
        }
    }
    let flipped = match flips {
        Some(f) => f,
        None => {
            let old = if let Some(ns) = ns {
                let old = py.import("ingetrazo_core.mesh")?.call_method0("Mesh")?;
                let store = py
                    .import("core._native")?
                    .getattr("native")?
                    .call_method1("MeshStore", (&ns,))?;
                store.call_method1("restore", (footprint, ns))?;
                crate::kernel::store_views::lifecycle::become_store(
                    &old,
                    &store,
                    PyList::empty(py).as_any(),
                    None,
                )?;
                Some(object_outline::winding_footprint(&old.getattr("faces")?)?.into_any())
            } else {
                None
            };
            object_outline::object_winding(&mesh.getattr("faces")?, old.as_ref().or(footprint))?
        }
    };
    object_repair::flip_faces(mesh, PyList::new(py, &flipped)?.as_any())?;
    Ok(flipped)
}
#[pyfunction]
#[pyo3(signature=(mesh,seedkeys=None))]
fn sweep_tjunctions(mesh: &Obj<'_>, seedkeys: Option<&Obj<'_>>) -> PyResult<()> {
    let py = mesh.py();
    let api = py.import("ingetrazo_core.scan_index")?;
    object_repair::sweep_junctions(
        mesh,
        &list(py, seedkeys)?,
        py.import("ingetrazo_core.mesh")?.getattr("_STITCH_TOL")?.extract()?,
        &api.getattr("scan")?,
        &api.getattr("Scan")?,
    )
}
#[pyfunction]
#[pyo3(signature=(mesh,max_iter=1000))]
fn resolve_tjunctions(mesh: &Obj<'_>, max_iter: i64) -> PyResult<()> {
    let _ = max_iter;
    sweep_tjunctions(mesh, None)
}
#[pyfunction]
#[pyo3(signature=(mesh,tolerance=1e-3,moved=None))]
fn fold_nonplanar_faces<'py>(
    mesh: &Obj<'py>,
    tolerance: f64,
    moved: Option<&Obj<'py>>,
) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    let moved = list(py, moved)?;
    if exact_mesh(mesh)?
        && !mesh
            .getattr("__dict__")?
            .call_method1("get", ("_planned_points",))?
            .is_truthy()?
    {
        if let Some(h) = hypot(py)? {
            let res = mesh
                .getattr("_store")?
                .call_method1("fold_nonplanar", (&moved, tolerance, h))?;
            if !res.is_none() {
                return mesh
                    .getattr("_inherit_face_attrs")?
                    .call1(res.cast::<PyTuple>()?);
            }
        }
    }
    let faces = match mesh.getattr_opt("nonplanar_faces")? {
        Some(s) if !s.is_none() => s.call1((tolerance,))?,
        _ => mesh.getattr("faces")?,
    };
    Ok(PyList::new(
        py,
        crate::object_fold::fold_objects(mesh, &faces, tolerance, moved.extract()?)?,
    )?
    .into_any())
}
#[pyfunction]
fn prune_collinear_orphan_edges<'py>(mesh: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    let api = py.import("ingetrazo_core.scan_index")?;
    let mut scan = api.call_method1("scan", (mesh,))?;
    if !scan.is_truthy()? {
        scan = api.call_method1("Scan", (mesh,))?;
    }
    let flags = py.import("ingetrazo_core.mesh")?;
    object_repair::prune_orphans(
        mesh,
        &scan,
        &flags.getattr("edge_flags")?,
        &flags.getattr("stamp_edge_flags")?,
    )
}
#[pyfunction]
#[allow(non_snake_case)]
fn _heal_partners(
    N: Vec<[f64; 3]>,
    A: Vec<f64>,
    LO: Vec<[f64; 3]>,
    HI: Vec<[f64; 3]>,
) -> PyResult<std::collections::HashMap<usize, Vec<usize>>> {
    object_outline::heal_partners(N, A, LO, HI, PAD)
}
#[pyfunction]
#[pyo3(signature=(mesh,coverage=0.5,partial=None,footprint=None))]
fn heal_overlapping_faces<'py>(
    mesh: &Obj<'py>,
    coverage: f64,
    partial: Option<&Obj<'py>>,
    footprint: Option<&Obj<'py>>,
) -> PyResult<Vec<Obj<'py>>> {
    if mesh.getattr("faces")?.len()? > HEAL_CAP {
        return Ok(Vec::new());
    }
    let py = mesh.py();
    let flat = _mesh_is_flat(mesh)?;
    let partial = partial.map(|x| x.is_truthy()).transpose()?.unwrap_or(flat);
    prune_collinear_orphan_edges(mesh)?;
    sweep_tjunctions(mesh, None)?;
    if flat {
        orient_coplanar_faces(mesh, footprint)?;
    }
    let api = store_api(py)?;
    let h = api.call_method0("_hypot")?;
    let store = if exact_mesh(mesh)? && !h.is_none() {
        Some(mesh.getattr("_store")?)
    } else {
        None
    };
    let removed = object_repair::heal_faces(
        mesh,
        coverage,
        partial,
        PAD,
        store.as_ref(),
        &api.getattr("FaceView")?,
        &h,
    )?;
    if !flat {
        py.import("ingetrazo_core.orient")?
            .call_method1("orient_outward", (mesh,))?;
    }
    Ok(removed)
}
#[pyfunction]
#[pyo3(signature=(vertices,area_eps=1e-10))]
fn defects_near<'py>(vertices: &Obj<'py>, area_eps: f64) -> PyResult<Bound<'py, PyDict>> {
    let py = vertices.py();
    let rows = py
        .get_type::<PyList>()
        .call1((vertices,))?
        .cast_into::<PyList>()?;
    let mut counts = None;
    if !rows.is_empty() {
        if let Some(m) = rows.get_item(0)?.getattr_opt("_m")? {
            if exact_mesh(&m)? {
                let cls = store_api(py)?.getattr("VertexView")?;
                let mut keys = Vec::new();
                for v in rows.iter() {
                    if !v.get_type().is(&cls) || !v.getattr("_m")?.is(&m) {
                        break;
                    }
                    keys.push(v.getattr("key")?.extract()?);
                }
                if keys.len() == rows.len() {
                    let st = m.getattr("_store")?.cast_into::<PyMeshStore>()?;
                    let h = store_api(py)?.call_method0("_hypot")?.extract::<String>()?;
                    match st.try_borrow()?.landing_defects(py, keys, area_eps, &h) {
                        Ok(c) => counts = Some(c),
                        Err(e) if e.is_instance_of::<PyKeyError>(py) => (),
                        Err(e) => return Err(e),
                    }
                }
            }
        }
    }
    let counts = match counts {
        Some(c) => c,
        None => {
            let h = store_api(py)?.call_method0("_hypot")?.extract::<String>()?;
            landing::handle_counts(&rows, area_eps, crate::hypot_of(&h)?)?
        }
    };
    let out = PyDict::new(py);
    for (k, v) in landing::DEFECTS.into_iter().zip(counts) {
        out.set_item(k, v)?;
    }
    Ok(out)
}
#[pyfunction]
fn new_defects(before: &Obj<'_>, after: &Obj<'_>) -> PyResult<Vec<&'static str>> {
    let mut a = [0i64; 6];
    let mut b = a;
    for (i, k) in landing::DEFECTS.iter().enumerate() {
        a[i] = before.call_method1("get", (k, 0))?.extract()?;
        b[i] = after.call_method1("get", (k, 0))?.extract()?;
    }
    Ok(landing::increased(a, b).collect())
}

pub(super) fn register(s: &Bound<'_, PyModule>) -> PyResult<()> {
    s.add_function(wrap_pyfunction!(_mesh_is_flat, s)?)?;
    s.add_function(wrap_pyfunction!(winding_footprint, s)?)?;
    s.add_function(wrap_pyfunction!(orient_coplanar_faces, s)?)?;
    s.add_function(wrap_pyfunction!(sweep_tjunctions, s)?)?;
    s.add_function(wrap_pyfunction!(resolve_tjunctions, s)?)?;
    s.add_function(wrap_pyfunction!(fold_nonplanar_faces, s)?)?;
    s.add_function(wrap_pyfunction!(prune_collinear_orphan_edges, s)?)?;
    s.add_function(wrap_pyfunction!(_heal_partners, s)?)?;
    s.add_function(wrap_pyfunction!(heal_overlapping_faces, s)?)?;
    s.add_function(wrap_pyfunction!(defects_near, s)?)?;
    s.add_function(wrap_pyfunction!(new_defects, s)?)?;
    Ok(())
}
