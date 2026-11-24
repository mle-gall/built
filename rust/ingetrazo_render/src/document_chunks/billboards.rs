// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use numpy::IntoPyArray;
#[pyfunction]
fn figure_token<'py>(v: &Obj<'py>, group: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let map = memo(v, "_figure_tokens")?;
    let mesh = group.getattr("mesh")?;
    let cv = host(py)?.call_method1("_content_ver", (v,))?;
    let xf = opt(group, "xform")?;
    let xft = if xf.is_none() {
        xf
    } else {
        tuple(&xf.call_method0("data")?)?
    };
    let hit = map.call_method1("get", (id(group),))?;
    if !hit.is_none()
        && hit.get_item(0)?.eq(&cv)?
        && hit.get_item(1)?.is(&mesh)
        && hit.get_item(2)?.is(group)
        && hit.get_item(3)?.get_item(1)?.eq(&xft)?
    {
        return hit.get_item(3);
    }
    let serial = opt(&mesh, "_mut_serial")?;
    let geo = if !hit.is_none()
        && hit.get_item(1)?.is(&mesh)
        && !serial.is_none()
        && hit.get_item(4)?.eq(&serial)?
    {
        hit.get_item(5)?
    } else {
        v.call_method1("_mesh_fingerprint", (&mesh,))?
    };
    let token = (
        id(&mesh),
        xft,
        &geo,
        v.call_method1("_faces_paint_fp", (&mesh,))?,
    )
        .into_pyobject(py)?;
    if map.len()? > 4096 {
        map.call_method0("clear")?;
    }
    map.set_item(id(group), (cv, mesh, group, &token, serial, geo))?;
    Ok(token.into_any())
}
#[pyfunction]
fn billboard_mesh<'py>(v: &Obj<'py>, group: &Obj<'py>) -> PyResult<Obj<'py>> {
    let xf = opt(group, "xform")?;
    let mesh = group.getattr("mesh")?;
    if xf.is_none() {
        return Ok(mesh);
    }
    let map = memo(v, "_billboard_world")?;
    let key = v.call_method1("_figure_token", (group,))?;
    let hit = map.call_method1("get", (id(group),))?;
    if !hit.is_none() && hit.get_item(0)?.eq(&key)? {
        return hit.get_item(1);
    }
    let placed = v
        .py()
        .import("core.group")?
        .call_method1("transformed_mesh", (mesh, xf))?;
    if map.len()? > 256 {
        map.call_method0("clear")?;
    }
    map.set_item(id(group), (key, &placed))?;
    Ok(placed)
}
fn bounds(mesh: &Obj) -> PyResult<Option<(DVec3, f64, f64)>> {
    let vs = mesh.getattr("vertices")?;
    if !vs.is_truthy()? {
        return Ok(None);
    }
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = -lo;
    for vertex in vs.try_iter()? {
        let p = xyz(&vertex?.getattr("position")?)?;
        lo = lo.min(p);
        hi = hi.max(p)
    }
    let mid = (hi + lo) * 0.5;
    let size = hi - lo;
    Ok(Some((
        DVec3::new(mid.x, mid.y, lo.z),
        size.x.hypot(size.y),
        size.z,
    )))
}
#[pyfunction]
fn billboard_extent<'py>(v: &Obj<'py>, group: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let key = v.call_method1("_figure_token", (group,))?;
    let map = memo(v, "_billboard_extents")?;
    let hit = map.call_method1("get", (id(group),))?;
    if !hit.is_none() && hit.get_item(0)?.eq(&key)? {
        return hit.get_item(1);
    }
    let ext = if let Some((a, w, h)) = bounds(&v.call_method1("_billboard_mesh", (group,))?)? {
        let mut path = none(py);
        for f in group.getattr("mesh")?.getattr("faces")?.try_iter()? {
            let tex = get(&f?.getattr("attrs")?, "texture")?;
            if tex.is_truthy()? {
                let p = get(&tex, "path")?;
                if p.is_truthy()? {
                    path = p;
                    break;
                }
            }
        }
        (vector(py, a)?, w, h, path).into_pyobject(py)?.into_any()
    } else {
        none(py)
    };
    if map.len()? > 4096 {
        map.call_method0("clear")?;
    }
    map.set_item(id(group), (key, &ext))?;
    Ok(ext)
}
#[pyfunction]
fn billboard_quad<'py>(v: &Obj<'py>, group: &Obj<'py>, face_dir: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let ext = v.call_method1("_billboard_extent", (group,))?;
    if ext.is_none() {
        return Ok(none(py));
    }
    let (mut anchor, w, h, tex): (Obj, f64, f64, Obj) = ext.extract()?;
    let moving = opt(v, "_preview_groups")?;
    if moving.is_truthy()? && moving.contains(id(group))? {
        let m = opt(v, "_preview_matrix")?;
        if !m.is_none() {
            anchor = m.call_method1("map", (anchor,))?
        } else {
            let off = opt(v, "_preview_offset")?;
            if !off.is_none() {
                anchor = anchor.add(off)?
            }
        }
    }
    if w < 1e-9 || h < 1e-9 {
        return Ok(none(py));
    }
    let mut d = if face_dir.is_none() {
        xyz(&v.call_method1("_faceme_dir", (&anchor,))?)?
    } else {
        xyz(face_dir)?
    };
    d.z = 0.;
    if d.length() < 1e-6 {
        d = DVec3::X
    }
    d = d.normalize();
    let r = DVec3::new(-d.y, d.x, 0.) * (w / 2.);
    let a = xyz(&anchor)?;
    let up = DVec3::Z * h;
    let corners = [a - r, a + r, a + r + up, a - r + up]
        .map(|p| vector(py, p))
        .into_iter()
        .collect::<PyResult<Vec<_>>>()?;
    Ok((corners, tex).into_pyobject(py)?.into_any())
}
#[pyfunction]
fn faceme_base<'py>(v: &Obj<'py>, g: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let map = memo(v, "_faceme_cache")?;
    let key = v.call_method1("_figure_token", (g,))?;
    let old = map.call_method1("get", (id(g),))?;
    if !old.is_none() && old.get_item(0)?.eq(&key)? {
        return old.get_item(1);
    }
    let mesh = v.call_method1("_billboard_mesh", (g,))?;
    let faces = mesh.getattr("faces")?;
    v.call_method1("_seed_tris_native", (&faces,))?;
    let by_tex = PyDict::new(py);
    let by_color = PyDict::new(py);
    let mut n0 = None;
    let mut best = 0.;
    for f in faces.try_iter()? {
        let f = f?;
        let area = if f.hasattr("area")? {
            f.call_method0("area")?.extract::<f64>()?
        } else {
            0.
        };
        if n0.is_none() || area > best {
            n0 = Some(xyz(&f.call_method0("normal")?)?);
            best = area
        }
        let attrs = f.getattr("attrs")?;
        let tex = get(&attrs, "texture")?;
        if !tex.is_none() && get(&tex, "path")?.is_truthy()? {
            v.call_method1("_append_textured_face", (&by_tex, f, tex))?;
            continue;
        }
        let col = get(&attrs, "color")?;
        let col = if col.is_truthy()? {
            tuple(&col)?
        } else {
            (0.96, 0.95, 0.925).into_pyobject(py)?.into_any()
        };
        let buf = if let Some(old) = by_color.get_item(&col)? {
            old
        } else {
            let b = py.import("array")?.call_method1("array", ("f",))?;
            by_color.set_item(col, &b)?;
            b
        };
        let mut data = Vec::new();
        for t in v.call_method1("_tris_of", (f,))?.try_iter()? {
            for p in t?.try_iter()? {
                data.extend(xyz(&p?)?.to_array().map(|c| c as f32));
                data.extend([0., 0.])
            }
        }
        buf.call_method1(
            "frombytes",
            (PyBytes::new(py, bytemuck::cast_slice(&data)),),
        )?;
    }
    let mut entry = none(py);
    if let Some(n) = n0 {
        let len = n.x.hypot(n.y);
        if len > 1e-9 && (!by_tex.is_empty() || !by_color.is_empty()) {
            if let Some((a, _, _)) = bounds(&mesh)? {
                let np = py.import("numpy")?;
                let freeze = |d: &Bound<'py, PyDict>| -> PyResult<Bound<'py, PyDict>> {
                    let out = PyDict::new(py);
                    for (k, b) in d.iter() {
                        let packed = b.call_method0("tobytes")?;
                        out.set_item(
                            k,
                            np.call_method1("frombuffer", (packed, "float32"))?
                                .call_method1("reshape", (-1, 5))?,
                        )?
                    }
                    Ok(out)
                };
                entry = (
                    freeze(&by_tex)?,
                    freeze(&by_color)?,
                    a.to_array().to_vec().into_pyarray(py),
                    vec![n.x / len, n.y / len].into_pyarray(py),
                )
                    .into_pyobject(py)?
                    .into_any();
            }
        }
    }
    map.set_item(id(g), (key, &entry))?;
    Ok(entry)
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(figure_token, m)?)?;
    m.add_function(wrap_pyfunction!(billboard_mesh, m)?)?;
    m.add_function(wrap_pyfunction!(billboard_extent, m)?)?;
    m.add_function(wrap_pyfunction!(billboard_quad, m)?)?;
    m.add_function(wrap_pyfunction!(faceme_base, m)?)?;
    Ok(())
}
