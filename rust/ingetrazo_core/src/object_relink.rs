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
    let py = mesh.py();
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
        let radial = edge.getattr(pyo3::intern!(py, "faces"))?;
        match edit {
            RadialEdit::Append => {
                radial.call_method1(pyo3::intern!(py, "append"), (face,))?;
            }
            RadialEdit::Attach | RadialEdit::Restore => {
                if !radial.contains(face)? {
                    radial.call_method1(pyo3::intern!(py, "append"), (face,))?;
                }
            }
            RadialEdit::Detach => {
                if radial.contains(face)? {
                    radial.call_method1(pyo3::intern!(py, "remove"), (face,))?;
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
    let vertex = mesh.getattr(pyo3::intern!(py, "vertex"))?;
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
    kw.set_item(
        "key",
        mesh.getattr(pyo3::intern!(py, "_keys"))?
            .call_method0(pyo3::intern!(py, "__next__"))?,
    )?;
    let face = py
        .import("core.mesh")?
        .getattr("Face")?
        .call((&outer, &holes), Some(&kw))?;
    for ring in std::iter::once(outer.into_any()).chain(holes.iter()) {
        edit_ring(mesh, &face, &ring, RadialEdit::Append)?;
    }
    mesh.getattr(pyo3::intern!(py, "faces"))?
        .call_method1(pyo3::intern!(py, "append"), (&face,))?;
    Ok(face)
}

#[pyfunction]
pub(crate) fn add_hole<'py>(
    mesh: &Obj<'py>,
    face: &Obj<'py>,
    positions: &Obj<'py>,
) -> PyResult<Bound<'py, PyList>> {
    let py = mesh.py();
    let ring = weld_ring(&mesh.getattr(pyo3::intern!(py, "vertex"))?, positions)?;
    face.getattr(pyo3::intern!(py, "hole_loops"))?
        .call_method1(pyo3::intern!(py, "append"), (&ring,))?;
    edit_ring(mesh, face, ring.as_any(), RadialEdit::Attach)?;
    Ok(ring)
}

#[pyfunction]
pub(crate) fn remove_hole(mesh: &Obj<'_>, face: &Obj<'_>, ring: &Obj<'_>) -> PyResult<()> {
    let py = mesh.py();
    let holes = face.getattr(pyo3::intern!(py, "hole_loops"))?;
    if holes.contains(ring)? {
        holes.call_method1(pyo3::intern!(py, "remove"), (ring,))?;
    }
    edit_ring(mesh, face, ring, RadialEdit::Detach)
}

#[pyfunction]
pub(crate) fn remove_face(mesh: &Obj<'_>, face: &Obj<'_>) -> PyResult<()> {
    let py = mesh.py();
    for ring in rings(face)? {
        edit_ring(mesh, face, &ring, RadialEdit::Detach)?;
    }
    let faces = mesh.getattr(pyo3::intern!(py, "faces"))?;
    if faces.contains(face)? {
        faces.call_method1(pyo3::intern!(py, "remove"), (face,))?;
    }
    Ok(())
}

#[pyfunction]
pub(crate) fn surface_of<'py>(mesh: &Obj<'py>, face: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let py = mesh.py();
    let find = mesh.getattr(pyo3::intern!(py, "find_edge"))?;
    let seen = PySet::empty(py)?;
    let mut todo = vec![face.clone()];
    seen.add(face)?;
    while let Some(f) = todo.pop() {
        for ring in rings(&f)? {
            let n = ring.len()?;
            for i in 0..n {
                let e = find.call1((ring.get_item(i)?, ring.get_item((i + 1) % n)?))?;
                if e.is_none() || !e.getattr(pyo3::intern!(py, "soft"))?.is_truthy()? {
                    continue;
                }
                for g in e.getattr(pyo3::intern!(py, "faces"))?.try_iter()? {
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

#[pyfunction]
pub(crate) fn find_edge<'py>(v0: &Obj<'py>, v1: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    let py = v0.py();
    let a = v0.getattr(pyo3::intern!(py, "edges"))?;
    let b = v1.getattr(pyo3::intern!(py, "edges"))?;
    let (incident, small, target) = if a.len()? <= b.len()? {
        (a, v0, v1)
    } else {
        (b, v1, v0)
    };
    for e in incident.try_iter()? {
        let e = e?;
        if e.call_method1(pyo3::intern!(py, "other"), (small,))?
            .is(target)
        {
            return Ok(Some(e));
        }
    }
    Ok(None)
}

fn mark_edges(mesh: &Obj<'_>) -> PyResult<()> {
    let py = mesh.py();
    mesh.setattr(pyo3::intern!(py, "_chunk_dirty"), true)?;
    let serial = pyo3::intern!(py, "_mut_serial");
    mesh.setattr(serial, mesh.getattr(serial)?.add(1)?)
}
#[pyfunction]
pub(crate) fn link_edge<'py>(
    mesh: &Obj<'py>,
    v0: &Obj<'py>,
    v1: &Obj<'py>,
    factory: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    mark_edges(mesh)?;
    let e = mesh.call_method1(pyo3::intern!(py, "find_edge"), (v0, v1))?;
    if !e.is_none() {
        return Ok(e);
    }
    let key = mesh
        .getattr(pyo3::intern!(py, "_keys"))?
        .call_method0(pyo3::intern!(py, "__next__"))?;
    let kw = PyDict::new(py);
    kw.set_item(pyo3::intern!(py, "key"), key)?;
    let e = factory.call((v0, v1), Some(&kw))?;
    for v in [v0, v1] {
        v.getattr(pyo3::intern!(py, "edges"))?
            .call_method1(pyo3::intern!(py, "add"), (&e,))?;
    }
    mesh.getattr(pyo3::intern!(py, "edges"))?
        .call_method1(pyo3::intern!(py, "append"), (&e,))?;
    Ok(e)
}
#[pyfunction]
pub(crate) fn add_edge<'py>(mesh: &Obj<'py>, a: &Obj<'py>, b: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    let vertex = mesh.getattr(pyo3::intern!(py, "vertex"))?;
    let v0 = vertex.call1((a,))?;
    let v1 = vertex.call1((b,))?;
    if v0.is(&v1) {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "degenerate edge: endpoints weld to one vertex",
        ));
    }
    mesh.call_method1(pyo3::intern!(py, "_link_edge"), (v0, v1))
}
#[pyfunction]
pub(crate) fn remove_edge(mesh: &Obj<'_>, edge: &Obj<'_>) -> PyResult<()> {
    let py = mesh.py();
    mark_edges(mesh)?;
    for end in [pyo3::intern!(py, "v0"), pyo3::intern!(py, "v1")] {
        edge.getattr(end)?
            .getattr(pyo3::intern!(py, "edges"))?
            .call_method1(pyo3::intern!(py, "discard"), (edge,))?;
    }
    let edges = mesh.getattr(pyo3::intern!(py, "edges"))?;
    if edges.contains(edge)? {
        edges.call_method1(pyo3::intern!(py, "remove"), (edge,))?;
    }
    Ok(())
}
