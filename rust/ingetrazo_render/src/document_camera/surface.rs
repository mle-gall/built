// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::document_host::host;
use pyo3::types::PyDict;

#[pyfunction]
fn init_navigation(v: &Obj) -> PyResult<()> {
    for (field, key) in [
        ("_invert_wheel", "nav/invert_wheel"),
        ("_invert_orbit_y", "nav/invert_orbit_y"),
    ] {
        v.setattr(field, crate::navigation::setting(v.py(), key, "0")? != "0")?;
    }
    v.setattr(
        "_trackpad_slide",
        crate::navigation::load_slide_mode(v.py())?,
    )?;
    for key in [
        "_last_pos",
        "_orbit_pivot",
        "_look_drag",
        "nav_mode",
        "_zoom_box_start",
        "_zoom_box_cur",
    ] {
        v.setattr(key, v.py().None())?;
    }
    for key in ["_pan_mode", "_middle_drag_active", "_zoom_box_active"] {
        v.setattr(key, false)?;
    }
    Ok(())
}
#[pyfunction]
fn nav_cursor(v: &Obj) -> PyResult<()> {
    let mode: Option<String> = v.getattr("nav_mode")?.extract()?;
    let Some(mode) =
        mode.filter(|m| matches!(m.as_str(), "orbit" | "pan" | "zoom" | "zoom_window"))
    else {
        return Ok(());
    };
    let cursor = v.call_method1("_pointer_for_tool", (&mode,))?;
    let cursor = if !cursor.is_none() {
        cursor
    } else {
        host(v.py())?
            .getattr("Qt")?
            .getattr(if mode.starts_with("zoom") {
                "CrossCursor"
            } else {
                "OpenHandCursor"
            })?
    };
    v.call_method1("setCursor", (cursor,))?;
    Ok(())
}

#[pyfunction]
fn look_move(v: &Obj, event: &Obj) -> PyResult<()> {
    let drag = v.getattr("_look_drag")?;
    let last = drag.get_item(1)?;
    let pos = event.call_method0("position")?.call_method0("toPoint")?;
    let (x, y): (i64, i64) = (
        pos.call_method0("x")?.extract()?,
        pos.call_method0("y")?.extract()?,
    );
    let dx = x - last.call_method0("x")?.extract::<i64>()?;
    let dy = y - last.call_method0("y")?.extract::<i64>()?;
    if dx != 0 || dy != 0 {
        v.getattr("active_tool")?
            .call_method1("on_look", (v, dx, dy))?;
    }
    v.setattr("_look_drag", (drag.get_item(0)?, pos))
}

#[pyfunction]
fn camera_drag(v: &Obj, mode: &Obj, delta: &Obj, pan_modifier: bool) -> PyResult<()> {
    let (dx, dy) = xy(delta)?;
    let cam = v.getattr("camera")?;
    if mode.eq("zoom")? {
        cam.call_method1("zoom", (-dy * 0.035,))?;
    } else if mode.eq("pan")? || pan_modifier {
        let depth = (xyz(&v.getattr("_pivot")?)? - xyz(&cam.call_method0("eye")?)?)
            .dot(xyz(&cam.call_method0("forward")?)?);
        let kw = PyDict::new(v.py());
        kw.set_item("depth", depth)?;
        cam.call_method("pan", (dx, dy, v.call_method0("height")?), Some(&kw))?;
    } else {
        let invert = opt(&v.getattr("source_viewport")?, "_invert_orbit_y")?.is_truthy()?;
        cam.call_method1(
            "orbit_about",
            (
                v.getattr("_pivot")?,
                dx,
                if invert { -dy } else { dy },
                v.call_method0("height")?,
            ),
        )?;
    }
    Ok(())
}
#[pyfunction]
fn metal_pivot<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let (seen, point): (String, Obj) = v.call_method1("_depth_world_at", (x, y))?.extract()?;
    if seen == "hit" {
        return Ok(point);
    }
    let (w, h) = size(v)?;
    let (o, d): (Obj, Obj) = v
        .call_method1("_pixel_to_ray", (w / 2., h / 2.))?
        .extract()?;
    let bounds = v.getattr("packed")?.getattr("world_bounds")?;
    let span: Option<(f64, f64)> = v
        .py()
        .import("ingetrazo_core")?
        .call_method1(
            "ray_aabb_span",
            (
                xyz(&o)?.to_array(),
                xyz(&d)?.to_array(),
                bounds.get_item(0)?,
                bounds.get_item(1)?,
            ),
        )?
        .extract()?;
    let p = if let Some((a, b)) = span {
        xyz(&o)? + xyz(&d)? * ((a + b) * 0.5)
    } else {
        xyz(&v.getattr("camera")?.getattr("target")?)?
    };
    vec3(v.py(), p)
}

#[pyfunction]
fn synced_projection(v: &Obj) -> PyResult<()> {
    let mode = v.getattr("camera")?.call_method0("projection_mode")?;
    if !mode.eq(opt(v, "_shown_projection")?)? {
        v.setattr("_shown_projection", &mode)?;
        v.getattr("projectionChanged")?
            .call_method1("emit", (mode,))?;
    }
    Ok(())
}

#[pyfunction]
fn framed_image<'py>(
    v: &Obj<'py>,
    width_px: &Obj<'py>,
    overlays: &Obj<'py>,
    annotations: &Obj<'py>,
    shadows: &Obj<'py>,
    transparent: &Obj<'py>,
    raster: bool,
) -> PyResult<Obj<'py>> {
    let render = v.getattr(if raster {
        "render_raster"
    } else {
        "render_image"
    })?;
    let py = v.py();
    let (w, h) = size(v)?;
    let (w, h) = (w.max(1.), h.max(1.));
    let aspect = opt(&v.getattr("camera")?, "frame_aspect")?;
    let rect: Option<(f64, f64, f64, f64)> = py
        .import("core.camera")?
        .call_method1("frame_rect", (w, h, &aspect))?
        .extract()?;
    let Some((x, y, fw, fh)) = rect.filter(|(_, _, fw, fh)| *fw < w - 0.5 || *fh < h - 0.5) else {
        return render.call1((
            width_px,
            py.None(),
            overlays,
            annotations,
            shadows,
            transparent,
            false,
        ));
    };
    let out_w = width_px.call_method0("__int__")?.extract::<i64>()?.max(64);
    let out_h = ((out_w as f64 / aspect.extract::<f64>()?).round_ties_even() as i64).max(1);
    let full = render.call1((
        ((out_w as f64 * w / fw).round_ties_even() as i64).max(64),
        py.None(),
        overlays,
        annotations,
        shadows,
        transparent,
        false,
    ))?;
    if full.is_none() {
        return Ok(full);
    }
    if raster {
        return full.call_method1(
            "frame_region",
            ((x / w, y / h, fw / w, fh / h), out_w, out_h),
        );
    }
    let (iw, ih) = size(&full)?;
    let (sx, sy) = (iw / w, ih / h);
    let (x, y) = (
        (x * sx).round_ties_even() as i64,
        (y * sy).round_ties_even() as i64,
    );
    let image = full.call_method1(
        "copy",
        (
            x,
            y,
            ((fw * sx).round_ties_even() as i64).min(iw as i64 - x),
            ((fh * sy).round_ties_even() as i64).min(ih as i64 - y),
        ),
    )?;
    if size(&image)? == (out_w as f64, out_h as f64) {
        return Ok(image);
    }
    let qt = host(py)?.getattr("Qt")?;
    image.call_method1(
        "scaled",
        (
            out_w,
            out_h,
            qt.getattr("IgnoreAspectRatio")?,
            qt.getattr("SmoothTransformation")?,
        ),
    )
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(init_navigation, m)?)?;
    m.add_function(wrap_pyfunction!(nav_cursor, m)?)?;
    m.add_function(wrap_pyfunction!(look_move, m)?)?;
    m.add_function(wrap_pyfunction!(camera_drag, m)?)?;
    m.add_function(wrap_pyfunction!(metal_pivot, m)?)?;
    m.add_function(wrap_pyfunction!(synced_projection, m)?)?;
    m.add_function(wrap_pyfunction!(framed_image, m)?)?;
    Ok(())
}
