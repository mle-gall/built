// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! `built_core.orient.orient_outward` over a `MeshScan`, decision for decision.
//!
//! Orientation decides every face of the pushed solid's edge-connected
//! component by ray parity: from a probe on the face, seven rays (the
//! normal and six jittered around it) vote on whether the side ahead is
//! outside. On the ride's largest mesh that component is all 267k faces,
//! twice per push/pull click (the interior rounds, then the flip pass) and
//! again at the stage — ~40 s each in Python, almost all of it the Python
//! around each query, not the rays.
//!
//! Everything a decision reads is reproduced here bit for bit:
//! - binary64 model geometry shared with the store: `Face.normal()`, `centroid()`, `triangulate()`,
//!   `_interior_probe`, `_on_face`, `_basis`, the jittered directions;
//! - the jitter itself: Python's `random.Random(seed)` (MT19937 seeded by
//!   `init_by_array`, `random()` = `genrand_res53`, `uniform`), one stream
//!   per round shared by the faces in list order — so the offset a face
//!   draws from depends on how many queries the faces before it made;
//! - the crossing counts: `crate::parity`'s f64 Möller–Trumbore over a ray
//!   index with the same splits and box margins as `parity::Tris` (built in
//!   parallel; the pruning is node for node the same, so are the counts);
//! - the rounds, the closed / open rules and the rollback of a flip that
//!   leaves a solid less consistent (decided before anything is turned,
//!   so the rollback no longer writes loops back — or materialises a
//!   store-backed mesh).
//!
//! The faces are judged in parallel. A face's jitter offset is the sum of
//! the draws of the faces before it (12 per query: one query, or two when
//! the side ahead reads inside), so the offsets are first PREDICTED from
//! the unjittered ray alone (cheap, and wrong only where a graze makes the
//! vote differ from it), the faces are judged at the predicted offsets in
//! parallel, and a sequential walk checks every offset; from a wrong one
//! on, the rest of the chunk is judged again at the corrected offsets. The
//! result is the sequential loop's whatever the predictions.

use crate::store::geometry::{self as facegeom, face_centroid, face_normal, plane_basis, Hypot, V3};
use crate::scans::{MeshScan, EFACES, FFLAGS, RINGS};
use crate::store::keys::{split_key, Kind, NONE};
use pyo3::prelude::*;
use pyo3::types::{PyList, PyString};
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

/// `BUILT_ORIENT_TIMING=1`: phase times on stderr (benches).
fn lap(what: &str, t: &mut std::time::Instant) {
    if std::env::var_os("BUILT_ORIENT_TIMING").is_some() {
        eprintln!("  orient {what}: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    }
    *t = std::time::Instant::now();
}

// ---- Binary64 model arithmetic ---------------------------------------------

#[inline]
fn add(a: V3, b: V3) -> V3 {
    V3(a.0 + b.0, a.1 + b.1, a.2 + b.2)
}
#[inline]
fn sub(a: V3, b: V3) -> V3 {
    V3(a.0 - b.0, a.1 - b.1, a.2 - b.2)
}
#[inline]
fn mul(a: V3, s: f64) -> V3 {
    V3(a.0 * s, a.1 * s, a.2 * s)
}
#[inline]
fn div(a: V3, s: f64) -> V3 {
    V3(a.0 / s, a.1 / s, a.2 / s)
}
#[inline]
fn neg(a: V3) -> V3 {
    V3(-a.0, -a.1, -a.2)
}
#[inline]
fn f64s(a: V3) -> [f64; 3] {
    [a.0 as f64, a.1 as f64, a.2 as f64]
}

// ---- per-face geometry -------------------------------------------------------

/// The per-revision face geometry the push/pull passes share: what
/// `Face.normal()`, `centroid()` and `triangulate()` return, per face.
pub struct Geom {
    pub h: Hypot,
    pub pos: Vec<V3>,
    pub normal: Vec<V3>,
    pub centroid: Vec<V3>,
    /// Face `f`'s triangles are `tris[tstart[f]..tstart[f+1]]` (vertex
    /// indices into `pos`).
    pub tstart: Vec<u32>,
    pub tris: Vec<[u32; 3]>,
}

impl Geom {
    /// Every face's geometry, in parallel.
    pub fn of(scan: &MeshScan, h: Hypot, compensated: bool) -> Geom {
        let pos: Vec<V3> = scan
            .positions()
            .iter()
            .map(|p| V3(p[0], p[1], p[2]))
            .collect();
        let nf = scan.n_faces();
        let per: Vec<(V3, V3, Vec<[u32; 3]>)> = (0..nf)
            .into_par_iter()
            .map(|f| {
                let rings = scan.rings_of(f);
                let outer: &[u32] = rings.first().copied().unwrap_or(&[]);
                let n = face_normal(&pos, outer, h);
                let c = face_centroid(&pos, outer, compensated);
                let t = if outer.len() < 3 {
                    Vec::new()
                } else {
                    facegeom::triangles(&pos, &rings, n, h)
                };
                (n, c, t)
            })
            .collect();
        let mut g = Geom {
            h,
            pos,
            normal: Vec::with_capacity(nf),
            centroid: Vec::with_capacity(nf),
            tstart: Vec::with_capacity(nf + 1),
            tris: Vec::new(),
        };
        g.tstart.push(0);
        for (n, c, t) in per {
            g.normal.push(n);
            g.centroid.push(c);
            g.tris.extend_from_slice(&t);
            g.tstart.push(g.tris.len() as u32);
        }
        g
    }

    pub fn tris_of(&self, f: usize) -> &[[u32; 3]] {
        &self.tris[self.tstart[f] as usize..self.tstart[f + 1] as usize]
    }
}

/// `built_core.arrangement.plane_basis(normal)`.
fn origin_in_polygon(poly: &[(f64, f64)]) -> bool {
    let (x, y) = (0.0f64, 0.0f64);
    let mut inside = false;
    let n = poly.len();
    if n == 0 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if ((yi > y) != (yj > y)) && (x < (xj - xi) * (y - yi) / (yj - yi + 1e-30) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// `built_core.orient._on_face(face, p)`.
fn on_face(g: &Geom, f: usize, outer: &[u32], p: V3) -> bool {
    let h = g.h;
    let n = g.normal[f];
    if (facegeom::length(n, h) as f64) < 1e-12 {
        return true;
    }
    let (u, v) = plane_basis(facegeom::normalized(n, h), h);
    let (ux, uy, uz) = (u.0 as f64, u.1 as f64, u.2 as f64);
    let (vx, vy, vz) = (v.0 as f64, v.1 as f64, v.2 as f64);
    let poly: Vec<(f64, f64)> = outer
        .iter()
        .map(|&q| {
            let r = sub(g.pos[q as usize], p);
            let (rx, ry, rz) = (r.0 as f64, r.1 as f64, r.2 as f64);
            (rx * ux + ry * uy + rz * uz, rx * vx + ry * vy + rz * vz)
        })
        .collect();
    origin_in_polygon(&poly)
}

/// `built_core.orient._interior_probe(face)`.
pub(crate) fn interior_probe(g: &Geom, f: usize, rings: &[&[u32]]) -> V3 {
    let h = g.h;
    let outer: &[u32] = rings.first().copied().unwrap_or(&[]);
    if rings.len() <= 1 {
        let c = g.centroid[f];
        if on_face(g, f, outer, c) {
            return c;
        }
    }
    let tris = g.tris_of(f);
    if tris.is_empty() {
        return g.centroid[f];
    }
    let mut best = 0usize;
    let mut best_key = 0.0f64;
    for (k, t) in tris.iter().enumerate() {
        let (a, b, c) = (g.pos[t[0] as usize], g.pos[t[1] as usize], g.pos[t[2] as usize]);
        let cr = sub(b, a).cross(sub(c, a), h);
        let key = cr.dot(cr, h);
        if k == 0 || key > best_key {
            best = k;
            best_key = key;
        }
    }
    let t = tris[best];
    div(add(add(g.pos[t[0] as usize], g.pos[t[1] as usize]), g.pos[t[2] as usize]), 3.0)
}

// ---- Python's random.Random ---------------------------------------------------

/// CPython's `random.Random`: MT19937 seeded by `init_by_array` with the
/// 32-bit words of `abs(seed)`.
#[derive(Clone)]
pub struct PyRandom {
    mt: Box<[u32; 624]>,
    idx: usize,
}

impl PyRandom {
    pub fn new(seed: u64) -> PyRandom {
        let mut key: Vec<u32> = Vec::new();
        let mut n = seed;
        while n != 0 {
            key.push(n as u32);
            n >>= 32;
        }
        if key.is_empty() {
            key.push(0);
        }
        let mut r = PyRandom { mt: Box::new([0u32; 624]), idx: 624 };
        r.init_genrand(19650218);
        let (mut i, mut j) = (1usize, 0usize);
        let len = key.len();
        let mut k = if 624 > len { 624 } else { len };
        while k > 0 {
            let prev = r.mt[i - 1];
            r.mt[i] = (r.mt[i] ^ ((prev ^ (prev >> 30)).wrapping_mul(1664525)))
                .wrapping_add(key[j])
                .wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= 624 {
                r.mt[0] = r.mt[623];
                i = 1;
            }
            if j >= len {
                j = 0;
            }
            k -= 1;
        }
        k = 623;
        while k > 0 {
            let prev = r.mt[i - 1];
            r.mt[i] = (r.mt[i] ^ ((prev ^ (prev >> 30)).wrapping_mul(1566083941))).wrapping_sub(i as u32);
            i += 1;
            if i >= 624 {
                r.mt[0] = r.mt[623];
                i = 1;
            }
            k -= 1;
        }
        r.mt[0] = 0x8000_0000;
        r
    }

    fn init_genrand(&mut self, s: u32) {
        self.mt[0] = s;
        for i in 1..624 {
            let prev = self.mt[i - 1];
            self.mt[i] = 1812433253u32.wrapping_mul(prev ^ (prev >> 30)).wrapping_add(i as u32);
        }
        self.idx = 624;
    }

    fn twist(&mut self) {
        const UPPER: u32 = 0x8000_0000;
        const LOWER: u32 = 0x7fff_ffff;
        const MATRIX_A: u32 = 0x9908_b0df;
        let mt = &mut self.mt;
        for kk in 0..624 {
            let y = (mt[kk] & UPPER) | (mt[(kk + 1) % 624] & LOWER);
            let mag = if y & 1 != 0 { MATRIX_A } else { 0 };
            mt[kk] = mt[(kk + 397) % 624] ^ (y >> 1) ^ mag;
        }
        self.idx = 0;
    }

    fn skip_draws(&mut self, draws: usize) {
        let mut words = draws * 2;
        while words != 0 {
            if self.idx == 624 { self.twist(); }
            let step = words.min(624 - self.idx);
            self.idx += step;
            words -= step;
        }
    }

    fn next_u32(&mut self) -> u32 {
        if self.idx == 624 { self.twist(); }
        let mut y = self.mt[self.idx];
        self.idx += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^= y >> 18;
        y
    }

    /// `random()`: `genrand_res53`.
    pub fn random(&mut self) -> f64 {
        let a = (self.next_u32() >> 5) as f64;
        let b = (self.next_u32() >> 6) as f64;
        (a * 67108864.0 + b) * (1.0 / 9007199254740992.0)
    }
}

// Distant sheet faces consume the same draws on successive orientations.
// Save an exact MT state every 65536 draws for the most recent seed on
// this thread. Larger offsets can always use the uncached jump.
fn skip_sheet_draws(rng: &mut PyRandom, seed: u64, consumed: usize, skip: usize) {
    const STRIDE: usize = 65536;
    const MAX_CHECKPOINTS: usize = 256;
    let target = consumed + skip;
    let checkpoint = target / STRIDE;
    if skip < STRIDE || checkpoint >= MAX_CHECKPOINTS {
        rng.skip_draws(skip);
        return;
    }
    thread_local! {
        static SAVED: std::cell::RefCell<(u64, Vec<PyRandom>)> = const { std::cell::RefCell::new((0, Vec::new())) };
    }
    SAVED.with_borrow_mut(|(last_seed, states)| {
        if *last_seed != seed || states.is_empty() {
            *last_seed = seed;
            states.clear();
            states.push(PyRandom::new(seed));
        }
        while states.len() <= checkpoint {
            let mut next = states.last().unwrap().clone();
            next.skip_draws(STRIDE);
            states.push(next);
        }
        rng.clone_from(&states[checkpoint]);
    });
    rng.skip_draws(target % STRIDE);
}

/// The jitter draws of one round, `uniform(-jitter, jitter)` each, made as
/// far as the queries need them.
struct Stream {
    rng: PyRandom,
    lo: f64,
    hi: f64,
    vals: Vec<f64>,
}

impl Stream {
    fn new(seed: u64, jitter: f64) -> Stream {
        Stream { rng: PyRandom::new(seed), lo: -jitter, hi: jitter, vals: Vec::new() }
    }
    fn ensure(&mut self, n: usize) {
        while self.vals.len() < n {
            // random.uniform: a + (b - a) * random()
            let r = self.rng.random();
            self.vals.push(self.lo + (self.hi - self.lo) * r);
        }
    }
}

// ---- the ray index: parity::Tris's splits, built in parallel ------------------

const EPS: f64 = 1e-7;
const T_MIN: f64 = 1e-6;
const LEAF: usize = 8;
const PAR_MIN: usize = 4096;

#[inline]
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
#[inline]
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn grown(lo: [f64; 3], hi: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let mut ext = 0.0f64;
    let mut big = 0.0f64;
    for k in 0..3 {
        ext = ext.max(hi[k] - lo[k]);
        big = big.max(lo[k].abs()).max(hi[k].abs());
    }
    let m = 1e-5 + 1e-6 * big + 1e-5 * ext;
    ([lo[0] - m, lo[1] - m, lo[2] - m], [hi[0] + m, hi[1] + m, hi[2] + m])
}

struct Node {
    lo: [f64; 3],
    hi: [f64; 3],
    // All triangle corners in this node share this axis coordinate.
    plane: Option<(usize, f64)>,
    /// Leaf: `order[start..end]`; inner: children `kids`.
    start: u32,
    end: u32,
    kids: [u32; 2],
    leaf: bool,
}

/// An envelope of the leaves outside one sheet plane. Every crossing from
/// a point on that plane must pass a box in this envelope: the remaining
/// leaves have an exactly zero ray-distance numerator.
struct SheetWindow {
    axis: usize,
    plane: f64,
    lo: [f64; 3],
    hi: [f64; 3],
}

impl SheetWindow {
    fn of(nodes: &[Node]) -> Option<Self> {
        if nodes.len() < 256 { return None; }
        let (axis, plane) = nodes.iter().find(|n| n.leaf)?.plane?;
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for n in nodes.iter().filter(|n| n.leaf && n.plane != Some((axis, plane))) {
            for k in 0..3 {
                if !n.lo[k].is_finite() || !n.hi[k].is_finite() { return None; }
                lo[k] = lo[k].min(n.lo[k]);
                hi[k] = hi[k].max(n.hi[k]);
            }
        }
        Some(Self { axis, plane, lo, hi })
    }

    fn misses(&self, point: V3, normal: V3, jitter: f64) -> bool {
        let k = self.axis;
        let (p, n) = (f64s(point), f64s(normal));
        let (u, v) = ((k + 1) % 3, (k + 2) % 3);
        if p[k] != self.plane || n[k].abs() != 1. || n[u] != 0. || n[v] != 0. || !(jitter.abs() <= 0.01) {
            return false;
        }
        if self.lo[k] > self.hi[k] { return true; }
        let travel = (self.hi[k] - p[k]).abs().max((self.lo[k] - p[k]).abs());
        // Both normalized tangent slopes are bounded by the uniform jitter.
        // Allow for floating arithmetic and the BVH's parallel-ray floor.
        let spread = (travel + 1.) * (2. * jitter.abs() + 1e-12);
        [u, v].into_iter().any(|i| {
            let pad = spread + 1e-10 * (1. + p[i].abs() + self.lo[i].abs() + self.hi[i].abs());
            p[i] < self.lo[i] - pad || p[i] > self.hi[i] + pad
        })
    }
}

/// `parity::Tris` with the same triangles in the same order, split at the
/// same medians, so every box prunes what the original's prunes.
pub struct RayIndex {
    sheet: Option<SheetWindow>,
    a: Vec<[f64; 3]>,
    e1: Vec<[f64; 3]>,
    e2: Vec<[f64; 3]>,
    face: Vec<u32>,
    order: Vec<u32>,
    nodes: Vec<Node>,
}

struct Sub {
    nodes: Vec<Node>,
}

impl RayIndex {
    pub fn new(a: Vec<[f64; 3]>, e1: Vec<[f64; 3]>, e2: Vec<[f64; 3]>, face: Vec<u32>) -> RayIndex {
        let n = a.len();
        let mut tlo = vec![[0.0; 3]; n];
        let mut thi = vec![[0.0; 3]; n];
        let mut cen = vec![[0.0; 3]; n];
        tlo.par_iter_mut()
            .zip(thi.par_iter_mut())
            .zip(cen.par_iter_mut())
            .enumerate()
            .for_each(|(i, ((lo, hi), ce))| {
                let b = [a[i][0] + e1[i][0], a[i][1] + e1[i][1], a[i][2] + e1[i][2]];
                let c = [a[i][0] + e2[i][0], a[i][1] + e2[i][1], a[i][2] + e2[i][2]];
                for k in 0..3 {
                    lo[k] = a[i][k].min(b[k]).min(c[k]);
                    hi[k] = a[i][k].max(b[k]).max(c[k]);
                }
                *ce = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5];
            });
        let mut order: Vec<u32> = (0..n as u32).collect();
        let mut nodes = Vec::new();
        if n > 0 {
            let sub = Self::build(&mut order, 0, &tlo, &thi, &cen);
            nodes = sub.nodes;
        }
        RayIndex { sheet: SheetWindow::of(&nodes), a, e1, e2, face, order, nodes }
    }

    /// The subtree over `order` (a slice of the whole starting at `base`),
    /// root first; children numbered within the returned vector.
    fn build(order: &mut [u32], base: usize, tlo: &[[f64; 3]], thi: &[[f64; 3]], cen: &[[f64; 3]]) -> Sub {
        let len = order.len();
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        let mut clo = [f64::INFINITY; 3];
        let mut chi = [f64::NEG_INFINITY; 3];
        for &r in order.iter() {
            let r = r as usize;
            for k in 0..3 {
                lo[k] = lo[k].min(tlo[r][k]);
                hi[k] = hi[k].max(thi[r][k]);
                clo[k] = clo[k].min(cen[r][k]);
                chi[k] = chi[k].max(cen[r][k]);
            }
        }
        let (mut glo, mut ghi) = grown(lo, hi);
        if !(glo.iter().chain(ghi.iter()).all(|v| v.is_finite())) {
            glo = [f64::NEG_INFINITY; 3];
            ghi = [f64::INFINITY; 3];
        }
        let plane = (0..3).find(|&k| lo[k] == hi[k]).map(|k| (k, lo[k]));
        let mut root = Node { plane, lo: glo, hi: ghi, start: base as u32, end: (base + len) as u32, kids: [0, 0], leaf: true };
        if len <= LEAF {
            return Sub { nodes: vec![root] };
        }
        let mut axis = 0;
        for k in 1..3 {
            if chi[k] - clo[k] > chi[axis] - clo[axis] {
                axis = k;
            }
        }
        let mid = len / 2;
        order.select_nth_unstable_by(mid, |&x, &y| {
            cen[x as usize][axis].partial_cmp(&cen[y as usize][axis]).unwrap_or(std::cmp::Ordering::Equal)
        });
        let (left, right) = order.split_at_mut(mid);
        let (l, r) = if len >= PAR_MIN {
            rayon::join(
                || Self::build(left, base, tlo, thi, cen),
                || Self::build(right, base + mid, tlo, thi, cen),
            )
        } else {
            (Self::build(left, base, tlo, thi, cen), Self::build(right, base + mid, tlo, thi, cen))
        };
        root.leaf = false;
        let off_l = 1u32;
        let off_r = 1 + l.nodes.len() as u32;
        root.kids = [off_l, off_r];
        let mut nodes = Vec::with_capacity(1 + l.nodes.len() + r.nodes.len());
        nodes.push(root);
        for (sub, off) in [(l, off_l), (r, off_r)] {
            for mut nd in sub.nodes {
                if !nd.leaf {
                    nd.kids = [nd.kids[0] + off, nd.kids[1] + off];
                }
                nodes.push(nd);
            }
        }
        Sub { nodes }
    }

    #[inline]
    fn hits_box(n: &Node, o: [f64; 3], inv: [f64; 3]) -> bool {
        let mut tmin = f64::NEG_INFINITY;
        let mut tmax = f64::INFINITY;
        for k in 0..3 {
            let t1 = (n.lo[k] - o[k]) * inv[k];
            let t2 = (n.hi[k] - o[k]) * inv[k];
            let (a, b) = if t1 < t2 { (t1, t2) } else { (t2, t1) };
            if a.is_nan() || b.is_nan() {
                continue;
            }
            tmin = tmin.max(a);
            tmax = tmax.min(b);
        }
        tmax >= tmin && tmax >= 0.0
    }

    /// `parity::Tris::hit`, operation for operation.
    #[inline]
    fn hit(&self, r: usize, o: [f64; 3], d: [f64; 3]) -> bool {
        let e1 = self.e1[r];
        let e2 = self.e2[r];
        let a = self.a[r];
        let tvec = [o[0] - a[0], o[1] - a[1], o[2] - a[2]];
        let p = cross(d, e2);
        let det = dot(e1, p);
        if !(det.abs() >= EPS) {
            return false;
        }
        let inv = 1.0 / det;
        let u = dot(tvec, p) * inv;
        if !(u >= -EPS && u <= 1.0 + EPS) {
            return false;
        }
        let q = cross(tvec, e1);
        let v = dot(q, d) * inv;
        if !(v >= -EPS && u + v <= 1.0 + EPS) {
            return false;
        }
        let t = dot(e2, q) * inv;
        t > T_MIN
    }

    /// Crossings ahead of `o` along each of `dirs` (at most 32), counting
    /// the triangles whose face `keep` accepts. One walk for all the rays:
    /// a ray tests a triangle exactly when every box above it holds the
    /// ray, as its own walk would.
    pub fn crossings(&self, o: [f64; 3], dirs: &[[f64; 3]], keep: impl Fn(u32) -> bool, out: &mut [u32]) {
        let nr = dirs.len();
        for c in out.iter_mut() {
            *c = 0;
        }
        if self.nodes.is_empty() || nr == 0 {
            return;
        }
        let mut inv = [[0.0f64; 3]; 32];
        for (k, d) in dirs.iter().enumerate() {
            for j in 0..3 {
                let dv = if d[j].abs() < 1e-30 { 1e-30 } else { d[j] };
                inv[k][j] = 1.0 / dv;
            }
        }
        let all: u32 = if nr >= 32 { u32::MAX } else { (1u32 << nr) - 1 };
        let mut stack: Vec<(u32, u32)> = Vec::with_capacity(64);
        stack.push((0, all));
        while let Some((ni, live)) = stack.pop() {
            let n = &self.nodes[ni as usize];
            // In the Möller–Trumbore test the distance numerator is
            // exactly zero for an origin on this axis-aligned plane.
            if n.plane.is_some_and(|(k, x)| o[k] == x) { continue; }
            let mut m = 0u32;
            let mut bits = live;
            while bits != 0 {
                let k = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if Self::hits_box(n, o, inv[k]) {
                    m |= 1 << k;
                }
            }
            if m == 0 {
                continue;
            }
            if !n.leaf {
                stack.push((n.kids[1], m));
                stack.push((n.kids[0], m));
                continue;
            }
            for &r in &self.order[n.start as usize..n.end as usize] {
                let r = r as usize;
                if !keep(self.face[r]) {
                    continue;
                }
                let mut bits = m;
                while bits != 0 {
                    let k = bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    if self.hit(r, o, dirs[k]) {
                        out[k] += 1;
                    }
                }
            }
        }
    }
}

// ---- orient_outward ------------------------------------------------------------

/// `_face_side_state`'s answer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Undecided,
    Outward,
    Inward,
    Interior,
}

struct Ctx<'a> {
    g: &'a Geom,
    /// Per face: its component's ray index.
    comp: &'a [u32],
    index: &'a HashMap<u32, RayIndex>,
    /// Per face: the probe and normal (`None`: degenerate, no query).
    probe: &'a [V3],
    deg: &'a [bool],
    certain: &'a [bool],
    rays: usize,
}

impl<'a> Ctx<'a> {
    /// The rays of one `ray_parity_outside(origin, dir)` call, jittered by
    /// `draws` (2 per jittered ray: the u then the v coefficient).
    fn rays_of(&self, dir: V3, draws: &[f64], out: &mut [[f64; 3]]) {
        let h = self.g.h;
        let n = facegeom::normalized(dir, h);
        // _basis(n)
        let mut r = V3(1.0, 0.0, 0.0);
        if (r.dot(n, h) as f64).abs() > 0.9 {
            r = V3(0.0, 1.0, 0.0);
        }
        let u = facegeom::normalized(n.cross(r, h), h);
        let v = facegeom::normalized(n.cross(u, h), h);
        out[0] = f64s(n);
        for k in 1..self.rays {
            let a = draws[2 * (k - 1)];
            let b = draws[2 * (k - 1) + 1];
            out[k] = f64s(facegeom::normalized(add(add(n, mul(u, a)), mul(v, b)), h));
        }
    }

    /// Votes of one query: Some(outside) (an odd number of rays: no tie).
    /// `known0`: the unjittered ray's count, already made by `guess`.
    fn vote(&self, f: u32, dir: V3, draws: &[f64], boundary: &[bool], known0: Option<u32>) -> Option<bool> {
        let mut dirs = [[0.0f64; 3]; 32];
        let r = self.rays;
        self.rays_of(dir, draws, &mut dirs[..r]);
        let mut counts = [0u32; 32];
        let o = f64s(self.probe[f as usize]);
        if let Some(ix) = self.index.get(&self.comp[f as usize]) {
            match known0 {
                Some(c0) => {
                    ix.crossings(o, &dirs[1..r], |g| g != f && boundary[g as usize], &mut counts[1..r]);
                    counts[0] = c0;
                }
                None => ix.crossings(o, &dirs[..r], |g| g != f && boundary[g as usize], &mut counts[..r]),
            }
        }
        let outside = counts[..r].iter().filter(|&&c| c % 2 == 0).count();
        let inside = r - outside;
        if outside == inside {
            None
        } else {
            Some(outside > inside)
        }
    }

    /// The unjittered ray alone: its crossing count (even: the side ahead
    /// reads outside).
    fn guess(&self, f: u32, dir: V3, boundary: &[bool]) -> u32 {
        let h = self.g.h;
        let d = [f64s(facegeom::normalized(dir, h))];
        let mut c = [0u32; 1];
        let o = f64s(self.probe[f as usize]);
        if let Some(ix) = self.index.get(&self.comp[f as usize]) {
            ix.crossings(o, &d, |g| g != f && boundary[g as usize], &mut c);
        }
        c[0]
    }

    /// `_face_side_state` with its queries drawing from `stream` at
    /// `off`; returns the side and the draws it made.
    fn side(&self, f: u32, stream: &[f64], off: usize, boundary: &[bool], known0: Option<u32>) -> (Side, usize) {
        if self.deg[f as usize] {
            return (Side::Undecided, 0);
        }
        let per = 2 * (self.rays - 1);
        let n = self.g.normal[f as usize];
        let ahead = self.vote(f, n, &stream[off..off + per], boundary, known0);
        match ahead {
            Some(true) => (Side::Outward, per),
            None => (Side::Undecided, per),
            Some(false) => {
                let behind = self.vote(f, neg(n), &stream[off + per..off + 2 * per], boundary, None);
                let s = match behind {
                    Some(true) => Side::Inward,
                    Some(false) => Side::Interior,
                    None => Side::Undecided,
                };
                (s, 2 * per)
            }
        }
    }

    /// One pass of `_face_side_state` over `list` (in order) with a fresh
    /// `random.Random(seed)`: exactly the sequential loop's answers.
    fn pass(&self, list: &[u32], boundary: &[bool], seed: u64, jitter: f64) -> Vec<Side> {
        let per = 2 * (self.rays - 1);
        let n = list.len();
        if !self.certain.is_empty() {
            let mut rng = PyRandom::new(seed);
            let mut skip = 0;
            let mut consumed = 0;
            return list.iter().map(|&face| {
                let f = face as usize;
                if self.deg[f] { return Side::Undecided; }
                if self.certain[f] {
                    skip += per;
                    return Side::Outward;
                }
                skip_sheet_draws(&mut rng, seed, consumed, skip);
                consumed += skip;
                skip = 0;
                let mut ask = |dir| {
                    let mut draws = [0.; 62];
                    for draw in &mut draws[..per] {
                        *draw = -jitter + (jitter - -jitter) * rng.random();
                    }
                    consumed += per;
                    self.vote(face, dir, &draws[..per], boundary, None)
                };
                match ask(self.g.normal[f]) {
                    Some(true) => Side::Outward,
                    None => Side::Undecided,
                    Some(false) => match ask(neg(self.g.normal[f])) {
                        Some(true) => Side::Inward,
                        Some(false) => Side::Interior,
                        None => Side::Undecided,
                    },
                }
            }).collect();
        }
        // Predicted draws: the unjittered ray's reading of the side ahead.
        let c0: Vec<u32> = list
            .par_iter()
            .map(|&f| if self.deg[f as usize] { 0 } else { self.guess(f, self.g.normal[f as usize], boundary) })
            .collect();
        let mut pstart = Vec::with_capacity(n + 1);
        pstart.push(0usize);
        for (k, &f) in list.iter().enumerate() {
            let p = if self.deg[f as usize] {
                0
            } else if c0[k] % 2 == 0 {
                per
            } else {
                2 * per
            };
            pstart.push(pstart.last().unwrap() + p);
        }
        let mut stream = Stream::new(seed, jitter);
        let mut out = vec![Side::Undecided; n];
        let mut cur = 0usize;
        const CHUNK: usize = 4096;
        let mut s = 0usize;
        while s < n {
            let e = (s + CHUNK).min(n);
            // Judge [i, e) assuming face i starts at `cur` and the rest
            // draw as predicted.
            let mut i = s;
            while i < e {
                let shift = cur as isize - pstart[i] as isize;
                let need = (pstart[e] as isize + shift) as usize + 2 * per;
                stream.ensure(need);
                let vals = &stream.vals;
                let judged: Vec<(Side, usize, usize)> = (i..e)
                    .into_par_iter()
                    .map(|k| {
                        let off = (pstart[k] as isize + shift) as usize;
                        let (sd, d) = self.side(list[k], vals, off, boundary, Some(c0[k]));
                        (sd, d, off)
                    })
                    .collect();
                let mut redo = None;
                for (j, (sd, d, off)) in judged.into_iter().enumerate() {
                    let k = i + j;
                    if off != cur {
                        redo = Some(k);
                        break;
                    }
                    out[k] = sd;
                    cur += d;
                }
                match redo {
                    Some(k) => i = k,
                    None => i = e,
                }
            }
            s = e;
        }
        out
    }
}

/// `_incoherent_edges(faces)` over `faces`, the ones in `flipped` walked
/// reversed.
fn incoherent(scan: &MeshScan, faces: &[u32], flipped: &[bool]) -> usize {
    let mut uses: HashMap<(u32, u32), (u32, (u32, u32))> = HashMap::new();
    for &f in faces {
        let rev = flipped[f as usize];
        for ring in scan.rings_of(f as usize) {
            let n = ring.len();
            for i in 0..n {
                let (mut a, mut b) = (ring[i], ring[(i + 1) % n]);
                if rev {
                    std::mem::swap(&mut a, &mut b);
                }
                let k = (a.min(b), a.max(b));
                let e = uses.entry(k).or_insert((0, (a, b)));
                e.0 += 1;
                if e.0 == 2 {
                    // the second use: remember whether it repeats the first
                    if e.1 != (a, b) {
                        e.1 = (u32::MAX, u32::MAX);
                    }
                }
            }
        }
    }
    uses.values().filter(|(c, d)| *c == 2 && d.0 != u32::MAX).count()
}

/// Axis-aligned probes whose jitter cones can reach nonplanar geometry.
type Slab = (usize, [f64; 3], [f64; 3]);
fn sheet_queries(g: &Geom, scope: &[u32], probes: &[V3], jitter: f64) -> Option<(Vec<bool>, Vec<Slab>)> {
    let seed = *scope.first()? as usize;
    let [a, b, c] = *g.tris_of(seed).first()?;
    let coord = |v: u32| f64s(g.pos[v as usize]);
    let axis = (0..3).find(|&k| coord(a)[k] == coord(b)[k] && coord(a)[k] == coord(c)[k])?;
    let plane = coord(a)[axis];
    let mut window = SheetWindow {
        axis,
        plane,
        lo: [f64::INFINITY; 3],
        hi: [f64::NEG_INFINITY; 3],
    };
    let mut magnitude = 0f64;
    for &point in &g.pos {
        for x in f64s(point) {
            if !x.is_finite() { return None; }
            magnitude = magnitude.max(x.abs());
        }
    }
    for &f in scope {
        for triangle in g.tris_of(f as usize) {
            if triangle.iter().all(|&v| coord(v)[axis] == plane) { continue; }
            for p in triangle.map(coord) {
                for k in 0..3 {
                    window.lo[k] = window.lo[k].min(p[k]);
                    window.hi[k] = window.hi[k].max(p[k]);
                }
            }
        }
    }
    (window.lo, window.hi) = grown(window.lo, window.hi);
    let mut certain = vec![false; g.normal.len()];
    let mut slabs = [([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]); 3];
    let mut uncertain = 0;
    for &f in scope {
        let f = f as usize;
        if window.misses(probes[f], g.normal[f], jitter) {
            certain[f] = true;
            continue;
        }
        uncertain += 1;
        if uncertain > scope.len() / 32 {
            return None;
        }
        let n = f64s(g.normal[f]);
        let k = (0..3).find(|&k| n[k].abs() == 1. && n[(k + 1) % 3] == 0. && n[(k + 2) % 3] == 0.)?;
        let p = f64s(probes[f]);
        let spread = (p[k].abs() + magnitude + 1.) * (2. * jitter.abs() + 1e-12) + 1e-4 * (1. + magnitude);
        for j in 0..3 {
            slabs[k].0[j] = slabs[k].0[j].min(p[j] - spread);
            slabs[k].1[j] = slabs[k].1[j].max(p[j] + spread);
        }
    }
    let slabs = slabs
        .into_iter()
        .enumerate()
        .filter(|(_, (lo, hi))| lo[0] <= hi[0])
        .map(|(k, (lo, hi))| (k, lo, hi))
        .collect();
    Some((certain, slabs))
}

/// What `orient_outward` decides: the faces whose `interior` mark changes
/// (with the new value) and the faces to turn over, in list order.
pub struct Decision {
    pub marks: Vec<(u32, bool)>,
    pub flips: Vec<u32>,
}

pub struct Params {
    pub seed: u64,
    pub rays: usize,
    pub jitter: f64,
}

pub(crate) fn component_root(p: &mut [u32], mut x: u32) -> u32 {
    while p[x as usize] != x {
        let gp = p[p[x as usize] as usize];
        p[x as usize] = gp;
        x = gp;
    }
    x
}

/// `built_core.orient.orient_outward(mesh, seed, only)` on a scan whose parts
/// RINGS, EFACES and FFLAGS are read and whose `geom` is built.
pub fn orient(scan: &MeshScan, g: &Geom, only: Option<&[u32]>, prm: &Params) -> Decision {
    orient_with_probes(scan,g,only,prm,None)
}

pub(crate) fn orient_with_probes(scan:&MeshScan,g:&Geom,only:Option<&[u32]>,prm:&Params,cached:Option<Vec<V3>>)->Decision{
    let nf = scan.n_faces();
    let h = g.h;
    let mut t = std::time::Instant::now();
    let mut dec = Decision { marks: Vec::new(), flips: Vec::new() };
    if nf == 0 {
        return dec;
    }
    // _all_coplanar
    {
        let n0 = g.normal[0];
        let o = g.centroid[0];
        let flat = (0..nf).all(|f| {
            if (g.normal[f].dot(n0, h) as f64).abs() < 0.999 {
                return false;
            }
            !((sub(g.centroid[f], o).dot(n0, h) as f64).abs() > 1e-4)
        });
        if flat {
            for f in 0..nf {
                if scan.finterior[f] {
                    dec.marks.push((f as u32, false));
                }
            }
            return dec;
        }
    }
    // _face_components: union-find through shared edges.
    let mut parent: Vec<u32> = (0..nf as u32).collect();
    for e in 0..scan.n_edges() {
        let fs = scan.faces_of_edge(e);
        let mut it = fs.iter().copied().filter(|&f| f != NONE && (f as usize) < nf);
        let Some(first) = it.next() else { continue };
        for j in it {
            let ra = component_root(&mut parent, first);
            let rb = component_root(&mut parent, j);
            if ra != rb {
                parent[ra as usize] = rb;
            }
        }
    }
    let comp: Vec<u32> = (0..nf as u32).map(|f| component_root(&mut parent, f)).collect();
    lap("coplanar + components", &mut t);
    let mut scope: Vec<u32> = match only {
        None => (0..nf as u32).collect(),
        Some(o) => {
            let mut live = vec![false; nf];
            let mut any = false;
            for &f in o {
                if (f as usize) < nf {
                    live[comp[f as usize] as usize] = true;
                    any = true;
                }
            }
            if !any {
                return dec;
            }
            (0..nf as u32).filter(|&f| live[comp[f as usize] as usize]).collect()
        }
    };
    let mut in_scope_comp = vec![false; nf];
    for &f in &scope {
        in_scope_comp[comp[f as usize] as usize] = true;
    }
    // The ray index of each component in scope: its triangles in the
    // packed order (faces in list order, each face's triangles in order).
    let mut rows: HashMap<u32, Vec<u32>> = HashMap::new();
    for f in 0..nf as u32 {
        let c = comp[f as usize];
        if in_scope_comp[c as usize] && g.tstart[f as usize + 1] > g.tstart[f as usize] {
            rows.entry(c).or_default().push(f);
        }
    }
    // Per face of the scope: probe and degeneracy (`normal().length()`).
    let probe = cached.unwrap_or_else(|| {
        let mut probe = vec![V3(0., 0., 0.); nf];
        let active: Vec<_> = scope
            .par_iter()
            .map(|&f| {
                let f = f as usize;
                if facegeom::length(g.normal[f], h) < 1e-9 {
                    V3(0., 0., 0.)
                } else {
                    interior_probe(g, f, &scan.rings_of(f))
                }
            })
            .collect();
        for (&f, p) in scope.iter().zip(active) {
            probe[f as usize] = p;
        }
        probe
    });
    let deg = g
        .normal
        .iter()
        .map(|&n| facegeom::length(n, h) < 1e-9)
        .collect::<Vec<_>>();
    // Distant probes on a common sheet cannot hit any off-plane triangle.
    // Keep their positions in the jitter stream. Only the other probes
    // need triangles in the crossing index.
    let windows = if rows.len() == 1 && scope.len() >= 8192 {
        sheet_queries(g, &scope, &probe, prm.jitter)
    } else {
        None
    };
    let index: HashMap<u32, RayIndex> = rows
        .into_par_iter()
        .map(|(c, faces)| {
            let mut a = Vec::new();
            let mut e1 = Vec::new();
            let mut e2 = Vec::new();
            let mut fid = Vec::new();
            for f in faces {
                for t in g.tris_of(f as usize) {
                    let pa = f64s(g.pos[t[0] as usize]);
                    let pb = f64s(g.pos[t[1] as usize]);
                    let pc = f64s(g.pos[t[2] as usize]);
                    if let Some((_, slabs)) = &windows {
                        if !slabs.iter().any(|&(axis, lo, hi)| {
                            (0..3).filter(|&k| k != axis).all(|k| {
                                pa[k].min(pb[k]).min(pc[k]) <= hi[k] && pa[k].max(pb[k]).max(pc[k]) >= lo[k]
                            })
                        }) {
                            continue;
                        }
                    }
                    a.push(pa);
                    e1.push([pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]]);
                    e2.push([pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]]);
                    fid.push(f);
                }
            }
            (c, RayIndex::new(a, e1, e2, fid))
        })
        .collect();
    lap("ray index", &mut t);
    let mut certain = windows.map(|(mask, _)| mask).unwrap_or_default();
    if certain.is_empty() && index.values().any(|ix| ix.sheet.is_some()) {
        certain.resize(nf, false);
        for &face in &scope {
            let f = face as usize;
            certain[f] = index
                .get(&comp[f])
                .and_then(|ix| ix.sheet.as_ref())
                .is_some_and(|s| s.misses(probe[f], g.normal[f], prm.jitter));
        }
        if certain.iter().filter(|c| **c).count() < scope.len() - scope.len() / 32 {
            certain.clear();
        }
    }
    let ctx = Ctx { g, comp: &comp, index: &index, probe: &probe, deg: &deg, certain: &certain, rays: prm.rays };
    lap("probes", &mut t);
    // The interior rounds.
    let mut boundary = vec![true; nf];
    // The last round's answers while they still hold for the final
    // boundary (the loop stopped on them).
    let mut settled: Option<(Vec<u32>, Vec<Side>)> = None;
    for _ in 0..4 {
        let sides = ctx.pass(&scope, &boundary, prm.seed, prm.jitter);
        lap("round", &mut t);
        let mut interior_now = vec![false; nf];
        for (k, &f) in scope.iter().enumerate() {
            if sides[k] == Side::Interior {
                interior_now[f as usize] = true;
            }
        }
        let same = scope.iter().all(|&f| interior_now[f as usize] == !boundary[f as usize]);
        if same {
            settled = Some((scope.clone(), sides));
            break;
        }
        for f in 0..nf {
            boundary[f] = !interior_now[f];
        }
    }
    for &f in &scope {
        let v = !boundary[f as usize];
        if scan.finterior[f as usize] != v {
            dec.marks.push((f, v));
        }
    }
    // Closed / open.
    let enf = scan.edge_face_counts();
    let mesh_closed = enf.iter().all(|&c| c >= 2);
    if !mesh_closed {
        if only.is_none() {
            return dec;
        }
        let mut open = vec![false; nf];
        for e in 0..scan.n_edges() {
            if enf[e] == 1 {
                for &f in scan.faces_of_edge(e) {
                    if f != NONE && (f as usize) < nf {
                        open[comp[f as usize] as usize] = true;
                    }
                }
            }
        }
        scope.retain(|&f| !open[comp[f as usize] as usize]);
        if scope.is_empty() {
            return dec;
        }
    }
    let queried: Vec<u32> = scope.iter().copied().filter(|&f| boundary[f as usize]).collect();
    // The flip pass asks the faces of the boundary in list order with a
    // fresh stream and the final boundary's mask: when those are exactly
    // the faces the settling round asked (no partition, nothing open
    // dropped), its answers are this pass's, query for query.
    let sides = match settled {
        Some((list, sides)) if list == queried => sides,
        _ => ctx.pass(&queried, &boundary, prm.seed, prm.jitter),
    };
    lap("flip pass", &mut t);
    let mut to_flip: Vec<u32> = queried
        .iter()
        .zip(sides.iter())
        .filter(|(_, &s)| s == Side::Inward)
        .map(|(&f, _)| f)
        .collect();
    if !mesh_closed && !to_flip.is_empty() {
        let mut flip_comp = vec![false; nf];
        for &f in &to_flip {
            flip_comp[comp[f as usize] as usize] = true;
        }
        let mut members: HashMap<u32, Vec<u32>> = HashMap::new();
        for &f in &scope {
            let c = comp[f as usize];
            if flip_comp[c as usize] {
                members.entry(c).or_default().push(f);
            }
        }
        let none = vec![false; nf];
        let mut flipped = vec![false; nf];
        for &f in &to_flip {
            flipped[f as usize] = true;
        }
        let mut worse = vec![false; nf];
        for (c, fs) in &members {
            if incoherent(scan, fs, &flipped) > incoherent(scan, fs, &none) {
                worse[*c as usize] = true;
            }
        }
        to_flip.retain(|&f| !worse[comp[f as usize] as usize]);
    }
    dec.flips = to_flip;
    dec
}

// ---- Python entry points -----------------------------------------------------

fn hypot_named(name: &str) -> PyResult<Hypot> {
    match name {
        "libstdc++" => Ok(Hypot::LibStdCxx),
        "msvc" => Ok(Hypot::Msvc),
        "libc++-fma" => Ok(Hypot::LibCxxFma),
        _ => Err(pyo3::exceptions::PyValueError::new_err(format!("unknown hypot {name:?}"))),
    }
}

/// The scan's face geometry, built once per scan (GIL released).
pub fn geom_of(py: Python<'_>, scan: &mut MeshScan, hypot: &str, compensated: bool) -> PyResult<Arc<Geom>> {
    let h = hypot_named(hypot)?;
    scan.need_parts(py, RINGS)?;
    if let Some(g) = &scan.geom {
        if g.h == h {
            return Ok(g.clone());
        }
    }
    let s: &MeshScan = scan;
    let g = Arc::new(py.detach(|| Geom::of(s, h, compensated)));
    scan.geom = Some(g.clone());
    Ok(g)
}

/// Face indices of `faces` (views or objects) in the scan, `NONE` for one
/// it does not hold.
pub fn face_indices(py: Python<'_>, scan: &mut MeshScan, faces: &Bound<'_, PyList>) -> PyResult<Vec<u32>> {
    let mut out = Vec::with_capacity(faces.len());
    if scan.store_backed() {
        let key_attr = PyString::intern(py, "key");
        let table = scan.face_slot_table();
        let ns = scan.namespace();
        for f in faces.iter() {
            let key: u128 = match f.getattr(&key_attr).and_then(|k| k.extract::<u128>()) {
                Ok(k) => k,
                Err(_) => {
                    out.push(NONE);
                    continue;
                }
            };
            let idx = match split_key(key) {
                Some((kns, Kind::Face, gen, slot)) if kns == ns => table(slot, gen),
                _ => NONE,
            };
            out.push(idx);
        }
    } else {
        let map = scan.face_pointer_map(py)?;
        for f in faces.iter() {
            out.push(map.get(&(f.as_ptr() as usize)).copied().unwrap_or(NONE));
        }
    }
    Ok(out)
}

/// Indices of `faces` in the scan's face list (None: not there).
#[pyfunction]
pub fn scan_face_indices(py: Python<'_>, mut scan: PyRefMut<'_, MeshScan>, faces: Bound<'_, PyList>) -> PyResult<Vec<Option<u32>>> {
    scan.need_parts(py, 0)?;
    Ok(face_indices(py, &mut scan, &faces)?.into_iter().map(|f| if f == NONE { None } else { Some(f) }).collect())
}

/// `orient_outward(mesh, seed, only)`'s decision: ``(marks, flips)`` —
/// the faces whose interior mark changes, with the new value, and the
/// faces to turn over, all as indices of the scan's face list.
#[pyfunction]
#[pyo3(signature = (scan, only, seed, rays, jitter, hypot, compensated))]
#[allow(clippy::too_many_arguments)]
pub fn orient_decision(
    py: Python<'_>,
    mut scan: PyRefMut<'_, MeshScan>,
    only: Option<Bound<'_, PyList>>,
    seed: u64,
    rays: usize,
    jitter: f64,
    hypot: &str,
    compensated: bool,
) -> PyResult<(Vec<(u32, bool)>, Vec<u32>)> {
    if !(1..=32).contains(&rays) || rays % 2 == 0 {
        return Err(pyo3::exceptions::PyValueError::new_err("rays: an odd count up to 31"));
    }
    let scan: &mut MeshScan = &mut scan;
    let mut t = std::time::Instant::now();
    scan.need_parts(py, RINGS | EFACES | FFLAGS)?;
    lap("parts", &mut t);
    let g = geom_of(py, scan, hypot, compensated)?;
    lap("geometry", &mut t);
    let only_idx: Option<Vec<u32>> = match only {
        None => None,
        Some(l) => Some(face_indices(py, scan, &l)?.into_iter().filter(|&f| f != NONE).collect()),
    };
    let prm = Params { seed, rays, jitter };
    let s: &MeshScan = scan;
    let dec = py.detach(|| orient(s, &g, only_idx.as_deref(), &prm));
    Ok((dec.marks, dec.flips))
}

/// `Face.normal()`, `centroid()` and `triangulate()` (corner positions)
/// of every face, from the scan's geometry (for the differential tests).
#[pyfunction]
#[allow(clippy::type_complexity)]
pub fn scan_face_geometry(
    py: Python<'_>,
    mut scan: PyRefMut<'_, MeshScan>,
    hypot: &str,
    compensated: bool,
) -> PyResult<(Vec<(f64, f64, f64)>, Vec<(f64, f64, f64)>, Vec<Vec<[(f64, f64, f64); 3]>>)> {
    let g = geom_of(py, &mut scan, hypot, compensated)?;
    let nf = g.normal.len();
    let p = |i: u32| {
        let v = g.pos[i as usize];
        (v.0, v.1, v.2)
    };
    Ok((
        g.normal.iter().map(|v| (v.0, v.1, v.2)).collect(),
        g.centroid.iter().map(|v| (v.0, v.1, v.2)).collect(),
        (0..nf).map(|f| g.tris_of(f).iter().map(|t| [p(t[0]), p(t[1]), p(t[2])]).collect()).collect(),
    ))
}

/// The probes `_interior_probe` picks, per face (differential tests).
#[pyfunction]
pub fn scan_face_probes(
    py: Python<'_>,
    mut scan: PyRefMut<'_, MeshScan>,
    hypot: &str,
    compensated: bool,
) -> PyResult<Vec<(f64, f64, f64)>> {
    let g = geom_of(py, &mut scan, hypot, compensated)?;
    let s: &MeshScan = &scan;
    Ok((0..g.normal.len())
        .map(|f| {
            let p = interior_probe(&g, f, &s.rings_of(f));
            (p.0, p.1, p.2)
        })
        .collect())
}

/// `random.Random(seed).random()`, `n` times (tests).
#[pyfunction]
pub fn py_random_stream(seed: u64, n: usize) -> Vec<f64> {
    let mut r = PyRandom::new(seed);
    (0..n).map(|_| r.random()).collect()
}

#[cfg(test)]
mod sheet_votes {
    use super::*;

    #[test]
    fn jumping_past_distant_probes_preserves_each_next_draw() {
        for seed in [12345, 771, 12345] {
            let mut reference = PyRandom::new(seed);
            let mut cached = PyRandom::new(seed);
            let mut used = 0;
            for step in [13, 140001, 11, 65536, 72145] {
                reference.skip_draws(step);
                skip_sheet_draws(&mut cached, seed, used, step);
                assert_eq!(reference.random(), cached.random());
                used += step + 1;
            }
        }
    }

    #[test]
    fn skipping_proven_sheet_votes_keeps_the_jitter_stream() {
        let mut a = Vec::new();
        for y in 0..64 {
            for x in 0..64 { a.push([x as f64, y as f64, 0.]); }
        }
        // Small sheets on opposite sides, with probes beside triangle edges
        // where the unjittered ray and the majority can disagree.
        for z in [-1., 1.] { a.push([32., 31., z]); }
        let n = a.len();
        let probe: Vec<_> = a.iter().map(|p| V3(p[0] + 0.5, p[1] + 0.49999, p[2])).collect();
        let faces: Vec<_> = (0..n as u32).collect();
        let normal = V3(0., 0., 1.);
        let ix = RayIndex::new(a.clone(), vec![[1.,0.,0.]; n], vec![[0.,1.,0.]; n], faces.clone());
        let window = ix.sheet.as_ref().unwrap();
        let certain: Vec<_> = probe.iter().map(|&p| window.misses(p, normal, 3e-4)).collect();
        assert!(certain.iter().filter(|v| **v).count() > n * 9 / 10);
        let index = HashMap::from([(0, ix)]);
        let pos = a
            .iter()
            .flat_map(|&[x, y, z]| [V3(x, y, z), V3(x + 1., y, z), V3(x, y + 1., z)])
            .collect();
        let geom = Geom {
            h: Hypot::LibStdCxx,
            pos,
            normal: vec![normal; n],
            centroid: vec![],
            tstart: (0..=n as u32).collect(),
            tris: faces.iter().map(|&i| [3 * i, 3 * i + 1, 3 * i + 2]).collect(),
        };
        let (mask, slabs) = sheet_queries(&geom, &faces, &probe, 3e-4).unwrap();
        let candidates: Vec<_> = faces
            .iter()
            .copied()
            .filter(|&f| {
                let [x, y, _] = a[f as usize];
                slabs
                    .iter()
                    .any(|&(axis, lo, hi)| axis == 2 && x <= hi[0] && x + 1. >= lo[0] && y <= hi[1] && y + 1. >= lo[1])
            })
            .collect();
        assert!(candidates.len() < n / 32);
        let count = candidates.len();
        let origins = candidates.iter().map(|&f| a[f as usize]).collect();
        let pruned = HashMap::from([(
            0,
            RayIndex::new(
                origins,
                vec![[1., 0., 0.]; count],
                vec![[0., 1., 0.]; count],
                candidates,
            ),
        )]);
        let (comp, deg, boundary) = (vec![0; n], vec![false; n], vec![true; n]);
        let mut ctx = Ctx { g: &geom, comp: &comp, index: &index, probe: &probe, deg: &deg,
            certain: &[], rays: 7 };
        for seed in [12345, 771] {
            ctx.index = &index;
            ctx.certain = &[];
            let full = ctx.pass(&faces, &boundary, seed, 3e-4);
            ctx.certain = &certain;
            assert_eq!(ctx.pass(&faces, &boundary, seed, 3e-4), full);
            ctx.index = &pruned;
            ctx.certain = &mask;
            assert_eq!(ctx.pass(&faces, &boundary, seed, 3e-4), full);
        }
    }
}
