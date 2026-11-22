// SPDX-License-Identifier: GPL-3.0-or-later
//! Geometry reads in face order, independent of dense vertex indices.
use super::{cow::FxHashMap, keys::Kind, Store, INTERIOR};
use crate::facegeom::{triangles, Hypot, V3};

impl Store {
    pub fn face_triangles(&self, f: u32, normal: Option<[f32; 3]>, h: Hypot) -> Vec<[[f32; 3]; 3]> {
        let rings = self.face_rings(f);
        if rings.first().is_none_or(|r| r.len() < 3) {
            return Vec::new();
        }
        let mut pos = Vec::new();
        let local: Vec<Vec<u32>> = rings
            .iter()
            .map(|ring| {
                ring.iter()
                    .map(|&v| {
                        let [x, y, z] = *self.state.vpos.get(v as usize);
                        let i = pos.len() as u32;
                        pos.push(V3(x as f32, y as f32, z as f32));
                        i
                    })
                    .collect()
            })
            .collect();
        let refs: Vec<_> = local.iter().map(Vec::as_slice).collect();
        let n = normal
            .map(|[x, y, z]| V3(x, y, z))
            .unwrap_or_else(|| crate::orient::face_normal(&pos, refs[0], h));
        triangles(&pos, &refs, n, h)
            .iter()
            .map(|t| {
                t.map(|i| {
                    let V3(x, y, z) = pos[i as usize];
                    [x, y, z]
                })
            })
            .collect()
    }

    pub fn solid_volume(&self, h: Hypot) -> Option<f64> {
        let exterior = |f: u32| *self.state.fflags.get(f as usize) & INTERIOR == 0;
        for e in self.live_slots(Kind::Edge) {
            if self.edge_faces(e).iter().filter(|&&f| exterior(f)).count() != 2 {
                return None;
            }
        }
        let faces: Vec<_> = self
            .live_slots(Kind::Face)
            .into_iter()
            .filter(|&f| exterior(f))
            .collect();
        self.face_set_volume(&faces, h).filter(|v| *v > 1e-12)
    }

    pub fn face_set_volume(&self, faces: &[u32], h: Hypot) -> Option<f64> {
        let mut uses = FxHashMap::default();
        for &f in faces {
            for r in self.face_rings(f) {
                for (&a, &b) in r.iter().zip(r.iter().cycle().skip(1)) {
                    *uses.entry((a.min(b), a.max(b))).or_insert(0usize) += 1;
                }
            }
        }
        if uses.values().any(|&n| n != 2) {
            return None;
        }
        let rings = self.face_rings(*faces.first()?);
        let anchor = self.state.vpos.get(*rings.first()?.first()? as usize);
        let rel = |p: [f32; 3]| {
            [
                p[0] as f64 - anchor[0],
                p[1] as f64 - anchor[1],
                p[2] as f64 - anchor[2],
            ]
        };
        let terms = faces
            .iter()
            .flat_map(|&f| self.face_triangles(f, None, h))
            .map(|tri| {
                let [a, b, c] = tri.map(rel);
                (a[0] * (b[1] * c[2] - b[2] * c[1])
                    + a[1] * (b[2] * c[0] - b[0] * c[2])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]))
                    / 6.0
            });
        Some(crate::facegeom::py_sum(terms, true).abs())
    }
}
