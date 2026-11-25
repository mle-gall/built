// SPDX-License-Identifier: GPL-3.0-or-later
//! Align CAD profiles and build their side polygons in arc-length order.
use crate::math::{xyz, Vec3};
use glam::DVec3;
use pyo3::prelude::*;
use pyo3::types::PyList;
const EPS: f64 = 1e-9;

struct Profile<'py> {
    objects: Vec<Bound<'py, PyAny>>,
    points: Vec<DVec3>,
}
impl<'py> Profile<'py> {
    fn read(seq: &Bound<'py, PyAny>) -> PyResult<Self> {
        let objects = seq.try_iter()?.collect::<PyResult<Vec<_>>>()?;
        let points = objects.iter().map(xyz).collect::<PyResult<_>>()?;
        Ok(Self { objects, points })
    }
    fn require_point(&self) -> PyResult<()> {
        if self.points.is_empty() {
            Err(pyo3::exceptions::PyIndexError::new_err(
                "empty bridge profile",
            ))
        } else {
            Ok(())
        }
    }
}
fn midpoint(p: &[DVec3]) -> DVec3 {
    let mut sum = DVec3::ZERO;
    let mut total = 0.;
    for (a, b) in p.iter().zip(p.iter().cycle().skip(1)) {
        let d = (*b - *a).length();
        sum += (*a + *b) * 0.5 * d;
        total += d;
    }
    if total < EPS {
        p.iter().copied().sum::<DVec3>() / p.len() as f64
    } else {
        sum / total
    }
}
#[pyfunction]
fn bridge_centre(points: &Bound<'_, PyAny>) -> PyResult<Vec3> {
    let p = Profile::read(points)?;
    p.require_point()?;
    Ok(midpoint(&p.points).into())
}
fn normalised(p: &[DVec3]) -> Vec<DVec3> {
    let c = midpoint(p);
    let mut v: Vec<_> = p.iter().map(|p| *p - c).collect();
    let r = v.iter().map(|p| p.length()).sum::<f64>() / v.len() as f64;
    if r > EPS {
        for p in &mut v {
            *p /= r;
        }
    }
    v
}
fn fractions(p: &[DVec3], closed: bool) -> Vec<f64> {
    let mut out = vec![0.];
    let mut total = 0.;
    for (a, b) in
        p.iter()
            .zip(p.iter().cycle().skip(1))
            .take(if closed { p.len() } else { p.len() - 1 })
    {
        total += (*b - *a).length();
        out.push(total);
    }
    let n = (out.len() - 1).max(1) as f64;
    for (i, t) in out.iter_mut().enumerate() {
        *t = if total < EPS {
            i as f64 / n
        } else {
            *t / total
        };
    }
    out
}
fn score(a: &[DVec3], ta: &[f64], b: &[DVec3]) -> f64 {
    let nb = normalised(b);
    if a.len() == b.len() {
        return a
            .iter()
            .zip(nb)
            .map(|(a, b)| (*a - b).length_squared())
            .sum();
    }
    let tb = fractions(b, true);
    let mut j = 0;
    let mut s = 0.;
    for (&a, &t) in a.iter().zip(ta) {
        while j + 2 < tb.len() && tb[j + 1] < t {
            j += 1
        }
        let span = tb[j + 1] - tb[j];
        let k = if span > EPS {
            ((t - tb[j]) / span).clamp(0., 1.)
        } else {
            0.
        };
        let q = nb[j] + (nb[(j + 1) % nb.len()] - nb[j]) * k;
        s += (a - q).length_squared();
    }
    s
}
#[pyfunction]
fn align_loop<'py>(a: &Bound<'py, PyAny>, b: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    let py = a.py();
    let a = Profile::read(a)?;
    let b = Profile::read(b)?;
    let n = b.points.len();
    if n == 0 {
        return Ok(PyList::empty(py));
    }
    a.require_point()?;
    let na = normalised(&a.points);
    let ta = fractions(&a.points, true);
    let mut best = f64::INFINITY;
    let mut best_indices = (0..n).collect::<Vec<_>>();
    for reverse in [false, true] {
        let mut order = (0..n).collect::<Vec<_>>();
        if reverse {
            order.reverse()
        }
        for _ in 0..n {
            let points = order.iter().map(|&i| b.points[i]).collect::<Vec<_>>();
            let s = score(&na, &ta, &points);
            if s < best - 1e-12 {
                best = s;
                best_indices.clone_from(&order);
            }
            order.rotate_left(1);
        }
    }
    PyList::new(py, best_indices.iter().map(|&i| &b.objects[i]))
}
#[pyfunction]
fn align_chain<'py>(a: &Bound<'py, PyAny>, b: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    let py = a.py();
    let a = Profile::read(a)?;
    let mut b = Profile::read(b)?;
    a.require_point()?;
    b.require_point()?;
    let (p, q) = (a.points[0], a.points[a.points.len() - 1]);
    let (r, s) = (b.points[0], b.points[b.points.len() - 1]);
    if p.distance(r) + q.distance(s) > p.distance(s) + q.distance(r) + 1e-12 {
        b.objects.reverse()
    }
    PyList::new(py, b.objects)
}
#[pyfunction]
fn strips<'py>(
    a: &Bound<'py, PyAny>,
    b: &Bound<'py, PyAny>,
    closed: bool,
) -> PyResult<Bound<'py, PyList>> {
    let py = a.py();
    let a = Profile::read(a)?;
    let b = Profile::read(b)?;
    a.require_point()?;
    b.require_point()?;
    let n = a.points.len();
    let m = b.points.len();
    let na = if closed { n } else { n - 1 };
    let nb = if closed { m } else { m - 1 };
    let (ta, tb) = if n == m {
        let t = (0..=na)
            .map(|i| i as f64 / na.max(1) as f64)
            .collect::<Vec<_>>();
        (t.clone(), t)
    } else {
        (fractions(&a.points, closed), fractions(&b.points, closed))
    };
    let mut i = 0;
    let mut j = 0;
    let out = PyList::empty(py);
    while i < na || j < nb {
        let p = &a.objects[i % n];
        let q = &b.objects[j % m];
        let r = &a.objects[(i + 1) % n];
        let s = &b.objects[(j + 1) % m];
        let poly = if i < na && j < nb && (ta[i + 1] - tb[j + 1]).abs() <= 1e-6 {
            i += 1;
            j += 1;
            vec![p, r, s, q]
        } else if j >= nb || (i < na && (ta[i + 1] - tb[j]).abs() < (tb[j + 1] - ta[i]).abs()) {
            i += 1;
            vec![p, r, q]
        } else {
            j += 1;
            vec![p, s, q]
        };
        out.append(PyList::new(py, poly)?)?;
    }
    Ok(out)
}
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(bridge_centre, m)?)?;
    m.add_function(wrap_pyfunction!(align_loop, m)?)?;
    m.add_function(wrap_pyfunction!(align_chain, m)?)?;
    m.add_function(wrap_pyfunction!(strips, m)?)?;
    Ok(())
}
