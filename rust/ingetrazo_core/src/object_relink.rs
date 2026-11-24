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

#[derive(Clone, Copy)]
enum RadialEdit {
    Append,
    Attach,
    Restore,
    Detach,
}

/// New faces record every occurrence of an edge, including repeated corners.
/// Reattaching an existing face inserts it at most once in each radial list.
fn edit_ring(mesh: &Obj<'_>, face: &Obj<'_>, ring: &Obj<'_>, edit: RadialEdit) -> PyResult<()> {
    let find = mesh.getattr(match edit {
        RadialEdit::Append | RadialEdit::Attach => "_link_edge",
        _ => "find_edge",
    })?;
    let n = ring.len()?;
    for i in 0..n {
        let edge = find.call1((ring.get_item(i)?, ring.get_item((i + 1) % n)?))?;
        if edge.is_none() {
            continue;
        }
        let radial = edge.getattr("faces")?;
        match edit {
            RadialEdit::Append => {
                radial.call_method1("append", (face,))?;
            }
            RadialEdit::Attach | RadialEdit::Restore => {
                if !radial.contains(face)? {
                    radial.call_method1("append", (face,))?;
                }
            }
            RadialEdit::Detach => {
                if radial.contains(face)? {
                    radial.call_method1("remove", (face,))?;
                }
            }
        }
    }
    Ok(())
}

fn weld_ring<'py>(vertex: &Obj<'py>, ring: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let out = PyList::empty(vertex.py());
    for p in ring.try_iter()? {
        out.append(vertex.call1((p?,))?)?;
    }
    Ok(out)
}

#[pyfunction]
#[pyo3(signature=(mesh,outer,holes=None))]
pub(crate) fn add_face<'py>(
    mesh: &Obj<'py>,
    outer: &Obj<'py>,
    holes: Option<&Obj<'py>>,
) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    let vertex = mesh.getattr("vertex")?;
    let outer = weld_ring(&vertex, outer)?;
    let raw = PyList::empty(py);
    if let Some(holes) = holes {
        for h in holes.try_iter()? {
            raw.append(py.get_type::<PyList>().call1((h?,))?)?;
        }
    }
    let keep = if raw.len() > 1 {
        crate::object_outline::object_hole_mask(raw.as_any())?
    } else {
        vec![true; raw.len()]
    };
    let holes = PyList::empty(py);
    for (h, keep) in raw.iter().zip(keep) {
        if keep {
            holes.append(weld_ring(&vertex, &h)?)?;
        }
    }
    let kw = PyDict::new(py);
    kw.set_item("key", mesh.getattr("_keys")?.call_method0("__next__")?)?;
    let face = py
        .import("core.mesh")?
        .getattr("Face")?
        .call((&outer, &holes), Some(&kw))?;
    for ring in std::iter::once(outer.into_any()).chain(holes.iter()) {
        edit_ring(mesh, &face, &ring, RadialEdit::Append)?;
    }
    mesh.getattr("faces")?.call_method1("append", (&face,))?;
    Ok(face)
}

#[pyfunction]
pub(crate) fn add_hole<'py>(
    mesh: &Obj<'py>,
    face: &Obj<'py>,
    positions: &Obj<'py>,
) -> PyResult<Bound<'py, PyList>> {
    let ring = weld_ring(&mesh.getattr("vertex")?, positions)?;
    face.getattr("hole_loops")?
        .call_method1("append", (&ring,))?;
    edit_ring(mesh, face, ring.as_any(), RadialEdit::Attach)?;
    Ok(ring)
}

#[pyfunction]
pub(crate) fn remove_hole(mesh: &Obj<'_>, face: &Obj<'_>, ring: &Obj<'_>) -> PyResult<()> {
    let holes = face.getattr("hole_loops")?;
    if holes.contains(ring)? {
        holes.call_method1("remove", (ring,))?;
    }
    edit_ring(mesh, face, ring, RadialEdit::Detach)
}

#[pyfunction]
pub(crate) fn remove_face(mesh: &Obj<'_>, face: &Obj<'_>) -> PyResult<()> {
    for ring in rings(face)? {
        edit_ring(mesh, face, &ring, RadialEdit::Detach)?;
    }
    let faces = mesh.getattr("faces")?;
    if faces.contains(face)? {
        faces.call_method1("remove", (face,))?;
    }
    Ok(())
}

#[pyfunction]
pub(crate) fn surface_of<'py>(mesh: &Obj<'py>, face: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let py = mesh.py();
    let find = mesh.getattr("find_edge")?;
    let seen = PySet::empty(py)?;
    let mut todo = vec![face.clone()];
    seen.add(face)?;
    while let Some(f) = todo.pop() {
        for ring in rings(&f)? {
            let n = ring.len()?;
            for i in 0..n {
                let e = find.call1((ring.get_item(i)?, ring.get_item((i + 1) % n)?))?;
                if e.is_none() || !e.getattr("soft")?.is_truthy()? {
                    continue;
                }
                for g in e.getattr("faces")?.try_iter()? {
                    let g = g?;
                    if !g.eq(&f)? && !seen.contains(&g)? {
                        seen.add(&g)?;
                        todo.push(g);
                    }
                }
            }
        }
    }
    PyList::new(py, seen.iter())
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
        edit_ring(mesh, face, &ring, RadialEdit::Attach)?;
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
    edit_ring(mesh, face, ring, RadialEdit::Restore)
}
