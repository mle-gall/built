//! Logical and physical screen coordinates at the native input boundary.
#![allow(non_snake_case)]
use glam::DVec2;
use pyo3::{exceptions::PyTypeError, prelude::*};

fn coords(obj: &Bound<'_, PyAny>) -> PyResult<DVec2> {
    if let Ok(p) = obj.cast::<ScreenPoint>() {
        return Ok(p.borrow().0);
    }
    if let Ok(p) = obj.cast::<PixelPoint>() {
        let p = p.borrow();
        return Ok(DVec2::new(p.0 as f64, p.1 as f64));
    }
    Ok(DVec2::new(
        obj.call_method0("x")?.extract()?,
        obj.call_method0("y")?.extract()?,
    ))
}

fn args(x: Option<&Bound<'_, PyAny>>, y: Option<f64>) -> PyResult<DVec2> {
    match (x, y) {
        (None, None) => Ok(DVec2::ZERO),
        (Some(p), None) => coords(p),
        (Some(x), Some(y)) => Ok(DVec2::new(x.extract()?, y)),
        _ => Err(PyTypeError::new_err("expected a point or two coordinates")),
    }
}

#[pyclass(module = "built_core", frozen)]
pub struct ScreenPoint(DVec2);

#[pymethods]
impl ScreenPoint {
    #[new]
    #[pyo3(signature=(x=None, y=None))]
    fn new(x: Option<&Bound<'_, PyAny>>, y: Option<f64>) -> PyResult<Self> {
        Ok(Self(args(x, y)?))
    }
    fn x(&self) -> f64 {
        self.0.x
    }
    fn y(&self) -> f64 {
        self.0.y
    }
    fn toTuple(&self) -> (f64, f64) {
        self.0.into()
    }
    fn toPoint(&self) -> PixelPoint {
        PixelPoint::from(self.0)
    }
    fn isNull(&self) -> bool {
        self.0 == DVec2::ZERO
    }
    fn manhattanLength(&self) -> f64 {
        self.0.abs().element_sum()
    }
    fn __add__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self(self.0 + coords(other)?))
    }
    fn __sub__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self(self.0 - coords(other)?))
    }
    fn __mul__(&self, n: f64) -> Self {
        Self(self.0 * n)
    }
    fn __truediv__(&self, n: f64) -> Self {
        Self(self.0 / n)
    }
    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        coords(other).is_ok_and(|v| v == self.0)
    }
}
#[pyclass(module = "built_core", frozen)]
pub struct PixelPoint(i32, i32);
impl From<DVec2> for PixelPoint {
    fn from(v: DVec2) -> Self {
        Self(v.x.round() as i32, v.y.round() as i32)
    }
}
#[pymethods]
impl PixelPoint {
    #[new]
    #[pyo3(signature=(x=None, y=None))]
    fn new(x: Option<&Bound<'_, PyAny>>, y: Option<f64>) -> PyResult<Self> {
        Ok(args(x, y)?.into())
    }
    fn x(&self) -> i32 {
        self.0
    }
    fn y(&self) -> i32 {
        self.1
    }
    fn isNull(&self) -> bool {
        self.0 == 0 && self.1 == 0
    }
    fn toTuple(&self) -> (i32, i32) {
        (self.0, self.1)
    }
    fn toPointF(&self) -> ScreenPoint {
        ScreenPoint(DVec2::new(self.0.into(), self.1.into()))
    }
    fn manhattanLength(&self) -> i64 {
        i64::from(self.0).abs() + i64::from(self.1).abs()
    }
    fn __add__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok((self.toPointF().0 + coords(other)?).into())
    }
    fn __sub__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok((self.toPointF().0 - coords(other)?).into())
    }
    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        self.toPointF().__eq__(other)
    }
}
#[pyclass(module = "built_core", frozen)]
pub struct ScreenSize(i32, i32);
#[pymethods]
impl ScreenSize {
    #[new]
    fn new(width: i32, height: i32) -> Self {
        Self(width, height)
    }
    fn width(&self) -> i32 {
        self.0
    }
    fn height(&self) -> i32 {
        self.1
    }
}
#[pyclass(module = "built_core", frozen)]
pub struct ScreenRect {
    start: DVec2,
    end: DVec2,
}
#[pymethods]
impl ScreenRect {
    #[new]
    #[pyo3(signature=(a,b,w=None,h=None))]
    fn new(
        a: &Bound<'_, PyAny>,
        b: &Bound<'_, PyAny>,
        w: Option<f64>,
        h: Option<f64>,
    ) -> PyResult<Self> {
        match (w, h) {
            (Some(w), Some(h)) => {
                let start = DVec2::new(a.extract()?, b.extract()?);
                Ok(Self {
                    start,
                    end: start + DVec2::new(w, h),
                })
            }
            (None, None) => Ok(Self {
                start: coords(a)?,
                end: coords(b)?,
            }),
            _ => Err(PyTypeError::new_err(
                "expected two points or x, y, width, height",
            )),
        }
    }
    fn x(&self) -> f64 {
        self.start.x
    }
    fn y(&self) -> f64 {
        self.start.y
    }
    fn width(&self) -> f64 {
        self.end.x - self.start.x
    }
    fn height(&self) -> f64 {
        self.end.y - self.start.y
    }
    fn center(&self) -> ScreenPoint {
        ScreenPoint((self.start + self.end) * 0.5)
    }
    fn contains(&self, p: &Bound<'_, PyAny>) -> PyResult<bool> {
        let p = coords(p)?;
        Ok(p.cmpge(self.start).all() && p.cmple(self.end).all())
    }
    fn normalized(&self) -> Self {
        Self {
            start: self.start.min(self.end),
            end: self.start.max(self.end),
        }
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PixelPoint>()?;
    m.add_class::<ScreenPoint>()?;
    m.add_class::<ScreenRect>()?;
    m.add_class::<ScreenSize>()
}
