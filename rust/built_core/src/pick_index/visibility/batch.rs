// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
type Box3 = (P, P);
struct Ray {
    row: usize,
    d: P,
    limit: f64,
    hidden: bool,
    fixed_done: bool,
}
struct Span<'py> {
    source: Obj<'py>,
    bounds: Option<Box3>,
}
fn ray_columns<'py>(
    py: Python<'py>,
    rays: &[Ray],
    indices: &[usize],
) -> PyResult<(Obj<'py>, Obj<'py>)> {
    let d = matrix3(py, &indices.iter().map(|&i| rays[i].d).collect::<Vec<_>>())?;
    let limits = indices
        .iter()
        .map(|&i| rays[i].limit)
        .collect::<Vec<_>>()
        .into_pyarray(py)
        .into_any();
    Ok((d, limits))
}
fn flags(o: &Obj) -> PyResult<Vec<bool>> {
    Ok(o.extract::<PyReadonlyArray1<bool>>()?.as_array().to_vec())
}
fn mesh_occlusion<'py>(
    set: &Obj<'py>,
    rows: &Obj<'py>,
    o: P,
    rays: &[Ray],
    ask: &[usize],
) -> PyResult<(Vec<bool>, Vec<bool>)> {
    let (d, limits) = ray_columns(set.py(), rays, ask)?;
    let (h, d): (Obj, Obj) = set
        .call_method1("occluded", (rows, triple(o), d, limits))?
        .extract()?;
    Ok((flags(&h)?, flags(&d)?))
}
fn cone(spans: &[Span], rays: &[Ray], origin: DVec3) -> Option<Vec<bool>> {
    if spans.len() <= 64 || rays.len() <= 1 {
        return None;
    }
    let axis = rays
        .iter()
        .filter(|r| !r.fixed_done)
        .map(|r| DVec3::from_array(r.d))
        .sum::<DVec3>()
        .normalize_or_zero();
    let cos = rays
        .iter()
        .filter(|r| !r.fixed_done)
        .map(|r| DVec3::from_array(r.d).dot(axis))
        .fold(1., f64::min);
    if cos <= 0.5 {
        return None;
    }
    let angle = cos.min(1.).acos();
    Some(
        spans
            .iter()
            .map(|s| {
                let Some((lo, hi)) = s.bounds else {
                    return true;
                };
                let a = DVec3::from_array(lo);
                let b = DVec3::from_array(hi);
                if !(a.is_finite() && b.is_finite()) {
                    return true;
                }
                let c = (a + b) * 0.5 - origin;
                let rad = (b - a).length() * 0.5 * (1. + 1e-6) + 1e-6;
                let dist = c.length();
                dist <= rad
                    || (c.dot(axis) / dist).clamp(-1., 1.).acos()
                        - (rad / dist).clamp(0., 1.).asin()
                        <= angle + 1e-6
            })
            .collect(),
    )
}
fn entrance(bounds: Option<Box3>, o: P, r: &Ray, many: bool) -> Option<f64> {
    let Some((lo, hi)) = bounds else {
        return Some(f64::NEG_INFINITY);
    };
    let mut near = f64::NEG_INFINITY;
    let mut far = f64::INFINITY;
    for k in 0..3 {
        let d = r.d[k];
        if !many && d.abs() < 1e-12 {
            if o[k] < lo[k] - 1e-9 || o[k] > hi[k] + 1e-9 {
                return None;
            }
            continue;
        }
        let inv = if many && d.abs() <= 1e-12 {
            1e12
        } else {
            1. / d
        };
        let (a, b) = ((lo[k] - o[k]) * inv, (hi[k] - o[k]) * inv);
        near = near.max(a.min(b));
        far = far.min(a.max(b))
    }
    (far >= near.max(0.) && near < r.limit).then_some(near)
}
fn triangles<'py>(idx: &Obj<'py>, s: &Obj<'py>) -> PyResult<Option<[Obj<'py>; 3]>> {
    let py = idx.py();
    let mut cols;
    if s.len()? == 3 {
        let start = s.get_item(1)?.extract::<isize>()?;
        let n = s.get_item(2)?.extract::<isize>()?;
        if n == 0 {
            return Ok(None);
        }
        let slice = PySlice::new(py, start, start + n, 1);
        let col = |key| idx.getattr(key)?.get_item(&slice);
        cols = [
            col("tri_v0")?,
            col("tri_e1")?,
            col("tri_e2")?,
            col("tri_ent")?,
        ];
    } else {
        if s.get_item(1)?.len()? == 0 {
            return Ok(None);
        }
        cols = [
            s.get_item(1)?,
            s.get_item(2)?,
            s.get_item(3)?,
            s.get_item(4)?.add(s.get_item(5)?)?,
        ];
    }
    let keep = idx.getattr("ent_vis")?.get_item(&cols[3])?;
    if !keep.call_method0("any")?.is_truthy()? {
        return Ok(None);
    }
    if !keep.call_method0("all")?.is_truthy()? {
        for c in &mut cols[..3] {
            *c = c.get_item(&keep)?
        }
    }
    let [a, b, c, _] = cols;
    Ok(Some([a, b, c]))
}
// The pick kernel flags numerically ambiguous boundaries. Resolve those rows
// using the same einsum and BLAS dot-product order as the original viewport.
#[pyfunction]
fn occlusion_block<'py>(
    o: &Obj<'py>,
    d: &Obj<'py>,
    v0: &Obj<'py>,
    e1: &Obj<'py>,
    e2: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let py = o.py();
    let np = py.import("numpy")?;
    if v0.len()? == 0 {
        return np.call_method1("full", (d.len()?, f64::INFINITY));
    }
    let all = PySlice::new(py, 0, isize::MAX, 1);
    let s = o.sub(v0)?;
    let q = np.call_method1("cross", (&s, e1))?;
    let uq = np.call_method1("einsum", ("ij,ij->i", e2, &q))?;
    let p = np.call_method1(
        "cross",
        (
            d.get_item((&all, py.None(), &all))?,
            e2.get_item((py.None(), &all, &all))?,
        ),
    )?;
    let det = np.call_method1("einsum", ("nj,rnj->rn", e1, &p))?;
    let ok = np.call_method1("greater", (np.call_method1("abs", (&det,))?, 1e-6))?;
    let inv = np.call_method1(
        "where",
        (
            &ok,
            np.call_method1("divide", (1., np.call_method1("where", (&ok, det, 1.))?))?,
            0.,
        ),
    )?;
    let u = np.call_method1("einsum", ("nj,rnj->rn", s, p))?.mul(&inv)?;
    let v = d.matmul(q.getattr("T")?)?.mul(&inv)?;
    let t = uq.get_item((py.None(), &all))?.mul(inv)?;
    let bound = crate::pick::BARY_EPS;
    let mut hit = ok;
    for (op, val, limit) in [
        ("greater_equal", &u, -bound),
        ("less_equal", &u, 1. + bound),
        ("greater_equal", &v, -bound),
        ("less_equal", &u.add(&v)?, 1. + bound),
        ("greater", &t, 1e-6),
    ] {
        hit = np.call_method1("logical_and", (hit, np.call_method1(op, (val, limit))?))?;
    }
    np.call_method1("where", (hit, t, f64::INFINITY))?
        .call_method1("min", (1,))
}
fn fixed<'py>(
    v: &Obj<'py>,
    idx: &Obj<'py>,
    o: P,
    rays: &mut [Ray],
    spans: &[Span<'py>],
) -> PyResult<()> {
    if spans.is_empty() || rays.iter().all(|r| r.fixed_done) {
        return Ok(());
    }
    let py = v.py();
    let cone = cone(spans, rays, DVec3::from_array(o));
    let many = spans.len() > 64;
    let mut live = Vec::new();
    for (i, span) in spans.iter().enumerate() {
        if cone.as_ref().is_some_and(|keep| !keep[i]) {
            continue;
        }
        let mut first = f64::INFINITY;
        let mut rows = Vec::new();
        for (j, r) in rays.iter().enumerate() {
            if !r.fixed_done {
                if let Some(t) = entrance(span.bounds, o, r, many) {
                    first = first.min(t);
                    rows.push(j)
                }
            }
        }
        if !rows.is_empty() {
            live.push((first, i, rows))
        }
    }
    live.sort_by(|a, b| a.0.total_cmp(&b.0));
    let native = host(py)?.call_method0("_occlusion_native")?;
    let origin = o.to_vec().into_pyarray(py).into_any();
    for (_, i, mut rows) in live {
        rows.retain(|&r| !rays[r].hidden);
        if rows.is_empty() {
            continue;
        }
        let Some([a, b, c]) = triangles(idx, &spans[i].source)? else {
            continue;
        };
        if !native.is_none() {
            let (d, l) = ray_columns(py, rays, &rows)?;
            let before = vec![f64::INFINITY; rows.len()].into_pyarray(py);
            let (hit, doubt): (PyReadonlyArray1<f64>, PyReadonlyArray1<bool>) = native
                .call_method1("occlusion_span", (triple(o), d, before, l, &a, &b, &c))?
                .extract()?;
            let mut check = Vec::new();
            for ((&j, &t), &uncertain) in rows
                .iter()
                .zip(hit.as_array().iter())
                .zip(doubt.as_array().iter())
            {
                if uncertain {
                    check.push(j)
                } else if t < rays[j].limit {
                    rays[j].hidden = true
                }
            }
            rows = check;
        }
        if !rows.is_empty() {
            let budget = v.getattr("_OCCL_BLOCK")?.extract::<usize>()?;
            let step = (budget / a.len()?.max(1)).max(1);
            for group in rows.chunks(step) {
                let (d, _) = ray_columns(py, rays, group)?;
                let nearest =
                    occlusion_block(&origin, &d, &a, &b, &c)?.extract::<PyReadonlyArray1<f64>>()?;
                for (&j, &t) in group.iter().zip(nearest.as_array().iter()) {
                    if t < rays[j].limit {
                        rays[j].hidden = true
                    }
                }
            }
        }
    }
    Ok(())
}
#[pyfunction]
fn occluded_many_now(v: &Obj, worlds: &Obj) -> PyResult<Vec<bool>> {
    let py = v.py();
    let points = coords(worlds)?;
    let mut out = vec![false; points.len()];
    if points.is_empty() {
        return Ok(out);
    }
    if !cut(v)?.is_none() {
        return points
            .into_iter()
            .map(|p| {
                v.call_method1("_is_occluded", (vec_handle(py, DVec3::from_array(p))?,))?
                    .is_truthy()
            })
            .collect();
    }
    if transparent(v)? {
        return Ok(out);
    }
    let nat = lazy(v)?;
    let idx = index(v, !nat.is_none())?;
    let own = if nat.is_none() {
        attr(&idx, "own_spans")?
    } else {
        none(py)
    };
    let lazy_vis = if nat.is_none() {
        none(py)
    } else {
        nat.get_item(2)?
    };
    let active = !lazy_vis.is_none() && lazy_vis.len()? > 0;
    let tri = attr(&idx, "tri_v0")?;
    if tri.is_none() && !own.is_truthy()? && !active {
        return Ok(out);
    }
    let o = xyz(&v.getattr("camera")?.call_method0("eye")?)?.to_array();
    let mut rays = Vec::new();
    for (row, p) in points.into_iter().enumerate() {
        if let Some((d, dist)) = crate::occlusion::unit_ray(o, p) {
            rays.push(Ray {
                row,
                d,
                limit: dist - 0.001,
                hidden: false,
                fixed_done: false,
            })
        }
    }
    if rays.is_empty() {
        return Ok(out);
    }
    if active && !tri.is_none() {
        let stat = v.call_method1("_static_occluders", (&idx,))?;
        if !stat.is_none() {
            let all = (0..rays.len()).collect::<Vec<_>>();
            let (h, d) = mesh_occlusion(&stat.get_item(0)?, &stat.get_item(1)?, o, &rays, &all)?;
            for ((r, h), d) in rays.iter_mut().zip(h).zip(d) {
                r.hidden = h;
                r.fixed_done = h || !d
            }
        }
    }
    let mut spans = Vec::new();
    if !tri.is_none() && rays.iter().any(|r| !r.fixed_done) {
        let f = attr(&idx, "tri_spans")?;
        if f.is_truthy()? {
            for s in f.try_iter()? {
                let s = s?;
                spans.push(Span {
                    bounds: s.get_item(0)?.extract()?,
                    source: s,
                })
            }
        } else {
            spans.push(Span {
                source: (py.None(), 0, tri.len()?).into_pyobject(py)?.into_any(),
                bounds: None,
            })
        }
    }
    if !own.is_none() {
        for s in own.try_iter()? {
            let s = s?;
            spans.push(Span {
                bounds: s.get_item(0)?.extract()?,
                source: s,
            })
        }
    }
    fixed(v, &idx, o, &mut rays, &spans)?;
    if active {
        let ask: Vec<_> = rays
            .iter()
            .enumerate()
            .filter(|(_, r)| !r.hidden)
            .map(|(i, _)| i)
            .collect();
        if !ask.is_empty() {
            let set = nat.get_item(0)?;
            let (h, d) = mesh_occlusion(&set, &lazy_vis, o, &rays, &ask)?;
            let origin = o.to_vec().into_pyarray(py).into_any();
            for ((&i, hidden), doubt) in ask.iter().zip(h).zip(d) {
                if hidden {
                    rays[i].hidden = true
                } else if doubt {
                    let r = &rays[i];
                    let (a, b, c): (Obj, Obj, Obj) = set
                        .call_method1("ray_tris", (&lazy_vis, triple(o), triple(r.d), r.limit))?
                        .extract()?;
                    let t = occlusion_block(&origin, &matrix3(py, &[r.d])?, &a, &b, &c)?
                        .get_item(0)?
                        .extract::<f64>()?;
                    rays[i].hidden = t < rays[i].limit
                }
            }
        }
    }
    for r in rays {
        out[r.row] = r.hidden
    }
    Ok(out)
}
#[pyfunction]
fn document_unit_rays<'py>(eye: &Obj<'py>, pts: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = eye.py();
    if !host(py)?.call_method0("_qv_hypot3")?.is_truthy()? {
        return Ok(none(py));
    }
    let o = xyz(eye)?.to_array();
    let mut rows = Vec::new();
    let mut dirs = Vec::new();
    let mut dist = Vec::new();
    for (i, p) in array_points(pts)?.into_iter().enumerate() {
        if let Some((d, len)) = crate::occlusion::unit_ray(o, p) {
            rows.push(i as i64);
            dirs.push(d);
            dist.push(len)
        }
    }
    Ok((
        rows.into_pyarray(py),
        matrix3(py, &dirs)?,
        dist.into_pyarray(py),
    )
        .into_pyobject(py)?
        .into_any())
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(occlusion_block, m)?)?;
    m.add_function(wrap_pyfunction!(document_unit_rays, m)?)?;
    m.add_function(wrap_pyfunction!(occluded_many_now, m)?)?;
    Ok(())
}
