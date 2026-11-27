// SPDX-License-Identifier: GPL-3.0-or-later
use crate::exact::{
    patch::{Coedge, Edge, Face, Topology},
    Curve, Frame, Rows, Surface,
};
use built_step::step_io::{
    model as m,
    scene::{CurveKind, Solid, SurfaceKind},
    Orientation, StepModel,
};
use glam::{DVec2, DVec3};
use std::{collections::HashMap, f64::consts::TAU};

type Result<T> = std::result::Result<T, &'static str>;

struct Geometry<'a> {
    model: &'a StepModel,
    metres: f64,
}
impl Geometry<'_> {
    fn point(&self, r: &m::CartesianPointRef) -> Result<DVec3> {
        let m::CartesianPointRef::CartesianPoint(i) = r else {
            return Err("unsupported point");
        };
        let p: [f64; 3] = self.model[*i]
            .coordinates
            .as_ref()
            .try_into()
            .map_err(|_| "expected 3D point")?;
        Ok(DVec3::from(p) * self.metres)
    }
    fn direction(&self, r: &m::DirectionRef) -> Result<DVec3> {
        let m::DirectionRef::Direction(i) = r else {
            return Err("unsupported direction");
        };
        let p: [f64; 3] = self.model[*i]
            .direction_ratios
            .as_ref()
            .try_into()
            .map_err(|_| "expected 3D direction")?;
        DVec3::from(p).try_normalize().ok_or("degenerate direction")
    }
    fn frame(&self, r: m::Axis2Placement3dId) -> Result<Frame> {
        let p = &self.model[r];
        let axis = p
            .axis
            .as_ref()
            .map(|d| self.direction(d))
            .transpose()?
            .unwrap_or(DVec3::Z);
        let x = p
            .ref_direction
            .as_ref()
            .map(|d| self.direction(d))
            .transpose()?
            .unwrap_or(if axis.cross(DVec3::X).length_squared() == 0. {
                DVec3::Y
            } else {
                DVec3::X
            });
        let reference = (x - axis * x.dot(axis))
            .try_normalize()
            .ok_or("degenerate reference direction")?;
        Ok(Frame {
            origin: self.point(&p.location)?,
            axis,
            reference,
        })
    }
    fn surface_frame(&self, p: &m::Axis2Placement3dRef) -> Result<Frame> {
        let m::Axis2Placement3dRef::Axis2Placement3d(i) = p else {
            return Err("unsupported surface frame");
        };
        self.frame(*i)
    }
    fn conic_frame(&self, p: &m::Axis2PlacementRef) -> Result<Frame> {
        let m::Axis2PlacementRef::Axis2Placement3d(i) = p else {
            return Err("expected spatial conic");
        };
        self.frame(*i)
    }
    fn curve(
        &self,
        c: CurveKind<'_>,
        a: DVec3,
        b: DVec3,
        closed: bool,
    ) -> Result<(Curve, [f64; 2])> {
        let (frame, major, minor) = match c {
            CurveKind::Line(l) => {
                let origin = self.point(&l.pnt)?;
                let m::VectorRef::Vector(i) = l.dir else {
                    return Err("unsupported line vector");
                };
                let v = &self.model[i];
                if !v.magnitude.is_finite() || v.magnitude <= 0. {
                    return Err("invalid line vector");
                }
                let direction = self.direction(&v.orientation)?;
                return Ok((
                    Curve::Line { origin, direction },
                    [(a - origin).dot(direction), (b - origin).dot(direction)],
                ));
            }
            CurveKind::Circle(c) => (self.conic_frame(&c.position)?, c.radius * self.metres, None),
            CurveKind::Ellipse(c) => (
                self.conic_frame(&c.position)?,
                c.semi_axis_1 * self.metres,
                Some(c.semi_axis_2 * self.metres),
            ),
            _ => return Err("unsupported STEP curve"),
        };
        let angle = |p: DVec3| {
            let d = p - frame.origin;
            (d.dot(frame.y()) / minor.unwrap_or(major)).atan2(d.dot(frame.reference) / major)
        };
        let t = angle(a);
        let dt = if closed {
            TAU
        } else {
            (angle(b) - t).rem_euclid(TAU)
        };
        let curve = if let Some(minor) = minor {
            Curve::Ellipse {
                frame,
                major,
                minor,
            }
        } else {
            Curve::Circle {
                frame,
                radius: major,
            }
        };
        Ok((curve, [t, t + dt]))
    }
}
fn uv(s: &Surface, p: DVec3) -> Result<DVec2> {
    let (frame, cyl) = match s {
        Surface::Plane { frame } => (frame, false),
        Surface::Cylinder { frame, .. } => (frame, true),
        _ => return Err("unsupported surface trim"),
    };
    let d = p - frame.origin;
    let x = d.dot(frame.reference);
    let y = d.dot(frame.y());
    Ok(if cyl {
        DVec2::new(y.atan2(x), d.dot(frame.axis))
    } else {
        DVec2::new(x, y)
    })
}

pub(super) fn rows(model: &StepModel, solid: Solid<'_>, metres: f64, tol: f64) -> Result<Rows> {
    let geom = Geometry { model, metres };
    if solid.voids().next().is_some() {
        return Err("void shell import unavailable");
    }
    let (shell, sense) = solid.outer().ok_or("missing shell")?;
    if !shell.is_closed() {
        return Err("open solid shell");
    }
    let mut t = Topology::default();
    t.tolerance = tol;
    let mut rows = Rows::default();
    let mut vertices = HashMap::new();
    let mut edges = HashMap::new();
    let mut surfaces = HashMap::new();
    let mut incidence = HashMap::<u64, Vec<bool>>::new();
    for face in shell.faces() {
        let s = face.surface();
        let surface = if let Some(&i) = surfaces.get(&s.key()) {
            i
        } else {
            let value = match s.kind() {
                SurfaceKind::Plane(p) => Surface::Plane {
                    frame: geom.surface_frame(&p.position)?,
                },
                SurfaceKind::Cylindrical(p) => Surface::Cylinder {
                    frame: geom.surface_frame(&p.position)?,
                    radius: p.radius * metres,
                },
                _ => return Err("unsupported STEP surface"),
            };
            let i = rows.surfaces.len() as u32;
            rows.surfaces.push(value);
            surfaces.insert(s.key(), i);
            i
        };
        let support = &rows.surfaces[surface as usize];
        let mut loops = Vec::new();
        let mut outer = None;
        for bound in face.bounds() {
            let mut uses: Vec<_> = bound.oriented_edges().collect();
            let invert = bound.orientation() != sense;
            if invert {
                uses.reverse()
            }
            let mut ring: Vec<Coedge> = Vec::new();
            for (edge, dir) in uses {
                let id = if let Some(&id) = edges.get(&edge.key()) {
                    id
                } else {
                    let mut pair = [0; 2];
                    for (slot, v) in pair.iter_mut().zip([edge.start(), edge.end()]) {
                        let v = v.ok_or("missing STEP vertex")?;
                        let n = vertices.len() as u64;
                        *slot = *vertices.entry(v.key()).or_insert(n);
                        t.vertices.entry(*slot).or_insert(
                            DVec3::from(v.point().ok_or("missing point")?.xyz()) * metres,
                        );
                    }
                    if !edge.same_sense() {
                        pair.swap(0, 1)
                    }
                    let c = edge.curve();
                    let (curve, interval) = geom.curve(
                        c.curve_3d().unwrap_or(c).kind(),
                        t.vertices[&pair[0]],
                        t.vertices[&pair[1]],
                        pair[0] == pair[1],
                    )?;
                    let id = t.edges.len() as u64;
                    t.edges.push(Edge {
                        id,
                        vertices: pair,
                        curve: rows.curves.len() as u32,
                        interval,
                    });
                    rows.curves.push(curve);
                    edges.insert(edge.key(), id);
                    id
                };
                let reversed = (dir == Orientation::Reversed) ^ invert ^ !edge.same_sense();
                incidence.entry(id).or_default().push(reversed);
                let e = &t.edges[id as usize];
                let start = reversed as usize;
                let mut a = uv(support, t.vertices[&e.vertices[start]])?;
                let mut b = uv(support, t.vertices[&e.vertices[1 - start]])?;
                let curve = &rows.curves[e.curve as usize];
                let projected = matches!(support, Surface::Plane { .. })
                    && !matches!(curve, Curve::Line { .. });
                if let Surface::Cylinder { frame, .. } = support {
                    if let Some(last) = ring.last() {
                        a.x += ((last.uv[1].x - a.x) / TAU).round() * TAU
                    }
                    let delta = if let Curve::Circle { frame: c, .. } = curve {
                        (e.interval[1] - e.interval[0])
                            * c.axis.dot(frame.axis).signum()
                            * if reversed { -1. } else { 1. }
                    } else {
                        0.
                    };
                    b.x += ((a.x + delta - b.x) / TAU).round() * TAU;
                }
                ring.push(Coedge {
                    edge: Some(id),
                    pole: None,
                    projected,
                    reversed,
                    uv: [a, b],
                    tolerance: tol,
                });
            }
            if ring.is_empty() {
                return Err("unsupported empty or vertex bound");
            }
            if bound.is_outer() && outer.replace(loops.len()).is_some() {
                return Err("multiple outer bounds");
            }
            loops.push(ring)
        }
        let first = outer
            .or_else(|| (loops.len() == 1).then_some(0))
            .ok_or("ambiguous outer bound")?;
        loops.swap(0, first);
        t.faces.push(Face {
            id: t.faces.len() as u64,
            surface,
            reversed: face.orientation() != sense,
            loops,
        });
    }
    if t.faces.is_empty() || incidence.values().any(|v| v.len() != 2 || v[0] == v[1]) {
        return Err("inconsistent closed shell incidence");
    }
    rows.topology = Some(t);
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admitted_cylinder_contains_two_distinct_seam_uses() {
        let bytes=include_bytes!("../../../../tests/fixtures/step/capped-cylinder.step");
        let exchange=built_step::read(bytes).unwrap();
        let scene=exchange.model.scene();
        let units=scene.units();
        let m=units.length.unwrap().to_si;
        let rows=rows(&exchange.model,scene.all_solids().next().unwrap(),m,units.precision.unwrap()*m).unwrap();
        let patch=crate::store::native::NativePatch::new(rows).unwrap();
        let wall=&patch.topology().faces[0].loops[0];
        assert_eq!(wall[1].edge,wall[3].edge);
        assert_ne!(wall[1].uv,wall[3].uv);
        assert!(!patch.mesh().unwrap().triangles.is_empty());
    }
}
