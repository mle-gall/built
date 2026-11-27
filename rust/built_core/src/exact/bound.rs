// SPDX-License-Identifier: GPL-3.0-or-later
//! Exact supports bound to ordinary store elements (docs/exact-geometry.md
//! §3, lane 3). A drawn circle keeps its segment edges; each edge carries
//! the circle and its parameter interval, and each face carries the
//! surface its vertices lie on. The store polygons stay the conforming
//! model-tolerance mesh, so dropping a binding is always a valid fallback.
//!
//! Pcurves are implicit: a face's trims are its store edges, mapped into
//! the support's parameters by the analytic inverses below. An edge lies
//! on a support only if enough distinct points of the whole curve do
//! (Bezout: a degree-m curve meets a degree-n surface in at most mn points
//! unless it lies on it), so no tolerance is ever inflated.
use super::{Curve, Frame, Surface};
use glam::{DMat3, DQuat, DVec2, DVec3};
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};

/// A curve with the parameters at the store edge's first and second end.
/// The interval may decrease: it records the edge's sense on the curve.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct EdgeBind {
    pub curve: Curve,
    pub interval: [f64; 2],
}

/// A surface; extrusions and revolutions name their generator as curve 0.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Support {
    pub surface: Surface,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<Curve>,
}

/// Roundoff allowed when deciding whether a stored point lies on a support,
/// relative to the coordinate scale of the definition.
pub(crate) fn tolerance(scale: f64) -> f64 {
    1e-9 * (1. + scale)
}

fn unit(v: DVec3) -> Option<DVec3> {
    v.try_normalize()
}

fn frame(origin: DVec3, axis: DVec3, reference: DVec3) -> Option<Frame> {
    let axis = unit(axis)?;
    let reference = unit(reference - axis * reference.dot(axis))?;
    let f = Frame {
        origin,
        axis,
        reference,
    };
    f.validate().ok().map(|_| f)
}

fn wrap(t: f64) -> f64 {
    let r = t.rem_euclid(TAU);
    if r > PI {
        r - TAU
    } else {
        r
    }
}

impl Curve {
    pub(crate) fn periodic(&self) -> bool {
        matches!(self, Self::Circle { .. } | Self::Ellipse { .. })
    }

    /// Polynomial degree of the point set (2 for conics).
    fn degree(&self) -> usize {
        match self {
            Self::Line { .. } => 1,
            Self::Circle { .. } | Self::Ellipse { .. } => 2,
            Self::Spline { axis, .. } => axis.degree.max(1),
        }
    }

    /// Characteristic size for display sampling and tolerances.
    pub(crate) fn size(&self) -> f64 {
        match self {
            Self::Line { .. } => 0.,
            Self::Circle { radius, .. } => *radius,
            Self::Ellipse { major, .. } => *major,
            Self::Spline { controls, .. } => controls
                .iter()
                .map(|c| c.truncate().length())
                .fold(0., f64::max),
        }
    }

    /// Parameter of the curve point closest to `p` (in `(-π, π]` for
    /// periodic curves). Splines have no analytic inverse here.
    pub(crate) fn param(&self, p: DVec3) -> Option<f64> {
        match self {
            Self::Line { origin, direction } => Some((p - *origin).dot(*direction)),
            Self::Circle { frame, .. } => {
                let d = p - frame.origin;
                Some(d.dot(frame.y()).atan2(d.dot(frame.reference)))
            }
            Self::Ellipse {
                frame,
                major,
                minor,
            } => {
                let d = p - frame.origin;
                Some((d.dot(frame.y()) / minor).atan2(d.dot(frame.reference) / major))
            }
            Self::Spline { .. } => None,
        }
    }

    pub(crate) fn contains(&self, p: DVec3, tol: f64) -> bool {
        self.param(p)
            .and_then(|t| self.point(t).ok())
            .is_some_and(|q| q.distance(p) <= tol)
    }

    /// The interval from `a` to `b`. A periodic curve takes the branch of
    /// `hint` (an interval this edge came from) when both ends lie on it,
    /// otherwise the shorter arc.
    pub(crate) fn interval(&self, a: DVec3, b: DVec3, hint: Option<[f64; 2]>) -> Option<[f64; 2]> {
        let (ta, tb) = (self.param(a)?, self.param(b)?);
        if !self.periodic() {
            return (ta != tb).then_some([ta, tb]);
        }
        if let Some([h0, h1]) = hint {
            let (lo, hi) = (h0.min(h1), h0.max(h1));
            let eps = 1e-9 * (1. + hi.abs());
            let lift = |t: f64| {
                let k = ((lo - t) / TAU).ceil();
                let u = t + k * TAU;
                (u <= hi + eps).then_some(u).or_else(|| {
                    // Roundoff just below the low end.
                    let w = u - TAU;
                    (w >= lo - eps).then_some(w)
                })
            };
            if let (Some(x), Some(y)) = (lift(ta), lift(tb)) {
                if x != y && (y - x).signum() == (h1 - h0).signum() {
                    return Some([x, y]);
                }
            }
        }
        let d = wrap(tb - ta);
        (d != 0.).then_some([ta, ta + d])
    }



    /// The image under `p ↦ m p + t`, or None if it collapses. A circle
    /// whose plane is not mapped by a similarity becomes an ellipse.
    pub(crate) fn mapped(&self, m: DMat3, t: DVec3) -> Option<Self> {
        let point = |p: DVec3| m * p + t;
        Some(match self {
            Self::Line { origin, direction } => Self::Line {
                origin: point(*origin),
                direction: unit(m * *direction)?,
            },
            Self::Circle { frame, radius } => {
                conic(point(frame.origin), m * frame.reference * *radius, m * frame.y() * *radius)?
            }
            Self::Ellipse {
                frame,
                major,
                minor,
            } => conic(
                point(frame.origin),
                m * frame.reference * *major,
                m * frame.y() * *minor,
            )?,
            Self::Spline { .. } => return None,
        })
    }

    /// The plane a conic lies in.
    pub(crate) fn plane(&self) -> Option<&Frame> {
        match self {
            Self::Circle { frame, .. } | Self::Ellipse { frame, .. } => Some(frame),
            _ => None,
        }
    }
}

/// The conic `c + cos(t) a + sin(t) b`, with principal axes. Circles stay
/// circles when `a` and `b` are orthogonal of equal length.
pub(crate) fn conic(c: DVec3, a: DVec3, b: DVec3) -> Option<Curve> {
    let (la, lb) = (a.length(), b.length());
    let axis = unit(a.cross(b))?;
    let scale = la.max(lb);
    if (la - lb).abs() <= 1e-12 * scale && a.dot(b).abs() <= 1e-12 * scale * scale {
        let f = frame(c, axis, a)?;
        return Some(Curve::Circle {
            frame: f,
            radius: 0.5 * (la + lb),
        });
    }
    // Principal directions: maximize |cos θ a + sin θ b|.
    let theta = 0.5 * (2. * a.dot(b)).atan2(la * la - lb * lb);
    let major_dir = a * theta.cos() + b * theta.sin();
    let minor_dir = b * theta.cos() - a * theta.sin();
    let (major, minor) = (major_dir.length(), minor_dir.length());
    let f = frame(c, axis, major_dir)?;
    if major < minor {
        let f = frame(c, axis, minor_dir)?;
        return Some(Curve::Ellipse {
            frame: f,
            major: minor,
            minor: major,
        });
    }
    Some(Curve::Ellipse {
        frame: f,
        major,
        minor,
    })
}

impl Support {
    /// A pool surface as a self-contained support (its generator curve,
    /// if any, carried along as curve 0).
    pub(crate) fn from_pool(curves: &[Curve], surface: &Surface) -> Option<Self> {
        Some(match surface {
            Surface::Extrusion { generator, vector } => Self {
                surface: Surface::Extrusion { generator: 0, vector: *vector },
                generator: Some(curves.get(*generator as usize)?.clone()),
            },
            Surface::Revolution { generator, origin, axis } => Self {
                surface: Surface::Revolution { generator: 0, origin: *origin, axis: *axis },
                generator: Some(curves.get(*generator as usize)?.clone()),
            },
            other => Self::new(other.clone()),
        })
    }

    pub(crate) fn new(surface: Surface) -> Self {
        Self {
            surface,
            generator: None,
        }
    }
    fn curves(&self) -> &[Curve] {
        self.generator.as_slice()
    }
    pub(crate) fn validate(&self) -> bool {
        self.surface.validate(self.curves()).is_ok()
    }
    pub(crate) fn point(&self, uv: DVec2) -> Option<DVec3> {
        self.surface.point(self.curves(), uv.x, uv.y).ok()
    }

    /// Which parameters wrap with period 2π.
    pub(crate) fn periodic(&self) -> [bool; 2] {
        match &self.surface {
            Surface::Plane { .. } => [false, false],
            Surface::Cylinder { .. } | Surface::Cone { .. } | Surface::Sphere { .. } => {
                [true, false]
            }
            Surface::Torus { .. } => [true, true],
            Surface::Extrusion { .. } => [self.generator.as_ref().is_some_and(Curve::periodic), false],
            Surface::Revolution { .. } => {
                [self.generator.as_ref().is_some_and(Curve::periodic), true]
            }
            Surface::Spline { .. } => [false, false],
        }
    }

    /// Algebraic degree of the point set (bounds curve intersections).
    fn degree(&self) -> usize {
        let g = || self.generator.as_ref().map_or(4, Curve::degree);
        match &self.surface {
            Surface::Plane { .. } => 1,
            Surface::Cylinder { .. } | Surface::Cone { .. } | Surface::Sphere { .. } => 2,
            Surface::Torus { .. } => 4,
            Surface::Extrusion { .. } => g(),
            Surface::Revolution { .. } => 2 * g(),
            Surface::Spline { .. } => 8,
        }
    }

    /// True where the parameterization degenerates (cone apex, sphere pole):
    /// u is undefined there.
    pub(crate) fn singular(&self, p: DVec3, tol: f64) -> bool {
        match &self.surface {
            Surface::Cone {
                frame,
                radius,
                half_angle,
            } => {
                let d = p - frame.origin;
                let v = d.dot(frame.axis) / half_angle.cos();
                (radius + v * half_angle.sin()).abs() <= tol
            }
            Surface::Sphere { frame, .. } => {
                let d = p - frame.origin;
                (d - frame.axis * d.dot(frame.axis)).length() <= tol
            }
            Surface::Revolution { origin, axis, .. } => {
                let d = p - *origin;
                (d - *axis * d.dot(*axis)).length() <= tol
            }
            _ => false,
        }
    }

    /// Analytic inverse: the parameters of the support point nearest `p`
    /// (exact for points on it). Periodic parameters are in `(-π, π]`.
    pub(crate) fn uv(&self, p: DVec3) -> Option<DVec2> {
        let angle = |f: &Frame, d: DVec3| d.dot(f.y()).atan2(d.dot(f.reference));
        Some(match &self.surface {
            Surface::Plane { frame } => {
                let d = p - frame.origin;
                DVec2::new(d.dot(frame.reference), d.dot(frame.y()))
            }
            Surface::Cylinder { frame, .. } => {
                let d = p - frame.origin;
                DVec2::new(angle(frame, d), d.dot(frame.axis))
            }
            Surface::Cone {
                frame,
                radius,
                half_angle,
            } => {
                let d = p - frame.origin;
                let v = d.dot(frame.axis) / half_angle.cos();
                // Below the apex the radial direction turns over.
                let d = if radius + v * half_angle.sin() < 0. { -d } else { d };
                DVec2::new(angle(frame, d), v)
            }
            Surface::Sphere { frame, .. } => {
                let d = p - frame.origin;
                let r = d.length();
                if r == 0. {
                    return None;
                }
                DVec2::new(angle(frame, d), (d.dot(frame.axis) / r).clamp(-1., 1.).asin())
            }
            Surface::Torus { frame, major, .. } => {
                let d = p - frame.origin;
                let (x, y) = (d.dot(frame.reference), d.dot(frame.y()));
                let radial = x.hypot(y) - major;
                DVec2::new(y.atan2(x), d.dot(frame.axis).atan2(radial))
            }
            Surface::Extrusion { vector, .. } => {
                let c = self.generator.as_ref()?;
                match c {
                    Curve::Line { origin, direction } => {
                        // p - o = u d + v V: least squares in the plane.
                        let w = p - *origin;
                        let (dd, dv, vv) = (1., direction.dot(*vector), vector.dot(*vector));
                        let det = dd * vv - dv * dv;
                        if det.abs() <= 1e-15 * vv {
                            return None;
                        }
                        let (a, b) = (w.dot(*direction), w.dot(*vector));
                        DVec2::new((a * vv - b * dv) / det, (b * dd - a * dv) / det)
                    }
                    _ => {
                        let f = c.plane()?;
                        let den = vector.dot(f.axis);
                        if den.abs() <= 1e-12 * vector.length() {
                            return None;
                        }
                        let v = (p - f.origin).dot(f.axis) / den;
                        DVec2::new(c.param(p - *vector * v)?, v)
                    }
                }
            }
            Surface::Revolution { origin, axis, .. } => {
                let c = self.generator.as_ref()?;
                let radial = |q: DVec3| {
                    let d = q - *origin;
                    d - *axis * d.dot(*axis)
                };
                let reference = [0., 1., -1., 0.5]
                    .into_iter()
                    .filter_map(|t| c.point(t).ok().map(radial))
                    .find(|r| r.length() > 1e-9 * (1. + c.size()))?
                    .normalize();
                let rp = radial(p);
                let v = reference.cross(rp).dot(*axis).atan2(reference.dot(rp));
                let q = *origin + DQuat::from_axis_angle(*axis, -v) * (p - *origin);
                DVec2::new(c.param(q)?, v)
            }
            Surface::Spline { .. } => return None,
        })
    }

    pub(crate) fn contains(&self, p: DVec3, tol: f64) -> bool {
        if self.singular(p, tol) {
            // Every parameterization reaches the apex or pole; the distance
            // test below would pick an arbitrary u.
            return match &self.surface {
                Surface::Sphere { frame, radius } => {
                    ((p - frame.origin).length() - radius).abs() <= tol
                }
                _ => true,
            };
        }
        self.uv(p)
            .and_then(|uv| self.point(uv))
            .is_some_and(|q| q.distance(p) <= tol)
    }

    /// Whether the whole curve lies on this support.
    pub(crate) fn holds_curve(&self, c: &Curve, interval: [f64; 2], tol: f64) -> bool {
        let n = c.degree() * self.degree() + 2;
        let (a, b) = if c.periodic() {
            (0., TAU * (n as f64 - 1.) / n as f64)
        } else {
            (interval[0], interval[1])
        };
        (0..n).all(|i| {
            let t = a + (b - a) * i as f64 / (n - 1) as f64;
            c.point(t).is_ok_and(|p| self.contains(p, tol))
        })
    }

    /// Whether the straight segment lies on this support.
    pub(crate) fn holds_segment(&self, a: DVec3, b: DVec3, tol: f64) -> bool {
        let n = self.degree() + 2;
        (0..n).all(|i| self.contains(a.lerp(b, i as f64 / (n - 1) as f64), tol))
    }

    /// The image under `p ↦ m p + t`, if it is again a supported surface.
    pub(crate) fn mapped(&self, m: DMat3, t: DVec3) -> Option<Self> {
        let point = |p: DVec3| m * p + t;
        let similarity = similarity(m);
        let out = match &self.surface {
            Surface::Plane { frame } => Self::new(Surface::Plane {
                frame: frame.mapped(point).ok()?,
            }),
            Surface::Cylinder { frame, radius } => {
                let section = Curve::Circle {
                    frame: frame.clone(),
                    radius: *radius,
                }
                .mapped(m, t)?;
                return extrusion(section, m * frame.axis);
            }
            Surface::Extrusion { vector, .. } => {
                return extrusion(self.generator.as_ref()?.mapped(m, t)?, m * *vector);
            }
            Surface::Cone {
                frame,
                radius,
                half_angle,
            } => {
                let s = similarity?;
                Self::new(Surface::Cone {
                    frame: frame.mapped(point).ok()?,
                    radius: radius * s,
                    half_angle: *half_angle,
                })
            }
            Surface::Sphere { frame, radius } => {
                let s = similarity?;
                Self::new(Surface::Sphere {
                    frame: frame.mapped(point).ok()?,
                    radius: radius * s,
                })
            }
            Surface::Torus {
                frame,
                major,
                minor,
            } => {
                let s = similarity?;
                Self::new(Surface::Torus {
                    frame: frame.mapped(point).ok()?,
                    major: major * s,
                    minor: minor * s,
                })
            }
            Surface::Revolution { origin, axis, .. } => {
                similarity?;
                let flip = if m.determinant() < 0. { -1. } else { 1. };
                Self {
                    surface: Surface::Revolution {
                        generator: 0,
                        origin: point(*origin),
                        axis: unit(m * *axis)? * flip,
                    },
                    generator: Some(self.generator.as_ref()?.mapped(m, t)?),
                }
            }
            Surface::Spline { .. } => return None,
        };
        out.validate().then_some(out).map(canonical)
    }
}

/// Uniform scale factor of `m` if it is a similarity.
pub(crate) fn similarity(m: DMat3) -> Option<f64> {
    let g = m.transpose() * m;
    let s2 = (g.x_axis.x + g.y_axis.y + g.z_axis.z) / 3.;
    let off = (g - DMat3::from_diagonal(DVec3::splat(s2)))
        .to_cols_array()
        .iter()
        .fold(0f64, |a, &b| a.max(b.abs()));
    (s2 > 0. && off <= 1e-10 * s2).then(|| s2.sqrt())
}

/// A curve swept along a direction: a cylinder when its right section is a
/// circle, otherwise a linear extrusion of the curve.
pub(crate) fn extrusion(c: Curve, direction: DVec3) -> Option<Support> {
    let d = unit(direction)?;
    if let Curve::Line {
        origin,
        direction: l,
    } = &c
    {
        return Some(canonical(Support::new(Surface::Plane {
            frame: frame(*origin, l.cross(d), *l)?,
        })));
    }
    let (center, a, b) = match &c {
        Curve::Circle { frame, radius } => (frame.origin, frame.reference * *radius, frame.y() * *radius),
        Curve::Ellipse {
            frame,
            major,
            minor,
        } => (frame.origin, frame.reference * *major, frame.y() * *minor),
        _ => return None,
    };
    if a.cross(b).dot(d).abs() <= 1e-12 * a.length() * b.length() {
        return None; // swept within its own plane
    }
    // Right section: project the conic along d onto the plane normal to d.
    let flat = |v: DVec3| v - d * v.dot(d);
    if let Some(Curve::Circle { frame: f, radius }) = conic(center, flat(a), flat(b)) {
        // Orient the cylinder axis with the sweep.
        let f = Frame {
            axis: d,
            reference: f.reference,
            origin: f.origin,
        };
        let f = frame(f.origin, f.axis, f.reference)?;
        return Some(canonical(Support::new(Surface::Cylinder { frame: f, radius })));
    }
    let s = Support {
        surface: Surface::Extrusion {
            generator: 0,
            vector: d,
        },
        generator: Some(c),
    };
    s.validate().then_some(s)
}

/// One representation per geometric support where it is cheap: a cylinder's
/// frame origin is the axis point nearest the definition origin, so walls
/// pushed in several steps share it.
/// Roundoff left by a fitted map: components far below the vector's own
/// magnitude become zero, so one support has one representation.
fn clean(v: DVec3) -> DVec3 {
    let m = v.abs().max_element();
    let c = |x: f64| if x.abs() <= 1e-14 * m { 0. } else { x };
    DVec3::new(c(v.x), c(v.y), c(v.z))
}
fn clean_point(v: DVec3) -> DVec3 {
    let m = 1. + v.abs().max_element();
    let c = |x: f64| if x.abs() <= 1e-14 * m { 0. } else { x };
    DVec3::new(c(v.x), c(v.y), c(v.z))
}
fn clean_dir(v: DVec3) -> DVec3 {
    clean(v).try_normalize().unwrap_or(v)
}
fn clean_len(x: f64) -> f64 {
    // 52-bit values that differ only in the last few bits are one length.
    let bits = x.to_bits() & !0xf;
    f64::from_bits(bits + if x.to_bits() & 0x8 != 0 { 0x10 } else { 0 })
}

/// Two supports equal up to roundoff in every number.
pub(crate) fn same_support(a: &Support, b: &Support) -> bool {
    a == b || approx(&serde_json::to_value(a).ok(), &serde_json::to_value(b).ok())
}
pub(crate) fn same_curve(a: &Curve, b: &Curve) -> bool {
    a == b || approx(&serde_json::to_value(a).ok(), &serde_json::to_value(b).ok())
}
fn approx(a: &Option<serde_json::Value>, b: &Option<serde_json::Value>) -> bool {
    use serde_json::Value;
    fn eq(a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Number(x), Value::Number(y)) => {
                let (x, y) = (x.as_f64().unwrap_or(f64::NAN), y.as_f64().unwrap_or(f64::NAN));
                (x - y).abs() <= 1e-12 * (1. + x.abs().max(y.abs()))
            }
            (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(x, y)| eq(x, y)),
            (Value::Object(x), Value::Object(y)) => {
                x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| eq(v, w)))
            }
            _ => a == b,
        }
    }
    match (a, b) {
        (Some(a), Some(b)) => eq(a, b),
        _ => false,
    }
}

pub(crate) fn canonical(s: Support) -> Support {
    // Parameter zero on a rotational support is a fixed direction normal
    // to its axis, so every facet of one surface carries the same row.
    let at = |origin: DVec3, axis: DVec3| {
        let axis = clean_dir(axis);
        Frame {
            origin: clean_point(origin),
            axis,
            reference: clean_dir(axis.any_orthonormal_vector()),
        }
    };
    let s = match s.surface {
        Surface::Cylinder { frame, radius } => Support::new(Surface::Cylinder { frame, radius: clean_len(radius) }),
        Surface::Cone { frame, radius, half_angle } => Support::new(Surface::Cone {
            frame,
            radius: clean_len(radius),
            half_angle: clean_len(half_angle),
        }),
        Surface::Sphere { frame, radius } => Support::new(Surface::Sphere { frame, radius: clean_len(radius) }),
        Surface::Torus { frame, major, minor } => Support::new(Surface::Torus {
            frame,
            major: clean_len(major),
            minor: clean_len(minor),
        }),
        other => Support { surface: other, generator: s.generator },
    };
    match s.surface {
        Surface::Cylinder { frame, radius } => {
            let o = frame.origin - frame.axis * frame.origin.dot(frame.axis);
            Support::new(Surface::Cylinder {
                frame: at(o, frame.axis),
                radius,
            })
        }
        Surface::Plane { frame } => Support::new(Surface::Plane {
            frame: at(frame.axis * frame.origin.dot(frame.axis), frame.axis),
        }),
        Surface::Cone {
            frame,
            radius,
            half_angle,
        } => Support::new(Surface::Cone {
            frame: at(frame.origin, frame.axis),
            radius,
            half_angle,
        }),
        Surface::Sphere { frame, radius } => Support::new(Surface::Sphere {
            frame: at(frame.origin, frame.axis),
            radius,
        }),
        Surface::Torus {
            frame,
            major,
            minor,
        } => Support::new(Surface::Torus {
            frame: at(frame.origin, frame.axis),
            major,
            minor,
        }),
        other => Support {
            surface: other,
            generator: s.generator,
        },
    }
}

/// The circle through `points`, if every point lies on it within `tol`.
/// The reference direction points at the first point.
pub(crate) fn fit_circle(points: &[DVec3], tol: f64) -> Option<Curve> {
    let n = points.len();
    if n < 3 {
        return None;
    }
    let (a, b, c) = (points[0], points[n / 3], points[(2 * n) / 3]);
    let (ab, ac) = (b - a, c - a);
    let normal = ab.cross(ac);
    let nn = normal.length_squared();
    if nn <= 1e-24 * ab.length_squared() * ac.length_squared() {
        return None;
    }
    let center = a + (normal.cross(ab) * ac.length_squared() + ac.cross(normal) * ab.length_squared())
        / (2. * nn);
    let radius = (a - center).length();
    let axis = normal / nn.sqrt();
    let f = frame(center, axis, a - center)?;
    let ok = points.iter().all(|&p| {
        let d = p - center;
        d.dot(axis).abs() <= tol && (d.length() - radius).abs() <= tol
    });
    ok.then_some(Curve::Circle { frame: f, radius })
}

/// The section of a cylinder by a plane: a circle or an ellipse (a plane
/// parallel to the axis cuts straight generators, which need no binding).
pub(crate) fn plane_section(s: &Support, plane: &Frame) -> Option<Curve> {
    let Surface::Cylinder { frame, radius } = &s.surface else {
        return None;
    };
    let (n, a) = (plane.axis, frame.axis);
    let na = n.dot(a);
    if na.abs() <= 1e-9 {
        return None;
    }
    let center = frame.origin + a * (plane.origin - frame.origin).dot(n) / na;
    if (1. - na.abs()) <= 1e-12 {
        return Some(Curve::Circle {
            frame: Frame {
                origin: center,
                ..frame.clone()
            },
            radius: *radius,
        });
    }
    let e2 = unit(n.cross(a))?;
    let e1 = a.cross(e2);
    let w = e1 - a * (n.dot(e1) / na);
    conic(center, w * *radius, e2 * *radius)
}

/// Revolve a profile curve about an axis line: the elementary surface when
/// one exists (cylinder, cone, plane, sphere, torus), else a revolution.
pub(crate) fn revolve(c: &Curve, origin: DVec3, axis: DVec3) -> Option<Support> {
    let axis = unit(axis)?;
    let radial = |p: DVec3| {
        let d = p - origin;
        d - axis * d.dot(axis)
    };
    let s = match c {
        Curve::Line {
            origin: o,
            direction,
        } => {
            let along = direction.dot(axis);
            let r0 = radial(*o);
            if direction.cross(axis).length() <= 1e-12 {
                let reference = unit(r0)?;
                Support::new(Surface::Cylinder {
                    frame: frame(origin, axis, reference)?,
                    radius: r0.length(),
                })
            } else {
                // The line must meet the axis plane through it (a meridian).
                let normal = unit(axis.cross(r0)).or_else(|| unit(axis.cross(*direction)))?;
                if direction.dot(normal).abs() > 1e-12 {
                    return None;
                }
                if along.abs() <= 1e-12 {
                    let center = origin + axis * (*o - origin).dot(axis);
                    Support::new(Surface::Plane {
                        frame: frame(center, axis, unit(radial(*o)).or_else(|| unit(*direction))?)?,
                    })
                } else {
                    // Apex where the line meets the axis.
                    let rd = radial(*direction);
                    let sign = if r0.dot(rd) < 0. { -1. } else { 1. };
                    let t = -sign * r0.length() / rd.length().max(f64::MIN_POSITIVE);
                    let apex = *o + *direction * t;
                    // Open the cone towards the profile.
                    let dir = unit(*o - apex)
                        .or_else(|| unit(*o + *direction - apex))?;
                    let up = if dir.dot(axis) < 0. { -axis } else { axis };
                    let half = dir.cross(up).length().atan2(dir.dot(up));
                    let reference = unit(radial(apex + dir))?;
                    Support::new(Surface::Cone {
                        frame: frame(apex - radial(apex), up, reference)?,
                        radius: 0.,
                        half_angle: half,
                    })
                }
            }
        }
        Curve::Circle { frame: f, radius } => {
            let rc = radial(f.origin);
            if rc.length() <= 1e-12 * (1. + radius) && f.axis.dot(axis).abs() <= 1e-12 {
                Support::new(Surface::Sphere {
                    frame: frame(f.origin, axis, f.reference - axis * f.reference.dot(axis))
                        .or_else(|| frame(f.origin, axis, f.y()))?,
                    radius: *radius,
                })
            } else if f.axis.cross(axis).length() <= 1e-12 {
                // A circle about the axis itself sweeps nothing new.
                return None;
            } else if f.axis.dot(axis).abs() <= 1e-12
                && (f.origin - origin).dot(f.axis).abs() <= 1e-12 * (1. + rc.length())
            {
                let center = origin + axis * (f.origin - origin).dot(axis);
                Support::new(Surface::Torus {
                    frame: frame(center, axis, unit(rc)?)?,
                    major: rc.length(),
                    minor: *radius,
                })
            } else {
                Support {
                    surface: Surface::Revolution {
                        generator: 0,
                        origin,
                        axis,
                    },
                    generator: Some(c.clone()),
                }
            }
        }
        _ => Support {
            surface: Surface::Revolution {
                generator: 0,
                origin,
                axis,
            },
            generator: Some(c.clone()),
        },
    };
    s.validate().then(|| canonical(s))
}

/// Best affine map with `m p + t ≈ q` on all pairs, verified within `tol`.
/// Coplanar points fix the map only in their plane: the normal then maps to
/// the normal of the image plane, which is all a curve or plane needs.
pub(crate) fn fit_affine(pairs: &[(DVec3, DVec3)], tol: f64) -> Option<(DMat3, DVec3)> {
    let n = pairs.len();
    if n == 0 {
        return None;
    }
    let pc = pairs.iter().map(|p| p.0).sum::<DVec3>() / n as f64;
    let qc = pairs.iter().map(|p| p.1).sum::<DVec3>() / n as f64;
    if pairs.iter().all(|(p, q)| (*q - *p - (qc - pc)).length() <= tol) {
        return Some((DMat3::IDENTITY, qc - pc));
    }
    let mut ppt = DMat3::ZERO;
    let mut qpt = DMat3::ZERO;
    for (p, q) in pairs {
        let (dp, dq) = (*p - pc, *q - qc);
        ppt += outer(dp, dp);
        qpt += outer(dq, dp);
    }
    let scale = ppt.to_cols_array().iter().fold(0f64, |a, &b| a.max(b.abs()));
    if scale == 0. {
        return None;
    }
    let m = if ppt.determinant().abs() > 1e-9 * scale.powi(3) {
        qpt * ppt.inverse()
    } else {
        // Rank 2: complete with the plane normal.
        let (e1, e2, normal) = basis(pairs.iter().map(|p| p.0 - pc))?;
        let to_plane = |v: DVec3| DVec2::new(v.dot(e1), v.dot(e2));
        let mut aa = [[0f64; 2]; 2];
        let mut bq = [DVec3::ZERO; 2];
        for (p, q) in pairs {
            let x = to_plane(*p - pc);
            let dq = *q - qc;
            for i in 0..2 {
                for j in 0..2 {
                    aa[i][j] += x[i] * x[j];
                }
                bq[i] += dq * x[i];
            }
        }
        let det = aa[0][0] * aa[1][1] - aa[0][1] * aa[1][0];
        if det.abs() <= 1e-18 * scale * scale {
            return None;
        }
        let inv = [
            [aa[1][1] / det, -aa[0][1] / det],
            [-aa[1][0] / det, aa[0][0] / det],
        ];
        let m1 = bq[0] * inv[0][0] + bq[1] * inv[1][0];
        let m2 = bq[0] * inv[0][1] + bq[1] * inv[1][1];
        let image_normal = m1.cross(m2);
        let len = (m1.length() * m2.length()).sqrt();
        let mn = unit(image_normal)? * len;
        // Columns in world coordinates: M = [m1 m2 mn] · [e1 e2 n]ᵀ.
        DMat3::from_cols(m1, m2, mn) * DMat3::from_cols(e1, e2, normal).transpose()
    };
    let t = qc - m * pc;
    pairs
        .iter()
        .all(|(p, q)| (m * *p + t - *q).length() <= tol)
        .then_some((m, t))
}

fn outer(a: DVec3, b: DVec3) -> DMat3 {
    DMat3::from_cols(a * b.x, a * b.y, a * b.z)
}

fn basis(points: impl Iterator<Item = DVec3> + Clone) -> Option<(DVec3, DVec3, DVec3)> {
    let first = points.clone().max_by(|a, b| a.length().total_cmp(&b.length()))?;
    let e1 = unit(first)?;
    let second = points
        .map(|p| p - e1 * p.dot(e1))
        .max_by(|a, b| a.length().total_cmp(&b.length()))?;
    let e2 = unit(second)?;
    Some((e1, e2, e1.cross(e2)))
}

impl Support {
    pub(crate) fn derivatives_at(&self, uv: DVec2) -> Option<(DVec3, DVec3)> {
        self.derivatives(uv)
    }

    /// A length scale of the support (radius-like), for pick tolerances.
    pub(crate) fn size_hint(&self) -> f64 {
        match &self.surface {
            Surface::Cylinder { radius, .. } | Surface::Sphere { radius, .. } | Surface::Cone { radius, .. } => radius.max(1e-3),
            Surface::Torus { major, minor, .. } => major + minor,
            _ => self.generator.as_ref().map_or(1., |c| c.size().max(1e-3)),
        }
    }

    fn derivatives(&self, uv: DVec2) -> Option<(DVec3, DVec3)> {
        let d = |axis: DVec2| {
            let h = 1e-6 * (1. + uv.dot(axis).abs());
            Some((self.point(uv + axis * h)? - self.point(uv - axis * h)?) / (2. * h))
        };
        Some((d(DVec2::X)?, d(DVec2::Y)?))
    }

    /// The unit normal along `du × dv`, None at a singular point.
    pub(crate) fn normal(&self, uv: DVec2) -> Option<DVec3> {
        let (du, dv) = self.derivatives(uv)?;
        du.cross(dv).try_normalize()
    }
}

/// Validate editable facet trims in the support's unwrapped parameter space.
pub(crate) fn trim_region(s: &Support, rings: &[Vec<DVec3>], tol: f64) -> Option<Vec<Vec<DVec2>>> {
    let periodic = s.periodic();
    let mut uv_rings: Vec<Vec<DVec2>> = Vec::new();
    let mut previous: Option<DVec2> = None;
    for ring in rings {
        let raw: Vec<Option<DVec2>> = ring
            .iter()
            .map(|&p| (!s.singular(p, tol)).then(|| s.uv(p)).flatten())
            .collect();
        let n = ring.len();
        let start = raw.iter().position(Option::is_some)?;
        // Unwrap periodic parameters along the ring, starting near the
        // previous ring so holes and outer loop share one branch.
        let mut last = raw[start].unwrap();
        if let Some(prev) = previous {
            for k in 0..2 {
                if periodic[k] {
                    last[k] += ((prev[k] - last[k]) / TAU).round() * TAU;
                }
            }
        }
        let mut lifted: Vec<Option<DVec2>> = vec![None; n];
        for step in 0..=n {
            let i = (start + step) % n;
            if let Some(mut uv) = raw[i] {
                for k in 0..2 {
                    if periodic[k] {
                        uv[k] = last[k] + wrap(uv[k] - last[k]);
                    }
                }
                if step == n {
                    // The ring must close on the branch it started on.
                    if uv.distance(lifted[start]?) > 1e-6 {
                        return None;
                    }
                    break;
                }
                lifted[i] = Some(uv);
                last = uv;
            }
        }
        let mut uv_ring = Vec::new();
        for i in 0..n {
            match lifted[i] {
                Some(uv) => {
                    uv_ring.push(uv);
                }
                None => {
                    // A pole: one model point, a parameter side from its
                    // neighbours' u to theirs.
                    let before = (1..n).find_map(|k| lifted[(i + n - k) % n])?;
                    let after = (1..n).find_map(|k| lifted[(i + k) % n])?;
                    let v = s.uv(ring[i]).map_or(before.y, |uv| uv.y);
                    for u in [before.x, after.x] {
                        let uv = DVec2::new(u, v);
                        if uv_ring.last() != Some(&uv) {
                            uv_ring.push(uv);
                        }
                    }
                }
            }
        }
        if uv_ring.len() < 3 {
            return None;
        }
        // A side along a parameter line (a rim, a rail, a generator) must
        // be exactly straight in parameters: inverse-map roundoff would
        // otherwise let the triangulation close slivers between its
        // samples, which in 3D stand across the surface.
        for k in 0..2 {
            let m = uv_ring.len();
            for i in 0..m {
                let (a, b) = (uv_ring[i][k], uv_ring[(i + 1) % m][k]);
                if a != b && (a - b).abs() <= 1e-9 * (1. + a.abs()) {
                    uv_ring[(i + 1) % m][k] = a;
                }
            }
        }
        previous = uv_ring.first().copied();
        uv_rings.push(uv_ring);
    }
    super::validate_polygon(&uv_rings).ok()?;
    Some(uv_rings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn z_frame() -> Frame {
        Frame {
            origin: DVec3::ZERO,
            axis: DVec3::Z,
            reference: DVec3::X,
        }
    }

    #[test]
    fn circle_fit_and_intervals() {
        let pts: Vec<_> = (0..24)
            .map(|i| {
                let t = TAU * i as f64 / 24.;
                DVec3::new(3. + 2. * t.cos(), 1. + 2. * t.sin(), 5.)
            })
            .collect();
        let c = fit_circle(&pts, 1e-9).unwrap();
        let i = c.interval(pts[23], pts[0], None).unwrap();
        assert!((i[1] - i[0] - TAU / 24.).abs() < 1e-12);
        // A split keeps the parent's branch.
        let half = c.interval(pts[0], pts[6], Some([0., TAU / 2.])).unwrap();
        assert!((half[1] - TAU / 4.).abs() < 1e-12, "{half:?}");
        assert!(fit_circle(&[DVec3::ZERO, DVec3::X, DVec3::X * 2.], 1e-9).is_none());
    }

    #[test]
    fn nonuniform_scale_turns_a_circle_into_an_ellipse() {
        let c = Curve::Circle {
            frame: z_frame(),
            radius: 1.,
        };
        let m = DMat3::from_diagonal(DVec3::new(2., 1., 1.));
        let Some(Curve::Ellipse { major, minor, .. }) = c.mapped(m, DVec3::ZERO) else {
            panic!()
        };
        assert!((major - 2.).abs() < 1e-12 && (minor - 1.).abs() < 1e-12);
        let cyl = Support::new(Surface::Cylinder {
            frame: z_frame(),
            radius: 1.,
        });
        // Scaling along the axis keeps a cylinder; across it, an extrusion.
        let tall = cyl
            .mapped(DMat3::from_diagonal(DVec3::new(1., 1., 3.)), DVec3::ZERO)
            .unwrap();
        assert!(matches!(tall.surface, Surface::Cylinder { radius, .. } if (radius - 1.).abs() < 1e-12));
        let wide = cyl.mapped(m, DVec3::ZERO).unwrap();
        assert!(matches!(wide.surface, Surface::Extrusion { .. }));
        assert!(wide.contains(DVec3::new(2., 0., 7.), 1e-9));
        assert!(wide.contains(DVec3::new(0., 1., -3.), 1e-9));
        assert!(!wide.contains(DVec3::new(1., 0., 0.), 1e-9));
    }

    #[test]
    fn sheared_cylinder_with_circular_right_section_stays_a_cylinder() {
        // Shear along the axis: x' = x, y' = y, z' = z + 0.5 x.
        let m = DMat3::from_cols(DVec3::new(1., 0., 0.5), DVec3::Y, DVec3::Z);
        let cyl = Support::new(Surface::Cylinder {
            frame: z_frame(),
            radius: 1.,
        });
        let out = cyl.mapped(m, DVec3::ZERO).unwrap();
        assert!(matches!(out.surface, Surface::Cylinder { .. }));
    }

    #[test]
    fn affine_fit_recovers_rotation_and_rejects_partial_moves() {
        let r = DMat3::from_rotation_z(0.7) * 2.;
        let pts = [DVec3::X, DVec3::Y, DVec3::Z, DVec3::ONE];
        let pairs: Vec<_> = pts.iter().map(|&p| (p, r * p + DVec3::X)).collect();
        let (m, t) = fit_affine(&pairs, 1e-9).unwrap();
        assert!((m - r).abs_diff_eq(DMat3::ZERO, 1e-12) || (m * DVec3::ONE - r * DVec3::ONE).length() < 1e-9);
        assert!((t - DVec3::X).length() < 1e-9);
        let mut bad = pairs.clone();
        bad[0].1 += DVec3::Z;
        bad.push((DVec3::new(2., 2., 2.), DVec3::new(2., 2., 2.)));
        assert!(fit_affine(&bad, 1e-9).is_none());
        // Coplanar points: in-plane map.
        let flat: Vec<_> = [DVec3::ZERO, DVec3::X, DVec3::Y]
            .iter()
            .map(|&p| (p, r * p))
            .collect();
        let (m, _) = fit_affine(&flat, 1e-9).unwrap();
        assert!((m * DVec3::X - r * DVec3::X).length() < 1e-9);
    }

    #[test]
    fn oblique_section_is_an_ellipse_on_the_cylinder() {
        let cyl = Support::new(Surface::Cylinder {
            frame: z_frame(),
            radius: 1.,
        });
        let plane = frame(DVec3::new(0., 0., 1.), DVec3::new(1., 0., 1.), DVec3::Y).unwrap();
        let e = plane_section(&cyl, &plane).unwrap();
        assert!(matches!(e, Curve::Ellipse { .. }));
        assert!(cyl.holds_curve(&e, [0., 1.], 1e-9));
    }

    #[test]
    fn revolutions_classify() {
        let axis = DVec3::Z;
        let line = Curve::Line {
            origin: DVec3::new(2., 0., 0.),
            direction: DVec3::Z,
        };
        assert!(matches!(revolve(&line, DVec3::ZERO, axis).unwrap().surface, Surface::Cylinder { .. }));
        let slanted = Curve::Line {
            origin: DVec3::new(2., 0., 0.),
            direction: DVec3::new(-1., 0., 1.).normalize(),
        };
        let cone = revolve(&slanted, DVec3::ZERO, axis).unwrap();
        assert!(matches!(cone.surface, Surface::Cone { .. }));
        assert!(cone.contains(DVec3::new(0., 1., 1.), 1e-9));
        assert!(cone.contains(DVec3::new(0., 0., 2.), 1e-9));
        let profile = Curve::Circle {
            frame: frame(DVec3::new(3., 0., 0.), DVec3::Y, DVec3::X).unwrap(),
            radius: 1.,
        };
        let torus = revolve(&profile, DVec3::ZERO, axis).unwrap();
        assert!(matches!(torus.surface, Surface::Torus { .. }));
        assert!(torus.contains(DVec3::new(0., 4., 0.), 1e-9));
        assert!(torus.holds_curve(&profile, [0., 1.], 1e-9));
    }

    #[test]
    fn crossing_trims_are_rejected() {
        let cyl = Support::new(Surface::Cylinder { frame: z_frame(), radius: 1. });
        let ring = [(0.,0.), (0.5,1.), (0.5,0.), (0.,1.)]
            .map(|(u,v)| cyl.point(DVec2::new(u,v)).unwrap());
        assert!(trim_region(&cyl, &[ring.to_vec()], 1e-9).is_none());
    }

    #[test]
    fn every_facet_of_a_near_axis_cylinder_has_a_region() {
        let cyl = Support::new(Surface::Cylinder {
            frame: Frame {
                origin: DVec3::new(-2.465190328815662e-32, -2.3408381773460992e-97, 7.596454196607839e-65),
                axis: DVec3::new(3.0814879110195774e-33, -3.0814879110195774e-33, 1.0),
                reference: DVec3::new(4.7477838728798994e-66, 1.0, 3.0814879110195774e-33),
            },
            radius: 2.,
        });
        for i in 0..24 {
            let p = |k: usize, z: f64| {
                let t = TAU * k as f64 / 24.;
                DVec3::new(2. * t.cos(), 2. * t.sin(), z)
            };
            let ring = vec![p(i, 0.), p(i + 1, 0.), p(i + 1, 4.), p(i, 4.)];
            assert!(trim_region(&cyl, &[ring], 4e-9).is_some(), "facet {i}");
        }
    }
}
