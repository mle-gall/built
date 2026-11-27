// SPDX-License-Identifier: GPL-3.0-or-later
//! Select the authored pcurve on this support, including both branches of
//! a seam. The native validator proves its lift agrees with the edge.
use super::exact_read::Geometry;
use crate::exact::Surface;
use built_step::step_io::{model as m, scene::CurveKind};
use glam::DVec2;

impl Geometry<'_> {
    pub(super) fn pcurve(
        &self,
        c: CurveKind<'_>,
        key: m::EntityKey,
        surface: &Surface,
        uv: [DVec2; 2],
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
            let origin = point(&self.model[*o].coordinates)? * scale;
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
