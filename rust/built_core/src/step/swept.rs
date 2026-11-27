// SPDX-License-Identifier: GPL-3.0-or-later
use super::{exact_read::Geometry, splines};
use crate::exact::{Axis, Curve, Surface};
use built_step::step_io::model as m;
use glam::{DVec2, DVec3};
type Result<T> = std::result::Result<T, &'static str>;
impl Geometry<'_> {
    pub(super) fn profile(&self, c: &m::CurveRef) -> Result<Curve> {
        let (degree, points, knots, mults, weights) = match c {
            m::CurveRef::Line(i) => {
                let l = &self.model[*i];
                let m::VectorRef::Vector(id) = l.dir else {
                    return Err("unresolved generator vector");
                };
                return Ok(Curve::Line {
                    origin: self.point(&l.pnt)?,
                    direction: self.direction(&self.model[id].orientation)?,
                });
            }
            m::CurveRef::Circle(i) => {
                let c = &self.model[*i];
                return Ok(Curve::Circle {
                    frame: self.conic_frame(&c.position)?,
                    radius: c.radius * self.metres,
                });
            }
            m::CurveRef::Ellipse(i) => {
                let c = &self.model[*i];
                return Ok(Curve::Ellipse {
                    frame: self.conic_frame(&c.position)?,
                    major: c.semi_axis_1 * self.metres,
                    minor: c.semi_axis_2 * self.metres,
                });
            }
            m::CurveRef::BSplineCurveWithKnots(i) => {
                let b = &self.model[*i];
                (
                    b.degree,
                    &b.control_points_list,
                    &b.knots,
                    &b.knot_multiplicities,
                    None,
                )
            }
            m::CurveRef::Complex(i) => {
                let mut base = None;
                let mut knots = None;
                let mut weights = None;
                for p in &self.model[*i].parts {
                    match p {
                        m::PartialEntity::BSplineCurve {
                            degree,
                            control_points_list,
                            ..
                        } => base = Some((*degree, control_points_list)),
                        m::PartialEntity::BSplineCurveWithKnots {
                            knots: k,
                            knot_multiplicities,
                            ..
                        } => knots = Some((k, knot_multiplicities)),
                        m::PartialEntity::RationalBSplineCurve { weights_data } => {
                            weights = Some(weights_data)
                        }
                        _ => (),
                    }
                }
                let (d, p) = base.ok_or("unresolved spline generator")?;
                let (k, m) = knots.ok_or("missing spline generator knots")?;
                (d, p, k, m, weights)
            }
            _ => return Err("unsupported sweep generator"),
        };
        if weights.is_some_and(|w| w.len() != points.len()) {
            return Err("invalid generator weights");
        }
        let axis = Axis {
            degree: degree.try_into().map_err(|_| "invalid generator degree")?,
            knots: knots.clone(),
            multiplicities: mults
                .iter()
                .map(|&v| v.try_into().map_err(|_| "invalid generator knot"))
                .collect::<Result<_>>()?,
            periodic: false,
        };
        axis.validate(points.len())?;
        let controls = points
            .iter()
            .enumerate()
            .map(|(i, p)| Ok(self.point(p)?.extend(weights.map_or(1., |w| w[i]))))
            .collect::<Result<_>>()?;
        Ok(Curve::Spline { axis, controls })
    }
}
fn parameter(c: &Curve, distance: impl Fn(DVec3) -> DVec3) -> Result<f64> {
    let [lo, hi] = match c {
        Curve::Circle { .. } | Curve::Ellipse { .. } => [0., std::f64::consts::TAU],
        Curve::Spline { axis, .. } => splines::domain(axis),
        _ => return Err("unbounded profile"),
    };
    let mut best = (f64::INFINITY, lo);
    for i in 0..=32 {
        let u = lo + (hi - lo) * i as f64 / 32.;
        let sq = distance(c.point(u)?).length_squared();
        if sq < best.0 {
            best = (sq, u);
        }
    }
    let mut u = best.1;
    for _ in 0..32 {
        let d = distance(c.point(u)?);
        if d.length() < 1e-13 {
            break;
        }
        let h = (hi - lo) * 1e-6;
        let a = (u - h).max(lo);
        let b = (u + h).min(hi);
        let slope = (distance(c.point(b)?) - distance(c.point(a)?)) / (b - a);
        let v = (u - d.dot(slope) / slope.length_squared()).clamp(lo, hi);
        if !v.is_finite() || v == u {
            break;
        }
        u = v;
    }
    Ok(u)
}
pub(super) fn uv(s: &Surface, p: DVec3, curves: &[Curve]) -> Result<DVec2> {
    match s {
        Surface::Extrusion { generator, vector } => {
            let c = &curves[*generator as usize];
            let z = vector.normalize();
            let flatten = |v: DVec3| v - z * v.dot(z);
            let u = if let Curve::Line { origin, direction } = c {
                let d = flatten(*direction);
                flatten(p - *origin).dot(d) / d.length_squared()
            } else {
                parameter(c, |q| flatten(q - p))?
            };
            let v = (p - c.point(u)?).dot(*vector) / vector.length_squared();
            Ok(DVec2::new(u, v))
        }
        Surface::Revolution {
            generator,
            origin,
            axis,
        } => {
            let c = &curves[*generator as usize];
            let d = p - *origin;
            let z = d.dot(*axis);
            let radial = d - *axis * z;
            let u = if let Curve::Line {
                origin: o,
                direction,
            } = c
            {
                let h = direction.dot(*axis);
                if h.abs() < 1e-14 {
                    return Err("ambiguous radial generator");
                }
                (z - (*o - *origin).dot(*axis)) / h
            } else {
                parameter(c, |q| {
                    let q = q - *origin;
                    let h = q.dot(*axis);
                    DVec3::new(h - z, (q - *axis * h).length() - radial.length(), 0.)
                })?
            };
            let g = c.point(u)? - *origin;
            let g = g - *axis * g.dot(*axis);
            Ok(DVec2::new(
                u,
                axis.dot(g.cross(radial)).atan2(g.dot(radial)),
            ))
        }
        _ => Err("expected a swept support"),
    }
}
