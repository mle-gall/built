// SPDX-License-Identifier: GPL-3.0-or-later
use super::Obj;
use crate::{
    location::{utm_forward, utm_inverse, zone_for_lon},
    math::{xyz, Vec3},
};
use glam::DVec3;
use pyo3::{prelude::*, types::PyDict};
#[pyclass(module = "georef.datum", dict, subclass, eq)]
#[derive(Clone, PartialEq)]
pub(crate) struct SceneDatum {
    #[pyo3(get, set)]
    pub lat: f64,
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
#[pymethods]
impl SceneDatum {
    #[new]
    #[pyo3(signature=(lat,lon,alt=0.,north=0.))]
    fn new(lat: f64, lon: f64, alt: f64, north: f64) -> Self {
        let mut v = Self {
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
        v
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
    fn geodetic_to_local(&self, lat: f64, lon: f64, alt: Option<f64>) -> Vec3 {
        let (e, n) = utm_forward(lat, lon, self.zone);
        self.utm_to_local(e, n, alt.unwrap_or(self.alt))
    }
    fn local_to_geodetic(&self, point: &Obj<'_>) -> PyResult<(f64, f64, f64)> {
        let (e, n, z) = self.local_to_utm(point)?;
        let (lat, lon) = utm_inverse(e, n, self.zone, self.northern);
        Ok((lat, lon, z))
    }
    #[pyo3(signature=(east,north,alt=0.))]
    fn utm_to_local(&self, east: f64, north: f64, alt: f64) -> Vec3 {
        let (x, y) = self.grid_to_local_xy(east - self.east, north - self.northing);
        Vec3::from(DVec3::new(x, y, alt - self.alt))
    }
    fn local_to_utm(&self, point: &Obj<'_>) -> PyResult<(f64, f64, f64)> {
        let p = xyz(point)?;
        let (e, n) = self.local_to_grid_xy(p.x, p.y);
        Ok((e + self.east, n + self.northing, p.z + self.alt))
    }
    fn to_dict<'p>(&self, py: Python<'p>) -> PyResult<Bound<'p, PyDict>> {
        let d = PyDict::new(py);
        for (k, v) in [("lat", self.lat), ("lon", self.lon), ("alt", self.alt)] {
            d.set_item(k, v)?
        }
        if self.north != 0. {
            d.set_item("north", self.north)?
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
        ))
    }
    fn __getnewargs__(&self) -> (f64, f64, f64, f64) {
        (self.lat, self.lon, self.alt, self.north)
    }
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<SceneDatum>()
}
