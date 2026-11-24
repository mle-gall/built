// SPDX-License-Identifier: GPL-3.0-or-later
//! Restore existing element handles and their incidence during object-mode undo.
use crate::object_weld::rings;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet},
};
type Obj<'py> = Bound<'py, PyAny>;

fn relist<'py>(
    mesh: &Obj<'py>,
    corners: impl IntoIterator<Item = PyResult<Obj<'py>>>,
    key: &Obj<'py>,
) -> PyResult<()> {
    let listed = mesh.getattr("vertices")?.cast_into::<PyList>()?;
    let ids = PySet::empty(mesh.py())?;
    for v in listed.iter() {
        ids.add(v.getattr("key")?)?;
    }
    let reg = mesh.getattr("_registry")?.cast_into::<PyDict>()?;
    for v in corners {
        let v = v?;
        let id = v.getattr("key")?;
        if !ids.contains(&id)? {
            listed.append(&v)?;
            ids.add(id)?;
            reg.call_method1("setdefault", (key.call1((v.getattr("position")?,))?, &v))?;
        }
    }
    Ok(())
}

#[pyfunction]
pub(crate) fn relink_vertices(mesh: &Obj<'_>, vertices: &Obj<'_>, key: &Obj<'_>) -> PyResult<()> {
    relist(mesh, vertices.try_iter()?, key)?;
    mesh.setattr("_chunk_dirty", true)?;
    mesh.setattr("_mut_serial", mesh.getattr("_mut_serial")?.add(1)?)
}

#[pyfunction]
pub(crate) fn relink_edge(mesh: &Obj<'_>, edge: &Obj<'_>, key: &Obj<'_>) -> PyResult<()> {
    let edges = mesh.getattr("edges")?.cast_into::<PyList>()?;
    if edges.contains(edge)? {
        return Ok(());
    }
    let ends = [edge.getattr("v0")?, edge.getattr("v1")?];
    relist(mesh, ends.iter().cloned().map(Ok), key)?;
    let registry = mesh.getattr("_registry")?;
    for v in ends {
        v.getattr("edges")?.cast_into::<PySet>()?.add(edge)?;
        registry.call_method1("setdefault", (key.call1((v.getattr("position")?,))?, &v))?;
    }
    edges.append(edge)?;
    mesh.call_method0("mark_modified")?;
    Ok(())
}

fn link_ring(mesh: &Obj<'_>, face: &Obj<'_>, ring: &Obj<'_>, create: bool) -> PyResult<()> {
    let n = ring.len()?;
    for i in 0..n {
        let edge = mesh.call_method1(
            if create { "_link_edge" } else { "find_edge" },
            (ring.get_item(i)?, ring.get_item((i + 1) % n)?),
        )?;
        if !edge.is_none() {
            let radial = edge.getattr("faces")?.cast_into::<PyList>()?;
            if !radial.contains(face)? {
                radial.append(face)?;
            }
        }
    }
    Ok(())
}

#[pyfunction]
pub(crate) fn relink_face(mesh: &Obj<'_>, face: &Obj<'_>, key: &Obj<'_>) -> PyResult<()> {
    let faces = mesh.getattr("faces")?.cast_into::<PyList>()?;
    if faces.contains(face)? {
        return Ok(());
    }
    let rings = rings(face)?;
    let corners = rings
        .iter()
        .map(|r| r.try_iter())
        .collect::<PyResult<Vec<_>>>()?;
    relist(mesh, corners.into_iter().flatten(), key)?;
    for ring in rings {
        link_ring(mesh, face, &ring, true)?;
    }
    faces.append(face)?;
    mesh.call_method0("mark_modified")?;
    Ok(())
}

#[pyfunction]
pub(crate) fn relink_hole(mesh: &Obj<'_>, face: &Obj<'_>, ring: &Obj<'_>) -> PyResult<()> {
    face.getattr("hole_loops")?
        .cast_into::<PyList>()?
        .append(ring)?;
    link_ring(mesh, face, ring, false)
}
