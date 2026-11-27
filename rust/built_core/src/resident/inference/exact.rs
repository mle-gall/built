// SPDX-License-Identifier: GPL-3.0-or-later
//! Refine a winning resident sample against the immutable support snapshot.
use crate::{
    math::{xyz, Vec3},
    pick::Projector,
    store::native::NativePatch,
};
use glam::{DMat4, DVec3};
use pyo3::{prelude::*, types::PyDict};
use std::sync::Arc;
type Obj<'p> = Bound<'p, PyAny>;
#[pyclass(frozen)]
struct Prototype {
    patch: Arc<NativePatch>,
    segments: Vec<(usize, f64, f64)>,
}
pub(super) fn remember(packed: &Obj<'_>, code: usize, patch: Arc<NativePatch>) -> PyResult<()> {
    let cache = match packed.getattr_opt("_exact_prototypes")? {
        Some(c) => c.cast_into::<PyDict>()?,
        None => {
            let c = PyDict::new(packed.py());
            packed.setattr("_exact_prototypes", &c)?;
            c
        }
    };
    let mut segments = Vec::new();
    for (&e, points) in &patch.mesh().expect("admitted exact patch").edge_samples {
        let n = points.len() - 1;
        segments
            .extend((0..n).map(|i| (e as usize, i as f64 / n as f64, (i + 1) as f64 / n as f64)));
    }
    cache.set_item(
        code,
        Bound::new(packed.py(), Prototype { patch, segments })?,
    )
}
pub(super) fn forget(packed: &Obj<'_>, code: usize) -> PyResult<()> {
    if let Some(c) = packed.getattr_opt("_exact_prototypes")? {
        c.call_method1("pop", (code, packed.py().None()))?;
    }
    Ok(())
}
fn placement<'p>(packed: &Obj<'p>, row: usize) -> PyResult<Option<(Bound<'p, Prototype>, DMat4)>> {
    let Some(cache) = packed.getattr_opt("_exact_prototypes")? else {
        return Ok(None);
    };
    let inst = packed.getattr("instances")?;
    let code: usize = inst.get_item("info")?.get_item((row, 0))?.extract()?;
    let Some(proto) = cache.cast::<PyDict>()?.get_item(code)? else {
        return Ok(None);
    };
    let world = inst
        .get_item("world")?
        .get_item(row)?
        .call_method1("reshape", (16,))?
        .extract::<[f64; 16]>()?;
    let origin = packed
        .getattr("prototype_origins")?
        .get_item(code)?
        .extract::<[f64; 3]>()?;
    let world = DMat4::from_cols_array(&world) * DMat4::from_translation(-DVec3::from(origin));
    Ok(Some((proto.cast_into::<Prototype>()?, world)))
}
pub(super) fn surface_distance(
    packed: &Obj<'_>,
    row: usize,
    origin: DVec3,
    dir: DVec3,
    broad: (f64, Option<[f64; 3]>),
    limit: f64,
) -> PyResult<Option<(f64, Option<[f64; 3]>)>> {
    let Some((proto, world)) = placement(packed, row)? else {
        return Ok(Some(broad));
    };
    let inv = world.inverse();
    if !inv.is_finite() {
        return Ok(None);
    }
    Ok(proto
        .get()
        .patch
        .ray(
            inv.transform_point3(origin),
            inv.transform_vector3(dir),
            None,
        )
        .filter(|(_, t, _)| *t <= limit)
        .map(|(_, t, n)| {
            (
                t,
                Some(
                    inv.transpose()
                        .transform_vector3(n)
                        .normalize_or_zero()
                        .to_array(),
                ),
            )
        }))
}
pub(super) fn refine_edge(
    packed: &Obj<'_>,
    cols: &Obj<'_>,
    result: &Obj<'_>,
    camera: &Projector,
    x: f64,
    y: f64,
) -> PyResult<()> {
    if !packed.hasattr("_exact_prototypes")?
        || !matches!(
            result.getattr("kind")?.extract::<String>()?.as_str(),
            "endpoint" | "midpoint" | "on_edge"
        )
    {
        return Ok(());
    }
    let p = xyz(&result.getattr("point")?)?;
    let rows = cols.get_item(0)?;
    let ids = cols.get_item(1)?;
    let (starts, ends) = (cols.get_item(2)?, cols.get_item(3)?);
    for i in 0..rows.len()? {
        let a = DVec3::from(starts.get_item(i)?.extract::<[f64; 3]>()?);
        let b = DVec3::from(ends.get_item(i)?.extract::<[f64; 3]>()?);
        let ab = b - a;
        let t = ((p - a).dot(ab) / ab.length_squared()).clamp(0., 1.);
        if !t.is_finite() || p.distance(a + t * ab) > 1e-8 * (1. + p.length()) {
            continue;
        }
        let row = rows.get_item(i)?.extract()?;
        let Some((proto, world)) = placement(packed, row)? else {
            continue;
        };
        let proto = proto.get();
        let Some(&(edge, lo, hi)) = proto.segments.get(ids.get_item(i)?.extract::<usize>()?) else {
            continue;
        };
        let cost = |p| {
            let (a, b, front) = camera.px(world.transform_point3(p).to_array());
            if front {
                (a - x).hypot(b - y)
            } else {
                f64::INFINITY
            }
        };
        let Some((_, u)) = proto.patch.closest_screen(edge, lo, hi, cost) else {
            continue;
        };
        let Some(point) = proto.patch.edge_point(edge, u) else {
            continue;
        };
        result.setattr("point", Vec3::from(world.transform_point3(point)))?;
        let endpoint = u == 0. || u == 1.;
        result.setattr("kind", if endpoint { "endpoint" } else { "on_edge" })?;
        result.setattr(
            "color",
            result
                .py()
                .import("built_core.snap")?
                .getattr(if endpoint {
                    "COLOR_ENDPOINT"
                } else {
                    "COLOR_ON_EDGE"
                })?,
        )?;
        result.setattr("label", result.py().None())?;
        let mesh = packed.getattr("groups")?.get_item(row)?.getattr("mesh")?;
        result.setattr("edge", mesh.getattr("edges")?.get_item(edge)?)?;
        break;
    }
    Ok(())
}
