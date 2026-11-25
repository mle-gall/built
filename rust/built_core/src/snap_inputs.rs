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
    Bound<'py, pyo3::types::PyList>,
    Vec<bool>,
);
#[pyfunction]
pub(crate) fn edge_columns<'py>(edges: &Obj<'py>) -> PyResult<Columns<'py>> {
    let data = Edges::read(edges)?;
    let points = |end: usize| {
        Array2::from_shape_fn((data.lines.len(), 3), |(i, j)| data.lines[i][end][j])
            .into_pyarray(edges.py())
    };
    Ok((
        points(0),
        points(1),
        pyo3::types::PyList::new(edges.py(), data.tags)?,
        data.grouped,
    ))
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

/// Use renderer-owned packed columns when supplied, or read the reference handles once.
#[pyfunction]
fn make_query(edges: &Obj<'_>, projector: &Obj<'_>, cursor: [f64; 2]) -> PyResult<SnapQuery> {
    let projection = projector
        .getattr_opt("snap_projection")?
        .filter(|p| !p.is_none());
    let (matrix, size, callback) = if let Some(p) = projection {
        let (m, s) = p.extract()?;
        (Some(m), Some(s), None)
    } else {
        (None, None, Some(projector.clone().unbind()))
    };
    if let Some(columns) = edges.getattr_opt("snap_columns")? {
        let columns = columns.call0()?;
        let (a, b, tags, grouped) = columns.extract()?;
        SnapQuery::new(a, b, tags, grouped, matrix, size, cursor, callback)
    } else {
        from_objects(
            edges,
            Projection::from_parts(matrix, size, callback)?,
            cursor,
        )
    }
}

#[pyfunction]
#[pyo3(signature=(query,edges,radius,hidden,*,midpoints=false,quadrants=None,centers=None,line=None,hidden_many=None))]
fn point_result<'py>(
    query: &Obj<'py>,
    edges: &Obj<'py>,
    radius: f64,
    hidden: Option<&Obj<'py>>,
    midpoints: bool,
    quadrants: Option<&Obj<'py>>,
    centers: Option<&Obj<'py>>,
    line: Option<super::Line>,
    hidden_many: Option<&Obj<'py>>,
) -> PyResult<Option<Obj<'py>>> {
    let py = query.py();
    let empty = pyo3::types::PyTuple::empty(py);
    let centers = centers.unwrap_or(empty.as_any());
    let (quads, extra) = point_inputs(quadrants.unwrap_or(empty.as_any()), centers)?;
    let visibility = if let Some(h) = hidden {
        if h.is_truthy()? {
            let batch = match hidden_many {
                Some(v) if v.is_truthy()? => Some(v.clone().unbind()),
                _ => None,
            };
            Some(
                crate::snap_visibility::PointVisibility::new(h.clone().unbind(), batch)
                    .into_pyobject(py)?,
            )
        } else {
            None
        }
    } else {
        None
    };
    let choice = if let Ok(native) = query.cast::<SnapQuery>() {
        native
            .borrow()
            .points(
                py,
                radius,
                midpoints,
                quads,
                extra,
                visibility.as_ref().map(|v| v.as_any()),
                line,
            )?
            .map(|(p, i, k)| (p, i, k.to_owned()))
    } else {
        query
            .call_method1(
                "points",
                (radius, midpoints, quads, extra, visibility, line),
            )?
            .extract::<Option<(P, i64, String)>>()?
    };
    let Some((p, i, kind)) = choice else {
        return Ok(None);
    };
    let context = if i >= 0 {
        edges.get_item(i)?.getattr_opt("context")?
    } else if i <= -2 {
        Some(centers.get_item(-2 - i)?.get_item(1)?)
    } else {
        None
    };
    let api = py.import("core.snap")?;
    let color = api.getattr(match kind.as_str() {
        "midpoint" | "quadrant" | "arc_midpoint" => "COLOR_MIDPOINT",
        "face_center" => "COLOR_ON_FACE",
        "origin" | "component_origin" => "COLOR_ORIGIN",
        _ => "COLOR_ENDPOINT",
    })?;
    let kw = pyo3::types::PyDict::new(py);
    kw.set_item("context", context)?;
    Ok(Some(api.getattr("SnapResult")?.call(
        (
            crate::math::Vec3::from(glam::DVec3::from_array(p)),
            kind,
            color,
        ),
        Some(&kw),
    )?))
}
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let sub = PyModule::new(m.py(), "core.snap_query")?;
    sub.add_function(wrap_pyfunction!(edge_tag, &sub)?)?;
    sub.add_function(wrap_pyfunction!(edge_columns, &sub)?)?;
    sub.add_function(wrap_pyfunction!(make_query, &sub)?)?;
    sub.add_function(wrap_pyfunction!(point_result, &sub)?)?;
    sub.add_class::<SnapQuery>()?;
    sub.add_class::<crate::snap_visibility::PointVisibility>()?;
    m.add_submodule(&sub)
}
