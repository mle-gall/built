// SPDX-License-Identifier: GPL-3.0-or-later
//! Keep plugin vector identity within a store revision and frame.
use super::{face_normal, Obj, Vec3};
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};

fn cache<'py>(mesh: &Obj<'py>, store: &Obj<'py>, name: &str) -> PyResult<Bound<'py, PyDict>> {
    let d = mesh.getattr("__dict__")?.cast_into::<PyDict>()?;
    let revision = store.getattr("revision")?;
    if let Some(entry) = d.get_item(name)? {
        if entry.get_item(2)?.is(store) && entry.get_item(0)?.eq(&revision)? {
            return Ok(entry.get_item(1)?.cast_into::<PyDict>()?);
        }
    }
    let rows = PyDict::new(mesh.py());
    d.set_item(name, (revision, &rows, store))?;
    Ok(rows)
}
fn cached_point<'py>(
    rows: &Bound<'py, PyDict>,
    store: &Obj<'py>,
    key: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    if let Some(point) = rows.get_item(key)? {
        return Ok(point);
    }
    let xyz: [f64; 3] = store.call_method1("vertex_position", (key,))?.extract()?;
    let p = Py::new(key.py(), Vec3::from(DVec3::from_array(xyz)))?
        .into_bound(key.py())
        .into_any();
    rows.set_item(key, &p)?;
    Ok(p)
}
#[pyfunction]
fn position<'py>(mesh: &Obj<'py>, key: &Obj<'py>) -> PyResult<Obj<'py>> {
    let store = mesh.getattr("_store")?;
    cached_point(&cache(mesh, &store, "_pos_cache")?, &store, key)
}
fn entry<'py>(mesh: &Obj<'py>, store: &Obj<'py>, key: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let rows = cache(mesh, store, "_face_cache")?;
    if let Some(value) = rows.get_item(key)? {
        return Ok(value.cast_into::<PyList>()?);
    }
    let py = mesh.py();
    let value = PyList::new(
        py,
        [
            store.call_method1("face_positions", (key,))?,
            py.None().into_bound(py),
            py.None().into_bound(py),
        ],
    )?;
    rows.set_item(key, &value)?;
    Ok(value)
}
#[pyfunction]
fn ring_positions<'py>(mesh: &Obj<'py>, key: &Obj<'py>) -> PyResult<Obj<'py>> {
    entry(mesh, &mesh.getattr("_store")?, key)?.get_item(0)
}
#[pyfunction]
fn ring_vectors<'py>(mesh: &Obj<'py>, key: &Obj<'py>) -> PyResult<Obj<'py>> {
    let st = mesh.getattr("_store")?;
    let row = entry(mesh, &st, key)?;
    let value = row.get_item(1)?;
    if !value.is_none() {
        return Ok(value);
    }
    let points = cache(mesh, &st, "_pos_cache")?;
    let rings = PyList::empty(mesh.py());
    for ring in st.call_method1("face_rings", (key,))?.try_iter()? {
        let vs = PyList::empty(mesh.py());
        for v in ring?.try_iter()? {
            vs.append(cached_point(&points, &st, &v?)?)?;
        }
        rings.append(vs)?;
    }
    row.set_item(1, &rings)?;
    Ok(rings.into_any())
}
#[pyfunction]
fn cached_normal(mesh: &Obj<'_>, key: &Obj<'_>) -> PyResult<Vec3> {
    let row = entry(mesh, &mesh.getattr("_store")?, key)?;
    let n = row.get_item(2)?;
    if !n.is_none() {
        return Ok(n.cast::<Vec3>()?.try_borrow()?.clone());
    }
    let n = face_normal(&row.get_item(0)?.get_item(0)?)?;
    row.set_item(2, n.clone())?;
    Ok(n)
}

#[pyfunction]
pub(super) fn exchange_paint(attrs: &Obj<'_>) -> PyResult<()> {
    let back = attrs.call_method1("pop", ("back",))?;
    let front = PyDict::new(attrs.py());
    for key in attrs
        .py()
        .import("core.mesh")?
        .getattr("PAINT_KEYS")?
        .try_iter()?
    {
        let key = key?;
        if attrs.contains(&key)? {
            front.set_item(&key, attrs.call_method1("pop", (&key,))?)?;
        }
    }
    attrs.call_method1("update", (back,))?;
    if !front.is_empty() {
        attrs.set_item("back", front)?;
    }
    Ok(())
}
#[pyfunction]
pub(super) fn swap_orientation_paint(face: &Obj<'_>) -> PyResult<()> {
    let attrs = face.getattr("attrs")?;
    if !attrs.is_none()
        && attrs
            .call_method1("get", ("back",))?
            .is_instance_of::<PyDict>()
    {
        exchange_paint(&attrs)?;
        face.setattr("attrs", attrs)?;
    }
    Ok(())
}
#[pyfunction]
fn turn_face_over(face: &Obj<'_>) -> PyResult<()> {
    swap_orientation_paint(face)?;
    face.call_method0("reverse_rings")?;
    Ok(())
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(position, m)?)?;
    m.add_function(wrap_pyfunction!(ring_vectors, m)?)?;
    m.add_function(wrap_pyfunction!(ring_positions, m)?)?;
    m.add_function(wrap_pyfunction!(cached_normal, m)?)?;
    m.add_function(wrap_pyfunction!(exchange_paint, m)?)?;
    m.add_function(wrap_pyfunction!(swap_orientation_paint, m)?)?;
    m.add_function(wrap_pyfunction!(turn_face_over, m)?)?;
    Ok(())
}
