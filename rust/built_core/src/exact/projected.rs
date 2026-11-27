// SPDX-License-Identifier: GPL-3.0-or-later
//! Certificates for planar trims containing a mixture of lines, conics and
//! rational splines. Each parameter subinterval lies in a bounded chord tube.
use super::{
    patch::{Edge, Face, Topology},
    *,
};
use glam::DVec2;
use std::collections::HashMap;

#[derive(Clone, Copy)]
struct Segment {
    a: DVec2,
    b: DVec2,
    error: f64,
    accel: f64,
    ring: usize,
    index: usize,
}
fn distance(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let ab = b - a;
    (a + ab * ((p - a).dot(ab) / ab.length_squared()).clamp(0., 1.)).distance(p)
}
pub(super) fn certify(
    t: &Topology,
    rows: &Rows,
    f: &Face,
    s: &Surface,
    edges: &HashMap<u64, &Edge>,
) -> Result<()> {
    let Surface::Plane { frame } = s else {
        return Err("projected trim needs a plane");
    };
    let project = |p: DVec3| {
        let d = p - frame.origin;
        DVec2::new(d.dot(frame.reference), d.dot(frame.y()))
    };
    let height = |p: DVec3| (p - frame.origin).dot(frame.axis).abs();
    let mut all = Vec::new();
    let mut rings = Vec::new();
    for (k, ring) in f.loops.iter().enumerate() {
        let first = all.len();
        let mut corners = Vec::new();
        for (i, c) in ring.iter().enumerate() {
            check(
                c.projected && c.pole.is_none() && c.tolerance > 0. && c.tolerance <= t.tolerance,
                "invalid projected interval",
            )?;
            let edge = edges
                .get(&c.edge.ok_or("missing projected edge")?)
                .ok_or("missing projected edge")?;
            let next = &ring[(i + 1) % ring.len()];
            let following = edges
                .get(&next.edge.ok_or("missing next edge")?)
                .ok_or("missing next edge")?;
            let start = c.reversed as usize;
            check(
                edge.vertices[1 - start] == following.vertices[next.reversed as usize],
                "open projected ring",
            )?;
            let curve = &rows.curves[edge.curve as usize];
            let deviation = match curve {
                Curve::Line { .. } => edge
                    .vertices
                    .map(|id| height(t.vertices[&id]))
                    .into_iter()
                    .fold(0., f64::max),
                Curve::Circle {
                    frame: b,
                    radius: r,
                } => height(b.origin) + r * b.axis.cross(frame.axis).length(),
                Curve::Ellipse {
                    frame: b,
                    major,
                    minor,
                } => {
                    height(b.origin)
                        + (major * b.reference.dot(frame.axis)).hypot(minor * b.y().dot(frame.axis))
                }
                Curve::Spline { controls, .. } => controls
                    .iter()
                    .map(|p| height(p.truncate()))
                    .fold(0., f64::max),
            };
            check(deviation <= c.tolerance, "projected trim leaves plane")?;
            for j in 0..2 {
                check(
                    project(t.vertices[&edge.vertices[start ^ j]]).distance(c.uv[j]) <= c.tolerance,
                    "invalid projected anchor",
                )?;
            }
            let dt = edge.interval[1 - start] - edge.interval[start];
            let accel = curve.acceleration(dt)?;
            let n = super::patch::divisions(accel, c.tolerance / 4.)?;
            check(
                all.len().saturating_add(n) <= 4_000_000,
                "projected trim budget exceeded",
            )?;
            let bound = accel / (n as f64).powi(2);
            let mut a = project(curve.point(edge.interval[start])?);
            for j in 1..=n {
                let b = project(curve.point(edge.interval[start] + dt * j as f64 / n as f64)?);
                check(a.distance(b) > bound * 0.5, "unresolved projected tangent")?;
                all.push(Segment {
                    a,
                    b,
                    error: bound / 8. + c.tolerance,
                    accel: bound,
                    ring: k,
                    index: corners.len(),
                });
                corners.push(a);
                a = b;
            }
        }
        check(corners.len() >= 3, "degenerate projected ring")?;
        for i in first..all.len() {
            let a = all[i];
            let b = all[first + (i - first + 1) % corners.len()];
            let da = a.b - a.a;
            let db = b.b - b.a;
            let bisector = (da.normalize() + db.normalize())
                .try_normalize()
                .ok_or("reversing projected trim")?;
            check(
                da.dot(bisector) > a.accel * 0.5 && db.dot(bisector) > b.accel * 0.5,
                "ambiguous projected join",
            )?;
        }
        rings.push(corners);
    }
    // Overlapping x ranges prune almost all pairs on finely sampled conics.
    all.sort_by(|a, b| (a.a.x.min(a.b.x) - a.error).total_cmp(&(b.a.x.min(b.b.x) - b.error)));
    for (i, &a) in all.iter().enumerate() {
        for &b in &all[i + 1..] {
            if b.a.x.min(b.b.x) - b.error > a.a.x.max(a.b.x) + a.error {
                break;
            }
            let gap = a.error + b.error;
            if a.a.y.min(a.b.y) > b.a.y.max(b.b.y) + gap
                || b.a.y.min(b.b.y) > a.a.y.max(a.b.y) + gap
            {
                continue;
            }
            let delta = a.index.abs_diff(b.index);
            if a.ring == b.ring && (delta == 1 || delta + 1 == rings[a.ring].len()) {
                continue;
            }
            check(
                !super::patch::crosses(a.a, a.b, b.a, b.b)
                    && [
                        distance(a.a, b.a, b.b),
                        distance(a.b, b.a, b.b),
                        distance(b.a, a.a, a.b),
                        distance(b.b, a.a, a.b),
                    ]
                    .into_iter()
                    .all(|d| d > gap),
                "uncertified projected boundary separation",
            )?;
        }
    }
    let inside = |r: &Vec<DVec2>, p| {
        super::patch::inside(
            r.iter()
                .zip(r.iter().cycle().skip(1))
                .map(|(&a, &b)| [a, b]),
            p,
        )
    };
    for (k, r) in rings.iter().enumerate() {
        let area: f64 = r
            .iter()
            .zip(r.iter().cycle().skip(1))
            .map(|(&a, &b)| (a - r[0]).perp_dot(b - r[0]))
            .sum();
        check(area.is_finite() && area != 0., "zero area projected trim")?;
        if k > 0 {
            check(
                inside(&rings[0], r[0])
                    && (1..rings.len())
                        .filter(|&j| j != k)
                        .all(|j| !inside(&rings[j], r[0])),
                "invalid projected hole nesting",
            )?;
        }
    }
    Ok(())
}
