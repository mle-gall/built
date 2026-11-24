// SPDX-License-Identifier: GPL-3.0-or-later
//! Cursor intersections with polygons, callback-defined patches and other planes.
use crate::{
    math::{xyz, Vec3},
    object_outline::positions,
    polygon::raw_newell,
    snap_planes::plane_cut_ray,
    store::drawing_face::inside,
    tools::plane::{plane_axes, Axes},
};
use glam::DVec3;
use pyo3::prelude::*;
type Obj<'py> = Bound<'py, PyAny>;
type P = [f64; 3];
type XY = [f64; 2];
struct Polygon {
    origin: DVec3,
    u: DVec3,
    v: DVec3,
    outer: Vec<XY>,
    holes: Vec<Vec<XY>>,
}
impl Polygon {
    fn project(&self, p: DVec3) -> XY {
        let d = p - self.origin;
        [d.dot(self.u), d.dot(self.v)]
    }
    fn contains(&self, p: DVec3) -> bool {
        let p = self.project(p);
        inside(p, &self.outer, false) && !self.holes.iter().any(|h| inside(p, h, false))
    }
}
enum Surface<'py> {
    Patch(Obj<'py>),
    Polygon(Polygon),
}
#[pyfunction]
#[pyo3(signature=(outline,patch,scene,projector,cursor,et,is_occluded,query))]
fn pierce<'py>(
    outline: &Obj<'py>,
    patch: bool,
    scene: &Obj<'py>,
    projector: &Obj<'py>,
    cursor: XY,
    et: f64,
    is_occluded: Option<&Obj<'py>>,
    query: Option<&Obj<'py>>,
) -> PyResult<Option<(Obj<'py>, Obj<'py>)>> {
    let py = outline.py();
    let (origin, n, context, surface) = if patch {
        let n = outline.get_item(1)?;
        if n.is_none() {
            return Ok(None);
        }
        let n = xyz(&n)?;
        if n.length() < 1e-12 {
            return Ok(None);
        }
        (
            xyz(&outline.get_item(0)?)?,
            n.normalize_or_zero(),
            outline.get_item(3)?,
            Surface::Patch(outline.get_item(2)?),
        )
    } else {
        let outer = positions(&outline.get_item(0)?)?;
        if outer.len() < 3 {
            return Ok(None);
        }
        let outer = outer.into_iter().map(DVec3::from_array).collect::<Vec<_>>();
        let n = raw_newell(&outer);
        if n.length() < 1e-12 {
            return Ok(None);
        }
        let n = n.normalize_or_zero();
        let origin = outer[0];
        let (u, v) = plane_axes(n, &Axes::world());
        let mut poly = Polygon {
            origin,
            u,
            v,
            outer: Vec::new(),
            holes: Vec::new(),
        };
        poly.outer = outer.into_iter().map(|p| poly.project(p)).collect();
        for h in outline.get_item(1)?.try_iter()? {
            let h = h?;
            if h.len()? >= 3 {
                poly.holes.push(
                    positions(&h)?
                        .into_iter()
                        .map(|p| poly.project(p.into()))
                        .collect(),
                );
            }
        }
        (origin, n, outline.get_item(2)?, Surface::Polygon(poly))
    };
    let q = match query {
        Some(q) => q.clone(),
        None => py
            .import("core.snap_query")?
            .call_method1("make_query", (scene.getattr("edges")?, projector, cursor))?,
    };
    let hits = q
        .call_method1("plane_crossings", (et, origin.to_array(), n.to_array()))?
        .extract::<Vec<P>>()?;
    for p in hits {
        let p = DVec3::from_array(p);
        let hit = Vec3::from(p).into_pyobject(py)?.into_any();
        let contained = match &surface {
            Surface::Polygon(poly) => poly.contains(p),
            Surface::Patch(test) => test.call1((&hit,))?.is_truthy()?,
        };
        if contained
            && !is_occluded
                .map(|f| f.call1((&hit,))?.is_truthy())
                .transpose()?
                .unwrap_or(false)
        {
            return Ok(Some((hit, context)));
        }
    }
    Ok(None)
}
#[pyfunction]
#[pyo3(signature=(px,py,origin,direction,f_plane,g_plane,world_to_pixel,reach_px,side_px=3.0))]
fn face_crossing_near_ray<'a>(
    px: f64,
    py: f64,
    origin: &Obj<'a>,
    direction: &Obj<'a>,
    f_plane: &Obj<'a>,
    g_plane: &Obj<'a>,
    world_to_pixel: &Obj<'a>,
    reach_px: f64,
    side_px: f64,
) -> PyResult<Option<(Obj<'a>, ((f64, f64), (f64, f64)))>> {
    let plane = |p: &Obj| -> PyResult<(P, P)> {
        Ok((
            xyz(&p.get_item(0)?)?.to_array(),
            xyz(&p.get_item(1)?)?.to_array(),
        ))
    };
    let Some((p, sample)) = plane_cut_ray(
        xyz(origin)?.to_array(),
        xyz(direction)?.to_array(),
        plane(f_plane)?,
        plane(g_plane)?,
    ) else {
        return Ok(None);
    };
    let p = Vec3::from(DVec3::from_array(p))
        .into_pyobject(origin.py())?
        .into_any();
    let Some((x, y)) = world_to_pixel
        .call1((&p,))?
        .extract::<Option<(f64, f64)>>()?
    else {
        return Ok(None);
    };
    if (x - px).hypot(y - py) > reach_px {
        return Ok(None);
    }
    let Some((qx, qy)) = world_to_pixel
        .call1((Vec3::from(DVec3::from_array(sample)),))?
        .extract::<Option<(f64, f64)>>()?
    else {
        return Ok(None);
    };
    let (dx, dy) = (qx - x, qy - y);
    let l = dx.hypot(dy);
    if l < 1e-9 {
        return Ok(None);
    }
    let (nx, ny) = (-dy / l * side_px, dx / l * side_px);
    Ok(Some((p, ((x + nx, y + ny), (x - nx, y - ny)))))
}
#[pyfunction]
fn pairs_that_may_meet(edges: &Obj<'_>) -> PyResult<Vec<(usize, usize)>> {
    let n = edges.len()?;
    let mut boxes = Vec::new();
    if n >= 8 {
        for e in edges.try_iter()? {
            let e = e?;
            let (a, b) = (xyz(&e.getattr("a")?)?, xyz(&e.getattr("b")?)?);
            let pad = DVec3::splat(
                2.5e-4 + 2e-7 * (b - a).length() + 1e-6 * a.abs().max(b.abs()).max_element(),
            );
            boxes.push((a.min(b) - pad, a.max(b) + pad));
        }
    }
    let mut pairs = Vec::new();
    for i in 0..n {
        for j in i + 1..n {
            if n < 8 || !(boxes[i].0.cmpgt(boxes[j].1).any() || boxes[j].0.cmpgt(boxes[i].1).any())
            {
                pairs.push((i, j));
            }
        }
    }
    Ok(pairs)
}
#[pyfunction]
fn closest_segment(p: XY, a: XY, b: XY) -> (f64, f64) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l = dx * dx + dy * dy;
    let t = if l == 0. {
        0.
    } else {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l).clamp(0., 1.)
    };
    ((p[0] - (a[0] + t * dx)).hypot(p[1] - (a[1] + t * dy)), t)
}
#[pyfunction]
fn point_on_segment(p: &Obj<'_>, a: &Obj<'_>, b: &Obj<'_>, tol: f64) -> PyResult<bool> {
    let (p, a, b) = (xyz(p)?, xyz(a)?, xyz(b)?);
    let d = b - a;
    let l = d.dot(d);
    if l < 1e-12 {
        return Ok((p - a).length() < tol);
    }
    let t = (p - a).dot(d) / l;
    Ok((-1e-6..=1.000001).contains(&t) && (p - (a + d * t)).length() < tol)
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let s = PyModule::new(m.py(), "snap_surfaces")?;
    s.add_function(wrap_pyfunction!(pierce, &s)?)?;
    s.add_function(wrap_pyfunction!(face_crossing_near_ray, &s)?)?;
    s.add_function(wrap_pyfunction!(pairs_that_may_meet, &s)?)?;
    s.add_function(wrap_pyfunction!(closest_segment, &s)?)?;
    s.add_function(wrap_pyfunction!(point_on_segment, &s)?)?;
    m.add_submodule(&s)
}
