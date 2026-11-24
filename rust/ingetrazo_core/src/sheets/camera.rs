// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! The camera a sheet frame looks through (docs/rust-sheets.md step 4):
//! core/camera.py's OrbitCamera (eye, up, view and projection) and
//! core/composition.py's `apply_frame_camera` (the view or scene it shows,
//! a true 90° plan, the frame's own orbit, its turn, exact scale or the
//! perspective's eye), ported line by line. Pinned by
//! tests/test_sheet_camera_parity.py against goldens from the Python.

use super::model::Fields;
use crate::doc::value::Value;
use glam::{DMat4, DQuat, DVec3};

const NEAR_FLOOR: f64 = 4e-4;
const FAR_PER_DISTANCE: f64 = 4.0;
const PARALLEL_REACH_MARGIN: f64 = 4.0;
const TWO_POINT_MIN_COS: f64 = 0.2;
const MAX_SHIFT: f64 = 1.0;
const RENDER_DPI: f64 = 300.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Cam {
    pub target: DVec3,
    pub distance: f64,
    pub yaw: f64,
    pub pitch: f64,
    pub up: DVec3,
    pub fov_deg: f64,
    pub aspect: f64,
    pub znear: f64,
    pub zfar: f64,
    pub perspective: bool,
    pub two_point: bool,
    pub shift_x: f64,
    pub shift_y: f64,
    pub roll_deg: f64,
    pub clip_near: Option<f64>,
    pub clip_far: Option<f64>,
    /// What the view holds as a sphere (centre, radius): the parallel depth range.
    pub depth_reach: Option<(DVec3, f64)>,
}

impl Default for Cam {
    fn default() -> Self {
        Cam {
            target: DVec3::ZERO,
            distance: 20.0,
            yaw: (-45f64).to_radians(),
            pitch: 30f64.to_radians(),
            up: DVec3::Z,
            fov_deg: 45.0,
            aspect: 1.0,
            znear: 0.1,
            zfar: 10000.0,
            perspective: true,
            two_point: false,
            shift_x: 0.0,
            shift_y: 0.0,
            roll_deg: 0.0,
            clip_near: None,
            clip_far: None,
            depth_reach: None,
        }
    }
}

impl Cam {
    pub fn eye(&self) -> DVec3 {
        let (cp, sp, cy, sy) = (self.pitch.cos(), self.pitch.sin(), self.yaw.cos(), self.yaw.sin());
        self.target + DVec3::new(self.distance * cp * cy, self.distance * cp * sy, self.distance * sp)
    }
    pub fn forward(&self) -> DVec3 {
        let d = self.target - self.eye();
        if d.length_squared() > 1e-18 { d.normalize() } else { DVec3::Y }
    }
    fn level_forward(&self) -> Option<DVec3> {
        if !(self.two_point && self.perspective) {
            return None;
        }
        let f = self.forward();
        let h = DVec3::new(f.x, f.y, 0.0);
        if h.length() < TWO_POINT_MIN_COS {
            return None;
        }
        Some(h / h.length())
    }
    pub fn up_vector(&self) -> DVec3 {
        let f = self.forward();
        let mut up = if f.dot(self.up).abs() < 0.9999 {
            self.up
        } else {
            let sign = if self.pitch > 0.0 { -1.0 } else { 1.0 };
            DVec3::new(sign * self.yaw.cos(), sign * self.yaw.sin(), 0.0)
        };
        if self.roll_deg != 0.0 && self.level_forward().is_none() {
            up = DQuat::from_axis_angle(f.normalize_or_zero(), self.roll_deg.to_radians()) * up;
        }
        up
    }
    pub fn view_matrix(&self) -> DMat4 {
        if let Some(level) = self.level_forward() {
            let eye = self.eye();
            return DMat4::look_at_rh(eye, eye + level, DVec3::Z);
        }
        DMat4::look_at_rh(self.eye(), self.target, self.up_vector().normalize_or_zero())
    }
    fn near_plane(&self) -> f64 {
        if let Some(n) = self.clip_near {
            return n.max(NEAR_FLOOR);
        }
        self.znear.min((self.distance * 0.02).max(NEAR_FLOOR))
    }
    fn far_plane(&self) -> f64 {
        if let Some(f) = self.clip_far {
            return f.max(self.near_plane() * 1.01);
        }
        self.zfar.max(self.distance * FAR_PER_DISTANCE)
    }
    fn parallel_depth(&self) -> f64 {
        let far = self.far_plane();
        let Some((centre, radius)) = self.depth_reach else { return far };
        let held = (self.eye() - centre).length() + radius;
        if !held.is_finite() {
            return far;
        }
        far.min((self.distance * FAR_PER_DISTANCE).max(held * PARALLEL_REACH_MARGIN).max(1.0))
    }
    fn shift(&self, half_w: f64, half_h: f64) -> (f64, f64) {
        (self.shift_x * 2.0 * half_w, self.shift_y * 2.0 * half_h)
    }
    fn frustum(l: f64, r: f64, b: f64, t: f64, n: f64, f: f64) -> DMat4 {
        DMat4::from_cols_array(&[
            2. * n / (r - l), 0., 0., 0.,
            0., 2. * n / (t - b), 0., 0.,
            (r + l) / (r - l), (t + b) / (t - b), -(f + n) / (f - n), -1.,
            0., 0., -2. * f * n / (f - n), 0.,
        ])
    }
    pub fn projection_matrix(&self) -> DMat4 {
        if self.perspective {
            let near = self.near_plane();
            if let Some(level) = self.level_forward() {
                let half_h = near * (self.fov_deg.to_radians() / 2.0).tan();
                let half_w = half_h * self.aspect;
                let rel = self.target - self.eye();
                let depth = rel.dot(level);
                let shift = rel.z * near / depth;
                let (sx, sy) = self.shift(half_w, half_h);
                return Self::frustum(-half_w + sx, half_w + sx, -half_h + shift + sy, half_h + shift + sy, near,
                                     self.far_plane());
            }
            if self.shift_x != 0.0 || self.shift_y != 0.0 {
                let half_h = near * (self.fov_deg.to_radians() / 2.0).tan();
                let half_w = half_h * self.aspect;
                let (sx, sy) = self.shift(half_w, half_h);
                return Self::frustum(-half_w + sx, half_w + sx, -half_h + sy, half_h + sy, near, self.far_plane());
            }
            return DMat4::perspective_rh_gl(self.fov_deg.to_radians(), self.aspect, near, self.far_plane());
        }
        let half_h = self.distance * (self.fov_deg.to_radians() / 2.0).tan();
        let half_w = half_h * self.aspect;
        let depth = self.parallel_depth();
        let (sx, sy) = self.shift(half_w, half_h);
        let mut near = self.clip_near.unwrap_or(-depth);
        let mut far = self.clip_far.unwrap_or(depth);
        if far <= near {
            near = -depth;
            far = depth;
        }
        DMat4::orthographic_rh_gl(-half_w + sx, half_w + sx, -half_h + sy, half_h + sy, near, far)
    }
    /// OrbitCamera.set_view: a standard view's yaw and pitch.
    pub fn set_view(&mut self, name: &str) {
        let (yaw, pitch) = match name {
            "top" => ((-90f64).to_radians(), 90f64.to_radians()),
            "bottom" => ((-90f64).to_radians(), (-90f64).to_radians()),
            "front" => ((-90f64).to_radians(), 0.0),
            "back" => (90f64.to_radians(), 0.0),
            "right" => (0.0, 0.0),
            "left" => (180f64.to_radians(), 0.0),
            "iso" => ((-45f64).to_radians(), 30f64.to_radians()),
            _ => return,
        };
        self.yaw = yaw;
        self.pitch = pitch;
    }
    /// OrbitCamera.apply_lens: shift, roll and clips; what the lens does
    /// not name goes back to its default.
    pub fn apply_lens(&mut self, lens: Option<&Value>) {
        self.shift_x = 0.0;
        self.shift_y = 0.0;
        self.roll_deg = 0.0;
        self.clip_near = None;
        self.clip_far = None;
        let Some(Value::Dict(d)) = lens else { return };
        let get = |k: &str| d.iter().find(|(kk, _)| matches!(kk, Value::Str(s) if s == k)).and_then(|(_, v)| match v {
            Value::Float(f) => Some(*f),
            Value::Int(i) => Some(*i as f64),
            _ => None,
        }).filter(|v| v.is_finite());
        if let Some(v) = get("shift_x") {
            self.shift_x = v.clamp(-MAX_SHIFT, MAX_SHIFT);
        }
        if let Some(v) = get("shift_y") {
            self.shift_y = v.clamp(-MAX_SHIFT, MAX_SHIFT);
        }
        if let Some(v) = get("roll_deg") {
            // math.remainder: the quotient rounded half to even.
            self.roll_deg = v - 360.0 * (v / 360.0).round_ties_even();
        }
        if let Some(v) = get("clip_near").filter(|v| *v > 0.0) {
            self.clip_near = Some(v);
        }
        if let Some(v) = get("clip_far").filter(|v| *v > 0.0) {
            self.clip_far = Some(v);
        }
        if let (Some(n), Some(f)) = (self.clip_near, self.clip_far) {
            if f <= n {
                self.clip_far = None;
            }
        }
    }
    /// core.composition.roll_camera: the drawing turned `deg` clockwise on paper.
    fn roll(&mut self, deg: f64) {
        if deg == 0.0 {
            return;
        }
        let th = deg.to_radians();
        let (cp, sp, cy, sy) = (self.pitch.cos(), self.pitch.sin(), self.yaw.cos(), self.yaw.sin());
        let f = DVec3::new(-cp * cy, -cp * sy, -sp);
        let u0 = self.up_vector();
        let r = DVec3::new(f.y * u0.z - f.z * u0.y, f.z * u0.x - f.x * u0.z, f.x * u0.y - f.y * u0.x);
        let rn = (r.x * r.x + r.y * r.y + r.z * r.z).sqrt();
        if rn < 1e-9 {
            return;
        }
        let r = r / rn;
        let u = DVec3::new(r.y * f.z - r.z * f.y, r.z * f.x - r.x * f.z, r.x * f.y - r.y * f.x);
        let (c, s) = (th.cos(), th.sin());
        self.up = DVec3::new(u.x * c - r.x * s, u.y * c - r.y * s, u.z * c - r.z * s);
    }
}

/// A saved view's camera, as its record holds it.
#[derive(Clone, Debug)]
pub struct ViewCam {
    pub target: DVec3,
    pub distance: f64,
    pub yaw: f64,
    pub pitch: f64,
    pub fov_deg: f64,
    pub perspective: bool,
    pub two_point: bool,
    pub lens: Option<Value>,
}

impl ViewCam {
    pub fn from_fields(f: &Fields) -> ViewCam {
        let t: Vec<f64> = f.list("target").iter().filter_map(|v| match v {
            Value::Float(x) => Some(*x),
            Value::Int(i) => Some(*i as f64),
            _ => None,
        }).collect();
        ViewCam {
            target: if t.len() == 3 { DVec3::new(t[0], t[1], t[2]) } else { DVec3::ZERO },
            distance: f.num("distance", 20.0),
            yaw: f.num("yaw", (-45f64).to_radians()),
            pitch: f.num("pitch", 30f64.to_radians()),
            fov_deg: f.num("fov_deg", 45.0),
            perspective: f.flag("perspective", true),
            two_point: f.flag("two_point", false),
            lens: f.0.get("lens").cloned(),
        }
    }
}

/// `mm_to_px` at the render resolution.
fn mm_to_px(mm: f64) -> f64 {
    (mm / 25.4 * RENDER_DPI).round_ties_even().max(1.0)
}

/// `fit_distance_for_scene`.
fn fit_distance(bounds: Option<(DVec3, DVec3)>, fov_deg: f64) -> f64 {
    let Some((a, b)) = bounds else { return 30.0 };
    let diag = ((b.x - a.x).powi(2) + (b.y - a.y).powi(2) + (b.z - a.z).powi(2)).sqrt();
    if diag <= 0.0 {
        return 30.0;
    }
    let t = (fov_deg.to_radians() / 2.0).tan();
    (diag / 2.0) / t.max(1e-6) * 1.15
}

fn vec3(v: &Value) -> Option<DVec3> {
    match v {
        Value::List(l) | Value::Tuple(l) if l.len() == 3 => {
            let n = |x: &Value| match x {
                Value::Float(f) => Some(*f),
                Value::Int(i) => Some(*i as f64),
                _ => None,
            };
            Some(DVec3::new(n(&l[0])?, n(&l[1])?, n(&l[2])?))
        }
        Value::Vector(v) => Some(DVec3::from(*v)),
        _ => None,
    }
}

/// core.composition.apply_frame_camera on `cam`: the frame's view (`view`
/// when it shows a scene), at exact scale or as its perspective says.
/// `bounds`: what the model holds, for a standard view's centre and a
/// perspective frame's fit.
pub fn frame_camera(mut cam: Cam, frame: &Fields, view: Option<&ViewCam>, bounds: Option<(DVec3, DVec3)>) -> Cam {
    let key = frame.text("view_key", "__current__");
    if let Some(v) = view {
        cam.target = v.target;
        cam.distance = v.distance;
        cam.yaw = v.yaw;
        cam.pitch = v.pitch;
        cam.fov_deg = v.fov_deg;
        cam.perspective = v.perspective;
        cam.two_point = v.two_point;
        cam.apply_lens(v.lens.as_ref());
    } else if let Some(std) = key.strip_prefix("std:") {
        cam.set_view(std);
        if std == "top" || std == "bottom" {
            cam.pitch = if std == "top" { 90f64.to_radians() } else { (-90f64).to_radians() };
            cam.up = DVec3::Y;
        }
        if let Some((lo, hi)) = bounds {
            cam.target = (lo + hi) * 0.5;
        }
    }
    if let Some(y) = frame.0.get("cam_yaw").and_then(|v| num(v)) {
        cam.yaw = y;
    }
    if let Some(p) = frame.0.get("cam_pitch").and_then(|v| num(v)) {
        cam.pitch = p;
        if p.abs() < 89.5f64.to_radians() {
            cam.up = DVec3::Z;
        }
    }
    if let Some(t) = frame.0.get("cam_target").and_then(vec3) {
        cam.target = t;
    }
    cam.roll(frame.num("rot_deg", 0.0));
    let (w, h) = (mm_to_px(frame.num("w_mm", 170.0)), mm_to_px(frame.num("h_mm", 170.0)));
    cam.aspect = w / h;
    if frame.flag("perspective", false) {
        let fov = frame.0.get("cam_fov").and_then(|v| num(v)).or(view.map(|v| v.fov_deg));
        if let Some(fov) = fov {
            cam.fov_deg = 5f64.max(120f64.min(fov));
        }
        let dist = frame.0.get("cam_distance").and_then(|v| num(v)).or(view.map(|v| v.distance))
            .unwrap_or_else(|| fit_distance(bounds, cam.fov_deg));
        cam.perspective = true;
        cam.distance = 1e-3f64.max(dist);
        return cam;
    }
    cam.perspective = false;
    let model_h = super::model::model_height_for_frame(frame.num("h_mm", 170.0), frame.num("scale_n", 100.0));
    cam.distance = (model_h / 2.0) / (cam.fov_deg.to_radians() / 2.0).tan();
    cam
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Float(f) => Some(*f),
        Value::Int(i) => Some(*i as f64),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plan_at_one_to_a_hundred() {
        let mut f = Fields::default();
        f.0.insert("view_key".into(), Value::Str("std:top".into()));
        f.0.insert("h_mm".into(), Value::Float(200.0));
        f.0.insert("w_mm".into(), Value::Float(200.0));
        f.0.insert("scale_n".into(), Value::Float(100.0));
        let c = frame_camera(Cam::default(), &f, None, Some((DVec3::ZERO, DVec3::new(10.0, 6.0, 3.0))));
        assert!(!c.perspective);
        assert_eq!(c.target, DVec3::new(5.0, 3.0, 1.5));
        // 200 mm at 1:100 shows 20 m: the ortho half-height is 10 m.
        let half_h = c.distance * (c.fov_deg.to_radians() / 2.0).tan();
        assert!((half_h - 10.0).abs() < 1e-9);
        assert_eq!(c.up, DVec3::Y);
    }
}

// ---- Python ------------------------------------------------------------------

use pyo3::prelude::*;
use pyo3::types::PyDict;

fn v3(d: &Bound<'_, PyDict>, k: &str) -> PyResult<Option<DVec3>> {
    match d.get_item(k)? {
        Some(v) if !v.is_none() => {
            let t: (f64, f64, f64) = match v.extract() {
                Ok(t) => t,
                Err(_) => (v.call_method0("x")?.extract()?, v.call_method0("y")?.extract()?, v.call_method0("z")?.extract()?),
            };
            Ok(Some(DVec3::new(t.0, t.1, t.2)))
        }
        _ => Ok(None),
    }
}

/// The camera a base camera's state starts from (``base``: a dict of
/// OrbitCamera fields — target, distance, yaw, pitch, up, fov_deg,
/// perspective, two_point, shift_x, shift_y, roll_deg, clip_near,
/// clip_far, depth_reach as ``(centre, radius)``; None: a new camera).
/// `base_cam` for the other sheet modules.
pub fn base_cam_of(base: Option<&Bound<'_, PyDict>>) -> PyResult<Cam> {
    base_cam(base)
}

fn base_cam(base: Option<&Bound<'_, PyDict>>) -> PyResult<Cam> {
    let mut c = Cam::default();
    let Some(d) = base else { return Ok(c) };
    if let Some(t) = v3(d, "target")? { c.target = t; }
    if let Some(u) = v3(d, "up")? { c.up = u; }
    let f = |k: &str| -> PyResult<Option<f64>> { d.get_item(k)?.filter(|v| !v.is_none()).map(|v| v.extract()).transpose() };
    if let Some(v) = f("distance")? { c.distance = v; }
    if let Some(v) = f("yaw")? { c.yaw = v; }
    if let Some(v) = f("pitch")? { c.pitch = v; }
    if let Some(v) = f("fov_deg")? { c.fov_deg = v; }
    if let Some(v) = f("shift_x")? { c.shift_x = v; }
    if let Some(v) = f("shift_y")? { c.shift_y = v; }
    if let Some(v) = f("roll_deg")? { c.roll_deg = v; }
    c.clip_near = f("clip_near")?;
    c.clip_far = f("clip_far")?;
    if let Some(v) = d.get_item("perspective")? { c.perspective = v.is_truthy()?; }
    if let Some(v) = d.get_item("two_point")? { c.two_point = v.is_truthy()?; }
    if let Some(r) = d.get_item("depth_reach")?.filter(|v| !v.is_none()) {
        let (centre, radius): ((f64, f64, f64), f64) = r.extract()?;
        c.depth_reach = Some((DVec3::new(centre.0, centre.1, centre.2), radius));
    }
    Ok(c)
}

/// The camera sheet frame ``frame`` looks through: ``view`` its saved view
/// (a SavedView record) or None, ``base`` the camera it starts from (see
/// ``base_cam``), ``bounds`` the model's ``(lo, hi)``. Returns view and
/// projection (16 values each, column-major as ``QMatrix4x4.data()``),
/// eye, target, up and the camera's fields (lens included).
#[pyfunction]
#[pyo3(signature = (frame, view=None, base=None, bounds=None))]
pub fn sheet_frame_camera<'py>(py: Python<'py>, frame: &Bound<'py, PyAny>, view: Option<Bound<'py, PyAny>>,
                               base: Option<Bound<'py, PyDict>>, bounds: Option<((f64, f64, f64), (f64, f64, f64))>)
    -> PyResult<Bound<'py, PyDict>> {
    py.import("core.doc_model")?.call_method0("flush")?;
    let f = super::model::read_fields(crate::doc::py::handle_key(frame)?)?;
    let v = match view.filter(|v| !v.is_none()) {
        Some(v) => Some(ViewCam::from_fields(&super::model::read_fields(crate::doc::py::handle_key(&v)?)?)),
        None => None,
    };
    let bounds = bounds.map(|(a, b)| (DVec3::new(a.0, a.1, a.2), DVec3::new(b.0, b.1, b.2)));
    let c = frame_camera(base_cam(base.as_ref())?, &f, v.as_ref(), bounds);
    let out = PyDict::new(py);
    out.set_item("view", c.view_matrix().to_cols_array().to_vec())?;
    out.set_item("projection", c.projection_matrix().to_cols_array().to_vec())?;
    out.set_item("eye", c.eye().to_array())?;
    out.set_item("target", c.target.to_array())?;
    out.set_item("up", c.up.to_array())?;
    out.set_item("distance", c.distance)?;
    out.set_item("fov_deg", c.fov_deg)?;
    out.set_item("perspective", c.perspective)?;
    out.set_item("aspect", c.aspect)?;
    out.set_item("yaw", c.yaw)?;
    out.set_item("pitch", c.pitch)?;
    out.set_item("two_point", c.two_point)?;
    out.set_item("shift_x", c.shift_x)?;
    out.set_item("shift_y", c.shift_y)?;
    out.set_item("roll_deg", c.roll_deg)?;
    out.set_item("clip_near", c.clip_near)?;
    out.set_item("clip_far", c.clip_far)?;
    Ok(out)
}
