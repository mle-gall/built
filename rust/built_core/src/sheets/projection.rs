// SPDX-License-Identifier: GPL-3.0-or-later
//! Camera-space metres to frame-local paper millimetres, for individual
//! points, HLR's arrays and anchored sheet items.
use numpy::{ndarray::Zip, IntoPyArray, PyArrayDyn, PyArrayMethods};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyFloat, PyInt},
};

#[pyclass(module = "built_core", frozen, skip_from_py_object)]
#[derive(Clone)]
pub struct PageProjection {
    w: f64,
    h: f64,
    aspect: f64,
    half_h: f64,
    scale: f64,
    tangent: Option<f64>,
}

impl PageProjection {
    pub(super) fn for_frame(w: f64, h: f64, n: f64, fov: Option<f64>) -> Self {
        let half_h = super::model::model_height_for_frame(h, n) / 2.;
        Self {
            w,
            h,
            aspect: if h != 0. { w / h } else { 1. },
            half_h,
            scale: if half_h != 0. { h / (2. * half_h) } else { 0. },
            tangent: fov.map(|v| (v.to_radians() / 2.).tan()),
        }
    }
    fn axis(&self, value: f64, depth: f64, horizontal: bool) -> f64 {
        if let Some(t) = self.tangent {
            let hh = t * if depth < 1e-4 { 1e-4 } else { depth };
            let (v, size) = if horizontal {
                ((value / (hh * self.aspect) + 1.) * self.w / 2., self.w)
            } else {
                ((1. - value / hh) * self.h / 2., self.h)
            };
            if v.is_nan() {
                v
            } else {
                v.max(-10. * size).min(10. * size + size)
            }
        } else if horizontal {
            (value + self.half_h * self.aspect) * self.scale
        } else {
            (self.half_h - value) * self.scale
        }
    }
    pub(super) fn local(&self, x: f64, y: f64, z: f64) -> [f64; 2] {
        [self.axis(x, z, true), self.axis(y, z, false)]
    }
}

#[pymethods]
impl PageProjection {
    #[new]
    #[pyo3(signature=(width,height,scale,fov=None))]
    fn new(width: f64, height: f64, scale: f64, fov: Option<f64>) -> PyResult<Self> {
        if fov.is_none() && (height <= 0. || scale <= 0.) {
            return Err(PyValueError::new_err(
                "frame height and scale must be positive",
            ));
        }
        Ok(Self::for_frame(width, height, scale, fov))
    }
    #[pyo3(signature=(cx,cy,cz=None))]
    fn __call__<'py>(
        &self,
        py: Python<'py>,
        cx: &Bound<'py, PyAny>,
        cy: &Bound<'py, PyAny>,
        cz: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let scalar = |v: &Bound<'py, PyAny>| {
            v.is_exact_instance_of::<PyFloat>() || v.is_exact_instance_of::<PyInt>()
        };
        if scalar(cx) && scalar(cy) && (self.tangent.is_none() || cz.is_some_and(scalar)) {
            let z = if self.tangent.is_some() {
                cz.unwrap().extract()?
            } else {
                0.
            };
            let [x, y] = self.local(cx.extract()?, cy.extract()?, z);
            return Ok((x, y).into_pyobject(py)?.into_any());
        }
        if self.tangent.is_none() {
            // NumPy keeps the input float width and independent x/y shapes.
            let x = cx.add(self.half_h * self.aspect)?.mul(self.scale)?;
            let y = self
                .half_h
                .into_pyobject(py)?
                .into_any()
                .sub(cy)?
                .mul(self.scale)?;
            return Ok((x, y).into_pyobject(py)?.into_any());
        }
        let z =
            cz.ok_or_else(|| PyValueError::new_err("a perspective frame projects with the depth"))?;
        let np = py.import("numpy")?;
        let dtype = np.getattr("float64")?;
        let array = |v: &Bound<'py, PyAny>| np.call_method1("asarray", (v, &dtype));
        let z = array(z)?;
        let scalar_depth = z.getattr("ndim")?.extract::<usize>()? == 0;
        let project = |v: &Bound<'py, PyAny>, horizontal| -> PyResult<Bound<'py, PyAny>> {
            let same = np.call_method1("broadcast_arrays", (array(v)?, &z))?;
            let coords = same.get_item(0)?.cast_into::<PyArrayDyn<f64>>()?;
            let depths = same.get_item(1)?.cast_into::<PyArrayDyn<f64>>()?;
            let coords = coords.readonly();
            let depths = depths.readonly();
            let projected = Zip::from(coords.as_array())
                .and(depths.as_array())
                .map_collect(|&v, &d| self.axis(v, d, horizontal))
                .into_pyarray(py);
            if scalar_depth {
                return py.import("builtins")?.call_method1("float", (projected,));
            }
            Ok(projected.into_any())
        };
        Ok((project(cx, true)?, project(cy, false)?)
            .into_pyobject(py)?
            .into_any())
    }
}
