// SPDX-License-Identifier: GPL-3.0-or-later
//! Share projection work between edge picking and the snap prefilter. A camera
//! change invalidates every chunk; baking another placement preserves the head.
use super::*;
use pyo3::types::PySlice;
pub(super) fn slice<'py>(
    v: &Obj<'py>,
    start: usize,
    stop: usize,
    step: isize,
) -> PyResult<Obj<'py>> {
    v.get_item(PySlice::new(v.py(), start as isize, stop as isize, step))
}
fn columns<'py>(view: &Obj<'py>, a: Obj<'py>, b: Obj<'py>) -> PyResult<Obj<'py>> {
    let a = view.call_method1("_project_px", (a,))?;
    let b = view.call_method1("_project_px", (b,))?;
    Ok((
        a.get_item(0)?,
        a.get_item(1)?,
        b.get_item(0)?,
        b.get_item(1)?,
        a.get_item(2)?.bitand(b.get_item(2)?)?,
    )
        .into_pyobject(view.py())?
        .into_any())
}
fn camera<'py>(v: &Obj<'py>) -> PyResult<(usize, Obj<'py>, Obj<'py>, Obj<'py>)> {
    Ok((
        id(&v.getattr("scene")?.getattr("mesh")?),
        v.call_method0("_np_mvp")?.call_method0("tobytes")?,
        v.call_method0("width")?,
        v.call_method0("height")?,
    ))
}
#[pyfunction]
fn ledge_screen<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let idx = skip_index(v)?;
    let a = idx.getattr("edge_a")?;
    if a.is_none() || a.len()? == 0 {
        return Ok(none(py));
    }
    let (mesh, m, w, h) = camera(v)?;
    let key = (
        host(py)?.call_method1("_content_ver", (v,))?,
        mesh,
        m,
        w,
        h,
        optional_call(v, "_rest_is_hidden")?.is_truthy()?,
    )
        .into_pyobject(py)?
        .into_any();
    if let Some(data) = cache(v, "_ledge_px_cache", &key)? {
        return Ok(data);
    }
    let data = columns(v, a, idx.getattr("edge_b")?)?;
    v.setattr("_ledge_px_cache", (key, &data))?;
    Ok(data)
}
#[pyfunction]
fn gedge_screen<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = v.py();
    let idx = skip_index(v)?;
    let a = idx.getattr("gedge_a")?;
    if a.is_none() || a.len()? == 0 {
        return Ok(none(py));
    }
    let camera = camera(v)?;
    let mode = opt(v, "_edit_rest_mode")?;
    let vkey = (camera.0, camera.1, camera.2, camera.3, mode).into_pyobject(py)?;
    let key = (id(&a), &vkey).into_pyobject(py)?;
    let old = opt(v, "_gedge_px_cache")?;
    if !old.is_none() && old.len()? > 2 && old.get_item(0)?.eq(&key)? && old.get_item(2)?.is(&a) {
        return old.get_item(1);
    }
    let b = idx.getattr("gedge_b")?;
    let fx = opt(&idx, "gedge_fixed")?;
    let head = if fx.is_none() {
        None
    } else {
        let n: usize = fx.get_item(1)?.extract()?;
        let chunks = fx.get_item(2)?.try_iter()?.collect::<PyResult<Vec<_>>>()?;
        let total = chunks
            .iter()
            .try_fold(n, |sum, c| Ok::<_, PyErr>(sum + c.len()?))?;
        (total == a.len()?).then_some((n, chunks))
    };
    let data = if let Some((n, chunks)) = head {
        let fixed = fx.get_item(0)?;
        let cached = opt(v, "_gedge_px_head")?;
        let projection = if !cached.is_none()
            && cached.get_item(0)?.eq(&vkey)?
            && cached.get_item(1)?.is(&fixed)
            && cached.get_item(2)?.eq(n)?
        {
            cached.get_item(3)?
        } else {
            let p = columns(v, slice(&a, 0, n, 1)?, slice(&b, 0, n, 1)?)?;
            v.setattr("_gedge_px_head", (&vkey, fixed, n, &p))?;
            p
        };
        let memo = opt(v, "_gedge_px_pieces")?;
        let pieces = if !memo.is_none() && memo.get_item(0)?.eq(&vkey)? {
            memo.get_item(1)?
        } else {
            let d = PyDict::new(py);
            v.setattr("_gedge_px_pieces", (&vkey, &d))?;
            d.into_any()
        };
        let np = py.import("numpy")?;
        let mut missing = Vec::new();
        let batch = PyList::empty(py);
        for c in &chunks {
            let hit = pieces.call_method1("get", (id(c),))?;
            if hit.is_none() || !hit.get_item(0)?.is(c) {
                missing.push(c);
                batch.append(c.call_method1("reshape", (-1, 3))?)?
            }
        }
        if !missing.is_empty() {
            let projected =
                v.call_method1("_project_px", (np.call_method1("concatenate", (batch,))?,))?;
            let x = projected.get_item(0)?;
            let y = projected.get_item(1)?;
            let ok = projected.get_item(2)?;
            let mut offset = 0;
            for c in missing {
                let end = offset + 2 * c.len()?;
                let p = (
                    slice(&x, offset, end, 2)?,
                    slice(&y, offset, end, 2)?,
                    slice(&x, offset + 1, end, 2)?,
                    slice(&y, offset + 1, end, 2)?,
                    slice(&ok, offset, end, 2)?.bitand(slice(&ok, offset + 1, end, 2)?)?,
                );
                pieces.set_item(id(c), (c, p))?;
                offset = end
            }
        }
        let mut parts = vec![projection];
        for c in chunks {
            parts.push(pieces.get_item(id(&c))?.get_item(1)?)
        }
        let cols = (0..5)
            .map(|k| {
                let col = parts
                    .iter()
                    .map(|p| p.get_item(k))
                    .collect::<PyResult<Vec<_>>>()?;
                np.call_method1("concatenate", (col,))
            })
            .collect::<PyResult<Vec<_>>>()?;
        PyTuple::new(py, cols)?.into_any()
    } else {
        columns(v, a.clone(), b)?
    };
    v.setattr("_gedge_px_cache", (key, &data, a))?;
    Ok(data)
}
pub(super) fn distances<'py>(
    p: &Obj<'py>,
    x: f64,
    y: f64,
    ids: Option<&Obj<'py>>,
) -> PyResult<Obj<'py>> {
    let mut args = p.try_iter()?.collect::<PyResult<Vec<_>>>()?;
    args.push(x.into_pyobject(p.py())?.into_any());
    args.push(y.into_pyobject(p.py())?.into_any());
    if let Some(rows) = ids {
        args.push(rows.clone())
    }
    p.py()
        .import("built_core")?
        .call_method1("segment_distances", PyTuple::new(p.py(), args)?)
}
#[pyfunction]
pub(super) fn gedge_dist<'py>(v: &Obj<'py>, x: f64, y: f64) -> PyResult<Obj<'py>> {
    let py = v.py();
    let p = v.call_method0("_gedge_screen")?;
    let key = (
        id(&p),
        x.round_ties_even() as i64,
        y.round_ties_even() as i64,
    )
        .into_pyobject(py)?;
    let old = opt(v, "_gedge_dist_cache")?;
    if !old.is_none() && old.len()? > 2 && old.get_item(0)?.eq(&key)? && old.get_item(2)?.is(&p) {
        return old.get_item(1);
    }
    let n = p.get_item(0)?.len()?;
    let h = host(py)?;
    let class = h.getattr("DocumentInteraction")?;
    let reach: f64 = class.getattr("GEDGE_REACH_PX")?.extract()?;
    let threshold = v
        .getattr_opt("pick_threshold_px")?
        .map(|q| q.extract::<f64>())
        .transpose()?
        .unwrap_or(0.);
    let mut d = None;
    if !h.getattr("_NO_SNAP_FAST")?.is_truthy()?
        && n >= class.getattr("GEDGE_GRID_MIN")?.extract()?
        && threshold <= reach
    {
        let slot = opt(v, "_gedge_grid_cache")?;
        if slot.is_none() || !slot.get_item(0)?.is(&p) {
            v.setattr("_gedge_grid_cache", (&p, py.None()))?
        } else {
            let mut grid = slot.get_item(1)?;
            if grid.is_none() {
                grid = py.import("built_core")?.call_method1(
                    "EdgeGrid",
                    (
                        p.get_item(0)?,
                        p.get_item(1)?,
                        p.get_item(2)?,
                        p.get_item(3)?,
                        p.get_item(4)?,
                        v.call_method0("width")?,
                        v.call_method0("height")?,
                        reach,
                    ),
                )?;
                v.setattr("_gedge_grid_cache", (&p, &grid))?
            }
            for radius in [
                threshold.max(class.getattr("GEDGE_NEAR_PX")?.extract()?),
                reach,
            ] {
                let ids = grid.call_method1("near", (x, y, radius))?;
                if ids.is_none() {
                    break;
                }
                let sub = distances(&p, x, y, Some(&ids))?;
                if radius >= reach
                    || sub
                        .call_method1("__lt__", (radius,))?
                        .call_method0("sum")?
                        .ge(class.getattr("GEDGE_ENOUGH")?)?
                {
                    let all = py
                        .import("numpy")?
                        .call_method1("full", (n, f64::INFINITY))?;
                    all.set_item(ids, sub)?;
                    d = Some(all);
                    break;
                }
            }
        }
    }
    let d = match d {
        Some(d) => d,
        None => distances(&p, x, y, None)?,
    };
    v.setattr("_gedge_dist_cache", (key, &d, p))?;
    Ok(d)
}

pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(ledge_screen, m)?)?;
    m.add_function(wrap_pyfunction!(gedge_screen, m)?)?;
    m.add_function(wrap_pyfunction!(gedge_dist, m)?)?;
    Ok(())
}
