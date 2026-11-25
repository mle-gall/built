// SPDX-License-Identifier: GPL-3.0-or-later
//! Ray traversal over fixed index ranges and per-placement triangle blocks.
use super::*;
use crate::pick::{ray_triangle, P};
use numpy::{PyReadonlyArray1, PyReadonlyArray2};
use pyo3::exceptions::PyValueError;

type Bounds = (P, P);

#[pyfunction]
pub fn ray_aabb_span(o: P, d: P, lo: P, hi: P) -> Option<(f64, f64)> {
    let (mut near, mut far) = (0.0f64, f64::INFINITY);
    for k in 0..3 {
        if d[k].abs() < 1e-12 {
            if o[k] < lo[k] - 1e-9 || o[k] > hi[k] + 1e-9 {
                return None;
            }
        } else {
            let a = (lo[k] - o[k]) / d[k];
            let b = (hi[k] - o[k]) / d[k];
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if near > far {
                return None;
            }
        }
    }
    Some((near, far))
}

fn intersects((lo, hi): Bounds, o: P, d: P, reach: f64, batched: bool) -> bool {
    let mut near = 0.0f64;
    let mut far = reach;
    for k in 0..3 {
        let inv = if batched && d[k].abs() <= 1e-12 {
            1e12
        } else if !batched && d[k].abs() < 1e-12 {
            if o[k] < lo[k] - 1e-9 || o[k] > hi[k] + 1e-9 {
                return false;
            }
            continue;
        } else {
            1. / d[k]
        };
        let a = (lo[k] - o[k]) * inv;
        let b = (hi[k] - o[k]) * inv;
        near = near.max(a.min(b));
        far = far.min(a.max(b));
    }
    near <= far
}

struct Triangles<'py> {
    points: [PyReadonlyArray2<'py, f64>; 3],
    entities: PyReadonlyArray1<'py, i64>,
    offset: usize,
}

impl<'py> Triangles<'py> {
    fn read(source: [Obj<'py>; 4], offset: usize) -> PyResult<Self> {
        let [v0, e1, e2, entities] = source;
        let np = v0.py().import("numpy")?;
        let vector = |col: Obj<'py>| -> PyResult<PyReadonlyArray2<'py, f64>> {
            match col.extract() {
                Ok(rows) => Ok(rows),
                Err(_) => Ok(np.call_method1("asarray", (col, "float64"))?.extract()?),
            }
        };
        let entities: PyReadonlyArray1<'py, i64> = match entities.extract() {
            Ok(ids) => ids,
            Err(_) => np.call_method1("asarray", (entities, "int64"))?.extract()?,
        };
        let points = [vector(v0)?, vector(e1)?, vector(e2)?];
        let n = entities.as_array().len();
        if points.iter().any(|a| a.as_array().dim() != (n, 3)) {
            return Err(PyValueError::new_err(
                "pick triangle columns must have shape (N, 3)",
            ));
        }
        Ok(Self {
            points,
            entities,
            offset,
        })
    }

    fn visit(&self, rows: impl IntoIterator<Item = usize>, ray: &mut Query) -> PyResult<()> {
        let [a, b, c] = self.points.each_ref().map(|v| v.as_array());
        let ids = self.entities.as_array();
        for row in rows {
            let id = *ids
                .get(row)
                .ok_or_else(|| PyValueError::new_err("pick triangle row outside the index"))?;
            let index = (id as usize)
                .checked_add(self.offset)
                .filter(|i| *i < ray.mask.len())
                .ok_or_else(|| PyValueError::new_err("invalid pick entity offset"))?;
            if !ray.mask[index] {
                continue;
            }
            let point =
                |v: &numpy::ndarray::ArrayView2<f64>| [v[[row, 0]], v[[row, 1]], v[[row, 2]]];
            if let Some(t) =
                ray_triangle(ray.origin, ray.direction, point(&a), point(&b), point(&c))
            {
                if let Some(ts) = &mut ray.times {
                    let slot = ts
                        .get_mut(index)
                        .ok_or_else(|| PyValueError::new_err("missing pick entity"))?;
                    *slot = slot.min(t);
                } else {
                    ray.best = ray.best.min(t);
                }
            }
        }
        Ok(())
    }
}

struct Query<'a> {
    origin: P,
    direction: P,
    mask: numpy::ndarray::ArrayView1<'a, bool>,
    times: Option<Vec<f64>>,
    best: f64,
}

#[pyclass]
struct SpanPlan {
    // Bounding boxes and local spans stay immutable for this index's lifetime.
    spans: Vec<(Option<Bounds>, usize, usize)>,
    own_boxes: Vec<Option<Bounds>>,
    static_ref: Py<PyAny>,
    own_ref: Py<PyAny>,
    tri_ref: Py<PyAny>,
}

impl SpanPlan {
    fn get<'py>(
        idx: &Obj<'py>,
        tri: &Obj<'py>,
        own: &Obj<'py>,
        use_own: bool,
    ) -> PyResult<Bound<'py, Self>> {
        let py = idx.py();
        let ranges = attr(idx, "tri_spans")?;
        let key = if use_own {
            "_native_all_spans"
        } else {
            "_native_fixed_spans"
        };
        if let Some(old) = idx.getattr_opt(key)? {
            let cache = old.cast_into::<Self>()?;
            let plan = cache.borrow();
            if plan.static_ref.bind(py).is(&ranges)
                && plan.own_ref.bind(py).is(own)
                && plan.tri_ref.bind(py).is(tri)
            {
                drop(plan);
                return Ok(cache);
            }
        }
        let spans = if tri.is_none() {
            Vec::new()
        } else if ranges.is_truthy()? {
            ranges.extract()?
        } else {
            vec![(None, 0, tri.len()?)]
        };
        let mut own_boxes = Vec::new();
        if !own.is_none() {
            for span in own.try_iter()? {
                own_boxes.push(span?.get_item(0)?.extract()?);
            }
        }
        let plan = Bound::new(
            py,
            Self {
                spans,
                own_boxes,
                static_ref: ranges.unbind(),
                own_ref: own.clone().unbind(),
                tri_ref: tri.clone().unbind(),
            },
        )?;
        idx.setattr(key, &plan)?;
        Ok(plan)
    }
}

#[pyfunction]
#[pyo3(signature=(view,idx,origin,direction,ent_mask,reduce_global,reach,own))]
pub fn index_ray_hits<'py>(
    view: &Obj<'py>,
    idx: &Obj<'py>,
    origin: &Obj<'py>,
    direction: &Obj<'py>,
    ent_mask: PyReadonlyArray1<'py, bool>,
    reduce_global: bool,
    reach: Option<f64>,
    own: bool,
) -> PyResult<Obj<'py>> {
    let py = view.py();
    let tri = idx.getattr("tri_v0")?;
    let local = if own {
        attr(idx, "own_spans")?
    } else {
        none(py)
    };
    let has_local = local.is_truthy()?;
    if tri.is_none() && !has_local {
        return Ok(none(py));
    }
    let origin = xyz(origin)?.to_array();
    let direction = xyz(direction)?.to_array();
    let fast =
        if reach.is_none() && !has_local && !tri.is_none() && view.hasattr("_static_occluders")? {
            let class = py
                .import("views.document_interaction")?
                .getattr("DocumentInteraction")?;
            class.call_method1(
                "_static_ray_rows",
                (view, idx, &*ent_mask, triple(origin), triple(direction)),
            )?
        } else {
            none(py)
        };
    let mut query = Query {
        origin,
        direction,
        mask: ent_mask.as_array(),
        times: if reduce_global {
            None
        } else {
            Some(vec![f64::INFINITY; idx.getattr("entities")?.len()?])
        },
        best: f64::INFINITY,
    };
    let plan = SpanPlan::get(idx, &tri, &local, own)?;
    let plan = plan.borrow();
    let batched = plan.spans.len() + plan.own_boxes.len() > 64;
    let limit = reach.unwrap_or(f64::INFINITY);
    if !tri.is_none() {
        let triangles = Triangles::read(
            [
                tri,
                idx.getattr("tri_e1")?,
                idx.getattr("tri_e2")?,
                idx.getattr("tri_ent")?,
            ],
            0,
        )?;
        if !fast.is_none() {
            let rows: PyReadonlyArray1<i64> = fast.extract()?;
            triangles.visit(rows.as_array().iter().map(|i| *i as usize), &mut query)?;
        } else {
            for &(bb, start, count) in &plan.spans {
                // Small static span ranges historically use unbounded box tests.
                if bb.is_none_or(|b| {
                    intersects(
                        b,
                        origin,
                        direction,
                        if batched { limit } else { f64::INFINITY },
                        batched,
                    )
                }) {
                    triangles.visit(
                        start
                            ..start
                                .checked_add(count)
                                .ok_or_else(|| PyValueError::new_err("invalid pick range"))?,
                        &mut query,
                    )?;
                }
            }
        }
    }
    for (i, &bb) in plan.own_boxes.iter().enumerate() {
        if bb.is_some_and(|b| !intersects(b, origin, direction, limit, batched)) {
            continue;
        }
        let span = local.get_item(i)?;
        let triangles = Triangles::read(
            [
                span.get_item(1)?,
                span.get_item(2)?,
                span.get_item(3)?,
                span.get_item(4)?,
            ],
            span.get_item(5)?.extract()?,
        )?;
        triangles.visit(0..triangles.entities.as_array().len(), &mut query)?;
    }
    Ok(match query.times {
        Some(ts) => ts.into_pyarray(py).into_any(),
        None => query.best.into_pyobject(py)?.into_any(),
    })
}
