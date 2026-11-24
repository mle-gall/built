// SPDX-License-Identifier: GPL-3.0-or-later
//! Widget-space query footprints and limits from the last depth frame.
use super::*;
use glam::{DMat4, DVec3, DVec4};
use numpy::{PyReadonlyArray1, PyReadonlyArray2};
use pyo3::exceptions::PyValueError;

enum Depth<'py> {
    Single(PyReadonlyArray2<'py, f32>),
    Double(PyReadonlyArray2<'py, f64>),
}
struct Frame<'py> {
    depth: Depth<'py>,
    inverse: DMat4,
    eye: DVec3,
    w: i64,
    h: i64,
    dpr: f64,
}
impl<'py> Frame<'py> {
    fn read(view: &Obj<'py>) -> PyResult<Option<Self>> {
        let snap = attr(view, "_depth_snap")?;
        if snap.is_none() {
            return Ok(None);
        }
        let raw = snap.get_item(0)?;
        let depth = match raw.extract() {
            Ok(v) => Depth::Single(v),
            Err(_) => Depth::Double(raw.extract()?),
        };
        let w: i64 = snap.get_item(3)?.extract()?;
        let h: i64 = snap.get_item(4)?.extract()?;
        let shape = match &depth {
            Depth::Single(a) => a.as_array().dim(),
            Depth::Double(a) => a.as_array().dim(),
        };
        if w < 1 || h < 1 || shape != (h as usize, w as usize) {
            return Err(PyValueError::new_err("invalid depth frame dimensions"));
        }
        let inverse = DMat4::from_cols_array(&snap.get_item(2)?.call_method0("data")?.extract()?);
        Ok(Some(Self {
            depth,
            inverse,
            eye: xyz(&view.getattr("camera")?.call_method0("eye")?)?,
            w,
            h,
            dpr: w as f64 / view.call_method0("width")?.extract::<f64>()?.max(1.),
        }))
    }
    fn sample(&self, x: i64, y: i64) -> f64 {
        match &self.depth {
            Depth::Single(a) => a.as_array()[[y as usize, x as usize]] as f64,
            Depth::Double(a) => a.as_array()[[y as usize, x as usize]],
        }
    }
    fn distance(&self, x: i64, y: i64, depth: f64) -> Option<f64> {
        let q = self.inverse
            * DVec4::new(
                (x as f64 + 0.5) / self.w as f64 * 2. - 1.,
                (y as f64 + 0.5) / self.h as f64 * 2. - 1.,
                depth * 2. - 1.,
                1.,
            );
        (q.w.abs() >= 1e-12).then(|| (q.truncate() / q.w).distance(self.eye))
    }
}

#[pyfunction]
pub fn pick_depth_limits<'py>(view: &Obj<'py>, x: f64, y: f64, r: f64) -> PyResult<Obj<'py>> {
    let py = view.py();
    let Some(f) = Frame::read(view)? else {
        return Ok(none(py));
    };
    let (x0, x1) = (
        ((x - r) * f.dpr).floor() as i64 - 1,
        ((x + r) * f.dpr).floor() as i64 + 1,
    );
    let (y0, y1) = (
        f.h - 2 - ((y + r) * f.dpr).floor() as i64,
        f.h - ((y - r) * f.dpr).floor() as i64,
    );
    let far_plane = view
        .getattr("camera")?
        .call_method0("far_plane")?
        .extract::<f64>()?;
    let mut limits = Vec::new();
    for iy in y0..=y1 {
        for ix in x0..=x1 {
            let mut limit = f64::INFINITY;
            if ix >= 1 && iy >= 1 && ix < f.w - 1 && iy < f.h - 1 && f.sample(ix, iy) < 1. {
                let mut d = 0.0f64;
                for sy in iy - 1..=iy + 1 {
                    for sx in ix - 1..=ix + 1 {
                        let v = f.sample(sx, sy);
                        if v < 1. {
                            d = d.max(v);
                        }
                    }
                }
                if let (Some(far), Some(next)) = (
                    f.distance(ix, iy, d),
                    f.distance(ix, iy, (d + 8. / (1 << 24) as f64).min(1.)),
                ) {
                    let padded = far + (next - far).abs() + 0.0005 * far + 0.01;
                    if padded.is_finite() && far <= 1.01 * far_plane {
                        limit = padded;
                    }
                }
            }
            limits.push(limit);
        }
    }
    let grid = limits
        .into_pyarray(py)
        .reshape([(y1 - y0 + 1).max(0) as usize, (x1 - x0 + 1).max(0) as usize])?;
    Ok((f.eye.to_array().to_vec(), grid, x0, y0, f.dpr, f.h)
        .into_pyobject(py)?
        .into_any())
}

#[pyfunction]
pub fn depth_hidden<'py>(
    py: Python<'py>,
    seen: &Obj<'py>,
    pts: PyReadonlyArray2<'py, f64>,
    sx: PyReadonlyArray1<'py, f64>,
    sy: PyReadonlyArray1<'py, f64>,
) -> PyResult<Obj<'py>> {
    let lim = crate::depth_limits(seen.extract()?)?;
    let pts = pts.as_array();
    let sx = sx.as_array();
    let sy = sy.as_array();
    if pts.dim() != (sx.len(), 3) || sy.len() != sx.len() {
        return Err(PyValueError::new_err("invalid projected points"));
    }
    Ok((0..sx.len())
        .map(|i| lim.hides([pts[[i, 0]], pts[[i, 1]], pts[[i, 2]]], sx[i], sy[i]))
        .collect::<Vec<_>>()
        .into_pyarray(py)
        .into_any())
}

#[pyfunction]
pub fn visible_depth_limit(view: &Obj, x: f64, y: f64) -> PyResult<Option<f64>> {
    let Some(f) = Frame::read(view)? else {
        return Ok(None);
    };
    let r = (view.getattr("_PICK_NEAR_PX")?.extract::<f64>()? * f.dpr) as i64 + 1;
    let (cx, cy) = ((x * f.dpr) as i64, ((f.h - 1) as f64 - y * f.dpr) as i64);
    let stop = |v: i64, n: i64| if v < 0 { (n + v).max(0) } else { v.min(n) };
    let mut best = None;
    for iy in (cy - r).max(0)..stop(cy + r + 1, f.h) {
        for ix in (cx - r).max(0)..stop(cx + r + 1, f.w) {
            let d = f.sample(ix, iy);
            if d >= 1. {
                return Ok(None);
            }
            if best.is_none_or(|(_, _, b)| d > b) {
                best = Some((ix, iy, d));
            }
        }
    }
    Ok(best
        .and_then(|(x, y, d)| f.distance(x, y, d))
        .map(|d| d * 1.02 + 0.05))
}

fn mvp_key<'py>(m: &Obj<'py>) -> PyResult<Obj<'py>> {
    let values: [f64; 16] = m.call_method0("ravel")?.extract()?;
    Ok(PyTuple::new(m.py(), values)?.into_any())
}

#[pyfunction]
pub fn footprint_rows<'py>(view: &Obj<'py>, table: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = view.py();
    let lo = table.get_item(1)?;
    let hi = table.get_item(2)?;
    let camera = view.getattr("camera")?;
    let eye = xyz(&camera.call_method0("eye")?)?;
    let mut size = attr(view, "_export_size")?;
    if !size.is_truthy()? {
        size = view.call_method0("_fb_size")?;
    }
    let m = view.call_method0("_np_mvp")?;
    let mkey = mvp_key(&m)?;
    let floor: f64 = view.getattr("_PICK_MIN_PX")?.extract()?;
    let key = (
        view.call_method0("_placements_epoch")?,
        mkey,
        (eye.x, eye.y, eye.z),
        size.get_item(1)?,
        camera.getattr("perspective")?.is_truthy()?,
        floor,
    )
        .into_pyobject(py)?;
    let memo = attr(view, "_footprint_rows_memo")?;
    if !memo.is_none()
        && memo.get_item(0)?.eq(&key)?
        && memo.get_item(1)?.is(&lo)
        && memo.get_item(2)?.is(&hi)
    {
        return memo.get_item(3);
    }
    let a: PyReadonlyArray2<f64> = lo.extract()?;
    let b: PyReadonlyArray2<f64> = hi.extract()?;
    let (a, b) = (a.as_array(), b.as_array());
    if a.ncols() != 3 || a.dim() != b.dim() {
        return Err(PyValueError::new_err("invalid placement bounds"));
    }
    let d = (0..a.nrows())
        .map(|i| {
            let p = DVec3::new(
                eye.x.max(a[[i, 0]]).min(b[[i, 0]]),
                eye.y.max(a[[i, 1]]).min(b[[i, 1]]),
                eye.z.max(a[[i, 2]]).min(b[[i, 2]]),
            );
            p.distance(eye)
        })
        .collect::<Vec<_>>();
    let small = if floor > 0. {
        let radii = view.call_method1("_bounding_spheres", (&lo, &hi))?;
        view.call_method1(
            "_spheres_below_a_pixel",
            (radii.get_item(0)?, radii.get_item(1)?, floor),
        )?
    } else {
        none(py)
    };
    let out = (d.into_pyarray(py), small).into_pyobject(py)?.into_any();
    view.setattr("_footprint_rows_memo", (key, lo, hi, &out))?;
    Ok(out)
}

#[pyfunction]
pub fn placements_under_px<'py>(
    view: &Obj<'py>,
    lo: PyReadonlyArray2<'py, f64>,
    hi: PyReadonlyArray2<'py, f64>,
    x: f64,
    y: f64,
    r: Option<f64>,
) -> PyResult<Obj<'py>> {
    let m = view
        .call_method0("_np_mvp")?
        .call_method0("ravel")?
        .extract()?;
    let r = match r {
        Some(r) => r,
        None => view.getattr("_PICK_NEAR_PX")?.extract()?,
    };
    Ok(crate::boxes_near_px(
        view.py(),
        lo,
        hi,
        m,
        view.call_method0("width")?.extract()?,
        view.call_method0("height")?.extract()?,
        x,
        y,
        r,
    )?
    .into_any())
}

#[pyfunction]
pub fn footprint_need<'py>(
    view: &Obj<'py>,
    table: &Obj<'py>,
    x: f64,
    y: f64,
    reach: Option<f64>,
) -> PyResult<Obj<'py>> {
    let py = view.py();
    let radius: f64 = view.getattr("_PICK_NEAR_PX")?.extract()?;
    let r = reach.unwrap_or(0.).max(radius);
    let slot = if r == radius {
        "_pick_near_memo"
    } else {
        "_edge_near_memo"
    };
    let m = view.call_method0("_np_mvp")?;
    let key = (
        view.call_method0("_placements_epoch")?,
        x.round_ties_even() as i64,
        y.round_ties_even() as i64,
        mvp_key(&m)?,
        r,
    )
        .into_pyobject(py)?;
    let memo = attr(view, slot)?;
    if !memo.is_none() && memo.get_item(0)?.eq(&key)? {
        return memo.get_item(1);
    }
    let mut need = view.call_method1(
        "_placements_under_px",
        (table.get_item(1)?, table.get_item(2)?, x, y, r),
    )?;
    let far = view
        .call_method1("_visible_depth_limit", (x, y))?
        .extract::<Option<f64>>()?;
    if need.len()? > 0 && (far.is_some() || view.getattr("_PICK_MIN_PX")?.extract::<f64>()? > 0.) {
        let (dist, small): (Obj, Obj) =
            view.call_method1("_footprint_rows", (table,))?.extract()?;
        if let Some(far) = far {
            need = need.get_item(
                dist.get_item(&need)?
                    .rich_compare(far, pyo3::basic::CompareOp::Le)?,
            )?;
        }
        if !small.is_none() {
            need = need.get_item(small.get_item(&need)?.call_method0("__invert__")?)?;
        }
    }
    if need.len()? > 0 {
        let iset = view.call_method1("_pick_instance_set", (table,))?;
        if !iset.is_none() {
            let edge: f64 = py
                .import("views.document_interaction")?
                .getattr("DocumentInteraction")?
                .getattr("GEDGE_REACH_PX")?
                .extract()?;
            need = iset.call_method1(
                "screen_filter",
                (
                    need.call_method1("astype", ("int64",))?,
                    m.getattr("T")?
                        .call_method0("ravel")?
                        .call_method0("tolist")?,
                    view.call_method0("width")?,
                    view.call_method0("height")?,
                    x,
                    y,
                    edge + 2.,
                    r + 2.,
                ),
            )?;
            need = py
                .import("numpy")?
                .call_method1("asarray", (need, "int64"))?;
        }
    }
    view.setattr(slot, (key, &need))?;
    Ok(need)
}
