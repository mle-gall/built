// SPDX-License-Identifier: GPL-3.0-or-later
//! Fit a circular cylinder bounded by two independent planes. Intersect
//! each plane with the cylinder to obtain its ellipse. The native patch
//! uses the original definition's vertex, edge and face slots.
use crate::{
    exact::{
        patch::{Coedge, Edge, Face, Topology},
        Curve, Frame, Rows, Surface,
    },
    store::{keys::Kind, native::NativePatch, Store},
};
use glam::{DVec2, DVec3};
use std::{
    collections::HashSet,
    f64::consts::{PI, TAU},
    sync::Arc,
};

pub(super) struct Cut {
    pub patch: Arc<NativePatch>,
    pub volume: f64,
    pub residual: f64,
    pub caps: [u32; 2],
}
fn angle(frame: &Frame, p: DVec3) -> f64 {
    let d = p - frame.origin;
    d.dot(frame.y()).atan2(d.dot(frame.reference))
}
fn unwrap(a: f64, b: f64) -> f64 {
    b + ((a - b) / TAU).round() * TAU
}
pub(super) fn fit(st: &Store, selection: &[u32], input: f64, budget: f64) -> Option<Cut> {
    if input <= 0. || selection.len() != st.n(Kind::Face) || !st.state.frame.is_model() {
        return None;
    }
    // A native definition indexes its logical geometry by compact slot.
    for k in Kind::ALL {
        if st.state.slots[k as usize].len() != st.n(k) {
            return None;
        }
    }
    let rings: Vec<Vec<u32>> = (0..st.n(Kind::Face))
        .map(|f| {
            let mut r = st.face_rings(f as u32);
            if r.len() == 1 {
                r.pop()
            } else {
                None
            }
        })
        .collect::<Option<_>>()?;
    let ends: Vec<_> = rings
        .iter()
        .enumerate()
        .filter(|(_, r)| r.len() >= 8)
        .map(|(f, _)| f)
        .collect();
    let [lo, hi] = *ends.as_slice() else {
        return None;
    };
    let n = rings[lo].len();
    if rings[hi].len() != n
        || rings.len() != n + 2
        || st.n(Kind::Vertex) != 2 * n
        || st.n(Kind::Edge) != 3 * n
    {
        return None;
    }
    let lower: HashSet<_> = rings[lo].iter().copied().collect();
    let upper: HashSet<_> = rings[hi].iter().copied().collect();
    if !lower.is_disjoint(&upper) {
        return None;
    }
    let p = |v: u32| DVec3::from(*st.state.vpos.get(v as usize));
    let v = rings[lo][0];
    let w = st
        .edges_at(v)
        .map(|e| *st.state.ends.get(e as usize))
        .flatten()
        .find(|v| upper.contains(v))?;
    let z = (p(w) - p(v)).try_normalize()?;
    let flatten = |q: DVec3| q - z * q.dot(z);
    let a = p(v);
    let x = flatten(p(rings[lo][n / 3]) - a);
    let y = flatten(p(rings[lo][2 * n / 3]) - a);
    let cross = x.cross(y);
    let center = a
        + (x.length_squared() * y.cross(cross) + y.length_squared() * cross.cross(x))
            / (2. * cross.length_squared());
    let radius = flatten(a - center).length();
    let cyl = Frame {
        origin: center,
        axis: z,
        reference: flatten(a - center).try_normalize()?,
    };
    let mut cap = Vec::new();
    for (i, &f) in [lo, hi].iter().enumerate() {
        let r = &rings[f];
        let o = p(r[0]);
        let normal = r[1..]
            .windows(2)
            .map(|v| (p(v[0]) - o).cross(p(v[1]) - o))
            .sum::<DVec3>()
            .try_normalize()?;
        if normal.dot(z) * if i == 0 { -1. } else { 1. } <= 1e-5 {
            return None;
        }
        let normal = if i == 0 { -normal } else { normal };
        let origin = center + z * ((o - center).dot(normal) / normal.dot(z));
        let radial = flatten(normal).try_normalize().unwrap_or(cyl.reference);
        let reference = (radial - z * (radial.dot(normal) / normal.dot(z))).try_normalize()?;
        cap.push((
            Frame {
                origin,
                axis: normal,
                reference,
            },
            radius / normal.dot(z),
        ));
    }
    if cap.iter().all(|(f, _)| f.axis.cross(z).length() < 1e-8) {
        return None;
    }
    let height = (cap[1].0.origin - cap[0].0.origin).dot(z);
    let gradient = |f: &Frame| flatten(f.axis) / f.axis.dot(z);
    if height <= radius * (gradient(&cap[1].0) - gradient(&cap[0].0)).length() {
        return None;
    }
    let volume = PI * radius * radius * height;
    if !volume.is_finite() || (volume - input).abs() > 0.1 * input {
        return None;
    }
    let tol = 1e-7_f64.min(budget / 4.);
    let mut t = Topology::default();
    t.tolerance = tol;
    t.display_tolerance = Some(5e-5_f64.max(tol));
    let mut curves = Vec::new();
    let mut residual = 0f64;
    for i in 0..st.n(Kind::Vertex) {
        t.vertices.insert(i as u64, p(i as u32));
    }
    for i in 0..st.n(Kind::Edge) {
        let e = i as u32;
        let vs = *st.state.ends.get(i);
        let [a, b] = vs.map(p);
        if st.edge_faces(e).len() != 2 {
            return None;
        }
        let caps = vs.map(|v| usize::from(upper.contains(&v)));
        let (curve, interval) = if caps[0] == caps[1] {
            let (mut f, major) = cap[caps[0]].clone();
            let theta = |f: &Frame, p: DVec3| {
                let d = p - f.origin;
                (d.dot(f.y()) / radius).atan2(d.dot(f.reference) / major)
            };
            let mut start = theta(&f, a);
            let mut end = unwrap(start, theta(&f, b));
            if end < start {
                f.axis = -f.axis;
                start = -start;
                end = -end;
            }
            let err = major * (1. - ((end - start) * 0.5).cos());
            residual = residual.max(err);
            (
                Curve::Ellipse {
                    frame: f,
                    major,
                    minor: radius,
                },
                [start, end],
            )
        } else {
            if flatten(b - a).length() > tol {
                return None;
            }
            (
                Curve::Line {
                    origin: a,
                    direction: (b - a).try_normalize()?,
                },
                [0., b.distance(a)],
            )
        };
        curves.push(curve);
        t.edges.push(Edge {
            id: i as u64,
            vertices: vs.map(u64::from),
            curve: e,
            interval,
        });
    }
    if residual + tol > budget {
        return None;
    }
    for (i, r) in rings.iter().enumerate() {
        let side = i != lo && i != hi;
        if side && (r.len() != 4 || r.iter().filter(|v| lower.contains(v)).count() != 2) {
            return None;
        }
        let k = usize::from(i == hi);
        let mut uses: Vec<Coedge> = Vec::new();
        for (&a, &b) in r.iter().zip(r.iter().cycle().skip(1)) {
            let e = st.find_edge(a, b)?;
            let edge = &t.edges[e as usize];
            let projected = !matches!(curves[e as usize], Curve::Line { .. });
            let uv = if side {
                let angle_a = angle(&cyl, p(a));
                let u = uses.last().map_or(angle_a, |c| c.uv[1].x);
                [
                    DVec2::new(u, (p(a) - center).dot(z)),
                    DVec2::new(unwrap(u, angle(&cyl, p(b))), (p(b) - center).dot(z)),
                ]
            } else {
                [a, b].map(|v| {
                    let f = &cap[k].0;
                    let d = p(v) - f.origin;
                    DVec2::new(d.dot(f.reference), d.dot(f.y()))
                })
            };
            uses.push(Coedge {
                edge: Some(e.into()),
                pole: None,
                reversed: edge.vertices[0] != u64::from(a),
                projected,
                uv,
                tolerance: tol,
            });
        }
        let sign = uses
            .iter()
            .map(|c| (c.uv[0] - uses[0].uv[0]).perp_dot(c.uv[1] - uses[0].uv[0]))
            .sum::<f64>();
        if side && sign <= 0. {
            return None;
        }
        t.faces.push(Face {
            id: i as u64,
            surface: if side { 0 } else { k as u32 + 1 },
            reversed: i == lo,
            loops: vec![uses],
        });
    }
    let rows = Rows {
        curves,
        surfaces: vec![
            Surface::Cylinder { frame: cyl, radius },
            Surface::Plane {
                frame: cap[0].0.clone(),
            },
            Surface::Plane {
                frame: cap[1].0.clone(),
            },
        ],
        topology: Some(t),
    };
    let patch = Arc::new(NativePatch::new(rows).ok()?);
    patch.mesh().ok()?;
    Some(Cut {
        patch,
        residual: residual + tol,
        volume,
        caps: [lo as u32, hi as u32],
    })
}
