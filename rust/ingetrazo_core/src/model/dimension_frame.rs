// SPDX-License-Identifier: GPL-3.0-or-later
//! A linear dimension's frame maths (`core.dimension`): the frame of a
//! linear dimension, its measured value, the offset squared to the segment,
//! the dimension line's ends, and what the cursor asks for while placing
//! one. The record and its anchors stay Python.
//!
//! The same `glam` operations `Vec3` runs (`math.rs`), in the Python's order,
//! so the answers are bit for bit (`tests/test_dimension_frame_parity.py`).

use glam::DVec3;
use pyo3::prelude::*;

use super::pyfloat;
use crate::math::{xyz, Vec3};

/// Below this angle with an axis (or above its complement) no linear
/// dimension is offered.
const LINEAR_MIN_DEG: f64 = 15.0;
/// A pull within this angle of square off the segment asks for the aligned
/// dimension, however far it goes.
const ALIGNED_CONE_DEG: f64 = 20.0;

fn axis_vector(axis: Option<&str>) -> Option<DVec3> {
    match axis? {
        "x" => Some(DVec3::X),
        "y" => Some(DVec3::Y),
        "z" => Some(DVec3::Z),
        _ => None,
    }
}

/// `(u, e)`: the axis and the extension-line direction, or `None`.
pub fn linear_frame(a: DVec3, b: DVec3, axis: Option<&str>) -> Option<(DVec3, DVec3)> {
    let u = axis_vector(axis)?;
    let ab = b - a;
    let length = ab.length();
    if length < 1e-9 {
        return None;
    }
    let d = ab / length;
    let e = d - u * d.dot(u);
    if e.length() < 1e-6 {
        return None;
    }
    Some((u, e / e.length()))
}

pub fn value(a: DVec3, b: DVec3, axis: Option<&str>) -> f64 {
    match linear_frame(a, b, axis) {
        Some((u, _)) => (b - a).dot(u).abs(),
        None => (b - a).length(),
    }
}

pub fn perpendicular_offset(a: DVec3, b: DVec3, offset: DVec3, axis: Option<&str>) -> DVec3 {
    if let Some((_, e)) = linear_frame(a, b, axis) {
        return e * offset.dot(e);
    }
    let ab = b - a;
    let length = ab.length();
    if length < 1e-9 {
        return offset;
    }
    let dir = ab / length;
    offset - dir * offset.dot(dir)
}

pub fn line_points(a: DVec3, b: DVec3, offset: DVec3, axis: Option<&str>) -> (DVec3, DVec3) {
    if let Some((_, e)) = linear_frame(a, b, axis) {
        let t_a = offset.dot(e);
        let t_b = t_a - (b - a).dot(e);
        return (a + e * t_a, b + e * t_b);
    }
    let off = perpendicular_offset(a, b, offset, axis);
    (a + off, b + off)
}

pub fn offset_for_cursor(a: DVec3, b: DVec3, cursor: DVec3) -> DVec3 {
    let ab = b - a;
    let length = ab.length();
    if length < 1e-9 {
        return cursor - a;
    }
    let dir = ab / length;
    let to_cursor = cursor - a;
    to_cursor - dir * to_cursor.dot(dir)
}

pub fn placement_for_cursor(a: DVec3, b: DVec3, cursor: DVec3) -> (DVec3, Option<&'static str>) {
    let ab = b - a;
    let length = ab.length();
    if length < 1e-9 {
        return (cursor - a, None);
    }
    let d = ab / length;
    let aligned = offset_for_cursor(a, b, cursor);
    let w = cursor - (a + b) * 0.5;
    let along = w.dot(d).abs();
    let perp = (w - d * w.dot(d)).length();
    let rad = |deg: f64| deg * (std::f64::consts::PI / 180.0);
    if along <= rad(ALIGNED_CONE_DEG).tan() * perp {
        return (aligned, None);
    }
    let lo = pyfloat::sin(rad(LINEAR_MIN_DEG));
    let hi = pyfloat::cos(rad(LINEAR_MIN_DEG));
    let mut best: Option<(f64, &'static str, DVec3)> = None;
    let mut beyond = 0;
    for name in ["x", "y", "z"] {
        let Some((u, e)) = linear_frame(a, b, Some(name)) else { continue };
        let along = d.dot(u).abs();
        if along < lo || along > hi {
            continue;
        }
        let t = (cursor - a).dot(e);
        let span = ab.dot(e);
        let (low, high) = (0f64.min(span), 0f64.max(span));
        let over = if t < low { low - t } else if t > high { t - high } else { 0.0 };
        if over <= 1e-9 {
            continue;
        }
        beyond += 1;
        if best.is_none_or(|(o, _, _)| over > o) {
            best = Some((over, name, e * t));
        }
    }
    match best {
        Some((_, name, offset)) if beyond == 1 => (offset, Some(name)),
        _ => (aligned, None),
    }
}

fn v(py: Python<'_>, p: DVec3) -> PyResult<Py<Vec3>> {
    Py::new(py, Vec3::from(p))
}

#[pyfunction]
#[pyo3(signature = (a, b, axis=None))]
fn dimension_linear_frame(py: Python<'_>, a: &Bound<'_, PyAny>, b: &Bound<'_, PyAny>, axis: Option<&str>)
    -> PyResult<Option<(Py<Vec3>, Py<Vec3>)>> {
    linear_frame(xyz(a)?, xyz(b)?, axis).map(|(u, e)| Ok((v(py, u)?, v(py, e)?))).transpose()
}

#[pyfunction]
#[pyo3(signature = (a, b, axis=None))]
fn dimension_value(a: &Bound<'_, PyAny>, b: &Bound<'_, PyAny>, axis: Option<&str>) -> PyResult<f64> {
    Ok(value(xyz(a)?, xyz(b)?, axis))
}

#[pyfunction]
#[pyo3(signature = (a, b, offset, axis=None))]
fn dimension_perpendicular_offset(py: Python<'_>, a: &Bound<'_, PyAny>, b: &Bound<'_, PyAny>, offset: &Bound<'_, PyAny>,
                                  axis: Option<&str>) -> PyResult<Py<Vec3>> {
    v(py, perpendicular_offset(xyz(a)?, xyz(b)?, xyz(offset)?, axis))
}

#[pyfunction]
#[pyo3(signature = (a, b, offset, axis=None))]
fn dimension_line_points(py: Python<'_>, a: &Bound<'_, PyAny>, b: &Bound<'_, PyAny>, offset: &Bound<'_, PyAny>,
                         axis: Option<&str>) -> PyResult<(Py<Vec3>, Py<Vec3>)> {
    let (p, q) = line_points(xyz(a)?, xyz(b)?, xyz(offset)?, axis);
    Ok((v(py, p)?, v(py, q)?))
}

/// The dimension line's midpoint.
#[pyfunction]
#[pyo3(signature = (a, b, offset, axis=None))]
fn dimension_midpoint(py: Python<'_>, a: &Bound<'_, PyAny>, b: &Bound<'_, PyAny>, offset: &Bound<'_, PyAny>,
                      axis: Option<&str>) -> PyResult<Py<Vec3>> {
    let (p, q) = line_points(xyz(a)?, xyz(b)?, xyz(offset)?, axis);
    v(py, (p + q) * 0.5)
}

#[pyfunction]
fn dimension_offset_for_cursor(py: Python<'_>, a: &Bound<'_, PyAny>, b: &Bound<'_, PyAny>, cursor: &Bound<'_, PyAny>)
    -> PyResult<Py<Vec3>> {
    v(py, offset_for_cursor(xyz(a)?, xyz(b)?, xyz(cursor)?))
}

#[pyfunction]
fn dimension_placement_for_cursor(py: Python<'_>, a: &Bound<'_, PyAny>, b: &Bound<'_, PyAny>, cursor: &Bound<'_, PyAny>)
    -> PyResult<(Py<Vec3>, Option<&'static str>)> {
    let (offset, axis) = placement_for_cursor(xyz(a)?, xyz(b)?, xyz(cursor)?);
    Ok((v(py, offset)?, axis))
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(dimension_linear_frame, m)?)?;
    m.add_function(wrap_pyfunction!(dimension_value, m)?)?;
    m.add_function(wrap_pyfunction!(dimension_perpendicular_offset, m)?)?;
    m.add_function(wrap_pyfunction!(dimension_line_points, m)?)?;
    m.add_function(wrap_pyfunction!(dimension_midpoint, m)?)?;
    m.add_function(wrap_pyfunction!(dimension_offset_for_cursor, m)?)?;
    m.add_function(wrap_pyfunction!(dimension_placement_for_cursor, m)?)?;
    Ok(())
}
