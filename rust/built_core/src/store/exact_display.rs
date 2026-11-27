// SPDX-License-Identifier: GPL-3.0-or-later
//! Optional shading normals for the editable facets. Ring corners, edge
//! flags, positions and ownership all come from the ordinary dense mesh.
//!
//! A facet with a curved support shades from the support's normal at its
//! smooth corners, and on a seam into a planar face (a fillet's tangent
//! line: soft, smooth normals off). A planar face beside it across a smooth
//! edge (the side of a rounded slot, a fillet seam smoothed by hand) is a
//! plane: it keeps its own normal at those corners, so the soft-edge fan
//! never bends a flat face. At a tangent seam the support's normal is the
//! plane's anyway.
use super::{keys::Kind, Store, FLAT, SOFT};
use glam::DVec3;
use std::collections::HashSet;

impl Store {
    fn smooth_edge(&self, e: Option<u32>) -> bool {
        e.is_some_and(|e| self.state.eflags.get(e as usize) & (SOFT | FLAT) == SOFT)
    }

    /// An edge from face `f` into a face without a curved support.
    fn into_planar(&self, f: u32, e: u32) -> bool {
        self.edge_faces(e).iter().any(|&g| g != f && self.curved_face(g).is_none())
    }

    fn facet_ring_normal(&self, outer: &[super::exact_bound::Side]) -> DVec3 {
        let origin = DVec3::from(*self.state.vpos.get(outer[0].a as usize));
        let point = |v: u32| DVec3::from(*self.state.vpos.get(v as usize)) - origin;
        outer.iter().map(|s| point(s.a).cross(point(s.b))).sum()
    }

    pub(crate) fn facet_normals(&self, corner_count: usize) -> Option<Vec<[f64; 3]>> {
        let curved = self.state.exact.curved.as_ref()?;
        if curved.faces.is_empty() {
            return None;
        }
        // Planar faces across a smooth edge from a curved support, found
        // from the bound faces only (a few), not from every face.
        let mut beside: HashSet<u32> = HashSet::new();
        for (&f, _) in curved.faces.iter() {
            if !self.alive(Kind::Face, f) {
                continue;
            }
            for e in self.sides(f).into_iter().flatten().filter_map(|s| s.edge) {
                if self.smooth_edge(Some(e)) {
                    beside.extend(
                        self.edge_faces(e).iter().filter(|&&g| g != f && self.curved_face(g).is_none()),
                    );
                }
            }
        }
        let mut normals = vec![[f64::NAN; 3]; corner_count];
        let mut at = 0;
        for f in self.state.order[2].live(&self.state.slots[Kind::Face as usize]) {
            let row = if self.curved_face(f).is_some() || beside.contains(&f) {
                self.face_facet_normals(f)
            } else {
                None
            };
            if let Some(row) = row {
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

    /// One face's corner normals in ring order (NaN: the soft-edge fan or
    /// the face normal decides), or None when the face has none to give.
    pub(crate) fn face_facet_normals(&self, f: u32) -> Option<Vec<[f64; 3]>> {
        let Some(support) = self.curved_face(f) else {
            return self.planar_seam_normals(f);
        };
        let mut out = Vec::new();
        let rings = self.sides(f);
        let face_normal = self.facet_ring_normal(&rings[0]);
        // A seam into a planar face that is soft with smooth normals off
        // (a fillet's tangent line): the support's normal there is the
        // plane's, so the strip takes it and meets the plane without a crease.
        let shaded = |e: Option<u32>| {
            self.smooth_edge(e)
                || e.is_some_and(|e| {
                    self.state.eflags.get(e as usize) & (SOFT | FLAT) == SOFT | FLAT
                        && self.into_planar(f, e)
                })
        };
        for ring in rings {
            for (i, side) in ring.iter().enumerate() {
                let mut normal = [f64::NAN; 3];
                if shaded(side.edge) || shaded(ring[(i + ring.len() - 1) % ring.len()].edge) {
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

    /// A face without a curved support, on a smooth edge into one: its own
    /// plane's normal at the corners on such edges; None when it has none.
    fn planar_seam_normals(&self, f: u32) -> Option<Vec<[f64; 3]>> {
        let rings = self.sides(f);
        let seam = |e: Option<u32>| {
            e.is_some_and(|e| {
                self.smooth_edge(Some(e))
                    && self.edge_faces(e).iter().any(|&g| g != f && self.curved_face(g).is_some())
            })
        };
        if !rings.iter().flatten().any(|s| seam(s.edge)) {
            return None;
        }
        let flat = self.facet_ring_normal(&rings[0]).try_normalize();
        let mut out = Vec::new();
        for ring in &rings {
            for (i, side) in ring.iter().enumerate() {
                let before = ring[(i + ring.len() - 1) % ring.len()].edge;
                out.push(match flat.filter(|_| seam(side.edge) || seam(before)) {
                    Some(n) => n.to_array(),
                    None => [f64::NAN; 3],
                });
            }
        }
        Some(out)
    }
}
