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
use crate::scans::{MeshScan, EFACES, FFLAGS, RINGS};
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

pub(crate) fn grown_ring_box(scan: &MeshScan, f: usize, pad: f64, rel: f64) -> [f64; 6] {
    let mut b = ring_box(scan, f);
    if b[0].is_finite() {
        let diagonal = ((b[3] - b[0]).powi(2) + (b[4] - b[1]).powi(2) + (b[5] - b[2]).powi(2)).sqrt();
        let g = pad + rel * diagonal + slack(big(&b));
        for i in 0..3 {
            b[i] -= g;
            b[i + 3] += g;
        }
    }
    b
}

pub(crate) fn holds_point(b: &[f64; 6], p: [f64; 3]) -> bool {
    (0..3).all(|i| p[i] >= b[i] && p[i] <= b[i + 3])
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
                let b = grown_ring_box(s, f as usize, pad, rel);
                points.iter().all(|&(x, y, z)| holds_point(&b, [x, y, z]))
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
    let d = V3(direction.0, direction.1, direction.2);
    let n = V3(n.0, n.1, n.2);
    let qs: Vec<V3> = samples.iter().map(|q| V3(q.0, q.1, q.2)).collect();
    Ok(py.detach(|| lateral_exit_t(s, &g, &qs, d, n, skip)))
}

/// The least `t > 1e-4` at which a ray from one of `samples` along `d`
/// crosses a triangle of a face that is not `skip`, not interior and not
/// parallel to the unit `n` (`scan_lateral_exit`'s answer).
fn lateral_exit_t(s: &MeshScan, g: &Geom, qs: &[V3], d: V3, n: V3, skip: Option<u32>) -> Option<f64> {
    let h = g.h;
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
                for &q in qs {
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
}

fn plane_basis(n: V3, h: Hypot) -> (V3, V3) {
    let n = facegeom::normalized(n, h);
    let mut r = V3(1.0, 0.0, 0.0);
    if r.dot(n, h).abs() > 0.9 {
        r = V3(0.0, 1.0, 0.0);
    }
    let u = facegeom::normalized(r.sub(n.mul(r.dot(n, h))), h);
    (u, facegeom::normalized(n.cross(u, h), h))
}

/// `core.arrangement._point_in_polygon`: even-odd ray cast.
fn in_polygon(p: [f64; 2], poly: &[[f64; 2]]) -> bool {
    let (x, y) = (p[0], p[1]);
    let mut inside = false;
    let mut j = poly.len().wrapping_sub(1);
    for i in 0..poly.len() {
        let (xi, yi) = (poly[i][0], poly[i][1]);
        let (xj, yj) = (poly[j][0], poly[j][1]);
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi + 1e-30) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// `core.push_pull._loops_overlap_2d`: an interior point of one inside the
/// other, or a vertex of either strictly inside the other.
fn loops_overlap(py: Python<'_>, a: &[[f64; 2]], b: &[[f64; 2]]) -> PyResult<bool> {
    let ia = crate::store::planar::plane_interior_point(py, a.to_vec())?;
    if in_polygon([ia.0, ia.1], b) {
        return Ok(true);
    }
    let ib = crate::store::planar::plane_interior_point(py, b.to_vec())?;
    if in_polygon([ib.0, ib.1], a) {
        return Ok(true);
    }
    Ok(a.iter().any(|&p| in_polygon(p, b)) || b.iter().any(|&p| in_polygon(p, a)))
}

/// How far a face may be pushed INTO the solid (`PushPullOperation.
/// _compute_inward_limit`, docs/rust-tools.md §19): None for unbounded.
///
/// - A parallel face on the material side that overlaps the base laterally
///   and stands behind the WHOLE base (#1076) blocks at its distance; one
///   the base, carried back, lies strictly inside is a through-hole target.
/// - A backing wall that leans away from the push overflows at any depth:
///   0 (#710: judged in metres, the far end's stray from square).
/// - The swept loop leaving the solid through a non-parallel face, a hair
///   short of the graze.
#[pyfunction]
#[allow(clippy::too_many_arguments)]
pub fn scan_inward_limit(
    py: Python<'_>,
    mut scan: PyRefMut<'_, MeshScan>,
    base: u32,
    n: (f64, f64, f64),
    anchor: (f64, f64, f64),
    stitch_tol: f64,
    hypot: &str,
    compensated: bool,
) -> PyResult<Option<f64>> {
    scan.need_parts(py, RINGS | EFACES | FFLAGS)?;
    let g: std::sync::Arc<Geom> = geom_of(py, &mut scan, hypot, compensated)?;
    let s: &MeshScan = &scan;
    let h = g.h;
    let b = base as usize;
    let n = facegeom::normalized(V3(n.0, n.1, n.2), h);
    let anchor = V3(anchor.0, anchor.1, anchor.2);
    let rings = s.rings_of(b);
    let Some(outer) = rings.first() else { return Ok(None) };
    let base_pts: Vec<V3> = outer.iter().map(|&v| g.pos[v as usize]).collect();
    let (pu, pv) = plane_basis(n, h);
    let project = |pts: &[V3]| -> Vec<[f64; 2]> { pts.iter().map(|p| [p.dot(pu, h), p.dot(pv, h)]).collect() };
    let base_xy = project(&base_pts);
    let mut limit: Option<f64> = None;

    // Parallel blockers.
    for f in 0..s.n_faces() {
        if f == b {
            continue;
        }
        let gn = facegeom::normalized(g.normal[f], h);
        if gn.dot(n, h).abs() < 0.999 {
            continue; // not parallel
        }
        let c = g.centroid[f];
        let dist = anchor.sub(c).dot(n, h);
        if dist <= 1e-6 {
            continue; // coplanar or on the outward side
        }
        if base_pts.iter().map(|v| v.sub(c).dot(n, h)).fold(f64::INFINITY, f64::min) <= stitch_tol {
            continue; // not behind the whole base (#1076)
        }
        let grings = s.rings_of(f);
        let Some(gouter) = grings.first() else { continue };
        let back: Vec<[f64; 3]> = base_pts.iter().map(|v| { let q = v.sub(n.mul(dist)); [q.0, q.1, q.2] }).collect();
        let outline = crate::store::Outline::from_rings(
            grings.iter().map(|r| r.iter().map(|&v| { let p = g.pos[v as usize]; [p.0, p.1, p.2] }).collect()).collect(),
            g.normal[f], h);
        if outline.contains(&back, false) {
            continue; // a through-hole target, not a blocker
        }
        let gpts: Vec<V3> = gouter.iter().map(|&v| g.pos[v as usize]).collect();
        if !loops_overlap(py, &base_xy, &project(&gpts))? {
            continue; // laterally elsewhere
        }
        if limit.is_none_or(|l| dist < l) {
            limit = Some(dist);
        }
    }

    // Immediate lateral exit: a backing wall leaning away from the push.
    let direction = n.mul(-1.0);
    let mut edge_of: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
    for e in 0..s.n_edges() {
        let [a, z] = s.edge_ends(e);
        edge_of.entry((a.min(z), a.max(z))).or_insert(e as u32);
    }
    for ring in &rings {
        let cnt = ring.len();
        for i in 0..cnt {
            let (a, z) = (ring[i], ring[(i + 1) % cnt]);
            let Some(&e) = edge_of.get(&(a.min(z), a.max(z))) else { continue };
            for &f in s.faces_of_edge(e as usize) {
                let f = f as usize;
                if f == b || f >= s.n_faces() || s.finterior[f] {
                    continue;
                }
                let gn = facegeom::normalized(g.normal[f], h);
                if gn.dot(n, h).abs() > 0.999 {
                    continue; // parallel: not a lateral wall
                }
                let lean = direction.dot(gn, h);
                if lean <= 1e-6 {
                    continue;
                }
                let root = g.pos[a as usize];
                let depth = s.rings_of(f).first().map(|r| r.iter()
                    .map(|&v| g.pos[v as usize].sub(root).dot(direction, h))
                    .fold(f64::NEG_INFINITY, f64::max)).unwrap_or(f64::NEG_INFINITY);
                if lean * depth > stitch_tol {
                    return Ok(Some(0.0));
                }
            }
        }
    }

    // Lateral exit bound: march inward from points just inside the base.
    let (u, w) = plane_basis(n, h);
    let origin3 = base_pts[0];
    let loop_xy: Vec<[f64; 2]> = base_pts.iter().map(|p| { let r = p.sub(origin3); [r.dot(u, h), r.dot(w, h)] }).collect();
    let ip = crate::store::planar::plane_interior_point(py, loop_xy)?;
    let ip3 = origin3.add(u.mul(ip.0)).add(w.mul(ip.1));
    let mut samples = vec![ip3];
    for (i, p) in base_pts.iter().enumerate() {
        samples.push(p.add(ip3.sub(*p).mul(1e-3)));
        let mid = p.add(base_pts[(i + 1) % base_pts.len()]).mul(0.5);
        samples.push(mid.add(ip3.sub(mid).mul(1e-3)));
    }
    if let Some(t) = py.detach(|| lateral_exit_t(s, &g, &samples, direction, n, Some(base))) {
        let t = (t - 1e-3).max(0.0);
        if limit.is_none_or(|l| t < l) {
            limit = Some(t);
        }
    }
    Ok(limit)
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

// ---- The base's classification (`PushPullOperation`, R2.5 slice 9) ----------

/// Python's `_key`: the 1e-4 grid cell, rounded half to even.
fn cell(p: V3) -> [i64; 3] {
    [(p.0 * 1e4).round_ties_even() as i64, (p.1 * 1e4).round_ties_even() as i64, (p.2 * 1e4).round_ties_even() as i64]
}

/// `core.topology._point_on_seg_incl` with its 1e-4 tolerance.
fn on_seg_incl(pt: V3, p: V3, q: V3, h: Hypot) -> bool {
    const TOL: f64 = 1e-4;
    let pq = q.sub(p);
    let length = facegeom::length(pq, h);
    if length < TOL {
        return cell(pt) == cell(p);
    }
    let t = pt.sub(p).dot(pq, h) / (length * length);
    if !(-TOL..=1.0 + TOL).contains(&t) {
        return false;
    }
    facegeom::length(pt.sub(p.add(pq.mul(t))), h) < TOL
}

fn unit(v: V3, h: Hypot) -> V3 { facegeom::normalized(v, h) }

/// `_faces_coplanar`: both normals normalised, |cos| > 0.999.
fn coplanar(a: V3, b: V3, h: Hypot) -> bool { unit(a, h).dot(unit(b, h), h).abs() > 0.999 }

/// Whether `a`–`b` lies on a boundary edge of face `f`, holes included
/// (`core.topology._segment_on_face_boundary`).
fn segment_on_boundary(s: &MeshScan, g: &Geom, f: usize, a: V3, b: V3) -> bool {
    s.rings_of(f).iter().any(|ring| {
        let n = ring.len();
        (0..n).any(|i| {
            let (p, q) = (g.pos[ring[i] as usize], g.pos[ring[(i + 1) % n] as usize]);
            on_seg_incl(a, p, q, g.h) && on_seg_incl(b, p, q, g.h)
        })
    })
}

/// Whether one of face `f`'s edges (holes too) joins the cells of `a`
/// and `b` (`frozenset((_key(a), _key(b))) in _face_all_edges(g)`).
fn has_cell_edge(s: &MeshScan, g: &Geom, f: usize, a: V3, b: V3) -> bool {
    let (ka, kb) = (cell(a), cell(b));
    s.rings_of(f).iter().any(|ring| {
        let n = ring.len();
        (0..n).any(|i| {
            let (p, q) = (cell(g.pos[ring[i] as usize]), cell(g.pos[ring[(i + 1) % n] as usize]));
            (p == ka && q == kb) || (p == kb && q == ka)
        })
    })
}

/// The faces near a rim: their rings' boxes, for those meeting the rim's
/// box (grown), in list order. One pass over the mesh per question.
struct Near {
    faces: Vec<usize>,
    boxes: Vec<[f64; 6]>,
}

impl Near {
    fn of(s: &MeshScan, pts: impl Iterator<Item = V3>) -> Near {
        let mut r = [f64::INFINITY, f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for p in pts {
            for (k, x) in [p.0, p.1, p.2].into_iter().enumerate() {
                r[k] = r[k].min(x);
                r[k + 3] = r[k + 3].max(x);
            }
        }
        let m = 2e-4 + slack(big(&r));
        let hits: Vec<(usize, [f64; 6])> = (0..s.n_faces())
            .into_par_iter()
            .filter_map(|f| {
                let b = ring_box(s, f);
                let meets = (0..3).all(|k| b[k] <= r[k + 3] + m && b[k + 3] >= r[k] - m);
                meets.then_some((f, b))
            })
            .collect();
        let (faces, boxes) = hits.into_iter().unzip();
        Near { faces, boxes }
    }

    /// Those whose box holds both `a` and `b` (a superset of the faces that
    /// can carry the segment), in list order.
    fn on_segment(&self, a: V3, b: V3) -> Vec<usize> {
        let m = slack(big(&[a.0, a.1, a.2, b.0, b.1, b.2]));
        let inside = |bx: &[f64; 6], p: V3| {
            p.0 >= bx[0] - m && p.1 >= bx[1] - m && p.2 >= bx[2] - m
                && p.0 <= bx[3] + m && p.1 <= bx[4] + m && p.2 <= bx[5] + m
        };
        self.faces.iter().zip(&self.boxes).filter(|(_, bx)| inside(bx, a) && inside(bx, b)).map(|(f, _)| *f).collect()
    }
}

/// `core.topology.classify_push_edge(face, a, b, faces)`: the first
/// coplanar face carrying the edge, else the first other one, else free.
fn classify_edge(s: &MeshScan, g: &Geom, near: &Near, face: usize, a: V3, b: V3) -> (&'static str, Option<usize>) {
    let n = g.normal[face];
    let cands = near.on_segment(a, b);
    let mut other = None;
    for &f in &cands {
        if f == face {
            continue;
        }
        if coplanar(n, g.normal[f], g.h) {
            if has_cell_edge(s, g, f, a, b) || segment_on_boundary(s, g, f, a, b) {
                return ("coplanar", Some(f));
            }
        } else if other.is_none() && segment_on_boundary(s, g, f, a, b) {
            other = Some(f);
        }
    }
    match other {
        Some(f) => ("perp", Some(f)),
        None => ("free", None),
    }
}

/// `_swept_by_push`: an edge of `f`'s outer loop runs with the push.
fn swept_by_push(s: &MeshScan, g: &Geom, f: usize, normal: V3) -> bool {
    let n = unit(normal, g.h);
    let Some(outer) = s.rings_of(f).first().copied() else { return false };
    let m = outer.len();
    (0..m).any(|i| {
        let e = g.pos[outer[(i + 1) % m] as usize].sub(g.pos[outer[i] as usize]);
        facegeom::length(e, g.h) > 1e-9 && unit(e, g.h).dot(n, g.h).abs() > 0.999
    })
}

/// `_corner_dragged`: moving the `rim` cells along `normal` bends face `f`
/// at one of its corners there.
fn corner_dragged(s: &MeshScan, g: &Geom, f: usize, rim: &std::collections::HashSet<[i64; 3]>, normal: V3) -> bool {
    let n = unit(normal, g.h);
    for ring in s.rings_of(f) {
        let m = ring.len();
        for i in 0..m {
            let v = g.pos[ring[i] as usize];
            if !rim.contains(&cell(v)) {
                continue;
            }
            let a = g.pos[ring[(i + m - 1) % m] as usize].sub(v);
            let b = g.pos[ring[(i + 1) % m] as usize].sub(v);
            if facegeom::length(a, g.h) < 1e-9 || facegeom::length(b, g.h) < 1e-9
                || unit(a, g.h).dot(n, g.h).abs() < 0.999 || unit(b, g.h).dot(n, g.h).abs() < 0.999 {
                return true;
            }
        }
    }
    false
}

/// `PushPullOperation._classify_base`: `(attached, prism_cap)` of face
/// `base`. Attached: every rim edge (holes too) is carried by another
/// face. Prism cap: every one backs onto a wall the push sweeps, and no
/// other face has a corner on the rim the translate would bend.
#[pyfunction]
pub fn scan_classify_base(py: Python<'_>, mut scan: PyRefMut<'_, MeshScan>, base: u32, hypot: &str,
                          compensated: bool) -> PyResult<(bool, bool)> {
    scan.need_parts(py, RINGS | EFACES)?;
    let g: std::sync::Arc<Geom> = geom_of(py, &mut scan, hypot, compensated)?;
    let s: &MeshScan = &scan;
    let b = base as usize;
    let rings: Vec<Vec<V3>> = s.rings_of(b).iter().map(|r| r.iter().map(|&v| g.pos[v as usize]).collect()).collect();
    let near = Near::of(s, rings.iter().flatten().copied());
    let mut kinds = vec![];
    for ring in &rings {
        let n = ring.len();
        for i in 0..n {
            kinds.push(classify_edge(s, &g, &near, b, ring[i], ring[(i + 1) % n]));
        }
    }
    let normal = g.normal[b];
    let attached = kinds.iter().all(|(k, _)| *k != "free");
    let mut prism = !kinds.is_empty()
        && kinds.iter().all(|(k, nb)| *k == "perp" && nb.is_some_and(|f| swept_by_push(s, &g, f, normal)));
    if prism {
        let walls: std::collections::HashSet<usize> = kinds.iter().filter_map(|(_, nb)| *nb).collect();
        let rim: std::collections::HashSet<[i64; 3]> = rings.iter().flatten().map(|p| cell(*p)).collect();
        prism = !near.faces.iter().any(|&f| {
            f != b && !walls.contains(&f)
                && s.rings_of(f).iter().any(|r| r.iter().any(|&v| rim.contains(&cell(g.pos[v as usize]))))
                && corner_dragged(s, &g, f, &rim, normal)
        });
    }
    Ok((attached, prism))
}

/// `PushPullOperation._sweeps_a_free_sheet`: no face off the plane of
/// `face` stands on an edge of the (refined) rim `loops`.
#[pyfunction]
pub fn scan_sweeps_free_sheet(py: Python<'_>, mut scan: PyRefMut<'_, MeshScan>, face: u32, loops: Vec<Vec<[f64; 3]>>,
                              hypot: &str, compensated: bool) -> PyResult<bool> {
    scan.need_parts(py, RINGS | EFACES)?;
    let g: std::sync::Arc<Geom> = geom_of(py, &mut scan, hypot, compensated)?;
    Ok(sweeps_free_sheet(&scan, &g, face, &loops))
}
pub(crate) fn sweeps_free_sheet(s: &MeshScan, g: &Geom, face: u32, loops: &[Vec<[f64; 3]>]) -> bool {
    let f0 = face as usize;
    let near = Near::of(s, loops.iter().flatten().map(|p| V3(p[0], p[1], p[2])));
    let normal = g.normal[f0];
    for lp in loops {
        let m = lp.len();
        for i in 0..m {
            let (a, b) = (lp[i], lp[(i + 1) % m]);
            let (a, b) = (V3(a[0], a[1], a[2]), V3(b[0], b[1], b[2]));
            for f in near.on_segment(a, b) {
                if f != f0 && !coplanar(normal, g.normal[f], g.h) && segment_on_boundary(s, g, f, a, b) {
                    return false;
                }
            }
        }
    }
    true
}

/// `_surface_normal`: `face`'s unit normal, turned to agree with the
/// majority of its coplanar neighbours across its outer loop's edges.
#[pyfunction]
pub fn scan_surface_normal(py: Python<'_>, mut scan: PyRefMut<'_, MeshScan>, face: u32, hypot: &str,
                           compensated: bool) -> PyResult<(f64, f64, f64)> {
    scan.need_parts(py, RINGS | EFACES)?;
    let g: std::sync::Arc<Geom> = geom_of(py, &mut scan, hypot, compensated)?;
    let s: &MeshScan = &scan;
    let f0 = face as usize;
    let n = unit(g.normal[f0], g.h);
    let outer: Vec<u32> = s.rings_of(f0).first().map(|r| r.to_vec()).unwrap_or_default();
    let on_loop: std::collections::HashSet<u32> = outer.iter().copied().collect();
    let mut by_vertex: std::collections::HashMap<u32, Vec<usize>> = std::collections::HashMap::new();
    for e in 0..s.n_edges() {
        let [a, b] = s.edge_ends(e);
        if on_loop.contains(&a) && on_loop.contains(&b) {
            by_vertex.entry(a).or_default().push(e);
            if b != a {
                by_vertex.entry(b).or_default().push(e);
            }
        }
    }
    let (mut agree, mut disagree) = (0, 0);
    for v in &outer {
        for &e in by_vertex.get(v).map(|x| x.as_slice()).unwrap_or(&[]) {
            for &f in s.faces_of_edge(e) {
                let f = f as usize;
                if f == f0 || f >= s.n_faces() {
                    continue;
                }
                let d = unit(g.normal[f], g.h).dot(n, g.h);
                if d.abs() > 0.999 {
                    if d > 0.0 { agree += 1 } else { disagree += 1 }
                }
            }
        }
    }
    let out = if disagree > agree { n.mul(-1.0) } else { n };
    Ok((out.0, out.1, out.2))
}

/// `_cap_passes_a_wall_vertex` on a Python mesh: shrinking the prism cap
/// by `d` passes a vertex partway up one of the faces with a corner on the
/// cap's rim (`push_band::passes_wall`).
#[pyfunction]
pub fn scan_cap_passes_wall(py: Python<'_>, mut scan: PyRefMut<'_, MeshScan>, face: u32, cap: Vec<[f64; 3]>,
                            normal: (f64, f64, f64), d: f64, hypot: &str, compensated: bool) -> PyResult<bool> {
    scan.need_parts(py, RINGS | EFACES)?;
    let g: std::sync::Arc<Geom> = geom_of(py, &mut scan, hypot, compensated)?;
    let s: &MeshScan = &scan;
    let Some(origin) = cap.first().map(|p| V3(p[0], p[1], p[2])) else { return Ok(false) };
    let n = V3(normal.0, normal.1, normal.2);
    let rim: std::collections::HashSet<[i64; 3]> = cap.iter().map(|p| cell(V3(p[0], p[1], p[2]))).collect();
    let near = Near::of(s, cap.iter().map(|p| V3(p[0], p[1], p[2])));
    let mut walls = vec![];
    for &f in &near.faces {
        if f == face as usize {
            continue;
        }
        let Some(outer) = s.rings_of(f).first().copied() else { continue };
        let pts: Vec<V3> = outer.iter().map(|&v| g.pos[v as usize]).collect();
        if !pts.iter().any(|p| rim.contains(&cell(*p))) {
            continue;
        }
        walls.push(pts.iter().filter(|p| !rim.contains(&cell(**p))).map(|p| origin.sub(*p).dot(n, g.h)).collect());
    }
    Ok(crate::store::push_band::passes_wall(&walls, d))
}
