// SPDX-License-Identifier: GPL-3.0-or-later
//! Collect source-authored conics once per definition. A curve group alone
//! carries no support. Recheck the endpoints after the ordinary store weld.
use crate::{
    exact::Frame,
    math::mat,
    store::{py::PyMeshStore, source_arcs::ArcBinding},
};
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use std::{
    collections::HashMap,
    f64::consts::{PI, TAU},
};

type Obj<'p> = Bound<'p, PyAny>;
pub(super) fn collect(d: &Obj<'_>, xf: &Obj<'_>, out: &Obj<'_>) -> PyResult<()> {
    let Some(arcs) = d
        .getattr_opt("arcs")?
        .filter(|a| a.is_truthy().unwrap_or(false))
    else {
        return Ok(());
    };
    let xf = mat(xf)?;
    let vertices = d.getattr("vertices")?;
    let vertices = vertices.cast::<PyDict>()?;
    let mut runs = HashMap::<u64, (Frame, f64, f64, f64, Vec<[[f64; 3]; 2]>)>::new();
    for (id, source) in arcs.cast::<PyDict>()? {
        let field = |k| source.getattr(k).or_else(|_| source.get_item(k));
        let vector =
            |k| -> PyResult<DVec3> { Ok(DVec3::from(field(k)?.extract::<[f64; 3]>()?) * 0.0254) };
        let origin = xf.transform_point3(vector("center")?);
        let x = xf.transform_vector3(vector("x_axis")?);
        let y = xf.transform_vector3(vector("y_axis")?);
        let radius = x.length();
        let Some(axis) = x.cross(y).try_normalize() else {
            continue;
        };
        if !origin.is_finite()
            || !radius.is_finite()
            || radius <= 0.
            || (y.length() - radius).abs() > radius * 1e-10
            || x.dot(y).abs() > radius * radius * 1e-10
        {
            continue;
        }
        let start: f64 = field("start_angle")?.extract()?;
        let end: f64 = field("end_angle")?.extract()?;
        if !start.is_finite() || !end.is_finite() {
            continue;
        }
        runs.insert(
            id.extract()?,
            (
                Frame {
                    origin,
                    axis,
                    reference: x / radius,
                },
                radius,
                start,
                end,
                vec![],
            ),
        );
    }
    let edges = d.getattr("edges")?;
    for (_, e) in edges.cast::<PyDict>()? {
        let Some(id) = e.getattr("curve_id")?.extract::<Option<u64>>()? else {
            continue;
        };
        let Some(run) = runs.get_mut(&id) else {
            continue;
        };
        if let Some(pair) = super::collect::ends(&e, vertices)? {
            run.4
                .push(pair.map(|p| xf.transform_point3(DVec3::from(p)).to_array()));
        }
    }
    for (_, (f, r, start, end, pairs)) in runs {
        if !pairs.is_empty() {
            out.call_method1("append", ((f.array(), r, start, end, pairs),))?;
        }
    }
    Ok(())
}

pub(super) fn bind(mesh: &Obj<'_>, rec: &Obj<'_>) -> PyResult<usize> {
    let Some(data) = rec.cast::<PyDict>()?.get_item("source_arcs")? else {
        return Ok(0);
    };
    let handle = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
    let mut held = handle.try_borrow_mut()?;
    let st = &mut held.inner;
    let mut count = 0;
    for row in data.cast::<PyList>()? {
        let (axes, radius, start, end, pairs): ([[f64; 3]; 3], f64, f64, f64, Vec<[[f64; 3]; 2]>) =
            row.extract()?;
        let frame = Frame::from_array(axes);
        if frame.validate().is_err() {
            continue;
        }
        let span = (end - start).rem_euclid(TAU);
        let full = span.min(TAU - span) < 1e-9;
        for [a, b] in pairs {
            let Some((a, b)) = st.lookup(a).zip(st.lookup(b)) else {
                continue;
            };
            let Some(e) = st.find_edge(a, b) else {
                continue;
            };
            let angles = st.state.ends.get(e as usize).map(|v| {
                let d = DVec3::from(*st.state.vpos.get(v as usize)) - frame.origin;
                d.dot(frame.y()).atan2(d.dot(frame.reference))
            });
            let dt = (angles[1] - angles[0] + PI).rem_euclid(TAU) - PI;
            if !full
                && [angles[0], angles[0] + dt * 0.5, angles[1]]
                    .iter()
                    .any(|t| {
                        let u = (t - start).rem_euclid(TAU);
                        u > span + 1e-8 && TAU - u > 1e-8
                    })
            {
                continue;
            }
            if st
                .bind_source_arc(
                    e,
                    ArcBinding {
                        fitted: false,
                        frame: frame.clone(),
                        radius,
                        interval: [angles[0], angles[0] + dt],
                    },
                )
                .is_ok()
            {
                count += 1;
            }
        }
    }
    Ok(count)
}
