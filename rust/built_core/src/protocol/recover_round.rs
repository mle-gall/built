// SPDX-License-Identifier: GPL-3.0-or-later
//! Recover a complete right circular extrusion only when topology, both cap
//! circles, every generator, chord deviation and the volume change agree.
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
    pub caps: [u32; 2],
}
pub(super) fn cylinder(st: &Store, slots: &[u32], budget: f64, input: f64) -> Option<RoundFit> {
    if input <= 0. || slots.len() < 10 {
        return None;
    }
    let point = |v: u32| {
        DVec3::from(
            st.state
                .frame
                .definition_point(*st.state.vpos.get(v as usize)),
        )
    };
    let mut rings = Vec::new();
    let mut walls = Vec::new();
    for &f in slots {
        let loops = st.face_rings(f);
        let [r] = loops.as_slice() else { return None };
        if r.len() == 4 {
            walls.push(r.clone())
        } else if r.len() >= 8 {
            rings.push((f, r.clone()))
        } else {
            return None;
        }
    }
    let [a, b] = rings.as_slice() else {
        return None;
    };
    if a.1.len() != b.1.len() || walls.len() != a.1.len() {
        return None;
    }
    let centroid = |r: &[u32]| r.iter().map(|&v| point(v)).sum::<DVec3>() / r.len() as f64;
    let ca = centroid(&a.1);
    let cb = centroid(&b.1);
    let delta = cb - ca;
    let axis = delta.try_normalize()?;
    let x = (point(a.1[0]) - ca).try_normalize()?;
    let radius = point(a.1[0]).distance(ca);
    let mut residual = 0f64;
    let mut result = Vec::new();
    let mut cap_edges = HashSet::new();
    let mut generators = HashSet::new();
    for (r, c) in [(&a.1, ca), (&b.1, cb)] {
        let frame = Frame {
            origin: c,
            axis,
            reference: x,
        };
        frame.validate().ok()?;
        let mut bindings = Vec::new();
        let mut sweep = 0.;
        for (&v, &w) in r.iter().zip(r.iter().cycle().skip(1)) {
            let e = st.find_edge(v, w)?;
            if !cap_edges.insert(e) || st.edge_faces(e).len() != 2 {
                return None;
            }
            let [start, end] = st.state.ends.get(e as usize).map(|i| point(i) - c);
            let theta = |v: DVec3| v.dot(frame.y()).atan2(v.dot(x));
            let t = theta(start);
            let d = (theta(end) - t + PI).rem_euclid(TAU) - PI;
            let arc = ArcBinding {
                frame: frame.clone(),
                radius,
                interval: [t, t + d],
                fitted: true,
            };
            if !st.arc_matches(e, &arc) || arc.validate().is_err() {
                return None;
            }
            let directed = if st.state.ends.get(e as usize)[0] == v {
                d
            } else {
                -d
            };
            if sweep * directed < 0. {
                return None;
            }
            sweep += directed;
            // The maximum distance in either direction between this chord
            // and its subtended minor arc occurs at the midpoint.
            residual = residual.max(radius * (1. - (0.5 * d).cos()));
            bindings.push((e, arc));
        }
        if (sweep.abs() - TAU).abs() > 1e-8 {
            return None;
        }
        result.push(bindings);
    }
    for r in walls {
        let mut count = 0;
        for (&v, &w) in r.iter().zip(r.iter().cycle().skip(1)) {
            let e = st.find_edge(v, w)?;
            if cap_edges.contains(&e) {
                count += 1;
                continue;
            }
            let d = point(w) - point(v);
            if (d - delta).length().min((d + delta).length()) > 1e-8 || st.edge_faces(e).len() != 2
            {
                return None;
            }
            generators.insert(e);
        }
        if count != 2 {
            return None;
        }
    }
    let volume = PI * radius * radius * delta.length();
    if generators.len() != a.1.len() || residual > budget || (volume - input).abs() > 0.1 * input {
        return None;
    }
    Some(RoundFit {
        rings: result,
        residual,
        volume,
        caps: [a.0, b.0],
    })
}
