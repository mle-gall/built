// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::math::xyz;
use glam::DVec3;
const VIEWS: [(&str, &str); 6] = [
    ("Top (plan)", "top"),
    ("Front", "front"),
    ("Back", "back"),
    ("Left", "left"),
    ("Right", "right"),
    ("Isometric", "iso"),
];
pub(super) fn install(m: &Bound<PyModule>) -> PyResult<()> {
    let py = m.py();
    let core = py.import("ingetrazo_core")?;
    let paper = PyDict::new(py);
    for (key, w, h) in model::PAPER_SIZES_MM {
        paper.set_item(key, (w, h))?;
    }
    m.add("PAPER_SIZES_MM", paper)?;
    m.add("RENDER_DPI", 300)?;
    m.add("NEW_FRAME_STYLE", "style:Architectural")?;
    m.add(
        "COMMON_SCALES",
        PyTuple::new(
            py,
            [
                0.1, 0.2, 0.5, 1., 2., 5., 10., 20., 25., 50., 100., 200., 250., 500., 1000., 2000.,
            ],
        )?,
    )?;
    m.add("STD_VIEWS", PyTuple::new(py, VIEWS)?)?;
    m.add("PT_TO_MM", 25.4 / 72.)?;
    m.add("SCENE_MIME", "application/x-ingetrazo-scene")?;
    for (name, key) in [
        ("readable_deg", "sheet_readable_deg"),
        ("model_height_for_frame", "sheet_model_height"),
        ("ortho_distance_for_height", "sheet_ortho_distance"),
    ] {
        m.add(name, core.getattr(key)?)?;
    }
    for f in [
        wrap_pyfunction!(mm_to_px, m)?,
        wrap_pyfunction!(pen_px, m)?,
        wrap_pyfunction!(format_scale, m)?,
        wrap_pyfunction!(parse_scale, m)?,
        wrap_pyfunction!(snap_mm, m)?,
        wrap_pyfunction!(frame_page_projector, m)?,
        wrap_pyfunction!(fit_distance_for_scene, m)?,
        wrap_pyfunction!(frame_view_name, m)?,
        wrap_pyfunction!(roll_camera, m)?,
        wrap_pyfunction!(apply_frame_camera, m)?,
    ] {
        m.add_function(f)?;
    }
    Ok(())
}
#[pyfunction(signature=(mm,dpi=300))]
pub(super) fn mm_to_px(mm: f64, dpi: i32) -> i64 {
    ((mm / 25.4 * f64::from(dpi)).round_ties_even() as i64).max(1)
}
#[pyfunction(signature=(mm,dpi=300))]
fn pen_px(mm: Option<f64>, dpi: i32) -> i64 {
    mm_to_px(mm.unwrap_or(0.), dpi)
}
#[pyfunction]
fn format_scale(n: &Obj) -> String {
    n.py()
        .get_type::<pyo3::types::PyFloat>()
        .call1((n,))
        .and_then(|v| v.extract())
        .map(model::format_scale)
        .unwrap_or_default()
}
#[pyfunction(signature=(text,default=100.))]
fn parse_scale(text: &Obj, default: f64) -> PyResult<f64> {
    let text = text.str()?.to_string().replace(',', ".");
    let pos = |s: &str| s.trim().parse::<f64>().ok().filter(|n| *n > 0.);
    Ok(if let Some((a, b)) = text.split_once(':') {
        pos(a).zip(pos(b)).map(|(a, b)| b / a)
    } else {
        pos(&text)
    }
    .unwrap_or(default))
}
#[pyfunction(signature=(value,targets,threshold=2.))]
fn snap_mm(value: f64, targets: &Obj, threshold: f64) -> PyResult<f64> {
    let mut best = value;
    let mut min = f64::INFINITY;
    for x in targets.try_iter()? {
        let x = x?.extract::<f64>()?;
        let d = (value - x).abs();
        if d <= threshold && d < min {
            min = d;
            best = x;
        }
    }
    Ok(best)
}
#[pyfunction]
fn frame_page_projector<'p>(frame: &Obj<'p>, camera: &Obj<'p>) -> PyResult<Obj<'p>> {
    let fov = if frame
        .getattr("perspective")
        .is_ok_and(|v| v.is_truthy().unwrap_or(false))
    {
        Some(
            camera
                .getattr("fov_deg")
                .and_then(|v| v.extract::<f64>())
                .unwrap_or(45.),
        )
    } else {
        None
    };
    frame.py().import("ingetrazo_core")?.call_method1(
        "PageProjection",
        (
            num(frame, "w_mm")?,
            num(frame, "h_mm")?,
            num(frame, "scale_n")?,
            fov,
        ),
    )
}
#[pyfunction(signature=(scene,fov_deg,default=30.))]
fn fit_distance_for_scene(scene: &Obj, fov_deg: f64, default: f64) -> PyResult<f64> {
    let Ok((lo, hi)) = scene
        .call_method0("bounds")
        .and_then(|b| b.extract::<(Obj, Obj)>())
    else {
        return Ok(default);
    };
    if lo.is_none() {
        return Ok(default);
    }
    let diag = xyz(&hi)?.distance(xyz(&lo)?);
    Ok(if diag > 0. {
        diag / 2. / (fov_deg.to_radians() / 2.).tan().max(1e-6) * 1.15
    } else {
        default
    })
}
#[pyfunction]
fn frame_view_name(frame: &Obj) -> PyResult<String> {
    let key = string(frame, "view_key")?;
    if let Some(name) = key.strip_prefix("scene:") {
        return Ok(name.into());
    }
    if let Some(name) = key.strip_prefix("std:") {
        return match VIEWS.iter().find(|(_, k)| *k == name) {
            Some((l, _)) => tr(frame.py(), l),
            None => Ok(name.into()),
        };
    }
    tr(frame.py(), "View")
}
fn vector<'p>(py: Python<'p>, v: DVec3) -> PyResult<Obj<'p>> {
    py.import("ingetrazo_core")?
        .call_method1("Vec3", (v.x, v.y, v.z))
}
#[pyfunction]
fn roll_camera(camera: &Obj, deg: f64) -> PyResult<()> {
    if deg == 0. {
        return Ok(());
    }
    let (sp, cp) = num(camera, "pitch")?.sin_cos();
    let (sy, cy) = num(camera, "yaw")?.sin_cos();
    let forward = DVec3::new(-cp * cy, -cp * sy, -sp);
    let up = if camera.hasattr("up_vector")? {
        camera.call_method0("up_vector")?
    } else {
        camera.getattr("up")?
    };
    let right = forward.cross(xyz(&up)?);
    if right.length() < 1e-9 {
        return Ok(());
    }
    let right = right.normalize();
    let (s, c) = deg.to_radians().sin_cos();
    camera.setattr(
        "up",
        vector(camera.py(), right.cross(forward) * c - right * s)?,
    )
}
#[pyfunction(signature=(camera,frame,saved_view=None,scene=None))]
fn apply_frame_camera(
    camera: &Obj,
    frame: &Obj,
    saved_view: Option<&Obj>,
    scene: Option<&Obj>,
) -> PyResult<()> {
    let py = frame.py();
    if let (Some(saved), Some(scene)) = (saved_view, scene) {
        saved.call_method1("apply", (scene, camera))?;
    } else if let Some(key) = string(frame, "view_key")?.strip_prefix("std:") {
        camera.call_method1("set_view", (key,))?;
        if matches!(key, "top" | "bottom") {
            camera.setattr(
                "pitch",
                if key == "top" { 1. } else { -1. } * std::f64::consts::FRAC_PI_2,
            )?;
            camera.setattr("up", vector(py, DVec3::Y)?)?;
        }
        if let Some(scene) = scene {
            let (lo, hi): (Obj, Obj) = scene.call_method0("bounds")?.extract()?;
            if !lo.is_none() {
                camera.setattr("target", lo.add(hi)?.mul(0.5)?)?;
            }
        }
    }
    for k in ["yaw", "pitch"] {
        if let Some(v) = frame
            .getattr(format!("cam_{k}"))?
            .extract::<Option<f64>>()?
        {
            camera.setattr(k, v)?;
            if k == "pitch" && v.abs() < 89.5f64.to_radians() {
                camera.setattr("up", vector(py, DVec3::Z)?)?;
            }
        }
    }
    let target = frame.getattr("cam_target")?;
    if !target.is_none() {
        camera.setattr("target", vector(py, xyz(&target)?)?)?;
    }
    roll_camera(camera, num(frame, "rot_deg")?)?;
    let (w, h): (f64, f64) = frame.call_method0("render_px")?.extract()?;
    camera.setattr("aspect", w / h)?;
    let perspective = flag(frame, "perspective")?;
    let dist = if perspective {
        let param = |key: &str, saved_key: &str| -> PyResult<Option<f64>> {
            let v = frame.getattr(key)?;
            if !v.is_none() {
                return v.extract();
            }
            match saved_view {
                Some(s) => s.getattr(saved_key)?.extract(),
                None => Ok(None),
            }
        };
        if let Some(f) = param("cam_fov", "fov_deg")? {
            camera.setattr("fov_deg", f.clamp(5., 120.))?;
        }
        match param("cam_distance", "distance")? {
            Some(d) => d,
            None => fit_distance_for_scene(
                &scene.cloned().unwrap_or_else(|| py.None().into_bound(py)),
                num(camera, "fov_deg")?,
                30.,
            )?,
        }
        .max(1e-3)
    } else {
        py.import("ingetrazo_core")?
            .call_method1(
                "sheet_ortho_distance",
                (
                    frame.call_method0("model_height_m")?,
                    num(camera, "fov_deg")?,
                ),
            )?
            .extract()?
    };
    camera.setattr("perspective", perspective)?;
    camera.setattr("distance", dist)
}
