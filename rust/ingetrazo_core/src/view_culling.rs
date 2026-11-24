// SPDX-License-Identifier: GPL-3.0-or-later
//! Camera half-spaces, instance sizes and visible vertex ranges.
use crate::math::mat;
use numpy::{
    ndarray::ArrayView2, AllowTypeChange, IntoPyArray, PyArray1, PyArray2, PyArrayLike1,
    PyArrayLike2, PyArrayMethods, PyReadonlyArray1,
};
use pyo3::{exceptions::PyValueError, prelude::*, types::PyDict};

type P = [f64; 3];
type Plane = [f64; 4];
type Box3 = (P, P);
type Obj<'py> = Bound<'py, PyAny>;
type Rows<'py> = PyArrayLike2<'py, f64, AllowTypeChange>;
type Spheres<'py> = (Bound<'py, PyArray2<f64>>, Bound<'py, PyArray1<f64>>);
type Span = (Option<Box3>, usize, usize);

#[pyfunction]
fn frustum_planes(mvp: &Obj) -> PyResult<Vec<(f64, f64, f64, f64)>> {
    let m = mat(mvp)?;
    let mut out = Vec::with_capacity(6);
    for i in 0..3 {
        for sign in [1., -1.] {
            let p = m.row(3) + sign * m.row(i);
            out.push((p.x, p.y, p.z, p.w))
        }
    }
    Ok(out)
}

#[pyfunction]
fn widened_planes(planes: [Plane; 6], frac: f64) -> Vec<(f64, f64, f64, f64)> {
    let mut wide = Vec::new();
    for p in &planes[..4] {
        let [a, b, c, d] = std::array::from_fn(|k| p[k] + frac * (planes[0][k] + planes[1][k]));
        wide.push((a, b, c, d))
    }
    let [a, b, c, d] = planes[5];
    wide.push((a, b, c, d));
    wide
}

#[pyfunction]
fn aabb_visible(planes: Vec<Plane>, lo: P, hi: P) -> bool {
    inside(&planes, (lo, hi))
}

fn inside(planes: &[Plane], (lo, hi): Box3) -> bool {
    planes.iter().all(|p| {
        let coord = |i: usize| if p[i] >= 0. { hi[i] } else { lo[i] };
        !(p[0] * coord(0) + p[1] * coord(1) + p[2] * coord(2) + p[3] < 0.)
    })
}

fn bounds<'a>(lo: &'a Rows, hi: &'a Rows) -> PyResult<(ArrayView2<'a, f64>, ArrayView2<'a, f64>)> {
    let (lo, hi) = (lo.as_array(), hi.as_array());
    if lo.ncols() != 3 || lo.dim() != hi.dim() {
        return Err(PyValueError::new_err(
            "expected two (N, 3) arrays of bounds",
        ));
    }
    Ok((lo, hi))
}
fn row(a: &ArrayView2<f64>, i: usize) -> P {
    [a[[i, 0]], a[[i, 1]], a[[i, 2]]]
}

#[pyfunction]
fn instances_in_frustum<'py>(
    py: Python<'py>,
    lo: Rows<'py>,
    hi: Rows<'py>,
    mut planes: Vec<Plane>,
    margin: f64,
) -> PyResult<Bound<'py, PyArray1<bool>>> {
    let (lo, hi) = bounds(&lo, &hi)?;
    if margin != 0. {
        for p in &mut planes {
            p[3] += margin * (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt()
        }
    }
    Ok((0..lo.nrows())
        .map(|i| {
            let (l, h) = (row(&lo, i), row(&hi, i));
            !l.iter().chain(&h).all(|x| x.is_finite()) || inside(&planes, (l, h))
        })
        .collect::<Vec<_>>()
        .into_pyarray(py))
}

fn sphere((lo, hi): Box3) -> (P, f64) {
    let c: P = std::array::from_fn(|i| (lo[i] + hi[i]) * 0.5);
    let d = std::array::from_fn::<_, 3, _>(|i| hi[i] - lo[i]);
    let r = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() * 0.5;
    if c.iter().all(|v| v.is_finite()) && r.is_finite() {
        (c, r)
    } else {
        ([0.; 3], f64::INFINITY)
    }
}

fn pack_spheres(py: Python<'_>, boxes: impl Iterator<Item = Box3>) -> PyResult<Spheres<'_>> {
    let (mut centres, mut radii) = (Vec::new(), Vec::new());
    for bounds in boxes {
        let (c, r) = sphere(bounds);
        centres.extend(c);
        radii.push(r)
    }
    Ok((
        centres.into_pyarray(py).reshape([radii.len(), 3])?,
        radii.into_pyarray(py),
    ))
}
#[pyfunction]
fn bounding_spheres<'py>(py: Python<'py>, lo: Rows<'py>, hi: Rows<'py>) -> PyResult<Spheres<'py>> {
    let (lo, hi) = bounds(&lo, &hi)?;
    pack_spheres(py, (0..lo.nrows()).map(|i| (row(&lo, i), row(&hi, i))))
}

#[pyfunction]
fn small_spheres<'py>(
    py: Python<'py>,
    ctr: Rows<'py>,
    rad: PyArrayLike1<'py, f64, AllowTypeChange>,
    w: Plane,
    scale: f64,
    perspective: bool,
    floor: f64,
) -> PyResult<Bound<'py, PyArray1<bool>>> {
    let ctr = ctr.as_array();
    let rad = rad.as_array();
    if ctr.dim() != (rad.len(), 3) {
        return Err(PyValueError::new_err(
            "inconsistent sphere centres and radii",
        ));
    }
    Ok(rad
        .iter()
        .enumerate()
        .map(|(i, &r)| {
            let c = row(&ctr, i);
            let depth =
                w[0] * c[0] + w[1] * c[1] + w[2] * c[2] + w[3] - if perspective { r } else { 0. };
            let pixels = r * scale / depth;
            depth > 1e-9 && pixels.is_finite() && pixels < floor
        })
        .collect::<Vec<_>>()
        .into_pyarray(py))
}

#[pyclass]
struct SpanTable {
    spans: Vec<Span>,
    centres: Py<PyArray2<f64>>,
    radii: Py<PyArray1<f64>>,
}

fn run(out: &mut Vec<(usize, usize)>, start: usize, count: usize, split: Option<usize>) {
    if let Some((prev, n)) = out.last_mut() {
        if prev.checked_add(*n) == Some(start) && !split.is_some_and(|s| *prev < s && s <= start) {
            *n += count;
            return;
        }
    }
    out.push((start, count))
}

#[pyfunction]
#[pyo3(signature=(view,spans,planes,split,batch))]
fn visible_draw_spans(
    view: &Obj,
    spans: &Obj,
    planes: &Obj,
    split: Option<usize>,
    batch: bool,
) -> PyResult<(Vec<(usize, usize)>, usize)> {
    let py = view.py();
    let mut out = Vec::new();
    let mut culled = 0;
    if !batch {
        for item in spans.try_iter()? {
            let (b, s, n): (Obj, usize, usize) = item?.extract()?;
            if n == 0 {
                continue;
            }
            if !b.is_none()
                && !view
                    .call_method1("_aabb_visible", (planes, b.get_item(0)?, b.get_item(1)?))?
                    .is_truthy()?
            {
                culled += n
            } else {
                run(&mut out, s, n, split)
            }
        }
        return Ok((out, culled));
    }
    let cache = match view.getattr_opt("_span_arrays")?.filter(|c| !c.is_none()) {
        Some(c) => c,
        None => {
            let d = PyDict::new(py);
            view.setattr("_span_arrays", &d)?;
            d.into_any()
        }
    };
    let key = spans.as_ptr() as usize;
    let cached = cache.call_method1("get", (key,))?;
    let table = if !cached.is_none() && cached.get_item(0)?.is(spans) {
        cached.get_item(1)?.cast_into::<SpanTable>()?
    } else {
        let rows: Vec<Span> = spans.extract()?;
        let (c, r) = pack_spheres(py, rows.iter().map(|s| s.0.unwrap_or(([0.; 3], [0.; 3]))))?;
        let table = Bound::new(
            py,
            SpanTable {
                spans: rows,
                centres: c.unbind(),
                radii: r.unbind(),
            },
        )?;
        if cache.len()? > 16 {
            cache.call_method0("clear")?;
        }
        cache.set_item(key, (spans, &table))?;
        table
    };
    let table = table.borrow();
    let small: PyReadonlyArray1<bool> = view
        .call_method1(
            "_spheres_below_a_pixel",
            (table.centres.bind(py), table.radii.bind(py)),
        )?
        .extract()?;
    let small = small.as_array();
    let planes: Vec<Plane> = planes.extract()?;
    if small.len() != table.spans.len() {
        return Err(PyValueError::new_err("wrong number of projected spheres"));
    }
    for (&(b, s, n), &tiny) in table.spans.iter().zip(small) {
        if n > 0 {
            if b.is_some_and(|bb| tiny || !inside(&planes, bb)) {
                culled += n
            } else {
                run(&mut out, s, n, split)
            }
        }
    }
    Ok((out, culled))
}

pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(frustum_planes, m)?)?;
    m.add_function(wrap_pyfunction!(widened_planes, m)?)?;
    m.add_function(wrap_pyfunction!(aabb_visible, m)?)?;
    m.add_function(wrap_pyfunction!(instances_in_frustum, m)?)?;
    m.add_function(wrap_pyfunction!(bounding_spheres, m)?)?;
    m.add_function(wrap_pyfunction!(small_spheres, m)?)?;
    m.add_function(wrap_pyfunction!(visible_draw_spans, m)?)?;
    Ok(())
}
