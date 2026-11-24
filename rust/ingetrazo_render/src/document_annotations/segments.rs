// SPDX-License-Identifier: GPL-3.0-or-later
//! Project and clip annotation segments at the display surface.
use super::*;
use glam::{DMat4, DVec4};

#[pyfunction]
fn segment_to_pixels<'py>(
    mvp: &Obj<'py>,
    p0: &Obj<'py>,
    p1: &Obj<'py>,
    near: f64,
    width: f64,
    height: f64,
) -> PyResult<Option<Vec<Point>>> {
    let m = DMat4::from_cols_array(&mvp.call_method0("data")?.extract()?);
    let mut a = m * xyz(p0)?.extend(1.);
    let mut b = m * xyz(p1)?.extend(1.);
    if a.w < near && b.w < near {
        return Ok(None);
    }
    let cut = |p: DVec4, q: DVec4| {
        let mut c = p + (q - p) * ((near - p.w) / (q.w - p.w));
        c.w = near;
        c
    };
    if a.w < near {
        a = cut(a, b);
    } else if b.w < near {
        b = cut(b, a);
    }
    let screen = |p: DVec4| {
        (
            (p.x / p.w * 0.5 + 0.5) * width,
            (1. - (p.y / p.w * 0.5 + 0.5)) * height,
        )
    };
    let line: Option<(Point, Point)> = mvp
        .py()
        .import("ingetrazo_core")?
        .call_method1(
            "clip_annotation_line",
            (screen(a), screen(b), width, height, 64.),
        )?
        .extract()?;
    Ok(line.map(|(a, b)| vec![a, b]))
}
#[pyfunction]
fn linework_slack_point<'py>(cam: &Obj<'py>, world: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = cam.py();
    let p = xyz(world)?;
    let perspective = cam
        .getattr_opt("perspective")?
        .map(|v| v.is_truthy())
        .transpose()?
        .unwrap_or(true);
    let delta = if perspective {
        xyz(&cam.call_method0("eye")?)? - p
    } else {
        -xyz(&cam.call_method0("forward")?)?
    };
    let len = delta.length();
    if len < 1e-12 {
        return vec3(py, p);
    }
    let eye = xyz(&cam.call_method0("eye")?)?;
    let forward = xyz(&cam.call_method0("forward")?)?;
    let h = host(py)?;
    let slack = h
        .getattr("LINEWORK_SLACK_MIN")?
        .extract::<f64>()?
        .max(h.getattr("LINEWORK_SLACK")?.extract::<f64>()? * (p - eye).dot(forward).abs());
    vec3(py, p + delta * (slack.min(0.5 * len) / len))
}
#[pyfunction]
fn faceme_dir<'py>(cam: &Obj<'py>, anchor: &Obj<'py>) -> PyResult<Obj<'py>> {
    let eye = cam.call_method0("eye")?;
    if cam.getattr("perspective")?.is_truthy()? {
        eye.sub(anchor)
    } else {
        eye.sub(cam.getattr("target")?)
    }
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(linework_slack_point, m)?)?;
    m.add_function(wrap_pyfunction!(segment_to_pixels, m)?)?;
    m.add_function(wrap_pyfunction!(faceme_dir, m)?)?;
    Ok(())
}
