// SPDX-License-Identifier: GPL-3.0-or-later
//! Translate a prism cap and repair the swept area in one store transaction.
use super::{
    commands::{cell, Cell},
    edit::Change,
    geometry::Hypot,
    keys::Kind,
    push_rebuild::{Fresh, Paint},
    topo::OpError,
    Store,
};
use crate::{
    orient::{orient, Geom, Params},
    scans::{MeshScan, EFACES, FFLAGS, RINGS},
};
use std::collections::HashSet;

#[derive(Default)]
pub(super) struct Translated {
    pub bumps: usize,
    pub before: Vec<(u128, u128)>,
    pub painted: Vec<(u128, u32)>,
    pub after: Vec<(u128, u128)>,
    pub replaced: Vec<(u128, u128)>,
    pub flipped: Vec<u128>,
}

/// Floor-grid prefilter of StoreMesh.vertices_in_cells. Rounded cells near
/// the int64 limits can have only some of their eight neighbours in range.
fn cap_neighbourhood(source: &HashSet<Cell>) -> HashSet<[i64; 3]> {
    let mut near = HashSet::new();
    for key in source {
        let center = key.map(|bits| (f64::from_bits(bits) * 1e4).round_ties_even() as i128);
        for dx in [0, -1] {
            for dy in [0, -1] {
                for dz in [0, -1] {
                    let neighbour = [0, 1, 2]
                        .map(|i| i64::try_from(center[i].saturating_add([dx, dy, dz][i])).ok());
                    if let [Some(x), Some(y), Some(z)] = neighbour {
                        near.insert([x, y, z]);
                    }
                }
            }
        }
    }
    near
}

impl Store {
    fn touched_face(&self, face: u32, seeds: &HashSet<Cell>) -> bool {
        self.outer_ring(face)
            .into_iter()
            .any(|v| seeds.contains(&cell(*self.state.vpos.get(v as usize))))
    }

    fn orient_push(&mut self, seeds: &HashSet<Cell>, h: Hypot, compensated: bool) -> Vec<u128> {
        let faces = self.live_slots(Kind::Face);
        let touched: Vec<_> = faces
            .iter()
            .enumerate()
            .filter_map(|(i, &f)| self.touched_face(f, seeds).then_some(i as u32))
            .collect();
        let scope = (!touched.is_empty()).then_some(touched.as_slice());
        let scan = MeshScan::from_state(self.state.clone(), RINGS | EFACES | FFLAGS);
        let geom = Geom::of(&scan, h, compensated);
        let params = Params {
            seed: 12345,
            rays: 7,
            jitter: 3e-4,
        };
        let decision = orient(&scan, &geom, scope, &params);
        let marks: Vec<_> = decision
            .marks
            .into_iter()
            .map(|(i, v)| (faces[i as usize], v))
            .collect();
        let flips: Vec<_> = decision
            .flips
            .into_iter()
            .map(|i| faces[i as usize])
            .collect();
        self.apply_orientation(&marks, &flips);
        flips.into_iter().map(|f| self.key(Kind::Face, f)).collect()
    }

    pub(super) fn push_translate(
        &mut self,
        cap: &[[f64; 3]],
        delta: [f64; 3],
        removing: bool,
        preview: bool,
        next_curve: &mut i64,
        paint: &mut Paint,
        h: Hypot,
        compensated: bool,
        ch: &mut Change,
    ) -> Result<Translated, OpError> {
        let mut result = Translated::default();
        let source: HashSet<_> = cap.iter().copied().map(cell).collect();
        let mut seeds = source.clone();
        seeds.extend(
            cap.iter()
                .map(|p| cell(std::array::from_fn(|i| p[i] + delta[i]))),
        );
        let seed_positions: Vec<_> = seeds.iter().map(|key| key.map(f64::from_bits)).collect();

        let near = cap_neighbourhood(&source);
        for v in self.live_slots(Kind::Vertex) {
            let p = *self.state.vpos.get(v as usize);
            if near.contains(&super::commands::floor_cell(p)) && source.contains(&cell(p)) {
                self.move_vertex(v, delta);
                ch.modified[0].push(self.key(Kind::Vertex, v));
                result.bumps += 1;
            }
        }
        let connected = self.stitch_connect(&seed_positions, true, 1e-4, h, ch)?;
        result.bumps += connected.bumps;
        for (drop, keep) in connected.deduped {
            let class = paint.class(self, drop);
            if class != 0 && paint.class(self, keep) == 0 {
                let k = self.key(Kind::Face, keep);
                paint.own.insert(k, class);
                result.before.push((self.key(Kind::Face, drop), k));
            }
        }
        match self.faces_are_flat(1e-3, h) {
            Some(true) => {
                let healed = self.heal_flat_push(next_curve, h, compensated, ch)?;
                result.bumps += healed.bumps;
                result.replaced = healed.replacements;
            }
            Some(false) => {
                let listed = self
                    .live_slots(Kind::Face)
                    .into_iter()
                    .filter(|&f| self.touched_face(f, &seeds))
                    .map(|f| self.key(Kind::Face, f))
                    .collect();
                let rebuilt = self.rebuild_push_planes(
                    Fresh::Listed(listed),
                    &seed_positions,
                    false,
                    removing,
                    paint,
                    compensated,
                    h,
                    ch,
                )?;
                result.bumps += rebuilt.bumps;
                result.painted = rebuilt.painted;
                result.after = rebuilt.deduped;
            }
            None => return Err(OpError::Fallback("cap translation found an empty face")),
        }
        if !preview {
            result.flipped = self.orient_push(&seeds, h, compensated);
        }
        Ok(result)
    }
}
