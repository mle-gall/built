// SPDX-License-Identifier: GPL-3.0-or-later
//! Remove internal soft boundaries only when a connected set of faces shares
//! one supporting plane. Keep the source faces if the union is ambiguous.
use built_step::planar::Body;
use glam::DVec3;
use std::collections::{BTreeMap, HashMap, HashSet};

type Edge = (usize, usize);
fn key(a: usize, b: usize) -> Edge {
    (a.min(b), a.max(b))
}

pub(super) fn merge(body: &mut Body, soft: &HashSet<Edge>) {
    if soft.is_empty() {
        return;
    }
    let planes: Vec<_> = body
        .faces
        .iter()
        .map(|r| {
            body.plane(r)
                .ok()
                .map(|(f, _)| (DVec3::from(f.origin) / 1000., DVec3::from(f.axis)))
        })
        .collect();
    let mut sides = HashMap::new();
    let mut adjacent = vec![Vec::new(); body.faces.len()];
    for (f, rings) in body.faces.iter().enumerate() {
        for r in rings {
            for (&a, &b) in r.iter().zip(r.iter().cycle().skip(1)) {
                if soft.contains(&key(a, b)) {
                    if let Some(other) = sides.insert(key(a, b), f) {
                        adjacent[f].push(other);
                        adjacent[other].push(f);
                    }
                }
            }
        }
    }
    let mut visited = vec![false; body.faces.len()];
    let mut result = Vec::new();
    for root in 0..body.faces.len() {
        if visited[root] {
            continue;
        }
        visited[root] = true;
        let mut group = vec![root];
        if let Some((origin, z)) = planes[root] {
            let mut i = 0;
            while i < group.len() {
                for &f in &adjacent[group[i]] {
                    if !visited[f]
                        && planes[f]
                            .is_some_and(|(_, n)| n.dot(z) > 0. && n.cross(z).length() <= 1e-10)
                        && body.faces[f]
                            .iter()
                            .flatten()
                            .all(|&v| (body.points[v] - origin).dot(z).abs() <= 1e-10)
                    {
                        group.push(f);
                        visited[f] = true;
                    }
                }
                i += 1;
            }
            if group.len() > 1 {
                if let Some(rings) = boundary(body, &group, soft, z) {
                    result.push(rings);
                    continue;
                }
            }
        }
        for f in group {
            result.push(std::mem::take(&mut body.faces[f]));
        }
    }
    body.faces = result;
}

fn boundary(
    body: &Body,
    group: &[usize],
    soft: &HashSet<Edge>,
    normal: DVec3,
) -> Option<Vec<Vec<usize>>> {
    let mut edges = HashMap::new();
    for &f in group {
        for r in &body.faces[f] {
            for (&a, &b) in r.iter().zip(r.iter().cycle().skip(1)) {
                if let Some(previous) = edges.remove(&key(a, b)) {
                    if previous != (b, a) || !soft.contains(&key(a, b)) {
                        return None;
                    }
                } else {
                    edges.insert(key(a, b), (a, b));
                }
            }
        }
    }
    let mut next = BTreeMap::new();
    for (_, (a, b)) in edges {
        if next.insert(a, b).is_some() {
            return None;
        }
    }
    let mut outer = None;
    let mut holes = Vec::new();
    while let Some((&start, _)) = next.first_key_value() {
        let mut ring = Vec::new();
        let mut at = start;
        loop {
            ring.push(at);
            at = next.remove(&at)?;
            if at == start {
                break;
            }
        }
        if ring.len() < 3 {
            return None;
        }
        let o = body.points[start];
        let oriented = ring[1..]
            .windows(2)
            .map(|pair| {
                (body.points[pair[0]] - o)
                    .cross(body.points[pair[1]] - o)
                    .dot(normal)
            })
            .sum::<f64>();
        if oriented > 0. {
            if outer.replace(ring).is_some() {
                return None;
            }
        } else if oriented < 0. {
            holes.push(ring)
        } else {
            return None;
        }
    }
    let mut rings = vec![outer?];
    rings.extend(holes);
    body.plane(&rings).ok()?;
    Some(rings)
}
