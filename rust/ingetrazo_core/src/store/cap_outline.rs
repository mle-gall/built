// SPDX-License-Identifier: GPL-3.0-or-later
//! Trace the boundary of adjacent arrangement regions, keeping creases.
use std::collections::{BTreeMap, BTreeSet};

type Point = [f64; 2];
type Cell = [i64; 2];
pub type Region = (Vec<Point>, Vec<Vec<Point>>);

pub(super) fn on_segment(p: Point, [a, b]: [Point; 2], tol: f64) -> bool {
    let (x, y) = (b[0] - a[0], b[1] - a[1]);
    let (px, py) = (p[0] - a[0], p[1] - a[1]);
    let dot = px * x + py * y;
    (x * py - y * px).abs() <= tol * (x.abs() + y.abs()).max(1.)
        && dot >= -tol
        && dot <= x * x + y * y + tol
}

/// Equal-angle ties in a degenerate graph depend on Python set iteration.
/// Let the reference tracer resolve those, and coordinates outside i64 cells.
pub fn trace(regions: Vec<Region>, keep: &[[Point; 2]], tol: f64) -> Option<Vec<Vec<Point>>> {
    let mut coords = BTreeMap::new();
    let mut counts = BTreeMap::new();
    for (outer, holes) in regions {
        for ring in std::iter::once(outer).chain(holes) {
            let mut cells = Vec::with_capacity(ring.len());
            let first = ring.first().copied();
            for p in ring {
                let scaled = p.map(|v| (v / tol).round_ties_even());
                if scaled
                    .iter()
                    .any(|v| !v.is_finite() || v.abs() >= i64::MAX as f64)
                {
                    return None;
                }
                let cell = scaled.map(|x| x as i64);
                coords.insert(cell, p);
                cells.push(cell);
            }
            for (&a, &b) in cells.iter().zip(cells.iter().cycle().skip(1)) {
                *counts.entry((a, b)).or_insert(0usize) += 1;
            }
            // The reference loop writes both ends per edge, so the last write
            // in the ring is its first vertex.
            if let Some(p) = first {
                coords.insert(cells[0], p);
            }
        }
    }
    let crease = |a: Cell, b: Cell| {
        let (p, q) = (coords[&a], coords[&b]);
        let mid = [(p[0] + q[0]) * 0.5, (p[1] + q[1]) * 0.5];
        keep.iter()
            .any(|&s| [mid, p, q].iter().all(|&v| on_segment(v, s, tol * 0.5)))
    };
    let mut net = BTreeMap::new();
    let mut links: BTreeMap<Cell, BTreeSet<Cell>> = BTreeMap::new();
    for (&(a, b), &count) in &counts {
        let n = if crease(a, b) {
            count
        } else {
            count.saturating_sub(*counts.get(&(b, a)).unwrap_or(&0))
        };
        if n > 0 {
            net.insert((a, b), n);
            links.entry(a).or_default().insert(b);
            links.entry(b).or_default().insert(a);
        }
    }
    let mut leaves: Vec<_> = links
        .iter()
        .filter(|(_, ns)| ns.len() == 1)
        .map(|(&k, _)| k)
        .collect();
    while let Some(k) = leaves.pop() {
        if links[&k].len() != 1 {
            continue;
        }
        let next = *links[&k].first()?;
        if crease(k, next) {
            net.remove(&(k, next));
            net.remove(&(next, k));
            links.get_mut(&k)?.remove(&next);
            links.get_mut(&next)?.remove(&k);
            if links[&next].len() == 1 {
                leaves.push(next);
            }
        }
    }
    let mut order = BTreeMap::new();
    for (k, ns) in links {
        let p = coords[&k];
        let mut angles: Vec<_> = ns
            .into_iter()
            .map(|n| {
                let q = coords[&n];
                ((q[1] - p[1]).atan2(q[0] - p[0]), n)
            })
            .collect();
        angles.sort_by(|a, b| a.0.total_cmp(&b.0));
        if angles.windows(2).any(|p| p[0].0 == p[1].0) {
            return None;
        }
        order.insert(k, angles.into_iter().map(|(_, n)| n).collect::<Vec<_>>());
    }
    let mut loops = Vec::new();
    let starts: Vec<_> = net.keys().copied().collect();
    for start in starts {
        while net[&start] > 0 {
            let mut edge = start;
            let mut ring = Vec::new();
            loop {
                *net.get_mut(&edge)? -= 1;
                let (a, b) = edge;
                ring.push(coords[&a]);
                if ring.len() > 100000 {
                    break;
                }
                let ns = &order[&b];
                let i = ns.iter().position(|&n| n == a)?;
                let Some(next) = (1..=ns.len())
                    .map(|s| (b, ns[(i + ns.len() - s) % ns.len()]))
                    .find(|step| *step == start || net.get(step).copied().unwrap_or(0) > 0)
                else {
                    break;
                };
                if next == start {
                    if ring.len() >= 3 {
                        loops.push(ring);
                    }
                    break;
                }
                edge = next;
            }
        }
    }
    Some(loops)
}
