// SPDX-License-Identifier: GPL-3.0-or-later
//! The section cut as the viewport draws it (`core.section`): the chords a
//! plane cuts out of the pick index's triangles (`PlaneReach`, kept while
//! the plane slides along its normal), the filled areas they close (rings
//! nested even-odd), those areas as triangles, their 45° hatching, and the
//! basis a sheet lays a cut's hatch in.
//!
//! Ported operation for operation from the NumPy (`docs/model-port.md`):
//! `(N, 3) @ (3,)` and `(N, 2) @ (2,)` are NumPy's plain left-to-right sums,
//! measured. A ring's area went through a BLAS dot (its order unnamed): it
//! only orders the rings for nesting, so a tie may break another way.
//! `tests/test_section_fill_parity.py` holds the answers to the Python's.

use numpy::{PyArray1, PyArrayMethods};
use pyo3::prelude::*;
use pyo3::types::{PyList, PyTuple};

use super::pyfloat;
use super::section_cut::chain_loops;

type P = [f64; 3];
type Q = [f64; 2];

pub(crate) fn flat_f64(obj: &Bound<'_, PyAny>) -> PyResult<Vec<f64>> {
    let np = obj.py().import("numpy")?;
    let arr = np
        .call_method1("ascontiguousarray", (obj, np.getattr("float64")?))?
        .call_method1("reshape", (-1,))?
        .cast_into::<PyArray1<f64>>()?;
    let ro = arr.readonly();
    Ok(ro.as_slice()?.to_vec())
}

fn rows3(flat: &[f64]) -> Vec<P> {
    flat.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect()
}

fn xyz(obj: &Bound<'_, PyAny>) -> PyResult<P> {
    Ok(crate::math::xyz(obj)?.to_array())
}

fn dot3(a: &P, b: &P) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// `np.cross(a, b)` for 3-vectors.
pub(crate) fn cross(a: &P, b: &P) -> P {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// `np.linalg.norm` of a 3-vector: the plain sum, measured.
pub(crate) fn norm3(a: &P) -> f64 {
    dot3(a, a).sqrt()
}

fn scaled(a: &P, s: f64) -> P {
    [a[0] / s, a[1] / s, a[2] / s]
}

pub(crate) fn arr<'py, T: numpy::Element>(py: Python<'py>, data: Vec<T>, shape: &[usize]) -> PyResult<Bound<'py, PyAny>> {
    Ok(PyArray1::from_vec(py, data).reshape(shape.to_vec())?.into_any())
}

fn none<'py>(py: Python<'py>, rows: bool) -> PyResult<Bound<'py, PyAny>> {
    if rows {
        Ok(PyTuple::new(py, [py.None(), py.None()])?.into_any())
    } else {
        Ok(py.None().into_bound(py))
    }
}

/// The corners of a set of triangles and how far along one normal each
/// stands (`core.section.PlaneReach`).
#[pyclass(module = "ingetrazo_core", name = "PlaneReach", frozen)]
pub struct PlaneReach {
    v0: Py<PyAny>,
    e1: Py<PyAny>,
    e2: Py<PyAny>,
    n: P,
    p: [Vec<P>; 3],
    h: [Vec<f64>; 3],
    lo: Vec<f64>,
    hi: Vec<f64>,
    worked: std::sync::atomic::AtomicUsize,
}

#[pymethods]
impl PlaneReach {
    #[new]
    fn new(v0: &Bound<'_, PyAny>, e1: &Bound<'_, PyAny>, e2: &Bound<'_, PyAny>, normal: &Bound<'_, PyAny>) -> PyResult<Self> {
        let n = xyz(normal)?;
        let p0 = rows3(&flat_f64(v0)?);
        let d1 = rows3(&flat_f64(e1)?);
        let d2 = rows3(&flat_f64(e2)?);
        let add = |d: &Vec<P>| -> Vec<P> {
            p0.iter().zip(d).map(|(a, b)| [a[0] + b[0], a[1] + b[1], a[2] + b[2]]).collect()
        };
        let (p1, p2) = (add(&d1), add(&d2));
        let height = |ps: &Vec<P>| -> Vec<f64> { ps.iter().map(|p| dot3(p, &n)).collect() };
        let h = [height(&p0), height(&p1), height(&p2)];
        let lo = (0..p0.len()).map(|i| h[0][i].min(h[1][i]).min(h[2][i])).collect();
        let hi = (0..p0.len()).map(|i| h[0][i].max(h[1][i]).max(h[2][i])).collect();
        Ok(PlaneReach {
            v0: v0.clone().unbind(),
            e1: e1.clone().unbind(),
            e2: e2.clone().unbind(),
            n,
            p: [p0, p1, p2],
            h,
            lo,
            hi,
            worked: 0.into(),
        })
    }

    /// Made from these very arrays and this normal.
    #[pyo3(signature = (v0, normal, e1=None, e2=None))]
    fn fits(&self, v0: &Bound<'_, PyAny>, normal: &Bound<'_, PyAny>, e1: Option<&Bound<'_, PyAny>>,
            e2: Option<&Bound<'_, PyAny>>) -> PyResult<bool> {
        Ok(v0.is(&self.v0)
            && e1.is_none_or(|e| e.is_none() || e.is(&self.e1))
            && e2.is_none_or(|e| e.is_none() || e.is(&self.e2))
            && xyz(normal)? == self.n)
    }

    /// How many triangles the last cut measured (its candidates).
    #[getter]
    fn worked(&self) -> usize {
        self.worked.load(std::sync::atomic::Ordering::Relaxed)
    }

    #[pyo3(signature = (point, visible=None, rows=false))]
    fn cut<'py>(&self, py: Python<'py>, point: &Bound<'py, PyAny>, visible: Option<&Bound<'py, PyAny>>,
                rows: bool) -> PyResult<Bound<'py, PyAny>> {
        let c = dot3(&self.n, &xyz(point)?);
        let mut near: Vec<usize> = (0..self.lo.len()).filter(|&i| self.lo[i] - c < 0.0 && self.hi[i] - c > 0.0).collect();
        if let Some(vis) = visible.filter(|v| !v.is_none()) {
            if !near.is_empty() {
                let np = py.import("numpy")?;
                let vis = np
                    .call_method1("asarray", (vis,))?
                    .call_method1("astype", (np.getattr("bool_")?,))?
                    .call_method1("reshape", (-1,))?
                    .cast_into::<PyArray1<bool>>()?;
                let vis = vis.readonly();
                let vis = vis.as_slice()?;
                near.retain(|&i| vis[i]);
            }
        }
        self.worked.store(near.len(), std::sync::atomic::Ordering::Relaxed);
        if near.is_empty() {
            return none(py, rows);
        }
        let mut chords: Vec<f32> = Vec::new();
        let mut which: Vec<i64> = Vec::new();
        for &i in &near {
            let p = [self.p[0][i], self.p[1][i], self.p[2][i]];
            let d = [self.h[0][i] - c, self.h[1][i] - c, self.h[2][i] - c];
            let mut hits: Vec<P> = Vec::with_capacity(3);
            for (a, b) in [(0, 1), (1, 2), (2, 0)] {
                let (da, db) = (d[a], d[b]);
                if da * db < 0.0 {
                    let t = da / (da - db);
                    let (pa, pb) = (p[a], p[b]);
                    hits.push([
                        pa[0] + (pb[0] - pa[0]) * t,
                        pa[1] + (pb[1] - pa[1]) * t,
                        pa[2] + (pb[2] - pa[2]) * t,
                    ]);
                }
            }
            if hits.len() == 2 {
                for q in &hits {
                    chords.extend(q.iter().map(|&v| v as f32));
                }
                which.push(i as i64);
            }
        }
        if which.is_empty() {
            return none(py, rows);
        }
        let k = which.len();
        let out = arr(py, chords, &[k, 2, 3])?;
        if rows {
            Ok(PyTuple::new(py, [out, arr(py, which, &[k])?])?.into_any())
        } else {
            Ok(out)
        }
    }
}

/// `cut_chords(v0, e1, e2, normal, point, visible=None, rows=False,
/// reach=None)`: see `core.section.cut_chords`.
#[pyfunction]
#[pyo3(signature = (v0, e1, e2, normal, point, visible=None, rows=false, reach=None))]
#[allow(clippy::too_many_arguments)]
fn section_cut_chords<'py>(py: Python<'py>, v0: &Bound<'py, PyAny>, e1: &Bound<'py, PyAny>, e2: &Bound<'py, PyAny>,
                           normal: &Bound<'py, PyAny>, point: &Bound<'py, PyAny>, visible: Option<&Bound<'py, PyAny>>,
                           rows: bool, reach: Option<&Bound<'py, PyAny>>) -> PyResult<Bound<'py, PyAny>> {
    if v0.is_none() || v0.len()? == 0 {
        return none(py, rows);
    }
    let reach = match reach.and_then(|r| r.cast::<PlaneReach>().ok()) {
        Some(r) if r.get().fits(v0, normal, Some(e1), Some(e2))? => r.clone(),
        _ => Bound::new(py, PlaneReach::new(v0, e1, e2, normal)?)?,
    };
    reach.get().cut(py, point, visible, rows)
}

/// `_plane_basis`: an in-plane pair square to `n`.
fn plane_basis(normal: P) -> (P, P) {
    let l = norm3(&normal);
    let n = scaled(&normal, if l == 0.0 { 1.0 } else { l });
    let helper = if n[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let u = cross(&n, &helper);
    let u = scaled(&u, norm3(&u));
    (u, cross(&n, &u))
}

/// `_sheet_basis`: a plan with x east and y north, a section with v up.
fn sheet_basis(normal: P) -> (P, P) {
    let l = norm3(&normal);
    let n = scaled(&normal, if l == 0.0 { 1.0 } else { l });
    let or1 = |x: f64| if x == 0.0 { 1.0 } else { x };
    if n[2].abs() >= 0.7 {
        let u = [1.0 - n[0] * n[0], 0.0 - n[1] * n[0], 0.0 - n[2] * n[0]];
        let u = scaled(&u, or1(norm3(&u)));
        let v = cross(&n, &u);
        return (u, v);
    }
    let v = [0.0 - n[0] * n[2], 0.0 - n[1] * n[2], 1.0 - n[2] * n[2]];
    let v = scaled(&v, or1(norm3(&v)));
    let u = cross(&v, &n);
    (scaled(&u, or1(norm3(&u))), v)
}

/// Even-odd: how many of `pts` lie inside `poly`.
fn inside_count(pts: &[Q], poly: &[Q]) -> usize {
    let n = poly.len();
    pts.iter()
        .filter(|&&[x, y]| {
            let mut c = 0usize;
            for k in 0..n {
                let ([x1, y1], [x2, y2]) = (poly[k], poly[(k + 1) % n]);
                let straddle = (y1 > y) != (y2 > y);
                let dy = if y2 == y1 { 1.0 } else { y2 - y1 };
                let xint = x1 + (y - y1) * (x2 - x1) / dy;
                if straddle && x < xint {
                    c += 1;
                }
            }
            c % 2 == 1
        })
        .count()
}

/// One filled area: its rings (outer first, then holes), in world and in
/// plane coordinates.
struct Area {
    world: Vec<Vec<P>>,
    flat: Vec<Vec<Q>>,
}

fn fill_groups(chords: &[f64], normal: P) -> (Vec<Area>, Option<(P, P)>) {
    let edges: Vec<[P; 2]> = chords.chunks_exact(6).map(|c| [[c[0], c[1], c[2]], [c[3], c[4], c[5]]]).collect();
    if edges.is_empty() {
        return (Vec::new(), None);
    }
    let (nodes, loops) = chain_loops(&edges, None);
    if loops.is_empty() {
        return (Vec::new(), None);
    }
    let rings: Vec<Vec<P>> = loops.iter().map(|(ids, _)| ids.iter().map(|&i| nodes[i]).collect()).collect();
    let (u, v) = plane_basis(normal);
    let flat: Vec<Vec<Q>> = rings.iter().map(|r| r.iter().map(|p| [dot3(p, &u), dot3(p, &v)]).collect()).collect();
    let area: Vec<f64> = flat
        .iter()
        .map(|f| {
            let m = f.len();
            let (mut d1, mut d2) = (0.0, 0.0);
            for i in 0..m {
                d1 += f[i][0] * f[(i + 1) % m][1];
                d2 += f[i][1] * f[(i + 1) % m][0];
            }
            (0.5 * (d1 - d2)).abs()
        })
        .collect();
    let bound = |f: &Vec<Q>, pick: fn(f64, f64) -> f64| -> Q {
        f[1..].iter().fold(f[0], |a, b| [pick(a[0], b[0]), pick(a[1], b[1])])
    };
    let lo: Vec<Q> = flat.iter().map(|f| bound(f, f64::min)).collect();
    let hi: Vec<Q> = flat.iter().map(|f| bound(f, f64::max)).collect();
    let count = rings.len();
    let mut holders: Vec<Vec<usize>> = vec![Vec::new(); count];
    for i in 0..count {
        for j in 0..count {
            if area[j] > area[i] && lo[j][0] <= lo[i][0] && lo[j][1] <= lo[i][1] && hi[j][0] >= hi[i][0]
                && hi[j][1] >= hi[i][1]
                && (inside_count(&flat[i], &flat[j]) as f64) / (flat[i].len() as f64) >= 0.9
            {
                holders[i].push(j);
            }
        }
    }
    let parent: Vec<Option<usize>> = holders
        .iter()
        .map(|h| h.iter().copied().fold(None, |best: Option<usize>, j| match best {
            Some(b) if area[b] <= area[j] => Some(b),
            _ => Some(j),
        }))
        .collect();
    let mut holes_of: Vec<Vec<usize>> = vec![Vec::new(); count];
    for k in 0..count {
        if holders[k].len() % 2 == 1 {
            if let Some(p) = parent[k] {
                holes_of[p].push(k);
            }
        }
    }
    let mut groups = Vec::new();
    for i in 0..count {
        if holders[i].len() % 2 == 1 {
            continue;
        }
        let members: Vec<usize> = std::iter::once(i).chain(holes_of[i].iter().copied()).collect();
        groups.push(Area {
            world: members.iter().map(|&k| rings[k].clone()).collect(),
            flat: members.iter().map(|&k| flat[k].clone()).collect(),
        });
    }
    (groups, Some((u, v)))
}

fn group_triangles(groups: &[Area]) -> Vec<f32> {
    let mut out = Vec::new();
    for g in groups {
        let mut pts: Vec<[f64; 2]> = Vec::new();
        let mut starts = Vec::new();
        for (k, f) in g.flat.iter().enumerate() {
            if k > 0 {
                starts.push(pts.len());
            }
            pts.extend(f.iter().copied());
        }
        let world: Vec<P> = g.world.iter().flatten().copied().collect();
        for t in crate::earcut::earcut(&pts, &starts) {
            for i in t {
                out.extend(world[i].iter().map(|&v| v as f32));
            }
        }
    }
    out
}

fn hatch(groups: &[Area], basis: (P, P), spacing: f64, both: bool) -> Vec<f32> {
    let (u, v) = basis;
    let n = cross(&u, &v);
    let first = groups[0].world[0][0];
    let o = dot3(&first, &n);
    let off = [o * n[0], o * n[1], o * n[2]];
    let mut out: Vec<f32> = Vec::new();
    let angles: &[f64] = if both { &[45.0, 135.0] } else { &[45.0] };
    for &ang in angles {
        let t = ang * (std::f64::consts::PI / 180.0);
        let d = [pyfloat::cos(t), pyfloat::sin(t)];
        let m = [-d[1], d[0]];
        let along = |p: &Q, w: &Q| p[0] * w[0] + p[1] * w[1];
        for g in groups {
            let all = g.flat.iter().flatten();
            let (mut smin, mut smax) = (f64::INFINITY, f64::NEG_INFINITY);
            for p in all {
                let s = along(p, &m);
                smin = smin.min(s);
                smax = smax.max(s);
            }
            let k0 = (smin / spacing).ceil() as i64;
            let k1 = (smax / spacing).floor() as i64;
            if k1 - k0 > 20000 {
                continue;
            }
            for k in k0..=k1 {
                let s = k as f64 * spacing;
                let mut hits: Vec<f64> = Vec::new();
                for f in &g.flat {
                    let len = f.len();
                    for i in 0..len {
                        let (a, b) = (f[i], f[(i + 1) % len]);
                        let (sa, sb) = (along(&a, &m) - s, along(&b, &m) - s);
                        if (sa > 0.0) != (sb > 0.0) {
                            let tt = sa / (sa - sb);
                            let p = [a[0] + (b[0] - a[0]) * tt, a[1] + (b[1] - a[1]) * tt];
                            hits.push(along(&p, &d));
                        }
                    }
                }
                hits.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let mut j = 0;
                while j + 1 < hits.len() {
                    for h in [hits[j], hits[j + 1]] {
                        let q = [d[0] * h + m[0] * s, d[1] * h + m[1] * s];
                        for c in 0..3 {
                            out.push((q[0] * u[c] + q[1] * v[c] + off[c]) as f32);
                        }
                    }
                    j += 2;
                }
            }
        }
    }
    out
}

fn chords_of(chords: &Bound<'_, PyAny>) -> PyResult<Vec<f64>> {
    if chords.is_none() || chords.len()? == 0 {
        return Ok(Vec::new());
    }
    flat_f64(chords)
}

fn vec_arr<'py>(py: Python<'py>, a: &[f64]) -> PyResult<Bound<'py, PyAny>> {
    arr(py, a.to_vec(), &[a.len()])
}

/// `_fill_groups(chords, normal)`: `([(rings3d, rings2d)], (u, v))`.
#[pyfunction]
fn section_fill_groups<'py>(py: Python<'py>, chords: &Bound<'py, PyAny>, normal: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyTuple>> {
    let (groups, basis) = fill_groups(&chords_of(chords)?, xyz(normal)?);
    let out = PyList::empty(py);
    for g in &groups {
        let w = PyList::empty(py);
        for r in &g.world {
            w.append(arr(py, r.iter().flatten().copied().collect(), &[r.len(), 3])?)?;
        }
        let f = PyList::empty(py);
        for r in &g.flat {
            f.append(arr(py, r.iter().flatten().copied().collect(), &[r.len(), 2])?)?;
        }
        out.append((w, f))?;
    }
    let basis = match basis {
        Some((u, v)) => PyTuple::new(py, [vec_arr(py, &u)?, vec_arr(py, &v)?])?.into_any(),
        None => py.None().into_bound(py),
    };
    PyTuple::new(py, [out.into_any(), basis])
}

fn tris_array<'py>(py: Python<'py>, data: Vec<f32>) -> PyResult<Bound<'py, PyAny>> {
    let t = data.len() / 9;
    arr(py, data, &[t, 3, 3])
}

/// `_group_triangles(groups)` over groups `section_fill_groups` made.
#[pyfunction]
fn section_group_triangles<'py>(py: Python<'py>, groups: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let mut areas = Vec::new();
    for g in groups.try_iter()? {
        let g = g?;
        let mut a = Area { world: Vec::new(), flat: Vec::new() };
        for r in g.get_item(0)?.try_iter()? {
            a.world.push(rows3(&flat_f64(&r?)?));
        }
        for r in g.get_item(1)?.try_iter()? {
            a.flat.push(flat_f64(&r?)?.chunks_exact(2).map(|c| [c[0], c[1]]).collect());
        }
        areas.push(a);
    }
    tris_array(py, group_triangles(&areas))
}

/// `fill_triangles(chords, normal)`: see `core.section.fill_triangles`.
#[pyfunction]
fn section_fill_triangles<'py>(py: Python<'py>, chords: &Bound<'py, PyAny>, normal: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let (groups, _) = fill_groups(&chords_of(chords)?, xyz(normal)?);
    tris_array(py, group_triangles(&groups))
}

/// `hatch_segments(chords, normal, spacing, cross=False)`.
#[pyfunction]
#[pyo3(signature = (chords, normal, spacing, cross=false))]
fn section_hatch_segments<'py>(py: Python<'py>, chords: &Bound<'py, PyAny>, normal: &Bound<'py, PyAny>,
                               spacing: f64, cross: bool) -> PyResult<Bound<'py, PyAny>> {
    let (groups, basis) = fill_groups(&chords_of(chords)?, xyz(normal)?);
    let lines = match basis {
        Some(b) if !groups.is_empty() && spacing > 0.0 => hatch(&groups, b, spacing, cross),
        _ => Vec::new(),
    };
    let l = lines.len() / 6;
    arr(py, lines, &[l, 2, 3])
}

/// `_sheet_basis(normal)`: `(u, v)` as NumPy rows.
#[pyfunction]
fn section_sheet_basis<'py>(py: Python<'py>, normal: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyTuple>> {
    let (u, v) = sheet_basis(xyz(normal)?);
    PyTuple::new(py, [vec_arr(py, &u)?, vec_arr(py, &v)?])
}

// ---- The plane: its letter, its fill, its archive ------------------------------

const FILL_PATTERNS: [&str; 5] = ["solid", "hatch", "cross", "none", "layers"];
const DEFAULT_HATCH_SPACING: f64 = 0.05;

/// `next_symbol(planes)`: A, B, C… then AA, AB — the first letter no plane
/// takes (sections are named by letter, «A … A» across the plan).
#[pyfunction]
fn section_next_symbol(planes: &Bound<'_, PyAny>) -> PyResult<String> {
    let mut taken = std::collections::HashSet::new();
    if planes.is_truthy()? {
        for p in planes.try_iter()? {
            let sym = p?.getattr_opt("symbol")?.filter(|s| s.is_truthy().unwrap_or(false));
            let text = match sym {
                Some(s) => s.str()?.call_method0("strip")?.call_method0("upper")?.extract::<String>()?,
                None => String::new(),
            };
            taken.insert(text);
        }
    }
    for i in 0.. {
        let (mut n, mut label) = (i as i64, String::new());
        loop {
            label.insert(0, (b'A' + (n % 26) as u8) as char);
            n = n / 26 - 1;
            if n < 0 {
                break;
            }
        }
        if !taken.contains(&label) {
            return Ok(label);
        }
    }
    unreachable!()
}

/// `effective_fill(plane, style)`: `(pattern, (r, g, b), spacing)`, the
/// plane's own settings over the style's; Section Fill off fills nothing.
#[pyfunction]
fn section_effective_fill<'py>(py: Python<'py>, plane: &Bound<'py, PyAny>, style: &Bound<'py, PyAny>)
    -> PyResult<Bound<'py, PyTuple>> {
    let own = plane.getattr_opt("fill")?.filter(|f| f.is_truthy().unwrap_or(false))
        .unwrap_or_else(|| pyo3::types::PyDict::new(py).into_any());
    let on = style.getattr_opt("section_fill")?.map_or(Ok(true), |v| v.is_truthy())?;
    let pattern = if on { own.call_method1("get", ("pattern", "solid"))? } else { "none".into_pyobject(py)?.into_any() };
    let color = own.call_method1("get", ("color",))?;
    let color = if color.is_truthy()? {
        color
    } else {
        match style.getattr_opt("section_fill_color")? {
            Some(c) => c,
            None => (0.35, 0.37, 0.41).into_pyobject(py)?.into_any(),
        }
    };
    let color = py.get_type::<PyTuple>().call1((color,))?;
    let spacing = own.call_method1("get", ("spacing",))?;
    let spacing = if spacing.is_truthy()? { spacing } else { DEFAULT_HATCH_SPACING.into_pyobject(py)?.into_any() };
    let spacing = py.get_type::<pyo3::types::PyFloat>().call1((spacing,))?;
    PyTuple::new(py, [pattern, color.into_any(), spacing.into_any()])
}

fn xyz_list<'py>(py: Python<'py>, v: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyList>> {
    PyList::new(py, [v.call_method0("x")?, v.call_method0("y")?, v.call_method0("z")?])
}

/// `SectionPlane.to_dict`: the .igz entry, defaults left out.
#[pyfunction]
fn section_plane_archive<'py>(py: Python<'py>, plane: &Bound<'py, PyAny>) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
    let out = pyo3::types::PyDict::new(py);
    out.set_item("point", xyz_list(py, &plane.getattr("point")?)?)?;
    out.set_item("normal", xyz_list(py, &plane.getattr("normal")?)?)?;
    out.set_item("uid", plane.getattr("uid")?)?;
    for key in ["name", "symbol"] {
        let v = plane.getattr(key)?;
        if v.is_truthy()? {
            out.set_item(key, v)?;
        }
    }
    for key in ["active", "hidden"] {
        if plane.getattr(key)?.is_truthy()? {
            out.set_item(key, true)?;
        }
    }
    let layer = plane.getattr("layer")?;
    if layer.is_truthy()? {
        out.set_item("layer", layer)?;
    }
    let fill = plane.getattr("fill")?;
    if fill.is_truthy()? {
        let d = pyo3::types::PyDict::new(py);
        for item in fill.call_method0("items")?.try_iter()? {
            let (k, v): (Bound<PyAny>, Bound<PyAny>) = item?.extract()?;
            let v = if k.eq("color")? { py.get_type::<PyList>().call1((v,))? } else { v };
            d.set_item(k, v)?;
        }
        out.set_item("fill", d)?;
    }
    Ok(out)
}

/// `SectionPlane.from_dict(raw)`: a plane from its entry, its fill read
/// defensively.
#[pyfunction]
fn section_plane_load<'py>(py: Python<'py>, cls: &Bound<'py, PyAny>, raw: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let vec = py.get_type::<crate::math::Vec3>();
    let get = |k: &str, d: Bound<'py, PyAny>| raw.call_method1("get", (k, d));
    let kw = pyo3::types::PyDict::new(py);
    kw.set_item("name", get("name", "".into_pyobject(py)?.into_any())?)?;
    kw.set_item("symbol", get("symbol", "".into_pyobject(py)?.into_any())?)?;
    kw.set_item("active", get("active", false.into_pyobject(py)?.to_owned().into_any())?.is_truthy()?)?;
    kw.set_item("uid", raw.call_method1("get", ("uid",))?)?;
    kw.set_item("hidden", get("hidden", false.into_pyobject(py)?.to_owned().into_any())?.is_truthy()?)?;
    let point = vec.call1(PyTuple::new(py, raw.get_item("point")?.try_iter()?.collect::<PyResult<Vec<_>>>()?)?)?;
    let normal = vec.call1(PyTuple::new(py, raw.get_item("normal")?.try_iter()?.collect::<PyResult<Vec<_>>>()?)?)?;
    let sp = cls.call((point, normal), Some(&kw))?;
    let layer = raw.call_method1("get", ("layer",))?;
    if layer.is_instance_of::<pyo3::types::PyString>() && layer.is_truthy()? {
        sp.setattr("layer", layer)?;
    }
    let fill = raw.call_method1("get", ("fill",))?;
    if fill.is_instance_of::<pyo3::types::PyDict>() {
        let clean = pyo3::types::PyDict::new(py);
        let pattern = fill.call_method1("get", ("pattern",))?;
        if FILL_PATTERNS.iter().any(|p| pattern.eq(*p).unwrap_or(false)) {
            clean.set_item("pattern", pattern)?;
        }
        let color = fill.call_method1("get", ("color",))?;
        if (color.is_instance_of::<PyList>() || color.is_instance_of::<PyTuple>()) && color.len()? >= 3 {
            let float = py.get_type::<pyo3::types::PyFloat>();
            let rgb = (0..3).map(|i| float.call1((color.get_item(i)?,))).collect::<PyResult<Vec<_>>>()?;
            clean.set_item("color", PyTuple::new(py, rgb)?)?;
        }
        let spacing = fill.call_method1("get", ("spacing",))?;
        let number = spacing.is_instance_of::<pyo3::types::PyInt>() || spacing.is_instance_of::<pyo3::types::PyFloat>();
        if number && spacing.gt(0)? {
            clean.set_item("spacing", py.get_type::<pyo3::types::PyFloat>().call1((spacing,))?)?;
        }
        sp.setattr("fill", if clean.len() > 0 { clean.into_any() } else { py.None().into_bound(py) })?;
    }
    Ok(sp)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PlaneReach>()?;
    m.add_function(wrap_pyfunction!(section_cut_chords, m)?)?;
    m.add_function(wrap_pyfunction!(section_fill_groups, m)?)?;
    m.add_function(wrap_pyfunction!(section_group_triangles, m)?)?;
    m.add_function(wrap_pyfunction!(section_fill_triangles, m)?)?;
    m.add_function(wrap_pyfunction!(section_hatch_segments, m)?)?;
    m.add_function(wrap_pyfunction!(section_sheet_basis, m)?)?;
    m.add_function(wrap_pyfunction!(section_next_symbol, m)?)?;
    m.add_function(wrap_pyfunction!(section_effective_fill, m)?)?;
    m.add_function(wrap_pyfunction!(section_plane_archive, m)?)?;
    m.add_function(wrap_pyfunction!(section_plane_load, m)?)?;
    Ok(())
}
