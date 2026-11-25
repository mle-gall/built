// SPDX-License-Identifier: GPL-3.0-or-later
//! Editable profiles and the kernel's section samples share one set of controls.
use crate::store::fillet::{sample_profile, valid_controls, CORNICE, CROWN};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyList, PyTuple},
};
type Control = (f64, f64, bool);
type Obj<'p> = Bound<'p, PyAny>;
pub(crate) fn read(points: &Obj) -> PyResult<Vec<Control>> {
    let float = points.py().import("builtins")?.getattr("float")?;
    points
        .try_iter()?
        .map(|p| {
            let row = p?.try_iter()?.collect::<PyResult<Vec<_>>>()?;
            let [x, y, s] = row.as_slice() else {
                return Err(PyValueError::new_err("expected x, y and smooth"));
            };
            Ok((
                float.call1((x,))?.extract()?,
                float.call1((y,))?.extract()?,
                s.is_truthy()?,
            ))
        })
        .collect()
}
#[pyfunction]
fn valid(controls: &Obj) -> bool {
    read(controls).is_ok_and(|r| valid_controls(&r))
}
fn sampled(name: &str, points: &[Control], count: isize) -> PyResult<Vec<(f64, f64)>> {
    sample_profile(name, points, count.max(1) as usize)
        .map_err(|(msg, _)| PyValueError::new_err(msg))
}
#[pyfunction]
fn controls(shape: &Obj, segments: isize) -> PyResult<Vec<Control>> {
    if shape.is_instance_of::<PyList>() || shape.is_instance_of::<PyTuple>() {
        if let Ok(pts) = read(shape) {
            if valid_controls(&pts) {
                return Ok(pts);
            }
        }
    }
    let name = shape.extract::<String>().unwrap_or_default();
    match name.as_str() {
        "cornice" => Ok(CORNICE.to_vec()),
        "crown" => Ok(CROWN.to_vec()),
        "steps" => Ok(sampled("steps", &[], segments)?
            .into_iter()
            .map(|(x, y)| (x, y, false))
            .collect()),
        shape => {
            let mut points = vec![(1., 0., false)];
            let support = shape == "supports";
            if support {
                points.push((1., 0.5, false));
            }
            for k in 1..=3 {
                let a = std::f64::consts::FRAC_PI_8 * f64::from(k);
                let (x, y) = (a.cos(), a.sin());
                points.push(if support {
                    (0.5 + x * 0.5, 0.5 + y * 0.5, true)
                } else {
                    (x, y, true)
                });
            }
            if support {
                points.push((0.5, 1., false));
            }
            points.push((0., 1., false));
            Ok(points)
        }
    }
}
#[pyfunction]
fn sample(shape: &Obj, segments: isize) -> PyResult<Vec<(f64, f64)>> {
    if shape.is_instance_of::<PyList>() || shape.is_instance_of::<PyTuple>() {
        return sampled("custom", &controls(shape, segments)?, segments);
    }
    let name = shape.extract::<String>().unwrap_or_default();
    if !["steps", "supports", "cornice", "crown"].contains(&name.as_str()) {
        return Err(PyValueError::new_err(shape.clone().unbind()));
    }
    sampled(&name, &[], segments)
}
#[pyfunction]
fn mirrored(points: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    points.into_iter().rev().map(|(x, y)| (y, x)).collect()
}
#[pyfunction]
fn steps_count(segments: isize) -> usize {
    (segments.max(1) as usize).div_ceil(2)
}
pub(crate) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    let py = m.py();
    let name = "ingetrazo_core.bevel_profiles";
    let sub = PyModule::new(py, name)?;
    sub.add("PRESETS", ("steps", "supports", "cornice", "crown"))?;
    sub.add("ASYMMETRIC", ("cornice", "crown"))?;
    for f in [
        wrap_pyfunction!(sample, &sub)?,
        wrap_pyfunction!(controls, &sub)?,
        wrap_pyfunction!(valid, &sub)?,
        wrap_pyfunction!(mirrored, &sub)?,
        wrap_pyfunction!(steps_count, &sub)?,
    ] {
        sub.add_function(f)?;
    }
    py.import("sys")?.getattr("modules")?.set_item(name, &sub)?;
    m.add("bevel_profiles", sub)
}
