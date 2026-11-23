// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared plane grid, interior sampling and ordered nesting of planar cycles.
use crate::facegeom::py_sum;
use pyo3::{
    exceptions::{PyValueError, PyZeroDivisionError},
    prelude::*,
    types::{PyFloat, PyTuple},
};
use std::collections::HashSet;

pub(super) type Point = [f64; 2];
pub(super) type Ring = Vec<Point>;
pub(super) const TOL: f64 = 1e-4;

/// The rounded quotient is already binary64. Its ordered bits represent
/// every finite grid coordinate, including integral floats beyond i128.
#[derive(Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(super) struct Cell([u64; 2]);
impl Cell {
    pub(super) fn at(p: Point, tol: f64) -> Option<Self> {
        let q = p.map(|x| (x / tol).round_ties_even());
        if !q.iter().all(|v| v.is_finite()) {
            return None;
        }
        Some(Self(q.map(|v| {
            let b = if v == 0. { 0 } else { v.to_bits() };
            if b >> 63 != 0 {
                !b
            } else {
                b ^ (1 << 63)
            }
        })))
    }
    pub(super) fn point(self) -> Point {
        self.0
            .map(|b| f64::from_bits(if b >> 63 != 0 { b ^ (1 << 63) } else { !b }))
    }
    pub(super) fn tuple(self, py: Python<'_>) -> PyResult<Bound<'_, PyTuple>> {
        let ns = self.point().map(|x| -> PyResult<_> {
            if x.abs() < i64::MAX as f64 {
                Ok((x as i64).into_pyobject(py)?.into_any())
            } else {
                PyFloat::new(py, x).call_method0("__round__")
            }
        });
        let [a, b] = ns;
        PyTuple::new(py, [a?, b?])
    }
}
pub(super) fn edges(r: &[Point]) -> impl Iterator<Item = (Point, Point)> + '_ {
    r.iter().copied().zip(r.iter().copied().cycle().skip(1))
}
pub(super) fn inside(r: &[Point], p: Point) -> bool {
    // Same even/odd ray convention as the offset planner.
    super::offset::contains_edges(edges(r).map(|(a, b)| (a.into(), b.into())), p.into())
}
pub(super) fn area(r: &[Point]) -> f64 {
    edges(r).fold(0., |s, (a, b)| s + (a[0] * b[1] - b[0] * a[1])) * 0.5
}
pub(super) fn scan(outer: &[Point], holes: &[Ring]) -> Option<Point> {
    let rings = || std::iter::once(outer).chain(holes.iter().map(Vec::as_slice));
    let mut ys: Vec<_> = rings().flatten().map(|p| p[1]).collect();
    ys.sort_by(f64::total_cmp);
    ys.dedup();
    let mut best = None;
    for band in ys.windows(2) {
        let height = band[1] - band[0];
        if height < 1e-9 {
            continue;
        }
        let y = (band[0] + band[1]) / 2.;
        let mut xs: Vec<_> = rings()
            .flat_map(edges)
            .filter(|(a, b)| (a[1] > y) != (b[1] > y))
            .map(|(a, b)| a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]))
            .collect();
        xs.sort_by(f64::total_cmp);
        for pair in xs.chunks_exact(2) {
            let width = (pair[1] - pair[0]).min(height);
            if best.is_none_or(|(w, _)| width > w) {
                best = Some((width, [(pair[0] + pair[1]) / 2., y]));
            }
        }
    }
    best.map(|(_, p)| p)
}

pub(super) struct Metrics<'py> {
    hypot: Bound<'py, PyAny>,
    compensated: bool,
}
impl<'py> Metrics<'py> {
    pub(super) fn new(py: Python<'py>) -> PyResult<Self> {
        let v = py.version_info();
        Ok(Self {
            hypot: py.import("math")?.getattr("hypot")?,
            compensated: (v.major, v.minor) >= (3, 12),
        })
    }
    fn length(&self, x: f64, y: f64) -> PyResult<f64> {
        self.hypot.call1((x, y))?.extract()
    }
    pub(super) fn floor(&self, r: &[Point]) -> PyResult<f64> {
        let lengths = edges(r)
            .map(|(a, b)| self.length(b[0] - a[0], b[1] - a[1]))
            .collect::<PyResult<Vec<_>>>()?;
        Ok(TOL.min(TOL * py_sum(lengths.into_iter(), self.compensated) / 2.))
    }
    pub(super) fn interior(&self, r: &[Point]) -> PyResult<Point> {
        if r.is_empty() {
            return Err(PyZeroDivisionError::new_err("division by zero"));
        }
        let center = std::array::from_fn(|i| {
            py_sum(r.iter().map(|p| p[i]), self.compensated) / r.len() as f64
        });
        if inside(r, center) {
            return Ok(center);
        }
        let b = r
            .get(1)
            .ok_or_else(|| pyo3::exceptions::PyIndexError::new_err("list index out of range"))?;
        let p = std::array::from_fn(|i| ((r[0][i] + b[i]) / 2.) * 0.999 + center[i] * 0.001);
        Ok(if inside(r, p) {
            p
        } else {
            scan(r, &[]).unwrap_or(p)
        })
    }
}

#[pyfunction]
pub(crate) fn plane_region_point(
    py: Python<'_>,
    outer: Ring,
    holes: Vec<Ring>,
) -> PyResult<(f64, f64)> {
    let metrics = Metrics::new(py)?;
    let p = metrics.interior(&outer)?;
    let valid = |p| inside(&outer, p) && !holes.iter().any(|h| inside(h, p));
    if valid(p) {
        return Ok(p.into());
    }
    for (a, b) in edges(&outer) {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let n = metrics.length(dx, dy)?;
        if n < TOL {
            continue;
        }
        let q = [
            (a[0] + b[0]) / 2. - dy / n * 1e-3,
            (a[1] + b[1]) / 2. + dx / n * 1e-3,
        ];
        if valid(q) {
            return Ok(q.into());
        }
    }
    Ok(scan(&outer, &holes).unwrap_or(p).into())
}

pub(super) fn nesting(
    metrics: &Metrics<'_>,
    rings: &[Ring],
    arranged: bool,
) -> PyResult<Vec<(usize, Vec<usize>)>> {
    let mut faces = Vec::new();
    let mut holes = Vec::new();
    for (i, r) in rings.iter().enumerate() {
        let a = area(r);
        let floor = metrics.floor(r)?;
        if a > floor {
            faces.push((i, a));
        } else if a < -floor {
            holes.push((i, -a, floor));
        }
    }
    faces.sort_by(|a, b| a.1.total_cmp(&b.1));
    if arranged && !holes.is_empty() {
        holes.sort_by(|a, b| b.1.total_cmp(&a.1));
        holes.remove(0);
    }
    let bounds: Vec<_> = faces
        .iter()
        .map(|(i, _)| {
            rings[*i].iter().fold(
                (f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY),
                |(lo, hi, x), p| (lo.min(p[1]), hi.max(p[1]), x.max(p[0])),
            )
        })
        .collect();
    let mut result: Vec<_> = faces.iter().map(|(i, _)| (*i, Vec::new())).collect();
    for (h, a, floor) in holes {
        let p = metrics.interior(&rings[h])?;
        let start = if arranged {
            faces.partition_point(|f| f.1 <= a + floor)
        } else {
            0
        };
        let cells = |r: &Ring| -> PyResult<HashSet<Cell>> {
            r.iter()
                .map(|&p| {
                    Cell::at(p, TOL)
                        .ok_or_else(|| PyValueError::new_err("non-finite plane coordinate"))
                })
                .collect()
        };
        let mut hole_cells = None;
        for k in start..faces.len() {
            let (lo, hi, x) = bounds[k];
            if p[1] < lo || p[1] > hi || p[0] > x + 1e-9 * x.abs().max(1.) {
                continue;
            }
            let r = &rings[faces[k].0];
            if !arranged && r.len() == rings[h].len() {
                if hole_cells.is_none() {
                    hole_cells = Some(cells(&rings[h])?);
                }
                if hole_cells.as_ref() == Some(&cells(r)?) {
                    continue;
                }
            }
            if inside(r, p) {
                result[k].1.push(h);
                break;
            }
        }
    }
    Ok(result)
}

#[pyfunction]
pub(crate) fn plane_nesting(
    py: Python<'_>,
    rings: Vec<Ring>,
    arranged: bool,
) -> PyResult<Vec<(usize, Vec<usize>)>> {
    nesting(&Metrics::new(py)?, &rings, arranged)
}

#[pyfunction]
pub(crate) fn plane_interior_point(py: Python<'_>, ring: Ring) -> PyResult<(f64, f64)> {
    Ok(Metrics::new(py)?.interior(&ring)?.into())
}
#[pyfunction]
pub(crate) fn plane_scan_point(outer: Ring, holes: Vec<Ring>) -> Option<(f64, f64)> {
    scan(&outer, &holes).map(Into::into)
}

#[pyfunction]
pub(crate) fn trace_plane_outline<'py>(
    regions: &Bound<'py, PyAny>,
    creases: Vec<[Point; 2]>,
) -> PyResult<Vec<Vec<Bound<'py, PyAny>>>> {
    let mut source = std::collections::HashMap::new();
    let mut polygons = Vec::new();
    let mut ring = |r: Bound<'py, PyAny>| -> PyResult<Ring> {
        let mut points = Vec::new();
        let mut first = None;
        for pt in r.try_iter()? {
            let pt = pt?;
            let p: Point = pt.extract()?;
            let cell = Cell::at(p, TOL)
                .ok_or_else(|| PyValueError::new_err("non-finite plane outline"))?;
            if first.is_none() {
                first = Some((cell, pt.clone()));
            }
            source.insert(cell, pt);
            points.push(p);
        }
        // The closing edge writes the original first endpoint last.
        if let Some((k, v)) = first {
            source.insert(k, v);
        }
        Ok(points)
    };
    for r in regions.try_iter()? {
        let r = r?;
        let outer = ring(r.get_item(0)?)?;
        let holes = r
            .get_item(1)?
            .try_iter()?
            .map(|h| ring(h?))
            .collect::<PyResult<_>>()?;
        polygons.push((outer, holes));
    }
    let loops = super::cap_outline::trace_ordered(polygons, &creases, TOL, Some(regions.py()))?
        .ok_or_else(|| PyValueError::new_err("non-finite plane outline"))?;
    Ok(loops
        .into_iter()
        .map(|r| {
            r.into_iter()
                .map(|p| source[&Cell::at(p, TOL).unwrap()].clone())
                .collect()
        })
        .collect())
}
