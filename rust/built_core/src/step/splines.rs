// SPDX-License-Identifier: GPL-3.0-or-later
//! Keep the original rational basis and physical control coordinates.
use crate::exact::{Axis, Curve, Surface};
use built_step::step_io::scene::{NurbsCurve, NurbsSurface};
use glam::{DVec2, DVec3, DVec4};

type Result<T> = std::result::Result<T, &'static str>;

fn axis(degree: usize, expanded: Vec<f64>) -> Axis {
    let mut a = Axis {
        degree,
        knots: Vec::new(),
        multiplicities: Vec::new(),
        periodic: false,
    };
    for k in expanded {
        if a.knots.last() == Some(&k) {
            *a.multiplicities.last_mut().unwrap() += 1;
        } else {
            a.knots.push(k);
            a.multiplicities.push(1);
        }
    }
    a
}
fn expanded(a: &Axis) -> Vec<f64> {
    a.knots
        .iter()
        .zip(&a.multiplicities)
        .flat_map(|(&k, &n)| std::iter::repeat_n(k, n))
        .collect()
}
fn control(p: [f64; 3], w: f64, m: f64) -> DVec4 {
    (DVec3::from(p) * m).extend(w)
}

pub(super) fn curve(n: NurbsCurve, m: f64) -> Result<Curve> {
    if n.control_points.len() != n.weights.len() {
        return Err("incomplete spline weights");
    }
    let controls: Vec<_> = n
        .control_points
        .into_iter()
        .zip(n.weights)
        .map(|(p, w)| control(p, w, m))
        .collect();
    let axis = axis(n.degree, n.knots);
    axis.validate(controls.len())?;
    Ok(Curve::Spline { axis, controls })
}
pub(super) fn surface(n: NurbsSurface, m: f64) -> Result<Surface> {
    let rows = n.control_points.len();
    let cols = n.control_points.first().ok_or("empty spline net")?.len();
    if n.weights.len() != rows
        || n.weights.iter().any(|r| r.len() != cols)
        || n.control_points.iter().any(|r| r.len() != cols)
    {
        return Err("invalid spline net");
    }
    let u = axis(n.degree_u, n.knots_u);
    let v = axis(n.degree_v, n.knots_v);
    u.validate(rows)?;
    v.validate(cols)?;
    let controls = n
        .control_points
        .into_iter()
        .flatten()
        .zip(n.weights.into_iter().flatten())
        .map(|(p, w)| control(p, w, m))
        .collect();
    Ok(Surface::Spline {
        u,
        v,
        rows,
        cols,
        controls,
    })
}
pub(super) fn output_curve(a: &Axis, controls: &[DVec4]) -> NurbsCurve {
    NurbsCurve {
        degree: a.degree,
        knots: expanded(a),
        control_points: controls
            .iter()
            .map(|p| (p.truncate() * 1000.).to_array())
            .collect(),
        weights: controls.iter().map(|p| p.w).collect(),
    }
}
pub(super) fn output_surface(u: &Axis, v: &Axis, cols: usize, controls: &[DVec4]) -> NurbsSurface {
    NurbsSurface {
        degree_u: u.degree,
        degree_v: v.degree,
        knots_u: expanded(u),
        knots_v: expanded(v),
        control_points: controls
            .chunks(cols)
            .map(|r| {
                r.iter()
                    .map(|p| (p.truncate() * 1000.).to_array())
                    .collect()
            })
            .collect(),
        weights: controls
            .chunks(cols)
            .map(|r| r.iter().map(|p| p.w).collect())
            .collect(),
    }
}
pub(super) fn domain(a: &Axis) -> [f64; 2] {
    let k = expanded(a);
    [k[a.degree], k[k.len() - 1 - a.degree]]
}
/// Inversion provides a candidate only. The lifted interval is subsequently
/// checked against the spatial curve over its entire extent by the topology validator.
pub(super) fn project(
    s: &Surface,
    curves: &[Curve],
    p: DVec3,
    limits: [[f64; 2]; 2],
    tol: f64,
) -> Result<DVec2> {
    let lo = DVec2::new(limits[0][0], limits[1][0]);
    let hi = DVec2::new(limits[0][1], limits[1][1]);
    let span = hi - lo;
    let eval = |q: DVec2| s.point(curves, q.x, q.y);
    let mut best = (f64::INFINITY, lo);
    for i in 0..=4 {
        for j in 0..=4 {
            let q = lo + span * DVec2::new(i as f64, j as f64) * 0.25;
            let dist = eval(q)?.distance_squared(p);
            if dist < best.0 {
                best = (dist, q);
            }
        }
    }
    let mut q = best.1;
    for _ in 0..32 {
        let delta = p - eval(q)?;
        if delta.length() <= tol.min(1e-12) {
            return Ok(q);
        }
        let tangent = |d: DVec2| -> Result<DVec3> {
            let a = (q - d).max(lo);
            let b = (q + d).min(hi);
            Ok((eval(b)? - eval(a)?) / (b - a).length())
        };
        let a = tangent(DVec2::new(span.x * 1e-6, 0.))?;
        let b = tangent(DVec2::new(0., span.y * 1e-6))?;
        let normal = a.cross(b);
        let n = normal.length_squared();
        if n == 0. {
            break;
        }
        let step = DVec2::new(delta.cross(b).dot(normal), a.cross(delta).dot(normal)) / n;
        if !step.is_finite() {
            break;
        }
        let next = (q + step).clamp(lo, hi);
        if next == q {
            break;
        }
        q = next;
    }
    if eval(q)?.distance(p) <= tol {
        return Ok(q);
    }
    Err("cannot resolve spline trim parameter")
}

/// A rational quadratic Bezier is circular exactly when its middle
/// control is the intersection of the endpoint tangents and its normalized
/// weight is cos(half sweep). Check every Bezier span and their common frame.
pub(super) fn circular(n: &NurbsCurve, m: f64) -> Option<(crate::exact::Frame, f64, f64)> {
    if n.degree != 2
        || n.control_points.len() < 3
        || n.control_points.len() % 2 != 1
        || n.weights.len() != n.control_points.len()
    {
        return None;
    }
    let a = axis(n.degree, n.knots.clone());
    if a.multiplicities.first() != Some(&3)
        || a.multiplicities.last() != Some(&3)
        || a.multiplicities[1..a.knots.len() - 1]
            .iter()
            .any(|&i| i != 2)
    {
        return None;
    }
    let mut result: Option<(crate::exact::Frame, f64)> = None;
    let mut sweep = 0.;
    for i in (0..n.control_points.len() - 2).step_by(2) {
        let [p, q, r] = [i, i + 1, i + 2].map(|k| DVec3::from(n.control_points[k]) * m);
        let x = (q - p).try_normalize()?;
        let z = x.cross(r - q).try_normalize()?;
        let inward = z.cross(x);
        let radius = (r - p).length_squared() / (2. * (r - p).dot(inward));
        if !radius.is_finite() || radius <= 0. {
            return None;
        }
        let center = p + radius * inward;
        let end = r - center;
        let start = p - center;
        let tol = 512. * f64::EPSILON * (1. + p.length() + r.length() + radius);
        if (end.length() - radius).abs() > tol || end.dot((r - q).try_normalize()?).abs() > tol {
            return None;
        }
        let theta = z.dot(start.cross(end)).atan2(start.dot(end));
        if theta <= 0. || theta >= std::f64::consts::PI {
            return None;
        }
        let w = n.weights[i + 1] / (n.weights[i] * n.weights[i + 2]).sqrt();
        if !w.is_finite() || (w - (theta * 0.5).cos()).abs() * radius > tol {
            return None;
        }
        if let Some((frame, first)) = &result {
            if center.distance(frame.origin) > tol
                || (radius - first).abs() > tol
                || z.dot(frame.axis) < 1. - 1e-12
            {
                return None;
            }
        } else {
            result = Some((
                crate::exact::Frame {
                    origin: center,
                    axis: z,
                    reference: start / radius,
                },
                radius,
            ));
        }
        sweep += theta;
    }
    let (frame, radius) = result?;
    Some((frame, radius, sweep))
}

/// Recognize a linear direction of a tensor product whose two end rows
/// are congruent coaxial circular Bezier spans (possibly differing radii).
pub(super) fn ruled_circle(n: &NurbsSurface, m: f64) -> Option<(Surface, bool)> {
    let transposed = n.degree_v == 1 && n.control_points.first()?.len() == 2;
    let line_degree = if transposed { n.degree_v } else { n.degree_u };
    let circular_degree = if transposed { n.degree_u } else { n.degree_v };
    if line_degree != 1 || circular_degree != 2 || (!transposed && n.control_points.len() != 2) {
        return None;
    }
    let row = |i: usize| NurbsCurve {
        degree: 2,
        knots: if transposed {
            n.knots_u.clone()
        } else {
            n.knots_v.clone()
        },
        control_points: if transposed {
            n.control_points.iter().map(|r| r[i]).collect()
        } else {
            n.control_points[i].clone()
        },
        weights: if transposed {
            n.weights.iter().map(|r| r[i]).collect()
        } else {
            n.weights[i].clone()
        },
    };
    let (first, second) = (row(0), row(1));
    // Equal relative weights make each generator a straight interpolation
    // between corresponding points on the two end sections.
    if first.weights.len() != second.weights.len()
        || first
            .weights
            .iter()
            .zip(&second.weights)
            .any(|(a, b)| (a * second.weights[0] - b * first.weights[0]).abs() > 1e-13)
    {
        return None;
    }
    let ca = circular(&first, m);
    let cb = circular(&second, m);
    let (mut frame, radius, apex, row_is_first) = match (&ca, &cb) {
        (Some((f, r, _)), None) => (Some(f.clone()), *r, &second, true),
        (None, Some((f, r, _))) => (Some(f.clone()), *r, &first, false),
        _ => (None, 0., &first, false),
    };
    if let Some(mut f) = frame.take() {
        let o = DVec3::from(*apex.control_points.first()?) * m;
        if !apex
            .control_points
            .iter()
            .all(|p| (DVec3::from(*p) * m).distance(o) < 1e-13)
        {
            return None;
        }
        let delta = f.origin - o;
        let h = delta.dot(f.axis);
        if h.abs() < 1e-12 || (delta - f.axis * h).length() > 1e-12 {
            return None;
        }
        let ab = if row_is_first { -h } else { h };
        f.axis *= h.signum();
        return Some((
            Surface::Cone {
                frame: f,
                radius,
                half_angle: (radius / h.abs()).atan(),
            },
            if transposed { ab < 0. } else { ab > 0. },
        ));
    }
    let (fa, ra, sa) = ca?;
    let (fb, rb, sb) = cb?;
    if (sa - sb).abs() > 1e-10
        || fa.axis.distance(fb.axis) > 1e-10
        || fa.reference.distance(fb.reference) > 1e-10
    {
        return None;
    }
    let delta = fb.origin - fa.origin;
    let h = delta.dot(fa.axis);
    if h.abs() < 1e-12 || (delta - fa.axis * h).length() > 1e-12 {
        return None;
    }
    let flip = if transposed { h < 0. } else { h > 0. };
    let surface = if (ra - rb).abs() < 1e-13 {
        Surface::Cylinder {
            frame: fa,
            radius: ra,
        }
    } else {
        Surface::Cone {
            frame: fa,
            radius: ra,
            half_angle: ((rb - ra) / h).atan(),
        }
    };
    Some((surface, flip))
}
