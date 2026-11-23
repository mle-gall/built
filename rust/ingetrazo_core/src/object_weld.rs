// SPDX-License-Identifier: GPL-3.0-or-later
//! Welding for materialised incidence. The mesh owns the live Python handles,
//! their disk sets and their registry, which may contain aliases to retired vertices.
use crate::math::xyz;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet, PyTuple},
};

pub(crate) fn rings<'py>(f: &Bound<'py, PyAny>) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let mut rings = vec![f.getattr("loop")?];
    for hole in f.getattr("hole_loops")?.try_iter()? {
        rings.push(hole?);
    }
    Ok(rings)
}

fn list<'py>(xs: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    Ok(xs.py().get_type::<PyList>().call1((xs,))?.cast_into()?)
}

fn append_missing(items: &Bound<'_, PyAny>, object: &Bound<'_, PyAny>) -> PyResult<()> {
    if !items.contains(object)? {
        items.call_method1("append", (object,))?;
    }
    Ok(())
}

#[pyfunction]
pub fn weld_object_vertex(
    mesh: &Bound<'_, PyAny>,
    keep: &Bound<'_, PyAny>,
    dup: &Bound<'_, PyAny>,
    key: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let py = mesh.py();
    let affected = PySet::empty(py)?;
    let mut disk = Vec::new();
    for e in dup.getattr("edges")?.try_iter()? {
        let edge = e?;
        for f in edge.getattr("faces")?.try_iter()? {
            affected.add(f?)?;
        }
        let other = edge.call_method1("other", (dup,))?;
        disk.push((xyz(&other.getattr("position")?)?.to_array(), other, edge));
    }
    let compare =
        |a: &[f64; 3], b: &[f64; 3]| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal);
    disk.sort_by(|a, b| compare(&a.0, &b.0));
    if disk.windows(2).any(|w| w[0].0 == w[1].0) {
        let order = PyDict::new(py);
        for (i, e) in mesh.getattr("edges")?.try_iter()?.enumerate() {
            order.set_item(e?.getattr("key")?, i)?;
        }
        let mut ranked = Vec::with_capacity(disk.len());
        for (point, v, e) in disk {
            let index = order
                .get_item(e.getattr("key")?)?
                .map(|i| i.extract::<isize>())
                .transpose()?
                .unwrap_or(-1);
            ranked.push((point, index, v, e));
        }
        ranked.sort_by(|a, b| compare(&a.0, &b.0).then_with(|| a.1.cmp(&b.1)));
        disk = ranked.into_iter().map(|(p, _, v, e)| (p, v, e)).collect();
    }
    for (_, other, edge) in disk {
        let incident = list(&edge.getattr("faces")?)?;
        mesh.call_method1("remove_edge", (&edge,))?;
        if other.is(keep) {
            continue;
        }
        let new = mesh.call_method1("find_edge", (keep, &other))?.is_none();
        let survivor = mesh.call_method1("_link_edge", (keep, &other))?;
        if new {
            survivor.setattr("key", edge.getattr("key")?)?;
        }
        let radial = survivor.getattr("faces")?;
        for f in incident.iter() {
            append_missing(&radial, &f)?;
        }
    }
    for f in affected.iter() {
        for ring in rings(&f)? {
            for i in 0..ring.len()? {
                if ring.get_item(i)?.is(dup) {
                    ring.set_item(i, keep)?;
                }
            }
            let mut i = 0;
            while ring.len()? > 1 && i < ring.len()? {
                let n = ring.len()?;
                let next = (i + 1) % n;
                if ring.get_item(i)?.is(ring.get_item(next)?) {
                    ring.del_item(if next == 0 { 0 } else { i })?;
                } else {
                    i += 1;
                }
            }
        }
        let distinct = PySet::empty(py)?;
        for v in f.getattr("loop")?.try_iter()? {
            distinct.add(v?.getattr("key")?)?;
        }
        if distinct.len() < 3 {
            mesh.call_method1("remove_face", (&f,))?;
        }
    }
    let vertices = mesh.getattr("vertices")?;
    if vertices.contains(dup)? {
        vertices.call_method1("remove", (dup,))?;
    }
    let registry = mesh.getattr("_registry")?;
    let cell = key.call1((dup.getattr("position")?,))?;
    if registry.call_method1("get", (&cell,))?.is(dup) {
        registry.set_item(cell, keep)?;
    }
    Ok(())
}

/// Cell identifiers stay Python integers at this boundary, including very
/// large model coordinates. Source order breaks ties between coincident keys.
#[pyfunction]
pub fn weld_object_mesh(
    mesh: &Bound<'_, PyAny>,
    key: &Bound<'_, PyAny>,
    tolerance: f64,
) -> PyResult<bool> {
    let py = mesh.py();
    let groups = PyDict::new(py);
    for v in mesh.getattr("vertices")?.try_iter()? {
        let v = v?;
        let cell = key.call1((v.getattr("position")?,))?;
        let bucket = if let Some(xs) = groups.get_item(&cell)? {
            xs.cast_into::<PyList>()?
        } else {
            let xs = PyList::empty(py);
            groups.set_item(cell, &xs)?;
            xs
        };
        bucket.append(v)?;
    }
    let mut changed = false;
    for (_, vs) in groups.iter() {
        for i in 1..vs.len()? {
            mesh.call_method1("_weld_into", (vs.get_item(0)?, vs.get_item(i)?))?;
            changed = true;
        }
    }
    let vertices = mesh.getattr("vertices")?;
    let listed = PySet::empty(py)?;
    for v in vertices.try_iter()? {
        listed.add(v?.getattr("key")?)?;
    }
    let reg = mesh.getattr("_registry")?;
    loop {
        let mut hit = None;
        'scan: for v in vertices.try_iter()? {
            let v = v?;
            let p = xyz(&v.getattr("position")?)?;
            let cell = key.call1((v.getattr("position")?,))?;
            if !reg.call_method1("get", (&cell,))?.is(&v) {
                continue;
            }
            let mut coords = Vec::with_capacity(3);
            for k in 0..3 {
                let j = cell.get_item(k)?;
                coords.push([j.sub(1)?, j.clone(), j.add(1)?]);
            }
            for (i, x) in coords[0].iter().enumerate() {
                for (j, y) in coords[1].iter().enumerate() {
                    for (k, z) in coords[2].iter().enumerate() {
                        if (i, j, k) == (1, 1, 1) {
                            continue;
                        }
                        let nc = PyTuple::new(py, [x, y, z])?;
                        let w = reg.call_method1("get", (&nc,))?;
                        if w.is_none() || w.is(&v) || !listed.contains(w.getattr("key")?)? {
                            continue;
                        }
                        if p.distance_squared(xyz(&w.getattr("position")?)?)
                            >= tolerance * tolerance
                        {
                            continue;
                        }
                        hit = Some(if (i, j, k) > (1, 1, 1) {
                            (v, w)
                        } else {
                            (w, v)
                        });
                        break 'scan;
                    }
                }
            }
        }
        let Some((keep, dup)) = hit else {
            break;
        };
        mesh.call_method1("_weld_into", (keep, &dup))?;
        listed.discard(dup.getattr("key")?)?;
        changed = true;
    }
    Ok(changed)
}

/// Create both halves before writing radial incidence. Only a newly allocated
/// first half inherits the source key.
fn halves<'py>(
    mesh: &Bound<'py, PyAny>,
    edge: &Bound<'py, PyAny>,
    a: &Bound<'py, PyAny>,
    b: &Bound<'py, PyAny>,
    mid: &Bound<'py, PyAny>,
) -> PyResult<[(Bound<'py, PyAny>, bool); 2]> {
    mesh.call_method1("remove_edge", (edge,))?;
    let new0 = mesh.call_method1("find_edge", (a, mid))?.is_none();
    let e0 = mesh.call_method1("_link_edge", (a, mid))?;
    if new0 {
        e0.setattr("key", edge.getattr("key")?)?;
    }
    let new1 = mesh.call_method1("find_edge", (mid, b))?.is_none();
    let e1 = mesh.call_method1("_link_edge", (mid, b))?;
    Ok([(e0, new0), (e1, new1)])
}

fn joins(
    p: &Bound<'_, PyAny>,
    q: &Bound<'_, PyAny>,
    a: &Bound<'_, PyAny>,
    b: &Bound<'_, PyAny>,
) -> bool {
    (p.is(a) && q.is(b)) || (p.is(b) && q.is(a))
}

#[pyfunction]
pub fn split_object_at_position<'py>(
    mesh: &Bound<'py, PyAny>,
    edge: &Bound<'py, PyAny>,
    position: &Bound<'py, PyAny>,
) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyAny>)> {
    let a = edge.getattr("v0")?;
    let b = edge.getattr("v1")?;
    let mid = mesh.call_method1("vertex", (position,))?;
    if mid.is(&a) || mid.is(&b) {
        return Ok((edge.clone(), edge.clone()));
    }
    let incident = PyDict::new(mesh.py());
    for f in edge.getattr("faces")?.try_iter()? {
        let f = f?;
        incident.set_item(f.getattr("key")?, f)?;
    }
    let [(e0, _), (e1, _)] = halves(mesh, edge, &a, &b, &mid)?;
    for (_, face) in incident.iter() {
        let loops = rings(&face)?;
        for ring in &loops {
            let mut i = 0;
            while i < ring.len()? {
                if joins(
                    &ring.get_item(i)?,
                    &ring.get_item((i + 1) % ring.len()?)?,
                    &a,
                    &b,
                ) {
                    ring.call_method1("insert", (i + 1, &mid))?;
                    i += 1;
                }
                i += 1;
            }
        }
        for half in [&e0, &e1] {
            let start = half.getattr("v0")?;
            let end = half.getattr("v1")?;
            let mut uses = 0;
            for ring in &loops {
                let n = ring.len()?;
                for i in 0..n {
                    if joins(
                        &ring.get_item(i)?,
                        &ring.get_item((i + 1) % n)?,
                        &start,
                        &end,
                    ) {
                        uses += 1;
                    }
                }
            }
            let radial = half.getattr("faces")?;
            let mut existing = 0;
            for f in radial.try_iter()? {
                if f?.is(&face) {
                    existing += 1;
                }
            }
            for _ in existing..uses {
                radial.call_method1("append", (&face,))?;
            }
        }
    }
    Ok((e0, e1))
}

#[pyfunction]
pub fn split_object_edge<'py>(
    mesh: &Bound<'py, PyAny>,
    edge: &Bound<'py, PyAny>,
    mid: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyDict>> {
    let a = edge.getattr("v0")?;
    let b = edge.getattr("v1")?;
    let incident = list(&edge.getattr("faces")?)?;
    let halves = halves(mesh, edge, &a, &b, mid)?;
    for (half, made) in &halves {
        if *made {
            for attr in ["soft", "flat", "layer", "curve"] {
                half.setattr(attr, edge.getattr(attr)?)?;
            }
        }
    }
    for f in incident.iter() {
        mesh.call_method1("_insert_into_face_loops", (&f, &a, &b, mid))?;
        for (half, _) in &halves {
            append_missing(&half.getattr("faces")?, &f)?;
        }
    }
    let result = PyDict::new(mesh.py());
    result.set_item("kind", "split")?;
    result.set_item("edge", edge)?;
    let [(e0, new0), (e1, new1)] = halves;
    result.set_item("e0", e0)?;
    result.set_item("e1", e1)?;
    result.set_item("e0_new", new0)?;
    result.set_item("e1_new", new1)?;
    result.set_item("mid", mid)?;
    result.set_item("faces", incident)?;
    Ok(result)
}

#[pyfunction]
pub fn revert_object_split(mesh: &Bound<'_, PyAny>, rec: &Bound<'_, PyAny>) -> PyResult<()> {
    let mid = rec.get_item("mid")?;
    let incident = rec.get_item("faces")?;
    for f in incident.try_iter()? {
        for ring in rings(&f?)? {
            if ring.contains(&mid)? {
                ring.call_method1("remove", (&mid,))?;
            }
        }
    }
    for (label, fresh) in [("e0", "e0_new"), ("e1", "e1_new")] {
        let half = rec.get_item(label)?;
        if rec.get_item(fresh)?.is_truthy()? {
            mesh.call_method1("remove_edge", (half,))?;
        } else {
            let radial = half.getattr("faces")?;
            for f in incident.try_iter()? {
                let f = f?;
                if radial.contains(&f)? {
                    radial.call_method1("remove", (f,))?;
                }
            }
        }
    }
    let edge = rec.get_item("edge")?;
    mesh.call_method1("relink_edge", (&edge,))?;
    let radial = edge.getattr("faces")?;
    for f in incident.try_iter()? {
        append_missing(&radial, &f?)?;
    }
    Ok(())
}
