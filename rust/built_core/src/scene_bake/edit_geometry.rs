// SPDX-License-Identifier: GPL-3.0-or-later
//! Geometry dependencies of the preceding editable prototype. Compare the
//! welded source arrays, including exact display facets and filtered rings,
//! so edits, undo and a change of material variant use the same rules.
use super::{Hypot, V3};
use std::collections::HashMap;

pub(super) struct GeometryMemo {
    pub h: Hypot,
    pos: Vec<V3>,
    corners: Vec<u32>,
    sizes: Vec<i64>,
    offsets: Vec<(usize, usize)>,
    triangles: Vec<[u32; 3]>,
    starts: Vec<usize>,
    normals: Vec<V3>,
    pub material: Option<MaterialMemo>,
    pub soft_edges: Vec<[u32; 2]>,
    pub fans: HashMap<(usize, usize), crate::store::geometry::V3>,
}

pub(super) struct Changes {
    pub faces: Vec<bool>,
    pub vertices: Vec<bool>,
}

impl GeometryMemo {
    pub fn new(
        p: &[V3],
        corners: &[u32],
        sizes: &[i64],
        counts: &[i64],
        offsets: &[(usize, usize)],
        h: Hypot,
        geometry: &[(std::borrow::Cow<'_, [[u32; 3]]>, V3)],
    ) -> Self {
        let mut offsets = offsets.to_vec();
        offsets.push((sizes.len(), corners.len()));
        let mut triangles = Vec::new();
        let mut starts = Vec::with_capacity(counts.len() + 1);
        let mut normals = Vec::with_capacity(counts.len());
        for (tris, normal) in geometry {
            starts.push(triangles.len());
            triangles.extend_from_slice(tris);
            normals.push(*normal);
        }
        starts.push(triangles.len());
        Self {
            h,
            pos: p.to_vec(),
            corners: corners.to_vec(),
            sizes: sizes.to_vec(),
            offsets,
            triangles,
            starts,
            normals,
            material: None,
            soft_edges: Vec::new(),
            fans: HashMap::new(),
        }
    }

    pub fn face(&self, f: usize) -> (std::borrow::Cow<'_, [[u32; 3]]>, V3) {
        (
            std::borrow::Cow::Borrowed(&self.triangles[self.starts[f]..self.starts[f + 1]]),
            self.normals[f],
        )
    }

    pub fn changes(
        &self,
        p: &[V3],
        corners: &[u32],
        sizes: &[i64],
        counts: &[i64],
        offsets: &[(usize, usize)],
    ) -> Changes {
        let bits = |v: V3| [v.0.to_bits(), v.1.to_bits(), v.2.to_bits()];
        let moved: Vec<_> = p
            .iter()
            .enumerate()
            .map(|(i, v)| self.pos.get(i).is_none_or(|old| bits(*old) != bits(*v)))
            .collect();
        let mut vertices = moved.clone();
        let mut faces = vec![true; counts.len()];
        for f in 0..self.normals.len().max(counts.len()) {
            let old = self.offsets.get(f..=f + 1).map(|pair| {
                (
                    &self.sizes[pair[0].0..pair[1].0],
                    &self.corners[pair[0].1..pair[1].1],
                )
            });
            let new = offsets.get(f).map(|&(r, c)| {
                let end = offsets
                    .get(f + 1)
                    .copied()
                    .unwrap_or((sizes.len(), corners.len()));
                (&sizes[r..end.0], &corners[c..end.1])
            });
            if new.is_some() && old == new && !new.unwrap().1.iter().any(|&v| moved[v as usize]) {
                faces[f] = false;
            } else {
                for &v in old.into_iter().chain(new).flat_map(|(_, corners)| corners) {
                    if let Some(dirty) = vertices.get_mut(v as usize) {
                        *dirty = true;
                    }
                }
            }
        }
        Changes { faces, vertices }
    }

    pub fn changed_edges(&self, edges: &[[u32; 2]], soft: &[bool], mask: &mut [bool]) {
        // Edge order matters to the summation of a fan. A mismatched pair
        // invalidates both endpoints even if both edges are still soft.
        let mut old = self.soft_edges.iter();
        let mut new = edges.iter().zip(soft).filter(|(_, s)| **s).map(|(e, _)| e);
        loop {
            let pair = (old.next(), new.next());
            if pair == (None, None) {
                break;
            }
            if pair.0 != pair.1 {
                for &v in pair.0.into_iter().chain(pair.1).flatten() {
                    if let Some(dirty) = mask.get_mut(v as usize) {
                        *dirty = true;
                    }
                }
            }
        }
    }
}

/// A face's material rows depend on its triangulation, positions, both UV
/// mappings and both paints. Offsets are local to the preceding prototype.
#[derive(Clone, Copy, Default)]
pub(super) struct FaceMaterial {
    pub mapping: [u32; 16],
    pub paints: [u32; 3],
    pub start: usize,
    pub end: usize,
}

#[derive(Default)]
pub(super) struct MaterialMemo {
    pub faces: Vec<FaceMaterial>,
    pub rows: Vec<u8>,
}
