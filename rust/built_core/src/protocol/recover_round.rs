// SPDX-License-Identifier: GPL-3.0-or-later
//! Fit matching oriented cap loops only after verifying the intervening
//! generators, the deviation along each chord and the complete solid volume.
use crate::{
    exact::Frame,
    store::{source_arcs::ArcBinding, Store},
};
use glam::DVec3;
use std::{
    collections::HashSet,
    f64::consts::{PI, TAU},
};

pub(super) struct RoundFit {
    pub rings: Vec<Vec<(u32, ArcBinding)>>,
    pub residual: f64,
    pub volume: f64,
    pub curved: HashSet<u32>,
    pub supports: usize,
    pub method: &'static str,
}
pub(super) struct Profile {
    pub arcs: Vec<Vec<(u32, ArcBinding)>>,
    pub area: f64,
    pub residual: f64,
    pub rounded: bool,
}
fn point(st: &Store, v: u32) -> DVec3 {
    st.state
        .frame
        .definition_point(*st.state.vpos.get(v as usize))
        .into()
}
pub(super) fn bind(
    st: &Store,
    v: u32,
    w: u32,
    frame: Frame,
    radius: f64,
) -> Option<(u32, ArcBinding, f64, f64)> {
    let e = st.find_edge(v, w)?;
    if st.edge_faces(e).len() != 2 {
        return None;
    }
    let ends = *st.state.ends.get(e as usize);
    let [start, end] = ends.map(|v| point(st, v) - frame.origin);
    let theta = |p: DVec3| p.dot(frame.y()).atan2(p.dot(frame.reference));
    let t = theta(start);
    let d = (theta(end) - t + PI).rem_euclid(TAU) - PI;
    let arc = ArcBinding {
        frame: frame.into(),
        radius,
        interval: [t, t + d],
        fitted: true,
    };
    if !st.arc_matches(e, &arc) || arc.validate().is_err() {
        return None;
    }
    Some((
        e,
        arc,
        if ends[0] == v { d } else { -d },
        radius * (1. - (d * 0.5).cos()),
    ))
}
fn profile(st: &Store, r: &[u32], axis: DVec3) -> Option<Profile> {
    if r.len() < 8 {
        return None;
    }
    let p: Vec<_> = r.iter().map(|&v| point(st, v)).collect();
    let center = p.iter().sum::<DVec3>() / r.len() as f64;
    let radius = p[0].distance(center);
    let reference = (p[0] - center - axis * axis.dot(p[0] - center)).try_normalize()?;
    let frame = Frame {
        origin: center,
        axis,
        reference,
    };
    let mut arc = Vec::new();
    let mut sweep = 0.;
    let mut residual = 0f64;
    for (&v, &w) in r.iter().zip(r.iter().cycle().skip(1)) {
        let Some((e, b, dt, err)) = bind(st, v, w, frame.clone(), radius) else {
            return super::recover_profile::rounded(st, r, &p, axis);
        };
        if sweep * dt < 0. {
            return None;
        }
        sweep += dt;
        residual = residual.max(err);
        arc.push((e, b));
    }
    if (sweep.abs() - TAU).abs() > 1e-8 {
        return None;
    }
    Some(Profile {
        arcs: vec![arc],
        area: PI * radius * radius,
        residual,
        rounded: false,
    })
}
pub(super) fn cylinder(st: &Store, slots: &[u32], budget: f64, input: f64) -> Option<RoundFit> {
    if input <= 0. || slots.len() < 10 {
        return None;
    }
    let caps: Vec<_> = slots
        .iter()
        .filter_map(|&f| {
            let r = st.face_rings(f);
            (r.len() > 1 || r.first().is_some_and(|v| v.len() >= 8)).then_some((f, r))
        })
        .collect();
    let [(fa, a), (_fb, b)] = caps.as_slice() else {
        return None;
    };
    if a.len() != b.len() {
        return None;
    }
    let mut fit = RoundFit {
        rings: Vec::new(),
        residual: 0.,
        volume: input,
        curved: HashSet::new(),
        supports: 0,
        method: "Coaxial circular rims and planar end cuts",
    };
    let mut paired = HashSet::new();
    for (k, r) in a.iter().enumerate() {
        let set: HashSet<_> = r.iter().copied().collect();
        let mut above = HashSet::new();
        let mut sides = Vec::new();
        for (&v, &w) in r.iter().zip(r.iter().cycle().skip(1)) {
            let e = st.find_edge(v, w)?;
            let fs = st.edge_faces(e);
            if fs.len() != 2 {
                return None;
            }
            let side = *fs.iter().find(|&&f| f != *fa)?;
            let rings = st.face_rings(side);
            let [q] = rings.as_slice() else { return None };
            if q.len() != 4 {
                return None;
            }
            above.extend(q.iter().filter(|v| !set.contains(v)).copied());
            sides.push((e, side, q.clone()));
        }
        let (other, s) = b
            .iter()
            .enumerate()
            .find(|(_, s)| s.len() == above.len() && s.iter().all(|v| above.contains(v)))?;
        if !paired.insert(other) || (k == 0) != (other == 0) {
            return None;
        }
        let centroid =
            |ring: &Vec<u32>| ring.iter().map(|&v| point(st, v)).sum::<DVec3>() / ring.len() as f64;
        let delta = centroid(s) - centroid(r);
        let axis = delta.try_normalize()?;
        for (_, _, q) in &sides {
            for (&v, &w) in q.iter().zip(q.iter().cycle().skip(1)) {
                if set.contains(&v) != set.contains(&w) {
                    let generator = if set.contains(&v) {
                        point(st, w) - point(st, v)
                    } else {
                        point(st, v) - point(st, w)
                    };
                    if generator.distance(delta) > 1e-8 {
                        return None;
                    }
                }
            }
        }
        let (Some(p), Some(q)) = (profile(st, r, axis), profile(st, s, axis)) else {
            continue;
        };
        if p.rounded != q.rounded || (p.area - q.area).abs() > 1e-12 {
            return None;
        }
        let residual = p.residual.max(q.residual);
        if residual > budget {
            continue;
        }
        let edges: HashSet<_> = p.arcs.iter().flatten().map(|(e, _)| *e).collect();
        fit.curved.extend(
            sides
                .iter()
                .filter(|(e, _, _)| edges.contains(e))
                .map(|(_, f, _)| *f),
        );
        fit.supports += p.arcs.len();
        let base = point(st, r[0]);
        let area = 0.5
            * r[1..]
                .windows(2)
                .map(|v| {
                    (point(st, v[0]) - base)
                        .cross(point(st, v[1]) - base)
                        .dot(axis)
                })
                .sum::<f64>()
                .abs();
        fit.volume += (p.area - area) * delta.length() * if k == 0 { 1. } else { -1. };
        fit.residual = fit.residual.max(residual);
        if p.rounded {
            fit.method = "Tangent rounded-rectangle profile and planar end cuts";
        } else if k > 0 {
            fit.method = "Cylindrical bore and planar end cuts";
        }
        fit.rings.extend(p.arcs);
        fit.rings.extend(q.arcs);
    }
    (fit.supports > 0 && (fit.volume - input).abs() <= 0.1 * input).then_some(fit)
}
