//! Least-squares fit in the two principal directions of a curve's vertices.
use crate::snap::{along, dot, sub, P};
use pyo3::prelude::*;

/// Jacobi rotations diagonalize the symmetric 3x3 scatter matrix. Each
/// corresponding row of basis accumulates the direction of one eigenvector.
fn plane_basis(q: &[P]) -> (P, P) {
    let mut a = [[0.; 3]; 3];
    for p in q {
        for i in 0..3 {
            for j in 0..3 {
                a[i][j] += p[i] * p[j];
            }
        }
    }
    let mut basis = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    for _ in 0..24 {
        let (i, j) = [(0, 1), (0, 2), (1, 2)]
            .into_iter()
            .max_by(|&(i, j), &(k, l)| a[i][j].abs().total_cmp(&a[k][l].abs()))
            .unwrap();
        if a[i][j].abs() <= f64::EPSILON * (a[i][i].abs() + a[j][j].abs()) {
            break;
        }
        let angle = 0.5 * (2. * a[i][j]).atan2(a[i][i] - a[j][j]);
        let (s, c) = angle.sin_cos();
        let mix = |x, y| (c * x + s * y, c * y - s * x);
        for k in 0..3 {
            (a[i][k], a[j][k]) = mix(a[i][k], a[j][k]);
            (basis[i][k], basis[j][k]) = mix(basis[i][k], basis[j][k]);
        }
        for row in &mut a {
            (row[i], row[j]) = mix(row[i], row[j]);
        }
    }
    let mut order = [0, 1, 2];
    order.sort_by(|&i, &j| a[j][j].total_cmp(&a[i][i]));
    (basis[order[0]], basis[order[1]])
}

#[pyfunction]
pub fn snap_circle(py: Python<'_>, points: Vec<P>, tolerance: f64) -> Option<(P, f64)> {
    py.detach(|| {
        if points.len() < 3 {
            return None;
        }
        let n = points.len() as f64;
        let origin = points[0];
        let mut mean = [0.; 3];
        for &p in &points {
            mean = along(mean, sub(p, origin), 1. / n);
        }
        let q = points
            .iter()
            .map(|&p| sub(sub(p, origin), mean))
            .collect::<Vec<_>>();
        let (u, v) = plane_basis(&q);
        let xy = q
            .iter()
            .map(|&p| (dot(p, u), dot(p, v)))
            .collect::<Vec<_>>();
        let (mut xx, mut yy, mut xb, mut yb) = (0., 0., 0., 0.);
        for &(x, y) in &xy {
            let b = x * x + y * y;
            xx += x * x;
            yy += y * y;
            xb += x * b;
            yb += y * b;
        }
        // The centered PCA coordinates have zero mean and are orthogonal:
        // the least-squares normal equations for [2x,2y,1] are diagonal.
        let cutoff = (2. * xx.sqrt()).max(n.sqrt()) * n * f64::EPSILON;
        let center = |v: f64, b| {
            if 2. * v.sqrt() > cutoff {
                b / (2. * v)
            } else {
                0.
            }
        };
        let (cx, cy) = (center(xx, xb), center(yy, yb));
        let r = ((xx + yy) / n + cx * cx + cy * cy).sqrt();
        if !(r > 0.)
            || xy
                .iter()
                .any(|&(x, y)| ((x - cx).hypot(y - cy) - r).abs() > 0.002f64.max(tolerance * r))
        {
            return None;
        }
        Some((along(along(along(origin, mean, 1.), u, cx), v, cy), r))
    })
}

pub(crate) type Circle = (P, f64, P);

pub(crate) fn rim((c, r, n): Circle, p: P, fallback: P) -> P {
    let radial = sub(along(p, n, -dot(sub(p, c), n)), c);
    let dir = if crate::snap::norm(radial) < 1e-9 {
        fallback
    } else {
        crate::snap::unit(radial)
    };
    along(c, dir, r)
}

pub(crate) fn segment_circle([a, b]: [P; 2], (c, r, n): Circle) -> Vec<P> {
    let ab = sub(b, a);
    let dd = dot(ab, ab);
    if r <= 0. || dd < 1e-12 {
        return vec![];
    }
    let offset = dot(sub(c, a), n);
    let den = dot(ab, n);
    if den.abs() > 1e-9 * dd.sqrt() {
        let t = offset / den;
        let p = along(a, ab, t);
        return vec![p]
            .into_iter()
            .filter(|&p| {
                (0.0..=1.0).contains(&t) && (crate::snap::norm(sub(p, c)) - r).abs() < 1e-6
            })
            .collect();
    }
    if offset.abs() > 1e-6 {
        return vec![];
    }
    let f = sub(a, c);
    let df = dot(ab, f);
    let disc = df * df - dd * (dot(f, f) - r * r);
    if disc < 0. {
        return vec![];
    }
    let mut hits = Vec::new();
    for t in [(-df - disc.sqrt()) / dd, (-df + disc.sqrt()) / dd] {
        let p = along(a, ab, t);
        if (0.0..=1.0).contains(&t)
            && hits
                .last()
                .is_none_or(|&q| crate::snap::norm(sub(p, q)) > 1e-6)
        {
            hits.push(p);
        }
    }
    hits
}

pub(crate) fn circle_circle((c, r, n): Circle, (c2, r2, n2): Circle) -> Vec<P> {
    let d = sub(c2, c);
    let l = crate::snap::norm(d);
    if r <= 0.
        || r2 <= 0.
        || (dot(n, n2).abs() - 1.).abs() > 1e-6
        || l < 1e-12
        || dot(d, n).abs() > 1e-6 * l.max(1.)
    {
        return vec![];
    }
    let u = d.map(|v| v / l);
    let a = (l * l + r * r - r2 * r2) / (2. * l);
    let h2 = r * r - a * a;
    if h2 < -1e-12 {
        return vec![];
    }
    let mid = along(c, u, a);
    let h = h2.max(0.).sqrt();
    if h < 1e-9 {
        return vec![mid];
    }
    let perp = crate::snap::cross(n, u);
    vec![along(mid, perp, h), along(mid, perp, -h)]
}

#[pyfunction]
pub fn circle_tangents(circle: Circle, p: P) -> Vec<P> {
    let (c, r, n) = circle;
    let proj = along(p, n, -dot(sub(p, c), n));
    let delta = sub(proj, c);
    let l = crate::snap::norm(delta);
    if r <= 0. || l < r {
        return vec![];
    }
    if l <= r {
        return vec![proj];
    }
    let u = delta.map(|x| x / l);
    let cos = r / l;
    let mid = along(c, u, r * cos);
    let side = r * (1. - cos * cos).sqrt();
    let perp = crate::snap::cross(n, u);
    vec![along(mid, perp, side), along(mid, perp, -side)]
}

#[pyfunction]
pub fn arc_midpoint(points: Vec<P>, center: P, radius: f64) -> Option<P> {
    let mut sum = [0.; 3];
    for p in points {
        sum = along(sum, sub(p, center), 1.);
    }
    let l = crate::snap::norm(sum);
    if l < 1e-9 {
        return None;
    }
    // Match QVector3D::normalized() at the null-vector threshold.
    Some(if l <= 1e-5 {
        center
    } else {
        along(center, sum, radius / l)
    })
}
