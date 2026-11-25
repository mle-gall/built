// SPDX-License-Identifier: GPL-3.0-or-later
//! Publish local topology edits, including snapshot-backed stitch undo records.
use super::{
    edits::{checked, dirty, key, view},
    Obj,
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
fn api(py: Python<'_>) -> PyResult<Obj<'_>> {
    crate::model::module(py, "built_core.store_mesh")
}
fn hypot(py: Python<'_>) -> PyResult<Obj<'_>> {
    crate::model::module(py, "core._native")?.call_method0("qt_hypot")
}
fn list_views<'py>(mesh: &Obj<'py>, kind: usize, keys: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let out = PyList::empty(mesh.py());
    for k in keys.try_iter()? {
        out.append(view(mesh, kind, &k?)?)?;
    }
    Ok(out)
}
fn dominant(dst: &Obj<'_>, faces: &Obj<'_>) -> PyResult<()> {
    let d = crate::kernel::object_incidence::dominant_attrs(faces)?;
    dst.setattr("attrs", dst.py().get_type::<PyDict>().call1((d,))?)
}
#[pyfunction]
fn split_edge<'py>(
    mesh: &Obj<'py>,
    edge: &Obj<'py>,
    position: &Obj<'py>,
) -> PyResult<(Obj<'py>, Obj<'py>)> {
    let k = key(mesh, edge, 1, true, "split_edge")?;
    let result = checked(
        mesh.getattr("_store")?.call_method1(
            "split_edge",
            (k, mesh.call_method1("_input_point", (position,))?),
        ),
        mesh.py(),
        "split_edge",
    )?
    .get_item(0)?;
    dirty(mesh, result.get_item(2)?.extract()?)?;
    Ok((
        view(mesh, 1, &result.get_item(0)?)?,
        view(mesh, 1, &result.get_item(1)?)?,
    ))
}
#[pyfunction]
fn split_edge_at<'py>(
    mesh: &Obj<'py>,
    edge: &Obj<'py>,
    mid: &Obj<'py>,
) -> PyResult<Bound<'py, PyDict>> {
    let op = "split_edge_at";
    let k = key(mesh, edge, 1, true, op)?;
    let m = key(mesh, mid, 0, true, op)?;
    let undo = mesh.call_method0("_undo_state")?;
    let r = checked(
        mesh.getattr("_store")?.call_method1(op, (&k, &m)),
        mesh.py(),
        op,
    )?
    .get_item(0)?;
    dirty(mesh, 3)?;
    let out = PyDict::new(mesh.py());
    out.set_item("kind", "split")?;
    out.set_item("_undo", undo)?;
    out.set_item("edge", view(mesh, 1, &k)?)?;
    out.set_item("mid", view(mesh, 0, &m)?)?;
    for (i, n) in ["e0", "e1"].iter().enumerate() {
        out.set_item(*n, view(mesh, 1, &r.get_item(i)?)?)?;
    }
    out.set_item("e0_new", r.get_item(2)?)?;
    out.set_item("e1_new", r.get_item(3)?)?;
    out.set_item("faces", list_views(mesh, 2, &r.get_item(4)?)?)?;
    Ok(out)
}
#[pyfunction]
fn collapse_vertex<'py>(mesh: &Obj<'py>, v: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let op = "collapse_vertex";
    let k = key(mesh, v, 0, true, op)?;
    let undo = mesh.call_method0("_undo_state")?;
    let r = checked(
        mesh.getattr("_store")?.call_method1(op, (&k,)),
        mesh.py(),
        op,
    )?
    .get_item(0)?;
    dirty(mesh, 3)?;
    let out = PyDict::new(mesh.py());
    out.set_item("kind", "collapse")?;
    out.set_item("_undo", undo)?;
    out.set_item("v", view(mesh, 0, &k)?)?;
    for (i, (name, kind)) in [("e1", 1), ("e2", 1), ("n1", 0), ("n2", 0), ("new_edge", 1)]
        .into_iter()
        .enumerate()
    {
        out.set_item(name, view(mesh, kind, &r.get_item(i)?)?)?;
    }
    out.set_item("new_edge_new", r.get_item(5)?)?;
    out.set_item("faces", list_views(mesh, 2, &r.get_item(6)?)?)?;
    Ok(out)
}
#[pyfunction]
fn dissolve_edge<'py>(
    mesh: &Obj<'py>,
    edge: &Obj<'py>,
    tolerant: bool,
) -> PyResult<Option<Obj<'py>>> {
    let py = mesh.py();
    let name = if tolerant {
        "dissolve_edge"
    } else {
        "dissolve_edge_recorded"
    };
    let error = || -> PyResult<PyErr> {
        Ok(PyErr::from_value(api(py)?.getattr("StoreRefusal")?.call1(
            (format!("{name} requires representable edge incidence"),),
        )?))
    };
    let Some(k) = super::edits::owned(mesh, edge, 1, true)? else {
        return Err(error()?);
    };
    let undo = if tolerant {
        None
    } else {
        Some(mesh.call_method0("_undo_state")?)
    };
    let raw = mesh.getattr("_store")?.call_method1(
        "dissolve_edge",
        (&k, hypot(py)?, tolerant, api(py)?.getattr("_hole_filter")?),
    );
    let r = match raw {
        Ok(v) => v.get_item(0)?,
        Err(e) if e.is_instance_of::<crate::store::py::StoreFallback>(py) => return Err(error()?),
        Err(e) => return Err(e),
    };
    if r.is_none() {
        return Ok(None);
    }
    dirty(mesh, r.get_item(2)?.extract()?)?;
    let pair = r.get_item(1)?;
    let a = view(mesh, 2, &pair.get_item(0)?)?;
    let b = view(mesh, 2, &pair.get_item(1)?)?;
    let made = view(mesh, 2, &r.get_item(0)?)?;
    dominant(&made, PyList::new(py, [&a, &b])?.as_any())?;
    if tolerant {
        return Ok(Some(made));
    }
    let out = PyDict::new(py);
    out.set_item("kind", "merge")?;
    out.set_item("edge", view(mesh, 1, &k)?)?;
    out.set_item("face_a", a)?;
    out.set_item("face_b", b)?;
    out.set_item("merged", made)?;
    out.set_item("_undo", undo)?;
    Ok(Some(out.into_any()))
}
#[pyfunction]
fn dissolve_coplanar_pair<'py>(
    mesh: &Obj<'py>,
    face_a: &Obj<'py>,
    face_b: &Obj<'py>,
) -> PyResult<Option<Obj<'py>>> {
    let py = mesh.py();
    let op = "dissolve_coplanar_pair";
    let ka = key(mesh, face_a, 2, true, op)?;
    let kb = key(mesh, face_b, 2, true, op)?;
    let undo = mesh.call_method0("_undo_state")?;
    let r = checked(
        mesh.getattr("_store")?.call_method1(
            "dissolve_pair",
            (&ka, &kb, hypot(py)?, api(py)?.getattr("_hole_filter")?),
        ),
        py,
        op,
    )?
    .get_item(0)?;
    if r.is_none() {
        return Ok(None);
    }
    dirty(mesh, r.get_item(2)?.extract()?)?;
    let a = view(mesh, 2, &ka)?;
    let b = view(mesh, 2, &kb)?;
    let made = view(mesh, 2, &r.get_item(0)?)?;
    dominant(&made, PyList::new(py, [&a, &b])?.as_any())?;
    let out = PyDict::new(py);
    out.set_item("kind", "merge_pair")?;
    out.set_item("face_a", a)?;
    out.set_item("face_b", b)?;
    out.set_item("edges", list_views(mesh, 1, &r.get_item(1)?)?)?;
    out.set_item("merged", made)?;
    out.set_item("_undo", undo)?;
    Ok(Some(out.into_any()))
}
#[pyfunction]
fn dissolve_coplanar_region<'py>(mesh: &Obj<'py>, faces: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    let py = mesh.py();
    let op = "dissolve_coplanar_region";
    let faces = faces.try_iter()?.collect::<PyResult<Vec<_>>>()?;
    if faces.len() < 2 {
        return Ok(None);
    }
    let keys = PyList::empty(py);
    for f in faces {
        keys.append(key(mesh, &f, 2, true, op)?)?;
    }
    let r = checked(
        mesh.getattr("_store")?.call_method1(
            "dissolve_region",
            (&keys, hypot(py)?, api(py)?.getattr("_hole_filter")?),
        ),
        py,
        op,
    )?
    .get_item(0)?;
    if r.is_none() {
        return Ok(None);
    }
    dirty(mesh, r.get_item(2)?.extract()?)?;
    let views = list_views(mesh, 2, keys.as_any())?;
    let made = if r.get_item(0)?.is_truthy()? {
        let first = views.get_item(0)?;
        if !first.getattr("attrs")?.is_truthy()? {
            dominant(&first, views.get_slice(1, views.len()).as_any())?;
        }
        first
    } else {
        let made = view(mesh, 2, &r.get_item(1)?)?;
        dominant(&made, views.as_any())?;
        made
    };
    Ok(Some(made))
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(split_edge, m)?)?;
    m.add_function(wrap_pyfunction!(split_edge_at, m)?)?;
    m.add_function(wrap_pyfunction!(collapse_vertex, m)?)?;
    m.add_function(wrap_pyfunction!(dissolve_edge, m)?)?;
    m.add_function(wrap_pyfunction!(dissolve_coplanar_pair, m)?)?;
    m.add_function(wrap_pyfunction!(dissolve_coplanar_region, m)?)?;
    Ok(())
}
