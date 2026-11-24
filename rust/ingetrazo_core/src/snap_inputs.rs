// SPDX-License-Identifier: GPL-3.0-or-later
//! Materialise fallback edge references once. Packed viewport columns bypass this path.
use super::{Projection, SnapQuery, P};
use crate::math::xyz;
use numpy::{ndarray::Array2, IntoPyArray, PyArray2};
use pyo3::{prelude::*, types::PyString};

type Obj<'py> = Bound<'py, PyAny>;
fn flag(e: &Obj<'_>, field: &Bound<'_, PyString>) -> PyResult<bool> {
    Ok(match e.getattr_opt(field)? {
        Some(v) => v.is_truthy()?,
        None => false,
    })
}

#[pyfunction]
pub(crate) fn edge_tag(e: &Obj<'_>) -> PyResult<u8> {
    let py = e.py();
    let mut tags = if flag(e, pyo3::intern!(py, "figure"))? {
        8
    } else {
        0
    };
    if flag(e, pyo3::intern!(py, "soft"))?
        || flag(e, pyo3::intern!(py, "hidden"))?
        || e.getattr_opt(pyo3::intern!(py, "curve"))?
            .is_some_and(|v| !v.is_none())
    {
        tags |= 16;
    }
    for (tag, field) in [
        (2, pyo3::intern!(py, "center")),
        (3, pyo3::intern!(py, "component_origin")),
        (4, pyo3::intern!(py, "arc_midpoint")),
        (1, pyo3::intern!(py, "guide")),
    ] {
        if flag(e, field)? {
            return Ok(tags | tag);
        }
    }
    Ok(tags)
}

struct Edges {
    lines: Vec<[P; 2]>,
    tags: Vec<u8>,
    grouped: Vec<bool>,
}
impl Edges {
    fn read(input: &Obj<'_>) -> PyResult<Self> {
        let py = input.py();
        let mut out = Self {
            lines: Vec::new(),
            tags: Vec::new(),
            grouped: Vec::new(),
        };
        for e in input.try_iter()? {
            let e = e?;
            let a = xyz(&e.getattr(pyo3::intern!(py, "a"))?)?;
            let b = xyz(&e.getattr(pyo3::intern!(py, "b"))?)?;
            out.lines.push([a.to_array(), b.to_array()]);
            out.tags.push(edge_tag(&e)?);
            out.grouped.push(
                e.getattr_opt(pyo3::intern!(py, "context"))?
                    .is_some_and(|v| !v.is_none()),
            );
        }
        Ok(out)
    }
}

type Columns<'py> = (
    Bound<'py, PyArray2<f64>>,
    Bound<'py, PyArray2<f64>>,
    Vec<u8>,
    Vec<bool>,
);
#[pyfunction]
pub(crate) fn edge_columns<'py>(edges: &Obj<'py>) -> PyResult<Columns<'py>> {
    let data = Edges::read(edges)?;
    let points = |end: usize| {
        Array2::from_shape_fn((data.lines.len(), 3), |(i, j)| data.lines[i][end][j])
            .into_pyarray(edges.py())
    };
    Ok((points(0), points(1), data.tags, data.grouped))
}

pub(super) fn from_objects(
    edges: &Obj<'_>,
    projection: Projection,
    cursor: [f64; 2],
) -> PyResult<SnapQuery> {
    let data = Edges::read(edges)?;
    Ok(SnapQuery {
        edges: data.lines,
        tags: data.tags,
        grouped: data.grouped,
        projection,
        cursor,
    })
}

#[pyfunction]
pub(crate) fn point_inputs(
    quadrants: &Obj<'_>,
    centers: &Obj<'_>,
) -> PyResult<(Vec<P>, Vec<(P, bool)>)> {
    let mut extras = Vec::new();
    for c in centers.try_iter()? {
        let c = c?;
        extras.push((xyz(&c.get_item(0)?)?.to_array(), !c.get_item(1)?.is_none()));
    }
    let quads = quadrants
        .try_iter()?
        .map(|p| Ok(xyz(&p?)?.to_array()))
        .collect::<PyResult<_>>()?;
    Ok((quads, extras))
}
