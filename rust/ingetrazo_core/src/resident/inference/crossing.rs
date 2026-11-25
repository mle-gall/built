// SPDX-License-Identifier: GPL-3.0-or-later
//! Crossings of two visible faces, verified by a pick on either side.
use super::context::{interaction, matrix, near_rows, snap_context, Obj};
use crate::math::{xyz, Vec3};
use glam::DVec3;
use numpy::{PyReadonlyArray1, PyReadonlyArray2};
use pyo3::prelude::*;

type Plane = (DVec3, DVec3);
type Hit = (DVec3, DVec3, i64);

fn front(
    index: &Obj<'_>,
    view: &Obj<'_>,
    x: f64,
    y: f64,
    rows: Option<&Obj<'_>>,
    mvp: &Obj<'_>,
) -> PyResult<Option<Hit>> {
    let ray = view.call_method1("_pixel_to_ray", (x, y))?;
    let (o, d) = (ray.get_item(0)?, ray.get_item(1)?);
    if o.is_none() || d.is_none() {
        return Ok(None);
    }
    let near = near_rows(index, view, mvp, x, y, 1., rows)?;
    if near.is_empty()? {
        return Ok(None);
    }
    let (o, d) = (xyz(&o)?, xyz(&d)?);
    let (pl, h) =
        super::picking::hit_rows(index, &near, o.to_array(), d.to_array(), f64::INFINITY)?;
    let Some((k, t, Some(n))) = h else {
        return Ok(None);
    };
    Ok(Some((
        o + d * t,
        DVec3::from_array(n),
        pl.get_item(k)?.extract()?,
    )))
}
#[pyfunction(signature=(index,view,x,y,rows,mvp))]
fn front_at(
    index: &Obj<'_>,
    view: &Obj<'_>,
    x: f64,
    y: f64,
    rows: Option<&Obj<'_>>,
    mvp: &Obj<'_>,
) -> PyResult<Option<(Vec3, Vec3, i64)>> {
    Ok(front(index, view, x, y, rows, mvp)?.map(|(p, n, r)| (p.into(), n.into(), r)))
}
fn on_plane(hit: Hit, plane: Plane, depth: f64) -> bool {
    hit.1.dot(plane.1).abs() > 0.999 && (hit.0 - plane.0).dot(plane.1).abs() <= 1e-6 + 1e-5 * depth
}
#[pyfunction(signature=(index,view,x,y,row,point,normal,rows,project,context))]
pub(super) fn face_crossing<'p>(
    index: &Obj<'p>,
    view: &Obj<'p>,
    x: f64,
    y: f64,
    row: &Obj<'p>,
    point: &Obj<'p>,
    normal: &Obj<'p>,
    rows: Option<&Obj<'p>>,
    project: &Obj<'p>,
    context: &Obj<'p>,
) -> PyResult<Option<(Obj<'p>, Option<&'static str>)>> {
    let py = view.py();
    let doc = py.import("views.document_interaction")?;
    if !doc
        .call_method1("_active_cut", (view.getattr("scene")?,))?
        .is_none()
    {
        return Ok(None);
    }
    let ray = view.call_method1("_pixel_to_ray", (x, y))?;
    let (o, d) = (ray.get_item(0)?, ray.get_item(1)?);
    if o.is_none() || d.is_none() {
        return Ok(None);
    }
    let (eye, dir) = (xyz(&o)?, xyz(&d)?);
    let f = (xyz(point)?, xyz(normal)?.normalize_or_zero());
    let depth = (f.0 - eye).dot(dir);
    let interaction = interaction(py)?;
    let ring: f64 = interaction.getattr("_CROSSING_RING_PX")?.extract()?;
    let ray2 = view.call_method1("_pixel_to_ray", (x + ring, y))?;
    let (o2, d2) = (ray2.get_item(0)?, ray2.get_item(1)?);
    if o2.is_none() || d2.is_none() {
        return Ok(None);
    }
    let reach = (xyz(&o2)? + xyz(&d2)? * depth - (eye + dir * depth)).length() * 50.;
    let mvp = matrix(view)?;
    let near = near_rows(index, view, &mvp, x, y, 1., rows)?;
    if near.is_empty()? {
        return Ok(None);
    }
    let parts = index.call_method1(
        "ray_cands",
        (
            near,
            o.call_method0("toTuple")?,
            d.call_method0("toTuple")?,
            depth + reach,
        ),
    )?;
    let col =
        |i: usize| -> PyResult<_> { Ok(parts.get_item(i)?.extract::<PyReadonlyArray2<f64>>()?) };
    let (v, a, b) = (col(2)?, col(3)?, col(4)?);
    let (v, a, b) = (v.as_array(), a.as_array(), b.as_array());
    let pl = parts.get_item(0)?.extract::<PyReadonlyArray1<i64>>()?;
    let pl = pl.as_array();
    let slack = 1e-6 + 1e-5 * depth;
    let mut hits = Vec::new();
    for (i, ((v, a), b)) in v.rows().into_iter().zip(a.rows()).zip(b.rows()).enumerate() {
        let vec = |r: numpy::ndarray::ArrayView1<f64>| DVec3::new(r[0], r[1], r[2]);
        let (p, a, b) = (vec(v), vec(a), vec(b));
        if let Some(t) = crate::pick::ray_triangle(
            eye.to_array(),
            dir.to_array(),
            p.to_array(),
            a.to_array(),
            b.to_array(),
        ) {
            if t >= depth - slack && t <= depth + reach {
                hits.push((t, p, a.cross(b), pl[i]));
            }
        }
    }
    hits.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut planes: Vec<(Plane, i64)> = Vec::new();
    for (_, p, n, r) in hits {
        let len = n.length();
        if len < 1e-30 {
            continue;
        }
        let n = n / len;
        if f.1.cross(n).length() < 0.02
            || planes
                .iter()
                .any(|&((q, m), _)| n.dot(m).abs() > 0.999 && (p - q).dot(m).abs() <= slack)
        {
            continue;
        }
        planes.push(((p, n), r));
        if planes.len() == 7 {
            break;
        }
    }
    let reach_px = view
        .getattr_opt("edge_snap_threshold_px")?
        .filter(|v| v.is_truthy().unwrap_or(false))
        .map(|v| v.extract::<f64>())
        .transpose()?
        .unwrap_or(14.)
        .max(ring);
    for (g, r) in planes {
        let found = py.import("core.snap")?.call_method1(
            "face_crossing_near_ray",
            (
                x,
                y,
                &o,
                &d,
                (point, Vec3::from(f.1)),
                (Vec3::from(g.0), Vec3::from(g.1)),
                project,
                reach_px,
                interaction.getattr("_CROSSING_SIDE_PX")?,
            ),
        )?;
        if found.is_none() {
            continue;
        }
        let crossing = found.get_item(0)?;
        let depth = (xyz(&crossing)? - eye).length();
        let mut sides = Vec::new();
        for q in found.get_item(1)?.try_iter()? {
            let q = q?;
            let Some(hit) = front(
                index,
                view,
                q.get_item(0)?.extract()?,
                q.get_item(1)?.extract()?,
                rows,
                &mvp,
            )?
            else {
                break;
            };
            if on_plane(hit, f, depth) {
                sides.push(0);
            } else if on_plane(hit, g, depth) {
                sides.push(1);
            } else {
                break;
            }
        }
        if sides != [0, 1] && sides != [1, 0] {
            continue;
        }
        let a = snap_context(view, row, context)?;
        let b = snap_context(view, r.into_pyobject(py)?.as_any(), context)?;
        let word = if [a, b].contains(&Some("outside")) {
            Some("outside")
        } else {
            a.or(b)
        };
        return Ok(Some((crossing, word)));
    }
    Ok(None)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(front_at, m)?)?;
    m.add_function(wrap_pyfunction!(face_crossing, m)?)
}
