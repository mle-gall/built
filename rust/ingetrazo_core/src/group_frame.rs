//! A group's derived frame (the yaw about Z whose footprint is tightest)
//! and its oriented box, from `core.group` (`docs/model-port.md`, bounds).
//!
//! `frame_from_points` picks the smallest of tied areas on symmetric shapes,
//! so it is ported operation for operation, the convex hull's pruning
//! stages included: elementwise as NumPy does them, and `hull @ u` as the
//! plain left-to-right sum NumPy computes for a 2-vector (measured). The
//! oriented box multiplies `(N, 3)` points by the frame as NumPy's BLAS
//! does for most sizes (`fma(z, a2, fma(y, a1, x·a0))`); for a few sizes
//! BLAS takes another order, so the box agrees to a few ulps
//! (`tests/test_group_frame_parity.py`).

use numpy::{PyArray1, PyArrayMethods, PyReadonlyArray2};
use pyo3::prelude::*;

type P2 = [f64; 2];

fn cross(a: P2, b: P2, q: P2) -> f64 {
    (b[0] - a[0]) * (q[1] - a[1]) - (b[1] - a[1]) * (q[0] - a[0])
}

/// The points not strictly inside the convex polygon `poly` (CCW).
fn outside(points: &[P2], poly: &[P2]) -> Vec<bool> {
    points
        .iter()
        .map(|p| {
            let inside = (0..poly.len()).all(|i| {
                let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]) > 1e-12
            });
            !inside
        })
        .collect()
}

fn argmin_by(points: &[P2], k: usize) -> usize {
    let mut best = 0;
    for (i, p) in points.iter().enumerate() {
        if p[k] < points[best][k] {
            best = i;
        }
    }
    best
}

fn argmax_by(points: &[P2], k: usize) -> usize {
    let mut best = 0;
    for (i, p) in points.iter().enumerate() {
        if p[k] > points[best][k] {
            best = i;
        }
    }
    best
}

fn keep(points: Vec<P2>, mask: &[bool]) -> Vec<P2> {
    if !mask.iter().any(|m| *m) {
        return points;
    }
    points.into_iter().zip(mask).filter(|(_, m)| **m).map(|(p, _)| p).collect()
}

/// Convex hull of 2D points (monotone chain), counter-clockwise from the
/// lowest x (then y), with `core.group._hull_2d`'s pruning stages.
pub fn hull_2d(mut pts: Vec<P2>) -> Vec<P2> {
    if pts.len() > 64 {
        let quad = [pts[argmin_by(&pts, 0)], pts[argmin_by(&pts, 1)], pts[argmax_by(&pts, 0)], pts[argmax_by(&pts, 1)]];
        let mask = outside(&pts, &quad);
        pts = keep(pts, &mask);
    }
    if pts.len() > 64 {
        let dirs: Vec<P2> = (0..16)
            .map(|k| {
                let a = k as f64 * (2.0 * std::f64::consts::PI / 16.0);
                [a.cos(), a.sin()]
            })
            .collect();
        let mut ext: Vec<usize> = dirs
            .iter()
            .map(|d| {
                let mut best = 0;
                let mut val = f64::NEG_INFINITY;
                for (i, p) in pts.iter().enumerate() {
                    let v = p[0] * d[0] + p[1] * d[1];
                    if v > val {
                        val = v;
                        best = i;
                    }
                }
                best
            })
            .collect();
        ext.sort_unstable();
        ext.dedup();
        let mut poly: Vec<P2> = ext.iter().map(|i| pts[*i]).collect();
        let n = poly.len() as f64;
        let c = [poly.iter().map(|p| p[0]).sum::<f64>() / n, poly.iter().map(|p| p[1]).sum::<f64>() / n];
        let mut keyed: Vec<(f64, P2)> = poly.iter().map(|p| ((p[1] - c[1]).atan2(p[0] - c[0]), *p)).collect();
        keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        poly = keyed.into_iter().map(|(_, p)| p).collect();
        if poly.len() >= 3 {
            let mask = outside(&pts, &poly);
            pts = keep(pts, &mask);
        }
    }
    pts.sort_by(|a, b| {
        a[0].partial_cmp(&b[0]).unwrap_or(std::cmp::Ordering::Equal)
            .then(a[1].partial_cmp(&b[1]).unwrap_or(std::cmp::Ordering::Equal))
    });
    pts.dedup_by(|a, b| a[0] == b[0] && a[1] == b[1]);
    if pts.len() < 3 {
        return pts;
    }
    let half = |seq: &mut dyn Iterator<Item = P2>| -> Vec<P2> {
        let mut out: Vec<P2> = Vec::new();
        for q in seq {
            while out.len() >= 2 {
                let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
                if cross(a, b, q) > 0.0 {
                    break;
                }
                out.pop();
            }
            out.push(q);
        }
        out
    };
    let mut lower = half(&mut pts.iter().copied());
    let mut upper = half(&mut pts.iter().rev().copied());
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// The in-plane unit axis of the tightest footprint, or `None` (the world
/// axes): see `core.group.frame_from_points`.
pub fn frame_axis(points: &[P2]) -> Option<P2> {
    if points.len() < 3 {
        return None;
    }
    let hull = hull_2d(points.to_vec());
    if hull.len() < 3 {
        return None;
    }
    let mut best: Option<(f64, P2)> = None;
    for i in 0..hull.len() {
        let (a, b) = (hull[i], hull[(i + 1) % hull.len()]);
        let d = [b[0] - a[0], b[1] - a[1]];
        let length = d[0].hypot(d[1]);
        if length < 1e-12 {
            continue;
        }
        let u = [d[0] / length, d[1] / length];
        let v = [-u[1], u[0]];
        let (mut amin, mut amax, mut bmin, mut bmax) =
            (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
        for p in &hull {
            let pa = p[0] * u[0] + p[1] * u[1];
            let pb = p[0] * v[0] + p[1] * v[1];
            amin = amin.min(pa);
            amax = amax.max(pa);
            bmin = bmin.min(pb);
            bmax = bmax.max(pb);
        }
        let area = (amax - amin) * (bmax - bmin);
        if best.is_none_or(|(b, _)| area < b) {
            best = Some((area, u));
        }
    }
    best.map(|(_, u)| u)
}

fn xy_rows(points: &PyReadonlyArray2<'_, f64>) -> PyResult<Vec<P2>> {
    let a = points.as_array();
    if a.shape()[1] < 2 {
        return Err(pyo3::exceptions::PyValueError::new_err("points need x and y"));
    }
    Ok((0..a.shape()[0]).map(|i| [a[[i, 0]], a[[i, 1]]]).collect())
}

/// `_hull_2d(pts)`: the hull as an `(H, 2)` array.
#[pyfunction]
pub fn group_hull_2d<'py>(py: Python<'py>, points: PyReadonlyArray2<'py, f64>) -> PyResult<Bound<'py, PyAny>> {
    let hull = hull_2d(xy_rows(&points)?);
    let n = hull.len();
    Ok(PyArray1::from_vec(py, hull.into_iter().flatten().collect()).reshape([n, 2])?.into_any())
}

/// `frame_from_points`' first axis `(ux, uy)`, or None for the world axes.
#[pyfunction]
pub fn group_frame_axis(points: PyReadonlyArray2<'_, f64>) -> PyResult<Option<(f64, f64)>> {
    Ok(frame_axis(&xy_rows(&points)?).map(|u| (u[0], u[1])))
}

/// `(lo, hi)` of `points` `(N, 3)` along the three `axes` rows `(3, 3)`.
#[pyfunction]
pub fn group_oriented_extent(points: PyReadonlyArray2<'_, f64>, axes: PyReadonlyArray2<'_, f64>)
    -> PyResult<((f64, f64, f64), (f64, f64, f64))> {
    let p = points.as_array();
    let a = axes.as_array();
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for i in 0..p.shape()[0] {
        for k in 0..3 {
            let v = p[[i, 2]].mul_add(a[[k, 2]], p[[i, 1]].mul_add(a[[k, 1]], p[[i, 0]] * a[[k, 0]]));
            lo[k] = lo[k].min(v);
            hi[k] = hi[k].max(v);
        }
    }
    Ok(((lo[0], lo[1], lo[2]), (hi[0], hi[1], hi[2])))
}

/// `points @ m[:3, :3] + m[3, :3]` for a `(4, 4)` row-major matrix `m`
/// (`core.group.placement_points`): the fused chain NumPy's BLAS takes for
/// most sizes, then the translation.
#[pyfunction]
pub fn group_placed_points<'py>(py: Python<'py>, points: PyReadonlyArray2<'py, f64>, m: PyReadonlyArray2<'py, f64>)
    -> PyResult<Bound<'py, PyAny>> {
    let p = points.as_array();
    let m = m.as_array();
    let n = p.shape()[0];
    let mut out = Vec::with_capacity(n * 3);
    for i in 0..n {
        for k in 0..3 {
            let v = p[[i, 2]].mul_add(m[[2, k]], p[[i, 1]].mul_add(m[[1, k]], p[[i, 0]] * m[[0, k]]));
            out.push(v + m[[3, k]]);
        }
    }
    Ok(PyArray1::from_vec(py, out).reshape([n, 3])?.into_any())
}

/// The box of `points` placed by the column-major matrix `d` (16 values;
/// None: as they are), as `Scene.bounds` reads it: `((lo), (hi))`.
#[pyfunction]
#[pyo3(signature = (points, d=None))]
pub fn group_placed_box(points: PyReadonlyArray2<'_, f64>, d: Option<Vec<f64>>)
    -> PyResult<((f64, f64, f64), (f64, f64, f64))> {
    let p = points.as_array();
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for i in 0..p.shape()[0] {
        let (x, y, z) = (p[[i, 0]], p[[i, 1]], p[[i, 2]]);
        for k in 0..3 {
            let v = match &d {
                Some(d) => z.mul_add(d[8 + k], y.mul_add(d[4 + k], x * d[k])) + d[12 + k],
                None => [x, y, z][k],
            };
            lo[k] = lo[k].min(v);
            hi[k] = hi[k].max(v);
        }
    }
    Ok(((lo[0], lo[1], lo[2]), (hi[0], hi[1], hi[2])))
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(group_placed_points, m)?)?;
    m.add_function(wrap_pyfunction!(group_placed_box, m)?)?;
    m.add_function(wrap_pyfunction!(group_hull_2d, m)?)?;
    m.add_function(wrap_pyfunction!(group_frame_axis, m)?)?;
    m.add_function(wrap_pyfunction!(group_oriented_extent, m)?)?;
    Ok(())
}
