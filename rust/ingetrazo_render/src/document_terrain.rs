// SPDX-License-Identifier: GPL-3.0-or-later
//! Restore and rebuild one document's terrain from its capture, imagery and DEM.
use crate::{context_edits::surface_sampler, document_host::*};
use pyo3::{prelude::*, types::PyDict};

pub(super) fn enabled(v: &Obj) -> PyResult<bool> {
    let layer = opt(&v.getattr("scene")?, "tile_layer")?;
    opt(&layer, "terrain3d")?.is_truthy()
}

fn capture_bounds(layer: &Obj) -> PyResult<(f64, f64, f64, f64)> {
    let patches = layer.getattr("patches")?;
    let mut bounds = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    if patches.is_empty()? {
        let r: f64 = layer.getattr("radius_m")?.extract()?;
        return Ok((-r, -r, r, r));
    }
    for patch in patches.try_iter()? {
        let (x, y, hw, hh): (f64, f64, f64, f64) = patch?.extract()?;
        bounds = (
            bounds.0.min(x - hw),
            bounds.1.min(y - hh),
            bounds.2.max(x + hw),
            bounds.3.max(y + hh),
        );
    }
    Ok(bounds)
}

pub(super) fn texture(v: &Obj, terrain: &Obj, layer: &Obj) -> PyResult<()> {
    let image = v
        .py()
        .import("georef.terrain")?
        .call_method1("build_mosaic", (terrain, layer.getattr("images")?))?;
    terrain.setattr("texture_image", image)?;
    v.call_method1("upload_terrain", (terrain,))?;
    Ok(())
}

#[pyfunction]
pub(super) fn rebuild_terrain(v: &Obj) -> PyResult<()> {
    if !enabled(v)? {
        return Ok(());
    }
    let py = v.py();
    let scene = v.getattr("scene")?;
    let datum = scene.getattr("georef")?;
    if datum.is_none() {
        return Ok(());
    }
    let layer = scene.getattr("tile_layer")?;
    let sampler = surface_sampler(v, &datum)?;
    let bbox = capture_bounds(&layer)?;
    let terrain_mod = py.import("georef.terrain")?;
    let area: (f64, f64, f64, f64) = terrain_mod
        .call_method1("_bbox_ll", (&datum, bbox))?
        .extract()?;
    sampler.call_method1("ensure_area", area)?;
    let ground = datum.getattr("alt")?;
    let ground = if ground.is_truthy()? {
        ground
    } else {
        py.import("georef.surface")?
            .call_method1("ground_reference", (&sampler, &datum))?
    };
    if ground.is_none() {
        return Ok(());
    }
    let kwargs = PyDict::new(py);
    kwargs.set_item("zoom", layer.getattr("zoom")?)?;
    let terrain = terrain_mod.call_method(
        "build_terrain",
        (&datum, sampler, ground, bbox),
        Some(&kwargs),
    )?;
    if terrain.is_none() {
        return Ok(());
    }
    let previous = scene.getattr("terrain")?;
    if !previous.is_none() {
        terrain.setattr("visible", previous.getattr("visible")?)?;
    }
    scene.setattr("terrain", &terrain)?;
    texture(v, &terrain, &layer)?;
    if opt(v, "_terrain_fit_layer")?.is(&layer) {
        v.setattr("_terrain_fit_layer", py.None())?;
        let camera = v.getattr("camera")?;
        camera.call_method1("set_view", ("iso",))?;
        let (lo, hi): (Obj, Obj) = terrain.call_method0("bounds")?.extract()?;
        camera.call_method1("fit_to", (lo, hi))?;
    }
    Ok(())
}

/// Only the original, still-open document may start work after the first picture.
#[pyfunction]
fn restore_after_picture(weak_view: &Obj, weak_scene: &Obj) -> PyResult<()> {
    let view = weak_view.call0()?;
    if !view.is_none()
        && view.getattr("scene")?.is(&weak_scene.call0()?)
        && !opt(&opt(&view, "keys_stand_in")?, "_closed")?.is_truthy()?
    {
        restore_terrain(&view)?;
    }
    Ok(())
}

#[pyfunction]
fn defer_document_terrain(host: &Obj, view: &Obj) -> PyResult<()> {
    if enabled(view)? {
        let py = view.py();
        let reference = py.import("weakref")?.getattr("ref")?;
        let callback = py.import("functools")?.call_method1(
            "partial",
            (
                wrap_pyfunction!(restore_after_picture, py)?,
                reference.call1((view,))?,
                reference.call1((view.getattr("scene")?,))?,
            ),
        )?;
        host.call_method1("after_picture", (callback,))?;
    }
    Ok(())
}

/// Request imagery once per capture. DEM and tile completions subsequently
/// update geometry and texture independently without resetting the saved camera.
#[pyfunction]
pub(super) fn restore_terrain(v: &Obj) -> PyResult<()> {
    if !enabled(v)? {
        return Ok(());
    }
    let scene = v.getattr("scene")?;
    let layer = scene.getattr("tile_layer")?;
    let datum = scene.getattr("georef")?;
    if datum.is_none() {
        return Ok(());
    }
    let tiles = layer.call_method1("flat_tiles", (datum,))?;
    v.call_method1(
        "prefetch_tiles",
        (layer.getattr("source")?, tiles, layer.getattr("zoom")?),
    )?;
    rebuild_terrain(v)
}

#[pyfunction]
pub(super) fn set_terrain_enabled(v: &Obj, on: bool) -> PyResult<()> {
    let scene = v.getattr("scene")?;
    let layer = scene.getattr("tile_layer")?;
    if layer.is_none() {
        return Ok(());
    }
    let terrain = scene.getattr("terrain")?;
    if enabled(v)? != on {
        layer.setattr("terrain3d", on)?;
        scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)?;
        v.call_method0("notify_scene_changed")?;
    }
    if !on {
        v.setattr("_terrain_fit_layer", v.py().None())?;
        scene.setattr("terrain", v.py().None())?;
        v.call_method1("upload_terrain", (v.py().None(),))?;
    } else if terrain.is_none() {
        v.setattr("_terrain_fit_layer", &layer)?;
        restore_terrain(v)?;
    } else {
        terrain.setattr("visible", true)?;
        v.call_method0("update")?;
    }
    Ok(())
}

pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(defer_document_terrain, m)?)?;
    m.add_function(wrap_pyfunction!(restore_terrain, m)?)?;
    m.add_function(wrap_pyfunction!(set_terrain_enabled, m)?)?;
    Ok(())
}
