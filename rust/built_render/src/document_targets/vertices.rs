// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use std::collections::HashSet;
fn projection<'py>(v: &Obj<'py>, idx: &Obj<'py>) -> PyResult<Obj<'py>> {
    let lp = call_opt(v, "_ledge_screen")?;
    if !lp.is_none() {
        return Ok(lp);
    }
    let a = idx.getattr("edge_a")?;
    if a.is_none() || a.len()? == 0 {
        return Ok(lp);
    }
    let (ax, ay, ok): (Obj, Obj, Obj) = v.call_method1("_project_px", (a,))?.extract()?;
    let (bx, by, other): (Obj, Obj, Obj) = v
        .call_method1("_project_px", (idx.getattr("edge_b")?,))?
        .extract()?;
    Ok((ax, ay, bx, by, ok.bitand(other)?)
        .into_pyobject(v.py())?
        .into_any())
}
fn pixel_dist<'py>(x: &Obj<'py>, y: &Obj<'py>, px: f64, py: f64) -> PyResult<Obj<'py>> {
    x.py()
        .import("numpy")?
        .call_method1("hypot", (x.sub(px)?, y.sub(py)?))
}
fn first_visible_after_test<'py>(
    v: &Obj<'py>,
    points: Vec<([f64; 3], Obj<'py>)>,
    as_rows: bool,
) -> PyResult<Obj<'py>> {
    let py = v.py();
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    for (point, record) in points {
        let key = point.map(|n| if n == 0. { 0 } else { n.to_bits() });
        if point.iter().any(|x| x.is_nan()) || seen.insert(key) {
            unique.push((point, record))
        }
    }
    let object = |i: usize| -> PyResult<Obj<'py>> {
        let (p, record) = &unique[i];
        if record.is_none() {
            vec3(py, DVec3::from_array(*p))
        } else {
            Ok(record.clone())
        }
    };
    if unique.is_empty() {
        return Ok(none(py));
    }

    let mut k = 1;
    let mut size = 8;
    let many = opt(v, "_occluded_many")?;
    while k < unique.len() {
        let end = (k + size).min(unique.len());
        let mut objs = Vec::new();
        let hidden = if as_rows {
            let coords = unique[k..end]
                .iter()
                .flat_map(|(p, _)| *p)
                .collect::<Vec<_>>();
            use numpy::PyArrayMethods;
            many.call1((coords.into_pyarray(py).reshape([end - k, 3])?,))?
                .extract::<Vec<bool>>()?
        } else {
            for j in k..end {
                objs.push(object(j)?)
            }
            if many.is_none() {
                objs.iter()
                    .map(|p| v.call_method1("_is_occluded", (p,))?.is_truthy())
                    .collect::<PyResult<Vec<_>>>()?
            } else {
                many.call1((PyList::new(py, &objs)?,))?.extract()?
            }
        };
        if let Some(j) = hidden.iter().position(|h| !*h) {
            return if as_rows {
                object(k + j)
            } else {
                Ok(objs[j].clone())
            };
        }
        k = end;
        size = (size * if as_rows { 4 } else { 2 }).min(1024);
    }
    Ok(none(py))
}
#[pyfunction]
fn pick_vertex<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let py = v.py();
    let nat = call_opt(v, "_lazy_native")?;
    if !nat.is_none() {
        return v.call_method1("_pick_vertex_unbaked", (x, y, nat));
    }
    let idx = index(v, &("px", x, y).into_pyobject(py)?.into_any())?;
    let lp = projection(v, &idx)?;
    let gp = v.call_method0("_gedge_screen")?;
    if lp.is_none() && gp.is_none() {
        return Ok(none(py));
    }
    let n = if lp.is_none() {
        0
    } else {
        idx.getattr("edges")?.len()?
    };
    let m = if gp.is_none() {
        0
    } else {
        idx.getattr("gedge_a")?.len()?
    };
    let key = (id(&lp), id(&gp), n, m).into_pyobject(py)?;
    let old = opt(v, "_vertex_px_cache")?;
    let np = py.import("numpy")?;
    let cache = if !old.is_none() && old.get_item(0)?.eq(&key)? {
        old
    } else {
        let cat = |i, j| -> PyResult<Obj<'py>> {
            let mut cols = Vec::new();
            for p in [&lp, &gp] {
                if !p.is_none() {
                    cols.push(p.get_item(i)?);
                    cols.push(p.get_item(j)?);
                }
            }
            np.call_method1("concatenate", (cols,))
        };
        let c = (key, cat(0, 2)?, cat(1, 3)?, cat(4, 4)?, lp, gp)
            .into_pyobject(py)?
            .into_any();
        v.setattr("_vertex_px_cache", &c)?;
        c
    };
    let ds = np.call_method1(
        "where",
        (
            cache.get_item(3)?,
            pixel_dist(&cache.get_item(1)?, &cache.get_item(2)?, x, y)?,
            f64::INFINITY,
        ),
    )?;
    let near = np.call_method1(
        "flatnonzero",
        (np.call_method1("less", (&ds, v.getattr("pick_threshold_px")?))?,),
    )?;
    let order = near
        .get_item(ds.get_item(&near)?.call_method0("argsort")?)?
        .extract::<Vec<usize>>()?;
    let snap = if n == 0 {
        none(py)
    } else {
        opt(&idx, "edge_snap")?
    };
    let mut points = Vec::new();
    for i in order {
        let (p, obj) = if i < 2 * n {
            if !snap.is_none() && !snap.get_item(i % n)?.is_truthy()? {
                continue;
            }
            let e = idx.getattr("edges")?.get_item(i % n)?;
            let w = e.getattr(if i < n { "a" } else { "b" })?;
            (xyz(&w)?.to_array(), w)
        } else {
            let j = i - 2 * n;
            let arr = idx.getattr(if j < m { "gedge_a" } else { "gedge_b" })?;
            (arr.get_item(j % m)?.extract::<[f64; 3]>()?, none(py))
        };
        if points.is_empty() {
            let w = if obj.is_none() {
                vec3(py, DVec3::from_array(p))?
            } else {
                obj.clone()
            };
            if !v.call_method1("_is_occluded", (&w,))?.is_truthy()? {
                return Ok(w);
            }
        }
        points.push((p, obj));
    }
    first_visible_after_test(v, points, false)
}
#[pyfunction]
fn pick_vertex_unbaked<'py>(v: &Obj<'py>, x: f64, y: f64, nat: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let np = py.import("numpy")?;
    let idx = index(v, &"skip".into_pyobject(py)?.into_any())?;
    let threshold = v.getattr("pick_threshold_px")?.extract::<f64>()?;
    let mut parts = Vec::new();
    let lp = projection(v, &idx)?;
    if !lp.is_none() {
        let mask = opt(&idx, "edge_snap")?;
        let mut ok = lp.get_item(4)?;
        if !mask.is_none() {
            ok = ok.bitand(mask)?
        }
        for (j, name) in [(0, "edge_a"), (2, "edge_b")] {
            parts.push((
                lp.get_item(j)?,
                lp.get_item(j + 1)?,
                ok.clone(),
                idx.getattr(name)?,
            ));
        }
    }
    let fixed = opt(&idx, "gedge_fixed")?;
    if !fixed.is_none() {
        let n = fixed.get_item(1)?.extract::<isize>()?;
        if n > 0 {
            let gp = v.call_method0("_gedge_screen")?;
            if !gp.is_none() {
                let s = PySlice::new(py, 0, n, 1);
                for (j, name) in [(0, "gedge_a"), (2, "gedge_b")] {
                    parts.push((
                        gp.get_item(j)?.get_item(&s)?,
                        gp.get_item(j + 1)?.get_item(&s)?,
                        gp.get_item(4)?.get_item(&s)?,
                        idx.getattr(name)?.get_item(&s)?,
                    ));
                }
            }
        }
    }
    let depth = v.call_method1("_depth_limits", (x, y, threshold))?;
    let mut ds = Vec::new();
    let mut pts = Vec::new();
    let near = host(py)?.getattr("_px_near")?;
    for (ax, ay, ok, points) in parts {
        let (k, d): (Obj, Obj) = near
            .call1((ax, ay, ok, x, y, threshold, &points, &depth))?
            .extract()?;
        if k.len()? > 0 {
            ds.push(d);
            pts.push(points.get_item(k)?)
        }
    }
    let snap = nat.get_item(1)?;
    if snap.len()? > 0 {
        let mat = np
            .call_method1(
                "ascontiguousarray",
                (v.call_method0("_np_mvp")?.getattr("T")?,),
            )?
            .call_method0("ravel")?
            .call_method0("tolist")?;
        let corners = nat.get_item(0)?.call_method1(
            "corners_near",
            (
                snap,
                mat,
                v.call_method0("width")?,
                v.call_method0("height")?,
                x,
                y,
                threshold * (1. + 1e-9) + 1e-9,
                depth,
            ),
        )?;
        if corners.get_item(0)?.len()? > 0 {
            let d = pixel_dist(&corners.get_item(4)?, &corners.get_item(5)?, x, y)?;
            let k = np.call_method1("flatnonzero", (np.call_method1("less", (&d, threshold))?,))?;
            if k.len()? > 0 {
                ds.push(d.get_item(&k)?);
                pts.push(corners.get_item(3)?.get_item(k)?);
            }
        }
    }
    if ds.is_empty() {
        return Ok(none(py));
    }
    let kw = PyDict::new(py);
    kw.set_item("kind", "stable")?;
    let order = np
        .call_method1("concatenate", (ds,))?
        .call_method("argsort", (), Some(&kw))?;
    let points = np.call_method1("concatenate", (pts,))?.get_item(order)?;
    let first = v
        .call_method1("_first_clear", (&points,))?
        .extract::<Option<isize>>()?;
    if let Some(i) = first {
        return if i < 0 {
            Ok(none(py))
        } else {
            vec3(py, DVec3::from_array(points.get_item(i)?.extract()?))
        };
    }
    let w = vec3(py, DVec3::from_array(points.get_item(0)?.extract()?))?;
    if !v.call_method1("_is_occluded", (&w,))?.is_truthy()? {
        return Ok(w);
    }
    let pts = points
        .call_method0("tolist")?
        .extract::<Vec<[f64; 3]>>()?
        .into_iter()
        .map(|p| (p, none(py)))
        .collect();
    first_visible_after_test(v, pts, true)
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(pick_vertex, m)?)?;
    m.add_function(wrap_pyfunction!(pick_vertex_unbaked, m)?)?;
    Ok(())
}
