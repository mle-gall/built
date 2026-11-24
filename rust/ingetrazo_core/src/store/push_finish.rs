// SPDX-License-Identifier: GPL-3.0-or-later
//! Carry the base's curve runs and locate paint recipients after the
//! watertightness guard has accepted the push.
use super::{
    geometry::{normalized, Hypot},
    keys::Kind,
    Store, SOFT,
};
use crate::push_command::outline_curve_runs;
use std::collections::BTreeSet;

impl Store {
    pub(super) fn finish_push(
        &mut self,
        before: &Store,
        base: Option<u32>,
        shift: Option<[f64; 3]>,
        facet_cos: f64,
        next: &mut i64,
        painted: bool,
        h: Hypot,
    ) -> Vec<u128> {
        for edge in self.live_slots(Kind::Edge) {
            let flags = *self.state.eflags.get(edge as usize);
            if flags & SOFT != 0
                || before
                    .slot(Kind::Edge, self.key(Kind::Edge, edge))
                    .is_some()
            {
                continue;
            }
            if let &[a, b] = self.edge_faces(edge) {
                let dot = normalized(self.face_normal(a, h), h)
                    .dot(normalized(self.face_normal(b, h), h), h);
                if facet_cos < dot && dot < 0.99995 {
                    self.set_edge_flags(edge, flags | SOFT);
                }
            }
        }
        if let (Some(base), Some(shift)) = (base, shift) {
            self.tag_moved_curves(before, base, shift, next);
        }
        let mut fresh = BTreeSet::new();
        if painted {
            for v in self.live_slots(Kind::Vertex) {
                if before
                    .slot(Kind::Vertex, self.key(Kind::Vertex, v))
                    .is_none()
                {
                    for e in self.edges_at(v) {
                        fresh.extend(self.edge_faces(e).iter().map(|&f| self.key(Kind::Face, f)));
                    }
                }
            }
        }
        fresh.into_iter().collect()
    }

    fn tag_moved_curves(&mut self, before: &Store, face: u32, delta: [f64; 3], next: &mut i64) {
        for ring in before.face_rings(face) {
            let ids = ring
                .iter()
                .zip(ring.iter().cycle().skip(1))
                .map(|(&a, &b)| {
                    before
                        .find_edge(a, b)
                        .map(|e| *before.state.ecurve.get(e as usize))
                        .filter(|&c| c >= 0)
                })
                .collect();
            for (run, closed) in outline_curve_runs(ids, true) {
                let moved = |i: usize| {
                    let p = before.state.vpos.get(ring[i] as usize);
                    self.lookup(std::array::from_fn(|k| p[k] + delta[k]))
                };
                let edges: Option<Vec<_>> = run
                    .iter()
                    .zip(run.iter().cycle().skip(1))
                    .take(run.len() - usize::from(!closed))
                    .map(|(&a, &b)| {
                        let e = self.find_edge(moved(a)?, moved(b)?)?;
                        (*self.state.ecurve.get(e as usize) < 0).then_some(e)
                    })
                    .collect();
                if let Some(edges) = edges {
                    for e in edges {
                        self.set_edge_curve(e, *next);
                    }
                    *next += 1;
                }
            }
        }
    }
}
