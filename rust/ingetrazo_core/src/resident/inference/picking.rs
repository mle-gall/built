// SPDX-License-Identifier: GPL-3.0-or-later
//! Resolve surface and edit-context picks from resident triangles and edges.
use crate::math::Vec3;
use numpy::{IntoPyArray, PyReadonlyArray2, PyReadonlyArray3};
use pyo3::{exceptions::PyIndexError, prelude::*};

type Obj<'p> = Bound<'p, PyAny>;
type TriangleHit = (usize, f64, Option<[f64; 3]>);

fn hit_rows<'p>(
    index: &Obj<'p>,
    rows: &Obj<'p>,
    o: [f64; 3],
    d: [f64; 3],
    limit: f64,
) -> PyResult<(Obj<'p>, Option<TriangleHit>)> {
    let parts = index.call_method1(
        "ray_cands",
        (rows, (o[0], o[1], o[2]), (d[0], d[1], d[2]), limit),
    )?;
    let hit = super::nearest_inference_triangle(
        o,
        d,
        parts.get_item(2)?.extract()?,
        parts.get_item(3)?.extract()?,
        parts.get_item(4)?.extract()?,
    )?;
    Ok((parts.get_item(0)?, hit))
}

#[pyfunction(signature=(index,row,origin,direction,limit=f64::INFINITY))]
fn surface_hit<'p>(
    index: &Obj<'p>,
    row: &Obj<'p>,
    origin: &Obj<'p>,
    direction: &Obj<'p>,
    limit: f64,
) -> PyResult<Option<(Obj<'p>, Option<Vec3>)>> {
    let info = index
        .getattr("packed")?
        .getattr("instances")?
        .get_item("info")?
        .extract::<PyReadonlyArray2<u32>>()?;
    let info = info.as_array();
    let rows: Vec<i64> = if let Ok(r) = row.extract::<i64>() {
        vec![r]
    } else {
        index
            .py()
            .import("numpy")?
            .call_method1("asarray", (row, "int64"))?
            .call_method1("reshape", (-1,))?
            .extract()?
    };
    let rows: Vec<_> = rows
        .into_iter()
        .filter(|&r| r >= 0 && (r as usize) < info.nrows() && info[[r as usize, 2]] & 129 == 0)
        .collect();
    let (_, hit) = hit_rows(
        index,
        rows.into_pyarray(index.py()).as_any(),
        origin.call_method0("toTuple")?.extract()?,
        direction.call_method0("toTuple")?.extract()?,
        limit,
    )?;
    hit.map(|(_, t, n)| {
        Ok((
            origin.add(direction.mul(t)?)?,
            n.map(|n| Vec3::from(glam::DVec3::from_array(n))),
        ))
    })
    .transpose()
}
#[pyfunction(signature=(index,row,origin,direction,limit=f64::INFINITY))]
fn surface<'p>(
    index: &Obj<'p>,
    row: &Obj<'p>,
    origin: &Obj<'p>,
    direction: &Obj<'p>,
    limit: f64,
) -> PyResult<Option<Obj<'p>>> {
    Ok(surface_hit(index, row, origin, direction, limit)?.map(|h| h.0))
}

#[pyfunction(signature=(packed,row,origin,direction))]
fn packed_surface<'p>(
    packed: &Obj<'p>,
    row: Option<i64>,
    origin: &Obj<'p>,
    direction: &Obj<'p>,
) -> PyResult<Option<Obj<'p>>> {
    let Some(row) = row.and_then(|r| usize::try_from(r).ok()) else {
        return Ok(None);
    };
    let instances = packed.getattr("instances")?;
    let info = instances
        .get_item("info")?
        .extract::<PyReadonlyArray2<u32>>()?;
    let info = info.as_array();
    if row >= info.nrows() || info[[row, 2]] & 129 != 0 {
        return Ok(None);
    }
    let face = packed.getattr("commands")?.get_item(info[[row, 0]])?;
    let start: usize = face.get_item("base")?.extract()?;
    let count: usize = face.get_item("count")?.extract()?;
    if count < 3 {
        return Ok(None);
    }
    let world = instances
        .get_item("world")?
        .extract::<PyReadonlyArray3<f32>>()?;
    let world = world.as_array();
    let pos = packed
        .getattr("vertices")?
        .get_item("pos")?
        .extract::<PyReadonlyArray2<f32>>()?;
    let pos = pos.as_array();
    let end = start
        .checked_add(count - count % 3)
        .ok_or_else(|| PyIndexError::new_err(start))?
        .min(pos.nrows());
    let point = |i: usize| {
        glam::DVec3::from_array(std::array::from_fn(|j| {
            (pos[[i, 0]] as f64 * world[[row, 0, j]] as f64
                + pos[[i, 1]] as f64 * world[[row, 1, j]] as f64
                + pos[[i, 2]] as f64 * world[[row, 2, j]] as f64)
                + world[[row, 3, j]] as f64
        }))
    };
    let o = origin.call_method0("toTuple")?.extract()?;
    let d = direction.call_method0("toTuple")?.extract()?;
    let mut best = f64::INFINITY;
    for i in (start..end.saturating_sub(2)).step_by(3) {
        let a = point(i);
        if let Some(t) = crate::pick::ray_triangle(
            o,
            d,
            a.to_array(),
            (point(i + 1) - a).to_array(),
            (point(i + 2) - a).to_array(),
        ) {
            best = best.min(t);
        }
    }
    if best.is_finite() {
        Ok(Some(origin.add(direction.mul(best)?)?))
    } else {
        Ok(None)
    }
}

#[pyfunction]
fn context_hit<'p>(view: &Obj<'p>, context: &Obj<'p>, x: f64, y: f64) -> PyResult<Option<i64>> {
    let index = view.getattr("inference_index")?;
    let camera = view.getattr("camera")?;
    let mvp = camera
        .call_method0("projection_matrix")?
        .mul(camera.call_method0("view_matrix")?)?;
    let (w, h) = (
        view.call_method0("width")?.extract()?,
        view.call_method0("height")?.extract()?,
    );
    let r = view
        .getattr("tools")?
        .getattr_opt("pick_threshold_px")?
        .map(|v| v.extract())
        .transpose()?
        .unwrap_or(8.);
    let subset = view.call_method1("_rows_for_group", (context,))?;
    let rows = super::screen::screen_rows(
        &index,
        &view.getattr("packed")?,
        &mvp,
        w,
        h,
        x,
        y,
        r,
        Some(&subset),
    )?;
    let ray = view.call_method1("_pixel_to_ray", (x, y))?;
    let (placements, hit) = hit_rows(
        &index,
        &rows,
        ray.get_item(0)?.call_method0("toTuple")?.extract()?,
        ray.get_item(1)?.call_method0("toTuple")?.extract()?,
        f64::INFINITY,
    )?;
    if let Some((i, _, _)) = hit {
        return Ok(Some(placements.get_item(i)?.extract()?));
    }
    let edges = index
        .call_method1(
            "edges_near",
            (rows, mvp.call_method0("data")?, w, h, x, y, r, 1),
        )?
        .get_item(0)?;
    if edges.is_empty()? {
        Ok(None)
    } else {
        Ok(Some(edges.get_item(0)?.extract()?))
    }
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(context_hit, m)?)?;
    m.add_function(wrap_pyfunction!(surface, m)?)?;
    m.add_function(wrap_pyfunction!(surface_hit, m)?)?;
    m.add_function(wrap_pyfunction!(packed_surface, m)?)
}
