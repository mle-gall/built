// SPDX-License-Identifier: GPL-3.0-or-later
//! Exact 2D curves: lines, counter-clockwise arcs and circles, with their
//! intersections, projections and bounds. Lengths are sketch units (mm).
use glam::DVec2;
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};

/// Points closer than this are the same point (sketch units).
pub const EPS: f64 = 1e-6;

/// One drawn entity. Arcs run counter-clockwise from `a0` to `a1`
/// (radians, `a0 < a1 <= a0 + 2π`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Shape {
    Line { a: DVec2, b: DVec2 },
    Arc { c: DVec2, r: f64, a0: f64, a1: f64 },
    Circle { c: DVec2, r: f64 },
}

pub fn polar(c: DVec2, r: f64, t: f64) -> DVec2 {
    c + DVec2::new(t.cos(), t.sin()) * r
}

/// The angle of `p` about `c`, in `[a0, a0 + 2π)`.
pub fn angle_from(c: DVec2, p: DVec2, a0: f64) -> f64 {
    let t = (p - c).y.atan2((p - c).x);
    a0 + (t - a0).rem_euclid(TAU)
}

pub fn arc(c: DVec2, r: f64, a0: f64, a1: f64) -> Shape {
    let mut a1 = a0 + (a1 - a0).rem_euclid(TAU);
    if a1 - a0 < 1e-12 {
        a1 += TAU;
    }
    Shape::Arc { c, r, a0, a1 }
}

/// The arc through three points (start, a point on it, end), or None
/// when they are collinear.
pub fn arc3(p: DVec2, m: DVec2, q: DVec2) -> Option<Shape> {
    let c = circumcentre(p, m, q)?;
    let r = (p - c).length();
    let a0 = angle_from(c, p, 0.0);
    let am = angle_from(c, m, a0);
    let aq = angle_from(c, q, a0);
    Some(if am <= aq { arc(c, r, a0, aq) } else { arc(c, r, aq, a0 + TAU) })
}

pub fn circumcentre(a: DVec2, b: DVec2, c: DVec2) -> Option<DVec2> {
    let d = 2.0 * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
    let scale = (b - a).length().max((c - a).length()).max(EPS);
    if d.abs() < 1e-12 * scale * scale {
        return None;
    }
    let (a2, b2, c2) = (a.length_squared(), b.length_squared(), c.length_squared());
    Some(DVec2::new(
        (a2 * (b.y - c.y) + b2 * (c.y - a.y) + c2 * (a.y - b.y)) / d,
        (a2 * (c.x - b.x) + b2 * (a.x - c.x) + c2 * (b.x - a.x)) / d,
    ))
}

impl Shape {
    pub fn start(&self) -> Option<DVec2> {
        match *self {
            Shape::Line { a, .. } => Some(a),
            Shape::Arc { c, r, a0, .. } => Some(polar(c, r, a0)),
            Shape::Circle { .. } => None,
        }
    }
    pub fn end(&self) -> Option<DVec2> {
        match *self {
            Shape::Line { b, .. } => Some(b),
            Shape::Arc { c, r, a1, .. } => Some(polar(c, r, a1)),
            Shape::Circle { .. } => None,
        }
    }
    pub fn ends(&self) -> Option<[DVec2; 2]> {
        Some([self.start()?, self.end()?])
    }
    pub fn length(&self) -> f64 {
        match *self {
            Shape::Line { a, b } => (b - a).length(),
            Shape::Arc { r, a0, a1, .. } => r * (a1 - a0),
            Shape::Circle { r, .. } => TAU * r,
        }
    }
    pub fn mid(&self) -> DVec2 {
        self.at(0.5)
    }
    /// The point at parameter `t` in [0, 1] (circles start at angle 0).
    pub fn at(&self, t: f64) -> DVec2 {
        match *self {
            Shape::Line { a, b } => a.lerp(b, t),
            Shape::Arc { c, r, a0, a1 } => polar(c, r, a0 + (a1 - a0) * t),
            Shape::Circle { c, r } => polar(c, r, TAU * t),
        }
    }
    /// The unit tangent at `t`, in the drawing direction.
    pub fn tangent(&self, t: f64) -> DVec2 {
        match *self {
            Shape::Line { a, b } => (b - a).normalize_or_zero(),
            Shape::Arc { a0, a1, .. } => {
                let th = a0 + (a1 - a0) * t;
                DVec2::new(-th.sin(), th.cos())
            }
            Shape::Circle { .. } => {
                let th = TAU * t;
                DVec2::new(-th.sin(), th.cos())
            }
        }
    }
    pub fn centre(&self) -> Option<DVec2> {
        match *self {
            Shape::Arc { c, .. } | Shape::Circle { c, .. } => Some(c),
            Shape::Line { .. } => None,
        }
    }
    pub fn radius(&self) -> Option<f64> {
        match *self {
            Shape::Arc { r, .. } | Shape::Circle { r, .. } => Some(r),
            Shape::Line { .. } => None,
        }
    }
    /// The same curve run the other way (circles are unchanged).
    pub fn reversed(&self) -> Shape {
        match *self {
            Shape::Line { a, b } => Shape::Line { a: b, b: a },
            other => other,
        }
    }
    /// Parameter of the closest point, clamped to the curve.
    pub fn project(&self, p: DVec2) -> f64 {
        match *self {
            Shape::Line { a, b } => {
                let d = b - a;
                let l2 = d.length_squared();
                if l2 < EPS * EPS {
                    0.0
                } else {
                    ((p - a).dot(d) / l2).clamp(0.0, 1.0)
                }
            }
            Shape::Arc { c, a0, a1, .. } => {
                let t = angle_from(c, p, a0);
                if t <= a1 {
                    (t - a0) / (a1 - a0)
                } else {
                    // Past the end: the nearer end.
                    let to_end = t - a1;
                    let to_start = a0 + TAU - t;
                    if to_end < to_start {
                        1.0
                    } else {
                        0.0
                    }
                }
            }
            Shape::Circle { c, .. } => angle_from(c, p, 0.0) / TAU,
        }
    }
    pub fn distance(&self, p: DVec2) -> f64 {
        (self.at(self.project(p)) - p).length()
    }
    /// Parameter of a point known to lie on the (unbounded) carrier; values
    /// outside [0, 1] are beyond the ends (lines) or outside the span (arcs:
    /// measured on from `a1`).
    pub fn param_of(&self, p: DVec2) -> f64 {
        match *self {
            Shape::Line { a, b } => {
                let d = b - a;
                (p - a).dot(d) / d.length_squared().max(EPS * EPS)
            }
            Shape::Arc { c, a0, a1, .. } => (angle_from(c, p, a0) - a0) / (a1 - a0),
            Shape::Circle { c, .. } => angle_from(c, p, 0.0) / TAU,
        }
    }
    /// The piece between parameters `t0 < t1`.
    pub fn sub(&self, t0: f64, t1: f64) -> Shape {
        match *self {
            Shape::Line { .. } => Shape::Line { a: self.at(t0), b: self.at(t1) },
            Shape::Arc { c, r, a0, a1 } => {
                let s = a1 - a0;
                Shape::Arc { c, r, a0: a0 + s * t0, a1: a0 + s * t1 }
            }
            Shape::Circle { c, r } => Shape::Arc { c, r, a0: TAU * t0, a1: TAU * t1 },
        }
    }
    pub fn transformed(&self, f: &dyn Fn(DVec2) -> DVec2, scale: f64, rot: f64) -> Shape {
        match *self {
            Shape::Line { a, b } => Shape::Line { a: f(a), b: f(b) },
            Shape::Arc { c, r, a0, a1 } => Shape::Arc { c: f(c), r: r * scale, a0: a0 + rot, a1: a1 + rot },
            Shape::Circle { c, r } => Shape::Circle { c: f(c), r: r * scale },
        }
    }
    pub fn scaled(&self, about: DVec2, k: f64) -> Shape {
        self.transformed(&|p| about + (p - about) * k, k, 0.0)
    }
    pub fn translated(&self, d: DVec2) -> Shape {
        self.transformed(&|p| p + d, 1.0, 0.0)
    }
    /// Exact bounds: arcs include the axis extremes inside their span.
    pub fn bounds(&self) -> (DVec2, DVec2) {
        match *self {
            Shape::Line { a, b } => (a.min(b), a.max(b)),
            Shape::Circle { c, r } => (c - DVec2::splat(r), c + DVec2::splat(r)),
            Shape::Arc { c, r, a0, a1 } => {
                let (s, e) = (polar(c, r, a0), polar(c, r, a1));
                let (mut lo, mut hi) = (s.min(e), s.max(e));
                let mut k = (a0 / (PI / 2.0)).ceil();
                while k * PI / 2.0 <= a1 + 1e-12 {
                    let p = polar(c, r, k * PI / 2.0);
                    lo = lo.min(p);
                    hi = hi.max(p);
                    k += 1.0;
                }
                (lo, hi)
            }
        }
    }
    /// Points along the curve: lines are their ends, round parts follow
    /// `segments` per full turn (the model's facet count).
    pub fn polyline(&self, segments: u32) -> Vec<DVec2> {
        match *self {
            Shape::Line { a, b } => vec![a, b],
            Shape::Arc { c, r, a0, a1 } => {
                let n = (((a1 - a0) / TAU * segments.max(3) as f64).ceil() as usize).max(1);
                (0..=n).map(|i| polar(c, r, a0 + (a1 - a0) * i as f64 / n as f64)).collect()
            }
            Shape::Circle { c, r } => {
                let n = segments.max(3) as usize;
                (0..=n).map(|i| polar(c, r, TAU * i as f64 / n as f64)).collect()
            }
        }
    }
    /// Area term of the closed boundary through this curve: half of the
    /// integral of x dy - y dx along it, in the drawing direction.
    pub fn area_term(&self) -> f64 {
        match *self {
            Shape::Line { a, b } => 0.5 * a.perp_dot(b),
            Shape::Arc { c, r, a0, a1 } => {
                0.5 * (r * r * (a1 - a0) + r * (c.x * (a1.sin() - a0.sin()) - c.y * (a1.cos() - a0.cos())))
            }
            Shape::Circle { r, .. } => PI * r * r,
        }
    }
    /// Is `p` on the curve's own span (not only on its carrier)?
    pub fn contains(&self, p: DVec2, tol: f64) -> bool {
        self.distance(p) <= tol
    }
}

/// Intersections of the carriers (infinite line, full circle).
fn carrier_hits(a: &Shape, b: &Shape) -> Vec<DVec2> {
    use Shape::*;
    match (*a, *b) {
        (Line { a: p, b: q }, Line { a: r, b: s }) => {
            let (d1, d2) = (q - p, s - r);
            let den = d1.perp_dot(d2);
            if den.abs() < 1e-12 * d1.length() * d2.length() {
                return vec![];
            }
            let t = (r - p).perp_dot(d2) / den;
            vec![p + d1 * t]
        }
        (Line { a: p, b: q }, Arc { c, r, .. } | Circle { c, r }) | (Arc { c, r, .. } | Circle { c, r }, Line { a: p, b: q }) => {
            let d = q - p;
            let l = d.length();
            if l < EPS {
                return vec![];
            }
            let u = d / l;
            let foot = p + u * (c - p).dot(u);
            let h2 = r * r - (foot - c).length_squared();
            if h2 < -1e-9 * r.max(1.0) {
                vec![]
            } else if h2 <= 1e-12 * r.max(1.0) * r.max(1.0) {
                vec![foot]
            } else {
                let h = h2.sqrt();
                vec![foot - u * h, foot + u * h]
            }
        }
        (Arc { c: c1, r: r1, .. } | Circle { c: c1, r: r1 }, Arc { c: c2, r: r2, .. } | Circle { c: c2, r: r2 }) => {
            let d = (c2 - c1).length();
            if d < EPS || d > r1 + r2 + 1e-9 || d < (r1 - r2).abs() - 1e-9 {
                return vec![];
            }
            let x = (d * d + r1 * r1 - r2 * r2) / (2.0 * d);
            let h = (r1 * r1 - x * x).max(0.0).sqrt();
            let u = (c2 - c1) / d;
            let m = c1 + u * x;
            if h < 1e-9 {
                vec![m]
            } else {
                vec![m + u.perp() * h, m - u.perp() * h]
            }
        }
    }
}

/// Is a point of the carrier inside the curve's own span?
pub fn on_span(s: &Shape, p: DVec2, tol: f64) -> bool {
    match *s {
        Shape::Line { a, b } => {
            let t = s.param_of(p);
            let l = (b - a).length().max(EPS);
            t >= -tol / l && t <= 1.0 + tol / l
        }
        Shape::Arc { r, a0, a1, c } => {
            let t = angle_from(c, p, a0);
            let slack = tol / r.max(EPS);
            t <= a1 + slack || t >= a0 + TAU - slack
        }
        Shape::Circle { .. } => true,
    }
}

/// Where two curves cross or touch, within their spans.
pub fn intersections(a: &Shape, b: &Shape) -> Vec<DVec2> {
    carrier_hits(a, b)
        .into_iter()
        .filter(|p| on_span(a, *p, 1e-7) && on_span(b, *p, 1e-7))
        .collect()
}

/// Intersections of `a`'s carrier with `b`'s span (for Extend).
pub fn carrier_intersections(a: &Shape, b: &Shape) -> Vec<DVec2> {
    carrier_hits(a, b).into_iter().filter(|p| on_span(b, *p, 1e-7)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn v(x: f64, y: f64) -> DVec2 {
        DVec2::new(x, y)
    }
    #[test]
    fn line_and_circle_meet_twice_and_arcs_keep_their_span() {
        let l = Shape::Line { a: v(-10., 0.), b: v(10., 0.) };
        let c = Shape::Circle { c: v(0., 0.), r: 5. };
        let mut hits = intersections(&l, &c);
        hits.sort_by(|a, b| a.x.total_cmp(&b.x));
        assert!((hits[0] - v(-5., 0.)).length() < 1e-9 && (hits[1] - v(5., 0.)).length() < 1e-9);
        let upper = arc(v(0., 0.), 5., 0.1, PI - 0.1);
        assert!(intersections(&l, &upper).is_empty());
        let bounds = arc(v(0., 0.), 5., -0.5, 0.5).bounds();
        assert!((bounds.1.x - 5.).abs() < 1e-12);
    }
    #[test]
    fn three_points_make_the_arc_through_them() {
        let a = arc3(v(5., 0.), v(0., 5.), v(-5., 0.)).unwrap();
        let Shape::Arc { c, r, a0, a1 } = a else { panic!() };
        assert!(c.length() < 1e-9 && (r - 5.).abs() < 1e-9);
        assert!((a0 - 0.).abs() < 1e-9 && (a1 - PI).abs() < 1e-9);
        // The same points the other way round: clockwise is stored CCW.
        let b = arc3(v(-5., 0.), v(0., 5.), v(5., 0.)).unwrap();
        assert!((b.length() - a.length()).abs() < 1e-9);
        assert!(arc3(v(0., 0.), v(1., 1.), v(2., 2.)).is_none());
    }
    #[test]
    fn the_area_of_a_closed_boundary_is_exact() {
        // A 10 x 10 square with a half-disc on top, counter-clockwise.
        let parts = [
            Shape::Line { a: v(0., 0.), b: v(10., 0.) },
            Shape::Line { a: v(10., 0.), b: v(10., 10.) },
            arc(v(5., 10.), 5., 0., PI),
            Shape::Line { a: v(0., 10.), b: v(0., 0.) },
        ];
        let area: f64 = parts.iter().map(|s| s.area_term()).sum();
        assert!((area - (100. + PI * 12.5)).abs() < 1e-9);
    }
}
