// SPDX-License-Identifier: GPL-3.0-or-later
//! Binary64 navigation in the model's Z-up frame. Screen deltas are logical pixels.
use crate::math::{xyz, Vec3};
use glam::{DQuat, DVec3};
use pyo3::prelude::*;

fn eye_offset(yaw: f64, pitch: f64, distance: f64) -> DVec3 {
    let horizontal = distance * pitch.cos();
    DVec3::new(
        horizontal * yaw.cos(),
        horizontal * yaw.sin(),
        distance * pitch.sin(),
    )
}

#[pyfunction]
fn orbit_angles(yaw: f64, pitch: f64, dx: f64, dy: f64, height: f64) -> (f64, f64) {
    let scale = std::f64::consts::PI / height.max(1.0);
    let limit = 89_f64.to_radians();
    (yaw - dx * scale, (pitch + dy * scale).clamp(-limit, limit))
}

#[pyfunction]
fn orbit_target(
    target: &Vec3,
    pivot: &Bound<'_, PyAny>,
    old_yaw: f64,
    old_pitch: f64,
    yaw: f64,
    pitch: f64,
) -> PyResult<Vec3> {
    let pivot = xyz(pivot)?;
    let target = DVec3::from_array(target.coords());
    if !pivot.is_finite() || ((yaw - old_yaw).abs() < 1e-12 && (pitch - old_pitch).abs() < 1e-12) {
        return Ok(target.into());
    }
    let yaw_turn = DQuat::from_rotation_z(yaw - old_yaw);
    let axis = eye_offset(yaw, old_pitch, 1.0).cross(DVec3::Z);
    let turn = if axis.length() < 1e-9 {
        yaw_turn
    } else {
        DQuat::from_axis_angle(axis.normalize(), pitch - old_pitch) * yaw_turn
    };
    Ok((pivot + turn * (target - pivot)).into())
}

#[pyfunction]
fn pan_target(
    target: &Vec3,
    up: &Vec3,
    yaw: f64,
    pitch: f64,
    span: f64,
    fov_deg: f64,
    dx: f64,
    dy: f64,
    height: f64,
) -> Vec3 {
    let forward = -eye_offset(yaw, pitch, 1.0);
    let right = forward
        .cross(DVec3::from_array(up.coords()))
        .normalize_or_zero();
    let screen_up = right.cross(forward).normalize_or_zero();
    let scale = 2.0 * span * (fov_deg.to_radians() * 0.5).tan() / height.max(1.0);
    (DVec3::from_array(target.coords()) - right * (dx * scale) + screen_up * (dy * scale)).into()
}

#[pyfunction]
fn zoom_distance(distance: f64, steps: f64, minimum: f64, maximum: f64) -> f64 {
    (distance * 0.9_f64.powf(steps)).min(maximum).max(minimum)
}

#[pyfunction]
fn zoom_focus(
    target: &Vec3,
    focus: &Bound<'_, PyAny>,
    eye: &Vec3,
    yaw: f64,
    pitch: f64,
    distance: f64,
    steps: f64,
    minimum: f64,
    maximum: f64,
    retreat: f64,
) -> PyResult<(Vec3, f64)> {
    let target = DVec3::from_array(target.coords());
    let focus = xyz(focus)?;
    let eye = DVec3::from_array(eye.coords());
    let mut factor = 0.9_f64.powf(steps);
    let mut next_distance = zoom_distance(distance, steps, minimum, maximum);
    if steps < 0.0 && distance > 0.0 {
        factor = factor.min(next_distance / distance);
    }
    let next_target = focus + (target - focus) * factor;
    if steps < 0.0 && retreat > 0.0 && next_distance < maximum {
        let moved = (next_target + eye_offset(yaw, pitch, next_distance)).distance(eye);
        if moved < retreat {
            next_distance = (next_distance + (retreat - moved)).min(maximum);
        }
    }
    Ok((next_target.into(), next_distance))
}

pub fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(orbit_angles, module)?)?;
    module.add_function(wrap_pyfunction!(orbit_target, module)?)?;
    module.add_function(wrap_pyfunction!(pan_target, module)?)?;
    module.add_function(wrap_pyfunction!(zoom_distance, module)?)?;
    module.add_function(wrap_pyfunction!(zoom_focus, module)?)?;
    Ok(())
}
