// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use glam::{DMat4, DVec3};

#[pyfunction]
fn planar_camera<'py>(v: &Obj<'py>, cursor: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let cam = v.getattr("camera")?;
    let eye = cam.call_method0("eye")?;
    let eye_point = xyz(&eye)?;
    let view = xyz(&cam.getattr("target")?)? - eye_point;
    if view.length_squared() < 1e-18 {
        return Ok(none(py));
    }
    let view = view.normalize();
    let axes = py.import("built_core.axes")?;
    let origin = axes.call_method0("origin")?;
    let raw: Option<(Obj, Obj)> = if cursor.is_none() {
        None
    } else {
        let (o, d): (Obj, Obj) = v
            .call_method1("_pixel_to_ray", cursor.extract::<(f64, f64)>()?)?
            .extract()?;
        (!o.is_none() && !d.is_none()).then_some((o, d))
    };
    let (o, d) = if let Some((o, d)) = &raw {
        (o.clone(), xyz(d)?.normalize_or_zero())
    } else {
        (eye, view)
    };
    let dir = vec3(py, d)?;
    let normals = [
        axes.call_method1("axis", ("x",))?,
        axes.call_method1("axis", ("y",))?,
        axes.call_method1("axis", ("z",))?,
    ];
    let kw = pyo3::types::PyDict::new(py);
    kw.set_item("allow_grazing", true)?;
    let hit = |o: &Obj<'py>, d: &Obj<'py>, n: &Obj<'py>| {
        v.call_method("_ray_plane", (o, d, &origin, n), Some(&kw))
    };
    let perspective = cam.getattr("perspective")?.is_truthy()?;
    let mut depths = [None; 3];
    for (i, n) in normals.iter().enumerate() {
        let point = hit(&o, &dir, n)?;
        if !point.is_none() {
            let off = xyz(&point)? - eye_point;
            let depth = if perspective {
                off.length()
            } else {
                off.dot(view)
            };
            if depth >= 0. {
                depths[i] = Some(depth);
            }
        }
    }
    if let Some(ground) = depths[2] {
        let angle = view
            .dot(xyz(&normals[2])?)
            .abs()
            .min(1.)
            .asin()
            .to_degrees();
        let key = if angle <= number(v, "GROUND_PREFERENCE_ANGLE_DEG")? {
            "GROUND_DEPTH_PREFERENCE_RATIO"
        } else {
            "PERSPECTIVE_GROUND_DEPTH_PREFERENCE_RATIO"
        };
        let ratio = number(v, key)?;
        if [depths[0], depths[1]]
            .into_iter()
            .flatten()
            .reduce(f64::min)
            .is_none_or(|other| ground <= other * ratio)
        {
            return Ok((origin, &normals[2]).into_pyobject(py)?.into_any());
        }
    }
    let mut candidates = [true; 2];
    if let Some((o, dir)) = &raw {
        for (i, c) in candidates.iter_mut().enumerate() {
            *c = !hit(o, dir, &normals[i])?.is_none();
        }
        if candidates == [false, false] {
            candidates = [true, true];
        }
    }
    let ax = d.dot(xyz(&normals[0])?).abs();
    let ay = d.dot(xyz(&normals[1])?).abs();
    let x = candidates[0]
        && (!candidates[1]
            || (ax > ay && (ax - ay).abs() > number(v, "PLANE_ALIGNMENT_TIE_MARGIN")?));
    Ok((origin, &normals[if x { 0 } else { 1 }])
        .into_pyobject(py)?
        .into_any())
}

fn forward(v: &Obj) -> PyResult<DVec3> {
    let cam = v.getattr("camera")?;
    Ok(xyz(&cam.getattr("target")?)? - xyz(&cam.call_method0("eye")?)?)
}
#[pyfunction]
fn horizon_vertical<'py>(v: &Obj<'py>, start: &Obj<'py>) -> PyResult<Obj<'py>> {
    let f = forward(v)?;
    if f.length() < 1e-9 {
        return Ok(none(v.py()));
    }
    let f = f.normalize();
    if f.z.abs() >= number(v, "HORIZON_PITCH_THRESHOLD_DEG")?.to_radians().sin() {
        return Ok(none(v.py()));
    }
    let normal = if f.x.abs() >= f.y.abs() {
        DVec3::X
    } else {
        DVec3::Y
    };
    Ok((start, vec3(v.py(), normal)?)
        .into_pyobject(v.py())?
        .into_any())
}
#[pyfunction]
fn start_plane<'py>(v: &Obj<'py>, tool: &Obj<'py>, start: &Obj<'py>) -> PyResult<Obj<'py>> {
    let f = forward(v)?;
    let mut normal = DVec3::Z;
    if f.length() >= 1e-9 {
        if opt(tool, "prefers_vertical_drag")?.is_truthy()? {
            let f = f.normalize();
            let h = DVec3::new(f.x, f.y, 0.);
            if h.length() >= number(v, "HORIZON_PITCH_THRESHOLD_DEG")?.to_radians().sin() {
                normal = h.normalize_or_zero();
            }
        } else {
            let plane = v.call_method1("_near_horizon_vertical", (start,))?;
            if !plane.is_none() {
                return Ok(plane);
            }
        }
    }
    Ok((start, vec3(v.py(), normal)?)
        .into_pyobject(v.py())?
        .into_any())
}
#[pyfunction]
fn horizon_y(v: &Obj, mvp: &Obj, camera: &Obj) -> PyResult<f64> {
    let camera = if camera.is_truthy()? {
        camera.clone()
    } else {
        v.getattr("camera")?
    };
    let eye = xyz(&camera.call_method0("eye")?)?;
    let f = xyz(&camera.getattr("target")?)? - eye;
    let fallback = if f.z < 0. { 2. } else { -2. };
    let h = DVec3::new(f.x, f.y, 0.);
    if h.length() < 1e-5 {
        return Ok(fallback);
    }
    let matrix: [f64; 16] = mvp.call_method0("data")?.extract()?;
    let clip = DMat4::from_cols_array(&matrix) * (eye + h.normalize() * 1e6).extend(1.);
    Ok(if clip.w.abs() < 1e-9 {
        fallback
    } else {
        clip.y / clip.w
    })
}
pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(planar_camera, m)?)?;
    m.add_function(wrap_pyfunction!(horizon_vertical, m)?)?;
    m.add_function(wrap_pyfunction!(start_plane, m)?)?;
    m.add_function(wrap_pyfunction!(horizon_y, m)?)?;
    Ok(())
}
