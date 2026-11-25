// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! The line drawing of a sheet frame.
//!
//! What `core/hlr.py`'s `hlr_drawing` did after it had the camera-space
//! edges: every edge's visible spans and its hidden remainder, the section
//! cap that takes what lies inside the material, the zero-length cull, the
//! Profiles probes, then the collinear merge and the hidden-under-visible
//! drop. One call, the same arithmetic in the same order, so the drawing is
//! the one the Python pass drew (pinned by tests/test_hlr_parity.py).

use crate::hlr::Occluders;
use std::collections::HashMap;

/// built_core.hlr._T_EPS.
const T_EPS: f64 = 1e-6;

pub const KIND_EDGE: i8 = 0;
pub const KIND_PROFILE: i8 = 1;
pub const KIND_CUT: i8 = 2;
pub const KIND_HIDDEN: i8 = 3;

/// The section cap: the stacked ring edges `p → q` (camera plane) and the
/// cut plane's depth `abg` as an affine function of the camera plane.
pub struct Cap {
    pub p: Vec<[f64; 2]>,
    pub q: Vec<[f64; 2]>,
    pub abg: [f64; 3],
}

pub struct Input<'a> {
    /// Edge endpoints in camera space (x right, y up, depth away).
    pub a: &'a [[f64; 3]],
    pub b: &'a [[f64; 3]],
    /// The same edges in world space.
    pub world: &'a [[[f64; 3]; 2]],
    /// Rows `[0, n_hard)` are hard edges, the next `n_sil` soft
    /// silhouettes, the rest section-cut chords.
    pub n_hard: usize,
    pub n_sil: usize,
    /// The occluding triangles in camera space.
    pub tris: Vec<[[f64; 3]; 3]>,
    pub profiles: bool,
    pub hidden: bool,
    pub cap: Option<Cap>,
    /// A tag id per row (the layer it is inked with); rows of different
    /// tags never merge.
    pub tags: Option<&'a [i64]>,
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct Drawing {
    pub segs: Vec<[f64; 4]>,
    pub world: Vec<[[f64; 3]; 2]>,
    pub kinds: Vec<i8>,
    /// The input row each segment came from (a merged run: its seed's).
    pub rows: Vec<i64>,
}

impl Drawing {
    fn push(&mut self, s: [f64; 4], w: [[f64; 3]; 2], k: i8, r: i64) {
        self.segs.push(s);
        self.world.push(w);
        self.kinds.push(k);
        self.rows.push(r);
    }
}

#[inline]
fn py_max(a: f64, b: f64) -> f64 {
    if b > a { b } else { a }
}

#[inline]
fn py_min(a: f64, b: f64) -> f64 {
    if b < a { b } else { a }
}

/// `float(x) or 1.0`.
#[inline]
fn or_one(x: f64) -> f64 {
    if x == 0.0 { 1.0 } else { x }
}

/// The largest extent over the axes of a point set (`(max - min).max()`).
fn extent<const N: usize>(pts: impl Iterator<Item = [f64; N]>) -> f64 {
    let mut lo = [f64::INFINITY; N];
    let mut hi = [f64::NEG_INFINITY; N];
    let mut any = false;
    for p in pts {
        any = true;
        for k in 0..N {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    if !any {
        return 0.0;
    }
    (0..N).map(|k| hi[k] - lo[k]).fold(f64::NEG_INFINITY, f64::max)
}

fn interval_from_linear(c0: f64, c1: f64) -> Option<(f64, f64)> {
    if c1.abs() < 1e-15 {
        return if c0 >= 0.0 { Some((0.0, 1.0)) } else { None };
    }
    let t = -c0 / c1;
    if c1 > 0.0 { Some((t, 1.0)) } else { Some((0.0, t)) }
}

/// built_core.hlr.subtract_spans.
pub fn subtract_spans(spans: &[(f64, f64)], hidden: &[(f64, f64)]) -> Vec<(f64, f64)> {
    if hidden.is_empty() {
        return spans.to_vec();
    }
    let mut out = Vec::new();
    for &(lo, hi) in spans {
        let mut cur = lo;
        for &(c0, c1) in hidden {
            if c1 <= cur || c0 >= hi {
                continue;
            }
            if c0 - cur > T_EPS {
                out.push((cur, py_min(c0, hi)));
            }
            cur = py_max(cur, c1);
            if cur >= hi {
                break;
            }
        }
        if hi - cur > T_EPS {
            out.push((cur, hi));
        }
    }
    out
}

/// built_core.hlr._inside_cap: even-odd over the stacked ring edges.
fn inside_cap(px: f64, py: f64, cap: &Cap) -> bool {
    let mut count = 0usize;
    for (p, q) in cap.p.iter().zip(&cap.q) {
        let (ya, yb) = (p[1], q[1]);
        if (ya > py) == (yb > py) {
            continue;
        }
        let xi = p[0] + (q[0] - p[0]) * (py - ya) / (yb - ya);
        if xi > px {
            count += 1;
        }
    }
    count & 1 == 1
}

/// built_core.hlr.cap_hidden_spans.
fn cap_hidden_spans(a2: [f64; 2], b2: [f64; 2], az: f64, bz: f64, cap: &Cap, eps: f64) -> Vec<(f64, f64)> {
    let d2 = [b2[0] - a2[0], b2[1] - a2[1]];
    let abg = cap.abg;
    let behind = interval_from_linear(
        az - (abg[0] * a2[0] + abg[1] * a2[1] + abg[2]) - eps,
        (bz - az) - (abg[0] * d2[0] + abg[1] * d2[1]),
    );
    let Some(behind) = behind else { return Vec::new() };
    let lo = py_max(0.0, behind.0);
    let hi = py_min(1.0, behind.1);
    if hi - lo <= T_EPS {
        return Vec::new();
    }
    let mut cuts = Vec::new();
    for (p, q) in cap.p.iter().zip(&cap.q) {
        let e = [q[0] - p[0], q[1] - p[1]];
        let w = [p[0] - a2[0], p[1] - a2[1]];
        let denom = d2[0] * e[1] - d2[1] * e[0];
        if denom.abs() <= 1e-15 {
            continue;
        }
        let t = (w[0] * e[1] - w[1] * e[0]) / denom;
        let s = (w[0] * d2[1] - w[1] * d2[0]) / denom;
        if s >= 0.0 && s <= 1.0 && t > lo && t < hi {
            cuts.push(t);
        }
    }
    cuts.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
    let mut bounds = Vec::with_capacity(cuts.len() + 2);
    bounds.push(lo);
    bounds.extend(cuts);
    bounds.push(hi);
    let mut out: Vec<(f64, f64)> = Vec::new();
    for w in bounds.windows(2) {
        let (t0, t1) = (w[0], w[1]);
        if t1 - t0 <= T_EPS {
            continue;
        }
        let tm = (t0 + t1) * 0.5;
        if inside_cap(a2[0] + tm * d2[0], a2[1] + tm * d2[1], cap) {
            match out.last_mut() {
                Some(last) if t0 - last.1 <= T_EPS => last.1 = t1,
                _ => out.push((t0, t1)),
            }
        }
    }
    out
}

#[inline]
fn lerp2(a: [f64; 2], b: [f64; 2], t: f64) -> [f64; 2] {
    [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]
}

#[inline]
fn lerp3(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1]), a[2] + t * (b[2] - a[2])]
}

/// The whole pass: `hlr_drawing` from its camera-space edges onwards.
pub fn drawing(input: &Input<'_>) -> Drawing {
    let n = input.a.len();
    let a = input.a;
    let b = input.b;
    let has_tris = !input.tris.is_empty();
    let (eps, delta, ptol) = if has_tris {
        let zs = input.tris.iter().flat_map(|t| t.iter().map(|v| [v[2]]));
        let span = or_one(extent::<1>(zs));
        let eps = py_max(span * 1e-4, 1e-9);
        let ext = or_one(extent::<2>(input.tris.iter().flat_map(|t| t.iter().map(|v| [v[0], v[1]]))));
        let delta = py_max(ext * 1e-4, 1e-9);
        (eps, delta, eps + delta * 10.0)
    } else {
        (0.0, 0.0, 0.0)
    };
    let ext_e = or_one(extent::<2>(a.iter().chain(b.iter()).map(|p| [p[0], p[1]])));
    let zero_len = ext_e * 1e-9;
    let cap = input.cap.as_ref().filter(|c| !c.p.is_empty());
    let (cap_eps, cap_box) = match cap {
        Some(c) => {
            let span = extent::<1>(a.iter().chain(b.iter()).map(|p| [p[2]]));
            let cap_eps = py_max(py_max(span * 1e-4, eps), 1e-9);
            let mut lo = [f64::INFINITY; 2];
            let mut hi = [f64::NEG_INFINITY; 2];
            for p in &c.p {
                for k in 0..2 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            (cap_eps, (lo, hi))
        }
        None => (0.0, ([0.0; 2], [0.0; 2])),
    };
    let occ = has_tris.then(|| Occluders::new(input.tris.clone()));
    let spans_of = occ.as_ref().map(|o| {
        let edges: Vec<([f64; 3], [f64; 3])> = (0..n).map(|i| (a[i], b[i])).collect();
        o.visible_spans_all(&edges, eps)
    });
    let mut out = Drawing::default();
    let mut probes: Vec<(usize, ([f64; 2], [f64; 2], f64))> = Vec::new();
    let whole = [(0.0, 1.0)];
    for i in 0..n {
        let a2 = [a[i][0], a[i][1]];
        let b2 = [b[i][0], b[i][1]];
        let (az, bz) = (a[i][2], b[i][2]);
        let base = if i >= input.n_hard + input.n_sil {
            KIND_CUT
        } else if i >= input.n_hard {
            KIND_PROFILE
        } else {
            KIND_EDGE
        };
        let mut spans: Vec<(f64, f64)> = match &spans_of {
            Some((off, sp)) => sp[off[i] as usize..off[i + 1] as usize].to_vec(),
            None => whole.to_vec(),
        };
        let mut hid = if input.hidden && i < input.n_hard { subtract_spans(&whole, &spans) } else { Vec::new() };
        if let Some(c) = cap {
            if !spans.is_empty() || !hid.is_empty() {
                let elo = [a2[0].min(b2[0]), a2[1].min(b2[1])];
                let ehi = [a2[0].max(b2[0]), a2[1].max(b2[1])];
                let (blo, bhi) = cap_box;
                if !(ehi[0] < blo[0] || elo[0] > bhi[0] || ehi[1] < blo[1] || elo[1] > bhi[1]) {
                    let inside = cap_hidden_spans(a2, b2, az, bz, c, cap_eps);
                    spans = subtract_spans(&spans, &inside);
                    hid = subtract_spans(&hid, &inside);
                }
            }
        }
        let w0 = input.world[i][0];
        let w1 = input.world[i][1];
        for &(t0, t1) in &hid {
            if t1 - t0 < 1e-9 {
                continue;
            }
            let p = lerp2(a2, b2, t0);
            let q = lerp2(a2, b2, t1);
            if super::pymath::hypot(q[0] - p[0], q[1] - p[1]) < zero_len {
                continue;
            }
            out.push([p[0], p[1], q[0], q[1]], [lerp3(w0, w1, t0), lerp3(w0, w1, t1)], KIND_HIDDEN, i as i64);
        }
        for &(t0, t1) in &spans {
            if t1 - t0 < 1e-9 {
                continue;
            }
            let p = lerp2(a2, b2, t0);
            let q = lerp2(a2, b2, t1);
            if super::pymath::hypot(q[0] - p[0], q[1] - p[1]) < zero_len {
                continue;
            }
            if input.profiles && base == KIND_EDGE && has_tris {
                let d = [q[0] - p[0], q[1] - p[1]];
                let ln = super::pymath::hypot(d[0], d[1]);
                if ln > 1e-12 {
                    let nrm = [-d[1] / ln * delta, d[0] / ln * delta];
                    let mid = [(p[0] + q[0]) * 0.5, (p[1] + q[1]) * 0.5];
                    let tm = (t0 + t1) * 0.5;
                    let zm = az + tm * (bz - az);
                    probes.push((out.segs.len(), (mid, nrm, zm)));
                }
            }
            out.push([p[0], p[1], q[0], q[1]], [lerp3(w0, w1, t0), lerp3(w0, w1, t1)], base, i as i64);
        }
    }
    if let (Some(o), false) = (&occ, probes.is_empty()) {
        let asks: Vec<([f64; 2], [f64; 2], f64)> = probes.iter().map(|p| p.1).collect();
        for ((row, _), outline) in probes.iter().zip(o.profile_flags(&asks, delta, ptol)) {
            if outline {
                out.kinds[*row] = KIND_PROFILE;
            }
        }
    }
    let tag_of = |d: &Drawing, i: usize| input.tags.map(|t| t[d.rows[i] as usize]);
    if out.segs.len() > 1 {
        out = merge_collinear(&out, ext_e * 1e-9, &tag_of);
    }
    if input.hidden && !out.segs.is_empty() {
        out = drop_hidden_under_visible(&out, py_max(ext_e * 1e-6, 1e-9));
    }
    out
}

fn norm3(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// built_core.hlr._merge_collinear.
fn merge_collinear(d: &Drawing, tol: f64, tag_of: &dyn Fn(&Drawing, usize) -> Option<i64>) -> Drawing {
    let n = d.segs.len();
    let inv = 1.0 / tol;
    let cell = |x: f64, y: f64| ((x * inv).floor() as i64, (y * inv).floor() as i64);
    let mut cells: HashMap<(i64, i64), Vec<(usize, usize)>> = HashMap::new();
    for i in 0..n {
        for end in 0..2 {
            let (x, y) = (d.segs[i][2 * end], d.segs[i][2 * end + 1]);
            cells.entry(cell(x, y)).or_default().push((i, end));
        }
    }
    let mut used = vec![false; n];
    let mut out = Drawing::default();
    let follow = |i: usize, end: usize, wdir: [f64; 3], used: &mut Vec<bool>| -> ([f64; 2], [f64; 3]) {
        let mut p2 = [d.segs[i][2 * end], d.segs[i][2 * end + 1]];
        let mut pw = d.world[i][end];
        loop {
            let (cx, cy) = cell(p2[0], p2[1]);
            let mut next = None;
            'search: for dx in -1..=1 {
                for dy in -1..=1 {
                    let Some(list) = cells.get(&(cx + dx, cy + dy)) else { continue };
                    for &(j, jend) in list {
                        if j == i || used[j] || d.kinds[j] != d.kinds[i] {
                            continue;
                        }
                        if tag_of(d, j) != tag_of(d, i) {
                            continue;
                        }
                        let q2 = [d.segs[j][2 * jend], d.segs[j][2 * jend + 1]];
                        if (q2[0] - p2[0]).abs() > tol || (q2[1] - p2[1]).abs() > tol {
                            continue;
                        }
                        let wj = d.world[j][jend];
                        let gap = (0..3).map(|k| (wj[k] - pw[k]).abs()).fold(f64::NEG_INFINITY, f64::max);
                        if gap > tol {
                            continue;
                        }
                        let far = d.world[j][1 - jend];
                        let other = [far[0] - wj[0], far[1] - wj[1], far[2] - wj[2]];
                        let ln = norm3(other);
                        if ln < 1e-15 {
                            continue;
                        }
                        let unit = [other[0] / ln, other[1] / ln, other[2] / ln];
                        let cr = cross(wdir, unit);
                        if dot(cr, cr) > 1e-14 || dot(wdir, other) <= 0.0 {
                            continue;
                        }
                        next = Some((j, jend));
                        break 'search;
                    }
                }
            }
            let Some((j, jend)) = next else { return (p2, pw) };
            used[j] = true;
            p2 = [d.segs[j][2 * (1 - jend)], d.segs[j][2 * (1 - jend) + 1]];
            pw = d.world[j][1 - jend];
        }
    };
    for i in 0..n {
        if used[i] {
            continue;
        }
        used[i] = true;
        let w = d.world[i];
        let dv = [w[1][0] - w[0][0], w[1][1] - w[0][1], w[1][2] - w[0][2]];
        let ln = norm3(dv);
        if ln < 1e-15 {
            out.push(d.segs[i], w, d.kinds[i], d.rows[i]);
            continue;
        }
        let wdir = [dv[0] / ln, dv[1] / ln, dv[2] / ln];
        let (b2, bw) = follow(i, 1, wdir, &mut used);
        let back = [-wdir[0], -wdir[1], -wdir[2]];
        let (a2, aw) = follow(i, 0, back, &mut used);
        out.push([a2[0], a2[1], b2[0], b2[1]], [aw, bw], d.kinds[i], d.rows[i]);
    }
    out
}

/// built_core.hlr._drop_hidden_under_visible.
fn drop_hidden_under_visible(d: &Drawing, tol: f64) -> Drawing {
    if !d.kinds.contains(&KIND_HIDDEN) {
        return d.clone();
    }
    let atol = 1e-6;
    let inv_d = 1.0 / py_max(tol, 1e-12);
    let key_and_span = |s: [f64; 4]| -> Option<((i64, i64), (f64, f64))> {
        let [x0, y0, x1, y1] = s;
        let (dx, dy) = (x1 - x0, y1 - y0);
        let ln = super::pymath::hypot(dx, dy);
        if ln < 1e-15 {
            return None;
        }
        let (mut ux, mut uy) = (dx / ln, dy / ln);
        if ux < -1e-12 || (ux.abs() <= 1e-12 && uy < 0.0) {
            ux = -ux;
            uy = -uy;
        }
        let ang = uy.atan2(ux);
        let off = ux * y0 - uy * x0;
        let t0 = ux * x0 + uy * y0;
        let t1 = ux * x1 + uy * y1;
        let key = ((ang / atol).round_ties_even() as i64, (off * inv_d).round_ties_even() as i64);
        Some((key, (py_min(t0, t1), py_max(t0, t1))))
    };
    let mut lines: HashMap<(i64, i64), Vec<(f64, f64)>> = HashMap::new();
    for i in 0..d.segs.len() {
        if d.kinds[i] != KIND_HIDDEN {
            if let Some((k, s)) = key_and_span(d.segs[i]) {
                lines.entry(k).or_default().push(s);
            }
        }
    }
    if lines.is_empty() {
        return d.clone();
    }
    let mut out = Drawing::default();
    for i in 0..d.segs.len() {
        if d.kinds[i] != KIND_HIDDEN {
            out.push(d.segs[i], d.world[i], d.kinds[i], d.rows[i]);
            continue;
        }
        let Some(((ka, ko), (lo, hi))) = key_and_span(d.segs[i]) else { continue };
        let mut cover: Vec<(f64, f64)> = Vec::new();
        for da in -1..=1 {
            for dof in -1..=1 {
                if let Some(v) = lines.get(&(ka + da, ko + dof)) {
                    cover.extend_from_slice(v);
                }
            }
        }
        if cover.is_empty() {
            out.push(d.segs[i], d.world[i], d.kinds[i], d.rows[i]);
            lines.entry((ka, ko)).or_default().push((lo, hi));
            continue;
        }
        let span = hi - lo;
        if span < 1e-15 {
            continue;
        }
        let mut rel: Vec<(f64, f64)> = cover
            .iter()
            .filter(|(c0, c1)| *c1 > lo && *c0 < hi)
            .map(|(c0, c1)| (py_max(0.0, (c0 - lo) / span), py_min(1.0, (c1 - lo) / span)))
            .collect();
        rel.sort_by(|p, q| {
            p.0.partial_cmp(&q.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(p.1.partial_cmp(&q.1).unwrap_or(std::cmp::Ordering::Equal))
        });
        let mut merged: Vec<(f64, f64)> = Vec::new();
        for (c0, c1) in rel {
            match merged.last_mut() {
                Some(last) if c0 <= last.1 => last.1 = py_max(last.1, c1),
                _ => merged.push((c0, c1)),
            }
        }
        let [x0, y0, x1, y1] = d.segs[i];
        let h = super::pymath::hypot(x1 - x0, y1 - y0);
        let ux = (x1 - x0) / h;
        let uy = (y1 - y0) / h;
        let forward = ux > 1e-12 || (ux.abs() <= 1e-12 && uy > 0.0);
        let (w0, w1) = (d.world[i][0], d.world[i][1]);
        for (t0, t1) in subtract_spans(&[(0.0, 1.0)], &merged) {
            let (a, b) = if forward { (t0, t1) } else { (1.0 - t1, 1.0 - t0) };
            out.push(
                [x0 + a * (x1 - x0), y0 + a * (y1 - y0), x0 + b * (x1 - x0), y0 + b * (y1 - y0)],
                [lerp3(w0, w1, a), lerp3(w0, w1, b)],
                KIND_HIDDEN,
                d.rows[i],
            );
            lines.entry((ka, ko)).or_default().push((lo + t0 * span, lo + t1 * span));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtract_keeps_the_rest() {
        assert_eq!(subtract_spans(&[(0.0, 1.0)], &[(0.25, 0.5)]), vec![(0.0, 0.25), (0.5, 1.0)]);
        assert_eq!(subtract_spans(&[(0.0, 1.0)], &[]), vec![(0.0, 1.0)]);
    }

    #[test]
    fn a_lone_edge_is_drawn_whole_and_merged_runs_join() {
        let a = [[0.0, 0.0, 1.0], [1.0, 0.0, 1.0]];
        let b = [[1.0, 0.0, 1.0], [2.0, 0.0, 1.0]];
        let w = [[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]], [[1.0, 0.0, 0.0], [2.0, 0.0, 0.0]]];
        let d = drawing(&Input {
            a: &a, b: &b, world: &w, n_hard: 2, n_sil: 0, tris: Vec::new(),
            profiles: true, hidden: false, cap: None, tags: None,
        });
        assert_eq!(d.segs, vec![[0.0, 0.0, 2.0, 0.0]]);
        assert_eq!(d.kinds, vec![KIND_EDGE]);
        assert_eq!(d.rows, vec![0]);
    }

    #[test]
    fn different_tags_never_merge() {
        let a = [[0.0, 0.0, 1.0], [1.0, 0.0, 1.0]];
        let b = [[1.0, 0.0, 1.0], [2.0, 0.0, 1.0]];
        let w = [[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]], [[1.0, 0.0, 0.0], [2.0, 0.0, 0.0]]];
        let tags = [3, 4];
        let d = drawing(&Input {
            a: &a, b: &b, world: &w, n_hard: 2, n_sil: 0, tris: Vec::new(),
            profiles: false, hidden: false, cap: None, tags: Some(&tags),
        });
        assert_eq!(d.segs.len(), 2);
    }
}
