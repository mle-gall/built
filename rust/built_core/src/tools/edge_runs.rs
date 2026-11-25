// SPDX-License-Identifier: GPL-3.0-or-later
//! Simple selected paths and degree-two runs shared by Offset and Follow Me.
use crate::{
    math::{xyz, Vec3},
    store::Store,
};
use glam::DVec3;
use pyo3::prelude::*;
use std::collections::{HashMap, HashSet};

#[pyfunction]
pub fn ordered_edge_path(edges: &Bound<'_, PyAny>) -> PyResult<Option<(Vec<Vec3>, bool)>> {
    let mut adj: HashMap<i128, Vec<(i128, i128)>> = HashMap::new();
    let mut points = HashMap::new();
    let mut order = Vec::new();
    let mut count = 0;
    for edge in edges.try_iter()? {
        let edge = edge?;
        let id = edge.getattr("key")?.extract()?;
        let [a, b] = ["v0", "v1"].map(|name| -> PyResult<i128> {
            let v = edge.getattr(name)?;
            let key = v.getattr("key")?.extract()?;
            if let std::collections::hash_map::Entry::Vacant(entry) = points.entry(key) {
                entry.insert(xyz(&v.getattr("position")?)?);
                order.push(key);
            }
            Ok(key)
        });
        let (a, b) = (a?, b?);
        adj.entry(a).or_default().push((id, b));
        adj.entry(b).or_default().push((id, a));
        count += 1;
    }
    if order.is_empty() || adj.values().any(|v| v.len() > 2) {
        return Ok(None);
    }
    let ends: Vec<_> = order
        .iter()
        .copied()
        .filter(|v| adj[v].len() == 1)
        .collect();
    if !matches!(ends.len(), 0 | 2) {
        return Ok(None);
    }
    let closed = ends.is_empty();
    let start = ends.first().copied().unwrap_or(order[0]);
    let mut vertex = start;
    let mut path = vec![Vec3::from(points[&start])];
    let mut seen = HashSet::new();
    while let Some(&(edge, next)) = adj[&vertex].iter().find(|(e, _)| !seen.contains(e)) {
        seen.insert(edge);
        vertex = next;
        if closed && vertex == start {
            break;
        }
        path.push(points[&vertex].into());
    }
    Ok((seen.len() == count).then_some((path, closed)))
}

#[pyfunction]
pub fn outline_normal(points: &Bound<'_, PyAny>) -> PyResult<Option<Vec3>> {
    let p = points
        .try_iter()?
        .map(|p| xyz(&p?))
        .collect::<PyResult<Vec<_>>>()?;
    if p.len() < 3 {
        return Ok(None);
    }
    let mut n = DVec3::ZERO;
    for (a, b) in p.iter().zip(p.iter().cycle().skip(1)) {
        n += DVec3::new(
            (a.y - b.y) * (a.z + b.z),
            (a.z - b.z) * (a.x + b.x),
            (a.x - b.x) * (a.y + b.y),
        );
    }
    let span2 = p
        .iter()
        .map(|q| q.distance_squared(p[0]))
        .fold(0., f64::max);
    Ok((span2 > 0. && n.length() >= 1e-6 * span2).then(|| n.normalize().into()))
}

pub(crate) fn store_run(store: &Store, seed: u32) -> (Vec<Vec3>, bool) {
    let [a, b] = *store.state.ends.get(seed as usize);
    let point = |v: u32| Vec3::from(DVec3::from(*store.state.vpos.get(v as usize)));
    let walk = |mut v: u32| {
        let mut path = Vec::new();
        let mut prev = seed;
        loop {
            let mut edges = store.edges_at(v);
            let (Some(first), Some(second), None) = (edges.next(), edges.next(), edges.next())
            else {
                break;
            };
            let edge = if first == prev { second } else { first };
            if edge == seed {
                return (path, true);
            }
            let [p, q] = *store.state.ends.get(edge as usize);
            v = if p == v { q } else { p };
            prev = edge;
            path.push(point(v));
        }
        (path, false)
    };
    let (mut forwards, closed) = walk(b);
    let mut path = if closed {
        forwards.pop();
        Vec::new()
    } else {
        let (mut back, _) = walk(a);
        back.reverse();
        back
    };
    path.extend([point(a), point(b)]);
    path.extend(forwards);
    (path, closed)
}
