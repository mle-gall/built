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

use glam::{DVec3, DVec4};
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

// ---- Fitting and first-person moves ------------------------------------------

const MAX_DISTANCE: f64 = 10000.0;
const EMPTY_SIZE: f64 = 0.02;

/// Python's `min(a, b)` and `max(a, b)`: the first unless the second is
/// strictly past it.
fn pmin(a: f64, b: f64) -> f64 {
    if b < a { b } else { a }
}
fn pmax(a: f64, b: f64) -> f64 {
    if b > a { b } else { a }
}
fn pmax_all(v: impl IntoIterator<Item = f64>) -> f64 {
    let mut it = v.into_iter();
    let first = it.next().unwrap_or(f64::NAN);
    it.fold(first, pmax)
}
fn pmin_all(v: impl IntoIterator<Item = f64>) -> f64 {
    let mut it = v.into_iter();
    let first = it.next().unwrap_or(f64::NAN);
    it.fold(first, pmin)
}

fn min_distance_at(p: DVec3) -> f64 {
    let far = (p.x * p.x + p.y * p.y + p.z * p.z).sqrt();
    if !far.is_finite() {
        return MIN_DISTANCE_FAR;
    }
    pmin(MIN_DISTANCE_FAR, pmax(MIN_DISTANCE, far * MIN_PER_METRE))
}

fn set_pose(obj: &Bound<'_, PyAny>, target: Option<DVec3>, distance: Option<f64>, angles: Option<(f64, f64)>)
    -> PyResult<()> {
    // The order of OrbitCamera's own writes: angles, then target, then distance.
    if let Some((yaw, pitch)) = angles {
        obj.setattr("yaw", yaw)?;
        obj.setattr("pitch", pitch)?;
    }
    if let Some(t) = target {
        obj.setattr("target", Py::new(obj.py(), Vec3::from(t))?)?;
    }
    if let Some(d) = distance {
        obj.setattr("distance", d)?;
    }
    Ok(())
}

/// The fitted depth range while it holds for the camera's view.
fn fitted_now(obj: &Bound<'_, PyAny>) -> PyResult<Option<(f64, f64)>> {
    if obj.getattr("_depth_range")?.is_none() {
        return Ok(None);
    }
    let f = obj.call_method0("_fitted_depth_range")?;
    if f.is_none() { Ok(None) } else { Ok(Some(f.extract()?)) }
}

/// `look_from`'s angles for a sight line `direction` (the yaw kept when
/// looking straight up or down).
fn look_angles(cam: &Cam, direction: DVec3) -> (DVec3, f64, f64) {
    let mut d = direction;
    if d.length_squared() < 1e-18 {
        d = cam.forward();
    }
    let d = d.normalize_or_zero();
    let pitch = pmax(-1.0, pmin(1.0, -d.z)).asin();
    let pitch = pmax(pmin(pitch, 89f64.to_radians()), (-89f64).to_radians());
    let yaw = if crate::pyfloat::cos(pitch).abs() > 1e-9 { (-d.y).atan2(-d.x) } else { cam.yaw };
    (d, yaw, pitch)
}

/// `look_from(eye, direction)`: the eye at `eye` looking along
/// `direction`, a near-vertical look kept just off the pole.
#[pyfunction]
pub fn camera_look_from(camera: &Bound<'_, PyAny>, eye: &Bound<'_, PyAny>, direction: &Bound<'_, PyAny>) -> PyResult<()> {
    let cam = cam_cached(camera, None)?;
    let (d, yaw, pitch) = look_angles(&cam, xyz(direction)?);
    set_pose(camera, Some(xyz(eye)? + d * cam.distance), None, Some((yaw, pitch)))
}

/// `turn(d_yaw_deg, d_pitch_deg)`: the head turns, the eye stays.
#[pyfunction]
pub fn camera_turn(camera: &Bound<'_, PyAny>, d_yaw_deg: f64, d_pitch_deg: f64) -> PyResult<()> {
    let cam = cam_cached(camera, None)?;
    let eye = cam.eye();
    let d = cam.forward();
    let yaw = d.y.atan2(d.x) - d_yaw_deg.to_radians();
    let pitch = pmax(-1.0, pmin(1.0, d.z)).asin() + d_pitch_deg.to_radians();
    let pitch = pmax(pmin(pitch, 89f64.to_radians()), (-89f64).to_radians());
    let cp = crate::pyfloat::cos(pitch);
    let (d, yaw, pitch) = look_angles(&cam, DVec3::new(cp * crate::pyfloat::cos(yaw), cp * crate::pyfloat::sin(yaw), crate::pyfloat::sin(pitch)));
    set_pose(camera, Some(eye + d * cam.distance), None, Some((yaw, pitch)))
}

/// `fit_to(lo, hi, margin)`: centred on the box, backed off to frame its
/// bounding sphere.
#[pyfunction]
#[pyo3(signature = (camera, lo, hi, margin=1.3))]
pub fn camera_fit_to(camera: &Bound<'_, PyAny>, lo: &Bound<'_, PyAny>, hi: &Bound<'_, PyAny>, margin: f64) -> PyResult<()> {
    let cam = cam_cached(camera, None)?;
    let (a, b) = (xyz(lo)?, xyz(hi)?);
    let center = DVec3::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5, (a.z + b.z) * 0.5);
    let mut diag = (b - a).length();
    let floor = min_distance_at(center);
    if diag < 1e-9 {
        diag = EMPTY_SIZE;
    }
    let diag = pmax(diag, floor);
    let fov = cam.fov_deg.to_radians();
    let distance = pmax(diag * margin / (2.0 * (fov / 2.0).tan()), floor);
    set_pose(camera, Some(center), Some(distance), None)
}

/// The settling of a perspective fit (`_settle_perspective_fit`): centre
/// the box on screen, then walk in until its wider half-span is
/// `1 / margin` of the window.
fn settle(camera: &Bound<'_, PyAny>, corners: &[DVec3], margin: f64) -> PyResult<()> {
    let goal = 1.0 / margin;
    for _ in 0..8 {
        let cam = cam_cached(camera, fitted_now(camera)?)?;
        let t = (cam.fov_deg.to_radians() / 2.0).tan();
        let mvp = cam.projection_matrix() * cam.view_matrix();
        let (mut xs, mut ys) = (Vec::with_capacity(8), Vec::with_capacity(8));
        for c in corners {
            let p = mvp * DVec4::new(c.x, c.y, c.z, 1.0);
            if p.w <= 1e-9 {
                return Ok(());
            }
            xs.push(p.x / p.w);
            ys.push(p.y / p.w);
        }
        let (x0, x1) = (pmin_all(xs.iter().copied()), pmax_all(xs.iter().copied()));
        let (y0, y1) = (pmin_all(ys.iter().copied()), pmax_all(ys.iter().copied()));
        let cx = (x1 + x0) * 0.5;
        let cy = (y1 + y0) * 0.5;
        let reach = pmax((x1 - x0) * 0.5, (y1 - y0) * 0.5);
        if reach <= 0.0 {
            return Ok(());
        }
        if (reach - goal).abs() < 0.005 && cx.abs() < 0.005 && cy.abs() < 0.005 {
            return Ok(());
        }
        let f = cam.forward();
        let r = f.cross(cam.up_vector()).normalize_or_zero();
        let u = r.cross(f);
        let half_h = cam.distance * t;
        let target = cam.target + r * (cx * half_h * cam.aspect) + u * (cy * half_h);
        let nearest = pmax_all(corners.iter().map(|c| -(*c - target).dot(f)));
        let gap = cam.distance - nearest;
        set_pose(camera, Some(target), None, None)?;
        let distance = pmin(pmax(nearest + gap * reach / goal, min_distance_at(target)), MAX_DISTANCE);
        set_pose(camera, None, Some(distance), None)?;
    }
    Ok(())
}

/// `fit_box`'s framing in the camera's present orientation (the box near
/// the origin; `OrbitCamera.fit_box` handles survey coordinates and the
/// free part of the window). Returns the right, up and sight vectors it
/// framed with, or None when it fell back to `fit_to`.
#[pyfunction]
pub fn camera_fit_box(py: Python<'_>, camera: &Bound<'_, PyAny>, lo: &Bound<'_, PyAny>, hi: &Bound<'_, PyAny>, margin: f64)
    -> PyResult<Option<(Py<Vec3>, Py<Vec3>, Py<Vec3>)>> {
    let cam = cam_cached(camera, None)?;
    let (a, b) = (xyz(lo)?, xyz(hi)?);
    let f = cam.forward();
    let r = f.cross(cam.up_vector());
    if r.length_squared() < 1e-18 {
        camera_fit_to(camera, lo, hi, 1.3)?;
        return Ok(None);
    }
    let r = r.normalize_or_zero();
    let u = r.cross(f);
    let mut corners = Vec::with_capacity(8);
    for x in [a.x, b.x] {
        for y in [a.y, b.y] {
            for z in [a.z, b.z] {
                corners.push(DVec3::new(x, y, z));
            }
        }
    }
    let center = (a + b) * 0.5;
    let pr: Vec<f64> = corners.iter().map(|c| (*c - center).dot(r)).collect();
    let pu: Vec<f64> = corners.iter().map(|c| (*c - center).dot(u)).collect();
    let (pr0, pr1) = (pmin_all(pr.iter().copied()), pmax_all(pr.iter().copied()));
    let (pu0, pu1) = (pmin_all(pu.iter().copied()), pmax_all(pu.iter().copied()));
    let target = center + r * ((pr1 + pr0) * 0.5) + u * ((pu1 + pu0) * 0.5);
    let half_w = (pr1 - pr0) * 0.5;
    let half_h = (pu1 - pu0) * 0.5;
    let t = (cam.fov_deg.to_radians() / 2.0).tan();
    let aspect = if cam.aspect > 1e-6 { cam.aspect } else { 1.0 };
    let floor = min_distance_at(target);
    let half = pmax(pmax(half_h, half_w / aspect), floor) * margin;
    let mut distance = half / t;
    if cam.perspective {
        let toward = pmax_all(corners.iter().map(|c| -(*c - target).dot(f)));
        distance += pmax(toward, 0.0);
    }
    set_pose(camera, Some(target), None, None)?;
    set_pose(camera, None, Some(pmin(pmax(distance, floor), MAX_DISTANCE)), None)?;
    if cam.perspective {
        settle(camera, &corners, margin)?;
    }
    Ok(Some((vec(py, r)?, vec(py, u)?, vec(py, f)?)))
}

/// `snap_iso`: the isometric view nearest the present sight line.
#[pyfunction]
pub fn camera_snap_iso(camera: &Bound<'_, PyAny>) -> PyResult<()> {
    let cam = cam_cached(camera, None)?;
    let f = cam.forward();
    let yaws = [-45.0f64, 45.0, 135.0, 225.0];
    let (mut best_yaw, mut best_pitch, mut best_dot) = (yaws[0].to_radians(), 30f64.to_radians(), -2.0);
    for sign in [1.0, -1.0] {
        let pitch = sign * 30f64.to_radians();
        let (cp, sp) = (crate::pyfloat::cos(pitch), crate::pyfloat::sin(pitch));
        for yd in yaws {
            let yaw = yd.to_radians();
            let d = DVec3::new(-cp * crate::pyfloat::cos(yaw), -cp * crate::pyfloat::sin(yaw), -sp);
            let dot = f.dot(d);
            if dot > best_dot {
                best_dot = dot;
                best_yaw = yaw;
                best_pitch = pitch;
            }
        }
    }
    set_pose(camera, None, None, Some((best_yaw, best_pitch)))
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(camera_look_from, m)?)?;
    m.add_function(wrap_pyfunction!(camera_turn, m)?)?;
    m.add_function(wrap_pyfunction!(camera_fit_to, m)?)?;
    m.add_function(wrap_pyfunction!(camera_fit_box, m)?)?;
    m.add_function(wrap_pyfunction!(camera_snap_iso, m)?)?;
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
