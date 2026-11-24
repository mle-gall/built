// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
#[pyfunction]
fn same_plane(a: &Obj, b: &Obj, tol: f64) -> PyResult<bool> {
    let na = xyz(&a.get_item(1)?)?.normalize_or_zero();
    let nb = xyz(&b.get_item(1)?)?.normalize_or_zero();
    Ok(na.dot(nb).abs() >= 0.001
        && (xyz(&b.get_item(0)?)? - xyz(&a.get_item(0)?)?)
            .dot(na)
            .abs()
            < tol)
}
#[pyfunction]
fn planar_face_plane<'py>(
    v: &Obj<'py>,
    tool: &Obj<'py>,
    cursor: &Obj<'py>,
    captured: &Obj<'py>,
    face_plane: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let no = none(v.py());
    if cursor.is_none()
        || tool.is_none()
        || !v.getattr("axis_lock")?.is_none()
        || !plane_lock_free(tool)?
        || !locking(tool)?
    {
        return Ok(no);
    }
    let plane = if face_plane.is_none() {
        v.call_method1("_hover_face_plane", (cursor,))?
    } else {
        face_plane.clone()
    };
    if plane.is_none()
        || !captured.is_none()
            && v.call_method1("_same_plane", (captured, &plane))?
                .is_truthy()?
    {
        Ok(no)
    } else {
        Ok(plane)
    }
}
#[pyfunction]
fn ray_plane<'py>(
    v: &Obj<'py>,
    o: &Obj<'py>,
    d: &Obj<'py>,
    p: &Obj<'py>,
    n: &Obj<'py>,
    allow_grazing: bool,
) -> PyResult<Obj<'py>> {
    let normal = xyz(n)?;
    let length = normal.length();
    let no = none(v.py());
    if length < 1e-9 {
        return Ok(no);
    }
    let normal = normal / length;
    let dir = xyz(d)?;
    let denom = normal.dot(dir);
    if denom.abs() < 1e-12
        || !allow_grazing
            && denom.abs()
                < v.getattr("GRAZING_PLANE_DEG")?
                    .extract::<f64>()?
                    .to_radians()
                    .sin()
    {
        return Ok(no);
    }
    let origin = xyz(o)?;
    let t = normal.dot(xyz(p)? - origin) / denom;
    if t < 0. {
        Ok(no)
    } else {
        vec3(v.py(), origin + dir * t)
    }
}
#[pyfunction]
fn world_from_pixel<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let py = v.py();
    let no = none(py);
    let (o, d): (Obj, Obj) = v.call_method1("_pixel_to_ray", (x, y))?.extract()?;
    if o.is_none() || d.is_none() {
        return Ok(no);
    }
    let kw = PyDict::new(py);
    kw.set_item("cursor", (x, y))?;
    let plane = v.call_method("_current_work_plane", (), Some(&kw))?;
    v.setattr("_last_work_plane", (plane.get_item(0)?, plane.get_item(1)?))?;
    let tool = v.getattr("active_tool")?;
    let mut grazing = false;
    if !tool.is_none()
        && opt(&tool, "work_plane")?.is_none()
        && plane_lock_free(&tool)?
        && locking(&tool)?
    {
        let axes = py.import("core.axes")?;
        let n = xyz(&plane.get_item(1)?)?.normalize_or_zero();
        if !opt(&tool, "start_point")?.is_none()
            || (xyz(&plane.get_item(0)?)? - xyz(&axes.call_method0("origin")?)?).length() < 1e-5
        {
            for name in ["x", "y", "z"] {
                if n.dot(xyz(&axes.call_method1("axis", (name,))?)?).abs() > 0.999 {
                    grazing = true;
                    break;
                }
            }
        }
    }
    let hit = ray_on(v, &o, &d, &plane, grazing)?;
    if !hit.is_none() {
        return Ok(hit);
    }
    let (face, group): (Obj, Obj) = v.call_method1("pick_face_inference", (x, y))?.extract()?;
    if !face.is_none() {
        let plane = plane_of(&face, &group)?;
        if !plane.get_item(1)?.is_none() {
            return ray_on(v, &o, &d, &plane, true);
        }
    }
    Ok(no)
}
#[pyfunction]
fn current_work_plane<'py>(v: &Obj<'py>, cursor: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let t = v.getattr("active_tool")?;
    let own = opt(&t, "drag_plane")?;
    if !own.is_none() {
        let p = own.call1((v,))?;
        if !p.is_none() {
            return Ok(p);
        }
    }
    let captured = opt(&t, "work_plane")?;
    let start = opt(&t, "start_point")?;
    if !captured.is_none() {
        let plane = v.call_method1("_planar_face_plane", (&t, cursor, &captured))?;
        if !plane.is_none() {
            t.setattr("work_plane", &plane)?;
            return Ok(plane);
        }
        if !start.is_none()
            && xyz(&captured.get_item(1)?)?.normalize_or_zero().z.abs() > 0.94
            && opt(&t, "plane_lock")?.is_none()
            && !locking(&t)?
        {
            let p = v.call_method1("_near_horizon_vertical", (&start,))?;
            if !p.is_none() {
                return Ok(p);
            }
        }
        return Ok(captured);
    }
    let planar = locking(&t)?;
    if start.is_none() {
        if planar {
            let locked = t.call_method1(
                "locked_work_plane",
                (py.import("core.axes")?.call_method0("origin")?,),
            )?;
            if !locked.is_none() {
                if !cursor.is_none() {
                    let p = v.call_method1("_locked_plane_through_reference", (&t, cursor))?;
                    if !p.is_none() {
                        return Ok(p);
                    }
                    let p = v.call_method1("_hover_face_plane", (cursor,))?;
                    if !p.is_none() {
                        let hit = v.call_method1("_ray_hit_plane", (cursor, p))?;
                        if !hit.is_none() {
                            return t.call_method1("locked_work_plane", (hit,));
                        }
                    }
                }
                return Ok(locked);
            }
        }
        if !cursor.is_none() && !t.is_none() {
            let f = opt(v, "pick_face_inference")?;
            let f = if f.is_truthy()? {
                f
            } else {
                v.getattr("pick_face_placement")?
            };
            let x = cursor.get_item(0)?;
            let y = cursor.get_item(1)?;
            let (face, g): (Obj, Obj) = f.call1((&x, &y))?.extract()?;
            if !face.is_none() {
                return plane_of(&face, &g);
            }
            let image = v.call_method1("image_plane_at", (x, y))?;
            if !image.is_none() {
                return image.call_method0("plane");
            }
        }
        if planar && !cursor.is_none() {
            let p = v.call_method1("_camera_planar_work_plane", (cursor,))?;
            if !p.is_none() {
                return Ok(p);
            }
        }
        let cam = v.getattr("camera")?;
        let forward = cam.getattr("target")?.sub(cam.call_method0("eye")?)?;
        let (lo, hi): (Obj, Obj) = v.getattr("scene")?.call_method0("bounds")?.extract()?;
        let center = if lo.is_none() {
            vec3(py, DVec3::ZERO)?
        } else {
            lo.add(hi)?.mul(0.5)?
        };
        let first = py
            .import("core.snap")?
            .call_method1("first_point_work_plane", (forward, center))?;
        if !first.is_none() {
            return Ok(first);
        }
        return Ok((vec3(py, DVec3::ZERO)?, vec3(py, DVec3::Z)?)
            .into_pyobject(py)?
            .into_any());
    }
    let mut base = v.call_method1("_start_point_plane", (&t, &start))?;
    let hover = opt(&t, "hover_plane")?;
    if planar && !hover.is_none() {
        base = (copied(&start)?, copied(&hover.get_item(1)?)?)
            .into_pyobject(py)?
            .into_any()
    }
    if !cursor.is_none() && v.getattr("axis_lock")?.is_none() {
        let face = v.call_method1("_hover_face_plane", (cursor,))?;
        if !face.is_none() {
            if planar {
                return Ok(face);
            }
            let hit = v.call_method1("_ray_hit_plane", (cursor, &base))?;
            let mut deg = opt(&t, "magnetic_axis_deg")?;
            if !deg.is_truthy()? {
                deg = v.getattr("inference_angle_deg")?
            }
            if hit.is_none()
                || py
                    .import("core.snap")?
                    .call_method1("_detect_axis_alignment", (start, hit, deg))?
                    .is_none()
            {
                return Ok(face);
            }
        }
    }
    Ok(base)
}
#[pyfunction]
fn ray_hit_plane<'py>(v: &Obj<'py>, cursor: &Obj<'py>, plane: &Obj<'py>) -> PyResult<Obj<'py>> {
    let (o, d): (Obj, Obj) = pixel(v, "_pixel_to_ray", cursor)?.extract()?;
    let no = none(v.py());
    if o.is_none() || d.is_none() {
        return Ok(no);
    }
    let n = xyz(&plane.get_item(1)?)?;
    let dir = xyz(&d)?;
    let denom = n.dot(dir);
    if denom.abs() < 1e-6 {
        return Ok(no);
    }
    let origin = xyz(&o)?;
    let t = n.dot(xyz(&plane.get_item(0)?)? - origin) / denom;
    if t < 0. {
        Ok(no)
    } else {
        vec3(v.py(), origin + dir * t)
    }
}
#[pyfunction]
fn locked_plane_through_reference<'py>(
    v: &Obj<'py>,
    t: &Obj<'py>,
    cursor: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let py = v.py();
    let axes = py.import("core.axes")?;
    let mut refs = v
        .getattr("_encouraged")?
        .try_iter()?
        .collect::<PyResult<Vec<_>>>()?;
    let acquired = v.getattr("_acquired_point")?;
    if !acquired.is_none() {
        refs.push(acquired)
    }
    let [x, y]: [f64; 2] = cursor.extract()?;
    for reference in refs.into_iter().rev() {
        let plane = t.call_method1("locked_work_plane", (&reference,))?;
        if plane.is_none() {
            continue;
        }
        let hit = v.call_method1("_ray_hit_plane", (cursor, &plane))?;
        if hit.is_none() {
            continue;
        }
        let n = xyz(&plane.get_item(1)?)?.normalize_or_zero();
        let p = xyz(&reference)?;
        let delta = xyz(&hit)? - p;
        for axis in ["x", "y", "z"] {
            let a = xyz(&axes.call_method1("axis", (axis,))?)?;
            if a.dot(n).abs() > 0.001 {
                continue;
            }
            let px = v
                .call_method1("_world_to_pixel", (vec3(py, p + a * delta.dot(a))?,))?
                .extract::<Option<[f64; 2]>>()?;
            if let Some(q) = px {
                if (q[0] - x).hypot(q[1] - y) <= v.getattr("snap_threshold_px")?.extract::<f64>()? {
                    return Ok(plane);
                }
            }
        }
    }
    Ok(none(py))
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(same_plane, m)?)?;
    m.add_function(wrap_pyfunction!(planar_face_plane, m)?)?;
    m.add_function(wrap_pyfunction!(ray_plane, m)?)?;
    m.add_function(wrap_pyfunction!(world_from_pixel, m)?)?;
    m.add_function(wrap_pyfunction!(current_work_plane, m)?)?;
    m.add_function(wrap_pyfunction!(ray_hit_plane, m)?)?;
    m.add_function(wrap_pyfunction!(locked_plane_through_reference, m)?)?;
    Ok(())
}
