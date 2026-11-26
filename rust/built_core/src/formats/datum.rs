// SPDX-License-Identifier: GPL-3.0-or-later
use super::Obj;
use crate::{
    location::{utm_forward, utm_inverse, zone_for_lon},
    math::{xyz, Vec3},
};
use glam::DVec3;
use pyo3::{prelude::*, types::PyDict};
#[pyclass(
    module = "built_core.georef.datum",
    dict,
    subclass,
    eq,
    skip_from_py_object
)]
#[derive(Clone, PartialEq)]
pub(crate) struct SceneDatum {
    #[pyo3(get, set)]
    pub lat: f64,
    #[pyo3(get)]
    placement: Option<(u32, f64, f64)>,
    #[pyo3(get, set)]
    pub lon: f64,
    #[pyo3(get, set)]
    pub alt: f64,
    #[pyo3(get, set)]
    pub north: f64,
    #[pyo3(get, set)]
    pub zone: i32,
    #[pyo3(get, set)]
    pub northern: bool,
    #[pyo3(get, set, name = "_east0")]
    east: f64,
    #[pyo3(get, set, name = "_north0")]
    northing: f64,
    #[pyo3(get, set, name = "_cos_n")]
    c: f64,
    #[pyo3(get, set, name = "_sin_n")]
    s: f64,
}
fn number(value: &Obj<'_>) -> PyResult<f64> {
    value
        .py()
        .get_type::<pyo3::types::PyFloat>()
        .call1((value,))?
        .extract()
}
impl SceneDatum {
    fn transform(&self) -> Option<crate::location::crs::MapTransform> {
        let (code, e, n) = self.placement?;
        crate::location::crs::MapTransform::new(code, e, n, self.north)
    }
}
#[pymethods]
impl SceneDatum {
    #[new]
    #[pyo3(signature=(lat,lon,alt=0.,north=0.,placement=None))]
    fn new(
        #[pyo3(from_py_with=number)] lat: f64,
        #[pyo3(from_py_with=number)] lon: f64,
        #[pyo3(from_py_with=number)] alt: f64,
        #[pyo3(from_py_with=number)] north: f64,
        placement: Option<(u32, f64, f64)>,
    ) -> PyResult<Self> {
        if let Some((code, e, n)) = placement {
            if crate::location::crs::lookup(code)
                .and_then(|c| c.inverse(e, n))
                .is_none()
                || !alt.is_finite()
                || !north.is_finite()
            {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "Invalid site placement",
                ));
            }
        }
        let mut v = Self {
            placement,
            lat,
            lon,
            alt,
            north,
            zone: 0,
            northern: false,
            east: 0.,
            northing: 0.,
            c: 0.,
            s: 0.,
        };
        v.__post_init__();
        Ok(v)
    }
    fn __post_init__(&mut self) {
        self.north = (self.north + 180.).rem_euclid(360.) - 180.;
        if self.north == -180. {
            self.north = 180.
        }
        self.zone = zone_for_lon(self.lon);
        self.northern = self.lat >= 0.;
        (self.east, self.northing) = utm_forward(self.lat, self.lon, self.zone);
        (self.s, self.c) = self.north.to_radians().sin_cos();
    }
    /// Metre-based IFC map axes. Geographic anchors use their local WGS84 UTM zone.
    fn map_conversion(&self) -> Option<(u32, f64, f64, f64, f64, f64)> {
        let t = self.transform()?;
        Some((t.crs.code, t.east, t.north, self.alt, t.cos, t.sin))
    }
    fn grid_to_local_xy(&self, east: f64, north: f64) -> (f64, f64) {
        (
            east * self.c + north * self.s,
            north * self.c - east * self.s,
        )
    }
    fn local_to_grid_xy(&self, x: f64, y: f64) -> (f64, f64) {
        (x * self.c - y * self.s, x * self.s + y * self.c)
    }
    fn north_vector(&self) -> Vec3 {
        Vec3::from(DVec3::new(self.s, self.c, 0.))
    }
    #[getter]
    fn hemisphere(&self) -> &str {
        if self.northern {
            "N"
        } else {
            "S"
        }
    }
    #[pyo3(signature=(lat,lon,alt=None))]
    fn geodetic_to_local(&self, lat: f64, lon: f64, alt: Option<f64>) -> PyResult<Vec3> {
        let (x, y) = if let Some(t) = self.transform() {
            t.local(lat, lon)
                .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("Outside projection"))?
        } else {
            let (e, n) = utm_forward(lat, lon, self.zone);
            self.grid_to_local_xy(e - self.east, n - self.northing)
        };
        Ok(Vec3::from(DVec3::new(
            x,
            y,
            alt.unwrap_or(self.alt) - self.alt,
        )))
    }
    fn local_to_geodetic(&self, point: &Obj<'_>) -> PyResult<(f64, f64, f64)> {
        let p = xyz(point)?;
        let (lat, lon) = if let Some(t) = self.transform() {
            t.geographic(p.x, p.y)
                .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("Outside projection"))?
        } else {
            let (e, n) = self.local_to_grid_xy(p.x, p.y);
            utm_inverse(self.east + e, self.northing + n, self.zone, self.northern)
        };
        Ok((lat, lon, p.z + self.alt))
    }
    #[pyo3(signature=(east,north,alt=0.))]
    fn utm_to_local(&self, east: f64, north: f64, alt: f64) -> PyResult<Vec3> {
        if self.placement.is_some() {
            let (lat, lon) = utm_inverse(east, north, self.zone, self.northern);
            return self.geodetic_to_local(lat, lon, Some(alt));
        }
        let (x, y) = self.grid_to_local_xy(east - self.east, north - self.northing);
        Ok(Vec3::from(DVec3::new(x, y, alt - self.alt)))
    }
    fn local_to_utm(&self, point: &Obj<'_>) -> PyResult<(f64, f64, f64)> {
        if self.placement.is_some() {
            let (lat, lon, z) = self.local_to_geodetic(point)?;
            let (e, n) = utm_forward(lat, lon, self.zone);
            return Ok((e, n, z));
        }
        let p = xyz(point)?;
        let (e, n) = self.local_to_grid_xy(p.x, p.y);
        Ok((self.east + e, self.northing + n, self.alt + p.z))
    }
    fn to_dict<'p>(&self, py: Python<'p>) -> PyResult<Bound<'p, PyDict>> {
        let d = PyDict::new(py);
        for (k, v) in [("lat", self.lat), ("lon", self.lon), ("alt", self.alt)] {
            d.set_item(k, v)?
        }
        if self.north != 0. {
            d.set_item("north", self.north)?
        }
        if let Some(p) = self.placement {
            d.set_item("placement", p)?;
        }
        Ok(d)
    }
    #[classmethod]
    fn from_dict<'p>(cls: &Bound<'p, pyo3::types::PyType>, data: &Obj<'p>) -> PyResult<Obj<'p>> {
        cls.call1((
            data.get_item("lat")?,
            data.get_item("lon")?,
            data.call_method1("get", ("alt", 0.))?,
            data.call_method1("get", ("north", 0.))?,
            data.call_method1("get", ("placement",))?
                .extract::<Option<Bound<PyAny>>>()?
                .map(|v| -> PyResult<_> { Ok((v.get_item(0)?, v.get_item(1)?, v.get_item(2)?)) })
                .transpose()?,
        ))
    }
    fn __getstate__<'p>(slf: &Bound<'p, Self>) -> PyResult<Obj<'p>> {
        let state = slf.getattr("__dict__")?.call_method0("copy")?;
        for k in ["zone", "northern", "_east0", "_north0", "_cos_n", "_sin_n"] {
            state.set_item(k, slf.getattr(k)?)?;
        }
        Ok(state)
    }
    fn __setstate__(slf: &Bound<'_, Self>, state: &Bound<'_, PyDict>) -> PyResult<()> {
        for (k, v) in state.iter() {
            slf.setattr(k.cast::<pyo3::types::PyString>()?, v)?;
        }
        Ok(())
    }
    fn __getnewargs__(&self) -> (f64, f64, f64, f64, Option<(u32, f64, f64)>) {
        (self.lat, self.lon, self.alt, self.north, self.placement)
    }
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<SceneDatum>()
}
