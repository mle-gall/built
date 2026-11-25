// SPDX-License-Identifier: GPL-3.0-or-later
//! Identify the store vertices a transform can preview in the resident
//! prototype without retriangulation or changing any shared vertex identity.
use crate::store::{keys::Kind, Store};
use std::collections::{HashMap, HashSet};
pub(crate) fn preview_points(
    s: &Store,
    keys: Vec<u128>,
    origin: [f64; 3],
) -> Option<Vec<[f64; 3]>> {
    let vertices: HashSet<u32> = keys
        .into_iter()
        .map(|key| s.slot(Kind::Vertex, key))
        .collect::<Option<_>>()?;
    let faces: HashSet<_> = vertices
        .iter()
        .flat_map(|&v| s.edges_at(v))
        .flat_map(|e| s.edge_faces(e).iter().copied())
        .collect();
    for f in faces {
        if !s
            .face_rings(f)
            .iter()
            .flatten()
            .all(|v| vertices.contains(v))
        {
            return None;
        }
    }
    let packed = |p: [f64; 3]| -> [u32; 3] {
        std::array::from_fn(|i| {
            let f = (p[i] - origin[i]) as f32;
            if f == 0. {
                0
            } else {
                f.to_bits()
            }
        })
    };
    let mut names = HashMap::new();
    let mut points = Vec::new();
    for &v in &vertices {
        let p = *s.state.vpos.get(v as usize);
        if names.insert(packed(p), v).is_some() {
            return None;
        }
        points.push(p)
    }
    for v in s.state.slots[Kind::Vertex as usize].live_slots() {
        if !vertices.contains(&v) && names.contains_key(&packed(*s.state.vpos.get(v as usize))) {
            return None;
        }
    }
    Some(points)
}
