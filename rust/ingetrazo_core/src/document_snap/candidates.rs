// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use numpy::{AllowTypeChange, IntoPyArray, PyArrayLike1, PyArrayLike2};
use pyo3::types::{PySet, PySlice};
use std::collections::{BTreeMap, HashSet};

#[pyfunction]
fn snap_context_of<'py>(v: &Obj<'py>, place: &Obj<'py>) -> PyResult<Option<&'static str>> {
    if place.is_none() {
        return Ok(None);
    }
    let ctx = opt(&opt(v, "scene")?, "edit_group")?;
    if !ctx.is_none() && !place.is(&ctx) && !opt(place, "context")?.is(&ctx) {
        return Ok(Some("outside"));
    }
    let owner = opt(place, "owner")?;
    let owner = if owner.is_truthy()? { &owner } else { place };
    Ok(Some(if optional_call(owner, "is_instance")?.is_truthy()? {
        "component"
    } else {
        "group"
    }))
}
#[pyfunction]
fn group_pseudo_edge<'py>(idx: &Obj<'py>, i: usize) -> PyResult<Obj<'py>> {
    let py = idx.py();
    let gi = opt(idx, "gedge_gi")?;
    let groups = opt(idx, "gedge_groups")?;
    let (g, owner) = if !gi.is_none() && groups.is_truthy()? {
        let g = gi.get_item(i)?.extract::<usize>()?;
        (g, groups.get_item(g)?)
    } else {
        (0, none(py))
    };
    let ctxs = opt(idx, "gedge_ctx")?;
    let ctx = if !ctxs.is_none() && g < ctxs.len()? {
        ctxs.get_item(g)?
    } else {
        (if optional_call(&owner, "is_instance")?.is_truthy()? {
            "component"
        } else {
            "group"
        })
        .into_pyobject(py)?
        .into_any()
    };
    let end = |k: &str| -> PyResult<Obj<'py>> {
        Ok(Bound::new(
            py,
            Vec3::from(glam::DVec3::from_array(
                idx.getattr(k)?.get_item(i)?.extract()?,
            )),
        )?
        .into_any())
    };
    snap_edge(
        py,
        &end("gedge_a")?,
        &end("gedge_b")?,
        Some("in_group"),
        Some(&ctx),
        Some(&owner),
    )
}
#[pyfunction]
fn active_cut<'py>(scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    if let Some(on) = scene.getattr_opt("show_section_cuts")? {
        if !on.is_truthy()? {
            return Ok(none(scene.py()));
        }
    }
    optional_call(scene, "active_section")
}
#[pyfunction]
fn snap_dropped_loose<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let h = host(py)?;
    let sc = v.getattr("scene")?;
    let mesh = sc.getattr("mesh")?;
    let key = (
        h.call_method1("_cache_ver", (v,))?,
        id(&mesh),
        mesh.getattr("edges")?.len()?,
        opt(&sc, "show_hidden_geometry")?.is_truthy()?,
    )
        .into_pyobject(py)?;
    if let Some(old) = cache(v, "_loose_snap_drops_cache", key.as_any())? {
        return Ok(old);
    }
    let drops = PySet::empty(py)?;
    let identity = h.getattr("element_identity")?;
    let show = sc.getattr("edge_snappable")?;
    for e in sc.getattr("edges")?.try_iter()? {
        let e = e?;
        if !show.call1((&e,))?.is_truthy()? {
            drops.add(identity.call1((e,))?)?;
        }
    }
    v.setattr("_loose_snap_drops_cache", (key, &drops))?;
    Ok(drops.into_any())
}
#[pyfunction]
fn plan_curved_edges<'py>(plan: &Obj<'py>) -> PyResult<Obj<'py>> {
    let extra = plan.getattr("extra")?;
    let hit = extra.call_method1("get", ("curved_edges",))?;
    if !hit.is_none() {
        return Ok(hit);
    }
    let curves = plan.getattr("curve")?;
    let mut out = vec![];
    if curves
        .call_method1("count", (plan.py().None(),))?
        .extract::<usize>()?
        != curves.len()?
    {
        for (i, c) in curves.try_iter()?.enumerate() {
            if !c?.is_none() {
                out.push(i as i64);
            }
        }
    }
    let out = out.into_pyarray(plan.py());
    extra.set_item("curved_edges", &out)?;
    Ok(out.into_any())
}
#[pyfunction]
fn lazy_edge_may_be_curve(mesh: &Obj, a: &Obj, b: &Obj, tol: f64) -> PyResult<bool> {
    let py = mesh.py();
    let p = py
        .import("core.lazy_mesh")?
        .call_method1("plan_of", (mesh,))?;
    if p.is_none() {
        return Ok(true);
    }
    let edges = p.getattr("edges")?;
    if edges.len()? == 0 {
        return Ok(false);
    }
    let curved = plan_curved_edges(&p)?;
    if curved.len()? == 0 {
        return Ok(false);
    }
    let ends = edges
        .get_item(curved)?
        .extract::<PyArrayLike2<i64, AllowTypeChange>>()?;
    let pos = p
        .getattr("pos")?
        .extract::<PyArrayLike2<f64, AllowTypeChange>>()?;
    let (ends, pos) = (ends.as_array(), pos.as_array());
    let (a, b) = (xyz(a)?, xyz(b)?);
    if pos.ncols() != 3 || ends.ncols() != 2 {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "invalid curve arrays",
        ));
    }
    let near = |i: i64, p: glam::DVec3| -> PyResult<bool> {
        if i < 0 || i as usize >= pos.nrows() {
            return Err(pyo3::exceptions::PyIndexError::new_err(i));
        }
        let row = pos.row(i as usize);
        Ok((0..3).all(|j| (row[j] - p[j]).abs() <= tol))
    };
    for r in ends.rows() {
        if (near(r[0], a)? && near(r[1], b)?) || (near(r[0], b)? && near(r[1], a)?) {
            return Ok(true);
        }
    }
    Ok(false)
}
#[pyfunction]
fn hypot3_f32<'py>(
    py: Python<'py>,
    a: PyArrayLike2<'py, f32, AllowTypeChange>,
    kind: &str,
) -> PyResult<Obj<'py>> {
    let rows = a.as_array();
    if rows.ncols() != 3 {
        return Err(pyo3::exceptions::PyValueError::new_err("expected XYZ rows"));
    }
    let h = if kind == "msvc" {
        crate::facegeom::Hypot::Msvc
    } else {
        crate::facegeom::Hypot::LibStdCxx
    };
    let out = rows
        .rows()
        .into_iter()
        .map(|r| {
            if kind != "msvc" && r.iter().any(|x| x.is_nan()) {
                0.
            } else {
                crate::facegeom::hypot3([r[0], r[1], r[2]], h)
            }
        })
        .collect::<Vec<_>>();
    Ok(out.into_pyarray(py).into_any())
}
#[pyfunction]
fn boxes_around<'py>(
    py: Python<'py>,
    keys: PyArrayLike1<'py, i64, AllowTypeChange>,
    x0: PyArrayLike1<'py, f64, AllowTypeChange>,
    y0: PyArrayLike1<'py, f64, AllowTypeChange>,
    x1: PyArrayLike1<'py, f64, AllowTypeChange>,
    y1: PyArrayLike1<'py, f64, AllowTypeChange>,
    px: f64,
    py_: f64,
    owner_of: &Obj<'py>,
    scene: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let keys = keys.as_array();
    let coords = [x0.as_array(), y0.as_array(), x1.as_array(), y1.as_array()];
    if coords.iter().any(|v| v.len() != keys.len()) {
        return Err(pyo3::exceptions::PyValueError::new_err("box lengths"));
    }
    let mut boxes = BTreeMap::new();
    for (i, &k) in keys.iter().enumerate() {
        let r = boxes.entry(k).or_insert([
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ]);
        for j in 0..4 {
            let x = coords[j][i];
            r[j] = if x.is_nan() || r[j].is_nan() {
                f64::NAN
            } else if j < 2 {
                r[j].min(x)
            } else {
                r[j].max(x)
            };
        }
    }
    let (keys, areas): (Vec<_>, Vec<_>) = boxes
        .into_iter()
        .filter(|(_, b)| b[0] <= px && px <= b[2] && b[1] <= py_ && py_ <= b[3])
        .map(|(k, b)| (k, (b[2] - b[0]) * (b[3] - b[1])))
        .unzip();
    let out = PyList::empty(py);
    let mut seen = HashSet::new();
    if keys.is_empty() {
        return Ok(out.into_any());
    }
    // numpy's default tie order is part of the old select cycle.
    for i in areas
        .into_pyarray(py)
        .call_method0("argsort")?
        .extract::<Vec<usize>>()?
    {
        let g = owner_of.call1((keys[i],))?;
        if !g.is_none()
            && !seen.contains(&id(&g))
            && scene
                .call_method1("entity_selectable", (&g,))?
                .is_truthy()?
        {
            seen.insert(id(&g));
            out.append(g)?;
        }
    }
    Ok(out.into_any())
}
#[pyfunction]
fn document_nearest_within<'py>(d: &Obj<'py>, radius: f64, cap: isize) -> PyResult<Obj<'py>> {
    let py = d.py();
    let np = py.import("numpy")?;
    let native = py.import("core._native")?.getattr("native")?;
    if cap > 0
        && d.len()? > 4096
        && host(py)?.getattr("_NEAREST_NATIVE")?.is_truthy()?
        && native.hasattr("nearest_within")?
    {
        return np.call_method1(
            "asarray",
            (
                native.call_method1(
                    "nearest_within",
                    (
                        np.call_method1("ascontiguousarray", (d, "float64"))?,
                        radius,
                        cap,
                    ),
                )?,
                "int64",
            ),
        );
    }
    let mut cand = np.call_method1("flatnonzero", (d.call_method1("__lt__", (radius,))?,))?;
    if cand.len()? as isize > cap {
        let mut dist = d.get_item(&cand)?;
        let end = PySlice::new(py, 0, cap, 1);
        if cand.len()? as isize > 4 * cap {
            let part = np
                .call_method1("argpartition", (&dist, cap - 1))?
                .get_item(&end)?;
            cand = cand.get_item(&part)?;
            dist = dist.get_item(&part)?;
        }
        cand = cand.get_item(
            np.call_method1("lexsort", ((&cand, &dist),))?
                .get_item(end)?,
        )?;
    }
    Ok(cand)
}
#[pyfunction]
fn px_near<'py>(
    x: &Obj<'py>,
    y: &Obj<'py>,
    ok: &Obj<'py>,
    px: f64,
    py_: f64,
    thr: f64,
    pts: &Obj<'py>,
    seen: &Obj<'py>,
) -> PyResult<(Obj<'py>, Obj<'py>)> {
    let py = x.py();
    let h = host(py)?;
    let np = py.import("numpy")?;
    let native = py.import("core._native")?.getattr("native")?;
    let mut hidden = !seen.is_none();
    let hypot = |x: &Obj<'py>, y: &Obj<'py>| np.call_method1("hypot", (x.sub(px)?, y.sub(py_)?));
    let (mut k, mut d) = if x.len()? > 4096 && native.hasattr("points_near_px")? {
        let a = np.call_method1("ascontiguousarray", (x, "float64"))?;
        let b = np.call_method1("ascontiguousarray", (y, "float64"))?;
        let valid = np.call_method1("ascontiguousarray", (ok, "bool"))?;
        let r = thr * (1. + 1e-6) + 1e-6;
        let plain = || native.call_method1("points_near_px", (&a, &b, &valid, px, py_, r));
        let k = if hidden && h.getattr("_NEAR_SEEN_NATIVE")?.is_truthy()? {
            match native.call_method1(
                "points_near_px",
                (
                    &a,
                    &b,
                    &valid,
                    px,
                    py_,
                    r,
                    np.call_method1("ascontiguousarray", (pts, "float64"))?,
                    seen,
                ),
            ) {
                Ok(k) => {
                    hidden = false;
                    k
                }
                Err(e) if e.is_instance_of::<pyo3::exceptions::PyTypeError>(py) => plain()?,
                Err(e) => return Err(e),
            }
        } else {
            plain()?
        };
        let d = hypot(&x.get_item(&k)?, &y.get_item(&k)?)?;
        let near = d.call_method1("__lt__", (thr,))?;
        (k.get_item(&near)?, d.get_item(&near)?)
    } else {
        let d = np.call_method1("where", (ok, hypot(x, y)?, f64::INFINITY))?;
        let k = np.call_method1("flatnonzero", (d.call_method1("__lt__", (thr,))?,))?;
        let dist = d.get_item(&k)?;
        (k, dist)
    };
    if hidden && k.len()? > 0 {
        let keep = h
            .getattr("DocumentInteraction")?
            .call_method1(
                "_depth_hidden",
                (seen, pts.get_item(&k)?, x.get_item(&k)?, y.get_item(&k)?),
            )?
            .call_method0("__invert__")?;
        k = k.get_item(&keep)?;
        d = d.get_item(&keep)?;
    }
    Ok((k, d))
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(active_cut, m)?)?;
    m.add_function(wrap_pyfunction!(snap_context_of, m)?)?;
    m.add_function(wrap_pyfunction!(group_pseudo_edge, m)?)?;
    m.add_function(wrap_pyfunction!(snap_dropped_loose, m)?)?;
    m.add_function(wrap_pyfunction!(plan_curved_edges, m)?)?;
    m.add_function(wrap_pyfunction!(lazy_edge_may_be_curve, m)?)?;
    m.add_function(wrap_pyfunction!(hypot3_f32, m)?)?;
    m.add_function(wrap_pyfunction!(boxes_around, m)?)?;
    m.add_function(wrap_pyfunction!(document_nearest_within, m)?)?;
    m.add_function(wrap_pyfunction!(px_near, m)?)?;
    Ok(())
}
