// SPDX-License-Identifier: GPL-3.0-or-later
//! Position lookups and face metrics used by public store views.
pub(crate) mod block;
mod copy_mesh;
mod deferred;
mod edits;
pub(crate) mod properties;
mod state;
pub(crate) use state::snapshot_attr;
pub(crate) mod attributes;
pub(crate) mod bulk;
mod geometry;
mod incidence;
pub(crate) mod lifecycle;
mod merge;
mod mesh_queries;
mod paint_classes;
pub(crate) mod pieces;
pub(crate) mod provenance;
mod push;
use crate::{math::Vec3, object_outline::positions, polygon::raw_newell, store::py::PyMeshStore};
use glam::DVec3;
use pyo3::{
    exceptions::{PyKeyError, PyOverflowError, PyTypeError},
    prelude::*,
    types::{PyInt, PySet},
};
use std::collections::HashSet;
type Obj<'py> = Bound<'py, PyAny>;
fn newell(ring: &Obj<'_>) -> PyResult<DVec3> {
    if ring.len()? < 3 {
        return Ok(DVec3::ZERO);
    }
    Ok(raw_newell(
        &positions(ring)?
            .into_iter()
            .map(DVec3::from_array)
            .collect::<Vec<_>>(),
    ))
}
#[pyfunction]
fn face_normal(ring: &Obj<'_>) -> PyResult<Vec3> {
    let n = newell(ring)?;
    let length = n.length();
    Ok((if length < 1e-9 { DVec3::Z } else { n / length }).into())
}
#[pyfunction]
fn face_area(ring: &Obj<'_>) -> PyResult<f64> {
    Ok(0.5 * newell(ring)?.length())
}

#[pyfunction]
fn vertices_in_cells<'py>(
    mesh: &Obj<'py>,
    store: &Bound<'py, PyMeshStore>,
    cells: &Obj<'py>,
    keyfn: &Obj<'py>,
    view: &Obj<'py>,
) -> PyResult<Vec<Obj<'py>>> {
    if !cells.is_truthy()? {
        return Ok(Vec::new());
    }
    let mut near = HashSet::new();
    for c in cells.try_iter()? {
        let c = c?;
        let mut cell = [0i128; 3];
        let mut representable = true;
        for (i, n) in cell.iter_mut().enumerate() {
            let x = c.get_item(i)?;
            let scaled = x.extract::<f64>()?
                * if x.is_instance_of::<PyInt>() {
                    1.
                } else {
                    10000.
                };
            let rounded = scaled.round_ties_even();
            if !rounded.is_finite() {
                // Preserve the exception Python's round raises for a nonfinite cell.
                pyo3::types::PyFloat::new(c.py(), scaled).call_method0("__round__")?;
            }
            if rounded.abs() > (1u64 << 63) as f64 {
                representable = false;
            }
            *n = rounded as i128;
        }
        if !representable {
            continue;
        }
        for dx in [-1, 0] {
            for dy in [-1, 0] {
                for dz in [-1, 0] {
                    if let (Ok(x), Ok(y), Ok(z)) = (
                        i64::try_from(cell[0] + dx),
                        i64::try_from(cell[1] + dy),
                        i64::try_from(cell[2] + dz),
                    ) {
                        near.insert([x, y, z]);
                    }
                }
            }
        }
    }
    let mut result = Vec::new();
    let rows = store.try_borrow()?.vertices_in_floor_cells(near);
    for (key, p) in rows {
        if cells.contains(keyfn.call1((p,))?)? {
            result.push(view.call1((mesh, key))?);
        }
    }
    Ok(result)
}
#[pyfunction]
fn cells_of<'py>(
    store: &Obj<'py>,
    vertex_keys: &Obj<'py>,
    keyfn: &Obj<'py>,
) -> PyResult<Bound<'py, PySet>> {
    let py = store.py();
    let out = PySet::empty(py)?;
    for k in vertex_keys.try_iter()? {
        let k = k?;
        let add = || out.add(keyfn.call1((store.call_method1("vertex_position", (k,))?,))?);
        if let Err(e) = add() {
            if !e.is_instance_of::<PyKeyError>(py)
                && !e.is_instance_of::<PyOverflowError>(py)
                && !e.is_instance_of::<PyTypeError>(py)
            {
                return Err(e);
            }
        }
    }
    Ok(out)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let s = PyModule::new(m.py(), "store_views")?;
    s.add_function(wrap_pyfunction!(face_normal, &s)?)?;
    s.add_function(wrap_pyfunction!(face_area, &s)?)?;
    s.add_function(wrap_pyfunction!(vertices_in_cells, &s)?)?;
    s.add_function(wrap_pyfunction!(cells_of, &s)?)?;
    s.add_function(wrap_pyfunction!(paint_classes::paint_classes, &s)?)?;
    provenance::register(&s)?;
    s.add_class::<state::MeshSnapshotRecord>()?;
    bulk::register(&s)?;
    lifecycle::register(&s)?;
    merge::register(&s)?;
    mesh_queries::register(&s)?;
    push::register(&s)?;
    copy_mesh::register(&s)?;
    edits::register(&s)?;
    block::register(&s)?;
    attributes::register(&s)?;
    incidence::register(&s)?;
    geometry::register(&s)?;
    deferred::register(&s)?;
    properties::register(&s)?;
    m.add_submodule(&s)
}
