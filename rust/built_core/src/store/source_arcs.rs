// SPDX-License-Identifier: GPL-3.0-or-later
//! Verified circular intervals on original straight display edges. Parameters
//! follow the store endpoints and may decrease across a reversed segment.
use super::{keys::Kind, Store};
use crate::exact::{Frame, Surface};
use glam::DVec3;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArcBinding {
    #[serde(default)]
    pub fitted: bool,
    pub frame: Arc<Frame>,
    pub radius: f64,
    pub interval: [f64; 2],
}
impl ArcBinding {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.frame.validate()?;
        let dt = (self.interval[0] - self.interval[1]).abs();
        if !self.radius.is_finite()
            || self.radius <= 0.
            || !dt.is_finite()
            || dt <= 0.
            || dt >= std::f64::consts::PI
        {
            return Err("invalid source circle interval");
        }
        Ok(())
    }
    fn endpoint(&self, k: usize) -> DVec3 {
        let (sin, cos) = self.interval[k].sin_cos();
        self.frame.origin + self.radius * (self.frame.reference * cos + self.frame.y() * sin)
    }
}
impl Store {
    pub(crate) fn arc_matches(&self, e: u32, arc: &ArcBinding) -> bool {
        self.alive(Kind::Edge, e)
            && self
                .state
                .ends
                .get(e as usize)
                .iter()
                .enumerate()
                .all(|(i, &v)| {
                    let p = DVec3::from(
                        self.state
                            .frame
                            .definition_point(*self.state.vpos.get(v as usize)),
                    );
                    let tol = 1e-8 + 64. * f64::EPSILON * (p.length() + arc.radius);
                    p.distance(arc.endpoint(i)) <= tol
                })
    }
    pub(crate) fn bind_source_arc(&mut self, e: u32, arc: ArcBinding) -> Result<(), &'static str> {
        arc.validate()?;
        if !self.arc_matches(e, &arc) {
            return Err("source arc misses stored endpoints");
        }
        Arc::make_mut(self.state.exact.arcs.get_or_insert_with(Arc::default)).insert(e, arc);
        Ok(())
    }
}

/// A ruled quad certified by two authored rims and two smooth generators.
/// The source curve group identifies the same support for neighbouring
/// facets. UV corners follow the original face ring. Angles are unwrapped.
pub(crate) struct Strip {
    pub group: i128,
    pub pair: (i128, i128),
    pub frame: Frame,
    pub radius: f64,
    pub semi_angle: f64,
    pub swept: Option<Surface>,
    pub uv: Vec<[f64; 2]>,
}
impl Store {
    pub(crate) fn source_strip(&self, face: u32) -> Option<Strip> {
        let arcs = self.state.exact.arcs.as_ref()?;
        let rings = self.face_rings(face);
        let [ring] = rings.as_slice() else {
            return None;
        };
        let vs: [u32; 4] = ring.as_slice().try_into().ok()?;
        let mut edges = [0; 4];
        for k in 0..4 {
            edges[k] = self.find_edge(vs[k], vs[(k + 1) % 4])?;
        }
        if edges.iter().all(|e| arcs.contains_key(e)) {
            return self.round_strip(vs, edges);
        }
        let rim = if arcs.contains_key(&edges[0]) { 0 } else { 1 };
        let opposite = (rim + 2) % 4;
        let (a, b) = (arcs.get(&edges[rim])?, arcs.get(&edges[opposite])?);
        if !self.arc_matches(edges[rim], a) || !self.arc_matches(edges[opposite], b) {
            return None;
        }
        for k in [(rim + 1) % 4, (rim + 3) % 4] {
            if arcs.contains_key(&edges[k])
                || (!a.fitted || !b.fitted)
                    && self.state.eflags.get(edges[k] as usize) & super::SOFT == 0
            {
                return None;
            }
        }
        let ga = *self.state.ecurve.get(edges[rim] as usize);
        let gb = *self.state.ecurve.get(edges[opposite] as usize);
        if ga == gb {
            return None;
        }
        let (base, other, group) = if ga < gb { (a, b, ga) } else { (b, a, gb) };
        let axis = base.frame.axis;
        let delta = other.frame.origin - base.frame.origin;
        let height = delta.dot(axis);
        let tol = 1e-8;
        if height.abs() <= tol
            || delta.cross(axis).length() > tol
            || axis.cross(other.frame.axis).length() > 1e-10
        {
            return None;
        }
        let slope = (other.radius - base.radius) / height;
        let semi_angle = slope.atan();
        let mut uv = Vec::new();
        for (i, &v) in vs.iter().enumerate() {
            let d = DVec3::from(
                self.state
                    .frame
                    .definition_point(*self.state.vpos.get(v as usize)),
            ) - base.frame.origin;
            let z = d.dot(axis);
            let r = d.dot(base.frame.reference).hypot(d.dot(base.frame.y()));
            if (base.radius + z * slope - r).abs() > tol {
                return None;
            }
            let mut theta = d.dot(base.frame.y()).atan2(d.dot(base.frame.reference));
            if i > 0 {
                let prev: [f64; 2] = uv[i - 1];
                theta += ((prev[0] - theta) / std::f64::consts::TAU).round() * std::f64::consts::TAU
            }
            uv.push([theta, z / semi_angle.cos()]);
        }
        for k in 0..4 {
            let j = (k + 1) % 4;
            if k == rim || k == opposite {
                if (uv[k][1] - uv[j][1]).abs() > tol {
                    return None;
                }
            } else if (uv[k][0] - uv[j][0]).abs() * base.radius.max(other.radius) > tol {
                return None;
            }
        }
        Some(Strip {
            group,
            pair: (ga.min(gb), ga.max(gb)),
            frame: (*base.frame).clone(),
            radius: base.radius,
            semi_angle,
            swept: None,
            uv,
        })
    }
}

impl Store {
    /// Four authored circular boundaries determine a spherical or toroidal
    /// patch only when the two meridians are sections of the same rotation.
    fn round_strip(&self, vs: [u32; 4], edges: [u32; 4]) -> Option<Strip> {
        let arcs = self.state.exact.arcs.as_ref()?;
        let arc = |k: usize| arcs.get(&edges[k % 4]);
        if (0..4).any(|k| !self.arc_matches(edges[k], arc(k).unwrap())) {
            return None;
        }
        let rim = (0..2).find(|&k| {
            let (a, b) = (arc(k).unwrap(), arc(k + 2).unwrap());
            a.frame.axis.cross(b.frame.axis).length() <= 1e-10
                && (a.frame.origin - b.frame.origin)
                    .cross(a.frame.axis)
                    .length()
                    <= 1e-8
                && a.frame.origin.distance(b.frame.origin) > 1e-8
        })?;
        let (a, b, m, n) = (arc(rim)?, arc(rim + 2)?, arc(rim + 1)?, arc(rim + 3)?);
        if [rim + 1, rim + 3]
            .iter()
            .any(|&k| self.state.eflags.get(edges[k % 4] as usize) & super::SOFT == 0)
        {
            return None;
        }
        let axis = a.frame.axis;
        let origin = a.frame.origin + axis * (m.frame.origin - a.frame.origin).dot(axis);
        let major = (m.frame.origin - origin).length();
        let radius = m.radius;
        let tol = 1e-8;
        for section in [m, n] {
            let d = section.frame.origin - origin;
            if (section.radius - radius).abs() > tol
                || d.dot(axis).abs() > tol
                || (d.length() - major).abs() > tol
                || axis.dot(section.frame.axis).abs() > 1e-10
                || d.dot(section.frame.axis).abs() > tol
            {
                return None;
            }
        }
        let basis = if axis.x.abs() < 0.8 {
            DVec3::X
        } else {
            DVec3::Y
        };
        let frame = Frame {
            origin,
            axis,
            reference: (basis - axis * basis.dot(axis)).normalize(),
        };
        let surface = if major <= tol {
            Surface::Sphere {
                frame: frame.clone(),
                radius,
            }
        } else if major > radius + tol {
            Surface::Torus {
                frame: frame.clone(),
                major,
                minor: radius,
            }
        } else {
            return None;
        };
        let mut uv: Vec<[f64; 2]> = Vec::new();
        for (i, v) in vs.into_iter().enumerate() {
            let p = DVec3::from(
                self.state
                    .frame
                    .definition_point(*self.state.vpos.get(v as usize)),
            );
            let d = p - origin;
            let z = d.dot(axis);
            let radial = d - axis * z;
            let rho = radial.length();
            if rho <= tol || ((rho - major).hypot(z) - radius).abs() > tol {
                return None;
            }
            let mut u = d.dot(frame.y()).atan2(d.dot(frame.reference));
            let mut v = z.atan2(rho - major);
            if let Some(last) = uv.last() {
                u += ((last[0] - u) / std::f64::consts::TAU).round() * std::f64::consts::TAU;
                if major > tol {
                    v += ((last[1] - v) / std::f64::consts::TAU).round() * std::f64::consts::TAU;
                }
            }
            let section = if i == (rim + 1) % 4 || i == (rim + 2) % 4 {
                m
            } else {
                n
            };
            if major > tol && (radial / rho * major + origin).distance(section.frame.origin) > tol {
                return None;
            }
            uv.push([u, v]);
        }
        for k in 0..4 {
            let j = (k + 1) % 4;
            let diff = if k == rim || k == (rim + 2) % 4 {
                (uv[j][1] - uv[k][1]).abs() * radius
            } else {
                let du = (uv[j][0] - uv[k][0] + std::f64::consts::PI)
                    .rem_euclid(std::f64::consts::TAU)
                    - std::f64::consts::PI;
                du.abs() * (major + radius)
            };
            if diff > tol {
                return None;
            }
        }
        let ga = *self.state.ecurve.get(edges[rim] as usize);
        let gb = *self.state.ecurve.get(edges[(rim + 2) % 4] as usize);
        if ga == gb {
            return None;
        }
        let _ = b;
        Some(Strip {
            group: ga.min(gb),
            pair: (ga.min(gb), ga.max(gb)),
            frame,
            radius,
            semi_angle: 0.,
            swept: Some(surface),
            uv,
        })
    }
}
