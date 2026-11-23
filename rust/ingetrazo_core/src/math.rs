// SPDX-License-Identifier: GPL-3.0-or-later
//! Binary64 model coordinates and OpenGL-convention camera transforms.
#![allow(non_snake_case)]
use glam::{DMat4, DQuat, DVec3, DVec4};
use pyo3::{
    exceptions::{PyIndexError, PyTypeError},
    prelude::*,
    types::PyTuple,
};

#[pyclass(module = "ingetrazo_core", skip_from_py_object)]
#[derive(Clone)]
pub struct Vec3 {
    v: DVec3,
    hashed: bool,
}

impl From<DVec3> for Vec3 {
    fn from(v: DVec3) -> Self {
        Self { v, hashed: false }
    }
}

// Accept legacy vectors and coordinate triples at API boundaries.
fn xyz(obj: &Bound<'_, PyAny>) -> PyResult<DVec3> {
    if let Ok(v) = obj.cast::<Vec3>() {
        return Ok(v.borrow().v);
    }
    let coords = if obj.hasattr("toTuple")? {
        obj.call_method0("toTuple")?
    } else {
        obj.clone()
    };
    Ok(DVec3::from_array(coords.extract()?))
}

impl Vec3 {
    fn update(&mut self, v: DVec3) -> PyResult<()> {
        if self.hashed {
            return Err(PyTypeError::new_err("cannot modify a hashed vector"));
        }
        self.v = v;
        Ok(())
    }
}

#[pymethods]
impl Vec3 {
    #[new]
    #[pyo3(signature=(x=None,y=None,z=None))]
    fn new(x: Option<&Bound<'_, PyAny>>, y: Option<f64>, z: Option<f64>) -> PyResult<Self> {
        let v = match (x, y, z) {
            (None, None, None) => DVec3::ZERO,
            (Some(x), Some(y), Some(z)) => DVec3::new(x.extract()?, y, z),
            (Some(x), None, None) => xyz(x)?,
            _ => {
                return Err(PyTypeError::new_err(
                    "expected a vector or three coordinates",
                ))
            }
        };
        Ok(v.into())
    }
    fn x(&self) -> f64 {
        self.v.x
    }
    fn y(&self) -> f64 {
        self.v.y
    }
    fn z(&self) -> f64 {
        self.v.z
    }
    fn setX(&mut self, x: f64) -> PyResult<()> {
        self.update(self.v.with_x(x))
    }
    fn setY(&mut self, y: f64) -> PyResult<()> {
        self.update(self.v.with_y(y))
    }
    fn setZ(&mut self, z: f64) -> PyResult<()> {
        self.update(self.v.with_z(z))
    }
    fn toTuple(&self) -> (f64, f64, f64) {
        self.v.into()
    }
    fn __add__(&self, r: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok((self.v + xyz(r)?).into())
    }
    fn __radd__(&self, r: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.__add__(r)
    }
    fn __sub__(&self, r: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok((self.v - xyz(r)?).into())
    }
    fn __rsub__(&self, l: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok((xyz(l)? - self.v).into())
    }
    fn __iadd__(slf: &Bound<'_, Self>, rhs: &Bound<'_, PyAny>) -> PyResult<()> {
        let result = slf.borrow().v + xyz(rhs)?;
        slf.borrow_mut().update(result)
    }
    fn __isub__(slf: &Bound<'_, Self>, rhs: &Bound<'_, PyAny>) -> PyResult<()> {
        let result = slf.borrow().v - xyz(rhs)?;
        slf.borrow_mut().update(result)
    }
    fn __imul__(&mut self, scale: f64) -> PyResult<()> {
        self.update(self.v * scale)
    }
    fn __itruediv__(&mut self, scale: f64) -> PyResult<()> {
        self.update(self.v / scale)
    }
    fn __neg__(&self) -> Self {
        (-self.v).into()
    }
    fn __mul__(&self, s: f64) -> Self {
        (self.v * s).into()
    }
    fn __rmul__(&self, s: f64) -> Self {
        self.__mul__(s)
    }
    fn __truediv__(&self, s: f64) -> Self {
        (self.v / s).into()
    }
    fn __eq__(&self, r: &Bound<'_, PyAny>) -> bool {
        r.hasattr("toTuple").unwrap_or(false) && xyz(r).is_ok_and(|v| v == self.v)
    }
    fn __hash__(&mut self, py: Python<'_>) -> PyResult<isize> {
        self.hashed = true;
        self.toTuple().into_pyobject(py)?.hash()
    }
    fn __reduce__<'py>(
        slf: &Bound<'py, Self>,
    ) -> (Bound<'py, pyo3::types::PyType>, (f64, f64, f64)) {
        (slf.get_type(), slf.borrow().toTuple())
    }
    fn __repr__(&self) -> String {
        format!("Vec3({}, {}, {})", self.v.x, self.v.y, self.v.z)
    }
    fn length(&self) -> f64 {
        self.v.length()
    }
    fn lengthSquared(&self) -> f64 {
        self.v.length_squared()
    }
    fn normalized(&self) -> Self {
        self.v.normalize_or_zero().into()
    }
    fn normalize(&mut self) -> PyResult<()> {
        self.update(self.v.normalize_or_zero())
    }
    fn isNull(&self) -> bool {
        self.v == DVec3::ZERO
    }
    fn distanceToPoint(&self, r: &Bound<'_, PyAny>) -> PyResult<f64> {
        Ok(self.v.distance(xyz(r)?))
    }
    #[staticmethod]
    fn dotProduct(a: &Bound<'_, PyAny>, b: &Bound<'_, PyAny>) -> PyResult<f64> {
        Ok(xyz(a)?.dot(xyz(b)?))
    }
    #[staticmethod]
    fn crossProduct(a: &Bound<'_, PyAny>, b: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(xyz(a)?.cross(xyz(b)?).into())
    }
}

#[pyclass(module = "ingetrazo_core")]
pub struct Quaternion(DQuat);
#[pymethods]
impl Quaternion {
    #[staticmethod]
    fn fromAxisAndAngle(axis: &Bound<'_, PyAny>, angle: f64) -> PyResult<Self> {
        Ok(Self(DQuat::from_axis_angle(
            xyz(axis)?.normalize_or_zero(),
            angle.to_radians(),
        )))
    }
    fn __mul__(&self, rhs: &Self) -> Self {
        Self(self.0 * rhs.0)
    }
    fn rotatedVector(&self, v: &Bound<'_, PyAny>) -> PyResult<Vec3> {
        Ok((self.0 * xyz(v)?).into())
    }
}

#[pyclass(module = "ingetrazo_core")]
pub struct Vec4(DVec4);
#[pymethods]
impl Vec4 {
    #[new]
    fn new(x: f64, y: f64, z: f64, w: f64) -> Self {
        Self(DVec4::new(x, y, z, w))
    }
    fn x(&self) -> f64 {
        self.0.x
    }
    fn y(&self) -> f64 {
        self.0.y
    }
    fn z(&self) -> f64 {
        self.0.z
    }
    fn w(&self) -> f64 {
        self.0.w
    }
    fn __add__(&self, rhs: &Self) -> Self {
        Self(self.0 + rhs.0)
    }
    fn __sub__(&self, rhs: &Self) -> Self {
        Self(self.0 - rhs.0)
    }
    fn __mul__(&self, rhs: f64) -> Self {
        Self(self.0 * rhs)
    }
    fn setW(&mut self, w: f64) {
        self.0.w = w;
    }
    fn toVector3D(&self) -> Vec3 {
        self.0.truncate().into()
    }
    fn toTuple(&self) -> (f64, f64, f64, f64) {
        self.0.into()
    }
}

#[pyclass(module = "ingetrazo_core", subclass, dict)]
pub struct Matrix4(DMat4);

fn mat(obj: &Bound<'_, PyAny>) -> PyResult<DMat4> {
    if let Ok(m) = obj.cast::<Matrix4>() {
        return Ok(m.borrow().0);
    }
    Ok(DMat4::from_cols_array(
        &obj.call_method0("data")?.extract()?,
    ))
}
fn dimensions(
    x: &Bound<'_, PyAny>,
    y: Option<f64>,
    z: Option<f64>,
    scale: bool,
) -> PyResult<DVec3> {
    if let Ok(x) = x.extract::<f64>() {
        if let Some(y) = y {
            return Ok(DVec3::new(x, y, z.unwrap_or(if scale { 1. } else { 0. })));
        }
        if scale && z.is_none() {
            return Ok(DVec3::splat(x));
        }
    }
    xyz(x)
}

#[pymethods]
impl Matrix4 {
    #[new]
    #[pyo3(signature=(*args))]
    fn new(args: &Bound<'_, PyTuple>) -> PyResult<Self> {
        if args.is_empty() {
            return Ok(Self(DMat4::IDENTITY));
        }
        let rows: [f64; 16] = if args.len() == 1 {
            let a = args.get_item(0)?;
            if a.hasattr("data")? && !a.hasattr("shape")? {
                return Ok(Self(mat(&a)?));
            }
            a.extract()?
        } else {
            args.extract()?
        };
        Ok(Self(DMat4::from_cols_array(&rows).transpose()))
    }
    fn data<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, self.0.to_cols_array())
    }
    fn copyDataTo(&self) -> Vec<f64> {
        self.0.transpose().to_cols_array().into()
    }
    fn determinant(&self) -> f64 {
        self.0.determinant()
    }
    fn isAffine(&self) -> bool {
        self.0.row(3) == DVec4::W
    }
    fn isIdentity(&self) -> bool {
        self.0 == DMat4::IDENTITY
    }
    fn setToIdentity(&mut self) {
        self.0 = DMat4::IDENTITY
    }
    fn inverted(&self) -> (Self, bool) {
        if self.determinant() == 0. {
            (Self(DMat4::IDENTITY), false)
        } else {
            (Self(self.0.inverse()), true)
        }
    }
    fn transposed(&self) -> Self {
        Self(self.0.transpose())
    }
    fn __eq__(&self, r: &Bound<'_, PyAny>) -> bool {
        mat(r).is_ok_and(|m| self.0 == m)
    }
    fn __mul__(&self, r: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self(self.0 * mat(r)?))
    }
    fn __imul__(slf: &Bound<'_, Self>, r: &Bound<'_, PyAny>) -> PyResult<()> {
        let rhs = mat(r)?;
        slf.borrow_mut().0 *= rhs;
        Ok(())
    }
    fn __iadd__(slf: &Bound<'_, Self>, r: &Bound<'_, PyAny>) -> PyResult<()> {
        let rhs = mat(r)?;
        slf.borrow_mut().0 += rhs;
        Ok(())
    }
    fn __isub__(slf: &Bound<'_, Self>, r: &Bound<'_, PyAny>) -> PyResult<()> {
        let rhs = mat(r)?;
        slf.borrow_mut().0 -= rhs;
        Ok(())
    }
    fn __itruediv__(&mut self, s: f64) {
        self.0 *= 1. / s
    }
    fn __reduce__<'py>(slf: &Bound<'py, Self>) -> (Bound<'py, pyo3::types::PyType>, (Vec<f64>,)) {
        (slf.get_type(), (slf.borrow().copyDataTo(),))
    }
    fn mapVector(&self, v: &Bound<'_, PyAny>) -> PyResult<Vec3> {
        Ok(self.0.transform_vector3(xyz(v)?).into())
    }
    fn map<'py>(&self, v: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        if let Ok(v4) = v.cast::<Vec4>() {
            return Ok(Vec4(self.0 * v4.borrow().0)
                .into_pyobject(v.py())?
                .into_any());
        }
        if !v.is_instance_of::<Vec3>() && v.hasattr("w")? {
            let coords: [f64; 4] = v.call_method0("toTuple")?.extract()?;
            return Ok(Vec4(self.0 * DVec4::from_array(coords))
                .into_pyobject(v.py())?
                .into_any());
        }
        let h = self.0 * xyz(v)?.extend(1.);
        Ok(Vec3::from(h.truncate() / h.w)
            .into_pyobject(v.py())?
            .into_any())
    }
    #[pyo3(signature=(x,y=None,z=None))]
    fn translate(&mut self, x: &Bound<'_, PyAny>, y: Option<f64>, z: Option<f64>) -> PyResult<()> {
        self.0 *= DMat4::from_translation(dimensions(x, y, z, false)?);
        Ok(())
    }
    #[pyo3(signature=(x,y=None,z=None))]
    fn scale(&mut self, x: &Bound<'_, PyAny>, y: Option<f64>, z: Option<f64>) -> PyResult<()> {
        self.0 *= DMat4::from_scale(dimensions(x, y, z, true)?);
        Ok(())
    }
    #[pyo3(signature=(angle,x,y=None,z=None))]
    fn rotate(
        &mut self,
        angle: f64,
        x: &Bound<'_, PyAny>,
        y: Option<f64>,
        z: Option<f64>,
    ) -> PyResult<()> {
        let axis = dimensions(x, y, z, false)?.normalize_or_zero();
        if axis != DVec3::ZERO {
            self.0 *= DMat4::from_axis_angle(axis, angle.to_radians())
        }
        Ok(())
    }
    fn lookAt(
        &mut self,
        eye: &Bound<'_, PyAny>,
        target: &Bound<'_, PyAny>,
        up: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        self.0 *= DMat4::look_at_rh(xyz(eye)?, xyz(target)?, xyz(up)?.normalize_or_zero());
        Ok(())
    }
    fn perspective(&mut self, angle: f64, aspect: f64, near: f64, far: f64) {
        self.0 *= DMat4::perspective_rh_gl(angle.to_radians(), aspect, near, far)
    }
    fn ortho(&mut self, l: f64, r: f64, b: f64, t: f64, n: f64, f: f64) {
        self.0 *= DMat4::orthographic_rh_gl(l, r, b, t, n, f)
    }
    fn frustum(&mut self, l: f64, r: f64, b: f64, t: f64, n: f64, f: f64) {
        self.0 *= DMat4::from_cols_array(&[
            2. * n / (r - l),
            0.,
            0.,
            0.,
            0.,
            2. * n / (t - b),
            0.,
            0.,
            (r + l) / (r - l),
            (t + b) / (t - b),
            -(f + n) / (f - n),
            -1.,
            0.,
            0.,
            -2. * f * n / (f - n),
            0.,
        ])
    }
    fn __call__(&self, row: usize, column: usize) -> PyResult<f64> {
        if row >= 4 || column >= 4 {
            return Err(PyIndexError::new_err("matrix index"));
        }
        Ok(self.0.col(column)[row])
    }
    fn __getitem__(&self, index: (usize, usize)) -> PyResult<f64> {
        self.__call__(index.0, index.1)
    }
    fn column(&self, i: usize) -> PyResult<Vec4> {
        if i >= 4 {
            return Err(PyIndexError::new_err("column"));
        }
        Ok(Vec4(self.0.col(i)))
    }
    fn row(&self, i: usize) -> PyResult<Vec4> {
        if i >= 4 {
            return Err(PyIndexError::new_err("row"));
        }
        Ok(Vec4(self.0.row(i)))
    }
    fn setColumn(&mut self, i: usize, v: &Vec4) -> PyResult<()> {
        if i >= 4 {
            return Err(PyIndexError::new_err("column"));
        }
        *self.0.col_mut(i) = v.0;
        Ok(())
    }
    fn setRow(&mut self, i: usize, v: &Vec4) -> PyResult<()> {
        if i >= 4 {
            return Err(PyIndexError::new_err("row"));
        }
        let mut m = self.0.transpose();
        *m.col_mut(i) = v.0;
        self.0 = m.transpose();
        Ok(())
    }
    fn fill(&mut self, v: f64) {
        self.0 = DMat4::from_cols_array(&[v; 16])
    }
    fn flipCoordinates(&mut self) {
        self.0 *= DMat4::from_scale(DVec3::new(1., -1., -1.))
    }
    fn optimize(&self) {}
    #[pyo3(signature=(x,y,w,h,n=0.,f=1.))]
    fn viewport(&mut self, x: f64, y: f64, w: f64, h: f64, n: f64, f: f64) {
        self.0 *= DMat4::from_translation(DVec3::new(x + w / 2., y + h / 2., (n + f) / 2.))
            * DMat4::from_scale(DVec3::new(w / 2., h / 2., (f - n) / 2.))
    }
}
