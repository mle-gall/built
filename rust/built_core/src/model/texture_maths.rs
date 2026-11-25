// SPDX-License-Identifier: GPL-3.0-or-later
//! Texture maths (`built_core.texture`): the .skp projection basis and axes,
//! planar and fitted UVs, the map a face renders with, the tile size and
//! turn a fitted map shows, the map carried across a soft edge, the
//! reference triad, and the HLS colour maths of a tinted image.
//!
//! The same operations as the Python, in its order (`Vec3`'s are `glam`'s);
//! `x ** 0.5` stays `pow` (an opaque exponent keeps LLVM from making it
//! `sqrt`). The one exception: `fit_uv_affine` over more than
//! 13 points searched its best edge pair with a NumPy Gram matmul (BLAS), so
//! there a tie may be broken another way. `tests/test_texture_maths_parity.py`.

use glam::DVec3;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PySet, PyTuple};

use super::pyfloat;
use super::section_fill::{arr, flat_f64};
use crate::math::{xyz, Vec3};

/// |Z × n| below which a face maps with the world axes (measured).
const VERTICAL_TOLERANCE: f64 = 1e-3;

/// `projection_basis(normal)`: the .skp in-plane axes for a normal.
pub fn projection_basis(normal: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let [mut nx, mut ny, mut nz] = normal;
    let ln = (nx * nx + ny * ny + nz * nz).powf(std::hint::black_box(0.5));
    if ln > 1e-30 {
        (nx, ny, nz) = (nx / ln, ny / ln, nz / ln);
    }
    let (xx, xy) = (-ny, nx);
    let lx = (xx * xx + xy * xy).powf(std::hint::black_box(0.5));
    if lx < VERTICAL_TOLERANCE {
        return if nz > 0.0 {
            ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0])
        } else {
            ([-1.0, 0.0, 0.0], [0.0, 1.0, 0.0])
        };
    }
    let (xx, xy) = (xx / lx, xy / lx);
    ([xx, xy, 0.0], [-nz * xy, nz * xx, nx * xy - ny * xx])
}

fn rad(deg: f64) -> f64 {
    deg * (std::f64::consts::PI / 180.0)
}

/// `projection_axes(normal, rot)`: the basis turned in-plane by `rot`°.
pub fn projection_axes(normal: DVec3, rot: f64) -> (DVec3, DVec3) {
    let (xr, yr) = projection_basis(normal.to_array());
    let (u, v) = (DVec3::from_array(xr), DVec3::from_array(yr));
    if rot != 0.0 {
        let a = rad(rot);
        let (c, s) = (pyfloat::cos(a), pyfloat::sin(a));
        return (u * c + v * s, v * c - u * s);
    }
    (u, v)
}

fn vec(py: Python<'_>, v: DVec3) -> PyResult<Py<Vec3>> {
    Py::new(py, Vec3::from(v))
}

#[pyfunction]
fn texture_projection_basis(normal: &Bound<'_, PyAny>) -> PyResult<((f64, f64, f64), (f64, f64, f64))> {
    let n = [normal.get_item(0)?.extract()?, normal.get_item(1)?.extract()?, normal.get_item(2)?.extract()?];
    let (a, b) = projection_basis(n);
    Ok(((a[0], a[1], a[2]), (b[0], b[1], b[2])))
}

#[pyfunction]
#[pyo3(signature = (normal, rot=0.0))]
fn texture_projection_axes(py: Python<'_>, normal: &Bound<'_, PyAny>, rot: f64) -> PyResult<(Py<Vec3>, Py<Vec3>)> {
    let (u, v) = projection_axes(xyz(normal)?, rot);
    Ok((vec(py, u)?, vec(py, v)?))
}

#[pyfunction]
#[pyo3(signature = (normal, positions, sw, sh, rot=0.0))]
fn texture_planar_uv(normal: &Bound<'_, PyAny>, positions: &Bound<'_, PyAny>, sw: f64, sh: f64, rot: f64)
    -> PyResult<Vec<(f64, f64)>> {
    let (u, v) = projection_axes(xyz(normal)?, rot);
    let sw = if sw.abs() > 1e-9 { sw } else { 1.0 };
    let sh = if sh.abs() > 1e-9 { sh } else { 1.0 };
    positions.try_iter()?.map(|p| {
        let p = xyz(&p?)?;
        Ok((p.dot(u) / sw, p.dot(v) / sh))
    }).collect()
}

fn coords(p: &Bound<'_, PyAny>) -> PyResult<[f64; 3]> {
    if p.hasattr("x")? {
        return Ok([p.call_method0("x")?.extract()?, p.call_method0("y")?.extract()?, p.call_method0("z")?.extract()?]);
    }
    Ok([p.get_item(0)?.extract()?, p.get_item(1)?.extract()?, p.get_item(2)?.extract()?])
}

/// `fit_uv_affine(points, uvs)`: `[gu.xyz, u0, gv.xyz, v0]`, or `None`.
pub fn fit_uv_affine(pts: &[[f64; 3]], uvs: &[[f64; 2]]) -> Option<[f64; 8]> {
    let n = pts.len();
    if n < 3 || uvs.len() < n {
        return None;
    }
    let [x0, y0, z0] = pts[0];
    let d: Vec<[f64; 3]> = pts[1..].iter().map(|&[x, y, z]| [x - x0, y - y0, z - z0]).collect();
    let m = n - 1;
    let (bi, bj);
    if m <= 12 {
        let mut best = 1e-24;
        let (mut i0, mut j0) = (usize::MAX, usize::MAX);
        for i in 0..m {
            let [ax, ay, az] = d[i];
            for j in i + 1..m {
                let [bx, by, bz] = d[j];
                let cx = ay * bz - az * by;
                let cy = az * bx - ax * bz;
                let cz = ax * by - ay * bx;
                let c2 = cx * cx + cy * cy + cz * cz;
                if c2 > best {
                    best = c2;
                    (i0, j0) = (i, j);
                }
            }
        }
        if i0 == usize::MAX {
            return None;
        }
        (bi, bj) = (i0, j0);
    } else {
        // |a×b|² = |a|²|b|² − (a·b)², the first largest in row order.
        let dot = |a: &[f64; 3], b: &[f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let n2: Vec<f64> = d.iter().map(|a| dot(a, a)).collect();
        let (mut best, mut at) = (f64::NEG_INFINITY, (0, 0));
        for i in 0..m {
            for j in 0..m {
                let g = dot(&d[i], &d[j]);
                let c = n2[i] * n2[j] - g * g;
                if c > best {
                    best = c;
                    at = (i, j);
                }
            }
        }
        if best <= 1e-24 {
            return None;
        }
        (bi, bj) = at;
    }
    let [e1x, e1y, e1z] = d[bi];
    let [e2x, e2y, e2z] = d[bj];
    let g11 = e1x * e1x + e1y * e1y + e1z * e1z;
    let g12 = e1x * e2x + e1y * e2y + e1z * e2z;
    let g22 = e2x * e2x + e2y * e2y + e2z * e2z;
    let det = g11 * g22 - g12 * g12;
    if det.abs() < 1e-18 {
        return None;
    }
    let (i, j) = (bi + 1, bj + 1);
    let mut out = [0.0; 8];
    for k in 0..2 {
        let d1 = uvs[i][k] - uvs[0][k];
        let d2 = uvs[j][k] - uvs[0][k];
        let a = (d1 * g22 - d2 * g12) / det;
        let b = (d2 * g11 - d1 * g12) / det;
        let gx = e1x * a + e2x * b;
        let gy = e1y * a + e2y * b;
        let gz = e1z * a + e2z * b;
        let c = uvs[0][k] - (gx * x0 + gy * y0 + gz * z0);
        out[4 * k..4 * k + 4].copy_from_slice(&[gx, gy, gz, c]);
    }
    Some(out)
}

#[pyfunction]
fn texture_fit_uv_affine(points: &Bound<'_, PyAny>, uvs: &Bound<'_, PyAny>) -> PyResult<Option<Vec<f64>>> {
    let n = points.len()?;
    let nu = uvs.len()?;
    if n < 3 || nu < n {
        return Ok(None);
    }
    let pts: Vec<[f64; 3]> = points.try_iter()?.map(|p| {
        let p = p?;
        if p.hasattr("x")? { Ok(xyz(&p)?.to_array()) } else { coords(&p) }
    }).collect::<PyResult<_>>()?;
    let uv: Vec<[f64; 2]> = uvs.try_iter()?.take(n).map(|q| {
        let q = q?;
        Ok([q.get_item(0)?.extract()?, q.get_item(1)?.extract()?])
    }).collect::<PyResult<_>>()?;
    Ok(fit_uv_affine(&pts, &uv).map(|o| o.to_vec()))
}

#[pyfunction]
fn texture_affine_uv(uvw: &Bound<'_, PyAny>, positions: &Bound<'_, PyAny>) -> PyResult<Vec<(f64, f64)>> {
    let g: Vec<f64> = (0..8).map(|i| uvw.get_item(i)?.extract()).collect::<PyResult<_>>()?;
    positions.try_iter()?.map(|p| {
        let [x, y, z] = coords(&p?)?;
        Ok((g[0] * x + g[1] * y + g[2] * z + g[3], g[4] * x + g[5] * y + g[6] * z + g[7]))
    }).collect()
}

fn get_f64(tex: &Bound<'_, PyAny>, key: &str, default: f64) -> PyResult<Option<f64>> {
    let v = tex.call_method1("get", (key, default))?;
    if !v.is_truthy()? {
        return Ok(None);
    }
    Ok(Some(v.extract()?))
}

/// `face_uv_axes(tex, normal)`: `(gu, cu, gv, cv)`.
#[pyfunction]
fn texture_face_uv_axes<'py>(py: Python<'py>, tex: &Bound<'py, PyAny>, normal: &Bound<'py, PyAny>)
    -> PyResult<(Py<Vec3>, f64, Py<Vec3>, f64)> {
    let uvw = tex.call_method1("get", ("uvw",))?;
    if uvw.is_truthy()? {
        let g: Vec<f64> = (0..8).map(|i| uvw.get_item(i)?.extract()).collect::<PyResult<_>>()?;
        return Ok((vec(py, DVec3::new(g[0], g[1], g[2]))?, g[3], vec(py, DVec3::new(g[4], g[5], g[6]))?, g[7]));
    }
    let rot = get_f64(tex, "rot", 0.0)?.unwrap_or(0.0);
    let (u, v) = projection_axes(xyz(normal)?, rot);
    let sw = get_f64(tex, "sw", 1.0)?.unwrap_or(1.0);
    let sh = get_f64(tex, "sh", 1.0)?.unwrap_or(1.0);
    Ok((vec(py, u / sw)?, 0.0, vec(py, v / sh)?, 0.0))
}

/// `placement_of(tex, normal)`: `(sw, sh, rot)` a fitted map shows on the
/// face of `normal`, or `None`.
#[pyfunction]
fn texture_placement_of(tex: &Bound<'_, PyAny>, normal: &Bound<'_, PyAny>) -> PyResult<Option<(f64, f64, f64)>> {
    let uvw = tex.call_method1("get", ("uvw",))?;
    if !uvw.is_truthy()? || uvw.len()? != 8 {
        return Ok(None);
    }
    let g: Vec<f64> = (0..8).map(|i| uvw.get_item(i)?.extract()).collect::<PyResult<_>>()?;
    let n = xyz(normal)?;
    if n.length_squared() < 1e-18 {
        return Ok(None);
    }
    let n = n.normalize_or_zero();
    let gu = DVec3::new(g[0], g[1], g[2]);
    let gv = DVec3::new(g[4], g[5], g[6]);
    let gu = gu - n * gu.dot(n);
    let gv = gv - n * gv.dot(n);
    let g11 = gu.dot(gu);
    let g12 = gu.dot(gv);
    let g22 = gv.dot(gv);
    let det = g11 * g22 - g12 * g12;
    if det.abs() < 1e-24 {
        return Ok(None);
    }
    let e_u = (gu * g22 - gv * g12) / det;
    let e_v = (gv * g11 - gu * g12) / det;
    let (sw, sh) = (e_u.length(), e_v.length());
    if sw < 1e-9 || sh < 1e-9 {
        return Ok(None);
    }
    let (u, v) = projection_axes(n, 0.0);
    let rot = e_u.dot(v).atan2(e_u.dot(u)) * (180.0 / std::f64::consts::PI);
    Ok(Some((sw, sh, rot)))
}

/// `_rotated_map(gu, cu, gv, cv, axis_point, axis_dir, cos_t, sin_t)`: the
/// map carried across a hinge.
#[pyfunction]
#[allow(clippy::too_many_arguments)]
fn texture_rotated_map(py: Python<'_>, gu: &Bound<'_, PyAny>, cu: f64, gv: &Bound<'_, PyAny>, cv: f64,
                       axis_point: &Bound<'_, PyAny>, axis_dir: &Bound<'_, PyAny>, cos_t: f64, sin_t: f64)
    -> PyResult<(Py<Vec3>, f64, Py<Vec3>, f64)> {
    let (gu2, cu2, gv2, cv2) = rotated((xyz(gu)?, cu, xyz(gv)?, cv), xyz(axis_point)?, xyz(axis_dir)?, cos_t, sin_t);
    Ok((vec(py, gu2)?, cu2, vec(py, gv2)?, cv2))
}

type Map = (DVec3, f64, DVec3, f64);

fn rotated((gu, cu, gv, cv): Map, a: DVec3, d: DVec3, cos_t: f64, sin_t: f64) -> Map {
    let rot = |v: DVec3| v * cos_t + d.cross(v) * sin_t + d * (d.dot(v) * (1.0 - cos_t));
    let (gu2, gv2) = (rot(gu), rot(gv));
    (gu2, cu + gu.dot(a) - gu2.dot(a), gv2, cv + gv.dot(a) - gv2.dot(a))
}

/// `continuous_maps(mesh, faces, seed, tex)`: `{face.key: tex dict with
/// uvw}` for the faces of `faces` reached from `seed` across soft edges,
/// the map turned about each hinge into the next facet's plane (the
/// classic way to paint a cylinder). The mesh answers through its own
/// methods (`find_edge`, the faces' loops and normals); the maths is
/// glam's, as `Vec3`'s is.
#[pyfunction]
fn texture_continuous_maps<'py>(py: Python<'py>, mesh: &Bound<'py, PyAny>, faces: &Bound<'py, PyAny>,
                                seed: &Bound<'py, PyAny>, tex: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyDict>> {
    let faces: Vec<Bound<'py, PyAny>> = faces.try_iter()?.collect::<PyResult<_>>()?;
    let result = PyDict::new(py);
    if seed.is_none() || faces.is_empty() {
        return Ok(result);
    }
    let ids = PySet::empty(py)?;
    for f in &faces {
        ids.add(f.getattr("key")?)?;
    }
    let (gu, cu, gv, cv) = texture_face_uv_axes(py, tex, &seed.call_method0("normal")?)?;
    let mut maps: Vec<Map> = vec![(xyz(gu.bind(py))?, cu, xyz(gv.bind(py))?, cv)];
    // face key → its row in `maps`.
    let out = PyDict::new(py);
    out.set_item(seed.getattr("key")?, 0usize)?;
    let mut stack = vec![seed.clone()];
    while let Some(f) = stack.pop() {
        let here = maps[out.get_item(f.getattr("key")?)?.unwrap().extract::<usize>()?];
        let n1 = xyz(&f.call_method0("normal")?)?;
        if n1.length_squared() < 1e-18 {
            continue;
        }
        let n1 = n1.normalize_or_zero();
        let mut loops = vec![f.getattr("loop")?];
        for lp in f.getattr("hole_loops")?.try_iter()? {
            loops.push(lp?);
        }
        for lp in loops {
            let n = lp.len()?;
            for i in 0..n {
                let e = mesh.call_method1("find_edge", (lp.get_item(i)?, lp.get_item((i + 1) % n)?))?;
                if e.is_none() || !e.getattr("soft")?.is_truthy()? {
                    continue;
                }
                for g in e.getattr("faces")?.try_iter()? {
                    let g = g?;
                    let key = g.getattr("key")?;
                    if g.eq(&f)? || !ids.contains(&key)? || out.contains(&key)? {
                        continue;
                    }
                    let n2 = xyz(&g.call_method0("normal")?)?;
                    let a = xyz(&e.getattr("a")?)?;
                    let d = xyz(&e.getattr("b")?)? - a;
                    if n2.length_squared() < 1e-18 || d.length_squared() < 1e-18 {
                        continue;
                    }
                    let (n2, d) = (n2.normalize_or_zero(), d.normalize_or_zero());
                    let cos_t = n1.dot(n2);
                    let sin_t = d.dot(n1.cross(n2));
                    if cos_t < -0.999 {
                        continue; // folded back: no hinge
                    }
                    out.set_item(&key, maps.len())?;
                    maps.push(rotated(here, a, d, cos_t, sin_t));
                    stack.push(g);
                }
            }
        }
    }
    let base = PyDict::new(py);
    for (k, v) in tex.call_method0("items")?.try_iter()?.map(|kv| kv.and_then(|kv| kv.extract::<(Bound<'py, PyAny>, Bound<'py, PyAny>)>())) .collect::<PyResult<Vec<_>>>()? {
        if !(k.eq("uvw")? || k.eq("rot")?) {
            base.set_item(k, v)?;
        }
    }
    for f in &faces {
        let key = f.getattr("key")?;
        let Some(row) = out.get_item(&key)? else { continue };
        let (gu, cu, gv, cv) = maps[row.extract::<usize>()?];
        let t = base.copy()?;
        t.set_item("uvw", PyList::new(py, [gu.x, gu.y, gu.z, cu, gv.x, gv.y, gv.z, cv])?)?;
        result.set_item(key, t)?;
    }
    Ok(result)
}

/// `uv_reference_points(points, normal=None)`: a right-angled triad on the
/// face's plane, sized to it, or `None`.
#[pyfunction]
#[pyo3(signature = (points, normal=None))]
fn texture_uv_reference_points(py: Python<'_>, points: &Bound<'_, PyAny>, normal: Option<&Bound<'_, PyAny>>)
    -> PyResult<Option<(Py<Vec3>, Py<Vec3>, Py<Vec3>)>> {
    let pts: Vec<DVec3> = points.try_iter()?.map(|p| xyz(&p?)).collect::<PyResult<_>>()?;
    if pts.len() < 3 {
        return Ok(None);
    }
    let mut centre = DVec3::ZERO;
    for p in &pts {
        centre += *p;
    }
    centre /= pts.len() as f64;
    let mut n = match normal.filter(|n| !n.is_none()) {
        Some(n) => xyz(n)?,
        None => DVec3::ZERO,
    };
    if normal.is_none_or(|n| n.is_none()) || n.length_squared() < 1e-18 {
        n = DVec3::ZERO;
        let count = pts.len();
        for i in 0..count {
            let (a, b) = (pts[i], pts[(i + 1) % count]);
            n += DVec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
        }
    }
    if n.length_squared() < 1e-18 {
        return Ok(None);
    }
    let nn = Bound::new(py, Vec3::from(n.normalize_or_zero()))?;
    let (e1, e2) = crate::tools::plane::drawing_plane_axes(nn.as_any(), None)?;
    let (e1, e2) = (DVec3::from_array(e1.coords()), DVec3::from_array(e2.coords()));
    let span = pts.iter().map(|p| (*p - centre).length()).fold(f64::NEG_INFINITY, f64::max);
    if span < 1e-9 {
        return Ok(None);
    }
    Ok(Some((vec(py, centre)?, vec(py, centre + e1 * span)?, vec(py, centre + e2 * span)?)))
}

// ---- Colour: HLS, vectorised as colorsys --------------------------------

pub fn rgb_to_hls(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let maxc = r.max(g).max(b);
    let minc = r.min(g).min(b);
    let lum = (maxc + minc) / 2.0;
    let delta = maxc - minc;
    let nz = delta > 1e-12;
    let denom = if lum <= 0.5 { maxc + minc } else { 2.0 - maxc - minc };
    let sat = if nz && denom > 1e-12 { delta / denom } else { 0.0 };
    let safe = if nz { delta } else { 1.0 };
    let rc = (maxc - r) / safe;
    let gc = (maxc - g) / safe;
    let bc = (maxc - b) / safe;
    let hue = if maxc == r {
        bc - gc
    } else if maxc == g {
        2.0 + rc - bc
    } else {
        4.0 + gc - rc
    };
    let hue = if nz { pyfloat::modulo(hue / 6.0, 1.0) } else { 0.0 };
    (hue, lum, sat)
}

pub fn hls_to_rgb(hue: f64, lum: f64, sat: f64) -> [f64; 3] {
    let m2 = if lum <= 0.5 { lum * (1.0 + sat) } else { lum + sat - lum * sat };
    let m1 = 2.0 * lum - m2;
    let channel = |h: f64| {
        let h = pyfloat::modulo(h, 1.0);
        if h < 1.0 / 6.0 {
            m1 + (m2 - m1) * h * 6.0
        } else if h < 0.5 {
            m2
        } else if h < 2.0 / 3.0 {
            m1 + (m2 - m1) * (2.0 / 3.0 - h) * 6.0
        } else {
            m1
        }
    };
    [channel(hue + 1.0 / 3.0), channel(hue), channel(hue - 1.0 / 3.0)]
}

fn shape_of(a: &Bound<'_, PyAny>) -> PyResult<Vec<usize>> {
    a.getattr("shape")?.extract()
}

/// `rgb_to_hls(rgb)` over an (..., 3) array: `(hue, lum, sat)` arrays.
#[pyfunction]
fn texture_rgb_to_hls<'py>(py: Python<'py>, rgb: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyTuple>> {
    let np = py.import("numpy")?;
    let a = np.call_method1("asarray", (rgb, np.getattr("float64")?))?;
    let mut shape = shape_of(&a)?;
    shape.pop();
    let v = flat_f64(&a)?;
    let (mut h, mut l, mut s) = (Vec::new(), Vec::new(), Vec::new());
    for c in v.chunks_exact(3) {
        let (x, y, z) = rgb_to_hls(c[0], c[1], c[2]);
        h.push(x);
        l.push(y);
        s.push(z);
    }
    PyTuple::new(py, [arr(py, h, &shape)?, arr(py, l, &shape)?, arr(py, s, &shape)?])
}

/// `hls_to_rgb(hue, lum, sat)`: an (..., 3) array.
#[pyfunction]
fn texture_hls_to_rgb<'py>(py: Python<'py>, hue: &Bound<'py, PyAny>, lum: &Bound<'py, PyAny>, sat: &Bound<'py, PyAny>)
    -> PyResult<Bound<'py, PyAny>> {
    let np = py.import("numpy")?;
    let b = np.call_method1("broadcast_arrays", (np.call_method1("asarray", (hue, np.getattr("float64")?))?,
                                                 np.call_method1("asarray", (lum, np.getattr("float64")?))?,
                                                 np.call_method1("asarray", (sat, np.getattr("float64")?))?))?;
    let mut shape = shape_of(&b.get_item(0)?)?;
    let (h, l, s) = (flat_f64(&b.get_item(0)?)?, flat_f64(&b.get_item(1)?)?, flat_f64(&b.get_item(2)?)?);
    let out: Vec<f64> = (0..h.len()).flat_map(|i| hls_to_rgb(h[i], l[i], s[i])).collect();
    shape.push(3);
    arr(py, out, &shape)
}

/// The colourize pass over RGBA pixels (`built_core.texture.colorize_image`):
/// every pixel's hue, lightness and saturation moved by the delta between
/// the image average (`avg`, of the visible pixels) and `target` (0..255),
/// or, `ctype` 1, hue and saturation replaced. Alpha kept.
#[pyfunction]
fn texture_colorize_pixels<'py>(py: Python<'py>, rgba: &[u8], avg: [f64; 3], target: [f64; 3], ctype: i64)
    -> PyResult<Bound<'py, PyAny>> {
    let (h0, l0, s0) = rgb_to_hls(avg[0], avg[1], avg[2]);
    let (ht, lt, st) = rgb_to_hls(target[0] / 255.0, target[1] / 255.0, target[2] / 255.0);
    let clip = |x: f64, lo: f64, hi: f64| x.max(lo).min(hi);
    let out: Vec<u8> = py.detach(|| {
        let mut out = Vec::with_capacity(rgba.len());
        for px in rgba.chunks_exact(4) {
            let (r, g, b) = (px[0] as f64 / 255.0, px[1] as f64 / 255.0, px[2] as f64 / 255.0);
            let (mut hue, mut lum, mut sat) = rgb_to_hls(r, g, b);
            if ctype == 1 {
                hue = ht;
                sat = st;
            } else {
                hue = pyfloat::modulo(hue + (ht - h0), 1.0);
                sat = clip(sat + (st - s0), 0.0, 1.0);
            }
            lum = clip(lum + (lt - l0), 0.0, 1.0);
            for c in hls_to_rgb(hue, lum, sat) {
                out.push(clip(c * 255.0 + 0.5, 0.0, 255.0) as u8);
            }
            out.push(px[3]);
        }
        out
    });
    Ok(pyo3::types::PyBytes::new(py, &out).into_any())
}

/// The mean colour of the visible pixels (alpha > 0) of RGBA bytes, in
/// 0..1, summed row after row as NumPy's `rgb[vis].mean(axis=0)`; of every
/// pixel when none is visible.
#[pyfunction]
fn texture_mean_visible(rgba: &[u8]) -> (f64, f64, f64) {
    let (mut sum, mut n) = ([0.0f64; 3], 0usize);
    for px in rgba.chunks_exact(4).filter(|p| p[3] > 0) {
        for k in 0..3 {
            sum[k] += px[k] as f64 / 255.0;
        }
        n += 1;
    }
    if n == 0 {
        for px in rgba.chunks_exact(4) {
            for k in 0..3 {
                sum[k] += px[k] as f64 / 255.0;
            }
            n += 1;
        }
    }
    let n = n as f64;
    (sum[0] / n, sum[1] / n, sum[2] / n)
}

/// `flattened_texture(tex, normal=None)`: `tex` without its per-face map,
/// keeping the look it had on the plane of `normal` as a planar projection.
#[pyfunction]
#[pyo3(signature = (tex, normal=None))]
fn texture_flattened<'py>(py: Python<'py>, tex: &Bound<'py, PyAny>, normal: Option<&Bound<'py, PyAny>>)
    -> PyResult<Bound<'py, pyo3::types::PyDict>> {
    let flat = pyo3::types::PyDict::new(py);
    for item in tex.call_method0("items")?.try_iter()? {
        let (k, v): (Bound<PyAny>, Bound<PyAny>) = item?.extract()?;
        if !k.eq("uvw")? {
            flat.set_item(k, v)?;
        }
    }
    let Some(normal) = normal.filter(|n| !n.is_none()) else { return Ok(flat) };
    if !tex.call_method1("get", ("uvw",))?.is_truthy()? {
        return Ok(flat);
    }
    let Some((sw, sh, rot)) = texture_placement_of(tex, normal)? else { return Ok(flat) };
    flat.set_item("sw", sw)?;
    flat.set_item("sh", sh)?;
    if rot.abs() > 1e-6 {
        flat.set_item("rot", pyfloat::round(rot, 6))?;
    } else if flat.contains("rot")? {
        flat.del_item("rot")?;
    }
    Ok(flat)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(texture_projection_basis, m)?)?;
    m.add_function(wrap_pyfunction!(texture_projection_axes, m)?)?;
    m.add_function(wrap_pyfunction!(texture_planar_uv, m)?)?;
    m.add_function(wrap_pyfunction!(texture_fit_uv_affine, m)?)?;
    m.add_function(wrap_pyfunction!(texture_affine_uv, m)?)?;
    m.add_function(wrap_pyfunction!(texture_face_uv_axes, m)?)?;
    m.add_function(wrap_pyfunction!(texture_placement_of, m)?)?;
    m.add_function(wrap_pyfunction!(texture_rotated_map, m)?)?;
    m.add_function(wrap_pyfunction!(texture_continuous_maps, m)?)?;
    m.add_function(wrap_pyfunction!(texture_uv_reference_points, m)?)?;
    m.add_function(wrap_pyfunction!(texture_rgb_to_hls, m)?)?;
    m.add_function(wrap_pyfunction!(texture_hls_to_rgb, m)?)?;
    m.add_function(wrap_pyfunction!(texture_colorize_pixels, m)?)?;
    m.add_function(wrap_pyfunction!(texture_mean_visible, m)?)?;
    m.add_function(wrap_pyfunction!(texture_flattened, m)?)?;
    Ok(())
}
