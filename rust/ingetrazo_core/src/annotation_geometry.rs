// SPDX-License-Identifier: GPL-3.0-or-later
//! Camera clipping and depth sampling for document and export annotations.
use crate::math::{mat, xyz, Vec3};
use glam::{DVec3, DVec4};
use numpy::PyReadonlyArray2;
use pyo3::{exceptions::PyValueError, prelude::*, types::PyList};

type Obj<'py> = Bound<'py, PyAny>;
type Pixel = (f64, f64);
type Segment = (Pixel, Pixel);
type Sample = (Option<Pixel>, bool);

/// Intersect an interval with offset + rate * t >= 0.
fn constrain(lo: &mut f64, hi: &mut f64, offset: f64, rate: f64, eps: f64) -> bool {
    if rate.abs() <= eps {
        return offset >= 0.;
    }
    let t = -offset / rate;
    if rate > 0. {
        *lo = lo.max(t);
    } else {
        *hi = hi.min(t);
    }
    *lo <= *hi
}

#[pyfunction]
fn clip_annotation_line(
    a: Pixel,
    b: Pixel,
    width: f64,
    height: f64,
    margin: f64,
) -> Option<Segment> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (mut lo, mut hi) = (0., 1.);
    for (offset, rate) in [
        (a.0 + margin, dx),
        (width + margin - a.0, -dx),
        (a.1 + margin, dy),
        (height + margin - a.1, -dy),
    ] {
        if !constrain(&mut lo, &mut hi, offset, rate, 0.) {
            return None;
        }
    }
    let at = |t| (a.0 + dx * t, a.1 + dy * t);
    Some((at(lo), at(hi)))
}

#[pyfunction]
fn clip_annotation_front(mvp: &Obj, a: &Obj, b: &Obj) -> PyResult<Option<(Vec3, Vec3)>> {
    let m = mat(mvp)?;
    let (a, b) = (xyz(a)?, xyz(b)?);
    let (wa, wb) = ((m * a.extend(1.)).w, (m * b.extend(1.)).w);
    const NEAR: f64 = 0.001;
    if wa <= NEAR && wb <= NEAR {
        return Ok(None);
    }
    let cut = || a + (b - a) * ((NEAR - wa) / (wb - wa));
    Ok(Some((
        (if wa <= NEAR { cut() } else { a }).into(),
        (if wb <= NEAR { cut() } else { b }).into(),
    )))
}

#[pyfunction]
fn clip_snap_guide(
    mvp: &Obj,
    p: &Obj,
    u: &Obj,
    reach: f64,
    margin: f64,
) -> PyResult<Option<(Vec3, Vec3)>> {
    let m = mat(mvp)?;
    let (p, u) = (xyz(p)?, xyz(u)?);
    let (a, b) = (m * p.extend(1.), m * u.extend(0.));
    let (mut lo, mut hi) = (-reach, reach);
    for (offset, rate) in [
        (a.w - 0.001, b.w),
        (margin * a.w - a.x, margin * b.w - b.x),
        (margin * a.w + a.x, margin * b.w + b.x),
        (margin * a.w - a.y, margin * b.w - b.y),
        (margin * a.w + a.y, margin * b.w + b.y),
    ] {
        if !constrain(&mut lo, &mut hi, offset, rate, 1e-15) || lo >= hi {
            return Ok(None);
        }
    }
    Ok(Some(((p + lo * u).into(), (p + hi * u).into())))
}

#[pyfunction]
fn section_frame(
    point: &Obj,
    axes: (Obj, Obj),
    bounds: (Option<Obj>, Option<Obj>),
) -> PyResult<Vec<Vec3>> {
    let (p, u, v) = (xyz(point)?, xyz(&axes.0)?, xyz(&axes.1)?);
    let (centre, hu, hv) = if let (Some(lo), Some(hi)) = bounds {
        let (lo, hi) = (xyz(&lo)?, xyz(&hi)?);
        let centre = (lo + hi) * 0.5 - p;
        let extent = hi - lo;
        (
            p + u * u.dot(centre) + v * v.dot(centre),
            u.abs().dot(extent) * 0.56 + 0.2,
            v.abs().dot(extent) * 0.56 + 0.2,
        )
    } else {
        (p, 1.5, 1.5)
    };
    Ok([(-hu, -hv), (hu, -hv), (hu, hv), (-hu, hv)]
        .into_iter()
        .map(|(x, y)| (centre + x * u + y * v).into())
        .collect())
}

#[pyfunction]
fn annotation_samples(view: &Obj, pts: &Obj, mvp: &Obj) -> PyResult<Vec<Sample>> {
    let snap = view.getattr_opt("_depth_snap")?.filter(|s| !s.is_none());
    let Some(snapshot) = snap else {
        return pts
            .try_iter()?
            .map(|p| {
                let p = p?;
                Ok((
                    view.call_method1("_world_to_pixel", (&p, mvp))?.extract()?,
                    !view
                        .call_method1("_occluded_on_screen", (p,))?
                        .is_truthy()?,
                ))
            })
            .collect();
    };
    let (buf, smvp, inverse, sw, sh, eye, fwd): (Obj, Obj, Obj, usize, usize, Obj, Obj) =
        snapshot.extract()?;
    let m = mat(mvp)?;
    let sm = mat(&smvp)?;
    let inv = mat(&inverse)?;
    let (eye, fwd) = (xyz(&eye)?, xyz(&fwd)?);
    let (w, h): (f64, f64) = (
        view.call_method0("width")?.extract()?,
        view.call_method0("height")?.extract()?,
    );
    let project = |p: DVec3| {
        let c = m * p.extend(1.);
        (c.w > 0.).then(|| {
            (
                (c.x / c.w * 0.5 + 0.5) * w,
                (1. - (c.y / c.w * 0.5 + 0.5)) * h,
            )
        })
    };
    let depth32 = buf.extract::<PyReadonlyArray2<f32>>().ok();
    let depth64 = if depth32.is_none() {
        Some(buf.extract::<PyReadonlyArray2<f64>>()?)
    } else {
        None
    };
    let dims = depth32
        .as_ref()
        .map(|d| d.as_array().dim())
        .or_else(|| depth64.as_ref().map(|d| d.as_array().dim()));
    if dims != Some((sh, sw)) {
        return Err(PyValueError::new_err(
            "depth snapshot dimensions differ from its viewport",
        ));
    }
    let depth = |y, x| {
        depth32.as_ref().map_or_else(
            || depth64.as_ref().unwrap().as_array()[[y, x]],
            |d| f64::from(d.as_array()[[y, x]]),
        )
    };
    let visible = |p: DVec3| {
        let c = sm * p.extend(1.);
        if c.w <= 1e-9 {
            return true;
        }
        let (nx, ny) = (c.x / c.w, c.y / c.w);
        let (x, y) = (
            ((nx + 1.) * 0.5 * sw as f64) as isize,
            ((ny + 1.) * 0.5 * sh as f64) as isize,
        );
        if x < 0 || y < 0 || x >= sw as isize || y >= sh as isize {
            return true;
        }
        let mut far = f64::NEG_INFINITY;
        for j in (y - 1).max(0)..=(y + 1).min(sh as isize - 1) {
            for i in (x - 1).max(0)..=(x + 1).min(sw as isize - 1) {
                let d = depth(j as usize, i as usize);
                if d.is_nan() {
                    return true;
                }
                far = far.max(d);
            }
        }
        if far >= 1. {
            return true;
        }
        let q = inv * DVec4::new(nx, ny, far * 2. - 1., 1.);
        let along = (p - eye).dot(fwd);
        q.w.abs() < 1e-12
            || (q.truncate() / q.w - eye).dot(fwd) >= along - 0.02f64.max(0.003 * along.abs())
    };
    pts.try_iter()?
        .map(|p| {
            let p = xyz(&p?)?;
            Ok((project(p), visible(p)))
        })
        .collect()
}

#[derive(Clone, Copy)]
struct Transition {
    step: usize,
    lo: f64,
    hi: f64,
    edge: Option<Pixel>,
}

#[pyfunction]
fn annotation_strokes(
    view: &Obj,
    a: &Obj,
    b: &Obj,
    mvp: &Obj,
    samples: usize,
) -> PyResult<Vec<Segment>> {
    let pixel = |p: &Obj| {
        view.call_method1("_world_to_pixel", (p, mvp))?
            .extract::<Option<Pixel>>()
    };
    let mut n = samples;
    if let (Some((x, y)), Some((xx, yy))) = (pixel(a)?, pixel(b)?) {
        let w: f64 = view.call_method0("width")?.extract()?;
        let h: f64 = view.call_method0("height")?.extract()?;
        if x.max(xx) < -2. || y.max(yy) < -2. || x.min(xx) > w + 2. || y.min(yy) > h + 2. {
            return Ok(vec![]);
        }
        let max: usize = view.getattr("_OCCLUSION_MAX_SAMPLES")?.extract()?;
        let step: f64 = view.getattr("_OCCLUSION_STEP_PX")?.extract()?;
        n = n.max(max.min(((xx - x).hypot(yy - y) / step) as usize));
    }
    if n == 0 {
        return Err(PyValueError::new_err(
            "expected at least one annotation sample",
        ));
    }
    let a = xyz(a)?;
    let delta = xyz(b)? - a;
    let sample = |times: &[f64]| -> PyResult<Vec<Sample>> {
        let pts = PyList::new(view.py(), times.iter().map(|t| Vec3::from(a + delta * *t)))?;
        let result: Vec<Sample> = view.call_method1("_project_samples", (pts,))?.extract()?;
        if result.len() != times.len() {
            return Err(PyValueError::new_err("annotation sample count mismatch"));
        }
        Ok(result)
    };
    let times: Vec<_> = (0..=n).map(|i| i as f64 / n as f64).collect();
    let rows = sample(&times)?;
    let mut cuts = Vec::new();
    for i in 0..n {
        let ((a, u), (b, v)) = (rows[i], rows[i + 1]);
        if a.is_some() && b.is_some() && u != v {
            cuts.push(Transition {
                step: i,
                lo: times[i],
                hi: times[i + 1],
                edge: if u { a } else { b },
            })
        }
    }
    if !cuts.is_empty() {
        let k: usize = view.getattr("_OCCLUSION_SPLIT")?.extract()?;
        let rounds: usize = view.getattr("_OCCLUSION_ROUNDS")?.extract()?;
        if k < 2 {
            return Err(PyValueError::new_err("expected at least two subdivisions"));
        }
        for _ in 0..rounds {
            let ts: Vec<_> = cuts
                .iter()
                .flat_map(|c| (1..k).map(move |j| c.lo + (c.hi - c.lo) * j as f64 / k as f64))
                .collect();
            let probes = sample(&ts)?;
            for ((c, times), row) in cuts
                .iter_mut()
                .zip(ts.chunks(k - 1))
                .zip(probes.chunks(k - 1))
            {
                let start_visible = rows[c.step].1;
                for (&t, &(px, v)) in times.iter().zip(row) {
                    if (px.is_some() && v) == start_visible {
                        c.lo = t;
                        if start_visible {
                            c.edge = px
                        }
                    } else {
                        c.hi = t;
                        if !start_visible {
                            c.edge = px
                        }
                        break;
                    }
                }
            }
        }
    }
    let mut cuts = cuts.into_iter().peekable();
    let mut strokes = Vec::new();
    for i in 0..n {
        if let ((Some(a), va), (Some(b), vb)) = (rows[i], rows[i + 1]) {
            if va && vb {
                strokes.push((a, b))
            } else if cuts.peek().is_some_and(|c| c.step == i) {
                if let Some(end) = cuts.next().and_then(|c| c.edge) {
                    strokes.push((if va { a } else { b }, end))
                }
            }
        }
    }
    Ok(strokes)
}

pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(annotation_samples, m)?)?;
    m.add_function(wrap_pyfunction!(annotation_strokes, m)?)?;
    m.add_function(wrap_pyfunction!(clip_annotation_line, m)?)?;
    m.add_function(wrap_pyfunction!(clip_annotation_front, m)?)?;
    m.add_function(wrap_pyfunction!(clip_snap_guide, m)?)?;
    m.add_function(wrap_pyfunction!(section_frame, m)?)?;
    Ok(())
}
