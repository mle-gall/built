//! Plugin-facing camera controls. The existing live-camera kernels handle poses and matrices.
use super::camera_live as live;
use crate::math::{xyz, Matrix4, Vec3};
use crate::navigation as nav;
use glam::DVec3;
use numpy::PyReadonlyArray2;
use pyo3::{
    ffi,
    prelude::*,
    types::{PyDict, PyList, PyString, PyTuple},
};

const FULL_FRAME: (f64, f64) = (36., 24.);
const MAX_DISTANCE: f64 = 10000.;
const NEAR_FLOOR: f64 = 0.0004;
const LENS: [&str; 7] = [
    "shift_x",
    "shift_y",
    "roll_deg",
    "clip_near",
    "clip_far",
    "frame_aspect",
    "sensor",
];
type Obj<'py> = Bound<'py, PyAny>;
type Camera<'py> = Bound<'py, OrbitCamera>;
fn number(obj: &Obj<'_>, key: &str) -> PyResult<f64> {
    obj.getattr(key)?.extract()
}
fn point<'py>(py: Python<'py>, v: DVec3) -> PyResult<Bound<'py, Vec3>> {
    Bound::new(py, Vec3::from(v))
}
fn preset(name: &str) -> Option<(f64, f64)> {
    let (y, p): (f64, f64) = match name {
        "top" => (-90., 90.),
        "bottom" => (-90., -90.),
        "front" => (-90., 0.),
        "back" => (90., 0.),
        "right" => (0., 0.),
        "left" => (180., 0.),
        "iso" => (-45., 30.),
        _ => return None,
    };
    Some((y.to_radians(), p.to_radians()))
}

#[pyfunction]
fn sensor_picture_height(sensor: &Obj<'_>, aspect: Option<f64>) -> PyResult<f64> {
    let (w, h) = if sensor.is_truthy()? {
        let [w, h] = sensor.extract::<[f64; 2]>()?;
        (w, h)
    } else {
        FULL_FRAME
    };
    Ok(match aspect {
        Some(a) if a > 0. && a > w / h => w / a,
        _ => h,
    })
}
#[pyfunction]
#[pyo3(signature=(fov_deg,picture_height_mm=24.))]
fn focal_from_fov(fov_deg: f64, picture_height_mm: f64) -> f64 {
    // min(179, nan) in the Python original selects 179.
    let half = fov_deg.min(179.).max(0.001).to_radians() / 2.;
    (picture_height_mm / 2.) / half.tan()
}
#[pyfunction]
#[pyo3(signature=(focal_mm,picture_height_mm=24.))]
fn fov_from_focal(focal_mm: f64, picture_height_mm: f64) -> f64 {
    (2. * ((picture_height_mm / 2.) / focal_mm.max(0.001)).atan()).to_degrees()
}
#[pyfunction]
fn frame_rect(width: f64, height: f64, aspect: Option<f64>) -> Option<(f64, f64, f64, f64)> {
    let a = aspect?;
    if a <= 0. || width <= 0. || height <= 0. {
        return None;
    }
    let (w, h) = if width / height > a {
        (height * a, height)
    } else {
        (width, width / a)
    };
    Some(((width - w) / 2., (height - h) / 2., w, h))
}
fn read_sensor(v: &Obj<'_>) -> Option<(f64, f64)> {
    if !(v.is_instance_of::<PyList>() || v.is_instance_of::<PyTuple>()) || v.len().ok()? != 2 {
        return None;
    }
    let convert = v.py().get_type::<pyo3::types::PyFloat>();
    let parse = |i| {
        convert
            .call1((v.get_item(i).ok()?,))
            .ok()?
            .extract::<f64>()
            .ok()
    };
    let (w, h) = (parse(0)?, parse(1)?);
    ((1.0..=100.).contains(&w) && (1.0..=100.).contains(&h) && (w, h) != FULL_FRAME)
        .then_some((w, h))
}
#[pyclass(dict, weakref, subclass, module = "built_core.camera")]
pub struct OrbitCamera;

#[pymethods]
impl OrbitCamera {
    #[new]
    #[pyo3(signature=(*_args, **_kwargs))]
    fn new(_args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>) -> Self {
        Self
    }
    fn __init__(s: &Camera<'_>) -> PyResult<()> {
        let py = s.py();
        s.setattr("target", point(py, DVec3::ZERO)?)?;
        s.setattr("up", point(py, DVec3::Z)?)?;
        for (k, v) in [
            ("distance", 20.),
            ("yaw", (-45f64).to_radians()),
            ("pitch", 30f64.to_radians()),
            ("fov_deg", 45.),
            ("aspect", 1.),
            ("znear", 0.1),
            ("zfar", 10000.),
            ("shift_x", 0.),
            ("shift_y", 0.),
            ("roll_deg", 0.),
        ] {
            s.setattr(k, v)?;
        }
        for k in [
            "depth_reach",
            "_depth_range",
            "clip_near",
            "clip_far",
            "frame_aspect",
            "sensor",
        ] {
            s.setattr(k, py.None())?;
        }
        s.setattr("perspective", true)?;
        s.setattr("two_point", false)
    }
    fn __setattr__(s: &Camera<'_>, name: &Bound<'_, PyString>, value: &Obj<'_>) -> PyResult<()> {
        let py = s.py();
        let value = if matches!(name.to_str()?, "target" | "up") && !value.is_instance_of::<Vec3>()
        {
            point(py, xyz(value)?)?.into_any()
        } else {
            value.clone()
        };
        // GenericSetAttr preserves Python subclass descriptors as well as instance overrides.
        if unsafe { ffi::PyObject_GenericSetAttr(s.as_ptr(), name.as_ptr(), value.as_ptr()) } < 0 {
            return Err(PyErr::fetch(py));
        }
        let attrs = s.getattr("__dict__")?.cast_into::<PyDict>()?;
        if attrs.contains("_rs_cam")? {
            attrs.del_item("_rs_cam")?;
        }
        Ok(())
    }
    fn __delattr__(s: &Camera<'_>, name: &Bound<'_, PyString>) -> PyResult<()> {
        if unsafe { ffi::PyObject_GenericSetAttr(s.as_ptr(), name.as_ptr(), std::ptr::null_mut()) }
            < 0
        {
            return Err(PyErr::fetch(s.py()));
        }
        let attrs = s.getattr("__dict__")?.cast_into::<PyDict>()?;
        if attrs.contains("_rs_cam")? {
            attrs.del_item("_rs_cam")?;
        }
        Ok(())
    }
    fn __reduce_ex__<'py>(
        s: &Camera<'py>,
        _protocol: i32,
    ) -> PyResult<(Obj<'py>, (Bound<'py, pyo3::types::PyType>,), Obj<'py>)> {
        Ok((
            s.py().import("copyreg")?.getattr("__newobj__")?,
            (s.as_any().get_type(),),
            s.call_method0("__getstate__")?,
        ))
    }
    #[classattr]
    const PROJECTIONS: (&'static str, &'static str, &'static str) =
        ("parallel", "perspective", "two_point");
    #[classattr]
    #[allow(non_snake_case)]
    fn _STANDARD_VIEWS(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
        let d = PyDict::new(py);
        for name in ["top", "bottom", "front", "back", "right", "left", "iso"] {
            d.set_item(name, preset(name))?;
        }
        Ok(d)
    }
    #[classattr]
    const _ISO_YAWS: (f64, f64, f64, f64) = (-45., 45., 135., 225.);
    #[classattr]
    const _ISO_PITCH_DEG: f64 = 30.;
    fn lens_dict<'py>(s: &Camera<'py>) -> PyResult<Bound<'py, PyDict>> {
        let out = PyDict::new(s.py());
        for k in LENS {
            let v = s.getattr(k)?;
            if k == "sensor" {
                if !v.is_none() {
                    let arr = v.extract::<[f64; 2]>()?;
                    if (arr[0], arr[1]) != FULL_FRAME {
                        out.set_item(k, arr.to_vec())?;
                    }
                }
            } else {
                let is_default = match k {
                    "shift_x" | "shift_y" | "roll_deg" => v.eq(0.)?,
                    _ => v.is_none(),
                };
                if !is_default {
                    out.set_item(k, v)?;
                }
            }
        }
        Ok(out)
    }
    #[staticmethod]
    fn _read_sensor(v: &Obj<'_>) -> Option<(f64, f64)> {
        read_sensor(v)
    }
    fn apply_lens(s: &Camera<'_>, raw: &Obj<'_>) -> PyResult<()> {
        let py = s.py();
        let raw = raw.cast::<PyDict>().ok();
        for k in LENS {
            if matches!(k, "shift_x" | "shift_y" | "roll_deg") {
                s.setattr(k, 0.)?;
            } else {
                s.setattr(k, py.None())?;
            }
            let Some(v) = raw
                .map(|d| d.get_item(k))
                .transpose()?
                .flatten()
                .filter(|v| !v.is_none())
            else {
                continue;
            };
            if k == "sensor" {
                s.setattr(k, read_sensor(&v))?;
                continue;
            }
            let v = match py.import("builtins")?.call_method1("float", (v,)) {
                Ok(v) => v.extract::<f64>()?,
                Err(e)
                    if e.is_instance_of::<pyo3::exceptions::PyValueError>(py)
                        || e.is_instance_of::<pyo3::exceptions::PyTypeError>(py) =>
                {
                    continue
                }
                Err(e) => return Err(e),
            };
            if !v.is_finite() {
                continue;
            }
            let v = match k {
                "shift_x" | "shift_y" => v.clamp(-1., 1.),
                "roll_deg" => super::module(py, "math")?
                    .call_method1("remainder", (v, 360.))?
                    .extract()?,
                _ if v <= 0. => continue,
                _ => v,
            };
            s.setattr(k, v)?;
        }
        if let (Some(n), Some(f)) = (
            s.getattr("clip_near")?.extract::<Option<f64>>()?,
            s.getattr("clip_far")?.extract::<Option<f64>>()?,
        ) {
            if f <= n {
                s.setattr("clip_far", py.None())?;
            }
        }
        Ok(())
    }
    fn reset_lens(s: &Camera<'_>) -> PyResult<()> {
        let sensor = s.getattr("sensor")?;
        s.call_method1("apply_lens", (PyDict::new(s.py()),))?;
        s.setattr("sensor", sensor)
    }
    fn set_sensor(s: &Camera<'_>, size: &Obj<'_>) -> PyResult<()> {
        s.setattr("sensor", read_sensor(size))
    }
    fn sensor_size(s: &Camera<'_>) -> PyResult<(f64, f64)> {
        let sensor = s.getattr("sensor")?;
        if sensor.is_truthy()? {
            let [w, h] = sensor.extract()?;
            Ok((w, h))
        } else {
            Ok(FULL_FRAME)
        }
    }
    fn picture_aspect(s: &Camera<'_>) -> PyResult<f64> {
        let a = s.getattr("frame_aspect")?;
        if a.is_truthy()? {
            a.extract()
        } else {
            number(s, "aspect")
        }
    }
    fn picture_height_mm(s: &Camera<'_>) -> PyResult<f64> {
        sensor_picture_height(
            &s.call_method0("sensor_size")?,
            s.call_method0("picture_aspect")?.extract()?,
        )
    }
    #[getter]
    fn focal_mm(s: &Camera<'_>) -> PyResult<f64> {
        Ok(focal_from_fov(
            number(s, "fov_deg")?,
            s.call_method0("picture_height_mm")?.extract()?,
        ))
    }
    fn fov_for_focal(s: &Camera<'_>, focal_mm: f64) -> PyResult<f64> {
        Ok(fov_from_focal(
            focal_mm,
            s.call_method0("picture_height_mm")?.extract()?,
        ))
    }
    fn eye(s: &Camera<'_>) -> PyResult<Py<Vec3>> {
        live::camera_eye(s.py(), s)
    }
    fn forward(s: &Camera<'_>) -> PyResult<Py<Vec3>> {
        live::camera_forward(s.py(), s)
    }
    fn up_vector(s: &Camera<'_>) -> PyResult<Py<Vec3>> {
        live::camera_up_vector(s.py(), s)
    }
    fn look_from(s: &Camera<'_>, eye: &Obj<'_>, direction: &Obj<'_>) -> PyResult<()> {
        live::camera_look_from(s, eye, direction)
    }
    fn turn(s: &Camera<'_>, d_yaw_deg: f64, d_pitch_deg: f64) -> PyResult<()> {
        live::camera_turn(s, d_yaw_deg, d_pitch_deg)
    }
    fn move_eye(s: &Camera<'_>, delta: &Obj<'_>) -> PyResult<()> {
        s.setattr(
            "target",
            point(s.py(), xyz(&s.getattr("target")?)? + xyz(delta)?)?,
        )
    }
    fn place(s: &Camera<'_>, eye: &Obj<'_>, target: &Obj<'_>) -> PyResult<bool> {
        let d = xyz(target)? - xyz(eye)?;
        let length = d.length();
        if !length.is_finite() || length < 1e-6 {
            return Ok(false);
        }
        let before = xyz(&s.getattr("target")?)?;
        let dist = number(s, "distance")?;
        let yaw = number(s, "yaw")?;
        let pitch = number(s, "pitch")?;
        s.setattr("distance", length.min(MAX_DISTANCE))?;
        s.call_method1("look_from", (eye, point(s.py(), d)?))?;
        if s.call_method0("is_finite")?.is_truthy()? {
            return Ok(true);
        }
        s.setattr("target", point(s.py(), before)?)?;
        s.setattr("distance", dist)?;
        s.setattr("yaw", yaw)?;
        s.setattr("pitch", pitch)?;
        Ok(false)
    }
    fn view_matrix(s: &Camera<'_>) -> PyResult<Py<Matrix4>> {
        live::camera_view_matrix(s.py(), s)
    }
    fn projection_matrix(s: &Camera<'_>) -> PyResult<Py<Matrix4>> {
        live::camera_projection_matrix(s.py(), s, fitted(s)?)
    }
    fn parallel_depth(s: &Camera<'_>) -> PyResult<f64> {
        Ok(live::camera_depths(s, fitted(s)?)?.2)
    }
    fn near_plane(s: &Camera<'_>) -> PyResult<f64> {
        Ok(live::camera_depths(s, fitted(s)?)?.0)
    }
    fn far_plane(s: &Camera<'_>) -> PyResult<f64> {
        Ok(live::camera_depths(s, fitted(s)?)?.1)
    }
    #[pyo3(signature=(at=None))]
    fn min_distance(s: &Camera<'_>, at: Option<&Obj<'_>>) -> PyResult<f64> {
        match at {
            Some(v) => live::camera_min_distance(v),
            None => live::camera_min_distance(&s.getattr("target")?),
        }
    }
    fn picking_projection_matrix<'py>(s: &Camera<'py>) -> PyResult<Obj<'py>> {
        let old = s.getattr("_depth_range")?;
        s.setattr("_depth_range", s.py().None())?;
        let result = s.call_method0("projection_matrix");
        s.setattr("_depth_range", old)?;
        result
    }
    fn _depth_view_key<'py>(s: &Camera<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let py = s.py();
        let mut key = vec![py
            .get_type::<PyTuple>()
            .call1((s.call_method0("view_matrix")?.call_method0("data")?,))?];
        for k in [
            "fov_deg",
            "aspect",
            "perspective",
            "two_point",
            "znear",
            "zfar",
            "distance",
            "shift_x",
            "shift_y",
            "clip_near",
            "clip_far",
        ] {
            key.push(s.getattr(k)?);
        }
        PyTuple::new(py, key)
    }
    fn _fitted_depth_range(s: &Camera<'_>) -> PyResult<Option<(f64, f64)>> {
        let r = s.getattr("_depth_range")?;
        if r.is_none() || !r.get_item(0)?.eq(s.call_method0("_depth_view_key")?)? {
            return Ok(None);
        }
        Ok(Some((r.get_item(1)?.extract()?, r.get_item(2)?.extract()?)))
    }
    fn fit_depth_range(s: &Camera<'_>, lo: &Obj<'_>, hi: &Obj<'_>) -> PyResult<()> {
        s.setattr("_depth_range", s.py().None())?;
        if !s.getattr("perspective")?.is_truthy()?
            || !s.getattr("clip_near")?.is_none()
            || !s.getattr("clip_far")?.is_none()
            || lo.len()? == 0
        {
            return Ok(());
        }
        let np = super::module(s.py(), "numpy")?;
        let lo = np.call_method1("asarray", (lo, "float64"))?;
        let hi = np.call_method1("asarray", (hi, "float64"))?;
        let lo = lo.extract::<PyReadonlyArray2<f64>>()?;
        let hi = hi.extract::<PyReadonlyArray2<f64>>()?;
        let (lo, hi) = (lo.as_array(), hi.as_array());
        if lo.dim() != hi.dim() || lo.ncols() != 3 {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "expected matching N×3 camera bounds",
            ));
        }
        if !lo.iter().chain(hi.iter()).all(|v| v.is_finite()) {
            return Ok(());
        }
        let v: [f64; 16] = s
            .call_method0("view_matrix")?
            .call_method0("data")?
            .extract()?;
        let m: [f64; 16] = s
            .call_method0("projection_matrix")?
            .mul(s.call_method0("view_matrix")?)?
            .call_method0("data")?
            .extract()?;
        let planes = [(0, 1.), (0, -1.), (1, 1.), (1, -1.)].map(|(row, sign)| {
            std::array::from_fn::<_, 4, _>(|i| m[i * 4 + 3] + sign * m[i * 4 + row])
        });
        let depth = [-v[2], -v[6], -v[10], -v[14]];
        let (mut near, mut far) = (f64::INFINITY, f64::NEG_INFINITY);
        for (lo, hi) in lo.rows().into_iter().zip(hi.rows()) {
            // Keep the exact left-associated multiplies and adds of the NumPy expressions.
            let extent = |p: &[f64; 4], maximum: bool| {
                let corner = |i: usize| {
                    if (p[i] >= 0.) == maximum {
                        hi[i]
                    } else {
                        lo[i]
                    }
                };
                ((corner(0) * p[0] + corner(1) * p[1]) + corner(2) * p[2]) + p[3]
            };
            if planes.iter().any(|p| !(extent(p, true) >= 0.)) {
                continue;
            }
            let f = extent(&depth, true);
            if f > NEAR_FLOOR {
                near = near.min(extent(&depth, false));
                far = far.max(f);
            }
        }
        if far != f64::NEG_INFINITY {
            let n = NEAR_FLOOR.max(near * 0.95);
            if n > s.call_method0("near_plane")?.extract::<f64>()? {
                s.setattr(
                    "_depth_range",
                    (
                        s.call_method0("_depth_view_key")?,
                        n,
                        (n + NEAR_FLOOR).max(far * 1.05),
                    ),
                )?;
            }
        }
        Ok(())
    }
    fn orbit(s: &Camera<'_>, dx_pixels: f64, dy_pixels: f64, viewport_h: f64) -> PyResult<()> {
        let (y, p) = nav::orbit_angles(
            number(s, "yaw")?,
            number(s, "pitch")?,
            dx_pixels,
            dy_pixels,
            viewport_h,
        );
        s.setattr("yaw", y)?;
        s.setattr("pitch", p)
    }
    fn orbit_about(
        s: &Camera<'_>,
        pivot: &Obj<'_>,
        dx_pixels: f64,
        dy_pixels: f64,
        viewport_h: f64,
    ) -> PyResult<()> {
        let target = s.getattr("target")?.cast_into::<Vec3>()?;
        let (y, p) = (number(s, "yaw")?, number(s, "pitch")?);
        s.call_method1("orbit", (dx_pixels, dy_pixels, viewport_h))?;
        s.setattr(
            "target",
            nav::orbit_target(
                &target.borrow(),
                pivot,
                y,
                p,
                number(s, "yaw")?,
                number(s, "pitch")?,
            )?,
        )?;
        if !s.call_method0("is_finite")?.is_truthy()? {
            s.setattr("target", target)?;
            s.setattr("yaw", y)?;
            s.setattr("pitch", p)?;
        }
        Ok(())
    }
    #[pyo3(signature=(dx_pixels,dy_pixels,viewport_h,depth=None))]
    fn pan(
        s: &Camera<'_>,
        dx_pixels: f64,
        dy_pixels: f64,
        viewport_h: f64,
        depth: Option<f64>,
    ) -> PyResult<()> {
        let span = if let Some(d) = depth.filter(|d| {
            d.is_finite()
                && *d > 1e-6
                && s.getattr("perspective")
                    .is_ok_and(|v| v.is_truthy().unwrap_or(false))
        }) {
            d
        } else {
            number(s, "distance")?
        };
        let target = s.getattr("target")?.cast_into::<Vec3>()?;
        let up = s.call_method0("up_vector")?.cast_into::<Vec3>()?;
        let next = nav::pan_target(
            &target.borrow(),
            &up.borrow(),
            number(s, "yaw")?,
            number(s, "pitch")?,
            span,
            number(s, "fov_deg")?,
            dx_pixels,
            dy_pixels,
            viewport_h,
        );
        s.setattr("target", next)
    }
    fn zoom(s: &Camera<'_>, steps: f64) -> PyResult<()> {
        s.setattr(
            "distance",
            nav::zoom_distance(
                number(s, "distance")?,
                steps,
                s.call_method0("min_distance")?.extract()?,
                MAX_DISTANCE,
            ),
        )
    }
    #[pyo3(signature=(steps,focus,min_step=0.))]
    fn zoom_to(s: &Camera<'_>, steps: f64, focus: &Obj<'_>, min_step: f64) -> PyResult<()> {
        let eye = s.call_method0("eye")?.cast_into::<Vec3>()?;
        if steps < 0. && (xyz(&eye)? - xyz(focus)?).length() >= MAX_DISTANCE {
            s.call_method1("zoom", (steps,))?;
            return Ok(());
        }
        let target = s.getattr("target")?.cast_into::<Vec3>()?;
        let distance = number(s, "distance")?;
        let (next, d) = nav::zoom_focus(
            &target.borrow(),
            focus,
            &eye.borrow(),
            number(s, "yaw")?,
            number(s, "pitch")?,
            distance,
            steps,
            s.call_method1("min_distance", (focus,))?.extract()?,
            MAX_DISTANCE,
            min_step,
        )?;
        s.setattr("target", next)?;
        s.setattr("distance", d)?;
        if !s.call_method0("is_finite")?.is_truthy()? {
            s.setattr("target", target)?;
            s.setattr("distance", distance)?;
        }
        Ok(())
    }
    fn is_finite(s: &Camera<'_>) -> PyResult<bool> {
        Ok(xyz(&s.getattr("target")?)?.is_finite()
            && xyz(&s.getattr("up")?)?.is_finite()
            && number(s, "distance")?.is_finite()
            && number(s, "yaw")?.is_finite()
            && number(s, "pitch")?.is_finite())
    }
    fn sees_from_below(s: &Camera<'_>, z: f64) -> PyResult<bool> {
        if s.getattr("perspective")?.is_truthy()? {
            Ok(xyz(&s.call_method0("eye")?)?.z <= z)
        } else {
            Ok(xyz(&s.call_method0("forward")?)?.z > 0.)
        }
    }
    fn set_aspect(s: &Camera<'_>, w: f64, h: f64) -> PyResult<()> {
        s.setattr("aspect", w.max(1.) / h.max(1.))
    }
    fn toggle_projection(s: &Camera<'_>) -> PyResult<()> {
        s.setattr("perspective", !s.getattr("perspective")?.is_truthy()?)?;
        s.setattr("two_point", false)
    }
    fn toggle_two_point(s: &Camera<'_>) -> PyResult<()> {
        let on = !s.getattr("two_point")?.is_truthy()?;
        s.setattr("two_point", on)?;
        if on {
            s.setattr("perspective", true)?;
        }
        Ok(())
    }
    fn projection_mode(s: &Camera<'_>) -> PyResult<&'static str> {
        Ok(if !s.getattr("perspective")?.is_truthy()? {
            "parallel"
        } else if s.getattr("two_point")?.is_truthy()? {
            "two_point"
        } else {
            "perspective"
        })
    }
    fn set_projection(s: &Camera<'_>, mode: &str) -> PyResult<()> {
        if matches!(mode, "parallel" | "perspective" | "two_point") {
            s.setattr("perspective", mode != "parallel")?;
            s.setattr("two_point", mode == "two_point")?;
        }
        Ok(())
    }
    #[pyo3(signature=(min_pt,max_pt,margin=1.3))]
    fn fit_to(s: &Camera<'_>, min_pt: &Obj<'_>, max_pt: &Obj<'_>, margin: f64) -> PyResult<()> {
        live::camera_fit_to(s, min_pt, max_pt, margin)
    }
    #[pyo3(signature=(min_pt,max_pt,margin=1.1,free=None))]
    fn fit_box(
        s: &Camera<'_>,
        min_pt: &Obj<'_>,
        max_pt: &Obj<'_>,
        margin: f64,
        free: Option<&Obj<'_>>,
    ) -> PyResult<()> {
        let py = s.py();
        let (lo, hi) = (xyz(min_pt)?, xyz(max_pt)?);
        let center = (lo + hi) * 0.5;
        let anchor = DVec3::from_array(crate::resident::assemble::choose_origin(
            center.to_array(),
            [0.; 3],
        ));
        if anchor != DVec3::ZERO {
            let local = super::module(py, "copy")?.call_method1("copy", (s,))?;
            local.setattr("target", point(py, DVec3::ZERO)?)?;
            local.call_method1(
                "fit_box",
                (
                    point(py, lo - anchor)?,
                    point(py, hi - anchor)?,
                    margin,
                    free,
                ),
            )?;
            s.setattr(
                "target",
                point(py, anchor + xyz(&local.getattr("target")?)?)?,
            )?;
            return s.setattr("distance", local.getattr("distance")?);
        }
        let axes = live::camera_fit_box(py, s, min_pt, max_pt, margin)?;
        if let (Some((r, u, f)), Some(free)) = (axes, free) {
            let target = s.getattr("target")?;
            let (next, d) = nav::frame_box(
                min_pt,
                max_pt,
                &target,
                number(s, "distance")?,
                r.bind(py),
                u.bind(py),
                f.bind(py),
                number(s, "fov_deg")?,
                number(s, "aspect")?,
                s.getattr("perspective")?.is_truthy()?,
                (number(s, "shift_x")?, number(s, "shift_y")?),
                free.extract()?,
                margin,
                s.call_method1("min_distance", (&target,))?.extract()?,
                MAX_DISTANCE,
            )?;
            s.setattr("target", next)?;
            s.setattr("distance", d)?;
        }
        Ok(())
    }
    fn set_view(s: &Camera<'_>, name: &str) -> PyResult<()> {
        let presets = s.getattr("_STANDARD_VIEWS")?;
        if let Some(v) = presets.cast::<PyDict>()?.get_item(name)? {
            s.setattr("yaw", v.get_item(0)?)?;
            s.setattr("pitch", v.get_item(1)?)?;
        }
        Ok(())
    }
    fn snap_iso(s: &Camera<'_>) -> PyResult<()> {
        live::camera_snap_iso(s)
    }
}
fn fitted(s: &Camera<'_>) -> PyResult<Option<(f64, f64)>> {
    if s.getattr("_depth_range")?.is_none() {
        Ok(None)
    } else {
        s.call_method0("_fitted_depth_range")?.extract()
    }
}
pub(super) fn register(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = parent.py();
    let m = PyModule::new(py, "built_core.camera")?;
    m.add_class::<OrbitCamera>()?;
    for name in ["CameraPoint", "QVector3D"] {
        m.add(name, py.get_type::<Vec3>())?;
    }
    m.add("QMatrix4x4", py.get_type::<Matrix4>())?;
    for (k, v) in [
        ("MIN_DISTANCE", 0.005),
        ("MIN_DISTANCE_FAR", 0.02),
        ("_MIN_PER_METRE", 0.005 / 40.),
        ("_EMPTY_SIZE", 0.02),
        ("NEAR_FLOOR", NEAR_FLOOR),
        ("MAX_DISTANCE", MAX_DISTANCE),
        ("FAR_PER_DISTANCE", 4.),
        ("PARALLEL_REACH_MARGIN", 4.),
        ("SENSOR_HEIGHT_MM", 24.),
        ("MAX_SHIFT", 1.),
    ] {
        m.add(k, v)?;
    }
    m.add("FULL_FRAME", FULL_FRAME)?;
    m.add("SENSOR_RANGE_MM", (1., 100.))?;
    m.add("FOV_RANGE", (1., 120.))?;
    let sensors = PyDict::new(py);
    for (k, v) in [
        ("full_frame", FULL_FRAME),
        ("aps_c", (23.6, 15.6)),
        ("micro_four_thirds", (17.3, 13.0)),
        ("super_35", (24.9, 18.7)),
    ] {
        sensors.set_item(k, v)?;
    }
    m.add("SENSORS", sensors)?;
    let defaults = PyDict::new(py);
    for k in LENS {
        if matches!(k, "shift_x" | "shift_y" | "roll_deg") {
            defaults.set_item(k, 0.)?;
        } else {
            defaults.set_item(k, py.None())?;
        }
    }
    m.add("LENS_DEFAULTS", defaults)?;
    for f in [
        wrap_pyfunction!(focal_from_fov, &m)?,
        wrap_pyfunction!(fov_from_focal, &m)?,
        wrap_pyfunction!(frame_rect, &m)?,
        wrap_pyfunction!(sensor_picture_height, &m)?,
    ] {
        m.add_function(f)?;
    }
    parent.add("camera", m)
}
