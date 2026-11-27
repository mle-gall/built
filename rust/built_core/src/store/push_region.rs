// SPDX-License-Identifier: GPL-3.0-or-later
//! Refine the swept rings and classify their neighbours before opening the cap.
use super::{drawing_face::refine, geometry::Hypot, keys::Kind, topo::OpError, Store};
use crate::pushscan::{grow_box, holds_point};
use std::collections::HashSet;

type Ring = Vec<[f64; 3]>;

impl Store {
    pub(super) fn push_component_closed(&self, seed: u32) -> bool {
        let mut seen = HashSet::from([seed]);
        let mut todo = vec![seed];
        while let Some(f) = todo.pop() {
            for r in self.face_rings(f) {
                for (&a, &b) in r.iter().zip(r.iter().cycle().skip(1)) {
                    if let Some(e) = self.find_edge(a, b) {
                        let neighbours = self.edge_faces(e);
                        if neighbours.len() < 2 {
                            return false;
                        }
                        todo.extend(neighbours.iter().copied().filter(|n| seen.insert(*n)));
                    }
                }
            }
        }
        true
    }

    pub(super) fn push_region(
        &self,
        face: u32,
        closed: bool,
        h: Hypot,
        index: Option<&super::face_index::FaceIndex>,
    ) -> Result<(Vec<Ring>, bool, bool, bool), OpError> {
        const TOL: f64 = 1e-4;
        let rings = self.position_rings(face);
        let vertices = self.live_slots(Kind::Vertex);
        let mut refined = Vec::with_capacity(rings.len());
        for r in rings {
            let (lo, hi) = super::drawing_plane::bounds(&r);
            let big = lo.iter().chain(&hi).fold(0.0f64, |m, x| m.max(x.abs()));
            let g = 2. * TOL + 1e-6 * big + 1e-9 * (1. + big);
            let points: Vec<_> = vertices.iter().map(|&v| *self.state.vpos.get(v as usize))
                .filter(|p| (0..3).all(|i| lo[i] - g <= p[i] && p[i] <= hi[i] + g))
                .collect();
            refined.push(refine(&r, &points, TOL, h));
        }
        // The classifier grows each face box by its diagonal and position.
        // A bound from all listed points encloses every such growth. Include
        // multi-ring faces explicitly: the BVH indexes outer loops only.
        let all = self.live_slots(Kind::Face);
        let faces = if let Some(index) = index {
            let big = self.state.vpos.iter().flatten().fold(0f64, |m, x| m.max(x.abs()));
            let margin = 4. * TOL + 8. * TOL * big + 1e-5 * (1. + big);
            let mut bounds = [f64::INFINITY, f64::INFINITY, f64::INFINITY,
                              f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
            for p in refined.iter().flatten() {
                for k in 0..3 {
                    bounds[k] = bounds[k].min(p[k] - margin);
                    bounds[k + 3] = bounds[k + 3].max(p[k] + margin);
                }
            }
            let mut hits = index.faces(self, bounds, super::face_index::Query::Meeting);
            hits.extend(all.iter().copied().filter(|&f| self.state.frings.get(f as usize).n_rings > 1));
            hits.sort_unstable_by_key(|&f| self.state.list_rank(Kind::Face, f));
            hits.dedup();
            hits
        } else { all };
        let boxes: Vec<_> = faces.iter().map(|&f| {
            let mut bounds = [f64::INFINITY, f64::INFINITY, f64::INFINITY,
                              f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
            for v in self.face_rings(f).into_iter().flatten() {
                let p = self.state.vpos.get(v as usize);
                for k in 0..3 {
                    bounds[k] = bounds[k].min(p[k]);
                    bounds[k + 3] = bounds[k + 3].max(p[k]);
                }
            }
            grow_box(bounds, 4. * TOL, 2. * TOL)
        }).collect();
        let mut backed = true;
        let mut sheet = !closed;
        for (i, r) in refined.iter().enumerate() {
            if i > 0 && !sheet {
                break;
            }
            for (&a, &b) in r.iter().zip(r.iter().cycle().skip(1)) {
                let candidates: Vec<_> = faces
                    .iter()
                    .zip(&boxes)
                    .filter_map(|(&f, bounds)| {
                        (holds_point(bounds, a) && holds_point(bounds, b)).then_some(f)
                    })
                    .collect();
                let kind = self.classify_segment(face, a, b, &candidates, h).0;
                if i == 0 {
                    backed &= kind != "free";
                }
                sheet &= kind == "coplanar";
                if !sheet && (i > 0 || !backed) {
                    break;
                }
            }
        }
        sheet = sheet && !self.push_component_closed(face);
        let flat = self
            .faces_are_flat(1e-3, h)
            .ok_or(OpError::Fallback("empty face in push region"))?;
        Ok((refined, backed, sheet, !flat && !sheet))
    }
}
