// SPDX-License-Identifier: GPL-3.0-or-later
//! Resolve selectable document handles at viewport pixels.
use crate::document_host::*;
use glam::DVec3;
use numpy::{IntoPyArray, PyReadonlyArray1};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySlice, PyTuple},
};
type Point = (f64, f64);
fn class(py: Python) -> PyResult<Obj> {
    host(py)?.getattr("DocumentInteraction")
}
fn index<'py>(v: &Obj<'py>, near: &Obj<'py>) -> PyResult<Obj<'py>> {
    let kw = PyDict::new(v.py());
    kw.set_item("near", near)?;
    v.call_method("_pick_index", (), Some(&kw))
}
fn ray_index<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    index(v, &class(v.py())?.call_method1("_ray_near", (v, x, y))?)
}
fn ray<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Option<(Obj<'py>, Obj<'py>)>> {
    let (o, d): (Obj, Obj) = v.call_method1("_pixel_to_ray", (x, y))?.extract()?;
    Ok((!o.is_none() && !d.is_none()).then_some((o, d)))
}
fn project(v: &Obj, p: &Obj) -> PyResult<Option<Point>> {
    v.call_method1("_world_to_pixel", (p,))?.extract()
}
#[pyfunction]
fn point_segment_distance(point: Point, start: Point, end: Point) -> f64 {
    let (x, y) = point;
    let (ax, ay) = start;
    let (bx, by) = end;
    let (dx, dy) = (bx - ax, by - ay);
    let l = dx * dx + dy * dy;
    let t = if l == 0. {
        0.
    } else {
        (((x - ax) * dx + (y - ay) * dy) / l).clamp(0., 1.)
    };
    (x - (ax + t * dx)).hypot(y - (ay + t * dy))
}
fn coplanar(v: &Obj, hits: &Obj, o: &Obj, d: &Obj, loose_only: bool) -> PyResult<Vec<usize>> {
    let depths = hits.getattr("t")?.extract::<PyReadonlyArray1<f64>>()?;
    let t = depths.as_array();
    let mask = if loose_only {
        Some(hits.getattr("loose")?.extract::<PyReadonlyArray1<bool>>()?)
    } else {
        None
    };
    let mask = mask.as_ref().map(|a| a.as_array());
    let allowed = |i| {
        mask.as_ref()
            .is_none_or(|m| m.get(i).copied().unwrap_or(false))
    };
    let mut nearest = f64::INFINITY;
    for (i, &t) in t.iter().enumerate() {
        if allowed(i) {
            if t.is_nan() {
                return Ok(Vec::new());
            }
            nearest = nearest.min(t)
        }
    }
    if !nearest.is_finite() {
        return Ok(Vec::new());
    }
    let tol = v
        .call_method1("_coplanar_eps", (o, d, nearest))?
        .extract::<f64>()?;
    Ok(t.iter()
        .enumerate()
        .filter(|&(i, &t)| allowed(i) && t <= nearest + tol)
        .map(|(i, _)| i)
        .collect())
}
fn choose(v: &Obj, hits: &Obj, o: &Obj, d: &Obj, loose: bool) -> PyResult<Option<usize>> {
    if hits.is_none() {
        return Ok(None);
    }
    let mut ids = coplanar(v, hits, o, d, loose)?.into_iter();
    let Some(mut best) = ids.next() else {
        return Ok(None);
    };
    if ids.len() > 0 {
        let a = hits.getattr("area")?.extract::<PyReadonlyArray1<f64>>()?;
        let a = a.as_array();
        for i in ids {
            if !a[best].is_nan() && (a[i] < a[best] || a[i].is_nan()) {
                best = i
            }
        }
    }
    Ok(Some(best))
}
mod annotations;
mod faces;
mod vertices;
pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(point_segment_distance, m)?)?;
    faces::register(m)?;
    vertices::register(m)?;
    annotations::register(m)
}
