// SPDX-License-Identifier: GPL-3.0-or-later
//! Replay edits to a materialised mesh without replacing its public handles.
//! Dict snapshots are also consumed by extensions and by the store adapters.
use crate::math::{xyz, Vec3};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet, PySlice, PyString},
};

type Obj<'py> = Bound<'py, PyAny>;
#[derive(Clone, Copy)]
enum CopyField {
    Value,
    Point,
    Set,
    List,
    Rings,
    Dict,
}
use CopyField::*;
const VERTEX: &[(&str, &str, CopyField)] = &[("vpos", "position", Point), ("vedges", "edges", Set)];
const EDGE: &[(&str, &str, CopyField)] = &[
    ("efaces", "faces", List),
    ("esoft", "soft", Value),
    ("ehidden", "hidden", Value),
    ("eflat", "flat", Value),
    ("elayer", "layer", Value),
    ("ecurve", "curve", Value),
];
const FACE: &[(&str, &str, CopyField)] = &[
    ("floop", "loop", List),
    ("fholes", "hole_loops", Rings),
    ("fattrs", "attrs", Dict),
    ("finterior", "interior", Value),
];

fn copy_field<'py>(v: &Obj<'py>, kind: CopyField) -> PyResult<Obj<'py>> {
    let py = v.py();
    match kind {
        Value => Ok(v.clone()),
        Point => Ok(Vec3::from(xyz(v)?).into_pyobject(py)?.into_any()),
        List if v.is_exact_instance_of::<PyList>() => {
            let list = v.cast::<PyList>()?;
            Ok(list.get_slice(0, list.len()).into_any())
        }
        List => py.get_type::<PyList>().call1((v,)),
        Set => py.get_type::<PySet>().call1((v,)),
        Dict if v.is_exact_instance_of::<PyDict>() => Ok(v.cast::<PyDict>()?.copy()?.into_any()),
        Dict => py.get_type::<PyDict>().call1((v,)),
        Rings => {
            let rings = PyList::empty(py);
            for r in v.try_iter()? {
                rings.append(copy_field(&r?, List)?)?;
            }
            Ok(rings.into_any())
        }
    }
}
#[pyfunction]
fn capture<'py>(mesh: &Obj<'py>) -> PyResult<Bound<'py, PyDict>> {
    let py = mesh.py();
    let snap = PyDict::new(py);
    for (name, fields) in [("vertices", VERTEX), ("edges", EDGE), ("faces", FACE)] {
        let handles = copy_field(&mesh.getattr(name)?, List)?;
        let mut columns = Vec::new();
        for &(column, field, kind) in fields {
            let data = PyDict::new(py);
            snap.set_item(column, &data)?;
            columns.push((data, PyString::intern(py, field), kind));
        }
        for obj in handles.cast::<PyList>()?.iter() {
            let key = obj.getattr(pyo3::intern!(py, "key"))?;
            for (data, field, kind) in &columns {
                data.set_item(&key, copy_field(&obj.getattr(field)?, *kind)?)?;
            }
        }
        snap.set_item(name, handles)?;
    }
    snap.set_item("registry", copy_field(&mesh.getattr("_registry")?, Dict)?)?;
    Ok(snap)
}
#[pyfunction]
fn restore(mesh: &Obj<'_>, snap: &Bound<'_, PyDict>) -> PyResult<()> {
    let py = mesh.py();
    mesh.setattr("_chunk_dirty", true)?;
    mesh.getattr("__dict__")?
        .set_item("_attrs_dirty_value", true)?;
    mesh.setattr("_mut_serial", mesh.getattr("_mut_serial")?.add(1)?)?;
    let by_key = PyDict::new(py);
    for name in ["vertices", "edges", "faces"] {
        let handles = snap.as_any().get_item(name)?;
        mesh.getattr(name)?.set_item(PySlice::full(py), &handles)?;
        for obj in handles.try_iter()? {
            let obj = obj?;
            by_key.set_item(obj.getattr("key")?, obj)?;
        }
    }
    let registry = mesh.getattr("_registry")?;
    registry.call_method0("clear")?;
    registry.call_method1("update", (snap.as_any().get_item("registry")?,))?;
    mesh.getattr("_displaced")?.call_method0("clear")?;
    for &(column, field, kind) in VERTEX.iter().chain(EDGE).chain(FACE) {
        // The five edge flags and the two face attributes were optional
        // in older dictionary snapshots.
        let data = if matches!(kind, Value | Dict) {
            let Some(v) = snap.get_item(column)? else {
                continue;
            };
            v
        } else {
            snap.as_any().get_item(column)?
        };
        for pair in data.call_method0("items")?.try_iter()? {
            let pair = pair?;
            by_key
                .as_any()
                .get_item(pair.get_item(0)?)?
                .setattr(field, copy_field(&pair.get_item(1)?, kind)?)?;
        }
    }
    Ok(())
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "object_replay")?;
    sub.add_function(wrap_pyfunction!(capture, &sub)?)?;
    sub.add_function(wrap_pyfunction!(restore, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_relink::find_edge, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_relink::link_edge, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_relink::add_edge, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_relink::remove_edge, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_relink::add_face, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_relink::remove_face, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_relink::add_hole, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_relink::remove_hole, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_relink::surface_of, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_relink::relink_vertices, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_relink::relink_edge, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_relink::relink_face, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_relink::relink_hole, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_vertices::probe_vertex, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_vertices::vertex, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_vertices::weld_candidates, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_vertices::bulk_weld, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_bulk::add_edges_welded, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_bulk::add_faces_welded, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_fold::fold_objects, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_registry::rekey, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_registry::drop_orphans, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_registry::registers, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::object_registry::purge_debris, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::kernel::object_incidence::vertex_faces, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::kernel::object_incidence::face_vertices, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::kernel::object_incidence::face_holes, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::kernel::object_incidence::reverse_rings, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::kernel::object_incidence::insert_between, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::kernel::object_incidence::collapse_collinear, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::kernel::object_incidence::curve_edges, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::kernel::object_incidence::dominant_attrs, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::kernel::object_incidence::mesh_key, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::kernel::object_incidence::ring_normal, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::kernel::object_incidence::orient_union, &sub)?)?;
    sub.add_function(wrap_pyfunction!(crate::kernel::object_incidence::loop_area, &sub)?)?;
    m.add_submodule(&sub)
}
