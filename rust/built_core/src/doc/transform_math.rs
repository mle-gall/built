// SPDX-License-Identifier: GPL-3.0-or-later
//! The matrices the transform commands and tools share: rotation about an
//! axis, reflection about a plane, scale about a centre (along the world's
//! axes or a frame's), and the words for a refused weld.
use crate::math::{xyz, Matrix4};
use glam::{DMat4, DVec3};
use pyo3::exceptions::PyKeyError;
use pyo3::prelude::*;
use pyo3::types::{PyList, PyTuple};

type Obj<'py> = Bound<'py, PyAny>;

/// Rigid rotation of `degrees` around `axis` through `centre`.
pub(crate) fn rotation(centre: DVec3, axis: DVec3, degrees: f64) -> DMat4 {
    // Normalised twice, as the Python helper did (once itself, once in
    // `Matrix4.rotate`): the same bits as every recorded rotation.
    let axis = axis.normalize_or_zero().normalize_or_zero();
    let mut m = DMat4::from_translation(centre);
    if axis != DVec3::ZERO {
        m *= DMat4::from_axis_angle(axis, degrees.to_radians());
    }
    m * DMat4::from_translation(-centre)
}

/// Reflection about the plane through `centre` with normal `axis` (Householder).
pub(crate) fn mirror(centre: DVec3, axis: DVec3) -> DMat4 {
    let n = axis.normalize_or_zero();
    let col = |i: usize| {
        let mut c = -2.0 * n[i] * n;
        c[i] += 1.0;
        c.extend(0.0)
    };
    let r = DMat4::from_cols(col(0), col(1), col(2), glam::DVec4::W);
    DMat4::from_translation(centre) * r * DMat4::from_translation(-centre)
}

/// Scale by `factors` about `centre`, along `axes` (red, green, blue) when
/// given, else along the world's.
pub(crate) fn scale(centre: DVec3, factors: DVec3, axes: Option<[DVec3; 3]>) -> DMat4 {
    let s = DMat4::from_scale(factors);
    match axes {
        Some([x, y, z]) => {
            let rot = DMat4::from_cols(x.extend(0.0), y.extend(0.0), z.extend(0.0), glam::DVec4::W);
            DMat4::from_translation(centre) * rot * s * rot.transpose() * DMat4::from_translation(-centre)
        }
        None => DMat4::from_translation(centre) * s * DMat4::from_translation(-centre),
    }
}

fn wrap<'py>(py: Python<'py>, m: DMat4) -> PyResult<Obj<'py>> {
    Ok(Bound::new(py, Matrix4::from(m))?.into_any())
}

/// A scale factor as given: one number for every axis, or three.
fn factors(factor: &Obj<'_>) -> PyResult<(DVec3, bool)> {
    if factor.is_instance_of::<PyTuple>() || factor.is_instance_of::<PyList>() {
        let f: (f64, f64, f64) = (factor.get_item(0)?.extract()?, factor.get_item(1)?.extract()?, factor.get_item(2)?.extract()?);
        Ok((DVec3::new(f.0, f.1, f.2), true))
    } else {
        Ok((DVec3::splat(factor.extract()?), false))
    }
}

/// Rigid rotation of `degrees` around `axis` through `center`.
#[pyfunction]
pub(crate) fn rotation_matrix<'py>(py: Python<'py>, center: &Obj<'py>, axis: &Obj<'py>, degrees: f64) -> PyResult<Obj<'py>> {
    wrap(py, rotation(xyz(center)?, xyz(axis)?, degrees))
}

/// Reflection about the plane through `centre` with normal `axis`: the
/// Flip tool.
#[pyfunction]
pub(crate) fn mirror_matrix<'py>(py: Python<'py>, centre: &Obj<'py>, axis: &Obj<'py>) -> PyResult<Obj<'py>> {
    wrap(py, mirror(xyz(centre)?, xyz(axis)?))
}

/// Scale about `center`: a number scales uniformly, three scale each axis
/// by its own factor (a negative one mirrors through the centre along that
/// axis). `axes` = `(red, green, blue)` unit vectors: the three factors act
/// along those instead of the world's (the grip box of a turned group).
#[pyfunction]
#[pyo3(signature = (center, factor, axes = None))]
pub(crate) fn scale_matrix<'py>(py: Python<'py>, center: &Obj<'py>, factor: &Obj<'py>, axes: Option<&Obj<'py>>) -> PyResult<Obj<'py>> {
    let (f, per_axis) = factors(factor)?;
    let frame = match axes.filter(|a| !a.is_none() && per_axis) {
        Some(a) => Some([xyz(&a.get_item(0)?)?, xyz(&a.get_item(1)?)?, xyz(&a.get_item(2)?)?]),
        None => None,
    };
    wrap(py, scale(xyz(center)?, f, frame))
}

/// The factor that undoes `factor`, per axis for three. A zero factor
/// (refused upstream) inverts to 1 instead of dividing by zero.
#[pyfunction]
pub(crate) fn invert_scale_factor<'py>(py: Python<'py>, factor: &Obj<'py>) -> PyResult<Obj<'py>> {
    let inv = |f: f64| if f.abs() > 1e-12 { 1.0 / f } else { 1.0 };
    let (f, per_axis) = factors(factor)?;
    if per_axis {
        Ok(PyTuple::new(py, [inv(f.x), inv(f.y), inv(f.z)])?.into_any())
    } else {
        Ok(inv(f.x).into_pyobject(py)?.into_any())
    }
}

/// The words a refused weld uses for a defect kind, in the user's language.
#[pyfunction]
pub(crate) fn defect_name(py: Python<'_>, kind: &str) -> PyResult<String> {
    let text = match kind {
        "duplicate_face" => "a face doubled over another",
        "overlapping_face" => "two faces overlapping in one plane",
        "pinched_face" => "a face that touches itself",
        "crossed_face" => "a face whose sides cross",
        "flat_face" => "a face with no area",
        "zero_edge" => "an edge of zero length",
        other => return Err(PyKeyError::new_err(other.to_owned())),
    };
    py.import("built_core.i18n")?.getattr("tr")?.call1((text,))?.extract()
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(rotation_matrix, m)?)?;
    m.add_function(wrap_pyfunction!(mirror_matrix, m)?)?;
    m.add_function(wrap_pyfunction!(scale_matrix, m)?)?;
    m.add_function(wrap_pyfunction!(invert_scale_factor, m)?)?;
    m.add_function(wrap_pyfunction!(defect_name, m)?)
}
