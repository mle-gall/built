// SPDX-License-Identifier: GPL-3.0-or-later
//! Coplanar union and edge dissolves against a materialised mesh's live handles.
use crate::{
    math::xyz,
    store::stitch::{splice_loops, trace_loops},
};
use pyo3::{
    exceptions::PyIndexError,
    prelude::*,
    types::{PyDict, PyFrozenSet, PyList, PySet},
};
use std::collections::HashMap;
type PyObj<'py> = Bound<'py, PyAny>;

#[pyfunction]
pub fn trace_object_loops<'py>(outline: &PyObj<'py>) -> PyResult<Option<Vec<Vec<PyObj<'py>>>>> {
    let indices = PyDict::new(outline.py());
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for pair in outline.try_iter()? {
        let pair = pair?;
        let mut ends = [0; 2];
        for (i, slot) in ends.iter_mut().enumerate() {
            let v = pair.get_item(i)?;
            let k = v.getattr("key")?;
            *slot = if let Some(n) = indices.get_item(&k)? {
                n.extract()?
            } else {
                let n = nodes.len() as u32;
                indices.set_item(k, n)?;
                nodes.push(v);
                n
            };
        }
        edges.push((ends[0], ends[1]));
    }
    Ok(trace_loops(&edges).map(|rings| {
        rings
            .into_iter()
            .map(|r| r.into_iter().map(|v| nodes[v as usize].clone()).collect())
            .collect()
    }))
}

fn add_union<'py>(
    mesh: &PyObj<'py>,
    faces: &Bound<'py, PyList>,
    edges: &Bound<'py, PyList>,
    outer: &PyObj<'py>,
    holes: &PyObj<'py>,
    only_bare: bool,
    dominant: &PyObj<'py>,
) -> PyResult<PyObj<'py>> {
    let py = mesh.py();
    let positions = |r: &PyObj<'py>| {
        r.try_iter()?
            .map(|v| v?.getattr("position"))
            .collect::<PyResult<Vec<_>>>()
    };
    let outer = positions(outer)?;
    let holes = holes
        .try_iter()?
        .map(|r| positions(&r?))
        .collect::<PyResult<Vec<_>>>()?;
    for f in faces.iter() {
        mesh.call_method1("remove_face", (f,))?;
    }
    for e in edges.iter() {
        if !only_bare || e.getattr("faces")?.is_empty()? {
            mesh.call_method1("remove_edge", (e,))?;
        }
    }
    let result = mesh.call_method1("add_face", (outer, (!holes.is_empty()).then_some(holes)))?;
    result.setattr(
        "attrs",
        py.get_type::<PyDict>()
            .call1((dominant.call1((faces,))?,))?,
    )?;
    Ok(result)
}

#[pyfunction]
pub fn dissolve_object_region<'py>(
    mesh: &PyObj<'py>,
    faces: &Bound<'py, PyList>,
    pair: bool,
    trace: &PyObj<'py>,
    area: &PyObj<'py>,
    orient: &PyObj<'py>,
    dominant: &PyObj<'py>,
) -> PyResult<Option<PyObj<'py>>> {
    let py = mesh.py();
    if faces.len() < 2 {
        return Ok(None);
    }
    let first = faces.get_item(0)?;
    if pair && first.is(faces.get_item(1)?) {
        return Ok(None);
    }
    let normal = first.call_method0("normal")?;
    let n = xyz(&normal)?;
    for f in faces.iter().skip(1) {
        let dot = n.dot(xyz(&f.call_method0("normal")?)?);
        if (if pair { dot } else { dot.abs() }) < 0.999 {
            return Ok(None);
        }
    }
    let counts = PyDict::new(py);
    let nodes = PyDict::new(py);
    let mut segments = Vec::new();
    for f in faces.iter() {
        for ring in crate::object_weld::rings(&f)? {
            let size = ring.len()?;
            for i in 0..size {
                let a = ring.get_item(i)?;
                let b = ring.get_item((i + 1) % size)?;
                let ka = a.getattr("key")?;
                nodes.set_item(&ka, &a)?;
                let side = PyFrozenSet::new(py, [ka, b.getattr("key")?])?;
                let count = counts
                    .get_item(&side)?
                    .map(|v| v.extract::<usize>())
                    .transpose()?
                    .unwrap_or(0);
                counts.set_item(&side, count + 1)?;
                segments.push((a, b, side));
            }
        }
    }
    let shared = counts
        .iter()
        .filter_map(|(key, n)| match n.extract::<usize>() {
            Ok(1) => None,
            Ok(_) => Some(Ok(key)),
            Err(e) => Some(Err(e)),
        })
        .collect::<PyResult<Vec<_>>>()?;
    if shared.is_empty() {
        return Ok(None);
    }
    let outline = PyList::empty(py);
    for (a, b, key) in segments {
        if counts.get_item(key)?.unwrap().extract::<usize>()? == 1 {
            outline.append((a, b))?;
        }
    }
    let loops = trace.call1((outline,))?;
    if loops.is_none() {
        return Ok(None);
    }
    if loops.is_empty()? {
        if pair {
            return Err(PyIndexError::new_err("list index out of range"));
        }
        let rest = faces.get_slice(1, faces.len());
        if !first.getattr("attrs")?.is_truthy()? {
            first.setattr(
                "attrs",
                py.get_type::<PyDict>()
                    .call1((dominant.call1((&rest,))?,))?,
            )?;
        }
        for f in rest.iter() {
            mesh.call_method1("remove_face", (f,))?;
        }
        return Ok(Some(first));
    }
    // Keep the tracer's ordering for equal-area holes.
    let mut ranked = Vec::new();
    for ring in loops.try_iter()? {
        let ring = ring?;
        ranked.push((area.call1((&ring,))?.extract::<f64>()?, ring));
    }
    ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let outer = &ranked[0].1;
    let holes = PyList::new(py, ranked.iter().skip(1).map(|(_, r)| r))?;
    orient.call1((outer, &holes, &normal))?;
    let edges = PyList::empty(py);
    for side in shared {
        let keys = side.try_iter()?.collect::<PyResult<Vec<_>>>()?;
        let a = nodes.get_item(&keys[0])?.unwrap();
        let b = nodes.get_item(keys.last().unwrap())?.unwrap();
        let e = mesh.call_method1("find_edge", (a, b))?;
        if !e.is_none() {
            edges.append(e)?;
        }
    }
    let merged = add_union(mesh, faces, &edges, outer, &holes, !pair, dominant)?;
    if !pair {
        return Ok(Some(merged));
    }
    let rec = PyDict::new(py);
    rec.set_item("kind", "merge_pair")?;
    rec.set_item("face_a", first)?;
    rec.set_item("face_b", faces.get_item(1)?)?;
    rec.set_item("edges", edges)?;
    rec.set_item("merged", merged)?;
    Ok(Some(rec.into_any()))
}

#[pyfunction]
pub fn dissolve_object_edge<'py>(
    mesh: &PyObj<'py>,
    edge: &PyObj<'py>,
    record: bool,
    dominant: &PyObj<'py>,
) -> PyResult<Option<PyObj<'py>>> {
    let py = mesh.py();
    let faces = edge.getattr("faces")?;
    if faces.len()? != 2 {
        return Ok(None);
    }
    let a = faces.get_item(0)?;
    let b = faces.get_item(1)?;
    if a.is(&b) {
        return Ok(None);
    }
    let dot = xyz(&a.call_method0("normal")?)?.dot(xyz(&b.call_method0("normal")?)?);
    if (if record { dot } else { dot.abs() }) < 0.999 {
        return Ok(None);
    }
    let mut by_handle = HashMap::new();
    let mut nodes = Vec::new();
    let mut index = |v: PyObj<'py>| {
        *by_handle.entry(v.as_ptr()).or_insert_with(|| {
            let i = nodes.len() as u32;
            nodes.push(v);
            i
        })
    };
    let mut ring = |f: &PyObj<'py>| {
        f.getattr("loop")?
            .try_iter()?
            .map(|v| Ok(index(v?)))
            .collect::<PyResult<Vec<_>>>()
    };
    let left = ring(&a)?;
    let mut right = ring(&b)?;
    if dot < 0. {
        right.reverse();
    }
    let Some(merged) = splice_loops(
        &left,
        &right,
        index(edge.getattr("v0")?),
        index(edge.getattr("v1")?),
    ) else {
        return Ok(None);
    };
    let outer = PyList::new(py, merged.iter().map(|&v| &nodes[v as usize]))?;
    let unique = PySet::empty(py)?;
    for v in outer.iter() {
        unique.add(v.getattr("key")?)?;
    }
    if unique.len() != outer.len() {
        return Ok(None);
    }
    let holes = PyList::empty(py);
    for face in [&a, &b] {
        for h in face.getattr("hole_loops")?.try_iter()? {
            holes.append(h?)?;
        }
    }
    let merged = add_union(
        mesh,
        &PyList::new(py, [&a, &b])?,
        &PyList::new(py, [edge])?,
        &outer,
        &holes,
        false,
        dominant,
    )?;
    if !record {
        return Ok(Some(merged));
    }
    let rec = PyDict::new(py);
    rec.set_item("kind", "merge")?;
    rec.set_item("edge", edge)?;
    rec.set_item("face_a", &a)?;
    rec.set_item("face_b", &b)?;
    rec.set_item("merged", merged)?;
    Ok(Some(rec.into_any()))
}

fn seed_cells(seeds: Vec<[f64; 3]>) -> std::collections::HashSet<[u64; 3]> {
    seeds
        .into_iter()
        .map(crate::store::commands::supplied_cell)
        .collect()
}

#[pyfunction]
pub fn merge_seeded_edges(
    mesh: &PyObj<'_>,
    seeds: Vec<[f64; 3]>,
    records: &PyObj<'_>,
    store_of: &PyObj<'_>,
    edge_view: &PyObj<'_>,
) -> PyResult<usize> {
    let cells = seed_cells(seeds.clone());
    let mut count = 0;
    loop {
        let store = store_of.call1((mesh,))?;
        let edges = if store.is_none() {
            mesh.getattr("edges")?
        } else {
            store.call_method1("seeded_merge_edges", (&seeds,))?
        };
        let mut changed = false;
        for row in edges.try_iter()? {
            let row = row?;
            let e = if store.is_none() {
                row
            } else {
                edge_view.call1((mesh, row))?
            };
            let faces = e.getattr("faces")?;
            if faces.len()? != 2 {
                continue;
            }
            if store.is_none() {
                let cell = |end: &str| -> PyResult<_> {
                    Ok(crate::store::commands::cell(
                        xyz(&e.getattr(end)?.getattr("position")?)?.to_array(),
                    ))
                };
                if !cells.contains(&cell("v0")?) && !cells.contains(&cell("v1")?) {
                    continue;
                }
            }
            let (a, b) = (faces.get_item(0)?, faces.get_item(1)?);
            let joined = mesh.call_method1("dissolve_edge", (&e,))?;
            if !joined.is_none() {
                if !records.is_none() {
                    records.call_method1("append", ((a, b, e, joined),))?;
                }
                count += 1;
                changed = true;
                break;
            }
        }
        if !changed {
            return Ok(count);
        }
    }
}

fn restore_union<'py>(
    mesh: &PyObj<'py>,
    a: &PyObj<'py>,
    b: &PyObj<'py>,
    edges: impl IntoIterator<Item = PyObj<'py>>,
    merged: &PyObj<'py>,
) -> PyResult<()> {
    mesh.call_method1("remove_face", (merged,))?;
    for edge in edges {
        mesh.call_method1("relink_edge", (edge,))?;
    }
    mesh.call_method1("relink_face", (a,))?;
    mesh.call_method1("relink_face", (b,))?;
    Ok(())
}

#[pyfunction]
pub fn revert_recorded_union<'py>(mesh: &PyObj<'py>, rec: &PyObj<'py>, pair: bool) -> PyResult<()> {
    let edges = if pair {
        rec.get_item("edges")?
            .try_iter()?
            .collect::<PyResult<Vec<_>>>()?
    } else {
        vec![rec.get_item("edge")?]
    };
    restore_union(
        mesh,
        &rec.get_item("face_a")?,
        &rec.get_item("face_b")?,
        edges,
        &rec.get_item("merged")?,
    )
}

#[pyfunction]
pub fn revert_seeded_merges<'py>(mesh: &PyObj<'py>, records: &Bound<'py, PyList>) -> PyResult<()> {
    for r in records.iter().rev() {
        restore_union(
            mesh,
            &r.get_item(0)?,
            &r.get_item(1)?,
            [r.get_item(2)?],
            &r.get_item(3)?,
        )?;
    }
    Ok(())
}
