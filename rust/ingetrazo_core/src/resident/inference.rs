// SPDX-License-Identifier: GPL-3.0-or-later
//! Screen-space bounds of resident instances for cursor queries.
mod edges;
mod lease;
use numpy::{ndarray::Array2, IntoPyArray, PyArray2, PyReadonlyArray2};
use pyo3::{exceptions::PyValueError, prelude::*};
#[pyfunction]
fn project_boxes<'py>(
    py: Python<'py>,
    wc: PyReadonlyArray2<'py, f64>,
    we: PyReadonlyArray2<'py, f64>,
    mvp: &Bound<'py, PyAny>,
    width: f64,
    height: f64,
) -> PyResult<(Bound<'py, PyArray2<f64>>, Bound<'py, PyArray2<f64>>)> {
    let m: [f64; 16] = mvp.call_method0("data")?.extract()?;
    let c = wc.as_array();
    let e = we.as_array();
    if c.dim() != e.dim() || c.ncols() != 3 {
        return Err(PyValueError::new_err(
            "expected matching (n, 3) box centres and extents",
        ));
    }
    let mut lo = Array2::from_elem((c.nrows(), 2), f64::INFINITY);
    let mut hi = Array2::from_elem((c.nrows(), 2), f64::NEG_INFINITY);
    for i in 0..c.nrows() {
        let clip_center =
            |j: usize| c[[i, 0]] * m[j] + c[[i, 1]] * m[4 + j] + c[[i, 2]] * m[8 + j] + m[12 + j];
        let center = [clip_center(0), clip_center(1), clip_center(3)];
        let mut front = 0;
        for sx in [-1., 1.] {
            for sy in [-1., 1.] {
                for sz in [-1., 1.] {
                    let clip = |j: usize, k: usize| {
                        center[k]
                            + sx * e[[i, 0]] * m[j]
                            + sy * e[[i, 1]] * m[4 + j]
                            + sz * e[[i, 2]] * m[8 + j]
                    };
                    let w = clip(3, 2);
                    if w <= 1e-3 || w.is_nan() {
                        continue;
                    }
                    front += 1;
                    for (j, v) in [
                        (0, (1. + clip(0, 0) / w) * (width / 2.)),
                        (1, (1. - clip(1, 1) / w) * (height / 2.)),
                    ] {
                        if v.is_nan() {
                            lo[[i, j]] = v;
                            hi[[i, j]] = v;
                        } else if !lo[[i, j]].is_nan() {
                            lo[[i, j]] = lo[[i, j]].min(v);
                            hi[[i, j]] = hi[[i, j]].max(v);
                        }
                    }
                }
            }
        }
        if front < 8 {
            lo.row_mut(i).fill(if front == 0 {
                f64::INFINITY
            } else {
                f64::NEG_INFINITY
            });
            hi.row_mut(i).fill(f64::INFINITY);
        }
    }
    Ok((lo.into_pyarray(py), hi.into_pyarray(py)))
}
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<edges::ResidentEdges>()?;
    m.add_class::<lease::SceneLease>()?;
    m.add_function(wrap_pyfunction!(nearest_inference_triangle, m)?)?;
    m.add_function(wrap_pyfunction!(resident_curve_features, m)?)?;
    m.add_function(wrap_pyfunction!(project_boxes, m)?)
}

/// The front triangle and its normal, using the same barycentric slack as picking.
#[pyfunction]
fn nearest_inference_triangle(
    origin: [f64; 3],
    direction: [f64; 3],
    v0: PyReadonlyArray2<'_, f64>,
    e1: PyReadonlyArray2<'_, f64>,
    e2: PyReadonlyArray2<'_, f64>,
) -> PyResult<Option<(usize, f64, Option<[f64; 3]>)>> {
    let (v, a, b) = (v0.as_array(), e1.as_array(), e2.as_array());
    if v.ncols() != 3 || v.dim() != a.dim() || v.dim() != b.dim() {
        return Err(PyValueError::new_err(
            "expected matching triangle arrays with three coordinates",
        ));
    }
    let mut best = None;
    for (i, ((v, a), b)) in v.rows().into_iter().zip(a.rows()).zip(b.rows()).enumerate() {
        let vector = |r: numpy::ndarray::ArrayView1<'_, f64>| [r[0], r[1], r[2]];
        if let Some(t) =
            crate::pick::ray_triangle(origin, direction, vector(v), vector(a), vector(b))
        {
            if best.is_none_or(|(_, d)| t < d) {
                best = Some((i, t));
            }
        }
    }
    Ok(best.map(|(i, t)| {
        let a = glam::DVec3::new(a[[i, 0]], a[[i, 1]], a[[i, 2]]);
        let b = glam::DVec3::new(b[[i, 0]], b[[i, 1]], b[[i, 2]]);
        let n = a.cross(b);
        let len = n.length();
        (i, t, (len > 1e-30).then(|| (n / len).to_array()))
    }))
}

/// Fit each named chain once; repeated corners are interior, singleton corners
/// are endpoints. Preserve lexicographic ordering for coincident candidates.
#[pyfunction]
fn resident_curve_features<'py>(
    curves: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, pyo3::types::PyTuple>> {
    use glam::DVec3;
    use pyo3::types::{PyList, PyTuple};
    let py = curves.py();
    let chains = match curves.getattr_opt("curve_segments")? {
        Some(f) => f.call0()?,
        None => curves.clone(),
    };
    let centers = PyList::empty(py);
    let tangents = PyList::empty(py);
    let mids = PyList::empty(py);
    let asarray = py.import("numpy")?.getattr("asarray")?;
    for chain in chains.try_iter()? {
        let v = asarray
            .call1((chain?, "float64"))?
            .call_method1("reshape", (-1, 3))?;
        let ends: Vec<[f64; 3]> = v.extract()?;
        if ends.len() % 2 != 0 {
            return Err(PyValueError::new_err("expected pairs of curve endpoints"));
        }
        let mut order: Vec<usize> = (0..ends.len()).collect();
        order.sort_by(|&a, &b| {
            ends[a]
                .partial_cmp(&ends[b])
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut points = Vec::new();
        let mut tips = Vec::new();
        for run in order.chunk_by(|&a, &b| ends[a] == ends[b]) {
            points.push(ends[run[0]]);
            if run.len() == 1 {
                tips.push(run[0]);
            }
        }
        let Some((center, radius)) = crate::snap_curve::snap_circle(py, points.clone(), 0.01)
        else {
            continue;
        };
        centers.append(center.to_vec().into_pyarray(py))?;
        if tips.len() == 2 {
            if let Some(mid) = crate::snap_curve::arc_midpoint(points, center, radius) {
                mids.append(mid.to_vec().into_pyarray(py))?;
            }
        }
        for i in tips {
            let p = DVec3::from_array(ends[i]);
            let leave = p - DVec3::from_array(ends[i ^ 1]);
            let r = (p - DVec3::from_array(center)).normalize();
            let tangent = leave - r * leave.dot(r);
            if tangent.length() > 1e-9 {
                tangents.append((
                    ends[i].to_vec().into_pyarray(py),
                    tangent.normalize().to_array().to_vec().into_pyarray(py),
                ))?;
            }
        }
    }
    PyTuple::new(py, [centers, tangents, mids])
}
