// SPDX-License-Identifier: GPL-3.0-or-later
//! Read portable SpaceMouse profiles into the driver's settings and write them back.
use super::{ACTIONS, AXES};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyBool, PyDict, PyFloat, PyInt},
};
use std::path::PathBuf;

const FORMAT: &str = "built.ndof.profile";
const FIELDS: [(&str, &str); 3] = [
    ("mapping", "axis_mapping"),
    ("inversion", "axis_inversion"),
    ("sensitivity", "axis_sensitivity"),
];

#[pyfunction]
pub(crate) fn profile_to_dict<'p>(settings: &Bound<'p, PyAny>) -> PyResult<Bound<'p, PyDict>> {
    let py = settings.py();
    let profile = PyDict::new(py);
    profile.set_item("format", FORMAT)?;
    profile.set_item("version", 1)?;
    for (field, attr) in FIELDS {
        let values = settings.getattr(attr)?;
        let axes = PyDict::new(py);
        for axis in AXES {
            axes.set_item(axis, values.get_item(axis)?)?;
        }
        profile.set_item(field, axes)?;
    }
    profile.set_item("global_speed", settings.getattr("sensitivity")?)?;
    profile.set_item("deadzone", settings.getattr("deadzone")?)?;
    Ok(profile)
}

fn number(value: Option<Bound<'_, PyAny>>, label: &str, low: f64, high: f64) -> PyResult<f64> {
    let error = || PyValueError::new_err(format!("{label} must be a number"));
    let value = value.ok_or_else(error)?;
    if value.is_instance_of::<PyBool>()
        || !(value.is_instance_of::<PyFloat>() || value.is_instance_of::<PyInt>())
    {
        return Err(error());
    }
    let x = value.extract::<f64>()?;
    if !(low..=high).contains(&x) {
        return Err(PyValueError::new_err(format!(
            "{label} must be between {low} and {high}"
        )));
    }
    Ok(x)
}

#[pyfunction]
pub(crate) fn profile_from_dict<'p>(data: &Bound<'p, PyAny>) -> PyResult<Bound<'p, PyAny>> {
    let py = data.py();
    let unsupported = || PyValueError::new_err("Unsupported SpaceMouse profile format or version");
    let profile = data.cast::<PyDict>().map_err(|_| unsupported())?;
    if !profile
        .get_item("format")?
        .ok_or_else(unsupported)?
        .eq(FORMAT)?
        || !profile
            .get_item("version")?
            .ok_or_else(unsupported)?
            .eq(1)?
    {
        return Err(unsupported());
    }
    let kwargs = PyDict::new(py);
    kwargs.set_item("mapping_preset", "custom")?;
    let settings = py
        .import("built_render.ndof")?
        .getattr("NdofSettings")?
        .call((), Some(&kwargs))?;
    for (field, attr) in FIELDS {
        let invalid_axes = || {
            PyValueError::new_err(format!(
                "{field} must contain exactly the six physical axes"
            ))
        };
        let axes = profile
            .get_item(field)?
            .ok_or_else(invalid_axes)?
            .cast_into::<PyDict>()
            .map_err(|_| invalid_axes())?;
        if axes.len() != AXES.len() {
            return Err(invalid_axes());
        }
        let values = settings.getattr(attr)?;
        for axis in AXES {
            let value = axes.get_item(axis)?.ok_or_else(invalid_axes)?;
            match field {
                "mapping" => {
                    let action = value
                        .extract::<String>()
                        .ok()
                        .filter(|a| ACTIONS.contains(&a.as_str()))
                        .ok_or_else(|| {
                            PyValueError::new_err(format!("Invalid action for {axis}"))
                        })?;
                    values.set_item(axis, action)?;
                }
                "inversion" => {
                    if !value.is_instance_of::<PyBool>() {
                        return Err(PyValueError::new_err(format!(
                            "Invalid inversion for {axis}"
                        )));
                    }
                    values.set_item(axis, value)?;
                }
                _ => values.set_item(
                    axis,
                    number(Some(value), &format!("{axis} sensitivity"), 0.25, 4.0)?,
                )?,
            }
        }
    }
    settings.setattr(
        "sensitivity",
        number(profile.get_item("global_speed")?, "Global speed", 0.25, 4.0)?,
    )?;
    settings.setattr(
        "deadzone",
        number(profile.get_item("deadzone")?, "Deadzone", 0.0, 0.95)?,
    )?;
    Ok(settings)
}

#[pyfunction]
pub(crate) fn load_profile(py: Python<'_>, path: PathBuf) -> PyResult<Bound<'_, PyAny>> {
    let data = std::fs::read(path)?;
    let parsed = py
        .import("json")?
        .call_method1("loads", (pyo3::types::PyBytes::new(py, &data),))?;
    profile_from_dict(&parsed)
}

#[pyfunction]
fn save_profile(path: PathBuf, settings: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = settings.py();
    let kwargs = PyDict::new(py);
    kwargs.set_item("indent", 2)?;
    let mut json: String = py
        .import("json")?
        .getattr("dumps")?
        .call((profile_to_dict(settings)?,), Some(&kwargs))?
        .extract()?;
    json.push('\n');
    std::fs::write(path, json)?;
    Ok(())
}
pub(crate) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(profile_from_dict, m)?)?;
    m.add_function(wrap_pyfunction!(profile_to_dict, m)?)?;
    m.add_function(wrap_pyfunction!(load_profile, m)?)?;
    m.add_function(wrap_pyfunction!(save_profile, m)?)
}
