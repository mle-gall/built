// SPDX-License-Identifier: GPL-3.0-or-later
//! Direct conforming tessellation of a complete cone/cylinder band and its
//! two planar caps. Both rims and both uses of the seam share vertex indices.
use super::{
    patch::{Coedge, Face, Mesh, Topology},
    *,
};
use glam::DVec2;
use std::{collections::HashMap, f64::consts::TAU};

fn triangle(m: &mut Mesh, f: &Face, mut indices: [usize; 3], mut uv: [DVec2; 3]) {
    if ((uv[1] - uv[0]).perp_dot(uv[2] - uv[0]) < 0.) != f.reversed {
        indices.swap(1, 2);
        uv.swap(1, 2);
    }
    m.triangles.push(indices);
    m.parameters.push(uv);
    m.face_ids.push(f.id);
}
pub(super) fn capped(
    t: &Topology,
    rows: &Rows,
    angle: Option<f64>,
    tol: f64,
) -> Result<Option<Mesh>> {
    if t.faces.len() != 3 || t.edges.len() != 3 || t.vertices.len() != 2 {
        return Ok(None);
    }
    let Some(w) = t.faces.iter().find(|f| {
        matches!(
            rows.surfaces[f.surface as usize],
            Surface::Cone { .. } | Surface::Cylinder { .. }
        )
    }) else {
        return Ok(None);
    };
    let [loop_] = w.loops.as_slice() else {
        return Ok(None);
    };
    let [a, b, c, d] = loop_.as_slice() else {
        return Ok(None);
    };
    let y0 = a.uv[0].y;
    let y1 = c.uv[0].y;
    let x0 = a.uv[0].x;
    let x1 = a.uv[1].x;
    if a.edge == c.edge
        || b.edge != d.edge
        || a.uv[1].y != y0
        || c.uv[1].y != y1
        || b.uv[1].x != x1
        || d.uv[0].x != x0
        || ((x1 - x0).abs() - TAU).abs() > 1e-12
    {
        return Ok(None);
    }
    let edge = t.edge_index();
    let use_edge = |u: &Coedge| u.edge.and_then(|id| edge.get(&id).copied());
    let (Some(bottom), Some(seam), Some(top)) = (use_edge(a), use_edge(b), use_edge(c)) else {
        return Ok(None);
    };
    let radius = |i: u32| match &rows.curves[i as usize] {
        Curve::Circle { radius, .. } => Some(*radius),
        _ => None,
    };
    let (Some(r0), Some(r1)) = (radius(bottom.curve), radius(top.curve)) else {
        return Ok(None);
    };
    if !matches!(rows.curves[seam.curve as usize], Curve::Line { .. }) {
        return Ok(None);
    }
    let ends: Vec<_> = t.faces.iter().filter(|f| f.id != w.id).collect();
    let cap = |f: &Face| match f.loops.as_slice() {
        [r] => match r.as_slice() {
            [u] if u.projected => u.edge,
            _ => None,
        },
        _ => None,
    };
    if !ends
        .iter()
        .all(|f| matches!(rows.surfaces[f.surface as usize], Surface::Plane { .. }))
        || ![bottom.id, top.id]
            .iter()
            .all(|id| ends.iter().any(|f| cap(f) == Some(*id)))
    {
        return Ok(None);
    }
    // At a barycentric point, angular interpolation contributes at most
    // max(r) Δu²/8. Linear radius variation contributes at most Δr Δu/4
    // by the covariance bound for two variables in finite intervals.
    let width = (x1 - x0).abs();
    let n = ((r0.max(r1) * width * width / (2. * tol)).sqrt().ceil() as usize)
        .max(angle.map_or(1, |lim| (width / lim).ceil() as usize))
        .max(8);
    let k = (((r1 - r0).abs() * width / (n as f64 * tol)).ceil() as usize).max(1);
    if n.saturating_mul(k.saturating_add(1)) > 4_000_000 {
        return Err("tessellation vertex budget exceeded");
    }
    let mut m = Mesh::default();
    let mut verts = HashMap::new();
    for (&id, p) in &t.vertices {
        verts.insert(id, m.points.len());
        m.points.push(p.to_array());
    }
    for (e, count) in [(bottom, n), (top, n), (seam, k)] {
        let mut ids = vec![verts[&e.vertices[0]]];
        for i in 1..count {
            ids.push(m.points.len());
            let s = e.interval[0] + (e.interval[1] - e.interval[0]) * (i as f64 / count as f64);
            m.points
                .push(rows.curves[e.curve as usize].point(s)?.to_array());
        }
        ids.push(verts[&e.vertices[1]]);
        m.edge_samples.insert(e.id, ids);
    }
    let surface = &rows.surfaces[w.surface as usize];
    let uv = |i: usize, j: usize| {
        DVec2::new(
            x0 + (x1 - x0) * (i as f64 / n as f64),
            y0 + (y1 - y0) * (j as f64 / k as f64),
        )
    };
    let mut previous = Vec::new();
    for j in 0..=k {
        let mut current = Vec::with_capacity(n + 1);
        for i in 0..=n {
            let q = uv(i, j);
            let p = surface.point(&rows.curves, q.x, q.y)?;
            let id = if j == 0 {
                m.edge_samples[&bottom.id][if a.reversed { n - i } else { i }]
            } else if j == k {
                m.edge_samples[&top.id][if c.reversed { i } else { n - i }]
            } else if i == 0 || i == n {
                m.edge_samples[&seam.id][if b.reversed { k - j } else { j }]
            } else {
                let id = m.points.len();
                m.points.push(p.to_array());
                id
            };
            if DVec3::from(m.points[id]).distance(p) > tol * 0.25 {
                return Ok(None);
            }
            current.push(id);
        }
        if j > 0 {
            for i in 0..n {
                triangle(
                    &mut m,
                    w,
                    [previous[i], previous[i + 1], current[i + 1]],
                    [uv(i, j - 1), uv(i + 1, j - 1), uv(i + 1, j)],
                );
                triangle(
                    &mut m,
                    w,
                    [previous[i], current[i + 1], current[i]],
                    [uv(i, j - 1), uv(i + 1, j), uv(i, j)],
                );
            }
        }
        previous = current;
    }
    for f in ends {
        let id = cap(f).unwrap();
        let Curve::Circle { frame: circle, .. } = &rows.curves[edge[&id].curve as usize] else {
            unreachable!()
        };
        let Surface::Plane { frame } = &rows.surfaces[f.surface as usize] else {
            unreachable!()
        };
        let center = m.points.len();
        m.points.push(circle.origin.to_array());
        let sample = m.edge_samples[&id].clone();
        for pair in sample.windows(2) {
            let tri = [center, pair[0], pair[1]];
            let projected = tri.map(|i| {
                let d = DVec3::from(m.points[i]) - frame.origin;
                DVec2::new(d.dot(frame.reference), d.dot(frame.y()))
            });
            triangle(&mut m, f, tri, projected);
        }
    }
    Ok(Some(m))
}

/// A regular untrimmed parameter rectangle has a conforming tensor grid.
pub(super) fn rectangle(
    t: &Topology,
    rows: &Rows,
    angle: Option<f64>,
    tol: f64,
) -> Result<Option<Mesh>> {
    let [f] = t.faces.as_slice() else {
        return Ok(None);
    };
    let [ring] = f.loops.as_slice() else {
        return Ok(None);
    };
    let [a, b, c, d] = ring.as_slice() else {
        return Ok(None);
    };
    if t.edges.len() != 4
        || t.vertices.len() != 4
        || ring.iter().any(|u| u.projected || u.edge.is_none())
    {
        return Ok(None);
    }
    let surface = &rows.surfaces[f.surface as usize];
    if !matches!(
        surface,
        Surface::Spline { .. }
            | Surface::Extrusion { .. }
            | Surface::Revolution { .. }
            | Surface::Torus { .. }
            | Surface::Sphere { .. }
    ) {
        return Ok(None);
    }
    let origin = a.uv[0];
    let u = a.uv[1] - origin;
    let v = b.uv[1] - b.uv[0];
    if u.dot(v).abs() > 1e-13 * u.length() * v.length()
        || u.x.abs().min(u.y.abs()) > 1e-13 * u.length()
        || v.x.abs().min(v.y.abs()) > 1e-13 * v.length()
        || c.uv[1].distance(origin + v) > 1e-13
        || d.uv[1].distance(origin) > 1e-13
    {
        return Ok(None);
    }
    let lo = origin.min(c.uv[0]);
    let hi = origin.max(c.uv[0]);
    let bound = surface.acceleration(&rows.curves, lo, hi)?;
    let n = ((bound / tol).sqrt().ceil() as usize)
        .max(surface.angular_steps(lo, hi, angle)?)
        .max(1);
    if n.saturating_add(1).saturating_pow(2) > 4_000_000 {
        return Err("tessellation vertex budget exceeded");
    }
    let mut m = Mesh::default();
    let mut verts = HashMap::new();
    for (&id, p) in &t.vertices {
        verts.insert(id, m.points.len());
        m.points.push(p.to_array());
    }
    for e in &t.edges {
        let mut ids = vec![verts[&e.vertices[0]]];
        for i in 1..n {
            ids.push(m.points.len());
            m.points.push(
                rows.curves[e.curve as usize]
                    .point(e.interval[0] + (e.interval[1] - e.interval[0]) * i as f64 / n as f64)?
                    .to_array(),
            );
        }
        ids.push(verts[&e.vertices[1]]);
        m.edge_samples.insert(e.id, ids);
    }
    let uv = |i: usize, j: usize| origin + u * (i as f64 / n as f64) + v * (j as f64 / n as f64);
    let mut previous = Vec::new();
    for j in 0..=n {
        let mut current = Vec::with_capacity(n + 1);
        for i in 0..=n {
            let q = uv(i, j);
            let p = surface.point(&rows.curves, q.x, q.y)?;
            let side = if j == 0 {
                Some((a, i))
            } else if i == n {
                Some((b, j))
            } else if j == n {
                Some((c, n - i))
            } else if i == 0 {
                Some((d, n - j))
            } else {
                None
            };
            let id = if let Some((e, h)) = side {
                m.edge_samples[&e.edge.unwrap()][if e.reversed { n - h } else { h }]
            } else {
                let id = m.points.len();
                m.points.push(p.to_array());
                id
            };
            if p.distance(m.points[id].into()) > tol / 2. {
                return Ok(None);
            }
            current.push(id);
        }
        if j > 0 {
            for i in 0..n {
                triangle(
                    &mut m,
                    f,
                    [previous[i], previous[i + 1], current[i + 1]],
                    [uv(i, j - 1), uv(i + 1, j - 1), uv(i + 1, j)],
                );
                triangle(
                    &mut m,
                    f,
                    [previous[i], current[i + 1], current[i]],
                    [uv(i, j - 1), uv(i + 1, j), uv(i, j)],
                );
            }
        }
        previous = current;
    }
    Ok(Some(m))
}

/// Triangulate an axis-aligned band directly between the two ordered rim
/// polylines. Their vertices may come from several original arc segments.
pub(super) fn strip(
    m: &mut Mesh,
    rows: &Rows,
    f: &Face,
    edges: &std::collections::BTreeMap<u64, Vec<usize>>,
    tol: f64,
    angle: Option<f64>,
) -> Result<bool> {
    let surface = &rows.surfaces[f.surface as usize];
    let (radius, slope) = match surface {
        Surface::Cylinder { radius, .. } => (*radius, 0.),
        Surface::Cone {
            radius, half_angle, ..
        } => (*radius, half_angle.sin()),
        _ => return Ok(false),
    };
    let [ring] = f.loops.as_slice() else {
        return Ok(false);
    };
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for c in ring {
        if c.projected || c.edge.is_none() {
            return Ok(false);
        }
        lo = lo.min(c.uv[0].y);
        hi = hi.max(c.uv[0].y);
    }
    if hi <= lo {
        return Ok(false);
    }
    let mut rims = [Vec::<(DVec2, usize)>::new(), Vec::new()];
    let mut sides = Vec::new();
    for c in ring {
        let [a, b] = c.uv;
        let sample = &edges[&c.edge.unwrap()];
        if (a.y - b.y).abs() > 1e-12 {
            if (a.x - b.x).abs() > 1e-12 || sample.len() != 2 {
                return Ok(false);
            }
            sides.push(a.x);
            continue;
        }
        let side = if (a.y - lo).abs() < 1e-12 {
            0
        } else if (a.y - hi).abs() < 1e-12 {
            1
        } else {
            return Ok(false);
        };
        let n = sample.len() - 1;
        for i in 0..=n {
            let q = if i == n {
                b
            } else {
                a + (b - a) * (i as f64 / n as f64)
            };
            let id = sample[if c.reversed { n - i } else { i }];
            if DVec3::from(m.points[id]).distance(surface.point(&rows.curves, q.x, q.y)?) > tol / 4.
            {
                return Ok(false);
            }
            rims[side].push((q, id));
        }
    }
    if sides.len() != 2 {
        return Ok(false);
    }
    for rim in &mut rims {
        rim.sort_by(|a, b| a.0.x.total_cmp(&b.0.x));
        rim.dedup_by(|a, b| a.1 == b.1 && (a.0.x - b.0.x).abs() < 1e-12);
    }
    if rims.iter().any(|r| r.len() < 2) {
        return Ok(false);
    }
    let (l, h) = (&rims[0], &rims[1]);
    if (l[0].0.x - h[0].0.x).abs() > 1e-12
        || (l.last().unwrap().0.x - h.last().unwrap().0.x).abs() > 1e-12
    {
        return Ok(false);
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i + 1 < l.len() || j + 1 < h.len() {
        let mut t = [l[i], h[j], h[j]];
        if j + 1 == h.len() || (i + 1 < l.len() && l[i + 1].0.x <= h[j + 1].0.x) {
            i += 1;
            t[2] = l[i]
        } else {
            j += 1;
            t[2] = h[j]
        }
        let width = t.iter().map(|p| p.0.x).fold(f64::NEG_INFINITY, f64::max)
            - t.iter().map(|p| p.0.x).fold(f64::INFINITY, f64::min);
        let r = (radius + lo * slope).abs().max((radius + hi * slope).abs());
        if r * width.powi(2) / 8. + (hi - lo) * slope.abs() * width / 4. > tol * 0.75
            || angle.is_some_and(|a| width > a)
        {
            return Ok(false);
        }
        out.push(t);
    }
    for t in out {
        triangle(m, f, t.map(|p| p.1), t.map(|p| p.0));
    }
    Ok(true)
}
