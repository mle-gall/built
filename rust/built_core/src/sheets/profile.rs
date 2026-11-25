// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! The ground under a terrain profile item's path, as `ComposerWindow.profile_for` and `georef.profile
//! .sample_profile` worked it out: stations along the path, each point
//! taken to latitude and longitude through the scene's datum and read off
//! the decoded DEM tiles (`georef.dem.DEMSampler`, bilinear, in float32
//! as NumPy computes it), or off the survey when one shows.
//!
//! Fetching and decoding the tiles stays with the georef service: the
//! sampler's decoded fields are read as they are, and the area is asked
//! for first (`ensure_area`), so tiles on disk are there at once and the
//! others arrive later (the sampler's `changed`).

use super::pymath;
use numpy::{PyReadonlyArray2, PyUntypedArrayMethods};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

const TILE_PX: f64 = 256.0;

unsafe extern "C" {
    /// libm's, as CPython's `math.asinh` (Rust's own differs in the last bits).
    #[link_name = "asinh"]
    fn libm_asinh(x: f64) -> f64;
}

fn tr(py: Python<'_>, text: &str, kw: &[(&str, String)]) -> PyResult<String> {
    let d = PyDict::new(py);
    for (k, v) in kw {
        d.set_item(k, v)?;
    }
    py.import("built_core.i18n")?.getattr("tr")?.call((text,), Some(&d))?.extract()
}

/// `georef.profile.polyline_length`.
pub fn length(pts: &[[f64; 2]]) -> f64 {
    let mut total = 0.0;
    for w in pts.windows(2) {
        total += pymath::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1]);
    }
    total
}

/// `georef.profile.point_at_station`.
pub fn point_at(pts: &[[f64; 2]], s: f64) -> [f64; 2] {
    let Some(first) = pts.first() else { return [0.0, 0.0] };
    if s <= 0.0 {
        return *first;
    }
    let mut acc = 0.0;
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let seg = pymath::hypot(b[0] - a[0], b[1] - a[1]);
        if acc + seg >= s {
            let t = if seg > 1e-12 { (s - acc) / seg } else { 0.0 };
            return [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        }
        acc += seg;
    }
    *pts.last().unwrap()
}

/// The scene datum (`georef.datum.Datum`): local metres → (lat, lon).
pub struct Datum {
    east0: f64,
    north0: f64,
    cos_n: f64,
    sin_n: f64,
    zone: i32,
    northern: bool,
}

impl Datum {
    fn of(d: &Bound<'_, PyAny>) -> PyResult<Datum> {
        Ok(Datum {
            east0: d.getattr("_east0")?.extract()?,
            north0: d.getattr("_north0")?.extract()?,
            cos_n: d.getattr("_cos_n")?.extract()?,
            sin_n: d.getattr("_sin_n")?.extract()?,
            zone: d.getattr("zone")?.extract()?,
            northern: d.getattr("northern")?.is_truthy()?,
        })
    }
    pub fn geodetic(&self, x: f64, y: f64) -> (f64, f64) {
        let (c, s) = (self.cos_n, self.sin_n);
        let (de, dn) = (x * c - y * s, x * s + y * c);
        crate::location::utm_inverse(de + self.east0, dn + self.north0, self.zone, self.northern)
    }
}

/// `georef.tiles.deg2num`.
pub fn deg2num(lat: f64, lon: f64, zoom: i32) -> (f64, f64) {
    let lat = lat.clamp(-85.05112878, 85.05112878);
    let lat_rad = lat.to_radians();
    let n = 2f64.powi(zoom);
    let x = (lon + 180.0) / 360.0 * n;
    // SAFETY: a pure libm function.
    let y = (1.0 - unsafe { libm_asinh(lat_rad.tan()) } / std::f64::consts::PI) / 2.0 * n;
    (x, y)
}

/// `georef.dem.bilinear` on a float32 field, in float32 as NumPy 2 does it
/// (a Python float next to a float32 scalar is cast to float32).
pub fn bilinear(field: &numpy::ndarray::ArrayView2<'_, f32>, px: f64, py: f64) -> f64 {
    let (h, w) = (field.nrows() as i64, field.ncols() as i64);
    let x0 = px.floor() as i64;
    let y0 = py.floor() as i64;
    let fx = px - x0 as f64;
    let fy = py - y0 as f64;
    let x0 = x0.clamp(0, w - 1);
    let y0 = y0.clamp(0, h - 1);
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let at = |y: i64, x: i64| field[[y as usize, x as usize]];
    let (gx, ox) = ((1.0 - fx) as f32, fx as f32);
    let top = at(y0, x0) * gx + at(y0, x1) * ox;
    let bot = at(y1, x0) * gx + at(y1, x1) * ox;
    f64::from(top * ((1.0 - fy) as f32) + bot * (fy as f32))
}

/// Where the elevations come from.
enum Ground<'py> {
    /// The DEM sampler's decoded fields at its zoom.
    Dem { fields: Bound<'py, PyDict>, zoom: i32 },
    /// A survey sampler: `elevation_at_local(point)`.
    Survey(Bound<'py, PyAny>),
}

impl Ground<'_> {
    fn at(&self, py: Python<'_>, datum: &Datum, x: f64, y: f64) -> PyResult<Option<f64>> {
        match self {
            Ground::Dem { fields, zoom } => {
                let (lat, lon) = datum.geodetic(x, y);
                let (xf, yf) = deg2num(lat, lon, *zoom);
                let (tx, ty) = (xf.floor(), yf.floor());
                let key = PyTuple::new(py, [tx as i64, ty as i64, i64::from(*zoom)])?;
                let Some(field) = fields.get_item(key)? else { return Ok(None) };
                let field: PyReadonlyArray2<'_, f32> = field.extract()?;
                if field.shape()[0] == 0 || field.shape()[1] == 0 {
                    return Ok(None);
                }
                Ok(Some(bilinear(&field.as_array(), (xf - tx) * TILE_PX, (yf - ty) * TILE_PX)))
            }
            Ground::Survey(s) => {
                let p = py.import("built_core")?.getattr("Vec3")?.call1((x, y, 0.0))?;
                s.call_method1("elevation_at_local", (p,))?.extract()
            }
        }
    }
}

/// `georef.profile.sample_profile`: ``{"samples": [(station, x, y,
/// elevation or None)], "length", "complete"}``.
fn sample<'py>(py: Python<'py>, pts: &[[f64; 2]], spacing: Option<f64>, datum: &Datum,
               ground: &Ground<'_>, sampler: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    let total = length(pts);
    let mut samples = Vec::new();
    let mut complete = true;
    if pts.len() >= 2 && total > 1e-9 {
        let spacing = spacing.unwrap_or_else(|| (total / 256.0).max(1.0));
        let n = ((total / spacing).ceil() as i64).max(1);
        // The DEM over the trace, asked for first (disk tiles decode now).
        let (mut lat_s, mut lon_w, mut lat_n, mut lon_e) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for p in pts {
            let (la, lo) = datum.geodetic(p[0], p[1]);
            lat_s = lat_s.min(la);
            lat_n = lat_n.max(la);
            lon_w = lon_w.min(lo);
            lon_e = lon_e.max(lo);
        }
        sampler.call_method1("ensure_area", (lat_s, lon_w, lat_n, lon_e))?;
        for i in 0..=n {
            let s = total.min(i as f64 * spacing);
            let [x, y] = point_at(pts, s);
            let e = ground.at(py, datum, x, y)?;
            complete &= e.is_some();
            samples.push((s, x, y, e));
        }
    }
    out.set_item("samples", samples)?;
    out.set_item("length", if pts.len() >= 2 && total > 1e-9 { total } else { 0.0 })?;
    out.set_item("complete", complete)?;
    Ok(out)
}

/// ``{item key: (profile or None, path name, message or None)}`` for the
/// terrain profile items of sheet ``comp`` in ``scene``: what
/// ``sheet_page_json`` paints. ``dem`` the DEM sampler (its ``_fields``,
/// ``zoom`` and ``ensure_area``), ``survey`` a survey sampler when the
/// survey shows (it is then read instead).
#[pyfunction]
#[pyo3(signature = (comp, scene, dem, survey=None))]
pub fn sheet_terrain_profiles<'py>(py: Python<'py>, comp: &Bound<'py, PyAny>, scene: &Bound<'py, PyAny>,
                                   dem: &Bound<'py, PyAny>, survey: Option<Bound<'py, PyAny>>)
    -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    let paths: Vec<Bound<'py, PyAny>> = match scene.getattr_opt("geo_paths")? {
        Some(p) if !p.is_none() => p.try_iter()?.collect::<PyResult<_>>()?,
        _ => Vec::new(),
    };
    let datum = scene.getattr_opt("georef")?.filter(|d| !d.is_none());
    let ground = match &survey {
        Some(s) if !s.is_none() => Ground::Survey(s.clone()),
        _ => Ground::Dem { fields: dem.getattr("_fields")?.cast_into::<PyDict>()?, zoom: dem.getattr("zoom")?.extract()? },
    };
    let sampler = match &survey {
        Some(s) if !s.is_none() => s.clone(),
        _ => dem.clone(),
    };
    for item in comp.getattr("perfiles")?.try_iter()? {
        let item = item?;
        let key = item.getattr("_doc_key")?;
        let none = py.None();
        if paths.is_empty() {
            out.set_item(key, (&none, "", tr(py, "Trace a path with the Path tool (T) first.", &[])?))?;
            continue;
        }
        let idx: i64 = item.getattr("path_index")?.extract()?;
        if idx < 0 || idx as usize >= paths.len() {
            out.set_item(key, (&none, "", tr(py, "Path {n} no longer exists.", &[("n", (idx + 1).to_string())])?))?;
            continue;
        }
        let path = &paths[idx as usize];
        let name = match path.getattr("name")?.extract::<Option<String>>()? {
            Some(n) if !n.is_empty() => n,
            _ => tr(py, "Path {n}", &[("n", (idx + 1).to_string())])?,
        };
        let Some(datum) = &datum else {
            out.set_item(key, (&none, name, tr(py, "Set a base map location first (Tray ▸ Base map).", &[])?))?;
            continue;
        };
        let pts: Vec<[f64; 2]> = path.call_method0("profile_points")?.try_iter()?
            .map(|p| { let p = p?; Ok([p.call_method0("x")?.extract()?, p.call_method0("y")?.extract()?]) })
            .collect::<PyResult<_>>()?;
        if pts.len() < 2 {
            out.set_item(key, (&none, name, tr(py, "The path needs two points.", &[])?))?;
            continue;
        }
        let spacing: f64 = item.getattr("spacing_m")?.extract()?;
        let prof = sample(py, &pts, (spacing != 0.0).then_some(spacing), &Datum::of(datum)?, &ground, &sampler)?;
        let samples: Vec<(f64, f64, f64, Option<f64>)> = prof.get_item("samples")?.unwrap().extract()?;
        let complete: bool = prof.get_item("complete")?.unwrap().extract()?;
        let message = if samples.is_empty() || samples.iter().all(|s| s.3.is_none()) {
            Some(tr(py, "Loading terrain…", &[])?)
        } else if !complete {
            Some(tr(py, "(loading DEM…)", &[])?)
        } else {
            None
        };
        out.set_item(key, (prof, name, message))?;
    }
    Ok(out)
}
