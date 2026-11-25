// SPDX-License-Identifier: GPL-3.0-or-later
//! Circular closed sweeps whose path axis lies in the profile plane.
use super::sweep::Station;
use crate::store::geometry::{py_sum, Hypot, V3};
use crate::store::geometry::{face_centroid, face_normal};

type Point = [f64; 3];
fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[(i + 1) % 3] * b[(i + 2) % 3] - a[(i + 2) % 3] * b[(i + 1) % 3])
}
fn unit(v: Point) -> Option<Point> {
    let len = dot(v, v).sqrt();
    (len >= 1e-12).then(|| v.map(|x| x / len))
}
fn clip_half(points: &[Point], tolerance: f64) -> Option<Vec<Point>> {
    let mut out = Vec::new();
    for (i, &[h, x, y]) in points.iter().enumerate() {
        let [hh, xx, _] = points[(i + 1) % points.len()];
        let inside = x >= -tolerance;
        if inside {
            out.push(if x > tolerance {
                [h, x, y]
            } else {
                [h, 0., 0.]
            });
        }
        if inside != (xx >= -tolerance) {
            out.push([h + (hh - h) * (x / (x - xx)), 0., 0.]);
        }
    }
    let same = |a: &Point, b: &Point| (0..3).all(|i| (a[i] - b[i]).abs() <= 1e-12);
    out.dedup_by(|a, b| same(a, b));
    while out.len() > 1 && same(&out[0], out.last().unwrap()) {
        out.pop();
    }
    (out.len() >= 3).then_some(out)
}

pub(super) fn stations(
    path: &[Point],
    profile: &Station,
    h: Hypot,
    compensated: bool,
) -> Option<Vec<Station>> {
    if path.len() < 3 {
        return None;
    }
    let outer = profile.first()?;
    if outer.len() < 3 {
        return None;
    }
    let center: Point =
        std::array::from_fn(|k| py_sum(path.iter().map(|p| p[k]), compensated) / path.len() as f64);
    let relative = |p: Point| std::array::from_fn(|k| p[k] - center[k]);
    let rel: Vec<_> = path.iter().copied().map(relative).collect();
    let mut axis = [0.; 3];
    for (i, a) in path.iter().enumerate() {
        let b = path[(i + 1) % path.len()];
        for (k, value) in axis.iter_mut().enumerate() {
            let (j, l) = ((k + 1) % 3, (k + 2) % 3);
            *value += (a[j] - b[j]) * (a[l] + b[l]);
        }
    }
    let axis = unit(axis)?;
    let radii: Vec<_> = rel.iter().map(|&r| dot(r, r).sqrt()).collect();
    let radius = py_sum(radii.iter().copied(), compensated) / path.len() as f64;
    let tol = 1e-4_f64.max(1e-3 * radius);
    let spread = radii.iter().copied().fold(f64::NEG_INFINITY, f64::max)
        - radii.iter().copied().fold(f64::INFINITY, f64::min);
    if radius < 1e-9
        || spread > tol + 0.01 * radius
        || rel.iter().any(|&r| dot(r, axis).abs() > tol)
    {
        return None;
    }
    let points: Vec<_> = outer
        .iter()
        .map(|p| V3(p[0], p[1], p[2]))
        .collect();
    let indices: Vec<_> = (0..points.len() as u32).collect();
    let xyz = |p: V3| [p.0, p.1, p.2];
    let normal = unit(xyz(face_normal(&points, &indices, h)))?;
    if dot(normal, axis).abs() > 1e-3 || dot(relative(outer[0]), normal).abs() > tol {
        return None;
    }
    let mut radial = unit(cross(axis, normal))?;
    let centroid = xyz(face_centroid(&points, &indices, compensated));
    if dot(relative(centroid), radial) < 0. {
        radial = radial.map(|x| -x);
    }
    let side = cross(axis, radial);
    let local = |&p: &Point| {
        let r = relative(p);
        [dot(r, axis), dot(r, radial), dot(r, side)]
    };
    let mut outline: Vec<_> = outer.iter().map(local).collect();
    let size = outline.iter().flatten().map(|x| x.abs()).fold(0., f64::max);
    let keep_tol = 1e-7_f64.max(1e-6 * if size == 0. { 1. } else { size });
    if outline.iter().any(|p| p[1] < -keep_tol) {
        outline = clip_half(&outline, keep_tol)?;
    } else {
        for p in &mut outline {
            p[2] = 0.;
            if p[1].abs() <= keep_tol {
                p[1] = 0.;
            }
        }
    }
    let mut loops = vec![outline];
    for ring in &profile[1..] {
        let hole: Vec<_> = ring.iter().map(local).collect();
        if hole.iter().all(|p| p[1] >= -keep_tol) {
            loops.push(hole);
        } else if hole.iter().any(|p| p[1] > keep_tol) {
            return None;
        }
    }
    Some(
        rel.iter()
            .map(|&r| {
                let (ca, sa) = (dot(r, radial), dot(r, side));
                let norm = ca.hypot(sa);
                let norm = if norm == 0. { 1. } else { norm };
                let (ca, sa) = (ca / norm, sa / norm);
                loops
                    .iter()
                    .map(|ring| {
                        ring.iter()
                            .map(|&[height, x, y]| {
                                std::array::from_fn(|k| {
                                    center[k]
                                        + axis[k] * height
                                        + (radial[k] * ca + side[k] * sa) * x
                                        + (side[k] * ca - radial[k] * sa) * y
                                })
                            })
                            .collect()
                    })
                    .collect()
            })
            .collect(),
    )
}
