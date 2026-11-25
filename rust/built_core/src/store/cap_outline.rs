// SPDX-License-Identifier: GPL-3.0-or-later
//! Trace the boundary of adjacent arrangement regions, keeping creases.
use super::planar::Cell;
use pyo3::{prelude::*, types::PySet};
use std::collections::{BTreeMap, BTreeSet};

type Point = [f64; 2];
pub type Region = (Vec<Point>, Vec<Vec<Point>>);

pub(super) fn on_segment(p: Point, [a, b]: [Point; 2], tol: f64) -> bool {
    let (x, y) = (b[0] - a[0], b[1] - a[1]);
    let (px, py) = (p[0] - a[0], p[1] - a[1]);
    let dot = px * x + py * y;
    (x * py - y * px).abs() <= tol * (x.abs() + y.abs()).max(1.)
        && dot >= -tol
        && dot <= x * x + y * y + tol
}

/// How angle ties around a vertex are broken: as the old tracer's Python
/// set ordered the neighbours (that set, or its replay in Rust), or not at
/// all (the trace declines).
#[derive(Clone, Copy)]
pub(super) enum Ties<'py> {
    Python(Python<'py>),
    Native,
    Decline,
}

/// Store-only callers can decline ties when no interpreter is attached.
pub fn trace(regions: Vec<Region>, keep: &[[Point; 2]], tol: f64) -> Option<Vec<Vec<Point>>> {
    trace_ordered(regions, keep, tol, Ties::Decline).ok().flatten()
}

/// `trace` with ties broken as the Python set broke them, in Rust.
pub(super) fn trace_native(regions: Vec<Region>, keep: &[[Point; 2]], tol: f64) -> Option<Vec<Vec<Point>>> {
    trace_ordered(regions, keep, tol, Ties::Native).ok().flatten()
}

pub(super) fn trace_ordered(
    regions: Vec<Region>,
    keep: &[[Point; 2]],
    tol: f64,
    ties: Ties<'_>,
) -> PyResult<Option<Vec<Vec<Point>>>> {
    let record = !matches!(ties, Ties::Decline);
    let mut encounter = Vec::new();
    let mut coords = BTreeMap::new();
    let mut counts = BTreeMap::new();
    for (outer, holes) in regions {
        for ring in std::iter::once(outer).chain(holes) {
            let mut cells = Vec::with_capacity(ring.len());
            let first = ring.first().copied();
            for p in ring {
                let Some(cell) = Cell::at(p, tol) else {
                    return Ok(None);
                };
                if matches!(ties, Ties::Decline) && cell.point().iter().any(|v| v.abs() >= i64::MAX as f64) {
                    return Ok(None);
                }
                coords.insert(cell, p);
                cells.push(cell);
            }
            for (&a, &b) in cells.iter().zip(cells.iter().cycle().skip(1)) {
                let count = counts.entry((a, b)).or_insert(0usize);
                if *count == 0 && record {
                    encounter.push((a, b));
                }
                *count += 1;
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
        let next = *links[&k].first().unwrap();
        if crease(k, next) {
            net.remove(&(k, next));
            net.remove(&(next, k));
            links.get_mut(&k).unwrap().remove(&next);
            links.get_mut(&next).unwrap().remove(&k);
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
        angles.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        if angles.windows(2).any(|p| p[0].0 == p[1].0) {
            // The old tracer inserts surviving neighbours into a Python set
            // in directed-edge encounter order, then stable-sorts by angle.
            let around = encounter
                .iter()
                .filter(|&&(a, b)| net.contains_key(&(a, b)) && (a == k || b == k))
                .map(|&(a, b)| if a == k { b } else { a });
            let cells: Vec<Cell> = match ties {
                Ties::Decline => return Ok(None),
                Ties::Python(py) => {
                    let set = PySet::empty(py)?;
                    for c in around {
                        set.add(c.tuple(py)?)?;
                    }
                    set.iter()
                        .map(|cell| Ok(Cell::at(cell.extract()?, 1.).unwrap()))
                        .collect::<PyResult<_>>()?
                }
                Ties::Native => {
                    let mut set: crate::pyset::PySet<[i128; 2]> = crate::pyset::PySet::new();
                    for c in around {
                        set.add(super::arrangement::cell_key(c));
                    }
                    set.iter()
                        .map(|[a, b]| Cell::at([a as f64, b as f64], 1.).unwrap())
                        .collect()
                }
            };
            angles.clear();
            for key in cells {
                let q = coords[&key];
                angles.push(((q[1] - p[1]).atan2(q[0] - p[0]), key));
            }
            angles.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
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
                *net.get_mut(&edge).unwrap() -= 1;
                let (a, b) = edge;
                ring.push(coords[&a]);
                if ring.len() > 100000 {
                    break;
                }
                let ns = &order[&b];
                let Some(i) = ns.iter().position(|&n| n == a) else {
                    return Ok(None);
                };
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
    Ok(Some(loops))
}
