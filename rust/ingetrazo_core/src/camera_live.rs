//! The live camera's pose and projection (`core.camera.OrbitCamera`), on the
//! sheets' `Cam` (`sheets/camera.rs`), so the viewport and the sheets use one
//! implementation (`docs/model-port.md`, camera).
//!
//! Each function reads the camera's fields from the Python object and
//! answers what `OrbitCamera` computed: eye, sight line, up, view and
//! projection matrices, near / far / parallel depth and the closest
//! approach. A depth range fitted to the view (`fit_depth_range`) is passed
//! in by the caller, who knows whether it still holds.
//! `tests/test_camera_parity.py` pins them bit for bit.

use glam::DVec3;
use pyo3::prelude::*;

use crate::math::{xyz, Matrix4, Vec3};
use crate::sheets::camera::Cam;

const MIN_DISTANCE: f64 = 0.005;
const MIN_DISTANCE_FAR: f64 = 0.02;
const MIN_PER_METRE: f64 = MIN_DISTANCE / 40.0;

fn opt_f64(obj: &Bound<'_, PyAny>, name: &str) -> PyResult<Option<f64>> {
    let v = obj.getattr(name)?;
    if v.is_none() { Ok(None) } else { Ok(Some(v.extract()?)) }
}

/// The camera's fields as `Cam`.
pub fn cam_of(obj: &Bound<'_, PyAny>, fitted: Option<(f64, f64)>) -> PyResult<Cam> {
    let reach = obj.getattr("depth_reach")?;
    let depth_reach = if reach.is_none() {
        None
    } else {
        Some((xyz(&reach.get_item(0)?)?, reach.get_item(1)?.extract::<f64>()?))
    };
    Ok(Cam {
        target: xyz(&obj.getattr("target")?)?,
        distance: obj.getattr("distance")?.extract()?,
        yaw: obj.getattr("yaw")?.extract()?,
        pitch: obj.getattr("pitch")?.extract()?,
        up: xyz(&obj.getattr("up")?)?,
        fov_deg: obj.getattr("fov_deg")?.extract()?,
        aspect: obj.getattr("aspect")?.extract()?,
        znear: obj.getattr("znear")?.extract()?,
        zfar: obj.getattr("zfar")?.extract()?,
        perspective: obj.getattr("perspective")?.is_truthy()?,
        two_point: obj.getattr("two_point")?.is_truthy()?,
        shift_x: obj.getattr("shift_x")?.extract()?,
        shift_y: obj.getattr("shift_y")?.extract()?,
        roll_deg: obj.getattr("roll_deg")?.extract()?,
        clip_near: opt_f64(obj, "clip_near")?,
        clip_far: opt_f64(obj, "clip_far")?,
        depth_reach,
        fitted,
    })
}

fn vec(py: Python<'_>, v: DVec3) -> PyResult<Py<Vec3>> {
    Py::new(py, Vec3::from(v))
}

#[pyfunction]
pub fn camera_eye(py: Python<'_>, camera: &Bound<'_, PyAny>) -> PyResult<Py<Vec3>> {
    vec(py, cam_of(camera, None)?.eye())
}

#[pyfunction]
pub fn camera_forward(py: Python<'_>, camera: &Bound<'_, PyAny>) -> PyResult<Py<Vec3>> {
    vec(py, cam_of(camera, None)?.forward())
}

#[pyfunction]
pub fn camera_up_vector(py: Python<'_>, camera: &Bound<'_, PyAny>) -> PyResult<Py<Vec3>> {
    vec(py, cam_of(camera, None)?.up_vector())
}

#[pyfunction]
pub fn camera_view_matrix(py: Python<'_>, camera: &Bound<'_, PyAny>) -> PyResult<Py<Matrix4>> {
    Py::new(py, Matrix4::from(cam_of(camera, None)?.view_matrix()))
}

#[pyfunction]
#[pyo3(signature = (camera, fitted=None))]
pub fn camera_projection_matrix(py: Python<'_>, camera: &Bound<'_, PyAny>, fitted: Option<(f64, f64)>)
    -> PyResult<Py<Matrix4>> {
    Py::new(py, Matrix4::from(cam_of(camera, fitted)?.projection_matrix()))
}

/// `(near, far, parallel depth)`.
#[pyfunction]
#[pyo3(signature = (camera, fitted=None))]
pub fn camera_depths(camera: &Bound<'_, PyAny>, fitted: Option<(f64, f64)>) -> PyResult<(f64, f64, f64)> {
    let c = cam_of(camera, fitted)?;
    Ok((c.near_plane(), c.far_plane(), c.parallel_depth()))
}

/// How close the camera may come to `at`: `MIN_DISTANCE` near the origin,
/// rising with the distance from it up to `MIN_DISTANCE_FAR`.
#[pyfunction]
pub fn camera_min_distance(at: &Bound<'_, PyAny>) -> PyResult<f64> {
    let p = xyz(at)?;
    let far = (p.x * p.x + p.y * p.y + p.z * p.z).sqrt();
    if !far.is_finite() {
        return Ok(MIN_DISTANCE_FAR);
    }
    Ok(MIN_DISTANCE_FAR.min(MIN_DISTANCE.max(far * MIN_PER_METRE)))
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(camera_eye, m)?)?;
    m.add_function(wrap_pyfunction!(camera_forward, m)?)?;
    m.add_function(wrap_pyfunction!(camera_up_vector, m)?)?;
    m.add_function(wrap_pyfunction!(camera_view_matrix, m)?)?;
    m.add_function(wrap_pyfunction!(camera_projection_matrix, m)?)?;
    m.add_function(wrap_pyfunction!(camera_depths, m)?)?;
    m.add_function(wrap_pyfunction!(camera_min_distance, m)?)?;
    Ok(())
}
