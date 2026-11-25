// SPDX-License-Identifier: GPL-3.0-or-later
//! A saved view from an eye/target/up camera (`core.saved_views.from_lookat`,
//! the importers' entry): the orbit camera's distance, yaw and pitch. The
//! view's archive is the document's own (`doc/file_view.rs`).
//!
//! Python's `math` calls are the C library's, as Rust's are
//! (`tests/test_saved_view_parity.py`).

use pyo3::prelude::*;

/// `(distance, yaw, pitch)` of the orbit camera looking from `eye` at
/// `target`. Straight up or down, the stored `up` gives the yaw (at the
/// pole the orbit camera's screen-up is −(cos yaw, sin yaw)): `down` is
/// that `−up`, negated by the caller (Python's `-0` of an int is `+0`). A
/// parallel view with `ortho_height` frames that visible height.
pub fn lookat(eye: [f64; 3], target: [f64; 3], down: [f64; 2], fov_deg: f64, perspective: bool,
              ortho_height: Option<f64>) -> (f64, f64, f64) {
    let rad = |d: f64| d * (std::f64::consts::PI / 180.0);
    let dx = eye[0] - target[0];
    let dy = eye[1] - target[1];
    let dz = eye[2] - target[2];
    let mut distance = (dx * dx + dy * dy + dz * dz).sqrt();
    if distance == 0.0 {
        distance = 1.0;
    }
    let mut pitch = (dz / distance).clamp(-1.0, 1.0).asin();
    let limit = rad(89.0);
    let yaw = if pitch.abs() < limit - 1e-6 {
        dy.atan2(dx)
    } else if down[0] != 0.0 || down[1] != 0.0 {
        down[1].atan2(down[0])
    } else {
        rad(-90.0)
    };
    pitch = pitch.min(limit).max(-limit);
    if let Some(h) = ortho_height.filter(|&h| !perspective && h > 0.0) {
        distance = (h / 2.0) / (rad(fov_deg) / 2.0).tan();
    }
    (distance, yaw, pitch)
}

#[pyfunction]
#[pyo3(signature = (eye, target, down, fov_deg=45.0, perspective=true, ortho_height=None))]
fn saved_view_lookat(eye: [f64; 3], target: [f64; 3], down: [f64; 2], fov_deg: f64, perspective: bool,
                     ortho_height: Option<f64>) -> (f64, f64, f64) {
    lookat(eye, target, down, fov_deg, perspective, ortho_height)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(saved_view_lookat, m)?)?;
    Ok(())
}
