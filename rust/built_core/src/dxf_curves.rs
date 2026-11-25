// SPDX-License-Identifier: GPL-3.0-or-later
//! DXF path primitives. Curves retain their cubic control points so each
//! import can flatten the same drawing at its chosen physical tolerance.
use super::{Ring, Tags};
use glam::DVec3;
use std::f64::consts::FRAC_PI_2;
pub(super) type Cubic = [DVec3; 4];

fn close(a: DVec3, b: DVec3) -> bool {
    (a - b)
        .abs()
        .cmple(a.abs().max(b.abs()) * 1e-9 + DVec3::splat(1e-12))
        .all()
}
fn curve_point(c: &Cubic, t: f64) -> DVec3 {
    let u = 1. - t;
    c[0] * (u * u * u) + c[1] * (3. * u * u * t) + c[2] * (3. * u * t * t) + c[3] * (t * t * t)
}
pub(super) fn flatten(curves: &[Cubic], tol: f64) -> Option<Ring> {
    let mut pts = vec![curves.first()?[0]];
    for c in curves {
        if !close(*pts.last()?, c[0]) {
            pts.push(c[0]);
        }
        if c[0] == c[1] && c[2] == c[3] {
            pts.push(c[3]);
            continue;
        }
        for j in 0..8 {
            let (a, b) = (j as f64 / 8., (j + 1) as f64 / 8.);
            let mut work = vec![(a, b, curve_point(c, a), curve_point(c, b))];
            while let Some((a, b, p, q)) = work.pop() {
                let mid = (a + b) * 0.5;
                let m = curve_point(c, mid);
                if m.distance((p + q) * 0.5) <= tol {
                    pts.push(q);
                } else {
                    if mid == a || mid == b || pts.len() > 1_000_000 {
                        return None;
                    }
                    work.push((mid, b, m, q));
                    work.push((a, mid, p, m));
                }
                if pts.len() % 4096 == 0 && crate::file_open::cancelled() {
                    return None;
                }
            }
        }
    }
    Some(pts)
}
pub(super) fn elliptic(center: DVec3, x: DVec3, y: DVec3, start: f64, span: f64) -> Vec<Cubic> {
    let count = (span.abs() / FRAC_PI_2).ceil() as usize;
    let step = span / count as f64;
    let arm = (step * 0.25).tan() * 4. / 3.;
    let pos = |t: f64| center + x * t.cos() + y * t.sin();
    let dir = |t: f64| y * t.cos() - x * t.sin();
    (0..count)
        .map(|i| {
            let a = start + i as f64 * step;
            let b = a + step;
            [pos(a), pos(a) + arm * dir(a), pos(b) - arm * dir(b), pos(b)]
        })
        .collect()
}
pub(super) fn polyline(verts: &[DVec3], bulges: &[f64], closed: bool) -> Vec<Cubic> {
    let mut curves = Vec::new();
    for i in 0..verts.len().saturating_sub(usize::from(!closed)) {
        let a = verts[i];
        let b = verts[(i + 1) % verts.len()];
        let bulge = bulges[i];
        if close(a, b) {
            continue;
        }
        if bulge.abs() < 1e-6 {
            curves.push([a, a, b, b]);
            continue;
        }
        let chord = b - a;
        let center = (a + b) * 0.5
            + DVec3::new(-chord.y, chord.x, 0.) * ((1. - bulge * bulge) / (4. * bulge));
        let r = a.distance(center);
        curves.extend(elliptic(
            center,
            DVec3::X * r,
            DVec3::Y * r,
            (a.y - center.y).atan2(a.x - center.x),
            4. * bulge.atan(),
        ));
    }
    curves
}
fn natural(
    points: &[DVec3],
    widths: &[f64],
    tangents: Option<(DVec3, DVec3)>,
) -> Option<Vec<Cubic>> {
    // C2 cubic interpolation. Solve the tridiagonal system for second
    // derivatives at the data points, with natural or clamped ends.
    let n = points.len();
    if n < 2 || widths.len() != n - 1 || widths.iter().any(|&h| h <= 0.) {
        return None;
    }
    let slopes: Vec<_> = points
        .windows(2)
        .zip(widths)
        .map(|(p, h)| (p[1] - p[0]) / h)
        .collect();
    let mut diag = vec![1.; n];
    let mut upper = vec![0.; n];
    let mut lower = vec![0.; n];
    let mut rhs = vec![DVec3::ZERO; n];
    for i in 1..n - 1 {
        lower[i] = widths[i - 1];
        upper[i] = widths[i];
        diag[i] = 2. * (lower[i] + upper[i]);
        rhs[i] = 6. * (slopes[i] - slopes[i - 1]);
    }
    if let Some((a, b)) = tangents {
        upper[0] = widths[0];
        diag[0] = 2. * upper[0];
        rhs[0] = 6. * (slopes[0] - a);
        lower[n - 1] = widths[n - 2];
        diag[n - 1] = 2. * lower[n - 1];
        rhs[n - 1] = 6. * (b - slopes[n - 2]);
    }
    for i in 1..n {
        let r = lower[i] / diag[i - 1];
        diag[i] -= r * upper[i - 1];
        let prev = rhs[i - 1];
        rhs[i] -= r * prev;
    }
    for i in (0..n).rev() {
        if i + 1 < n {
            let next = rhs[i + 1];
            rhs[i] -= upper[i] * next;
        }
        rhs[i] /= diag[i];
    }
    Some(
        (0..n - 1)
            .map(|i| {
                let h = widths[i];
                let delta = (points[i + 1] - points[i]) / 3.;
                [
                    points[i],
                    points[i] + delta - (2. * rhs[i] + rhs[i + 1]) * (h * h / 18.),
                    points[i + 1] - delta - (rhs[i] + 2. * rhs[i + 1]) * (h * h / 18.),
                    points[i + 1],
                ]
            })
            .collect(),
    )
}
fn chord(points: &[DVec3]) -> Vec<f64> {
    points.windows(2).map(|p| p[0].distance(p[1])).collect()
}
pub(super) fn spline(t: &Tags<'_>) -> Option<Vec<Cubic>> {
    let points = t.points(10)?;
    let mut curves = if points.is_empty() {
        let fit = t.points(11)?;
        let ends = match (t.point(12), t.point(13)) {
            (Some(a), Some(b)) => Some((a.try_normalize()?, b.try_normalize()?)),
            _ => None,
        };
        natural(&fit, &chord(&fit), ends)?
    } else {
        let degree = t.get(71)?.trim().parse::<usize>().ok()?;
        let n = points.len();
        if degree == 0 || degree >= n {
            return None;
        }
        let weights = t.numbers(41)?;
        if !weights.is_empty() && weights.len() != n {
            return None;
        }
        let mut knots = t.numbers(40)?;
        if knots.is_empty() {
            knots = (0..n + degree + 1)
                .map(|i| i.saturating_sub(degree).min(n - degree) as f64 / (n - degree) as f64)
                .collect();
        }
        if knots.len() != n + degree + 1 || knots.windows(2).any(|w| w[1] < w[0]) {
            return None;
        }
        let eps = t.number(42, 1e-10)?;
        let decimals = -eps.log10().trunc();
        if decimals > 0. && decimals < 16. {
            let scale = 10f64.powi(decimals as i32);
            for k in &mut knots {
                *k = (*k * scale).round_ties_even() / scale;
            }
        }
        if knots[0] != 0. {
            let start = knots[0];
            let span = *knots.last()? - start;
            for k in &mut knots {
                *k = (*k - start) / span;
            }
        }
        let eval = |u: f64, span: usize| -> Option<DVec3> {
            let mut basis = vec![0.; degree + 1];
            basis[0] = 1.;
            for j in 1..=degree {
                let mut saved = 0.;
                for r in 0..j {
                    let left = knots[(span + 1 + r).saturating_sub(j)];
                    let right = knots[span + r + 1];
                    let w = if right == left {
                        0.
                    } else {
                        basis[r] / (right - left)
                    };
                    basis[r] = saved + (right - u) * w;
                    saved = (u - left) * w;
                }
                basis[j] = saved;
            }
            let mut v = DVec3::ZERO;
            let mut total = 0.;
            for (i, w) in basis.into_iter().enumerate() {
                let idx = (n + span + i - degree) % n;
                let w = w * weights.get(idx).copied().unwrap_or(1.);
                v += points[idx] * w;
                total += w;
            }
            if !weights.is_empty() && total != 0. {
                v /= total;
            }
            v.is_finite().then_some(v)
        };
        if degree == 3
            && weights.is_empty()
            && knots[degree] == knots[0]
            && knots[n] == *knots.last()?
        {
            let mut result = vec![];
            for span in degree..n {
                let a = knots[span];
                let b = knots[span + 1];
                if a == b {
                    continue;
                }
                let p = eval(a, span)?;
                let q = eval(b, span)?;
                let l = eval(a + (b - a) / 3., span)? * 27. - 8. * p - q;
                let r = eval(a + 2. * (b - a) / 3., span)? * 27. - p - 8. * q;
                result.push([p, (2. * l - r) / 18., (2. * r - l) / 18., q]);
            }
            result
        } else {
            let widths = chord(&points);
            let total: f64 = widths.iter().sum();
            if total <= 0. {
                return None;
            }
            let mut data = vec![];
            let mut at = 0.;
            let mut add = |u: f64| -> Option<()> {
                let u = u * knots.last()? / total;
                let span = knots[..n].partition_point(|&k| k <= u).saturating_sub(1);
                data.push(eval(u, span)?);
                Some(())
            };
            for width in widths {
                for i in 0..8 {
                    add(at + width * i as f64 / 8.)?;
                }
                at += width;
            }
            add(total)?;
            natural(&data, &vec![1.; data.len() - 1], None)?
        }
    };
    if let (Some(first), Some(last)) = (curves.first(), curves.last()) {
        if close(first[0], last[3]) {
            curves.reverse();
            for c in &mut curves {
                c.reverse();
            }
        }
    }
    Some(curves)
}
