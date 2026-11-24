// SPDX-License-Identifier: GPL-3.0-or-later
//! Stable subdivision of baked triangles for ray queries.
use crate::document_host::*;
use glam::DVec3;
use numpy::{AllowTypeChange, IntoPyArray, PyArrayLike2};
use pyo3::prelude::*;
use pyo3::types::PyList;
mod warm;

struct Triangle {
    centre: DVec3,
    lo: DVec3,
    hi: DVec3,
}
type Cell = (((f64, f64, f64), (f64, f64, f64)), usize, usize);

fn subdivide(
    tris: &[Triangle],
    order: &mut [usize],
    offset: usize,
    depth: usize,
    target: usize,
    single: bool,
    cells: &mut Vec<Cell>,
) {
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = -lo;
    for &i in order.iter() {
        lo = lo.min(tris[i].centre);
        hi = hi.max(tris[i].centre);
    }
    // Centroid extents, weights and the subtraction from the cell origin
    // retain the input precision. Dividing the int64 bin counts by their
    // widths promotes the final scale to f64 in both cases.
    let bins = (order.len() as f64 / target as f64).powf(1. / 3.);
    let (span, counts) = if single {
        let extent = (hi.as_vec3() - lo.as_vec3()).max(glam::Vec3::splat(1e-9));
        let log_mean = (extent.x.ln() + extent.y.ln() + extent.z.ln()) / 3.;
        (
            extent.as_dvec3(),
            (extent / log_mean.exp() * bins as f32).as_dvec3(),
        )
    } else {
        let extent = (hi - lo).max(DVec3::splat(1e-9));
        let log_mean = (extent.x.ln() + extent.y.ln() + extent.z.ln()) / 3.;
        (extent, extent / log_mean.exp() * bins)
    };
    let n = counts
        .to_array()
        .map(|v| v.round_ties_even().max(1.) as i64);
    let scale = DVec3::from_array(n.map(|i| i as f64)) / span;
    let key = |i: usize| {
        let delta = if single {
            (tris[i].centre.as_vec3() - lo.as_vec3()).as_dvec3()
        } else {
            tris[i].centre - lo
        };
        let c = (delta * scale).to_array();
        let at = |axis: usize| (c[axis] as i64).min(n[axis] - 1);
        at(0)
            .wrapping_mul(n[1])
            .wrapping_add(at(1))
            .wrapping_mul(n[2])
            .wrapping_add(at(2))
    };
    order.sort_by_cached_key(|&i| key(i));
    let mut start = 0;
    while start < order.len() {
        let k = key(order[start]);
        let mut end = start + 1;
        while end < order.len() && key(order[end]) == k {
            end += 1;
        }
        let part = &mut order[start..end];
        if part.len() > 2 * target && depth < 3 {
            subdivide(tris, part, offset + start, depth + 1, target, single, cells);
        } else {
            let mut lo = DVec3::splat(f64::INFINITY);
            let mut hi = -lo;
            for &i in part.iter() {
                lo = lo.min(tris[i].lo);
                hi = hi.max(tris[i].hi);
            }
            cells.push((
                ((lo.x, lo.y, lo.z), (hi.x, hi.y, hi.z)),
                offset + start,
                part.len(),
            ));
        }
        start = end;
    }
}

#[pyfunction]
fn split_chunk_spans<'py>(chunk: &Obj<'py>, depth: usize, force: bool) -> PyResult<Obj<'py>> {
    let py = chunk.py();
    let h = host(py)?;
    let v = chunk.get_item("v0")?;
    let e = chunk.get_item("e1")?;
    let f = chunk.get_item("e2")?;
    let ids = chunk.get_item("tri_ent")?;
    let n = v.len()?;
    if h.getattr("_NO_SPAN_GRID")?.is_truthy()?
        || (!force && n <= h.getattr("_SPAN_GRID_MIN")?.extract()?)
    {
        return Ok((v, e, f, ids, py.None()).into_pyobject(py)?.into_any());
    }
    let key = (chunk.call_method1("get", ("rev",))?, n).into_pyobject(py)?;
    if !force {
        let old = chunk.call_method1("get", ("_span_grid",))?;
        if !old.is_none() && old.get_item(0)?.eq(&key)? {
            return old.get_item(1);
        }
    }
    let target = h.getattr("_SPAN_CELL_TRIS")?.extract::<usize>()?.max(1);
    // Form endpoints and centroids in the input dtype before comparing in f64.
    let centre = v.add(e.add(&f)?.div(3.)?)?;
    let single = centre.getattr("dtype")?.getattr("name")?.eq("float32")?;
    let a = v.add(&e)?;
    let b = v.add(&f)?;
    let arrays = [&centre, &v, &a, &b].map(|a| a.extract::<PyArrayLike2<f64, AllowTypeChange>>());
    let [centre, v0, a, b] = arrays;
    let (centre, v0, a, b) = (centre?, v0?, a?, b?);
    let views = [centre.as_array(), v0.as_array(), a.as_array(), b.as_array()];
    if views.iter().any(|a| a.shape() != [n, 3]) {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "expected N by 3 triangle coordinates",
        ));
    }
    let tris = (0..n)
        .map(|i| {
            let [c, p, q, r] = views
                .each_ref()
                .map(|a| DVec3::new(a[[i, 0]], a[[i, 1]], a[[i, 2]]));
            Triangle {
                centre: c,
                lo: p.min(q).min(r),
                hi: p.max(q).max(r),
            }
        })
        .collect::<Vec<_>>();
    let (order, cells) = py.detach(move || {
        let mut order = (0..n).collect::<Vec<_>>();
        let mut cells = Vec::new();
        subdivide(&tris, &mut order, 0, depth, target, single, &mut cells);
        (order, cells)
    });
    let order = order.into_pyarray(py);
    let out = (
        v.get_item(&order)?,
        e.get_item(&order)?,
        f.get_item(&order)?,
        ids.get_item(&order)?,
        PyList::new(py, cells)?,
    )
        .into_pyobject(py)?;
    if !force {
        chunk.set_item("_span_grid", (key, &out))?;
    }
    Ok(out.into_any())
}

#[pyfunction]
fn cat_kept<'py>(vp: &Obj<'py>, slot: &Obj<'py>, parts: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = vp.py();
    let mut memo = opt(vp, "_pick_cat_memo")?;
    if memo.is_none() {
        memo = pyo3::types::PyDict::new(py).into_any();
        if let Err(err) = vp.setattr("_pick_cat_memo", &memo) {
            if !err.is_instance_of::<pyo3::exceptions::PyAttributeError>(py) {
                return Err(err);
            }
        }
    }
    let hit = memo.call_method1("get", (slot,))?;
    if !hit.is_none() {
        let old = hit.get_item(0)?;
        if old.len()? == parts.len()? {
            let mut same = true;
            for (a, b) in old.try_iter()?.zip(parts.try_iter()?) {
                if !a?.is(&b?) {
                    same = false;
                    break;
                }
            }
            if same {
                return hit.get_item(1);
            }
        }
    }
    let out = if parts.len()? == 1 {
        parts.get_item(0)?
    } else {
        py.import("numpy")?.call_method1("concatenate", (parts,))?
    };
    memo.set_item(
        slot,
        (
            PyList::new(py, parts.try_iter()?.collect::<PyResult<Vec<_>>>()?)?,
            &out,
        ),
    )?;
    Ok(out)
}
#[pyfunction]
fn offset_kept<'py>(vp: &Obj<'py>, te: &Obj<'py>, offset: i64) -> PyResult<Obj<'py>> {
    if offset == 0 {
        return Ok(te.clone());
    }
    let memo = opt(vp, "_pick_te_memo")?;
    if !memo.is_none() && memo.get_item(0)?.is(te) && memo.get_item(1)?.eq(offset)? {
        return memo.get_item(2);
    }
    let out = te.add(offset)?;
    if let Err(err) = vp.setattr("_pick_te_memo", (te, offset, &out)) {
        if !err.is_instance_of::<pyo3::exceptions::PyAttributeError>(vp.py()) {
            return Err(err);
        }
    }
    Ok(out)
}
pub(super) fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(split_chunk_spans, m)?)?;
    m.add_function(wrap_pyfunction!(cat_kept, m)?)?;
    m.add_function(wrap_pyfunction!(offset_kept, m)?)?;

    warm::register(m)
}
