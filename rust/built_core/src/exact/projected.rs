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
    middle: Option<DVec2>,
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
                    middle: match curve {
                        Curve::Circle { frame, .. } | Curve::Ellipse { frame, .. }
                            if dt.abs() / (n as f64) < std::f64::consts::PI =>
                        {
                            let at = edge.interval[start] + dt * (j as f64 - 0.5) / n as f64;
                            Some(project(
                                frame.origin
                                    + (curve.point(at)? - frame.origin)
                                        / (dt / (2. * n as f64)).cos(),
                            ))
                        }
                        _ => None,
                    },
                    error: bound / 8. + c.tolerance,
                    accel: bound,
                    ring: k,
                    index: corners.len(),
                });
                corners.push(a);
                a = b;
            }
        }
        joins(&all[first..], corners.len())?;
        rings.push(corners);
    }
    separated(all, &rings)
}
/// An affine image of a positive-weight quadratic Bezier arc lies
/// strictly on one side of a line if all three control points are on
/// its closed side and at least one is on its open side. Endpoint
/// contacts are checked independently, including at a tangent cusp.
fn line_conic(line: Segment, arc: Segment, adjacent: bool) -> bool {
    let Some(mid) = arc.middle else { return false };
    if line.accel != 0. {
        return false;
    }
    let d = line.b - line.a;
    let side = |p: DVec2| d.perp_dot(p - line.a);
    let signs = [side(arc.a), side(mid), side(arc.b)];
    if !([1., -1.]
        .into_iter()
        .any(|sign| signs.iter().all(|&v| v * sign >= 0.) && signs.iter().any(|&v| v * sign > 0.)))
    {
        return false;
    }
    [arc.a, arc.b].into_iter().all(|p| {
        if side(p) != 0. {
            return true;
        }
        let t = (p - line.a).dot(d);
        t < 0. || t > d.length_squared() || (adjacent && (p == line.a || p == line.b))
    })
}
fn separated(mut all: Vec<Segment>, rings: &[Vec<DVec2>]) -> Result<()> {
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
            if line_conic(a, b, false) || line_conic(b, a, false) {
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

fn joins(all: &[Segment], count: usize) -> Result<()> {
    check(count >= 3, "degenerate projected ring")?;
    for i in 0..all.len() {
        let a = all[i];
        let b = all[(i + 1) % count];
        if line_conic(a, b, true) || line_conic(b, a, true) {
            continue;
        }
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
    Ok(())
}
/// Inverse radial projection on a regular unwrapped cylinder. The reference
/// angle selects the same period as the directed coedge.
pub(super) fn cylinder_uv(frame: &Frame, p: DVec3, reference: f64) -> DVec2 {
    let d = p - frame.origin;
    let u = d.dot(frame.y()).atan2(d.dot(frame.reference));
    DVec2::new(
        u + ((reference - u) / std::f64::consts::TAU).round() * std::f64::consts::TAU,
        d.dot(frame.axis),
    )
}
pub(super) fn cylinder(
    t: &Topology,
    rows: &Rows,
    f: &Face,
    s: &Surface,
    edges: &HashMap<u64, &Edge>,
) -> Result<()> {
    let Surface::Cylinder { frame, radius } = s else {
        return Err("expected a cylindrical trim");
    };
    let metric = |uv: DVec2| DVec2::new(uv.x * radius, uv.y);
    let mut all = Vec::new();
    let mut rings = Vec::new();
    for (k, ring) in f.loops.iter().enumerate() {
        let first = all.len();
        let mut corners = Vec::new();
        for (i, c) in ring.iter().enumerate() {
            let next = &ring[(i + 1) % ring.len()];
            let edge = edges
                .get(&c.edge.ok_or("missing cylindrical edge")?)
                .ok_or("missing cylindrical edge")?;
            let after = edges
                .get(&next.edge.ok_or("missing next edge")?)
                .ok_or("missing next edge")?;
            let start = usize::from(c.reversed);
            check(
                c.pole.is_none()
                    && c.tolerance > 0.
                    && c.tolerance <= t.tolerance
                    && edge.vertices[1 - start] == after.vertices[usize::from(next.reversed)]
                    && metric(c.uv[1] - next.uv[0]).length() <= c.tolerance,
                "open cylindrical trim",
            )?;
            let curve = &rows.curves[edge.curve as usize];
            if !c.projected {
                t.certify(rows, s, c, edge)?;
                let [a, b] = c.uv.map(metric);
                all.push(Segment {
                    a,
                    b,
                    middle: None,
                    error: 0.,
                    accel: 0.,
                    ring: k,
                    index: corners.len(),
                });
                corners.push(a);
                continue;
            }
            let dt = edge.interval[1 - start] - edge.interval[start];
            let speed = match curve {
                Curve::Line { .. } => dt.abs(),
                Curve::Circle { radius, .. } => radius * dt.abs(),
                Curve::Ellipse { major, .. } => major * dt.abs(),
                Curve::Spline { axis, controls } => spline::bounds(controls, &[(axis, dt)])?.speed,
            };
            let acc = curve.acceleration(dt)?;
            // Under rho >= R/2 the signed radial residual has second
            // derivative at most A+2V²/R. R*atan2 has bound 2A+8V²/R.
            let radial = acc + 2. * speed.powi(2) / radius;
            let chart = 3. * acc + 8. * speed.powi(2) / radius;
            let n = super::patch::divisions(chart, c.tolerance / 4.)?
                .max((speed * 4. / radius).ceil() as usize)
                .max(1);
            check(
                n <= 65536 && all.len().saturating_add(n) <= 4_000_000,
                "cylindrical trim budget exceeded",
            )?;
            let sq = (n as f64).powi(2);
            let mut prev = None;
            for j in 0..=n {
                let frac = j as f64 / n as f64;
                let p = curve.point(edge.interval[start] + dt * frac)?;
                let d = p - frame.origin;
                let rho = d.cross(frame.axis).length();
                check(
                    rho - speed / (2. * n as f64) >= radius / 2.
                        && (rho - radius).abs() + radial / (8. * sq) <= c.tolerance,
                    "edge leaves cylindrical support",
                )?;
                let reference = c.uv[0].x + (c.uv[1].x - c.uv[0].x) * frac;
                let q = cylinder_uv(frame, p, reference);
                // Keep the reference away from a branch change throughout
                // the interval, including when theta is not monotone.
                check(
                    (q.x - reference).abs()
                        + (2. * speed / radius + (c.uv[1].x - c.uv[0].x).abs()) / (2. * n as f64)
                        < std::f64::consts::PI,
                    "ambiguous cylindrical period",
                )?;
                if j == 0 || j == n {
                    check(
                        metric(q - c.uv[usize::from(j == n)]).length() <= c.tolerance,
                        "wrong cylindrical anchor",
                    )?;
                }
                let b = metric(q);
                if let Some(a) = prev {
                    all.push(Segment {
                        a,
                        b,
                        middle: None,
                        error: chart / (8. * sq),
                        accel: chart / sq,
                        ring: k,
                        index: corners.len(),
                    });
                    corners.push(a);
                }
                prev = Some(b);
            }
        }
        joins(&all[first..], corners.len())?;
        rings.push(corners);
    }
    separated(all, &rings)
}
