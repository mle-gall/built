// SPDX-License-Identifier: GPL-3.0-or-later
//! SpaceMouse driver coordinates, response curve and camera movement.
use super::{ACTIONS, AXES, FREECAD, ORIGINAL};
use pyo3::{
    exceptions::PyZeroDivisionError,
    prelude::*,
    types::{IntoPyDict, PyDict, PyTuple},
};

const FULL_SCALE: f64 = 350.;
const FREECAD_INVERSION: [bool; 6] = [false, true, false, true, false, false];

#[pyclass(get_all, set_all, eq, module = "ingetrazo_render.ndof", from_py_object)]
#[derive(Clone, Copy, PartialEq)]
struct NdofSample {
    right: f64,
    up: f64,
    forward: f64,
    tilt: f64,
    spin: f64,
    roll: f64,
}
#[pymethods]
impl NdofSample {
    #[new]
    #[pyo3(signature=(right=0.,up=0.,forward=0.,tilt=0.,spin=0.,roll=0.))]
    fn new(right: f64, up: f64, forward: f64, tilt: f64, spin: f64, roll: f64) -> Self {
        Self {
            right,
            up,
            forward,
            tilt,
            spin,
            roll,
        }
    }
    #[staticmethod]
    #[pyo3(signature=(right,up,forward,tilt,spin,roll,full_scale=FULL_SCALE))]
    fn from_raw(
        right: f64,
        up: f64,
        forward: f64,
        tilt: f64,
        spin: f64,
        roll: f64,
        full_scale: f64,
    ) -> PyResult<Self> {
        if full_scale == 0. {
            return Err(PyZeroDivisionError::new_err("full scale is zero"));
        }
        let [r, u, f, t, s, a] =
            [right, up, forward, tilt, spin, roll].map(|v| (v / full_scale).clamp(-1., 1.));
        Ok(Self::new(r, u, f, t, s, a))
    }
    fn is_idle(&self) -> bool {
        self.values().iter().all(|v| *v == 0.)
    }
}
impl NdofSample {
    fn values(&self) -> [f64; 6] {
        [
            self.right,
            self.up,
            self.forward,
            self.tilt,
            self.spin,
            self.roll,
        ]
    }
}
#[pyfunction]
fn from_hid(x: f64, y: f64, z: f64, rx: f64, ry: f64, rz: f64) -> PyResult<NdofSample> {
    NdofSample::from_raw(x, -z, y, -rx, -rz, ry, FULL_SCALE)
}
#[pyfunction]
fn from_spacenavd(x: f64, y: f64, z: f64, rx: f64, ry: f64, rz: f64) -> PyResult<NdofSample> {
    NdofSample::from_raw(x, y, -z, -rx, ry, -rz, FULL_SCALE)
}

#[pyclass(get_all, set_all, module = "ingetrazo_render.ndof")]
struct NdofSettings {
    enabled: bool,
    sensitivity: f64,
    deadzone: f64,
    invert_pan: bool,
    invert_zoom: bool,
    invert_rotate: bool,
    invert_pan_x: bool,
    invert_pan_y: bool,
    invert_tilt: bool,
    invert_spin: bool,
    lock_rotation: bool,
    mapping_preset: String,
    axis_mapping: Py<PyDict>,
    axis_inversion: Py<PyDict>,
    axis_sensitivity: Py<PyDict>,
}
#[pymethods]
impl NdofSettings {
    #[new]
    #[pyo3(signature=(enabled=true,sensitivity=1.,deadzone=0.05,invert_pan=false,invert_zoom=false,invert_rotate=false,invert_pan_x=false,invert_pan_y=false,invert_tilt=false,invert_spin=false,lock_rotation=false,mapping_preset="original",axis_mapping=None,axis_inversion=None,axis_sensitivity=None))]
    fn new(
        py: Python<'_>,
        enabled: bool,
        sensitivity: f64,
        deadzone: f64,
        invert_pan: bool,
        invert_zoom: bool,
        invert_rotate: bool,
        invert_pan_x: bool,
        invert_pan_y: bool,
        invert_tilt: bool,
        invert_spin: bool,
        lock_rotation: bool,
        mapping_preset: &str,
        axis_mapping: Option<Py<PyDict>>,
        axis_inversion: Option<Py<PyDict>>,
        axis_sensitivity: Option<Py<PyDict>>,
    ) -> PyResult<Self> {
        let axis_mapping = match axis_mapping {
            Some(v) => v,
            None => AXES.into_iter().zip(ORIGINAL).into_py_dict(py)?.unbind(),
        };
        let axis_inversion = match axis_inversion {
            Some(v) => v,
            None => AXES.into_iter().zip([false; 6]).into_py_dict(py)?.unbind(),
        };
        let axis_sensitivity = match axis_sensitivity {
            Some(v) => v,
            None => AXES.into_iter().zip([1.; 6]).into_py_dict(py)?.unbind(),
        };
        Ok(Self {
            enabled,
            sensitivity,
            deadzone,
            invert_pan,
            invert_zoom,
            invert_rotate,
            invert_pan_x,
            invert_pan_y,
            invert_tilt,
            invert_spin,
            lock_rotation,
            mapping_preset: mapping_preset.into(),
            axis_mapping,
            axis_inversion,
            axis_sensitivity,
        })
    }
}
#[pyfunction]
fn shape(v: f64, deadzone: f64) -> f64 {
    if v.abs() <= deadzone {
        return 0.;
    }
    let t = (v.abs() - deadzone) / (1. - deadzone);
    (0.6 * t * t + 0.4 * t).copysign(v)
}
fn movements(py: Python<'_>, sample: &NdofSample, settings: &NdofSettings) -> PyResult<[f64; 5]> {
    let original_inversion = [
        settings.invert_pan ^ settings.invert_pan_x,
        settings.invert_pan ^ settings.invert_pan_y,
        settings.invert_zoom,
        settings.invert_rotate ^ settings.invert_tilt,
        settings.invert_rotate ^ settings.invert_spin,
        false,
    ];
    let mut out = [0.; 5];
    for (i, v) in sample.values().into_iter().enumerate() {
        let axis = AXES[i];
        let (action, inverted, gain) = match settings.mapping_preset.as_str() {
            "custom" => {
                let action = settings
                    .axis_mapping
                    .bind(py)
                    .get_item(axis)?
                    .map(|s| s.extract::<String>())
                    .transpose()?
                    .unwrap_or_else(|| ORIGINAL[i].to_owned());
                let index = ACTIONS[..5].iter().position(|a| *a == action);
                let inverted = settings
                    .axis_inversion
                    .bind(py)
                    .get_item(axis)?
                    .map(|v| v.is_truthy())
                    .transpose()?
                    .unwrap_or(false);
                let gain = settings
                    .axis_sensitivity
                    .bind(py)
                    .get_item(axis)?
                    .map(|v| v.extract::<f64>())
                    .transpose()?
                    .unwrap_or(1.);
                (index, inverted, gain)
            }
            "freecad" => (
                ACTIONS[..5].iter().position(|a| *a == FREECAD[i]),
                FREECAD_INVERSION[i],
                1.,
            ),
            _ => (
                ACTIONS[..5].iter().position(|a| *a == ORIGINAL[i]),
                original_inversion[i],
                1.,
            ),
        };
        if let Some(index) = action {
            let value = shape(v, settings.deadzone) * gain;
            out[index] += if inverted { -value } else { value };
        }
    }
    Ok(out.map(|v| v.clamp(-1., 1.)))
}
#[pyfunction]
fn mapped_movements<'p>(
    py: Python<'p>,
    sample: &NdofSample,
    settings: &NdofSettings,
) -> PyResult<Bound<'p, PyDict>> {
    ACTIONS
        .into_iter()
        .zip(movements(py, sample, settings)?)
        .into_py_dict(py)
}
#[pyfunction(signature=(camera,sample,dt,viewport_h,settings=None))]
fn apply_ndof(
    camera: &Bound<PyAny>,
    sample: &NdofSample,
    dt: f64,
    viewport_h: i32,
    settings: Option<&NdofSettings>,
) -> PyResult<bool> {
    let py = camera.py();
    let default;
    let st = match settings {
        Some(st) => st,
        None => {
            default = NdofSettings::new(
                py, true, 1., 0.05, false, false, false, false, false, false, false, false,
                "original", None, None, None,
            )?;
            &default
        }
    };
    if !st.enabled || dt <= 0. {
        return Ok(false);
    }
    let [right, up, forward, spin, tilt] = movements(py, sample, st)?;
    let k = st.sensitivity * dt.min(0.1);
    let h = viewport_h.max(1);
    let pan = right != 0. || up != 0.;
    let orbit = !st.lock_rotation && (spin != 0. || tilt != 0.);
    if pan {
        let px = f64::from(h) * k;
        camera.call_method1("pan", (right * px, -up * px, h))?;
    }
    if forward != 0. {
        camera.call_method1("zoom", (-forward * 8. * k,))?;
    }
    if orbit {
        let px = 2. * k * f64::from(h) / std::f64::consts::PI;
        camera.call_method1("orbit", (spin * px, tilt * px, h))?;
    }
    Ok(pan || forward != 0. || orbit)
}
pub(crate) fn register(root: &Bound<PyModule>) -> PyResult<()> {
    let py = root.py();
    let name = "ingetrazo_render.ndof";
    let m = PyModule::new(py, name)?;
    m.add("AXES", PyTuple::new(py, AXES)?)?;
    m.add("ACTIONS", PyTuple::new(py, ACTIONS)?)?;
    m.add(
        "DEFAULT_MAPPING",
        AXES.into_iter().zip(ORIGINAL).into_py_dict(py)?,
    )?;
    m.add(
        "FREECAD_MAPPING",
        AXES.into_iter().zip(FREECAD).into_py_dict(py)?,
    )?;
    m.add(
        "FREECAD_INVERSION",
        AXES.into_iter().zip(FREECAD_INVERSION).into_py_dict(py)?,
    )?;
    m.add_class::<NdofSettings>()?;
    m.add_class::<NdofSample>()?;
    for f in [
        wrap_pyfunction!(from_hid, &m)?,
        wrap_pyfunction!(from_spacenavd, &m)?,
        wrap_pyfunction!(shape, &m)?,
        wrap_pyfunction!(apply_ndof, &m)?,
        wrap_pyfunction!(mapped_movements, &m)?,
    ] {
        m.add_function(f)?;
    }
    py.import("sys")?.getattr("modules")?.set_item(name, &m)?;
    root.add("ndof", m)
}
