// SPDX-License-Identifier: GPL-3.0-or-later
//! Angle softening plans preserve edge order, including hard edges whose
//! obsolete flat flag must be cleared. Material equality is host-defined.
use glam::DVec3;
use std::collections::HashMap;
use std::hash::Hash;

pub(super) type Flags = (bool, bool);
pub(super) type Change<K> = (K, Flags, Flags);
pub(super) type Edge<K> = (K, Flags, [K; 2]);

pub(super) struct Options {
    pub angle: f64,
    pub coplanar: bool,
    pub harden: bool,
    pub smooth: bool,
}

impl Options {
    pub fn plan<K: Eq + Hash + Copy, E>(
        &self,
        edges: impl IntoIterator<Item = Edge<K>>,
        mut material_matches: impl FnMut(K, K) -> Result<bool, E>,
        mut normal: impl FnMut(K) -> Result<[f64; 3], E>,
    ) -> Result<Vec<Change<K>>, E> {
        let mut changes = Vec::new();
        let mut normals = HashMap::new();
        let mut face_normal = |key| -> Result<Option<DVec3>, E> {
            if let Some(n) = normals.get(&key) {
                return Ok(*n);
            }
            let v = DVec3::from_array(normal(key)?);
            let n = (v.length() >= 1e-12).then(|| v.normalize());
            normals.insert(key, n);
            Ok(n)
        };
        for (edge, before, [a, b]) in edges {
            let soft = if self.harden {
                false
            } else if a == b {
                continue;
            } else if !material_matches(a, b)? {
                false
            } else {
                let (Some(a), Some(b)) = (face_normal(a)?, face_normal(b)?) else {
                    continue;
                };
                let dot = (a.x * b.x + a.y * b.y + a.z * b.z).clamp(-1., 1.).abs();
                let angle = dot.acos().to_degrees();
                if angle <= 1e-4 {
                    self.coplanar
                } else {
                    angle <= self.angle + 1e-5
                }
            };
            let after = (soft, soft && !self.smooth);
            if before != after {
                changes.push((edge, before, after));
            }
        }
        Ok(changes)
    }
}
