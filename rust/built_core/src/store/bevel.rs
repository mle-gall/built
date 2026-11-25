// SPDX-License-Identifier: GPL-3.0-or-later
//! Vertex bevel. Edges shortened by a cut inherit the original edge flags.
use super::{fillet::*, Store};
use crate::store::geometry::{length, Hypot, V3};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::FRAC_PI_2;

pub fn plan(
    s: &Store,
    vertices: Vec<u32>,
    size: f64,
    n: usize,
    profile: f64,
    h: Hypot,
) -> Result<RoundPlan> {
    let mut chosen = BTreeSet::new();
    let vertices: Vec<_> = vertices.into_iter().filter(|v| chosen.insert(*v)).collect();
    if chosen.is_empty() {
        return Err(("Pick a corner to bevel.", None));
    }
    check(size, profile)?;
    let mut plan = RoundPlan::default();
    let mut cuts = BTreeMap::new();
    let mut touched = BTreeSet::new();
    for &v in &vertices {
        let origin = pos(s, v);
        let faces = s.faces_around(&[v]);
        if faces.is_empty() {
            return Err(("Only a corner of a face can be bevelled.", None));
        }
        touched.extend(faces);
        for e in s.edges_at(v) {
            match s.edge_faces(e).len() {
                0 => return Err(at("The corner at {p} has an edge with no face.", origin)),
                1 | 2 => (),
                _ => return Err(at("The corner at {p} has too many faces to round.", origin)),
            }
            let w = other(s, e, v);
            let delta = pos(s, w) - origin;
            let need = if chosen.contains(&w) { size * 2. } else { size };
            if need >= length(delta, h) - 1e-9 {
                return Err(at("The size is too large for the edge at {p}.", origin));
            }
            cuts.insert(
                (v, e),
                unit(delta, h).ok_or(("A zero-length edge cannot be rounded.", None))? * size,
            );
            plan.edges.insert(e);
        }
    }
    let exponent = bevel_exponent(profile);
    let mut curves: BTreeMap<u32, Vec<Vec<V3>>> = BTreeMap::new();
    for f in touched {
        plan.materials.push(f);
        plan.rebuild(s, f, h, |ring, i| {
            let v = ring[i];
            let o = pos(s, v);
            if !chosen.contains(&v) {
                return Ok(vec![o]);
            }
            let cut = |w| {
                s.find_edge(v, w)
                    .and_then(|e| cuts.get(&(v, e)))
                    .copied()
                    .ok_or(("Could not read the faces around the edge.", None))
            };
            let a = cut(ring[(i + ring.len() - 1) % ring.len()])?;
            let b = cut(ring[(i + 1) % ring.len()])?;
            if a.dot(b, h) / (size * size) < -2f64.to_radians().cos() {
                return Err(at("The edges at {p} are too flat to bevel.", o));
            }
            let curve = face_curve(o, a, b, exponent, n);
            curves.entry(v).or_default().push(curve.clone());
            Ok(curve)
        })?;
    }
    for v in vertices {
        let o = pos(s, v);
        let face_curves = curves.remove(&v).unwrap_or_default();
        let edges: Vec<_> = s.edges_at(v).collect();
        if edges.iter().any(|&e| s.edge_faces(e).len() != 2) {
            if n > 1 {
                plan.curves.extend(face_curves)
            }
            continue;
        }
        if edges.len() < 3 {
            return Err(at("Nothing to bevel at {p}.", o));
        }
        let us: Vec<_> = edges.iter().map(|&e| cuts[&(v, e)]).collect();
        let out = s
            .faces_around(&[v])
            .iter()
            .fold(ZERO, |a, &f| a + s.face_normal(f, h));
        close_corner(&mut plan, o, &us, &face_curves, out, n, exponent, h, false)?;
    }
    for &e in &plan.edges {
        let [a, b] = ends(s, e).map(|v| pos(s, v) + cuts.get(&(v, e)).copied().unwrap_or(ZERO));
        plan.flags.push((e, a, b))
    }
    Ok(plan)
}

pub(super) fn face_curve(o: V3, a: V3, b: V3, exponent: f64, n: usize) -> Vec<V3> {
    (0..=n)
        .map(|k| {
            let angle = FRAC_PI_2 * k as f64 / n as f64;
            let (x, y) = if k == 0 {
                (0., 1.)
            } else if k == n {
                (1., 0.)
            } else {
                (
                    angle.sin().powf(2. / exponent),
                    angle.cos().powf(2. / exponent),
                )
            };
            o + a * (1. - x) + b * (1. - y)
        })
        .collect()
}

/// Construct the cap in incident edge order so its boundary matches the
/// existing face curves. Store and materialised meshes supply the same inputs.
pub(super) fn close_corner(
    plan: &mut RoundPlan,
    o: V3,
    us: &[V3],
    face_curves: &[Vec<V3>],
    out: V3,
    n: usize,
    exponent: f64,
    h: Hypot,
    compensated: bool,
) -> Result<()> {
    if us.len() < 3 {
        return Err(at("Nothing to bevel at {p}.", o));
    }
    let mut tips: Vec<_> = us.iter().map(|&u| o + u).collect();
    let centre = mean(&tips);
    let sign = if out.dot(o - centre, h) >= 0. {
        1.
    } else {
        -1.
    };
    if us.len() == 3 {
        let side = |i, j| {
            arc(face_curves, tips[i], tips[j], h).ok_or(at("Could not close the corner at {p}.", o))
        };
        grid_cap(
            plan,
            [side(0, 1)?, side(1, 2)?, side(0, 2)?],
            o,
            sign,
            false,
            h,
            |i, j, k| {
                let weights = [i, j, k].map(|l| (l as f64 / n as f64 * FRAC_PI_2).sin().powi(2));
                let total = super::geometry::py_sum(weights.into_iter(), compensated);
                us.iter().zip(weights).fold(o, |p, (&u, w)| {
                    p + u * (1. - (1. - w / total).max(0.).powf(1. / exponent))
                })
            },
        );
    } else {
        if n > 1 {
            return Err(at("More than three rounded edges meet at {p}.", o));
        }
        let err = at("The corner at {p} is not flat enough to cut.", o);
        let normal = unit(out, h).ok_or(err)?;
        let span = tips
            .iter()
            .map(|&p| length(p - centre, h))
            .fold(1., f64::max);
        if tips
            .iter()
            .any(|&p| (p - centre).dot(normal, h).abs() > 1e-3 * span)
        {
            return Err(err);
        }
        let reference = unit(tips[0] - centre, h).ok_or(err)?;
        let side = normal.cross(reference, h);
        let angle = |p: V3| {
            let d = p - centre;
            (d.dot(side, h) as f64).atan2(d.dot(reference, h) as f64)
        };
        tips.sort_by(|a, b| angle(*a).total_cmp(&angle(*b)));
        plan.patches.push(facing(tips, o, sign, h));
    }
    Ok(())
}

#[pyo3::pyfunction]
pub(crate) fn bevel_exponent(profile: f64) -> f64 { 4f64.ln() / (1. / profile).ln() }
