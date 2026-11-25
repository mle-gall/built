// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
#[pyfunction]
fn pick_face<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let Some((o, d)) = ray(v, x, y)? else {
        return Ok(none(v.py()));
    };
    let hits = v.call_method1("_hover_hits", (ray_index(v, x, y)?, &o, &d))?;
    if let Some(i) = choose(v, &hits, &o, &d, true)? {
        hits.call_method1("ent", (i,))?.get_item(0)
    } else {
        Ok(none(v.py()))
    }
}
fn memo_key<'py>(v: &Obj<'py>, x: f64, y: f64, idx: &Obj<'py>, which: &str) -> PyResult<Obj<'py>> {
    let py = v.py();
    let cam = v.getattr("camera")?;
    let target = xyz(&cam.getattr("target")?)?;
    let round = py.import("builtins")?.getattr("round")?;
    let head = (
        which,
        round.call1((x, 2))?,
        round.call1((y, 2))?,
        id(idx),
        cam.getattr("yaw")?,
        cam.getattr("pitch")?,
        cam.getattr("distance")?,
        cam.getattr("perspective")?,
        target.x,
        target.y,
        target.z,
    )
        .into_pyobject(py)?
        .to_list();
    head.append(v.call_method0("width")?)?;
    head.append(v.call_method0("height")?)?;
    head.append(py.import("views.array_bake")?.call_method0("reading_now")?)?;
    Ok(head.to_tuple().into_any())
}
#[pyfunction]
fn face_any<'py>(v: &Obj<'py>, x: f64, y: f64, which: &str) -> PyResult<Obj<'py>> {
    let py = v.py();
    if which == "infer"
        && (v.getattr("scene")?.getattr("edit_group")?.is_none()
            || call_opt(v, "_rest_is_hidden")?.is_truthy()?)
    {
        return class(py)?.call_method1("_face_any", (v, x, y, "sel"));
    }
    let idx = ray_index(v, x, y)?;
    let nat = call_opt(v, "_lazy_native")?;
    let key = match memo_key(v, x, y, &idx, which) {
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyAttributeError>(py) => none(py),
        k => k?,
    };
    let slot = if which == "sel" {
        "_face_any_memo"
    } else {
        "_face_infer_memo"
    };
    let old = opt(v, slot)?;
    if !key.is_none()
        && !old.is_none()
        && old.get_item(0)?.eq(&key)?
        && old.len()? > 3
        && old.get_item(2)?.is(&idx)
        && old.get_item(3)?.is(&nat)
    {
        return old.get_item(1);
    }
    let found = || -> PyResult<Option<Obj<'py>>> {
        let Some((o, d)) = ray(v, x, y)? else {
            return Ok(None);
        };
        let hits = v.call_method1(
            if which == "sel" {
                "_hover_hits"
            } else {
                "_infer_hits"
            },
            (&idx, &o, &d),
        )?;
        let Some(i) = choose(v, &hits, &o, &d, false)? else {
            return Ok(None);
        };
        let (f, owner): (Obj, Obj) = hits.call_method1("ent", (i,))?.extract()?;
        let place = hits.call_method1("place", (i,))?;
        let mesh = if place.is_none() {
            v.getattr("scene")?.getattr("mesh")?
        } else {
            opt(&place, "mesh")?
        };
        Ok(Some((f, owner, place, mesh).into_pyobject(py)?.into_any()))
    };
    let result = match found()? {
        Some(r) => r,
        None => (py.None(), py.None(), py.None(), py.None())
            .into_pyobject(py)?
            .into_any(),
    };
    if !key.is_none() {
        v.setattr(slot, (key, &result, idx, nat))?
    }
    Ok(result)
}
#[pyfunction]
fn smallest_at_front<'py>(
    v: &Obj<'py>,
    hits: &Obj<'py>,
    o: &Obj<'py>,
    d: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let mut ids = coplanar(v, hits, o, d, false)?;
    if !ids.is_empty() {
        let a = hits.getattr("area")?.extract::<PyReadonlyArray1<f64>>()?;
        let a = a.as_array();
        let min = ids
            .iter()
            .map(|&i| a[i])
            .reduce(|a, b| {
                if a.is_nan() || b.is_nan() {
                    f64::NAN
                } else {
                    a.min(b)
                }
            })
            .unwrap();
        ids.retain(|&i| a[i] <= min * (1. + 1e-6))
    }
    Ok(ids
        .into_iter()
        .map(|i| i as i64)
        .collect::<Vec<_>>()
        .into_pyarray(v.py())
        .into_any())
}
fn billboard<'py>(v: &Obj<'py>, scene: &Obj<'py>, group: &Obj<'py>) -> PyResult<Option<Obj<'py>>> {
    if opt(group, "billboard")?.is_truthy()?
        && scene
            .call_method1("entity_selectable", (group,))?
            .is_truthy()?
    {
        let q = v.call_method1("_billboard_quad", (group,))?;
        if !q.is_none() {
            return Ok(Some(q.get_item(0)?));
        }
    }
    Ok(None)
}
#[pyfunction]
fn pick_group<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let py = v.py();
    let scene = v.getattr("scene")?;
    let edit = scene.getattr("edit_group")?;
    if !edit.is_none() && !opt(&edit, "children")?.is_truthy()? {
        return Ok(none(py));
    }
    if let Some((o, d)) = ray(v, x, y)? {
        let hits = v.call_method1("_hover_hits", (ray_index(v, x, y)?, &o, &d))?;
        let mut loose = f64::INFINITY;
        let mut best = None;
        if !hits.is_none() && hits.getattr("t")?.len()? > 0 {
            let t = hits.getattr("t")?.extract::<PyReadonlyArray1<f64>>()?;
            let l = hits.getattr("loose")?.extract::<PyReadonlyArray1<bool>>()?;
            let t = t.as_array();
            let l = l.as_array();
            let mut grouped = false;
            for (&t, &l) in t.iter().zip(l.iter()) {
                if l {
                    loose = loose.min(t)
                } else {
                    grouped |= t.is_finite()
                }
            }
            if grouped {
                let ids = class(py)?
                    .call_method1("_smallest_at_front", (v, &hits, &o, &d))?
                    .extract::<Vec<usize>>()?;
                if let Some(&i) = ids.first() {
                    best = Some((
                        ids.iter().map(|&i| t[i]).fold(f64::INFINITY, f64::min),
                        if ids.iter().any(|&i| l[i]) {
                            none(py)
                        } else {
                            hits.call_method1("owner", (i,))?
                        },
                    ))
                }
            }
        }
        let mut board = false;
        for g in v.call_method0("_context_placements")?.try_iter()? {
            let g = g?;
            if let Some(q) = billboard(v, &scene, &g)? {
                let kernel = host(py)?.getattr("_ray_triangle")?;
                for [a, b, c] in [[0, 1, 2], [0, 2, 3]] {
                    if let Some(t) = kernel
                        .call1((&o, &d, q.get_item(a)?, q.get_item(b)?, q.get_item(c)?))?
                        .extract::<Option<f64>>()?
                    {
                        if best.as_ref().is_none_or(|(b, _)| t < *b) {
                            best = Some((t, v.call_method1("_owner_of", (&g,))?));
                            board = true
                        }
                    }
                }
            }
        }
        if let Some((t, g)) = best {
            if board
                && loose.is_finite()
                && loose
                    <= t + v
                        .call_method1("_coplanar_eps", (o, d, t))?
                        .extract::<f64>()?
            {
                return Ok(none(py));
            }
            return Ok(g);
        }
    }
    let nat = call_opt(v, "_lazy_native")?;
    let (mut best, mut group) = if nat.is_none() {
        let idx = index(v, &("px", x, y).into_pyobject(py)?.into_any())?;
        let proj = v.call_method0("_gedge_screen")?;
        let mut threshold = v.getattr("pick_threshold_px")?.extract::<f64>()?;
        let mut group = none(py);
        if !proj.is_none() && proj.get_item(4)?.call_method0("any")?.is_truthy()? {
            let grid = py.import("built_core")?;
            let mut args = proj.try_iter()?.collect::<PyResult<Vec<_>>>()?;
            args.extend([
                x.into_pyobject(py)?.into_any(),
                y.into_pyobject(py)?.into_any(),
            ]);
            let dist = grid.call_method1("segment_distances", PyTuple::new(py, args)?)?;
            let sel = opt(&idx, "gedge_sel")?;
            let np = py.import("numpy")?;
            let near = np.call_method1(
                "flatnonzero",
                (np.call_method1("less", (&dist, threshold))?,),
            )?;
            for i in near
                .get_item(dist.get_item(&near)?.call_method0("argsort")?)?
                .extract::<Vec<usize>>()?
            {
                if !sel.is_none() && !sel.get_item(i)?.is_truthy()? {
                    continue;
                }
                let a = DVec3::from_array(idx.getattr("gedge_a")?.get_item(i)?.extract()?);
                let b = DVec3::from_array(idx.getattr("gedge_b")?.get_item(i)?.extract()?);
                let col = |j| proj.get_item(j)?.get_item(i)?.extract::<f64>();
                let ax = col(0)?;
                let ay = col(1)?;
                let dx = col(2)? - ax;
                let dy = col(3)? - ay;
                let l = dx * dx + dy * dy;
                let t = (((x - ax) * dx + (y - ay) * dy) / if l > 1e-12 { l } else { 1. })
                    .clamp(0., 1.);
                let p = vec3(py, a + (b - a) * t)?;
                if !v.call_method1("_is_occluded", (p,))?.is_truthy()? {
                    threshold = dist.get_item(i)?.extract()?;
                    group = idx
                        .getattr("gedge_groups")?
                        .get_item(idx.getattr("gedge_gi")?.get_item(i)?)?;
                    break;
                }
            }
        }
        (threshold, group)
    } else {
        v.call_method1("_pick_group_edge_unbaked", (x, y, nat))?
            .extract()?
    };
    for g in v.call_method0("_placements")?.try_iter()? {
        let g = g?;
        if let Some(q) = billboard(v, &scene, &g)? {
            for i in 0..4 {
                if let (Some(a), Some(b)) = (
                    project(v, &q.get_item(i)?)?,
                    project(v, &q.get_item((i + 1) % 4)?)?,
                ) {
                    let dist = point_segment_distance((x, y), a, b);
                    if dist < best {
                        best = dist;
                        group = v.call_method1("_owner_of", (&g,))?
                    }
                }
            }
        }
    }
    Ok(group)
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(pick_face, m)?)?;
    m.add_function(wrap_pyfunction!(face_any, m)?)?;
    m.add_function(wrap_pyfunction!(smallest_at_front, m)?)?;
    m.add_function(wrap_pyfunction!(pick_group, m)?)?;
    Ok(())
}
