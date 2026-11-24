// SPDX-License-Identifier: GPL-3.0-or-later
//! Replay edits to a materialised mesh without replacing its public handles.
//! Dict snapshots are also consumed by extensions and by the store adapters.
use crate::math::{xyz, Vec3};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet, PySlice},
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
        List => py.get_type::<PyList>().call1((v,)),
        Set => py.get_type::<PySet>().call1((v,)),
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
            columns.push((data, field, kind));
        }
        for obj in handles.try_iter()? {
            let obj = obj?;
            let key = obj.getattr("key")?;
            for (data, field, kind) in &columns {
                data.set_item(&key, copy_field(&obj.getattr(*field)?, *kind)?)?;
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

fn stamp(edge: &Obj<'_>, cmd: &Obj<'_>) -> PyResult<()> {
    for field in ["soft", "curve"] {
        let value = cmd.getattr(field)?;
        if !value.is_none() {
            edge.setattr(field, value)?;
        }
    }
    let layer = cmd.getattr("layer")?;
    if !layer.is_none() {
        edge.py()
            .import("core.layers")?
            .call_method1("assign_layer", (edge, layer))?;
    }
    Ok(())
}
#[pyfunction]
fn draw_edge(cmd: &Obj<'_>, mesh: &Obj<'_>) -> PyResult<()> {
    if cmd.getattr("_owned")?.is_truthy()? {
        let edge = cmd.getattr("edge")?;
        if !edge.is_none() {
            if let Ok(vertices) = cmd.getattr("_made_vertices") {
                if vertices.is_truthy()? {
                    mesh.call_method1("relink_vertices", (vertices,))?;
                }
            }
            mesh.call_method1("relink_edge", (&edge,))?;
            return stamp(&edge, cmd);
        }
    }
    let a = cmd.getattr("a")?;
    let b = cmd.getattr("b")?;
    let v = mesh.call_method1("vertex_at", (&a,))?;
    let w = mesh.call_method1("vertex_at", (&b,))?;
    if !v.is_none() && !w.is_none() {
        let old = mesh.call_method1("find_edge", (&v, &w))?;
        if !old.is_none() {
            stamp(&old, cmd)?;
            if ["soft", "curve", "layer"]
                .iter()
                .any(|f| cmd.getattr(*f).is_ok_and(|v| !v.is_none()))
            {
                mesh.call_method0("mark_modified")?;
            }
            return Ok(());
        }
    }
    let edge = mesh.call_method1("add_edge", (a, b))?;
    cmd.setattr("edge", &edge)?;
    cmd.setattr("_owned", true)?;
    if !cmd.getattr("_journal_geometry")?.is_truthy()? {
        let made = PyList::empty(mesh.py());
        if v.is_none() {
            made.append(edge.getattr("v0")?)?;
        }
        if w.is_none() {
            made.append(edge.getattr("v1")?)?;
        }
        cmd.setattr("_made_vertices", made)?;
    }
    stamp(&edge, cmd)
}
#[pyfunction]
fn undo_edge(cmd: &Obj<'_>, mesh: &Obj<'_>) -> PyResult<bool> {
    let edge = cmd.getattr("edge")?;
    if !cmd.getattr("_owned")?.is_truthy()? || edge.is_none() {
        return Ok(false);
    }
    mesh.call_method1("remove_edge", (edge,))?;
    if let Ok(v) = cmd.getattr("_made_vertices") {
        if v.is_truthy()? {
            mesh.call_method1("drop_orphan_vertices", (v,))?;
        }
    }
    Ok(true)
}
#[pyfunction]
fn explode_curves<'py>(
    mesh: &Obj<'py>,
    selected: &Obj<'py>,
) -> PyResult<(usize, Vec<(Obj<'py>, Obj<'py>)>)> {
    let ids = PySet::empty(mesh.py())?;
    for edge in selected.try_iter()? {
        let edge = edge?;
        if edge.hasattr("curve")? {
            let id = edge.getattr("curve")?;
            if !id.is_none() {
                ids.add(id)?;
            }
        }
    }
    let mut prior = Vec::new();
    for edge in mesh.getattr("edges")?.try_iter()? {
        let edge = edge?;
        let id = edge.getattr("curve")?;
        if ids.contains(&id)? {
            prior.push((edge, id));
        }
    }
    for (edge, _) in &prior {
        edge.setattr("curve", mesh.py().None())?;
    }
    dirty_curves(mesh)?;
    Ok((ids.len(), prior))
}
fn dirty_curves(mesh: &Obj<'_>) -> PyResult<()> {
    mesh.setattr("_chunk_dirty", true)?;
    mesh.setattr("_mut_serial", mesh.getattr("_mut_serial")?.add(1)?)
}
#[pyfunction]
fn restore_curves(mesh: &Obj<'_>, prior: Vec<(Obj<'_>, Obj<'_>)>) -> PyResult<()> {
    for (edge, id) in prior {
        edge.setattr("curve", id)?;
    }
    dirty_curves(mesh)
}
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "object_replay")?;
    sub.add_function(wrap_pyfunction!(capture, &sub)?)?;
    sub.add_function(wrap_pyfunction!(restore, &sub)?)?;
    sub.add_function(wrap_pyfunction!(draw_edge, &sub)?)?;
    sub.add_function(wrap_pyfunction!(undo_edge, &sub)?)?;
    sub.add_function(wrap_pyfunction!(explode_curves, &sub)?)?;
    sub.add_function(wrap_pyfunction!(restore_curves, &sub)?)?;
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
    m.add_submodule(&sub)
}
