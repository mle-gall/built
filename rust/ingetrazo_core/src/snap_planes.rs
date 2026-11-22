//! Work planes and projected references shared by camera and drawing tools.
use crate::snap::{along, cross, dot, norm, sub, unit, P};
use pyo3::prelude::*;
type Plane = (P, P);

#[pyfunction]
pub fn plane_cut_ray(origin: P, ray: P, first: Plane, second: Plane) -> Option<(P, P)> {
    let (p, n) = first;
    let (q, m) = second;
    let axis = cross(n, m);
    let l = norm(axis);
    if l < 0.02 {
        return None;
    }
    let u = unit(axis);
    // Start on the first plane; solve the second plane relative to that
    // point to keep the small offset when both planes are in a distant frame.
    let anchor = along(p, cross(axis, n), dot(m, sub(q, p)) / (l * l));
    let delta = sub(anchor, origin);
    let cos = dot(u, ray);
    let den = 1. - cos * cos;
    if den < 1e-9 {
        return None;
    }
    let point = along(anchor, u, (cos * dot(delta, ray) - dot(delta, u)) / den);
    Some((
        point,
        along(point, u, (norm(sub(point, origin)) * 0.01).max(1e-3)),
    ))
}

#[pyfunction]
pub fn plane_alignments(reference: P, p: P, normal: P, axes: [P; 3]) -> Vec<(usize, P, P)> {
    if norm(normal) < 1e-9 {
        return vec![];
    }
    let n = unit(normal);
    axes.iter()
        .enumerate()
        .filter_map(|(i, &a)| {
            let line = cross(a, n);
            if norm(line) < 1e-5 {
                return None;
            }
            let d = unit(line);
            let m = cross(n, d);
            Some((i, along(p, m, -dot(sub(p, reference), a) / dot(m, a)), d))
        })
        .collect()
}

#[pyfunction]
pub fn lock_on_extension(start: P, axis: P, a: P, b: P) -> Option<(P, P)> {
    let v = sub(b, a);
    let w = sub(start, a);
    let (bb, cc, dd, ee, uu) = (
        dot(axis, v),
        dot(v, v),
        dot(axis, w),
        dot(v, w),
        dot(axis, axis),
    );
    let den = cc * uu - bb * bb;
    if cc <= 1e-12 || den.abs() <= 1e-9 * cc * uu {
        return None;
    }
    let s = (bb * ee - cc * dd) / den;
    let t = (ee * uu - bb * dd) / den;
    let hit = along(a, v, t);
    (norm(sub(along(start, axis, s), hit)) <= 1e-3 && norm(sub(hit, start)) > 1e-6)
        .then_some((hit, if t < 0.5 { a } else { b }))
}

#[pyfunction]
pub fn face_plane(points: Vec<P>) -> Option<(P, P)> {
    let origin = *points.first()?;
    let mut sum = [0.; 3];
    let mut normal = [0.; 3];
    for (&a, &b) in points.iter().zip(points.iter().cycle().skip(1)) {
        sum = along(sum, sub(a, origin), 1.);
        normal = along(normal, cross(sub(a, origin), sub(b, origin)), 1.);
    }
    let normal = if norm(normal) < 1e-12 {
        [0.; 3]
    } else {
        unit(normal)
    };
    Some((along(origin, sum, 1. / points.len() as f64), normal))
}
