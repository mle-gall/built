// SPDX-License-Identifier: GPL-3.0-or-later
//! Coordinate and key adapters shared by the push prefilters and drawing scans.
use crate::math::xyz;
use glam::DVec3;
use pyo3::{
    prelude::*,
    types::{PyDict, PySet, PyTuple},
};
type Obj<'py> = Bound<'py, PyAny>;
type P = (f64, f64, f64);
type Bounds = (P, P);
fn tuple(v: DVec3) -> P {
    (v.x, v.y, v.z)
}
// Scan bounds usually receive tuples from the store. Skip Python attribute
// lookup on these coordinates, retaining support for public vector types.
fn bound_point(p: &Obj<'_>) -> PyResult<DVec3> {
    if let Ok(row) = p.cast_exact::<PyTuple>() {
        let (x, y, z) = row.extract::<P>()?;
        return Ok(DVec3::new(x, y, z));
    }
    xyz(p)
}

struct Extent {
    lo: DVec3,
    hi: DVec3,
}
impl Extent {
    fn read(points: &Obj<'_>) -> PyResult<Option<Self>> {
        let mut it = points.try_iter()?;
        let Some(p) = it.next() else { return Ok(None) };
        let p = bound_point(&p?)?;
        let mut bounds = Self { lo: p, hi: p };
        for v in it {
            let v = bound_point(&v?)?;
            bounds.lo = bounds.lo.min(v);
            bounds.hi = bounds.hi.max(v);
        }
        Ok(Some(bounds))
    }
    fn bounds(&self, pad: f64, relative: bool) -> Bounds {
        let pad = if relative {
            pad + 1e-6 * self.lo.abs().max(self.hi.abs()).max_element()
        } else {
            pad
        };
        (
            tuple(self.lo - DVec3::splat(pad)),
            tuple(self.hi + DVec3::splat(pad)),
        )
    }
}
#[pyfunction]
#[pyo3(signature=(points,pad=0.0))]
fn raw_box(points: &Obj<'_>, pad: f64) -> PyResult<Bounds> {
    Ok(Extent::read(points)?
        .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("min() arg is an empty sequence"))?
        .bounds(pad, false))
}
#[pyfunction]
#[pyo3(signature=(points,pad=0.0))]
pub(super) fn box_of(points: &Obj<'_>, pad: f64) -> PyResult<Option<Bounds>> {
    Ok(Extent::read(points)?.map(|b| b.bounds(pad, true)))
}
#[pyfunction]
pub(super) fn grown_box(pts: &Obj<'_>) -> PyResult<Bounds> {
    Ok(Extent::read(pts)?
        .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("max() arg is an empty sequence"))?
        .bounds(2e-4, true))
}
#[pyfunction]
pub(super) fn key_boxes(points: &Obj<'_>) -> PyResult<Vec<[f64; 6]>> {
    points
        .try_iter()?
        .map(|p| {
            let v = xyz(&p?)?;
            let (a, b) = Extent { lo: v, hi: v }.bounds(2e-4, true);
            Ok([a.0, a.1, a.2, b.0, b.1, b.2])
        })
        .collect()
}
#[pyfunction]
pub(super) fn bounds_xy(points: Vec<[f64; 2]>) -> PyResult<((f64, f64), (f64, f64))> {
    let mut it = points.into_iter();
    let [x, y] = it
        .next()
        .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("empty plane bounds"))?;
    Ok(it.fold(((x, y), (x, y)), |((a, b), (c, d)), [u, v]| {
        ((a.min(u), b.min(v)), (c.max(u), d.max(v)))
    }))
}
#[pyfunction]
pub(super) fn positions(points: &Obj<'_>) -> PyResult<Vec<P>> {
    points.try_iter()?.map(|p| Ok(tuple(xyz(&p?)?))).collect()
}
#[pyfunction]
fn ring_positions(rings: &Obj<'_>) -> PyResult<Vec<Vec<P>>> {
    rings.try_iter()?.map(|r| positions(&r?)).collect()
}
#[pyfunction]
pub(super) fn views<'py>(
    mesh: &Obj<'py>,
    cls: &Obj<'py>,
    keys: &Obj<'py>,
) -> PyResult<Vec<Obj<'py>>> {
    keys.try_iter()?.map(|k| cls.call1((mesh, k?))).collect()
}
#[pyfunction]
pub(super) fn set_minus<'py>(
    mesh: &Obj<'py>,
    types: &Bound<'py, PyDict>,
    kind: &str,
    after_keys: &Obj<'py>,
    before: &Obj<'py>,
) -> PyResult<Option<Bound<'py, PySet>>> {
    let py = mesh.py();
    let keys = py
        .get_type::<PySet>()
        .call1((after_keys,))?
        .cast_into::<PySet>()?;
    if keys.len() >> 2 > before.len()? {
        return Ok(None);
    }
    let cls = types.as_any().get_item(kind)?;
    let out = PySet::empty(py)?;
    for k in keys.iter() {
        if !before.contains(&k)? {
            out.add(cls.call1((mesh, k))?)?;
        }
    }
    Ok(Some(out))
}
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let s = PyModule::new(m.py(), "push_queries")?;
    s.add_function(wrap_pyfunction!(raw_box, &s)?)?;
    s.add_function(wrap_pyfunction!(box_of, &s)?)?;
    s.add_function(wrap_pyfunction!(grown_box, &s)?)?;
    s.add_function(wrap_pyfunction!(key_boxes, &s)?)?;
    s.add_function(wrap_pyfunction!(bounds_xy, &s)?)?;
    s.add_function(wrap_pyfunction!(positions, &s)?)?;
    s.add_function(wrap_pyfunction!(ring_positions, &s)?)?;
    s.add_function(wrap_pyfunction!(views, &s)?)?;
    s.add_function(wrap_pyfunction!(set_minus, &s)?)?;
    m.add_submodule(&s)
}
