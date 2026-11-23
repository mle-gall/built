// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! The push/pull tool's searches over a whole mesh (`tools/pushpull.py`,
//! through `core/push_scans.py`), on a `MeshScan`.
//!
//! As in `crate::scans`, a query either answers exactly what the Python
//! loop computes (the lateral exit ray test, through the same float32
//! arithmetic; the cracked edges near a box) or is a PREFILTER: a superset,
//! in list order, of what the loop's own predicate can accept, which the
//! caller then runs unchanged on the candidates. On the ride's largest mesh
//! (267k faces) these loops took ~25 s of a click and ~10 s of a stage.

use crate::store::geometry::{self as facegeom, Hypot, V3};
use crate::orient::{geom_of, Geom};
use crate::scans::{MeshScan, FFLAGS, RINGS};
use crate::store::keys::Kind;
use crate::store::py::{PyMeshStore, PyStoreSnapshot};
use pyo3::prelude::*;
use rayon::prelude::*;

/// Float slack for a test Python makes in float32 on coordinates of
/// magnitude `m`: far above the float32 error of a few operations.
#[inline]
fn slack(m: f64) -> f64 {
    1e-5 * (1.0 + m)
}

/// Box of every ring of face `f` (holes too): `[lo; hi]`.
fn ring_box(scan: &MeshScan, f: usize) -> [f64; 6] {
    let pos = scan.positions();
    let mut b = [f64::INFINITY, f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for ring in scan.rings_of(f) {
        for &v in ring {
            let p = pos[v as usize];
            for k in 0..3 {
                b[k] = b[k].min(p[k]);
                b[k + 3] = b[k + 3].max(p[k]);
            }
        }
    }
    b
}

fn big(b: &[f64; 6]) -> f64 {
    b.iter().fold(0.0f64, |m, x| if x.is_finite() { m.max(x.abs()) } else { m })
}

/// Whether the mesh has faces and every listed edge borders at least two
/// (`core.orient.is_closed`, exactly).
#[pyfunction]
pub fn scan_is_closed(py: Python<'_>, mut scan: PyRefMut<'_, MeshScan>) -> PyResult<bool> {
    scan.need_parts(py, 0)?;
    Ok(scan.n_faces() > 0 && scan.edge_face_counts().iter().all(|&c| c >= 2))
}

/// Faces (list order) whose every-ring box, grown by `pad + rel ×` its
/// diagonal plus float slack, holds every one of `points`. A superset of
/// the faces with a boundary edge (outer or hole) through all the points
/// within `_point_on_seg_incl`'s tolerance `tol` (pass `pad ≥ 2·tol`,
/// `rel ≥ tol`), and of the faces with corners at their keys.
#[pyfunction]
pub fn scan_faces_holding_points(
    py: Python<'_>,
    mut scan: PyRefMut<'_, MeshScan>,
    points: Vec<(f64, f64, f64)>,
    pad: f64,
    rel: f64,
) -> PyResult<Vec<u32>> {
    scan.need_parts(py, RINGS)?;
    let s: &MeshScan = &scan;
    Ok(py.detach(|| {
        (0..s.n_faces() as u32)
            .into_par_iter()
            .filter(|&f| {
                let b = ring_box(s, f as usize);
                if !b[0].is_finite() {
                    return false; // no corner: no boundary edge to lie on
                }
                let diag = ((b[3] - b[0]).powi(2) + (b[4] - b[1]).powi(2) + (b[5] - b[2]).powi(2)).sqrt();
                let g = pad + rel * diag + slack(big(&b));
                if !g.is_finite() {
                    return true;
                }
                points.iter().all(|p| {
                    let p = [p.0, p.1, p.2];
                    (0..3).all(|k| p[k] >= b[k] - g && p[k] <= b[k + 3] + g)
                })
            })
            .collect()
    }))
}

/// Faces (list order, `skip` left out) whose normal is parallel to the unit
/// `n` (|cos| ≥ `dot_min` less slack) and whose outer loop, projected on
/// the axes `u`, `v` (absolute coordinates, as `_project_loop_2d`), has a
/// box meeting `[lo2, hi2]` (grown by float slack). A superset of the
/// parallel faces `_loops_overlap_2d` can find overlapping the base: every
/// point that test uses lies in its own loop's box.
#[pyfunction]
#[allow(clippy::too_many_arguments)]
pub fn scan_parallel_overlapping(
    py: Python<'_>,
    mut scan: PyRefMut<'_, MeshScan>,
    n: (f64, f64, f64),
    u: (f64, f64, f64),
    v: (f64, f64, f64),
    lo2: (f64, f64),
    hi2: (f64, f64),
    dot_min: f64,
    hypot: &str,
    compensated: bool,
) -> PyResult<Vec<u32>> {
    let g = geom_of(py, &mut scan, hypot, compensated)?;
    let s: &MeshScan = &scan;
    let n = [n.0, n.1, n.2];
    let u = [u.0, u.1, u.2];
    let v = [v.0, v.1, v.2];
    Ok(py.detach(|| {
        (0..s.n_faces() as u32)
            .into_par_iter()
            .filter(|&f| {
                let gn = g.normal[f as usize];
                let gn = [gn.0 as f64, gn.1 as f64, gn.2 as f64];
                let ln = (gn[0] * gn[0] + gn[1] * gn[1] + gn[2] * gn[2]).sqrt();
                // normalized() of a near-unit normal, then the dot: well
                // inside 1e-5 of the f64 value; a NaN normal is kept.
                let c = (gn[0] * n[0] + gn[1] * n[1] + gn[2] * n[2]) / ln;
                if c.abs() < dot_min - 1e-5 {
                    return false;
                }
                let rings = s.rings_of(f as usize);
                let Some(outer) = rings.first() else { return true };
                let pos = s.positions();
                let (mut a0, mut a1, mut b0, mut b1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
                let mut m = 0.0f64;
                for &q in outer.iter() {
                    let p = pos[q as usize];
                    let x = p[0] * u[0] + p[1] * u[1] + p[2] * u[2];
                    let y = p[0] * v[0] + p[1] * v[1] + p[2] * v[2];
                    a0 = a0.min(x);
                    a1 = a1.max(x);
                    b0 = b0.min(y);
                    b1 = b1.max(y);
                    m = m.max(p[0].abs()).max(p[1].abs()).max(p[2].abs());
                }
                if outer.is_empty() {
                    return true;
                }
                let sl = slack(m.max(lo2.0.abs()).max(lo2.1.abs()).max(hi2.0.abs()).max(hi2.1.abs()));
                !(a1 < lo2.0 - sl || a0 > hi2.0 + sl || b1 < lo2.1 - sl || b0 > hi2.1 + sl)
            })
            .collect()
    }))
}

/// `core.orient._ray_triangle(origin, direction, tri)` in float32 through
/// Qt's operations (`crate::facegeom`'s products), its parameter `t` as the
/// Python float it returns.
#[inline]
fn ray_triangle(o: V3, d: V3, a: V3, b: V3, c: V3, h: Hypot) -> Option<f64> {
    const EPS: f64 = 1e-7;
    const T_MIN: f64 = 1e-6;
    let e1 = V3(b.0 - a.0, b.1 - a.1, b.2 - a.2);
    let e2 = V3(c.0 - a.0, c.1 - a.1, c.2 - a.2);
    let p = d.cross(e2, h);
    let det = e1.dot(p, h) as f64;
    if det.abs() < EPS {
        return None;
    }
    let inv = 1.0 / det;
    let tvec = V3(o.0 - a.0, o.1 - a.1, o.2 - a.2);
    let u = tvec.dot(p, h) as f64 * inv;
    if u < -EPS || u > 1.0 + EPS {
        return None;
    }
    let q = tvec.cross(e1, h);
    let v = d.dot(q, h) as f64 * inv;
    if v < -EPS || u + v > 1.0 + EPS {
        return None;
    }
    let t = e2.dot(q, h) as f64 * inv;
    if t > T_MIN {
        Some(t)
    } else {
        None
    }
}

/// `_compute_inward_limit`'s lateral exit, exactly: the least `t > 1e-4`
/// at which a ray from one of `samples` along `direction` crosses a
/// triangle of a face that is not `skip`, not interior and not parallel to
/// the unit `n` (`|normal().normalized() · n| > 0.999`).
#[pyfunction]
#[allow(clippy::too_many_arguments)]
pub fn scan_lateral_exit(
    py: Python<'_>,
    mut scan: PyRefMut<'_, MeshScan>,
    samples: Vec<(f64, f64, f64)>,
    direction: (f64, f64, f64),
    n: (f64, f64, f64),
    skip: Option<u32>,
    hypot: &str,
    compensated: bool,
) -> PyResult<Option<f64>> {
    scan.need_parts(py, FFLAGS)?;
    let g: std::sync::Arc<Geom> = geom_of(py, &mut scan, hypot, compensated)?;
    let s: &MeshScan = &scan;
    let h = g.h;
    let d = V3(direction.0, direction.1, direction.2);
    let n = V3(n.0, n.1, n.2);
    let qs: Vec<V3> = samples.iter().map(|q| V3(q.0, q.1, q.2)).collect();
    Ok(py.detach(|| {
        (0..s.n_faces())
            .into_par_iter()
            .filter_map(|f| {
                if Some(f as u32) == skip || s.finterior[f] {
                    return None;
                }
                let gn = facegeom::normalized(g.normal[f], h);
                if (gn.dot(n, h) as f64).abs() > 0.999 {
                    return None;
                }
                let mut best: Option<f64> = None;
                for t in g.tris_of(f) {
                    let (a, b, c) = (g.pos[t[0] as usize], g.pos[t[1] as usize], g.pos[t[2] as usize]);
                    for &q in &qs {
                        if let Some(tt) = ray_triangle(q, d, a, b, c, h) {
                            if tt > 1e-4 && best.is_none_or(|bb| tt < bb) {
                                best = Some(tt);
                            }
                        }
                    }
                }
                best
            })
            .reduce_with(|a, b| if b < a { b } else { a })
    }))
}

/// Listed vertices (list order) inside any of `boxes` (`[x0, y0, z0, x1,
/// y1, z1]`, bounds included).
#[pyfunction]
pub fn scan_vertices_in_boxes(py: Python<'_>, mut scan: PyRefMut<'_, MeshScan>, boxes: Vec<[f64; 6]>) -> PyResult<Vec<u32>> {
    scan.need_parts(py, 0)?;
    let s: &MeshScan = &scan;
    let n = s.n_listed_vertices();
    Ok(py.detach(|| {
        let pos = s.positions();
        (0..n as u32)
            .into_par_iter()
            .filter(|&i| {
                let p = pos[i as usize];
                boxes.iter().any(|b| (0..3).all(|k| p[k] >= b[k] && p[k] <= b[k + 3]))
            })
            .collect()
    }))
}

/// `cap_rebuild.crack_planes`'s edges, exactly: listed edges bordering
/// one face with an end inside `near` (bounds included; every edge when
/// `near` is None), in list order.
#[pyfunction]
#[pyo3(signature = (scan, near=None))]
pub fn scan_cracked_edges(py: Python<'_>, mut scan: PyRefMut<'_, MeshScan>, near: Option<[f64; 6]>) -> PyResult<Vec<u32>> {
    scan.need_parts(py, 0)?;
    let s: &MeshScan = &scan;
    let pos = s.positions();
    let inside = |p: &[f64; 3]| match near {
        None => true,
        Some(b) => b[0] <= p[0] && p[0] <= b[3] && b[1] <= p[1] && p[1] <= b[4] && b[2] <= p[2] && p[2] <= b[5],
    };
    let enf = s.edge_face_counts();
    Ok((0..s.n_edges())
        .filter(|&e| {
            if enf[e] != 1 {
                return false;
            }
            let [a, b] = s.edge_ends(e);
            near.is_none() || inside(&pos[a as usize]) || inside(&pos[b as usize])
        })
        .map(|e| e as u32)
        .collect())
}

/// Faces (list order) whose every-ring box (holes too) meets `[lo, hi]`,
/// bounds included.
#[pyfunction]
pub fn scan_faces_meeting_rings(
    py: Python<'_>,
    mut scan: PyRefMut<'_, MeshScan>,
    lo: (f64, f64, f64),
    hi: (f64, f64, f64),
) -> PyResult<Vec<u32>> {
    scan.need_parts(py, RINGS)?;
    let s: &MeshScan = &scan;
    let w = [lo.0, lo.1, lo.2, hi.0, hi.1, hi.2];
    Ok(py.detach(|| {
        (0..s.n_faces() as u32)
            .into_par_iter()
            .filter(|&f| {
                let b = ring_box(s, f as usize);
                b[3] >= w[0] && b[0] <= w[3] && b[4] >= w[1] && b[1] <= w[4] && b[5] >= w[2] && b[2] <= w[5]
            })
            .collect()
    }))
}

/// The live keys of `kind` the store holds and its snapshot `snap` did not
/// (a slot dead there, or reused since under a newer generation), in list
/// order: what `[k for k in store.keys(kind) if not snap.has(k)]` gives,
/// without a Python int per element.
#[pyfunction]
pub fn store_keys_since(store: PyRef<'_, PyMeshStore>, snap: PyRef<'_, PyStoreSnapshot>, kind: &str) -> PyResult<Vec<u128>> {
    let k = Kind::parse(kind).ok_or_else(|| pyo3::exceptions::PyValueError::new_err(format!("kind {kind:?}")))?;
    let st = &store.inner;
    let old = &snap.inner.state;
    let same = old.ns == st.state.ns;
    let os = &old.slots[k as usize];
    let ns = &st.state.slots[k as usize];
    Ok(st
        .live_slots(k)
        .into_iter()
        .filter(|&s| !(same && os.alive(s) && os.gen(s) == ns.gen(s)))
        .map(|s| st.key(k, s))
        .collect())
}

/// `topology.sweep_tjunctions`' inputs for a scoped sweep: the listed
/// edges whose box meets `[lo, hi]` (bounds included), the listed vertices
/// inside the box of all of them grown by `tol` (a superset of every
/// vertex the sweep's slab tests can keep for one of them), both in list
/// order, the vertices' positions and the edges' ends as indices into
/// those vertices. None when an edge ends at a vertex the list does not
/// hold (the full arrays then).
#[pyfunction]
#[allow(clippy::type_complexity)]
pub fn scan_tjunction_arrays(
    py: Python<'_>,
    mut scan: PyRefMut<'_, MeshScan>,
    lo: (f64, f64, f64),
    hi: (f64, f64, f64),
    tol: f64,
) -> PyResult<Option<(Vec<u32>, Vec<u32>, Vec<f64>, Vec<i64>)>> {
    scan.need_parts(py, 0)?;
    let s: &MeshScan = &scan;
    let nl = s.n_listed_vertices();
    let w = [lo.0, lo.1, lo.2, hi.0, hi.1, hi.2];
    let pos = s.positions();
    let eidx: Vec<u32> = (0..s.n_edges() as u32)
        .into_par_iter()
        .filter(|&e| {
            let [a, b] = s.edge_ends(e as usize);
            let (pa, pb) = (pos[a as usize], pos[b as usize]);
            (0..3).all(|k| pa[k].max(pb[k]) >= w[k] && pa[k].min(pb[k]) <= w[k + 3])
        })
        .collect();
    let mut bx = [f64::INFINITY, f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for &e in &eidx {
        let [a, b] = s.edge_ends(e as usize);
        if a as usize >= nl || b as usize >= nl {
            return Ok(None);
        }
        for p in [pos[a as usize], pos[b as usize]] {
            for k in 0..3 {
                bx[k] = bx[k].min(p[k]);
                bx[k + 3] = bx[k + 3].max(p[k]);
            }
        }
    }
    let g = tol * (1.0 + 1e-6) + 1e-12;
    let vidx: Vec<u32> = if eidx.is_empty() {
        Vec::new()
    } else {
        (0..nl as u32)
            .into_par_iter()
            .filter(|&v| {
                let p = pos[v as usize];
                (0..3).all(|k| p[k] >= bx[k] - g && p[k] <= bx[k + 3] + g)
            })
            .collect()
    };
    let mut local = std::collections::HashMap::with_capacity(vidx.len());
    let mut flat = Vec::with_capacity(vidx.len() * 3);
    for (i, &v) in vidx.iter().enumerate() {
        local.insert(v, i as i64);
        flat.extend_from_slice(&pos[v as usize]);
    }
    let mut ends = Vec::with_capacity(eidx.len() * 2);
    for &e in &eidx {
        let [a, b] = s.edge_ends(e as usize);
        ends.push(local[&a]);
        ends.push(local[&b]);
    }
    Ok(Some((eidx, vidx, flat, ends)))
}

/// `core.topology._mesh_is_flat(mesh)` from face `start` on (the caller
/// walked the ones before): `faces[0]`'s normal `n0` and plane offset
/// `dot(n0, faces[0].vertices[0])`, then every face's normal coplanar with
/// it (`|n0.normalized() · n.normalized()| > 0.999`) and its first corner
/// within `tol` of that plane — in float32 through Qt's operations, as the
/// Python test. None when a face has no corner (Python raises there).
#[pyfunction]
pub fn scan_mesh_is_flat(
    py: Python<'_>,
    mut scan: PyRefMut<'_, MeshScan>,
    start: usize,
    tol: f64,
    hypot: &str,
    compensated: bool,
) -> PyResult<Option<bool>> {
    let g = geom_of(py, &mut scan, hypot, compensated)?;
    let s: &MeshScan = &scan;
    let nf = s.n_faces();
    if nf < 2 {
        return Ok(Some(true));
    }
    let h = g.h;
    let first = |f: usize| s.rings_of(f).first().and_then(|r| r.first().copied());
    let Some(c0) = first(0) else { return Ok(None) };
    let n0 = g.normal[0];
    let d0 = n0.dot(g.pos[c0 as usize], h) as f64;
    let u0 = facegeom::normalized(n0, h);
    for f in start.max(1)..nf {
        let un = facegeom::normalized(g.normal[f], h);
        if !((u0.dot(un, h) as f64).abs() > 0.999) {
            return Ok(Some(false));
        }
        let Some(c) = first(f) else { return Ok(None) };
        if ((n0.dot(g.pos[c as usize], h) as f64) - d0).abs() > tol {
            return Ok(Some(false));
        }
    }
    Ok(Some(true))
}
