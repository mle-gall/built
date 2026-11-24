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
use pyo3::types::PyDict;

use crate::math::{xyz, Matrix4, Vec3};
use crate::sheets::camera::Cam;

const MIN_DISTANCE: f64 = 0.005;
const MIN_DISTANCE_FAR: f64 = 0.02;
const MIN_PER_METRE: f64 = MIN_DISTANCE / 40.0;

/// The camera's fields as `Cam`: from its `__dict__` when it has one
/// (plain dictionary lookups), else by attribute.
pub fn cam_of(obj: &Bound<'_, PyAny>, fitted: Option<(f64, f64)>) -> PyResult<Cam> {
    let dict = obj.getattr("__dict__").ok().and_then(|d| d.cast_into::<PyDict>().ok());
    let get = |name: &str| -> PyResult<Bound<'_, PyAny>> {
        if let Some(d) = &dict {
            if let Some(v) = d.get_item(name)? {
                return Ok(v);
            }
        }
        obj.getattr(name)
    };
    let opt = |name: &str| -> PyResult<Option<f64>> {
        let v = get(name)?;
        if v.is_none() { Ok(None) } else { Ok(Some(v.extract()?)) }
    };
    let reach = get("depth_reach")?;
    let depth_reach = if reach.is_none() {
        None
    } else {
        Some((xyz(&reach.get_item(0)?)?, reach.get_item(1)?.extract::<f64>()?))
    };
    Ok(Cam {
        target: xyz(&get("target")?)?,
        distance: get("distance")?.extract()?,
        yaw: get("yaw")?.extract()?,
        pitch: get("pitch")?.extract()?,
        up: xyz(&get("up")?)?,
        fov_deg: get("fov_deg")?.extract()?,
        aspect: get("aspect")?.extract()?,
        znear: get("znear")?.extract()?,
        zfar: get("zfar")?.extract()?,
        perspective: get("perspective")?.is_truthy()?,
        two_point: get("two_point")?.is_truthy()?,
        shift_x: get("shift_x")?.extract()?,
        shift_y: get("shift_y")?.extract()?,
        roll_deg: get("roll_deg")?.extract()?,
        clip_near: opt("clip_near")?,
        clip_far: opt("clip_far")?,
        depth_reach,
        fitted,
    })
}

/// The camera's fields as last read, kept in its `__dict__` as `_rs_cam`.
/// `OrbitCamera.__setattr__` drops it on every write; the two vectors are
/// checked again on use, since a `Vec3` can be changed in place.
#[pyclass(module = "ingetrazo_core", name = "CameraState", frozen)]
pub struct CameraState {
    cam: Cam,
}

#[pymethods]
impl CameraState {
    /// Immutable: a copy is itself.
    fn __copy__(slf: Py<Self>) -> Py<Self> {
        slf
    }
    fn __deepcopy__(slf: Py<Self>, _memo: &Bound<'_, PyAny>) -> Py<Self> {
        slf
    }
    /// Pickled as None: the camera rebuilds it on first use.
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<(Bound<'py, PyAny>, ())> {
        Ok((py.None().into_bound(py).get_type().into_any(), ()))
    }
}

const CACHE: &str = "_rs_cam";

/// The camera as `Cam`, from its cached state while that still holds.
fn cam_cached(obj: &Bound<'_, PyAny>, fitted: Option<(f64, f64)>) -> PyResult<Cam> {
    let dict = obj.getattr("__dict__")?.cast_into::<PyDict>()?;
    if let Some(state) = dict.get_item(CACHE)? {
        if let Ok(state) = state.cast::<CameraState>() {
            let cam = &state.get().cam;
            let target = dict.get_item("target")?;
            let up = dict.get_item("up")?;
            if let (Some(t), Some(u)) = (target, up) {
                if xyz(&t)? == cam.target && xyz(&u)? == cam.up {
                    let mut c = cam.clone();
                    c.fitted = fitted;
                    return Ok(c);
                }
            }
        }
    }
    let cam = cam_of(obj, None)?;
    dict.set_item(CACHE, Py::new(obj.py(), CameraState { cam: cam.clone() })?)?;
    let mut c = cam;
    c.fitted = fitted;
    Ok(c)
}

fn vec(py: Python<'_>, v: DVec3) -> PyResult<Py<Vec3>> {
    Py::new(py, Vec3::from(v))
}

#[pyfunction]
pub fn camera_eye(py: Python<'_>, camera: &Bound<'_, PyAny>) -> PyResult<Py<Vec3>> {
    vec(py, cam_cached(camera, None)?.eye())
}

#[pyfunction]
pub fn camera_forward(py: Python<'_>, camera: &Bound<'_, PyAny>) -> PyResult<Py<Vec3>> {
    vec(py, cam_cached(camera, None)?.forward())
}

#[pyfunction]
pub fn camera_up_vector(py: Python<'_>, camera: &Bound<'_, PyAny>) -> PyResult<Py<Vec3>> {
    vec(py, cam_cached(camera, None)?.up_vector())
}

#[pyfunction]
pub fn camera_view_matrix(py: Python<'_>, camera: &Bound<'_, PyAny>) -> PyResult<Py<Matrix4>> {
    Py::new(py, Matrix4::from(cam_cached(camera, None)?.view_matrix()))
}

#[pyfunction]
#[pyo3(signature = (camera, fitted=None))]
pub fn camera_projection_matrix(py: Python<'_>, camera: &Bound<'_, PyAny>, fitted: Option<(f64, f64)>)
    -> PyResult<Py<Matrix4>> {
    Py::new(py, Matrix4::from(cam_cached(camera, fitted)?.projection_matrix()))
}

/// `(near, far, parallel depth)`.
#[pyfunction]
#[pyo3(signature = (camera, fitted=None))]
pub fn camera_depths(camera: &Bound<'_, PyAny>, fitted: Option<(f64, f64)>) -> PyResult<(f64, f64, f64)> {
    let c = cam_cached(camera, fitted)?;
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
    m.add_class::<CameraState>()?;
    m.add_function(wrap_pyfunction!(camera_eye, m)?)?;
    m.add_function(wrap_pyfunction!(camera_forward, m)?)?;
    m.add_function(wrap_pyfunction!(camera_up_vector, m)?)?;
    m.add_function(wrap_pyfunction!(camera_view_matrix, m)?)?;
    m.add_function(wrap_pyfunction!(camera_projection_matrix, m)?)?;
    m.add_function(wrap_pyfunction!(camera_depths, m)?)?;
    m.add_function(wrap_pyfunction!(camera_min_distance, m)?)?;
    Ok(())
}
