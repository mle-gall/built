// SPDX-License-Identifier: GPL-3.0-or-later
use super::{init_service, Obj};
use crate::formats::{
    ground::as_raster,
    tiles::{deg2num, tiles_covering},
};
use pyo3::{exceptions::PyValueError, prelude::*, types::PyDict};
#[pyfunction(signature=(image,encoding="terrarium"))]
fn decode_elevation<'p>(py: Python<'p>, image: &Obj<'p>, encoding: &str) -> PyResult<Obj<'p>> {
    as_raster(py, image)?.call_method1("elevation", (encoding,))
}
#[pyfunction]
fn bilinear(field: &Obj<'_>, px: f64, py: f64) -> PyResult<f64> {
    let (h, w) = field.getattr("shape")?.extract::<(usize, usize)>()?;
    if h == 0 || w == 0 {
        return Err(PyValueError::new_err("empty elevation field"));
    }
    let (fx, fy) = (px - px.floor(), py - py.floor());
    let x = (px.floor() as usize).min(w - 1);
    let y = (py.floor() as usize).min(h - 1);
    let at = |x, y| field.get_item((y, x))?.extract::<f64>();
    let right = (x + 1).min(w - 1);
    let below = (y + 1).min(h - 1);
    Ok((at(x, y)? * (1. - fx) + at(right, y)? * fx) * (1. - fy)
        + (at(x, below)? * (1. - fx) + at(right, below)? * fx) * fy)
}
#[pyfunction]
fn pixel_in_tile(lat: f64, lon: f64, zoom: i32) -> (i64, i64, f64, f64) {
    let (x, y) = deg2num(lat, lon, zoom);
    (
        x.floor() as i64,
        y.floor() as i64,
        (x - x.floor()) * 256.,
        (y - y.floor()) * 256.,
    )
}
#[pyfunction(signature=(o,datum,source=None,zoom=13,cache=None,parent=None))]
fn dem_init(
    o: &Obj<'_>,
    datum: &Obj<'_>,
    source: Option<Obj<'_>>,
    zoom: i32,
    cache: Option<&Obj<'_>>,
    parent: Option<&Obj<'_>>,
) -> PyResult<()> {
    init_service(o, parent)?;
    let py = o.py();
    o.setattr("datum", datum)?;
    o.setattr(
        "source",
        match source {
            Some(s) => s,
            None => py.import("georef.dem")?.getattr("AWS_TERRAIN")?,
        },
    )?;
    o.setattr("zoom", zoom)?;
    o.setattr("_fields", PyDict::new(py))?;
    o.setattr("_fetcher", py.None())?;
    o.setattr("_cache", cache)
}
#[pyfunction]
fn dem_fetcher<'p>(o: &Obj<'p>) -> PyResult<Obj<'p>> {
    let f = o.getattr("_fetcher")?;
    if !f.is_none() {
        return Ok(f);
    }
    let py = o.py();
    let m = py.import("georef.tile_fetcher")?;
    let mut cache = o.getattr("_cache")?;
    if !cache.is_truthy()? {
        let dir = m
            .call_method0("default_cache_dir")?
            .getattr("parent")?
            .call_method1("joinpath", ("dem",))?;
        cache = m.call_method1("TileCache", (dir,))?
    }
    let f = m.call_method1("TileFetcher", (cache, o))?;
    o.setattr("_fetcher", &f)?;
    f.getattr("tileReady")?
        .call_method1("connect", (o.getattr("_on_tile")?,))?;
    Ok(f)
}
#[pyfunction]
fn dem_tile(o: &Obj<'_>, source_id: &str, x: i64, y: i64, z: i32, image: &Obj<'_>) -> PyResult<()> {
    let source = o.getattr("source")?;
    if source.getattr("id")?.eq(source_id)? && o.getattr("zoom")?.eq(z)? {
        let e = source.getattr("encoding")?.extract::<String>()?;
        o.getattr("_fields")?
            .set_item((x, y, z), decode_elevation(o.py(), image, &e)?)?;
        o.getattr("changed")?.call_method0("emit")?;
    }
    Ok(())
}
#[pyfunction]
fn dem_field<'p>(o: &Obj<'p>, tx: i64, ty: i64) -> PyResult<Obj<'p>> {
    let z = o.getattr("zoom")?.extract::<i32>()?;
    let fields = o.getattr("_fields")?;
    let key = (tx, ty, z);
    if fields.contains(key)? {
        return fields.get_item(key);
    }
    let source = o.getattr("source")?;
    let img = o
        .call_method0("_ensure_fetcher")?
        .call_method1("request", (&source, tx, ty, z))?;
    if img.is_none() {
        return Ok(img);
    }
    let e = source.getattr("encoding")?.extract::<String>()?;
    let f = decode_elevation(o.py(), &img, &e)?;
    fields.set_item(key, &f)?;
    Ok(f)
}
#[pyfunction]
fn dem_area(o: &Obj<'_>, lat_s: f64, lon_w: f64, lat_n: f64, lon_e: f64) -> PyResult<()> {
    for (x, y) in tiles_covering(lat_s, lon_w, lat_n, lon_e, o.getattr("zoom")?.extract()?) {
        o.call_method1("_field_for", (x, y))?;
    }
    Ok(())
}
#[pyfunction]
fn dem_elevation(o: &Obj<'_>, lat: f64, lon: f64) -> PyResult<Option<f64>> {
    let (x, y, px, py) = pixel_in_tile(lat, lon, o.getattr("zoom")?.extract()?);
    let f = o.call_method1("_field_for", (x, y))?;
    if f.is_none() {
        Ok(None)
    } else {
        bilinear(&f, px, py).map(Some)
    }
}
#[pyfunction]
fn dem_local<'p>(o: &Obj<'p>, point: &Obj<'p>) -> PyResult<Obj<'p>> {
    let (lat, lon, _) = o
        .getattr("datum")?
        .call_method1("local_to_geodetic", (point,))?
        .extract::<(f64, f64, f64)>()?;
    o.call_method1("elevation_at", (lat, lon))
}
#[pyfunction]
fn dem_has(o: &Obj<'_>, lat: f64, lon: f64) -> PyResult<bool> {
    let z = o.getattr("zoom")?.extract()?;
    let (x, y, _, _) = pixel_in_tile(lat, lon, z);
    o.getattr("_fields")?.contains((x, y, z))
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    functions!(m;decode_elevation,bilinear,pixel_in_tile,dem_init,dem_fetcher,dem_tile,dem_field,dem_area,dem_elevation,dem_local,dem_has);
    Ok(())
}
