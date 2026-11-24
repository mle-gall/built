// SPDX-License-Identifier: GPL-3.0-or-later
//! Screen-space loose edge and wire-only group picking.
use super::*;
use crate::math::Vec3;
use glam::DVec2;
use numpy::{PyReadonlyArray1, PyReadonlyArray2};
use std::collections::HashSet;

fn near_index<'py>(view: &Obj<'py>, near: Obj<'py>) -> PyResult<Obj<'py>> {
    let kw = PyDict::new(view.py());
    kw.set_item("near", near)?;
    view.call_method("_pick_index", (), Some(&kw))
}

#[pyfunction]
pub fn edge_cursor_point<'py>(
    view: &Obj<'py>,
    edge: &Obj<'py>,
    x: f64,
    y: f64,
) -> PyResult<Option<Vec3>> {
    let a = edge.getattr("a")?;
    let b = edge.getattr("b")?;
    let pa = view
        .call_method1("_world_to_pixel", (&a,))?
        .extract::<Option<[f64; 2]>>()?;
    let pb = view
        .call_method1("_world_to_pixel", (&b,))?
        .extract::<Option<[f64; 2]>>()?;
    let (Some(pa), Some(pb)) = (pa, pb) else {
        return Ok(None);
    };
    let p = DVec2::from(pa);
    let delta = DVec2::from(pb) - p;
    let l = delta.length_squared();
    let t = if l < 1e-12 {
        0.
    } else {
        ((DVec2::new(x, y) - p).dot(delta) / l).clamp(0., 1.)
    };
    let a = xyz(&a)?;
    Ok(Some((a + (xyz(&b)? - a) * t).into()))
}

#[pyfunction]
pub fn linework_edge_rows<'py>(view: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let py = view.py();
    if view.hasattr("_lazy_native")? && !view.call_method0("_lazy_native")?.is_none() {
        return Ok(none(py));
    }
    let idx = near_index(view, ("px", x, y).into_pyobject(py)?.into_any())?;
    let wire = attr(&idx, "gedge_wire")?;
    if idx.getattr("gedge_a")?.is_none()
        || !wire.is_truthy()?
        || wire.len()? != idx.getattr("gedge_groups")?.len()?
    {
        return Ok(none(py));
    }
    let memo = attr(&idx, "_linework_rows")?;
    let gi = idx.getattr("gedge_gi")?;
    let rows = if !memo.is_none() && memo.get_item(0)?.is(&gi) {
        memo.get_item(1)?
    } else {
        let np = py.import("numpy")?;
        let mask = np.call_method1("asarray", (wire, "bool"))?.get_item(&gi)?;
        let mut rows = np.call_method1("flatnonzero", (mask,))?;
        let sel = attr(&idx, "gedge_sel")?;
        if !sel.is_none() {
            rows = rows.get_item(sel.get_item(&rows)?)?;
        }
        idx.setattr("_linework_rows", (gi, &rows))?;
        rows
    };
    if rows.len()? == 0 {
        return Ok(none(py));
    }
    let proj = view.call_method0("_gedge_screen")?;
    if proj.is_none() {
        return Ok(none(py));
    }
    let ok = proj.get_item(4)?.get_item(&rows)?;
    let chosen = rows.get_item(ok)?;
    if chosen.len()? == 0 {
        return Ok(none(py));
    }
    let col = |i: usize| proj.get_item(i)?.get_item(&chosen);
    Ok((idx, &chosen, col(0)?, col(1)?, col(2)?, col(3)?)
        .into_pyobject(py)?
        .into_any())
}

fn parameter(ax: f64, ay: f64, bx: f64, by: f64, x: f64, y: f64) -> (f64, f64) {
    let dx = bx - ax;
    let dy = by - ay;
    let l2 = dx * dx + dy * dy;
    let t = (((x - ax) * dx + (y - ay) * dy) / if l2 > 1e-12 { l2 } else { 1. }).clamp(0., 1.);
    (t, (ax + t * dx - x).hypot(ay + t * dy - y))
}

#[pyfunction]
pub fn linework_groups<'py>(view: &Obj<'py>, x: f64, y: f64, around: bool) -> PyResult<Obj<'py>> {
    let py = view.py();
    let result = PyList::empty(py);
    let found = view.call_method1("_linework_edge_rows", (x, y))?;
    if found.is_none() {
        return Ok(result.into_any());
    }
    let (idx, rows, ax, ay, bx, by): (
        Obj,
        Obj,
        PyReadonlyArray1<f64>,
        PyReadonlyArray1<f64>,
        PyReadonlyArray1<f64>,
        PyReadonlyArray1<f64>,
    ) = found.extract()?;
    let gi = idx.getattr("gedge_gi")?;
    let groups = idx.getattr("gedge_groups")?;
    let scene = view.getattr("scene")?;
    if around {
        let np = py.import("numpy")?;
        return py.import("views.document_interaction")?.call_method1(
            "_boxes_around",
            (
                gi.get_item(rows)?,
                np.call_method1("minimum", (&*ax, &*bx))?,
                np.call_method1("minimum", (&*ay, &*by))?,
                np.call_method1("maximum", (&*ax, &*bx))?,
                np.call_method1("maximum", (&*ay, &*by))?,
                x,
                y,
                groups.getattr("__getitem__")?,
                scene,
            ),
        );
    }
    let (ax, ay, bx, by) = (ax.as_array(), ay.as_array(), bx.as_array(), by.as_array());
    let n = rows.len()?;
    if [ax.len(), ay.len(), bx.len(), by.len()] != [n; 4] {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "invalid wire projections",
        ));
    }
    let threshold: f64 = view.getattr("pick_threshold_px")?.extract()?;
    let mut near = Vec::new();
    let mut distances = Vec::new();
    for i in 0..n {
        let (t, d) = parameter(ax[i], ay[i], bx[i], by[i], x, y);
        if d < threshold {
            near.push((i, t, d));
            distances.push(d);
        }
    }
    let order: Vec<usize> = distances
        .into_pyarray(py)
        .call_method0("argsort")?
        .extract()?;
    let mut seen = HashSet::new();
    for k in order.into_iter().take(64) {
        let (i, t, d) = near[k];
        let row = rows.get_item(i)?;
        let owner = groups.get_item(gi.get_item(&row)?)?;
        let id = owner.as_ptr() as usize;
        if seen.contains(&id)
            || !scene
                .call_method1("entity_selectable", (&owner,))?
                .is_truthy()?
        {
            continue;
        }
        let a = xyz(&idx.getattr("gedge_a")?.get_item(&row)?)?;
        let b = xyz(&idx.getattr("gedge_b")?.get_item(&row)?)?;
        if view
            .call_method1("linework_hidden", (Vec3::from(a + (b - a) * t),))?
            .is_truthy()?
        {
            continue;
        }
        seen.insert(id);
        result.append((owner, d))?;
    }
    Ok(result.into_any())
}

type Projection<'py> = (
    PyReadonlyArray1<'py, f64>,
    PyReadonlyArray1<'py, f64>,
    PyReadonlyArray1<'py, bool>,
);

#[pyfunction]
pub fn pick_edge_distance<'py>(
    view: &Obj<'py>,
    x: f64,
    y: f64,
    visible: bool,
    which: &str,
    hidden: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let py = view.py();
    let idx = near_index(view, "skip".into_pyobject(py)?.into_any())?;
    let a = idx.getattr("edge_a")?;
    if a.is_none() {
        return Ok(none(py));
    }
    let b = idx.getattr("edge_b")?;
    let (ax, ay, oka): Projection = view.call_method1("_project_px", (&a,))?.extract()?;
    let (bx, by, okb): Projection = view.call_method1("_project_px", (&b,))?.extract()?;
    let allowed = if which == "sel" {
        idx.getattr("edge_sel")?
    } else {
        idx.getattr_opt("edge_snap")?
            .unwrap_or(idx.getattr("edge_sel")?)
    };
    let allowed: PyReadonlyArray1<bool> = allowed.extract()?;
    let scene = view.getattr("scene")?;
    let soft = if visible && !attr(&scene, "show_hidden_geometry")?.is_truthy()? {
        attr(&idx, "edge_soft")?.extract::<Option<PyReadonlyArray1<bool>>>()?
    } else {
        None
    };
    let cut = py
        .import("views.document_interaction")?
        .call_method1("_active_cut", (scene,))?;
    let clipping = if cut.is_none() {
        None
    } else {
        let n = xyz(&cut.getattr("normal")?)?;
        Some((
            n.to_array(),
            n.dot(xyz(&cut.getattr("point")?)?),
            a.extract::<PyReadonlyArray2<f64>>()?,
            b.extract::<PyReadonlyArray2<f64>>()?,
        ))
    };
    let (ax, ay, oka, bx, by, okb, allowed) = (
        ax.as_array(),
        ay.as_array(),
        oka.as_array(),
        bx.as_array(),
        by.as_array(),
        okb.as_array(),
        allowed.as_array(),
    );
    let count = ax.len();
    if [
        ay.len(),
        oka.len(),
        bx.len(),
        by.len(),
        okb.len(),
        allowed.len(),
    ] != [count; 6]
        || soft.as_ref().is_some_and(|s| s.as_array().len() != count)
        || clipping.as_ref().is_some_and(|(_, _, a, b)| {
            a.as_array().dim() != (count, 3) || b.as_array().dim() != (count, 3)
        })
    {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "invalid edge projections",
        ));
    }
    let soft = soft.as_ref().map(|s| s.as_array());
    let threshold: f64 = view.getattr("pick_threshold_px")?.extract()?;
    let mut ds = vec![f64::INFINITY; count];
    let mut nearest = None;
    let mut best = f64::INFINITY;
    for i in 0..count {
        if !(oka[i] && okb[i] && allowed[i]) || soft.as_ref().is_some_and(|s| s[i]) {
            continue;
        }
        if let Some((n, c, a, b)) = &clipping {
            let side = |p: &PyReadonlyArray2<f64>| {
                let p = p.as_array();
                p[[i, 0]] * n[0] + p[[i, 1]] * n[1] + p[[i, 2]] * n[2] - c
            };
            if side(a) > 1e-6 && side(b) > 1e-6 {
                continue;
            }
        }
        let d = parameter(ax[i], ay[i], bx[i], by[i], x, y).1;
        ds[i] = d;
        if d < best {
            nearest = Some(i);
            best = d;
        }
    }
    if nearest.is_none() || best >= threshold {
        return Ok(none(py));
    }
    let edges = idx.getattr("edges")?;
    if !visible {
        return Ok((edges.get_item(nearest.unwrap())?, best)
            .into_pyobject(py)?
            .into_any());
    }
    let distances = ds.into_pyarray(py);
    let order: PyReadonlyArray1<i64> = distances.call_method0("argsort")?.extract()?;
    let hidden = if hidden.is_truthy()? {
        hidden.clone()
    } else {
        view.getattr("_is_occluded")?
    };
    for &i in order.as_array().iter().take(8) {
        let d = distances.get_item(i)?.extract::<f64>()?;
        if d >= threshold {
            break;
        }
        let edge = edges.get_item(i)?;
        let p = view.call_method1("edge_point_under_cursor", (&edge, x, y))?;
        if p.is_none() || !hidden.call1((p,))?.is_truthy()? {
            return Ok((edge, d).into_pyobject(py)?.into_any());
        }
    }
    Ok(none(py))
}
