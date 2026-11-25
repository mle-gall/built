// SPDX-License-Identifier: GPL-3.0-or-later
//! Keyed incidence views and atomic edits through the native store.
use super::Obj;
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyList, PySet},
};
fn views<'py>(mesh: &Obj<'py>, class: &str, keys: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let make = mesh.py().import("core.store_mesh")?.getattr(class)?;
    let elements = PyList::empty(mesh.py());
    for key in keys.try_iter()? {
        elements.append(make.call1((mesh, key?))?)?;
    }
    Ok(elements)
}
fn keys<'py>(mesh: &Obj<'py>, kind: u8, items: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let ids = PyList::empty(mesh.py());
    for item in items.try_iter()? {
        ids.append(mesh.call_method1("_incidence_key", (item?, kind))?)?;
    }
    Ok(ids)
}
#[pyfunction]
fn vertex_edges<'py>(view: &Obj<'py>) -> PyResult<Obj<'py>> {
    let mesh = view.getattr("_m")?;
    let st = mesh.getattr("_store")?;
    let keys = st.call_method1("vertex_edges", (view.getattr("key")?,))?;
    view.py()
        .import("core.store_mesh")?
        .call_method1("_MutSet", (views(&mesh, "EdgeView", &keys)?,))
}
#[pyfunction]
fn set_vertex_edges(view: &Obj<'_>, value: &Obj<'_>) -> PyResult<()> {
    let set = view.py().get_type::<PySet>();
    if set.call1((value,))?.ne(vertex_edges(view)?)? {
        return Err(PyValueError::new_err(
            "vertex.edges is determined by the endpoints of live edges",
        ));
    }
    Ok(())
}
#[pyfunction]
fn edge_faces<'py>(view: &Obj<'py>) -> PyResult<Obj<'py>> {
    let mesh = view.getattr("_m")?;
    let st = mesh.getattr("_store")?;
    let keys = st.call_method1("edge_faces", (view.getattr("key")?,))?;
    view.py().import("core.store_mesh")?.call_method1(
        "_MutList",
        (view, ("faces",), views(&mesh, "FaceView", &keys)?),
    )
}
#[pyfunction]
fn set_edge_faces(view: &Obj<'_>, value: &Obj<'_>) -> PyResult<()> {
    let mesh = view.getattr("_m")?;
    let st = mesh.getattr("_store")?;
    st.call_method1(
        "order_edge_faces",
        (view.getattr("key")?, keys(&mesh, 2, value)?),
    )?;
    mesh.call_method0("_dirty")?;
    Ok(())
}
#[pyfunction]
fn face_loops<'py>(holes: bool, view: &Obj<'py>) -> PyResult<Obj<'py>> {
    let mesh = view.getattr("_m")?;
    let st = mesh.getattr("_store")?;
    let rings = st.call_method1("face_rings", (view.getattr("key")?,))?;
    let list = view.py().import("core.store_mesh")?.getattr("_MutList")?;
    if !holes {
        return list.call1((
            view,
            ("loop",),
            views(&mesh, "VertexView", &rings.get_item(0)?)?,
        ));
    }
    let values = PyList::empty(view.py());
    for (i, ring) in rings.try_iter()?.skip(1).enumerate() {
        values.append(list.call1((
            view,
            ("hole_loops", i),
            views(&mesh, "VertexView", &ring?)?,
        ))?)?;
    }
    list.call1((view, ("hole_loops",), values))
}
#[pyfunction]
fn set_face_loops(holes: bool, view: &Obj<'_>, value: &Obj<'_>) -> PyResult<()> {
    let mesh = view.getattr("_m")?;
    let st = mesh.getattr("_store")?;
    let key = view.getattr("key")?;
    let rings = st.call_method1("face_rings", (&key,))?;
    let new = if holes {
        let new = PyList::empty(view.py());
        new.append(rings.get_item(0)?)?;
        for ring in value.try_iter()? {
            new.append(keys(&mesh, 0, &ring?)?)?;
        }
        new.into_any()
    } else {
        rings.set_item(0, keys(&mesh, 0, value)?)?;
        rings
    };
    st.call_method1("replace_rings", (key, new))?;
    mesh.call_method0("_dirty")?;
    Ok(())
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(vertex_edges, m)?)?;
    m.add_function(wrap_pyfunction!(set_vertex_edges, m)?)?;
    m.add_function(wrap_pyfunction!(edge_faces, m)?)?;
    m.add_function(wrap_pyfunction!(set_edge_faces, m)?)?;
    m.add_function(wrap_pyfunction!(face_loops, m)?)?;
    m.add_function(wrap_pyfunction!(set_face_loops, m)?)?;
    Ok(())
}
