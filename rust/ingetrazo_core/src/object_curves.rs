// SPDX-License-Identifier: GPL-3.0-or-later
//! Contour queries over public mesh handles, including placed occurrences.
use crate::{
    math::xyz,
    store::{
        curve_groups::{chains, planar_branch},
        drawing::{path_segments, StrokeSegment},
    },
};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use std::collections::{HashMap, HashSet};

type Obj<'py> = Bound<'py, PyAny>;

#[pyfunction]
#[pyo3(signature=(pt, p, q, tol=1e-4))]
pub(crate) fn point_on_segment(pt: &Obj<'_>, p: &Obj<'_>, q: &Obj<'_>, tol: f64) -> PyResult<bool> {
    Ok(StrokeSegment::new(xyz(p)?, xyz(q)?).contains(xyz(pt)?, tol))
}

#[pyfunction]
pub(crate) fn tag_curve(
    edges: &Obj<'_>,
    points: &Obj<'_>,
    closed: bool,
    id: &Obj<'_>,
) -> PyResult<bool> {
    let py = edges.py();
    let path = points
        .try_iter()?
        .map(|p| Ok(xyz(&p?)?.to_array()))
        .collect::<PyResult<Vec<_>>>()?;
    if path.len() < 2 {
        return Ok(false);
    }
    let segments = path_segments(&path, closed);
    let mut changed = false;
    for e in edges.try_iter()? {
        let e = e?;
        if !e.getattr(pyo3::intern!(py, "curve"))?.is_none() {
            continue;
        }
        let a = xyz(&e.getattr(pyo3::intern!(py, "a"))?)?;
        let b = xyz(&e.getattr(pyo3::intern!(py, "b"))?)?;
        if segments
            .iter()
            .any(|s| s.contains(a, 1e-4) && s.contains(b, 1e-4))
        {
            e.setattr(pyo3::intern!(py, "curve"), id)?;
            changed = true;
        }
    }
    Ok(changed)
}

fn key(e: &Obj<'_>) -> PyResult<i128> {
    e.getattr(pyo3::intern!(e.py(), "key"))?.extract()
}

fn branch(v: &Obj<'_>, group: &HashSet<i128>) -> PyResult<bool> {
    let py = v.py();
    let mut mine = Vec::new();
    let mut other = Vec::new();
    for e in v.getattr(pyo3::intern!(py, "edges"))?.try_iter()? {
        let e = e?;
        if group.contains(&key(&e)?) {
            mine.push(e);
        } else {
            other.push(e);
        }
    }
    let [a, b] = mine.as_slice() else {
        return Ok(true);
    };
    if other.is_empty() {
        return Ok(false);
    }
    let p = xyz(&v.getattr(pyo3::intern!(py, "position"))?)?;
    let direction = |e: &Obj<'_>| {
        Ok(xyz(&e
            .call_method1("other", (v,))?
            .getattr(pyo3::intern!(py, "position"))?)?
            - p)
    };
    let others = other.iter().map(direction).collect::<PyResult<Vec<_>>>()?;
    Ok(planar_branch(direction(a)?, direction(b)?, &others))
}

#[pyfunction]
pub(crate) fn resplit_curves<'py>(edges: &Obj<'py>, mut next_id: Obj<'py>) -> PyResult<Obj<'py>> {
    let py = edges.py();
    let groups = PyDict::new(py);
    for e in edges.try_iter()? {
        let e = e?;
        let id = e.getattr(pyo3::intern!(py, "curve"))?;
        if id.is_none() {
            continue;
        }
        if let Some(rows) = groups.get_item(&id)? {
            rows.cast_into::<PyList>()?.append(e)?;
        } else {
            groups.set_item(id, PyList::new(py, [e])?)?;
        }
    }
    for (id, rows) in groups {
        let edges = rows.cast::<PyList>()?;
        let mut vertices = HashMap::new();
        let mut ends = Vec::new();
        let mut group = HashSet::new();
        for edge in edges {
            group.insert(key(&edge)?);
            let a = edge.getattr(pyo3::intern!(py, "v0"))?;
            let b = edge.getattr(pyo3::intern!(py, "v1"))?;
            let pair = [key(&a)?, key(&b)?];
            vertices.insert(pair[0], a);
            vertices.insert(pair[1], b);
            ends.push(pair);
        }
        let mut breaks = HashSet::new();
        for (k, v) in vertices {
            if branch(&v, &group)? {
                breaks.insert(k);
            }
        }
        let chains = chains(&ends, &breaks);
        if chains.len() <= 1 {
            continue;
        }
        for (i, chain) in chains.into_iter().enumerate() {
            let tag = if i == 0 {
                id.clone()
            } else {
                let tag = next_id.clone();
                next_id = next_id.add(1)?;
                tag
            };
            for i in chain {
                edges
                    .get_item(i)?
                    .setattr(pyo3::intern!(py, "curve"), &tag)?;
            }
        }
    }
    Ok(next_id)
}
