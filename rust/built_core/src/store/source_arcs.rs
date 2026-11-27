// SPDX-License-Identifier: GPL-3.0-or-later
//! Verified circular intervals on original straight display edges. Parameters
//! follow the store endpoints and may decrease across a reversed segment.
use super::{keys::Kind, Store};
use crate::exact::Frame;
use glam::DVec3;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArcBinding {
    #[serde(default)]
    pub fitted: bool,
    pub frame: Frame,
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
    pub frame: Frame,
    pub radius: f64,
    pub semi_angle: f64,
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
            frame: base.frame.clone(),
            radius: base.radius,
            semi_angle,
            uv,
        })
    }
}
