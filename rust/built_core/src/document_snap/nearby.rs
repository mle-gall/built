// SPDX-License-Identifier: GPL-3.0-or-later
//! Combine fixed and lazy placement edges without materializing placement meshes.
use super::projection::{distances, gedge_dist, slice};
use super::*;
use numpy::PyReadonlyArray1;
fn pseudo<'py>(idx: &Obj<'py>, i: usize) -> PyResult<Obj<'py>> {
    host(idx.py())?.call_method1("_group_pseudo_edge", (idx, i))
}
fn prefix<'py>(a: &Obj<'py>, n: usize) -> PyResult<Obj<'py>> {
    slice(a, 0, n, 1)
}
fn within<'py>(d: &Obj<'py>, radius: f64) -> PyResult<Obj<'py>> {
    d.py()
        .import("numpy")?
        .call_method1("flatnonzero", (d.call_method1("__lt__", (radius,))?,))
}
#[pyfunction]
fn nearby_loose_edges<'py>(
    v: &Obj<'py>,
    x: f64,
    y: f64,
    radius: f64,
    cap: usize,
) -> PyResult<Obj<'py>> {
    let py = v.py();
    let p = v.call_method0("_ledge_screen")?;
    let out = PyList::empty(py);
    if p.is_none() || !p.get_item(4)?.call_method0("any")?.is_truthy()? {
        return Ok(out.into_any());
    }
    let d = distances(&p, x, y, None)?;
    let mut rows = within(&d, radius)?;
    if rows.len()? > cap {
        rows = rows.get_item(prefix(&d.get_item(&rows)?.call_method0("argsort")?, cap)?)?
    }
    let edges = skip_index(v)?.getattr("edges")?;
    let n = edges.len()?;
    for i in rows.extract::<Vec<usize>>()? {
        if i < n {
            out.append(edges.get_item(i)?)?
        }
    }
    Ok(out.into_any())
}
#[pyfunction]
fn nearby_group_edges<'py>(
    v: &Obj<'py>,
    x: f64,
    y: f64,
    radius: f64,
    cap: usize,
) -> PyResult<Obj<'py>> {
    let py = v.py();
    let h = host(py)?;
    let c = h.getattr("DocumentInteraction")?;
    let normal = radius <= c.getattr("GEDGE_REACH_PX")?.extract()?
        && cap <= c.getattr("GEDGE_ENOUGH")?.extract()?;
    let nat = optional_call(v, "_lazy_native")?;
    if normal && !nat.is_none() {
        return v.call_method1("_nearby_edges_unbaked", (x, y, radius, cap, nat));
    }
    let idx = index(v, ("snap", x, y).into_pyobject(py)?.into_any())?;
    let p = v.call_method0("_gedge_screen")?;
    let out = PyList::empty(py);
    if p.is_none() || !p.get_item(4)?.call_method0("any")?.is_truthy()? {
        return Ok(out.into_any());
    }
    let d = if !normal {
        distances(&p, x, y, None)?
    } else if let Some(f) = v.getattr_opt("_gedge_dist")? {
        f.call1((x, y))?
    } else {
        gedge_dist(v, x, y)?
    };
    let np = py.import("numpy")?;
    let mut rows = within(&d, radius)?;
    if rows.len()? > cap {
        let mut values = d.get_item(&rows)?;
        if rows.len()? > cap.saturating_mul(4) {
            let part = prefix(
                &np.call_method1("argpartition", (&values, cap as isize - 1))?,
                cap,
            )?;
            rows = rows.get_item(&part)?;
            values = values.get_item(part)?;
        }
        rows = rows.get_item(prefix(
            &np.call_method1("lexsort", ((&rows, values),))?,
            cap,
        )?)?;
    }
    for i in rows.extract::<Vec<usize>>()? {
        out.append(pseudo(&idx, i)?)?
    }
    Ok(out.into_any())
}
#[pyfunction]
fn nearby_edges_unbaked<'py>(
    v: &Obj<'py>,
    x: f64,
    y: f64,
    radius: f64,
    cap: usize,
    nat: &Obj<'py>,
    with_d: bool,
) -> PyResult<Obj<'py>> {
    let py = v.py();
    let h = host(py)?;
    let c = h.getattr("DocumentInteraction")?;
    let wide: f64 = c.getattr("GEDGE_SHARED_PX")?.extract()?;
    let ncap: usize = c.getattr("GEDGE_ENOUGH")?.extract()?;
    let serial = opt(v, "_input_serial")?;
    let out = if !h.getattr("_EDGES_SHARED")?.is_truthy()?
        || serial.is_none()
        || !(radius <= wide && cap > 0 && cap <= ncap)
    {
        v.call_method1("_nearby_edges_found", (x, y, radius, cap, nat))?
    } else {
        let m = v.call_method0("_np_mvp")?.call_method0("ravel")?;
        let key = (
            serial,
            x,
            y,
            id(&nat.get_item(0)?),
            id(&nat.get_item(1)?),
            v.call_method0("_placements_epoch")?,
            opt(&v.getattr("scene")?, "version")?,
            PyTuple::new(py, m.try_iter()?.collect::<PyResult<Vec<_>>>()?)?,
            v.call_method0("width")?,
            v.call_method0("height")?,
            (
                id(&skip_index(v)?),
                id(&v.call_method0("_gedge_screen")?),
                id(&opt(v, "_depth_snap")?),
            ),
        )
            .into_pyobject(py)?
            .into_any();
        let found = match cache(v, "_edges_near_shared", &key)? {
            Some(f) => f,
            None => {
                let f = v.call_method1("_nearby_edges_found", (x, y, wide, ncap, nat))?;
                v.setattr("_edges_near_shared", (key, &f))?;
                f
            }
        };
        let out = PyList::empty(py);
        for entry in found.try_iter()? {
            let e = entry?;
            let d = e.get_item(0)?;
            if d.lt(radius)? {
                out.append((d, h.call_method1("_copy_snap_edge", (e.get_item(1)?,))?))?;
                if out.len() == cap {
                    break;
                }
            }
        }
        out.into_any()
    };
    if with_d {
        return Ok(out);
    }
    let edges = PyList::empty(py);
    for e in out.try_iter()? {
        edges.append(e?.get_item(1)?)?
    }
    Ok(edges.into_any())
}
fn lazy_hits<'py>(
    v: &Obj<'py>,
    nat: &Obj<'py>,
    rows: &Obj<'py>,
    x: f64,
    y: f64,
    radius: f64,
    cap: usize,
) -> PyResult<Obj<'py>> {
    let np = v.py().import("numpy")?;
    let m = np
        .call_method1(
            "ascontiguousarray",
            (v.call_method0("_np_mvp")?.getattr("T")?,),
        )?
        .call_method0("ravel")?
        .call_method0("tolist")?;
    nat.get_item(0)?.call_method1(
        "edges_near",
        (
            np.call_method1("ascontiguousarray", (rows, "int64"))?,
            m,
            v.call_method0("width")?,
            v.call_method0("height")?,
            x,
            y,
            radius,
            cap,
        ),
    )
}
fn visible_rows<'py>(rows: Obj<'py>, allowed: &Obj<'py>) -> PyResult<Obj<'py>> {
    rows.get_item(
        rows.py()
            .import("numpy")?
            .call_method1("isin", (&rows, allowed))?,
    )
}
fn fixed_count(idx: &Obj) -> PyResult<usize> {
    let fx = opt(idx, "gedge_fixed")?;
    if fx.is_none() {
        Ok(0)
    } else {
        fx.get_item(1)?.extract()
    }
}
#[pyfunction]
fn nearby_edges_found<'py>(
    v: &Obj<'py>,
    x: f64,
    y: f64,
    radius: f64,
    cap: usize,
    nat: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let py = v.py();
    let idx = skip_index(v)?;
    let m = fixed_count(&idx)?;
    let mut out = Vec::<(f64, Obj)>::new();
    if m > 0 && !v.call_method0("_gedge_screen")?.is_none() {
        let d = prefix(&v.call_method1("_gedge_dist", (x, y))?, m)?;
        for i in host(py)?
            .call_method1("_nearest_within", (&d, radius, cap))?
            .extract::<Vec<usize>>()?
        {
            out.push((d.get_item(i)?.extract()?, pseudo(&idx, i)?))
        }
    }
    let table = v.call_method0("_pick_lazy_table")?;
    let mut rows = v.call_method1("_footprint_need", (&table, x, y, radius))?;
    if rows.len()? > 0 {
        rows = visible_rows(rows, &nat.get_item(1)?)?
    }
    if rows.len()? > 0 {
        let hits = lazy_hits(v, nat, &rows, x, y, radius, cap)?;
        let groups = table.get_item(0)?;
        let a = hits.get_item(2)?;
        let b = hits.get_item(3)?;
        let d = hits.get_item(4)?;
        for (k, i) in hits
            .get_item(0)?
            .extract::<Vec<usize>>()?
            .into_iter()
            .enumerate()
        {
            let g = groups.get_item(i)?;
            let aa = Bound::new(py, Vec3::from(xyz(&a.get_item(k)?)?))?.into_any();
            let bb = Bound::new(py, Vec3::from(xyz(&b.get_item(k)?)?))?.into_any();
            let e = snap_edge(
                py,
                &aa,
                &bb,
                Some("in_group"),
                Some(&context(v, &g)?),
                Some(&v.call_method1("_owner_of", (&g,))?),
            )?;
            out.push((d.get_item(k)?.extract()?, e))
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.truncate(cap);
    Ok(PyList::new(py, out)?.into_any())
}
#[pyfunction]
fn pick_group_edge_unbaked<'py>(
    v: &Obj<'py>,
    x: f64,
    y: f64,
    nat: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let py = v.py();
    let best: f64 = v.getattr("pick_threshold_px")?.extract()?;
    let idx = skip_index(v)?;
    let m = fixed_count(&idx)?;
    let mut candidates = Vec::new();
    let param = |ax: f64, ay: f64, bx: f64, by: f64| {
        let (dx, dy) = (bx - ax, by - ay);
        let l = dx * dx + dy * dy;
        let t = (((x - ax) * dx + (y - ay) * dy) / if l > 1e-12 { l } else { 1. }).clamp(0., 1.);
        (t, (ax + t * dx - x).hypot(ay + t * dy - y))
    };
    let p = if m > 0 {
        v.call_method0("_gedge_screen")?
    } else {
        none(py)
    };
    if !p.is_none() {
        let (ax, ay, bx, by, ok): (
            PyReadonlyArray1<f64>,
            PyReadonlyArray1<f64>,
            PyReadonlyArray1<f64>,
            PyReadonlyArray1<f64>,
            PyReadonlyArray1<bool>,
        ) = p.extract()?;
        let (ax, ay, bx, by, ok) = (
            ax.as_array(),
            ay.as_array(),
            bx.as_array(),
            by.as_array(),
            ok.as_array(),
        );
        let selection = opt(&idx, "gedge_sel")?.extract::<Option<PyReadonlyArray1<bool>>>()?;
        let sel = selection.as_ref().map(|s| s.as_array());
        let a = idx.getattr("gedge_a")?;
        let b = idx.getattr("gedge_b")?;
        let groups = idx.getattr("gedge_groups")?;
        let gi = idx.getattr("gedge_gi")?;
        for i in 0..m
            .min(ax.len())
            .min(ay.len())
            .min(bx.len())
            .min(by.len())
            .min(ok.len())
        {
            if !ok[i] || sel.as_ref().is_some_and(|s| s.get(i) != Some(&true)) {
                continue;
            }
            let (t, d) = param(ax[i], ay[i], bx[i], by[i]);
            if d < best {
                candidates.push((
                    d,
                    xyz(&a.get_item(i)?)?,
                    xyz(&b.get_item(i)?)?,
                    t,
                    groups.get_item(gi.get_item(i)?)?,
                ))
            }
        }
    }
    let table = v.call_method0("_pick_lazy_table")?;
    let mut rows = v.call_method1("_footprint_need", (&table, x, y, best))?;
    let c = host(py)?.getattr("DocumentInteraction")?;
    if rows.len()? > 0 {
        rows = visible_rows(rows, &c.call_method1("_lazy_sel_rows", (v, nat))?)?
    }
    if rows.len()? > 0 {
        let hits = lazy_hits(
            v,
            nat,
            &rows,
            x,
            y,
            best,
            c.getattr("GEDGE_ENOUGH")?.extract()?,
        )?;
        let ids = hits.get_item(0)?.extract::<Vec<usize>>()?;
        let n = ids.len();
        if n > 0 {
            let a = hits.get_item(2)?;
            let b = hits.get_item(3)?;
            let d = hits.get_item(4)?;
            let groups = table.get_item(0)?;
            let np = py.import("numpy")?;
            let ends = np.call_method1(
                "concatenate",
                ((
                    np.call_method1("asarray", (&a, "float64"))?,
                    np.call_method1("asarray", (&b, "float64"))?,
                ),),
            )?;
            let p = v.call_method1("_project_px", (ends,))?;
            let ex = p.get_item(0)?;
            let ey = p.get_item(1)?;
            for (k, i) in ids.into_iter().enumerate() {
                let (t, _) = param(
                    ex.get_item(k)?.extract()?,
                    ey.get_item(k)?.extract()?,
                    ex.get_item(n + k)?.extract()?,
                    ey.get_item(n + k)?.extract()?,
                );
                candidates.push((
                    d.get_item(k)?.extract()?,
                    xyz(&a.get_item(k)?)?,
                    xyz(&b.get_item(k)?)?,
                    t,
                    v.call_method1("_owner_of", (groups.get_item(i)?,))?,
                ))
            }
        }
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (d, a, b, t, owner) in candidates {
        if !v
            .call_method1("_is_occluded", (Vec3::from(a + (b - a) * t),))?
            .is_truthy()?
        {
            return Ok((d, owner).into_pyobject(py)?.into_any());
        }
    }
    Ok((best, py.None()).into_pyobject(py)?.into_any())
}
#[pyfunction]
fn pick_edge_any<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let py = v.py();
    let options = PyDict::new(py);
    options.set_item("visible_only", false)?;
    options.set_item("which", "snap")?;
    let loose = v.call_method("_pick_edge", (x, y), Some(&options))?;
    let nat = optional_call(v, "_lazy_native")?;
    let threshold = v.getattr("pick_threshold_px")?;
    let (dist, group) = if !nat.is_none() {
        let near = v.call_method(
            "_nearby_edges_unbaked",
            (x, y, threshold, 1, nat),
            Some(&kw(
                py,
                "with_d",
                &true.into_pyobject(py)?.to_owned().into_any(),
            )?),
        )?;
        if near.len()? == 0 {
            return Ok(loose);
        }
        near.get_item(0)?.extract::<(f64, Obj)>()?
    } else {
        let idx = index(v, ("px", x, y).into_pyobject(py)?.into_any())?;
        let p = v.call_method0("_gedge_screen")?;
        if p.is_none() || !p.get_item(4)?.call_method0("any")?.is_truthy()? {
            return Ok(loose);
        }
        let d = v.call_method1("_gedge_dist", (x, y))?;
        let i: usize = d.call_method0("argmin")?.extract()?;
        let distance = d.get_item(i)?.extract::<f64>()?;
        if distance >= threshold.extract()? {
            return Ok(loose);
        }
        (distance, pseudo(&idx, i)?)
    };
    if !loose.is_none() {
        let a = v.call_method1("_world_to_pixel", (loose.getattr("a")?,))?;
        let b = v.call_method1("_world_to_pixel", (loose.getattr("b")?,))?;
        if !a.is_none()
            && !b.is_none()
            && py
                .import("core.snap")?
                .call_method1("_closest_on_segment_2d", ((x, y), a, b))?
                .get_item(0)?
                .le(dist)?
        {
            return Ok(loose);
        }
    }
    Ok(group)
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(nearby_loose_edges, m)?)?;
    m.add_function(wrap_pyfunction!(nearby_group_edges, m)?)?;
    m.add_function(wrap_pyfunction!(nearby_edges_unbaked, m)?)?;
    m.add_function(wrap_pyfunction!(nearby_edges_found, m)?)?;
    m.add_function(wrap_pyfunction!(pick_group_edge_unbaked, m)?)?;
    m.add_function(wrap_pyfunction!(pick_edge_any, m)?)?;
    Ok(())
}
