// SPDX-License-Identifier: GPL-3.0-or-later
//! The hidden-line pass's view maths (`core.hlr`): the camera basis and
//! camera space, how deep a cut solid runs through its plane, the section
//! cap's depth plane and ring edges, the layer each ring closes, and the
//! perspective drawing's projective map with its clips against the view
//! pyramid.
//!
//! Ported operation for operation from the NumPy. Where NumPy went through
//! BLAS or LAPACK in an order we cannot name, the plain or fused order is
//! used and the parity test states a tolerance: `solid_depth` (`einsum`),
//! the cap's `linalg.solve`, and the clip-space products of the perspective
//! (`(N, 3) @ (3, 4)`, a fused chain at most sizes, measured).
//! `tests/test_hlr_rest_parity.py`.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

use super::section_cut::chain_loops;
use super::section_fill::{arr, cross, flat_f64, norm3};

type P = [f64; 3];
type C4 = [f64; 4];

fn dot3(a: &P, b: &P) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sub(a: &P, b: &P) -> P {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn xyz(obj: &Bound<'_, PyAny>) -> PyResult<P> {
    Ok(crate::math::xyz(obj)?.to_array())
}

fn rows3(flat: &[f64]) -> Vec<P> {
    flat.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect()
}

fn vec_arr<'py>(py: Python<'py>, a: &P) -> PyResult<Bound<'py, PyAny>> {
    arr(py, a.to_vec(), &[3])
}

/// Right/up/forward unit vectors and the eye, as the orbit camera's lookAt
/// derives them.
pub fn basis(eye: P, target: P, up: P) -> (P, P, P, P) {
    let mut f = sub(&target, &eye);
    let l = norm3(&f).max(1e-12);
    f = [f[0] / l, f[1] / l, f[2] / l];
    let mut r = cross(&f, &up);
    let l = norm3(&r).max(1e-12);
    r = [r[0] / l, r[1] / l, r[2] / l];
    let u = cross(&r, &f);
    (eye, r, u, f)
}

/// World to camera space: x right, y up, z the depth away from the eye.
pub fn to_cam(p: &P, eye: &P, r: &P, u: &P, f: &P) -> P {
    let d = sub(p, eye);
    [dot3(&d, r), dot3(&d, u), dot3(&d, f)]
}

#[pyfunction]
fn hlr_camera_basis<'py>(py: Python<'py>, camera: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyTuple>> {
    let eye = xyz(&camera.call_method0("eye")?)?;
    let target = xyz(&camera.getattr("target")?)?;
    let up = if camera.hasattr("up_vector")? { camera.call_method0("up_vector")? } else { camera.getattr("up")? };
    let (e, r, u, f) = basis(eye, target, xyz(&up)?);
    PyTuple::new(py, [vec_arr(py, &e)?, vec_arr(py, &r)?, vec_arr(py, &u)?, vec_arr(py, &f)?])
}

fn p3(obj: &Bound<'_, PyAny>) -> PyResult<P> {
    let v = flat_f64(obj)?;
    Ok([v[0], v[1], v[2]])
}

/// `_to_cam(points, eye, right, up, fwd)`: (N, 3) world to camera space.
#[pyfunction]
fn hlr_to_cam<'py>(py: Python<'py>, points: &Bound<'py, PyAny>, eye: &Bound<'py, PyAny>, right: &Bound<'py, PyAny>,
                   up: &Bound<'py, PyAny>, fwd: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let (e, r, u, f) = (p3(eye)?, p3(right)?, p3(up)?, p3(fwd)?);
    let pts = rows3(&flat_f64(points)?);
    let out: Vec<f64> = pts.iter().flat_map(|p| to_cam(p, &e, &r, &u, &f)).collect();
    arr(py, out, &[pts.len(), 3])
}

/// How far the solid of `tris` runs through the plane around `ring`,
/// square to it: the nearest hit along `normal` and against it, from the
/// ring's centre. `None` when a ray finds nothing.
pub fn solid_depth(ring: &[P], tris: &[[P; 3]], normal: P) -> Option<f64> {
    if ring.len() < 3 || tris.is_empty() {
        return None;
    }
    let l = norm3(&normal);
    let l = if l == 0.0 { 1.0 } else { l };
    let n = [normal[0] / l, normal[1] / l, normal[2] / l];
    let mut origin = [0.0; 3];
    for p in ring {
        for k in 0..3 {
            origin[k] += p[k];
        }
    }
    let m = ring.len() as f64;
    let origin = [origin[0] / m, origin[1] / m, origin[2] / m];
    let mut reach = 0.0;
    for d in [n, [-n[0], -n[1], -n[2]]] {
        let mut best: Option<f64> = None;
        for t in tris {
            let v0 = t[0];
            let e1 = sub(&t[1], &v0);
            let e2 = sub(&t[2], &v0);
            let pv = cross(&d, &e2);
            let det = dot3(&e1, &pv);
            let ok = det.abs() > 1e-12;
            let inv = if ok { 1.0 / det } else { 0.0 };
            let tv = sub(&origin, &v0);
            let u = dot3(&tv, &pv) * inv;
            let qv = cross(&tv, &e1);
            let v = dot3(&qv, &d) * inv;
            let tt = dot3(&e2, &qv) * inv;
            if ok && u >= -1e-9 && v >= -1e-9 && u + v <= 1.0 + 1e-9 && tt > 1e-9 {
                best = Some(best.map_or(tt, |b: f64| b.min(tt)));
            }
        }
        reach += best?;
    }
    Some(reach)
}

#[pyfunction]
fn hlr_solid_depth(ring: &Bound<'_, PyAny>, tris: &Bound<'_, PyAny>, normal: &Bound<'_, PyAny>) -> PyResult<Option<f64>> {
    let ring = rows3(&flat_f64(ring)?);
    let t = rows3(&flat_f64(tris)?);
    let tris: Vec<[P; 3]> = t.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
    Ok(solid_depth(&ring, &tris, p3(normal)?))
}

/// The 3×3 system solved as LAPACK's `dgesv` does: LU with partial
/// pivoting, then the two triangular solves.
fn solve3(mut a: [[f64; 3]; 3], mut b: [f64; 3]) -> Option<[f64; 3]> {
    for k in 0..3 {
        let mut p = k;
        for i in k + 1..3 {
            if a[i][k].abs() > a[p][k].abs() {
                p = i;
            }
        }
        if a[p][k] == 0.0 {
            return None;
        }
        a.swap(k, p);
        b.swap(k, p);
        let r = 1.0 / a[k][k];
        for i in k + 1..3 {
            a[i][k] *= r;
            for j in k + 1..3 {
                a[i][j] -= a[i][k] * a[k][j];
            }
        }
    }
    for i in 0..3 {
        for j in 0..i {
            b[i] -= a[i][j] * b[j];
        }
    }
    for i in (0..3).rev() {
        for j in i + 1..3 {
            b[i] -= a[i][j] * b[j];
        }
        b[i] /= a[i][i];
    }
    Some(b)
}

/// `cap_depth_affine(plane, eye, right, up, fwd)`: `(alpha, beta, gamma)`
/// with the cut plane's camera depth `alpha·x + beta·y + gamma`, or `None`
/// seen edge-on.
#[pyfunction]
fn hlr_cap_depth_affine<'py>(py: Python<'py>, plane: &Bound<'py, PyAny>, eye: &Bound<'py, PyAny>,
                             right: &Bound<'py, PyAny>, up: &Bound<'py, PyAny>, fwd: &Bound<'py, PyAny>)
    -> PyResult<Option<Bound<'py, PyAny>>> {
    let (u, v) = crate::tools::plane::drawing_plane_axes(&plane.getattr("normal")?, None)?;
    let (u, v) = (u.coords(), v.coords());
    let p = xyz(&plane.getattr("point")?)?;
    let (e, r, w, f) = (p3(eye)?, p3(right)?, p3(up)?, p3(fwd)?);
    let pts = [p, [p[0] + u[0], p[1] + u[1], p[2] + u[2]], [p[0] + v[0], p[1] + v[1], p[2] + v[2]]];
    let c: Vec<P> = pts.iter().map(|q| to_cam(q, &e, &r, &w, &f)).collect();
    let area2 = (c[1][0] - c[0][0]) * (c[2][1] - c[0][1]) - (c[1][1] - c[0][1]) * (c[2][0] - c[0][0]);
    if area2.abs() < 1e-9 {
        return Ok(None);
    }
    let m = [[c[0][0], c[0][1], 1.0], [c[1][0], c[1][1], 1.0], [c[2][0], c[2][1], 1.0]];
    match solve3(m, [c[0][2], c[1][2], c[2][2]]) {
        Some(x) => Ok(Some(vec_arr(py, &x)?)),
        None => Ok(None),
    }
}

/// `cap_ring_edges(loops)`: every ring as one pair of (K, 2) arrays P→Q.
#[pyfunction]
fn hlr_cap_ring_edges<'py>(py: Python<'py>, loops: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyTuple>> {
    let (mut p, mut q) = (Vec::new(), Vec::new());
    let mut any = false;
    for ring in loops.try_iter()? {
        let ring = ring?;
        let np = py.import("numpy")?;
        let a = np.call_method1("asarray", (&ring, np.getattr("float64")?))?;
        let cols: usize = a.getattr("shape")?.get_item(1)?.extract()?;
        let flat = flat_f64(&a)?;
        let pts: Vec<[f64; 2]> = flat.chunks_exact(cols).map(|c| [c[0], c[1]]).collect();
        any = true;
        for i in 0..pts.len() {
            p.extend(pts[i]);
            q.extend(pts[(i + 1) % pts.len()]);
        }
    }
    if !any {
        return PyTuple::new(py, [py.None(), py.None()]);
    }
    let k = p.len() / 2;
    PyTuple::new(py, [arr(py, p, &[k, 2])?, arr(py, q, &[k, 2])?])
}

/// `closing_tags(cut_edges, cut_tags)`: each chord given the layer of the
/// ring it closes (see `core.hlr.closing_tags`).
#[pyfunction]
fn hlr_closing_tags<'py>(py: Python<'py>, cut_edges: &Bound<'py, PyAny>, cut_tags: &Bound<'py, PyAny>)
    -> PyResult<Bound<'py, PyList>> {
    let tags: Vec<Bound<'py, PyAny>> = cut_tags.try_iter()?.collect::<PyResult<_>>()?;
    let ids = PyDict::new(py);
    let mut keys: Vec<Bound<'py, PyAny>> = Vec::new();
    let mut tag_id = Vec::with_capacity(tags.len());
    for t in &tags {
        let id = match ids.get_item(t)? {
            Some(i) => i.extract::<usize>()?,
            None => {
                ids.set_item(t, keys.len())?;
                keys.push(t.clone());
                keys.len() - 1
            }
        };
        tag_id.push(id);
    }
    let mut answer = tags.clone();
    if keys.len() < 2 {
        return PyList::new(py, answer);
    }
    let flat = flat_f64(cut_edges)?;
    let edges: Vec<[P; 2]> = flat.chunks_exact(6).map(|c| [[c[0], c[1], c[2]], [c[3], c[4], c[5]]]).collect();
    let mut loose: Vec<usize> = Vec::new();
    for t in 0..keys.len() {
        let idx: Vec<usize> = (0..tags.len()).filter(|&k| tag_id[k] == t).collect();
        let mine: Vec<[P; 2]> = idx.iter().map(|&k| edges[k]).collect();
        let (_, rings) = chain_loops(&mine, None);
        let mut inside = vec![false; idx.len()];
        for (_, rows) in &rings {
            for &j in rows {
                inside[j] = true;
            }
        }
        loose.extend(idx.iter().enumerate().filter(|(j, _)| !inside[*j]).map(|(_, &k)| k));
    }
    let mut kinds: Vec<usize> = loose.iter().map(|&k| tag_id[k]).collect();
    kinds.sort_unstable();
    kinds.dedup();
    if loose.is_empty() || kinds.len() < 2 {
        return PyList::new(py, answer);
    }
    let mixed: Vec<[P; 2]> = loose.iter().map(|&k| edges[k]).collect();
    let (_, rings) = chain_loops(&mixed, None);
    let names: Vec<String> = keys.iter().map(|k| k.str().map(|s| s.to_string())).collect::<PyResult<_>>()?;
    for (_, ring) in &rings {
        let mut length: Vec<(usize, f64)> = Vec::new();
        for &j in ring {
            let k = loose[j];
            let e = &edges[k];
            let d = norm3(&sub(&e[1], &e[0]));
            match length.iter_mut().find(|(t, _)| *t == tag_id[k]) {
                Some((_, l)) => *l += d,
                None => length.push((tag_id[k], d)),
            }
        }
        // The most outline; names break a tie (sorted by name, first max).
        length.sort_by(|a, b| names[a.0].cmp(&names[b.0]));
        let mut winner = length[0];
        for &c in &length[1..] {
            if c.1 > winner.1 {
                winner = c;
            }
        }
        for &j in ring {
            tag_id[loose[j]] = winner.0;
            answer[loose[j]] = keys[winner.0].clone();
        }
    }
    PyList::new(py, answer)
}

// ---- Perspective: the view pyramid in clip space ----------------------------

/// The kept side of each pyramid plane, `a·p ≥ 0`: left, right, bottom,
/// top, near. Its rows are ±1 and 0, so `a·p` is one exact sum.
fn sides(p: &C4) -> [f64; 5] {
    [p[0] + p[3], -p[0] + p[3], p[1] + p[3], -p[1] + p[3], p[2] + p[3]]
}

/// Sutherland–Hodgman against the view pyramid.
pub fn clip_polygon(mut poly: Vec<C4>) -> Vec<C4> {
    for a in 0..5 {
        let mut out = Vec::with_capacity(poly.len() + 2);
        let n = poly.len();
        for i in 0..n {
            let (p, q) = (poly[i], poly[(i + 1) % n]);
            let (dp, dq) = (sides(&p)[a], sides(&q)[a]);
            if dp >= 0.0 {
                out.push(p);
            }
            if (dp >= 0.0) != (dq >= 0.0) {
                let t = dp / (dp - dq);
                out.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t, p[2] + (q[2] - p[2]) * t,
                          p[3] + (q[3] - p[3]) * t]);
            }
        }
        poly = out;
        if poly.len() < 3 {
            return Vec::new();
        }
    }
    poly
}

/// (T, 3, 4) clip-space triangles: those inside first, then the fans of
/// the ones crossing a side.
pub fn clip_triangles(tris: &[[C4; 3]]) -> Vec<[C4; 3]> {
    let d: Vec<[[f64; 5]; 3]> = tris.iter().map(|t| [sides(&t[0]), sides(&t[1]), sides(&t[2])]).collect();
    let inside: Vec<bool> = d.iter().map(|t| t.iter().all(|v| v.iter().all(|&x| x >= 0.0))).collect();
    let outside: Vec<bool> = d.iter().map(|t| (0..5).any(|a| t.iter().all(|v| v[a] < 0.0))).collect();
    let mut out: Vec<[C4; 3]> = (0..tris.len()).filter(|&i| inside[i]).map(|i| tris[i]).collect();
    for i in 0..tris.len() {
        if inside[i] || outside[i] {
            continue;
        }
        let poly = clip_polygon(tris[i].to_vec());
        for k in 1..poly.len().saturating_sub(1) {
            out.push([poly[0], poly[k], poly[k + 1]]);
        }
    }
    out
}

fn nan_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.max(b) }
}

fn nan_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.min(b) }
}

/// Clip-space segments: the parts inside the pyramid.
pub fn clip_segments(a: &[C4], b: &[C4]) -> (Vec<C4>, Vec<C4>) {
    let (mut pa, mut pb) = (Vec::new(), Vec::new());
    for (p, q) in a.iter().zip(b) {
        let (sp, sq) = (sides(p), sides(q));
        let (mut t0, mut t1, mut keep) = (0.0f64, 1.0f64, true);
        for k in 0..5 {
            let (da, db) = (sp[k], sq[k]);
            keep &= !(da < 0.0 && db < 0.0);
            let t = da / (da - db);
            if da < 0.0 {
                t0 = nan_max(t0, t);
            }
            if db < 0.0 {
                t1 = nan_min(t1, t);
            }
        }
        if !(keep && t0 < t1) {
            continue;
        }
        let d = [q[0] - p[0], q[1] - p[1], q[2] - p[2], q[3] - p[3]];
        pa.push([p[0] + d[0] * t0, p[1] + d[1] * t0, p[2] + d[2] * t0, p[3] + d[3] * t0]);
        pb.push([p[0] + d[0] * t1, p[1] + d[1] * t1, p[2] + d[2] * t1, p[3] + d[3] * t1]);
    }
    (pa, pb)
}

fn rows4(flat: &[f64]) -> Vec<C4> {
    flat.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect()
}

fn c4_arr<'py>(py: Python<'py>, rows: &[C4], shape: &[usize]) -> PyResult<Bound<'py, PyAny>> {
    arr(py, rows.iter().flatten().copied().collect(), shape)
}

/// `_clip_polygon(poly)`: a list of 4-vectors (NumPy rows) clipped.
#[pyfunction]
fn hlr_clip_polygon<'py>(py: Python<'py>, poly: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    let pts: Vec<C4> = poly.try_iter()?.map(|p| Ok(rows4(&flat_f64(&p?)?)[0])).collect::<PyResult<_>>()?;
    PyList::new(py, clip_polygon(pts).iter().map(|p| arr(py, p.to_vec(), &[4])).collect::<PyResult<Vec<_>>>()?)
}

#[pyfunction]
fn hlr_clip_triangles<'py>(py: Python<'py>, tris: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    if tris.len()? == 0 {
        return Ok(tris.clone());
    }
    let rows = rows4(&flat_f64(tris)?);
    let t: Vec<[C4; 3]> = rows.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
    let out: Vec<C4> = clip_triangles(&t).into_iter().flatten().collect();
    c4_arr(py, &out, &[out.len() / 3, 3, 4])
}

#[pyfunction]
fn hlr_clip_segments<'py>(py: Python<'py>, a: &Bound<'py, PyAny>, b: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyTuple>> {
    if a.len()? == 0 {
        return PyTuple::new(py, [a.clone(), b.clone()]);
    }
    let (pa, pb) = clip_segments(&rows4(&flat_f64(a)?), &rows4(&flat_f64(b)?));
    PyTuple::new(py, [c4_arr(py, &pa, &[pa.len(), 4])?, c4_arr(py, &pb, &[pb.len(), 4])?])
}

/// `p @ M[:, :3].T + M[:, 3]`: NumPy's `(N, 3) @ (3, 4)`, a fused chain.
fn to_clip(m: &[[f64; 4]; 4], p: &P) -> C4 {
    let mut c = [0.0; 4];
    for j in 0..4 {
        c[j] = p[2].mul_add(m[j][2], p[1].mul_add(m[j][1], p[0] * m[j][0])) + m[j][3];
    }
    c
}

/// The perspective turned parallel (`core.hlr.hlr_perspective`): the
/// triangles and lines through the view's projective map, clipped to the
/// pyramid, as `(T (N, 3, 3), L (M, 2, 3), depth)` with `depth` the
/// farthest of them, or `None` when nothing is in view. `soft_n` NaN for an
/// open boundary's second normal; a soft edge is a line where its faces
/// turn opposite ways to the eye.
#[pyfunction]
#[allow(clippy::too_many_arguments)]
fn hlr_perspective_parts<'py>(py: Python<'py>, tris: &Bound<'py, PyAny>, hard: &Bound<'py, PyAny>,
                              cut: &Bound<'py, PyAny>, soft: &Bound<'py, PyAny>, soft_n: &Bound<'py, PyAny>,
                              camera: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyTuple>>> {
    let tri = rows3(&flat_f64(tris)?);
    let mut lines = rows3(&flat_f64(hard)?);
    lines.extend(rows3(&flat_f64(cut)?));
    let (soft, n) = if soft_n.is_none() {
        // The tuple lists of a collected scene: (p0, p1, na, nb or None).
        let (mut s, mut n) = (Vec::new(), Vec::new());
        for item in soft.try_iter()? {
            let item = item?;
            s.push(xyz(&item.get_item(0)?)?);
            s.push(xyz(&item.get_item(1)?)?);
            n.push(xyz(&item.get_item(2)?)?);
            let nb = item.get_item(3)?;
            n.push(if nb.is_none() { [f64::NAN; 3] } else { xyz(&nb)? });
        }
        (s, n)
    } else {
        (rows3(&flat_f64(soft)?), rows3(&flat_f64(soft_n)?))
    };
    if !soft.is_empty() {
        let e = xyz(&camera.call_method0("eye")?)?;
        for (k, s) in soft.chunks_exact(2).enumerate() {
            let mid = [(s[0][0] + s[1][0]) / 2.0, (s[0][1] + s[1][1]) / 2.0, (s[0][2] + s[1][2]) / 2.0];
            let v = sub(&mid, &e);
            let (fa, fb) = (dot3(&n[2 * k], &v), dot3(&n[2 * k + 1], &v));
            if fb.is_nan() || ((fa < 0.0) != (fb < 0.0)) {
                lines.extend_from_slice(s);
            }
        }
    }
    let proj = camera.call_method0("projection_matrix")?;
    let view = camera.call_method0("view_matrix")?;
    let data: Vec<f64> = proj.call_method1("__mul__", (view,))?.call_method0("data")?.extract()?;
    // Column-major data: M[j][k] = data[k * 4 + j].
    let mut m = [[0.0; 4]; 4];
    for j in 0..4 {
        for k in 0..4 {
            m[j][k] = data[k * 4 + j];
        }
    }
    let w_t = to_clip(&m, &xyz(&camera.getattr("target")?)?)[3];
    if w_t <= 1e-9 {
        return Ok(None);
    }
    let fov: f64 = camera.getattr("fov_deg")?.extract()?;
    let aspect: f64 = camera.getattr("aspect")?.extract()?;
    let half_h = w_t * (fov * (std::f64::consts::PI / 180.0) / 2.0).tan();
    let scale = [half_h * aspect, half_h, w_t];
    let parallel = |c: &C4| -> P {
        let w = c[3];
        [c[0] / w * scale[0], c[1] / w * scale[1], 1.0 / w * scale[2]]
    };
    let clip_t: Vec<[C4; 3]> = tri.chunks_exact(3)
        .map(|t| [to_clip(&m, &t[0]), to_clip(&m, &t[1]), to_clip(&m, &t[2])]).collect();
    let tc = clip_triangles(&clip_t);
    let a: Vec<C4> = lines.iter().step_by(2).map(|p| to_clip(&m, p)).collect();
    let b: Vec<C4> = lines.iter().skip(1).step_by(2).map(|p| to_clip(&m, p)).collect();
    let (a, b) = clip_segments(&a, &b);
    if a.is_empty() {
        return Ok(None);
    }
    let t: Vec<f64> = tc.iter().flatten().flat_map(parallel).collect();
    let l: Vec<f64> = a.iter().zip(&b).flat_map(|(p, q)| parallel(p).into_iter().chain(parallel(q))).collect();
    let far_t = t.chunks_exact(3).map(|c| c[2]).fold(f64::NEG_INFINITY, f64::max);
    let far_l = l.chunks_exact(3).map(|c| c[2]).fold(f64::NEG_INFINITY, f64::max);
    let depth = if tc.is_empty() { 0f64.max(far_l) } else { far_t.max(far_l) };
    let nt = tc.len();
    let nl = a.len();
    Ok(Some(PyTuple::new(py, [arr(py, t, &[nt, 3, 3])?, arr(py, l, &[nl, 2, 3])?,
                              depth.into_pyobject(py)?.into_any()])?))
}

/// `_geometry_as_lists(tris, hard, soft, soft_n)`: arrays to the tuple
/// lists `clip_to_section` walks.
#[pyfunction]
fn hlr_geometry_lists<'py>(py: Python<'py>, tris: &Bound<'py, PyAny>, hard: &Bound<'py, PyAny>,
                           soft: &Bound<'py, PyAny>, soft_n: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyTuple>> {
    let tup = |p: &P| PyTuple::new(py, p.iter().copied());
    let t = PyList::empty(py);
    for c in rows3(&flat_f64(tris)?).chunks_exact(3) {
        t.append((tup(&c[0])?, tup(&c[1])?, tup(&c[2])?))?;
    }
    let h = PyList::empty(py);
    for c in rows3(&flat_f64(hard)?).chunks_exact(2) {
        h.append((tup(&c[0])?, tup(&c[1])?))?;
    }
    let s = PyList::empty(py);
    let normals = rows3(&flat_f64(soft_n)?);
    for (k, c) in rows3(&flat_f64(soft)?).chunks_exact(2).enumerate() {
        let (na, nb) = (normals[2 * k], normals[2 * k + 1]);
        let nb = if nb.iter().any(|x| x.is_nan()) { py.None().into_bound(py) } else { tup(&nb)?.into_any() };
        s.append((tup(&c[0])?, tup(&c[1])?, tup(&na)?, nb))?;
    }
    PyTuple::new(py, [t.into_any(), h.into_any(), s.into_any()])
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(hlr_camera_basis, m)?)?;
    m.add_function(wrap_pyfunction!(hlr_to_cam, m)?)?;
    m.add_function(wrap_pyfunction!(hlr_solid_depth, m)?)?;
    m.add_function(wrap_pyfunction!(hlr_cap_depth_affine, m)?)?;
    m.add_function(wrap_pyfunction!(hlr_cap_ring_edges, m)?)?;
    m.add_function(wrap_pyfunction!(hlr_closing_tags, m)?)?;
    m.add_function(wrap_pyfunction!(hlr_clip_polygon, m)?)?;
    m.add_function(wrap_pyfunction!(hlr_clip_triangles, m)?)?;
    m.add_function(wrap_pyfunction!(hlr_clip_segments, m)?)?;
    m.add_function(wrap_pyfunction!(hlr_perspective_parts, m)?)?;
    m.add_function(wrap_pyfunction!(hlr_geometry_lists, m)?)?;
    Ok(())
}
