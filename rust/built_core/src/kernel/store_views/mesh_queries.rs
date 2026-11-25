// SPDX-License-Identifier: GPL-3.0-or-later
//! Store array projections for readers and weld/placement revision bookkeeping.
use super::{provenance::PieceElements, Obj};
use crate::store::py::PyMeshStore;
use numpy::IntoPyArray;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
#[pyfunction]
fn mesh_face_arrays<'py>(mesh: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    let store = mesh.getattr("_store")?;
    let st = PyMeshStore::read(&store)?;
    let inner = &st.inner;
    let mut data = py.detach(|| inner.dense());
    drop(st);
    let mut pos = vec![];
    let mut rank = vec![-1; data.pos.len()];
    for i in &mut data.corners {
        let id = *i as usize;
        if rank[id] < 0 {
            rank[id] = (pos.len() / 3) as i64;
            pos.extend(data.pos[id]);
        }
        *i = rank[id];
    }
    let counts = data
        .ef_start
        .windows(2)
        .map(|w| w[1] - w[0])
        .collect::<Vec<_>>();
    let kw = PyDict::new(py);
    kw.set_item("model_f64", true)?;
    let obj = crate::model::module(py, "built_core.geometry_arrays")?.call_method(
        "Arrays",
        (
            crate::flat3(py, pos)?,
            data.corners.into_pyarray(py),
            data.ring_sizes.into_pyarray(py),
            data.ring_counts.into_pyarray(py),
        ),
        Some(&kw),
    )?;
    obj.setattr("edge_face_counts", counts.into_pyarray(py))?;
    Ok(obj)
}
#[pyfunction]
fn stitch_arrays<'py>(mesh: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    let st = mesh.getattr("_store")?;
    let a = st.call_method0("arrays")?;
    let api = crate::model::module(py, "built_core.store_mesh")?;
    let col = |kind: &str, cls: &str| -> PyResult<Py<PieceElements>> {
        Py::new(
            py,
            PieceElements::new(mesh, st.call_method1("keys", (kind,))?, &api.getattr(cls)?),
        )
    };
    Ok((
        a.get_item("pos")?,
        a.get_item("edges")?,
        col("vertex", "VertexView")?,
        col("edge", "EdgeView")?,
    )
        .into_pyobject(py)?
        .into_any())
}
#[pyfunction]
fn face_count(mesh: &Obj<'_>) -> PyResult<usize> {
    let m = crate::model::module(mesh.py(), "built_core.store_mesh")?;
    if mesh.get_type().is(m.getattr("DeferredMesh")?) {
        return mesh
            .getattr("__dict__")?
            .get_item("_deferred_nfaces")?
            .extract();
    }
    mesh.getattr("faces")?.len()
}
#[pyfunction]
fn face_attribute_rows<'py>(mesh: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    let m = crate::model::module(py, "built_core.store_mesh")?;
    if mesh.get_type().is(m.getattr("DeferredMesh")?) {
        let src = mesh.getattr("__dict__")?.get_item("_deferred_source")?;
        let table = src.get_item("meta")?.call_method1("get", ("face_attrs",))?;
        let mut seen = std::collections::HashSet::new();
        let out = PyList::empty(py);
        let parse = crate::model::module(py, "built_core.formats.igz")?.getattr("_face_attrs_from_json")?;
        for k in src.get_item("face_attr")?.try_iter()? {
            let k = k?.extract::<i64>()?;
            if !seen.insert(k) {
                continue;
            }
            let attr = if k >= 0 {
                parse.call1((table.get_item(k)?,))?
            } else {
                py.None().into_bound(py)
            };
            if attr.is_truthy()? {
                out.append(attr)?
            } else {
                out.append(PyDict::new(py))?
            }
        }
        return Ok(out.into_any());
    }
    if mesh.is_instance(&m.getattr("StoreMesh")?)? {
        return Ok(super::attributes::attribute_rows(mesh)?
            .into_pyobject(py)?
            .into_any());
    }
    let out = PyList::empty(py);
    for face in mesh.getattr("faces")?.try_iter()? {
        out.append(face?.getattr("attrs")?)?;
    }
    Ok(out.into_any())
}
#[pyfunction]
fn mesh_revision<'py>(mesh: &Obj<'py>) -> PyResult<Obj<'py>> {
    mesh.getattr("__dict__")?
        .call_method1("get", ("_revision", 0))?
        .add(mesh.getattr("_mut_serial")?)
}
#[pyfunction]
fn bump_revision(mesh: &Obj<'_>) -> PyResult<()> {
    let d = mesh.getattr("__dict__")?;
    d.set_item(
        "_revision",
        d.call_method1("get", ("_revision", 0))?.add(1)?,
    )?;
    let def = d.call_method1("get", ("_instance_definition",))?;
    if !def.is_none() {
        def.call_method0("mark_modified")?;
    }
    Ok(())
}
#[pyfunction]
fn mark_modified(mesh: &Obj<'_>) -> PyResult<()> {
    mesh.setattr("_chunk_dirty", true)?;
    mesh.setattr("_attrs_dirty", true)?;
    mesh.setattr("_mut_serial", mesh.getattr("_mut_serial")?.add(1)?)?;
    wind(mesh)
}
fn wind(mesh: &Obj<'_>) -> PyResult<()> {
    let w = mesh
        .getattr_opt("_winding_rev")?
        .map(|v| v.add(1))
        .transpose()?;
    match w {
        Some(v) => mesh.setattr("_winding_rev", v),
        None => mesh.setattr("_winding_rev", 1),
    }
}
#[pyfunction]
fn note_rewound(mesh: &Obj<'_>) -> PyResult<()> {
    wind(mesh)?;
    mesh.setattr("_chunk_dirty", true)?;
    mesh.getattr("__dict__")?
        .set_item("_attrs_dirty_value", true)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(mesh_face_arrays, m)?)?;
    m.add_function(wrap_pyfunction!(stitch_arrays, m)?)?;
    m.add_function(wrap_pyfunction!(face_count, m)?)?;
    m.add_function(wrap_pyfunction!(face_attribute_rows, m)?)?;
    m.add_function(wrap_pyfunction!(mesh_revision, m)?)?;
    m.add_function(wrap_pyfunction!(bump_revision, m)?)?;
    m.add_function(wrap_pyfunction!(mark_modified, m)?)?;
    m.add_function(wrap_pyfunction!(note_rewound, m)?)?;
    Ok(())
}
