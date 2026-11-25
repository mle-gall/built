// SPDX-License-Identifier: GPL-3.0-or-later
//! Cursor anchors shared by the Metal window and compatibility surfaces.
use super::*;
use pyo3::types::{PyDict, PyFloat, PyTuple};

#[derive(Clone, Copy)]
enum FocalPlane {
    Recentre,
    BackgroundZoom,
}

fn focal_hit<'py>(
    camera: &Obj<'py>,
    origin: &Obj<'py>,
    direction: &Obj<'py>,
    usage: FocalPlane,
) -> PyResult<Obj<'py>> {
    let target = xyz(&camera.getattr("target")?)?;
    let origin = xyz(origin)?;
    let direction = xyz(direction)?;
    let view = target - xyz(&camera.call_method0("eye")?)?;
    if matches!(usage, FocalPlane::BackgroundZoom) && view.length() <= 1e-9 {
        return Ok(none(camera.py()));
    }
    let view = view.normalize_or_zero();
    let along = direction.dot(view);
    let crosses = match usage {
        FocalPlane::Recentre => along.abs() >= 1e-6,
        FocalPlane::BackgroundZoom => along.abs() > 1e-6,
    };
    if crosses {
        let distance = (target - origin).dot(view) / along;
        if distance > 0. {
            return vec3(camera.py(), origin + direction * distance);
        }
    }
    Ok(none(camera.py()))
}

#[pyfunction]
fn centre_depth<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let (origin, direction): (Obj, Obj) = v.call_method1("_pixel_to_ray", (x, y))?.extract()?;
    if origin.is_none() || direction.is_none() {
        return Ok(none(v.py()));
    }
    focal_hit(
        &v.getattr("camera")?,
        &origin,
        &direction,
        FocalPlane::Recentre,
    )
}

#[pyfunction]
fn cursor_world<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let (origin, direction): (Obj, Obj) = v.call_method1("_pixel_to_ray", (x, y))?.extract()?;
    if origin.is_none() || direction.is_none() {
        return Ok(none(v.py()));
    }
    let depth = match v.getattr_opt("_depth_world_at")? {
        Some(read) => read.call1((x, y))?,
        None => none(v.py()),
    };
    if !depth.is_none() && depth.get_item(0)?.eq("hit")? {
        return depth.get_item(1);
    }
    if depth.is_none() && call_opt(v, "_pick_lazy_table")?.is_none() {
        let kwargs = PyDict::new(v.py());
        kwargs.set_item("near", ("px", x, y))?;
        let index = v.call_method("_pick_index", (), Some(&kwargs))?;
        if !index.is_none() && index.getattr("entities")?.is_truthy()? {
            let kwargs = PyDict::new(v.py());
            kwargs.set_item("reduce_global", true)?;
            let hit = v.call_method(
                "_ray_hits",
                (&index, &origin, &direction, index.getattr("ent_vis")?),
                Some(&kwargs),
            )?;
            if !hit.is_none() {
                let t: f64 = hit.extract()?;
                if t.is_finite() {
                    return vec3(v.py(), xyz(&origin)? + xyz(&direction)? * t);
                }
            }
        }
    }
    focal_hit(
        &v.getattr("camera")?,
        &origin,
        &direction,
        FocalPlane::BackgroundZoom,
    )
}

#[pyfunction]
fn view_depth(v: &Obj, point: &Obj) -> PyResult<Option<f64>> {
    if point.is_none() {
        return Ok(None);
    }
    let cam = v.getattr("camera")?;
    Ok(Some(
        (xyz(point)? - xyz(&cam.call_method0("eye")?)?).dot(xyz(&cam.call_method0("forward")?)?),
    ))
}

#[pyfunction]
fn recentre(v: &Obj, x: f64, y: f64) -> PyResult<()> {
    let camera = v.getattr("camera")?;
    let point = v.call_method1("_model_point_at", (x, y))?;
    let target = if point.is_none() {
        let target = v.call_method1("_point_at_centre_depth", (x, y))?;
        if target.is_none() {
            return Ok(());
        }
        target
    } else {
        if camera.getattr("perspective")?.is_truthy()? {
            let depth: Option<f64> = v.call_method1("_depth_of", (&point,))?.extract()?;
            let Some(depth) = depth.filter(|d| d.is_finite() && *d > 0.) else {
                return Ok(());
            };
            let min: f64 = camera.call_method1("min_distance", (&point,))?.extract()?;
            camera.setattr("distance", depth.max(min))?;
        }
        vec3(v.py(), xyz(&point)?)?
    };
    camera.setattr("target", target)?;
    v.call_method0("update")?;
    Ok(())
}

#[pyfunction]
fn zoom_box(v: &Obj, start: &Obj, end: &Obj) -> PyResult<()> {
    let (ax, ay) = xy(start)?;
    let (bx, by) = xy(end)?;
    let (w, h) = ((bx - ax).abs(), (by - ay).abs());
    if w < 6. || h < 6. {
        return Ok(());
    }
    let focus = v.call_method1("_world_under_cursor", ((ax + bx) * 0.5, (ay + by) * 0.5))?;
    let cam = v.getattr("camera")?;
    if !focus.is_none() {
        cam.setattr("target", focus)?;
    }
    let (width, height) = size(v)?;
    let scale = (w / width.max(1.)).max(h / height.max(1.)).max(0.04);
    let distance = number(&cam, "distance")?;
    let min: f64 = cam.call_method0("min_distance")?.extract()?;
    let max = number(v.py().import("core.camera")?.as_any(), "MAX_DISTANCE")?;
    cam.setattr("distance", min.max((scale * distance).min(max)))?;
    v.call_method0("update")?;
    Ok(())
}

#[pyfunction]
fn lock_line<'py>(
    v: &Obj<'py>,
    start: &Obj<'py>,
    lock: &Obj<'py>,
    mut x: f64,
    mut y: f64,
) -> PyResult<Obj<'py>> {
    let p = xyz(start)?;
    let axis = xyz(lock)?.normalize_or_zero();
    let distance = v
        .getattr("camera")?
        .getattr_opt("distance")?
        .map(|v| v.extract::<f64>())
        .transpose()?
        .unwrap_or(1.);
    let a: Option<(f64, f64)> = v.call_method1("_world_to_pixel", (start,))?.extract()?;
    let b: Option<(f64, f64)> = v
        .call_method1(
            "_world_to_pixel",
            (vec3(v.py(), p + axis * (distance * 0.25).max(0.001))?,),
        )?
        .extract()?;
    if let (Some((ax, ay)), Some((bx, by))) = (a, b) {
        let (dx, dy) = (bx - ax, by - ay);
        let norm = dx * dx + dy * dy;
        if norm > 1. {
            let s = ((x - ax) * dx + (y - ay) * dy) / norm;
            (x, y) = (ax + dx * s, ay + dy * s);
        }
    }
    let (o, d): (Obj, Obj) = v.call_method1("_pixel_to_ray", (x, y))?.extract()?;
    if o.is_none() || d.is_none() {
        return Ok(start.clone());
    }
    let d = xyz(&d)?;
    let r = p - xyz(&o)?;
    let b = axis.dot(d);
    let denom = 1. - b * b;
    let t = if denom.abs() < 1e-6 {
        -axis.dot(r)
    } else {
        (b * d.dot(r) - axis.dot(r)) / denom
    };
    vec3(v.py(), p + axis * t)
}

// The cache key is public to host callbacks. Preserve Python's decimal
// rounding, including ties, when the camera has moved by less than one unit.
fn pose<'py>(cam: &Obj<'py>) -> PyResult<Bound<'py, PyTuple>> {
    let t = xyz(&cam.getattr("target")?)?;
    let mut coords = [
        number(cam, "yaw")?,
        number(cam, "pitch")?,
        number(cam, "distance")?,
        t.x,
        t.y,
        t.z,
    ];
    for (i, x) in coords.iter_mut().enumerate() {
        let (scale, places) = if i < 2 { (1e9, 9) } else { (1e6, 6) };
        let scaled = *x * scale;
        // Below 2^52, multiplication can change the rounding decision only
        // if it lands exactly on a half. For those ties and large coordinates,
        // let Python round the original value, as in the published cache key.
        *x = if scaled.abs() < (1u64 << 52) as f64 && scaled.fract().abs() != 0.5 {
            scaled.round_ties_even() / scale
        } else {
            PyFloat::new(cam.py(), *x)
                .call_method1("__round__", (places,))?
                .extract()?
        };
    }
    PyTuple::new(cam.py(), coords)
}

#[pyfunction]
fn cursor_zoom(v: &Obj, steps: f64, pos: &Obj) -> PyResult<(bool, bool)> {
    let (x, y) = xy(pos)?;
    let cam = v.getattr("camera")?;
    let scene = v.getattr("scene")?;
    let cached = opt(v, "_zoom_focus")?;
    let mut reused = false;
    let mut reprojected = false;
    let mut focus = none(v.py());
    if !cached.is_none() && scene.getattr("version")?.eq(cached.get_item(3)?)? {
        let old = cached.get_item(4)?;
        if cached.get_item(0)?.eq(pose(&cam)?)?
            && (x - cached.get_item(1)?.extract::<f64>()?).abs() < 24.
            && (y - cached.get_item(2)?.extract::<f64>()?).abs() < 24.
        {
            reused = true;
            focus = old;
        } else if !old.is_none() {
            let p: Option<(f64, f64)> = v.call_method1("_world_to_pixel", (&old,))?.extract()?;
            if p.is_some_and(|(a, b)| (a - x).abs() < 24. && (b - y).abs() < 24.) {
                reprojected = true;
                focus = old;
            }
        }
    }
    if !reused && focus.is_none() {
        focus = v.call_method1("_world_under_cursor", (x, y))?;
    }
    if focus.is_none() {
        cam.call_method1("zoom", (steps,))?;
    } else {
        let kw = PyDict::new(v.py());
        kw.set_item(
            "min_step",
            v.call_method0("_zoom_out_step")?.extract::<f64>()? * steps.abs(),
        )?;
        cam.call_method("zoom_to", (steps, &focus), Some(&kw))?;
    }
    v.setattr(
        "_zoom_focus",
        (pose(&cam)?, x, y, scene.getattr("version")?, focus),
    )?;
    Ok((reprojected, reused))
}

#[pyfunction]
fn retreat_step(v: &Obj) -> PyResult<f64> {
    let diag: f64 = v.call_method0("_model_diag")?.extract()?;
    let floor = number(v, "_ZOOM_OUT_STEP")?;
    if diag <= 0. {
        return Ok(floor);
    }
    let far = number(v.py().import("core.camera")?.as_any(), "MIN_DISTANCE_FAR")?;
    Ok((floor * (number(&v.getattr("camera")?, "distance")? / far).min(1.)).max(0.01 * diag))
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(cursor_zoom, m)?)?;
    m.add_function(wrap_pyfunction!(retreat_step, m)?)?;
    m.add_function(wrap_pyfunction!(centre_depth, m)?)?;
    m.add_function(wrap_pyfunction!(zoom_box, m)?)?;
    m.add_function(wrap_pyfunction!(recentre, m)?)?;
    m.add_function(wrap_pyfunction!(lock_line, m)?)?;
    m.add_function(wrap_pyfunction!(cursor_world, m)?)?;
    m.add_function(wrap_pyfunction!(view_depth, m)?)?;
    Ok(())
}
