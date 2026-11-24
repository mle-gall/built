// SPDX-License-Identifier: GPL-3.0-or-later
//! Maintain the object mesh's position index through moves, landing and undo.
use crate::object_vertices::probe_vertex;
use crate::object_weld::rings;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use std::collections::HashSet;
type Obj<'py> = Bound<'py, PyAny>;

fn element_key(v: &Obj<'_>) -> PyResult<i128> {
    v.getattr("key")?.extract()
}

#[pyfunction]
pub(crate) fn registers(mesh: &Obj<'_>, v: &Obj<'_>, cell: &Obj<'_>, tol2: f64) -> PyResult<bool> {
    let reg = mesh.getattr("_registry")?.cast_into::<PyDict>()?;
    let holder = reg.get_item(cell)?;
    if holder.as_ref().is_some_and(|w| w.is(v)) {
        return Ok(true);
    }
    if let Some(queue) = mesh
        .getattr("_displaced")?
        .cast_into::<PyDict>()?
        .get_item(cell)?
    {
        for w in queue.try_iter()? {
            if w?.is(v) {
                return Ok(true);
            }
        }
    }
    Ok(holder.is_none()
        && probe_vertex(&reg, cell, &v.getattr("position")?, tol2)?.is_some_and(|w| w.is(v)))
}

#[pyfunction]
pub(crate) fn rekey(
    mesh: &Obj<'_>,
    v: &Obj<'_>,
    old: &Obj<'_>,
    new: &Obj<'_>,
    key: &Obj<'_>,
) -> PyResult<()> {
    if old.eq(new)? {
        return Ok(());
    }
    let reg = mesh.getattr("_registry")?.cast_into::<PyDict>()?;
    let waiting = mesh.getattr("_displaced")?.cast_into::<PyDict>()?;
    if reg.get_item(old)?.is_some_and(|w| w.is(v)) {
        reg.del_item(old)?;
        if let Some(q) = waiting.get_item(old)? {
            waiting.del_item(old)?;
            for w in q.try_iter()? {
                let w = w?;
                if !w.is(v)
                    && w.getattr("edges")?.is_truthy()?
                    && key.call1((w.getattr("position")?,))?.eq(old)?
                {
                    reg.set_item(old, &w)?;
                    break;
                }
            }
        }
    } else if let Some(q) = waiting.get_item(old)? {
        let q = q.cast_into::<PyList>()?;
        for i in (0..q.len()).rev() {
            if q.get_item(i)?.is(v) {
                q.del_item(i)?;
            }
        }
        if q.is_empty() {
            waiting.del_item(old)?;
        }
    }
    match reg.get_item(new)? {
        None => reg.set_item(new, v)?,
        Some(w) if !w.is(v) => match waiting.get_item(new)? {
            Some(q) => q.cast_into::<PyList>()?.append(v)?,
            None => waiting.set_item(new, PyList::new(mesh.py(), [v])?)?,
        },
        _ => (),
    }
    Ok(())
}

fn remove_listed(vertices: &Bound<'_, PyList>, doomed: &HashSet<i128>) -> PyResult<()> {
    let keep = PyList::empty(vertices.py());
    for v in vertices.iter() {
        if !doomed.contains(&element_key(&v)?) {
            keep.append(v)?;
        }
    }
    vertices.set_slice(0, vertices.len(), &keep)
}
fn dirty(mesh: &Obj<'_>) -> PyResult<()> {
    mesh.setattr("_chunk_dirty", true)?;
    mesh.setattr("_mut_serial", mesh.getattr("_mut_serial")?.add(1)?)
}
#[pyfunction]
pub(crate) fn drop_orphans(mesh: &Obj<'_>, candidates: &Obj<'_>, key: &Obj<'_>) -> PyResult<usize> {
    let vertices = mesh.getattr("vertices")?.cast_into::<PyList>()?;
    let reg = mesh.getattr("_registry")?.cast_into::<PyDict>()?;
    let mut doomed = HashSet::new();
    let listed = vertices
        .iter()
        .map(|v| element_key(&v))
        .collect::<PyResult<HashSet<_>>>()?;
    let mut to_check = vec![];
    for v in candidates.try_iter()? {
        let v = v?;
        let id = element_key(&v)?;
        if !v.getattr("edges")?.is_truthy()? && listed.contains(&id) {
            doomed.insert(id);
        }
        to_check.push((id, v));
    }
    if !doomed.is_empty() {
        remove_listed(&vertices, &doomed)?;
        for (id, v) in to_check {
            if doomed.contains(&id) {
                let k = key.call1((v.getattr("position")?,))?;
                if reg.get_item(&k)?.is_some_and(|w| w.is(&v)) {
                    reg.del_item(k)?;
                }
            }
        }
        dirty(mesh)?;
    }
    Ok(doomed.len())
}

#[pyfunction]
pub(crate) fn purge_debris(mesh: &Obj<'_>) -> PyResult<(usize, usize)> {
    let vertices = mesh.getattr("vertices")?.cast_into::<PyList>()?;
    let mut used = HashSet::new();
    for f in mesh.getattr("faces")?.try_iter()? {
        for ring in rings(&f?)? {
            for v in ring.try_iter()? {
                used.insert(element_key(&v?)?);
            }
        }
    }
    let (mut doomed, mut live) = (HashSet::new(), HashSet::new());
    for v in vertices.iter() {
        let key = element_key(&v)?;
        if !v.getattr("edges")?.is_truthy()? && !used.contains(&key) {
            doomed.insert(key);
        } else {
            live.insert(key);
        }
    }
    let reg = mesh.getattr("_registry")?.cast_into::<PyDict>()?;
    let mut ghosts = vec![];
    for (k, v) in reg.iter() {
        if !live.contains(&element_key(&v)?) {
            ghosts.push(k);
        }
    }
    let displaced = mesh.getattr("_displaced")?.cast_into::<PyDict>()?;
    for (k, q) in displaced
        .items()
        .try_iter()?
        .map(|row| row?.extract::<(Obj<'_>, Obj<'_>)>())
        .collect::<PyResult<Vec<_>>>()?
    {
        let keep = PyList::empty(mesh.py());
        for v in q.try_iter()? {
            let v = v?;
            if live.contains(&element_key(&v)?) {
                keep.append(v)?;
            }
        }
        if keep.is_empty() {
            displaced.del_item(k)?;
        } else {
            displaced.set_item(k, keep)?;
        }
    }
    if !doomed.is_empty() {
        remove_listed(&vertices, &doomed)?;
    }
    for k in &ghosts {
        reg.del_item(k)?;
    }
    if !doomed.is_empty() || !ghosts.is_empty() {
        dirty(mesh)?;
    }
    Ok((doomed.len(), ghosts.len()))
}
