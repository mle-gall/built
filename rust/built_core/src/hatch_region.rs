// SPDX-License-Identifier: GPL-3.0-or-later
//! Even-odd clipping for paper hatch strokes and DXF SOLID strips.
use pyo3::prelude::*;
type Point = [f64; 2];
type Segment = (f64, f64, f64, f64);
type Corner = (f64, f64);
type Quad = (Corner, Corner, Corner, Corner);

fn edges(points: &[Point]) -> impl Iterator<Item = (Point, Point)> + '_ {
    points
        .iter()
        .copied()
        .zip(points.iter().copied().cycle().skip(1))
}

struct Ring {
    points: Vec<Point>,
    bounds: [f64; 4],
}

impl Ring {
    fn new(points: Vec<Point>) -> Self {
        let [x, y] = points[0];
        let mut bounds = [x, y, x, y];
        for &[x, y] in &points {
            bounds = [
                bounds[0].min(x),
                bounds[1].min(y),
                bounds[2].max(x),
                bounds[3].max(y),
            ];
        }
        Self { points, bounds }
    }
    fn contains(&self, x: f64, y: f64) -> bool {
        let [xmin, ymin, xmax, ymax] = self.bounds;
        if x < xmin || y < ymin || x > xmax || y > ymax {
            return false;
        }
        edges(&self.points).fold(false, |inside, ([ax, ay], [bx, by])| {
            inside ^ ((ay > y) != (by > y) && x < ax + (y - ay) * (bx - ax) / (by - ay))
        })
    }
}

#[pyfunction]
pub fn hatch_clip_segments(lines: Vec<[f64; 4]>, rings: Vec<Vec<Point>>) -> Vec<Segment> {
    let rings: Vec<_> = rings
        .into_iter()
        .filter(|r| r.len() >= 3)
        .map(Ring::new)
        .collect();
    let mut result = Vec::new();
    for [x0, y0, x1, y1] in lines {
        let (dx, dy) = (x1 - x0, y1 - y0);
        let mut ts = vec![0., 1.];
        for ring in &rings {
            let [xmin, ymin, xmax, ymax] = ring.bounds;
            if xmin > x0.max(x1) || xmax < x0.min(x1) || ymin > y0.max(y1) || ymax < y0.min(y1) {
                continue;
            }
            for ([ax, ay], [bx, by]) in edges(&ring.points) {
                let (ex, ey) = (bx - ax, by - ay);
                let den = dx * ey - dy * ex;
                if den.abs() < 1e-12 {
                    continue;
                }
                let t = ((ax - x0) * ey - (ay - y0) * ex) / den;
                let u = ((ax - x0) * dy - (ay - y0) * dx) / den;
                if 0. < t && t < 1. && (-1e-9..=1. + 1e-9).contains(&u) {
                    ts.push(t);
                }
            }
        }
        ts.sort_by(f64::total_cmp);
        for pair in ts.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if b - a < 1e-9 {
                continue;
            }
            let mid = (a + b) / 2.;
            if rings.iter().fold(false, |inside, r| {
                inside ^ r.contains(x0 + dx * mid, y0 + dy * mid)
            }) {
                result.push((x0 + dx * a, y0 + dy * a, x0 + dx * b, y0 + dy * b));
            }
        }
    }
    result
}

#[pyfunction]
pub fn hatch_fill_quads(rings: Vec<Vec<Point>>) -> Vec<Quad> {
    let rings: Vec<_> = rings.into_iter().filter(|r| r.len() >= 3).collect();
    let edges: Vec<_> = rings
        .iter()
        .flat_map(|r| edges(r))
        .filter(|(a, b)| (a[1] - b[1]).abs() > 1e-12)
        .collect();
    let mut heights: Vec<_> = rings.iter().flatten().map(|p| p[1]).collect();
    heights.sort_by(f64::total_cmp);
    heights.dedup();
    let at = |([ax, ay], [bx, by]): (Point, Point), y| ax + (y - ay) * (bx - ax) / (by - ay);
    let mut quads = Vec::new();
    for pair in heights.windows(2) {
        let (lo, hi) = (pair[0], pair[1]);
        if hi - lo < 1e-12 {
            continue;
        }
        let mid = (hi + lo) / 2.;
        let mut live: Vec<_> = edges
            .iter()
            .copied()
            .filter(|(a, b)| a[1].min(b[1]) < mid && mid < a[1].max(b[1]))
            .collect();
        live.sort_by(|&a, &b| {
            at(a, mid)
                .partial_cmp(&at(b, mid))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for sides in live.chunks_exact(2) {
            let (l, r) = (sides[0], sides[1]);
            quads.push((
                (at(l, lo), lo),
                (at(r, lo), lo),
                (at(l, hi), hi),
                (at(r, hi), hi),
            ));
        }
    }
    quads
}
