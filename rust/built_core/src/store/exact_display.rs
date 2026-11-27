// SPDX-License-Identifier: GPL-3.0-or-later
//! Optional shading normals for the editable facets. Ring corners, edge
//! flags, positions and ownership all come from the ordinary dense mesh.
use super::{keys::Kind, Store, FLAT, SOFT};
use glam::DVec3;

impl Store {
    pub(crate) fn facet_normals(&self, corner_count: usize) -> Option<Vec<[f64; 3]>> {
        let curved = self.state.exact.curved.as_ref()?;
        if curved.faces.is_empty() {
            return None;
        }
        let mut normals = vec![[f64::NAN; 3]; corner_count];
        let mut at = 0;
        for f in self.state.order[2].live(&self.state.slots[Kind::Face as usize]) {
            if let Some(row) = self.face_facet_normals(f) {
                normals[at..at + row.len()].copy_from_slice(&row);
                at += row.len();
            } else {
                let rings = self.state.frings.get(f as usize);
                at += (0..rings.n_rings)
                    .map(|i| *self.state.ring_sizes.get((rings.ring_start + i) as usize) as usize)
                    .sum::<usize>();
            }
        }
        Some(normals)
    }
    pub(crate) fn face_facet_normals(&self, f: u32) -> Option<Vec<[f64; 3]>> {
        let mut out = Vec::new();
        let support = self.curved_face(f)?;
        let rings = self.sides(f);
        let outer = &rings[0];
        let origin = DVec3::from(*self.state.vpos.get(outer[0].a as usize));
        let point = |v: u32| DVec3::from(*self.state.vpos.get(v as usize)) - origin;
        let face_normal: DVec3 = outer.iter().map(|s| point(s.a).cross(point(s.b))).sum();
        for ring in rings {
            let smooth = |e: Option<u32>| {
                e.is_some_and(|e| self.state.eflags.get(e as usize) & (SOFT | FLAT) == SOFT)
            };
            for (i, side) in ring.iter().enumerate() {
                let mut normal = [f64::NAN; 3];
                if smooth(side.edge) || smooth(ring[(i + ring.len() - 1) % ring.len()].edge) {
                    if let Some(n) = support
                        .uv(self.def_point(side.a))
                        .and_then(|uv| support.normal(uv))
                    {
                        let n = self.state.frame.model_normal(n);
                        normal = if n.dot(face_normal) < 0. { -n } else { n }.to_array();
                    }
                }
                out.push(normal);
            }
        }
        Some(out)
    }
}
