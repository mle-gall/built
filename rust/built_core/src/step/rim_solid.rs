// SPDX-License-Identifier: GPL-3.0-or-later
//! Merge a complete band between two certified circular boundaries and
//! planar caps. The source mesh stays in its original segmented topology.
use crate::{
    exact::{
        patch::{Coedge, Edge, Face, Topology},
        Curve, Frame, Rows, Surface,
    },
    store::{keys::Kind, native::NativePatch, Store},
};
use glam::DVec2;
use std::{collections::HashSet, f64::consts::TAU};

pub(super) fn recover(st: &Store) -> Result<NativePatch, &'static str> {
    let arcs = st
        .state
        .exact
        .arcs
        .as_ref()
        .ok_or("missing circular boundaries")?;
    let mut caps = Vec::new();
    let mut side = None;
    let mut count = 0;
    for f in st.live_slots(Kind::Face) {
        if let Some(strip) = st.source_strip(f) {
            if side.is_some_and(|g| g != strip.group) {
                return Err("multiple independent ruled bands");
            }
            side = Some(strip.group);
            count += 1;
        } else {
            let rings = st.face_rings(f);
            let [outer] = rings.as_slice() else {
                return Err("incomplete ruled band");
            };
            let mut group = None;
            let mut sweep = 0.;
            let mut used = HashSet::new();
            let mut geometry = None;
            for (&a, &b) in outer.iter().zip(outer.iter().cycle().skip(1)) {
                let e = st.find_edge(a, b).ok_or("missing cap edge")?;
                if !used.insert(e) || st.edge_faces(e).len() != 2 {
                    return Err("invalid cap incidence");
                }
                let g = *st.state.ecurve.get(e as usize);
                if group.replace(g).is_some_and(|prev| prev != g) {
                    return Err("mixed cap curves");
                }
                let arc = arcs
                    .get(&e)
                    .filter(|a| st.arc_matches(e, a))
                    .ok_or("cap boundary lacks a source circle")?;
                if geometry
                    .is_some_and(|(frame, radius)| frame != &arc.frame || radius != arc.radius)
                {
                    return Err("inconsistent cap circles");
                }
                let direction = if st.state.ends.get(e as usize)[0] == a {
                    1.
                } else {
                    -1.
                };
                let dt = direction * (arc.interval[1] - arc.interval[0]);
                if sweep * dt < 0. {
                    return Err("cap reverses along a source circle");
                }
                sweep += dt;
                geometry = Some((&arc.frame, arc.radius));
            }
            if (sweep.abs() - TAU).abs() > 1e-8 {
                return Err("incomplete circular cap");
            }
            let (frame, r) = geometry.ok_or("empty cap")?;
            caps.push((frame, r, sweep.signum(), used.len()));
        }
    }
    let [a, b] = caps.as_slice() else {
        return Err("expected two complete circular caps");
    };
    if count < 3 || count != a.3 || count != b.3 || arcs.len() != 2 * count {
        return Err("incomplete side band");
    }
    for e in st.live_slots(Kind::Edge) {
        if st.edge_faces(e).len() != 2 {
            return Err("open or nonmanifold band");
        }
    }
    let delta = b.0.origin - a.0.origin;
    let axis = delta.try_normalize().ok_or("coincident rims")?;
    if axis.dot(a.0.axis) * a.2 > -1. + 1e-10 || axis.dot(b.0.axis) * b.2 < 1. - 1e-10 {
        return Err("inconsistent cap orientation");
    }
    let x = (a.0.reference - axis * a.0.reference.dot(axis)).normalize();
    let frame = |origin| Frame {
        origin,
        axis,
        reference: x,
    };
    let fa = frame(a.0.origin);
    let fb = frame(b.0.origin);
    let points = [fa.origin + x * a.1, fb.origin + x * b.1];
    let v = points[1] - points[0];
    let angle = (b.1 - a.1).atan2(delta.length());
    let surface = if (b.1 - a.1).abs() <= 1e-12 {
        Surface::Cylinder {
            frame: fa.clone(),
            radius: a.1,
        }
    } else {
        Surface::Cone {
            frame: fa.clone(),
            radius: a.1,
            half_angle: angle,
        }
    };
    let mut t = Topology::default();
    t.tolerance = 1e-5;
    t.vertices
        .extend(points.into_iter().enumerate().map(|(i, p)| (i as u64, p)));
    for (id, vs, end) in [(0, [0, 0], TAU), (1, [1, 1], TAU), (2, [0, 1], v.length())] {
        t.edges.push(Edge {
            id,
            vertices: vs,
            curve: id as u32,
            interval: [0., end],
        });
    }
    let h = v.length();
    let corners = [[0., 0.], [TAU, 0.], [TAU, h], [0., h]];
    let mut wall = Vec::new();
    for (i, (edge, rev)) in [(0, false), (2, false), (1, true), (2, true)]
        .into_iter()
        .enumerate()
    {
        wall.push(Coedge {
            edge: Some(edge),
            pole: None,
            projected: false,
            reversed: rev,
            uv: [corners[i].into(), corners[(i + 1) % 4].into()],
            tolerance: 1e-6,
        })
    }
    t.faces.push(Face {
        id: 0,
        surface: 0,
        reversed: false,
        loops: vec![wall],
    });
    for (id, r) in [a.1, b.1].into_iter().enumerate() {
        t.faces.push(Face {
            id: id as u64 + 1,
            surface: id as u32 + 1,
            reversed: id == 0,
            loops: vec![vec![Coedge {
                edge: Some(id as u64),
                pole: None,
                projected: true,
                reversed: id == 0,
                uv: [DVec2::new(r, 0.); 2],
                tolerance: 1e-6,
            }]],
        });
    }
    NativePatch::new(Rows {
        curves: vec![
            Curve::Circle {
                frame: fa.clone(),
                radius: a.1,
            },
            Curve::Circle {
                frame: fb.clone(),
                radius: b.1,
            },
            Curve::Line {
                origin: points[0],
                direction: v.normalize(),
            },
        ],
        surfaces: vec![
            surface,
            Surface::Plane { frame: fa },
            Surface::Plane { frame: fb },
        ],
        topology: Some(t),
    })
}
