// SPDX-License-Identifier: GPL-3.0-or-later
//! Select the authored pcurve on this support, including both branches of
//! a seam. The native validator proves its lift agrees with the edge.
use super::exact_read::Geometry;
use crate::exact::{Curve, Surface};
use built_step::step_io::{model as m, scene::CurveKind};
use glam::DVec2;

impl Geometry<'_> {
    pub(super) fn pcurve(
        &self,
        c: CurveKind<'_>,
        key: m::EntityKey,
        surface: &Surface,
        uv: [DVec2; 2],
        edge: &Curve,
        tolerance: f64,
    ) -> Result<[DVec2; 2], &'static str> {
        let assoc = match c {
            CurveKind::SurfaceCurve(c) => &c.associated_geometry,
            CurveKind::SeamCurve(c) => &c.associated_geometry,
            CurveKind::BoundedSurfaceCurve(c) => &c.associated_geometry,
            CurveKind::IntersectionCurve(c) => &c.associated_geometry,
            _ => return Ok(uv),
        };
        let mut best = None;
        for item in assoc {
            let (basis, rep) = match item {
                m::PcurveOrSurfaceRef::Pcurve(id) => {
                    let p = &self.model[*id];
                    (&p.basis_surface, &p.reference_to_curve)
                }
                m::PcurveOrSurfaceRef::BoundedPcurve(id) => {
                    let p = &self.model[*id];
                    (&p.basis_surface, &p.reference_to_curve)
                }
                _ => continue,
            };
            if basis.entity_key() != key {
                continue;
            }
            let m::DefinitionalRepresentationRef::DefinitionalRepresentation(id) = rep else {
                return Err("unsupported pcurve representation");
            };
            let items = &self.model[*id].items;
            let conic = match items.as_slice() {
                [m::RepresentationItemRef::Circle(id)] => {
                    let c = &self.model[*id];
                    Some((&c.position, c.radius, c.radius))
                }
                [m::RepresentationItemRef::Ellipse(id)] => {
                    let c = &self.model[*id];
                    Some((&c.position, c.semi_axis_1, c.semi_axis_2))
                }
                _ => None,
            };
            if let Some((axes, major, minor)) = conic {
                let m::Axis2PlacementRef::Axis2Placement2d(id) = axes else {
                    return Err("expected planar conic placement");
                };
                let Surface::Plane { frame } = surface else {
                    return Err("conic pcurve on unsupported chart");
                };
                let pos = &self.model[*id];
                let m::CartesianPointRef::CartesianPoint(id) = pos.location else {
                    return Err("missing pcurve centre");
                };
                let [x, y] = *self.model[id].coordinates.as_ref() else {
                    return Err("expected a 2D conic origin");
                };
                let center = frame.origin + (frame.reference * x + frame.y() * y) * self.metres;
                let dir = match pos.ref_direction {
                    None => DVec2::X,
                    Some(m::DirectionRef::Direction(i)) => {
                        let [x, y] = *self.model[i].direction_ratios.as_ref() else {
                            return Err("expected a 2D conic direction");
                        };
                        DVec2::new(x, y)
                            .try_normalize()
                            .ok_or("zero pcurve direction")?
                    }
                    _ => return Err("unsupported pcurve axis"),
                };
                let (space, a, b) = match edge {
                    Curve::Circle { frame, radius } => (frame, *radius, *radius),
                    Curve::Ellipse {
                        frame,
                        major,
                        minor,
                    } => (frame, *major, *minor),
                    _ => return Err("planar conic pcurve disagrees with its spatial edge"),
                };
                let major = major * self.metres;
                let minor = minor * self.metres;
                let axis = frame.reference * dir.x + frame.y() * dir.y;
                // Bound the Hausdorff difference of the two complete ellipses:
                // centre, radii, plane tilt, and the anisotropic part's axis.
                let err = center.distance(space.origin)
                    + (major - a).abs().max((minor - b).abs())
                    + 2. * a * frame.axis.cross(space.axis).length()
                    + 2. * (a - b).abs() * axis.cross(space.reference).length();
                if major <= 0. || minor <= 0. || !err.is_finite() || err > tolerance {
                    return Err("planar conic pcurve disagrees with its spatial edge");
                }
                best = Some((0., uv));
                continue;
            }
            let [m::RepresentationItemRef::Line(line)] = self.model[*id].items.as_slice() else {
                return Err("unsupported nonlinear STEP pcurve");
            };
            let l = &self.model[*line];
            let (m::CartesianPointRef::CartesianPoint(o), m::VectorRef::Vector(v)) =
                (&l.pnt, &l.dir)
            else {
                return Err("invalid pcurve line");
            };
            let m::DirectionRef::Direction(d) = self.model[*v].orientation else {
                return Err("invalid pcurve direction");
            };
            let point = |p: &[f64]| -> Result<DVec2, &'static str> {
                let [x, y] = p else {
                    return Err("expected 2D pcurve");
                };
                Ok(DVec2::new(*x, *y))
            };
            let scale = match surface {
                Surface::Plane { .. } => DVec2::splat(self.metres),
                Surface::Cylinder { .. } | Surface::Cone { .. } => {
                    DVec2::new(self.angle.ok_or("missing plane angle unit")?, self.metres)
                }
                Surface::Sphere { .. } | Surface::Torus { .. } => {
                    DVec2::splat(self.angle.ok_or("missing plane angle unit")?)
                }
                Surface::Spline { .. } => DVec2::ONE,
                _ => return Err("unsupported swept pcurve domain"),
            };
            let mut origin = point(&self.model[*o].coordinates)? * scale;
            // Decimal exporters can round a seam's 2π coordinate. Use the same
            // period as the spatial curve when the two differ by roundoff.
            let snap_turn = |v: f64| {
                let period = std::f64::consts::TAU;
                let seam = (v / period).round() * period;
                if (v - seam).abs() <= 1e-12 {
                    seam
                } else {
                    v
                }
            };
            match surface {
                Surface::Torus { .. } => {
                    origin.x = snap_turn(origin.x);
                    origin.y = snap_turn(origin.y);
                }
                Surface::Sphere { .. } | Surface::Cylinder { .. } | Surface::Cone { .. } => {
                    origin.x = snap_turn(origin.x);
                }
                _ => {}
            }
            let dir = (point(&self.model[d].direction_ratios)? * scale)
                .try_normalize()
                .ok_or("zero pcurve direction")?;
            let candidate = uv.map(|p| origin + dir * (p - origin).dot(dir));
            let distance =
                candidate[0].distance_squared(uv[0]) + candidate[1].distance_squared(uv[1]);
            if best.is_none_or(|(score, _)| distance < score) {
                best = Some((distance, candidate));
            }
        }
        Ok(best.map_or(uv, |(_, p)| p))
    }
}
