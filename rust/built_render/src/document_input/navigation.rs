// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
#[pyfunction]
fn model_point_at<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let py = v.py();
    let (o, d): (Obj, Obj) = v.call_method1("_pixel_to_ray", (x, y))?.extract()?;
    let seen = match v.getattr_opt("_depth_world_at")? {
        Some(f) => f.call1((x, y))?,
        None => none(py),
    };
    if !seen.is_none() {
        return if seen.get_item(0)?.eq("hit")? {
            seen.get_item(1)
        } else {
            Ok(none(py))
        };
    }
    if o.is_none() || d.is_none() {
        return Ok(none(py));
    }
    let hit = (|| -> PyResult<Obj> {
        let kw = PyDict::new(py);
        kw.set_item("near", doc(v)?.call_method1("_ray_near", (v, x, y))?)?;
        let idx = v.call_method("_pick_index", (), Some(&kw))?;
        let hits = v.call_method1("_hover_hits", (idx, &o, &d))?;
        if !hits.is_none() {
            let ts = hits.getattr("t")?;
            if ts.len()? > 0 {
                let t: f64 = py.import("numpy")?.call_method1("min", (ts,))?.extract()?;
                if t.is_finite() && t > 0. {
                    return o.add(d.mul(t)?);
                }
            }
        }
        Ok(none(py))
    })();
    match hit {
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyException>(py) => Ok(none(py)),
        other => other,
    }
}
#[pyfunction]
fn orbit_pivot_at<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let p = doc(v)?.call_method1("_model_point_at", (v, x, y))?;
    if !p.is_none() {
        return Ok(p);
    }
    let (lo, hi): (Obj, Obj) = v.getattr("scene")?.call_method0("bounds")?.extract()?;
    let h = host(v.py())?;
    if !lo.is_none() {
        let w: f64 = v.call_method0("width")?.extract()?;
        let height: f64 = v.call_method0("height")?.extract()?;
        let (o, d): (Obj, Obj) = v
            .call_method1("_pixel_to_ray", (w * 0.5, height * 0.5))?
            .extract()?;
        if !o.is_none() && !d.is_none() {
            let coords = |p: &Obj| -> PyResult<[f64; 3]> {
                Ok([
                    p.call_method0("x")?.extract()?,
                    p.call_method0("y")?.extract()?,
                    p.call_method0("z")?.extract()?,
                ])
            };
            if let Some([a, b]) = h
                .call_method1(
                    "_ray_aabb_span",
                    (coords(&o)?, coords(&d)?, coords(&lo)?, coords(&hi)?),
                )?
                .extract::<Option<[f64; 2]>>()?
            {
                return o.add(d.mul((a + b) * 0.5)?);
            }
        }
    }
    h.call_method1("QVector3D", (v.getattr("camera")?.getattr("target")?,))
}
#[pyfunction]
fn set_middle_drag_pan_mode(v: &Obj, pan: bool) -> PyResult<()> {
    if v.getattr("_middle_drag_active")?.is_truthy()?
        && v.getattr("nav_mode")?.is_none()
        && !v.getattr("_pan_mode")?.eq(pan)?
    {
        v.setattr("_pan_mode", pan)?;
        let cur = v.call_method1("_pointer_for_tool", (if pan { "pan" } else { "orbit" },))?;
        if !cur.is_none() {
            v.call_method1("setCursor", (cur,))?;
        }
    }
    Ok(())
}
#[pyfunction]
fn set_nav_mode(v: &Obj, mode: &Obj) -> PyResult<()> {
    if mode.is_none() {
        let t = v.getattr("suspended_tool")?;
        if !t.is_none() {
            reset(v, &["suspended_tool"])?;
            t.call_method1("on_deactivate", (v,))?;
        }
    }
    let active = tool(v)?;
    if !active.is_none() {
        if mode.is_none() {
            active.call_method1("on_deactivate", (v,))?;
        } else {
            v.setattr("suspended_tool", &active)?;
            let suspend = opt(&active, "on_suspend")?;
            if suspend.is_callable() {
                suspend.call1((v,))?;
            }
        }
        reset(v, &["active_tool"])?
    }
    reset(
        v,
        &["_hover_entity", "last_snap", "_last_pos", "_orbit_pivot"],
    )?;
    v.setattr("nav_mode", mode)?;
    v.setattr("_pan_mode", false)?;
    v.setattr("_middle_drag_active", false)?;
    v.call_method0(if mode.is_none() {
        "unsetCursor"
    } else {
        "_apply_nav_cursor"
    })?;
    update(v)
}
#[pyfunction]
fn model_diag<'py>(v: &Obj<'py>) -> PyResult<f64> {
    let scene = v.getattr("scene")?;
    let cached = opt(&scene, "_bounds_cache")?;
    if cached.is_none() || !cached.get_item(0)?.eq(scene.getattr("version")?)? {
        if let Some(f) = v.getattr_opt("_pick_lazy_table")? {
            let table = f.call0()?;
            if !table.is_none() {
                let lo = table.get_item(1)?;
                let hi = table.get_item(2)?;
                let np = v.py().import("numpy")?;
                let finite =
                    |a: &Obj<'py>| np.call_method1("isfinite", (a,))?.call_method1("all", (1,));
                let ok = finite(&lo)?.bitand(finite(&hi)?)?;
                if ok.call_method0("any")?.is_truthy()? {
                    let d = hi
                        .get_item(&ok)?
                        .call_method1("max", (0,))?
                        .sub(lo.get_item(ok)?.call_method1("min", (0,))?)?;
                    return np.getattr("linalg")?.call_method1("norm", (d,))?.extract();
                }
            }
        }
    }
    let (lo, hi): (Obj, Obj) = scene.call_method0("bounds")?.extract()?;
    if lo.is_none() {
        Ok(0.)
    } else {
        hi.sub(lo)?.call_method0("length")?.extract()
    }
}
#[pyfunction]
fn apply_zoom_value(v: &Obj, typed: &str) -> PyResult<bool> {
    let py = v.py();
    let h = host(py)?;
    let parsed = py.import("re")?.call_method1(
        "fullmatch",
        (
            r"\s*(\d+(?:[.,]\d*)?|[.,]\d+)\s*(mm|deg|°)?\s*",
            typed.to_lowercase(),
        ),
    )?;
    let number = if parsed.is_none() {
        0.
    } else {
        py.import("builtins")?
            .call_method1(
                "float",
                (parsed
                    .call_method1("group", (1,))?
                    .call_method1("replace", (",", "."))?,),
            )?
            .extract::<f64>()?
    };
    if !(number.is_finite() && number > 0.) {
        v.call_method1(
            "flash_status",
            (h.call_method1(
                "tr",
                ("Type a focal length like 35mm, or an angle like 60deg.",),
            )?,),
        )?;
        return Ok(false);
    }
    let camera = v.getattr("camera")?;
    if !camera.getattr("perspective")?.is_truthy()? {
        v.call_method1(
            "flash_status",
            (h.call_method1(
                "tr",
                ("Field of view applies to perspective; switch to Perspective first.",),
            )?,),
        )?;
        return Ok(false);
    }
    let angle = if parsed.call_method1("group", (2,))?.eq("mm")? {
        camera
            .call_method1("fov_for_focal", (number,))?
            .extract::<f64>()?
    } else {
        number
    };
    let (lo, hi): (f64, f64) = py.import("core.camera")?.getattr("FOV_RANGE")?.extract()?;
    camera.setattr("fov_deg", angle.clamp(lo, hi))?;
    update(v)?;
    let kw = PyDict::new(py);
    kw.set_item(
        "mm",
        camera
            .getattr("focal_mm")?
            .call_method1("__format__", (".0f",))?,
    )?;
    kw.set_item(
        "deg",
        camera
            .getattr("fov_deg")?
            .call_method1("__format__", (".1f",))?,
    )?;
    v.call_method1(
        "flash_status",
        (
            h.getattr("tr")?
                .call(("Lens {mm} mm, field of view {deg}°",), Some(&kw))?,
            4000,
        ),
    )?;
    Ok(true)
}
#[pyfunction]
fn array_pending(v: &Obj) -> PyResult<bool> {
    let f = opt(&tool(v)?, "array_pending")?;
    Ok(f.is_callable() && f.call1((v,))?.is_truthy()?)
}
#[pyfunction]
fn set_value_buffer(v: &Obj, text: &Obj) -> PyResult<()> {
    if !v.getattr("_value_buffer")?.eq(text)? {
        v.setattr("_value_buffer", text)?;
        emit(v, "valueBufferChanged", text)?;
        update(v)?
    }
    Ok(())
}
#[pyfunction]
fn face_or_image_in_front<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let f = v.call_method1("pick_face", (x, y))?;
    let image = v.call_method1("image_in_front_of", (&f, x, y))?;
    if image.is_truthy()? {
        Ok(image)
    } else {
        Ok(f)
    }
}
#[pyfunction]
fn refresh_status_hint(v: &Obj) -> PyResult<()> {
    let f = opt(&v.call_method0("window")?, "_update_status_hint")?;
    if !f.is_none() {
        f.call0()?;
    }
    Ok(())
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(model_point_at, m)?)?;
    m.add_function(wrap_pyfunction!(orbit_pivot_at, m)?)?;
    m.add_function(wrap_pyfunction!(set_middle_drag_pan_mode, m)?)?;
    m.add_function(wrap_pyfunction!(set_nav_mode, m)?)?;
    m.add_function(wrap_pyfunction!(model_diag, m)?)?;
    m.add_function(wrap_pyfunction!(apply_zoom_value, m)?)?;
    m.add_function(wrap_pyfunction!(array_pending, m)?)?;
    m.add_function(wrap_pyfunction!(set_value_buffer, m)?)?;
    m.add_function(wrap_pyfunction!(face_or_image_in_front, m)?)?;
    m.add_function(wrap_pyfunction!(refresh_status_hint, m)?)?;
    Ok(())
}
