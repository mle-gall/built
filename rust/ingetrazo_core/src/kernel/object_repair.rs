// SPDX-License-Identifier: GPL-3.0-or-later
//! Replay ordered topology repair plans through the mesh's public edit methods.
//! Resolve all handles from a scan before its first edit invalidates the scan.
use crate::math::{xyz, Vec3};
use crate::object_outline::{heal_objects, object_hole_mask};
use glam::DVec3;
use pyo3::{
    exceptions::PyException,
    prelude::*,
    types::{PyDict, PyList},
};

type Obj<'py> = Bound<'py, PyAny>;
type Punches = Vec<(u128, Vec<u128>)>;
fn snapshot<'py>(items: &Obj<'py>) -> PyResult<Obj<'py>> {
    items.py().get_type::<PyList>().call1((items,))
}
fn points(ring: &Obj<'_>) -> PyResult<Vec<Vec3>> {
    ring.try_iter()?
        .map(|p| Ok(Vec3::from(xyz(&p?)?)))
        .collect()
}
fn holes(face: &Obj<'_>) -> PyResult<Vec<Vec<Vec3>>> {
    face.getattr("holes")?
        .try_iter()?
        .map(|r| points(&r?))
        .collect()
}
fn restore_face(mesh: &Obj<'_>, face: &Obj<'_>, outer: &Obj<'_>, holes: &Obj<'_>) -> PyResult<()> {
    let attrs = mesh
        .py()
        .get_type::<PyDict>()
        .call1((face.getattr("attrs")?,))?;
    mesh.call_method1("add_face", (outer, holes))?
        .setattr("attrs", attrs)
}
fn punch(mesh: &Obj<'_>, face: &Obj<'_>, smalls: Vec<Obj<'_>>) -> PyResult<()> {
    let py = mesh.py();
    let outer = points(&face.getattr("vertices")?)?.into_pyobject(py)?;
    let mut rings = holes(face)?;
    for s in smalls {
        rings.push(points(&s.getattr("vertices")?)?);
    }
    mesh.call_method1("remove_face", (face,))?;
    if let Err(e) = restore_face(mesh, face, &outer, &rings.into_pyobject(py)?) {
        if !e.is_instance_of::<PyException>(py) {
            return Err(e);
        }
        let old = holes(face)?;
        let old = (!old.is_empty()).then_some(old).into_pyobject(py)?;
        restore_face(mesh, face, &outer, &old)?;
    }
    Ok(())
}
fn dedupe_holes(mesh: &Obj<'_>, faces: &Obj<'_>) -> PyResult<()> {
    for f in faces.try_iter()? {
        let f = f?;
        let old = f.getattr("holes")?;
        if old.len()? < 2 {
            continue;
        }
        let mask = object_hole_mask(&old)?;
        if mask.iter().all(|k| *k) {
            continue;
        }
        let mut keep = Vec::new();
        for (r, k) in old.try_iter()?.zip(mask) {
            if k {
                keep.push(points(&r?)?)
            }
        }
        let outer = points(&f.getattr("vertices")?)?.into_pyobject(mesh.py())?;
        mesh.call_method1("remove_face", (&f,))?;
        let keep = (!keep.is_empty())
            .then_some(keep)
            .into_pyobject(mesh.py())?;
        restore_face(mesh, &f, &outer, &keep)?;
    }
    Ok(())
}

#[pyfunction]
#[pyo3(signature=(mesh, coverage, partial, pad, store, face_view, hypot))]
pub(crate) fn heal_faces<'py>(
    mesh: &Obj<'py>,
    coverage: f64,
    partial: bool,
    pad: f64,
    store: Option<&Obj<'py>>,
    face_view: &Obj<'py>,
    hypot: &Obj<'py>,
) -> PyResult<Vec<Obj<'py>>> {
    let py = mesh.py();
    let compensated = py.version_info().minor >= 12;
    let mut removed = Vec::new();
    if let Some(s) = store {
        let view = |key| face_view.call1((mesh, key));
        let keys = s
            .call_method0("multiple_hole_faces")?
            .extract::<Vec<u128>>()?;
        let holed = keys.into_iter().map(view).collect::<PyResult<Vec<_>>>()?;
        dedupe_holes(mesh, PyList::new(py, holed)?.as_any())?;
        let (keys, punches) = s
            .call_method1("heal_plan", (coverage, partial, pad, hypot, compensated))?
            .extract::<(Vec<u128>, Punches)>()?;
        for &k in &keys {
            removed.push(view(k)?);
        }
        if !keys.is_empty() {
            let n = keys.len();
            s.call_method1("remove_drawing_faces", (keys,))?;
            mesh.call_method1("_dirty", (n,))?;
        }
        for (i, inside) in punches {
            punch(
                mesh,
                &view(i)?,
                inside.into_iter().map(view).collect::<PyResult<_>>()?,
            )?;
        }
    } else {
        dedupe_holes(mesh, &snapshot(&mesh.getattr("faces")?)?)?;
        let faces = snapshot(&mesh.getattr("faces")?)?;
        let (keys, punches) = heal_objects(&faces, coverage, partial, pad, compensated)?;
        for i in keys {
            let f = faces.get_item(i)?;
            mesh.call_method1("remove_face", (&f,))?;
            removed.push(f);
        }
        for (i, inside) in punches {
            punch(
                mesh,
                &faces.get_item(i)?,
                inside
                    .into_iter()
                    .map(|k| faces.get_item(k))
                    .collect::<PyResult<_>>()?,
            )?;
        }
    }
    Ok(removed)
}

#[pyfunction]
pub(crate) fn flip_faces(mesh: &Obj<'_>, faces: &Obj<'_>) -> PyResult<()> {
    for f in faces.try_iter()? {
        let f = f?;
        let outer = snapshot(&f.getattr("vertices")?)?;
        outer.call_method0("reverse")?;
        let holes = f.getattr("holes")?;
        let holes = holes.is_truthy()?.then_some(holes);
        let attrs = mesh
            .py()
            .get_type::<PyDict>()
            .call1((f.getattr("attrs")?,))?;
        mesh.call_method1("remove_face", (&f,))?;
        mesh.call_method1("add_face", (outer, holes))?
            .setattr("attrs", attrs)?;
    }
    Ok(())
}

#[pyfunction]
pub(crate) fn sweep_junctions(
    mesh: &Obj<'_>,
    seeds: &Obj<'_>,
    tol: f64,
    cache: &Obj<'_>,
    factory: &Obj<'_>,
) -> PyResult<()> {
    loop {
        let mut scan = cache.call1((mesh,))?;
        if !scan.is_truthy()? {
            scan = factory.call1((mesh,))?
        }
        let hits = scan
            .getattr("native")?
            .call_method1("stitch_hits", (seeds, tol))?
            .extract::<Vec<(usize, usize)>>()?;
        if hits.is_empty() {
            return Ok(());
        }
        let (e, v): (Vec<_>, Vec<_>) = hits.into_iter().unzip();
        let edges = scan.call_method1("edges", (e,))?;
        let vertices = scan.call_method1("vertices", (v,))?;
        for (e, v) in edges.try_iter()?.zip(vertices.try_iter()?) {
            mesh.call_method1("split_edge_at", (e?, v?))?;
        }
    }
}

#[pyfunction]
pub(crate) fn prune_orphans<'py>(
    mesh: &Obj<'py>,
    scan: &Obj<'py>,
    read_flags: &Obj<'py>,
    stamp: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let cuts = scan
        .getattr("native")?
        .call_method0("orphan_pieces")?
        .extract::<Vec<(usize, Vec<[[f64; 3]; 2]>)>>()?;
    let removed =
        scan.call_method1("edges", (cuts.iter().map(|(i, _)| *i).collect::<Vec<_>>(),))?;
    let mut curved = false;
    for (e, (_, pieces)) in removed.try_iter()?.zip(cuts) {
        let e = e?;
        let flags = read_flags.call1((&e,))?;
        mesh.call_method1("remove_edge", (&e,))?;
        for [a, b] in pieces {
            let a = Vec3::from(DVec3::from_array(a)).into_pyobject(mesh.py())?;
            let b = Vec3::from(DVec3::from_array(b)).into_pyobject(mesh.py())?;
            let va = mesh.call_method1("vertex_at", (&a,))?;
            let vb = mesh.call_method1("vertex_at", (&b,))?;
            if !va.is_none()
                && !vb.is_none()
                && !mesh.call_method1("find_edge", (va, vb))?.is_none()
            {
                continue;
            }
            stamp.call1((mesh.call_method1("add_edge", (a, b))?, &flags))?;
            curved = curved || !e.getattr("curve")?.is_none();
        }
    }
    if curved {
        mesh.call_method0("resplit_curves")?;
    }
    Ok(removed)
}
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(heal_faces, m)?)?;
    m.add_function(wrap_pyfunction!(flip_faces, m)?)?;
    m.add_function(wrap_pyfunction!(sweep_junctions, m)?)?;
    m.add_function(wrap_pyfunction!(prune_orphans, m)?)?;
    Ok(())
}
