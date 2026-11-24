// SPDX-License-Identifier: GPL-3.0-or-later
//! Map-tile delivery, sky gradients and sunlight for the document viewport.
use crate::document_host::*;
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};

#[pyfunction]
fn base_map_showing(v: &Obj) -> PyResult<bool> {
    let sc = v.getattr("scene")?;
    Ok(opt(&opt(&sc, "tile_layer")?, "visible")?.is_truthy()? && !opt(&sc, "georef")?.is_none())
}
#[pyfunction]
fn terrain_showing(v: &Obj) -> PyResult<bool> {
    Ok(opt(&opt(&v.getattr("scene")?, "terrain")?, "visible")?.is_truthy()?)
}
#[pyfunction]
fn ensure_tile_fetcher<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let f = v.getattr("_tile_fetcher")?;
    if !f.is_none() {
        return Ok(f);
    }
    let py = v.py();
    let kw = PyDict::new(py);
    kw.set_item("parent", v)?;
    let f = py
        .import("georef.tile_fetcher")?
        .call_method("TileFetcher", (), Some(&kw))?;
    v.setattr("_tile_fetcher", &f)?;
    f.getattr("tileReady")?
        .call_method1("connect", (v.getattr("_on_tile_ready")?,))?;
    Ok(f)
}
#[pyfunction]
fn on_tile_ready(v: &Obj, source_id: &Obj, x: i64, y: i64, z: i64, img: &Obj) -> PyResult<()> {
    let layer = opt(&v.getattr("scene")?, "tile_layer")?;
    if !layer.is_none()
        && layer.getattr("source")?.getattr("id")?.eq(source_id)?
        && layer.getattr("zoom")?.eq(z)?
    {
        layer.getattr("images")?.set_item((x, y, z), img)?;
        v.getattr("tilesChanged")?.call_method0("emit")?;
        v.call_method0("update")?;
    }
    Ok(())
}
#[pyfunction]
fn disk_cached_tile<'py>(
    v: &Obj<'py>,
    source: &Obj<'py>,
    x: i64,
    y: i64,
    z: i64,
) -> PyResult<Obj<'py>> {
    let py = v.py();
    if z < 0 || z > source.getattr("max_zoom")?.extract()? {
        return Ok(none(py));
    }
    let f = v.getattr("_tile_fetcher")?;
    let cache = if !f.is_none() {
        f.getattr("cache")?
    } else {
        let mut c = opt(v, "_tile_disk_cache")?;
        if c.is_none() {
            let tiles = py.import("georef.tiles")?;
            c = tiles.call_method1("TileCache", (tiles.call_method0("default_cache_dir")?,))?;
            v.setattr("_tile_disk_cache", &c)?;
        }
        c
    };
    let data = cache.call_method1("get", (source.getattr("id")?, x, y, z))?;
    if data.is_none() {
        return Ok(data);
    }
    py.import("ingetrazo_render")?
        .getattr("SurveyRaster")?
        .call_method1("decode", (data,))
}
#[pyfunction]
fn schedule_tile_requests(v: &Obj) -> PyResult<()> {
    if v.getattr("_tile_requests_scheduled")?.is_truthy()? {
        return Ok(());
    }
    v.setattr("_tile_requests_scheduled", true)?;
    let h = host(v.py())?;
    let callback = v.getattr("_request_missing_tiles")?;
    if h.getattr("NATIVE")?.is_truthy()? {
        h.call_method1("later", (v, 0, callback))?;
    } else {
        h.getattr("QTimer")?
            .call_method1("singleShot", (0, callback))?;
    }
    Ok(())
}
fn request(
    v: &Obj,
    layer: &Obj,
    source: &Obj,
    tiles: &Obj,
    zoom: &Obj,
    missing: bool,
) -> PyResult<bool> {
    let f = v.call_method0("_ensure_tile_fetcher")?;
    let images = layer.getattr("images")?;
    let mut changed = false;
    for p in tiles.try_iter()? {
        let p = p?;
        let (x, y) = p.extract::<(i64, i64)>()?;
        let key = (x, y, zoom).into_pyobject(v.py())?;
        if missing && images.contains(&key)? {
            continue;
        }
        let img = f.call_method1("request", (source, x, y, zoom))?;
        if !img.is_none() {
            images.set_item(key, img)?;
            changed = true;
        }
    }
    Ok(changed)
}
#[pyfunction]
fn request_missing_tiles(v: &Obj) -> PyResult<()> {
    v.setattr("_tile_requests_scheduled", false)?;
    let sc = v.getattr("scene")?;
    let layer = opt(&sc, "tile_layer")?;
    let datum = opt(&sc, "georef")?;
    if layer.is_none() || datum.is_none() || !opt(&layer, "visible")?.is_truthy()? {
        return Ok(());
    }
    let Ok(tiles) = layer.call_method1("flat_tiles", (datum,)) else {
        return Ok(());
    };
    if request(
        v,
        &layer,
        &layer.getattr("source")?,
        &tiles,
        &layer.getattr("zoom")?,
        true,
    )? {
        v.call_method0("update")?;
    }
    Ok(())
}
#[pyfunction]
fn reset_tiles(v: &Obj) -> PyResult<()> {
    let f = v.getattr("_tile_fetcher")?;
    if !f.is_none() {
        f.call_method0("cancel_all")?;
    }
    let layer = opt(&v.getattr("scene")?, "tile_layer")?;
    if !layer.is_none() {
        layer.getattr("images")?.call_method0("clear")?;
    }
    v.call_method0("update")?;
    Ok(())
}
#[pyfunction]
fn prefetch_tiles(v: &Obj, source: &Obj, tiles: &Obj, zoom: &Obj) -> PyResult<()> {
    let layer = opt(&v.getattr("scene")?, "tile_layer")?;
    if !layer.is_none() {
        request(v, &layer, source, tiles, zoom, false)?;
    }
    Ok(())
}
#[pyfunction]
fn sky_gradient<'py>(
    py: Python<'py>,
    sky: [f64; 3],
    ground: [f64; 3],
    hy: f64,
) -> PyResult<Obj<'py>> {
    let cls = host(py)?.getattr("DocumentInteraction")?;
    let sky_span = cls.getattr("_SKY_GRAD_SPAN")?.extract::<f64>()?;
    let ground_span = cls.getattr("_GROUND_GRAD_SPAN")?.extract::<f64>()?;
    let sky = DVec3::from_array(sky);
    let ground = DVec3::from_array(ground);
    let mix = |a: DVec3, b: DVec3, t: f64| a + (b - a) * t;
    let sh = mix(sky, DVec3::ONE, 0.55);
    let gh = mix(ground, DVec3::ONE, 0.3);
    let s = |y: f64| mix(sh, sky, ((y - hy) / sky_span).clamp(0., 1.));
    let g = |y: f64| mix(gh, ground, ((hy - y) / ground_span).clamp(0., 1.));
    let y = hy.clamp(-1., 1.);
    Ok(PyTuple::new(
        py,
        [
            s(y),
            s(1.),
            g(y),
            g(-1.),
            mix(mix(sh, gh, 0.5), DVec3::ZERO, 0.12),
        ]
        .map(|p| (p.x, p.y, p.z)),
    )?
    .into_any())
}
#[pyfunction]
fn sun_dir_now<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    if py
        .import("os")?
        .getattr("environ")?
        .call_method1("get", ("INGETRAZO_NO_SHADOWS",))?
        .is_truthy()?
        || !v.getattr("plano_style")?.is_none()
    {
        return Ok(none(py));
    }
    py.import("core.sun")?
        .call_method1("scene_sun_direction", (v.getattr("scene")?,))
}
#[pyfunction]
fn shadow_footprint(m: &Obj) -> PyResult<(f64, f64)> {
    let span = |i: i32| -> PyResult<f64> {
        let n = m
            .call_method1("row", (i,))?
            .call_method0("toVector3D")?
            .call_method0("length")?
            .extract::<f64>()?;
        Ok(if n > 1e-12 { 2. / n } else { 1. })
    };
    Ok((span(0)?, span(2)?))
}
#[pyfunction]
fn light_view<'py>(d: [f64; 3], lo: &Obj<'py>, hi: &Obj<'py>) -> PyResult<(Obj<'py>, f64)> {
    let py = lo.py();
    let mut lo = xyz(lo)?;
    lo.z = lo.z.min(0.);
    let hi = xyz(hi)?;
    let c = (lo + hi) * 0.5;
    let r = (hi - lo).max_element().max(2.) * 0.87 * 1.25;
    let eye = c + DVec3::from_array(d) * (2. * r);
    let up = if d[2].abs() < 0.95 {
        DVec3::Z
    } else {
        DVec3::Y
    };
    let view = host(py)?.call_method0("QMatrix4x4")?;
    view.call_method1("lookAt", (vec3(py, eye)?, vec3(py, c)?, vec3(py, up)?))?;
    Ok((view, r))
}
#[pyfunction]
fn light_vp<'py>(d: &Obj<'py>, lo: &Obj<'py>, hi: &Obj<'py>, fit: &Obj<'py>) -> PyResult<Obj<'py>> {
    let h = host(d.py())?;
    let result = h
        .getattr("DocumentInteraction")?
        .call_method1("_light_view", (d, lo, hi))?;
    let r = result.get_item(1)?.extract::<f64>()?;
    let (x, y, half) = if fit.is_none() {
        (0., 0., r)
    } else {
        fit.extract()?
    };
    let proj = h.call_method0("QMatrix4x4")?;
    proj.call_method1(
        "ortho",
        (x - half, x + half, y - half, y + half, r * 0.5, r * 3.5),
    )?;
    proj.mul(result.get_item(0)?)
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(base_map_showing, m)?)?;
    m.add_function(wrap_pyfunction!(terrain_showing, m)?)?;
    m.add_function(wrap_pyfunction!(ensure_tile_fetcher, m)?)?;
    m.add_function(wrap_pyfunction!(on_tile_ready, m)?)?;
    m.add_function(wrap_pyfunction!(disk_cached_tile, m)?)?;
    m.add_function(wrap_pyfunction!(schedule_tile_requests, m)?)?;
    m.add_function(wrap_pyfunction!(request_missing_tiles, m)?)?;
    m.add_function(wrap_pyfunction!(reset_tiles, m)?)?;
    m.add_function(wrap_pyfunction!(prefetch_tiles, m)?)?;
    m.add_function(wrap_pyfunction!(sky_gradient, m)?)?;
    m.add_function(wrap_pyfunction!(sun_dir_now, m)?)?;
    m.add_function(wrap_pyfunction!(shadow_footprint, m)?)?;
    m.add_function(wrap_pyfunction!(light_vp, m)?)?;
    m.add_function(wrap_pyfunction!(light_view, m)?)?;
    Ok(())
}
