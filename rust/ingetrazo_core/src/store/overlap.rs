// SPDX-License-Identifier: GPL-3.0-or-later
//! Face replacements made by the drawing overlap heal after edge repair.
use super::{
    drawing::vector, edit::Change, fillet::maximal_holes, geometry::Hypot, keys::Kind,
    topo::OpError, Store,
};

type Ring = Vec<[f64; 3]>;

#[derive(Default)]
pub(super) struct Healed {
    pub replacements: Vec<(u128, u128)>,
    pub bumps: usize,
}

fn filter_holes(rings: &mut Vec<Ring>, h: Hypot) -> Result<(), OpError> {
    if rings.len() > 2 {
        if rings[1..].iter().any(|r| r.len() < 3) {
            return Err(OpError::Fallback("overlap heal found a short hole"));
        }
        let holes: Vec<_> = rings[1..]
            .iter()
            .map(|r| r.iter().copied().map(vector).collect())
            .collect();
        let mask = maximal_holes(&holes, h);
        let mut keep = std::iter::once(true).chain(mask);
        rings.retain(|_| keep.next().unwrap());
    }
    Ok(())
}

impl Store {
    fn replace_overlap_face(
        &mut self,
        face: u32,
        rings: &[Ring],
        result: &mut Healed,
        ch: &mut Change,
    ) -> Result<(), OpError> {
        if self.remove_face_blocked(face) {
            return Err(OpError::Fallback(
                "overlap heal would leave a dangling face",
            ));
        }
        let old = self.key(Kind::Face, face);
        self.remove_face(face, ch);
        let new = self.add_face(rings, ch)?;
        result.replacements.push((old, self.key(Kind::Face, new)));
        result.bumps += 2 + rings.iter().map(Vec::len).sum::<usize>();
        Ok(())
    }

    /// Heal the drawing left by a full cap collapse, with the same order as
    /// topology.heal_overlapping_faces on a flat mesh without a footprint.
    pub(super) fn heal_flat_push(
        &mut self,
        next_curve: &mut i64,
        h: Hypot,
        compensated: bool,
        ch: &mut Change,
    ) -> Result<Healed, OpError> {
        let mut result = Healed::default();
        if self.live_slots(Kind::Face).len() > 3000 {
            return Ok(result);
        }
        let (_, bumps, curved) = self.prune_overlapping_edges(h, ch)?;
        result.bumps += bumps;
        if curved {
            self.resplit_curves(next_curve, h);
            result.bumps += 1;
        }
        result.bumps += 3 * self.split_junctions(None, 1e-4, ch)?;
        for key in self.winding_flips(None, h, compensated) {
            let face = self.slot(Kind::Face, key).unwrap();
            let mut rings = self.position_rings(face);
            rings[0].reverse();
            filter_holes(&mut rings, h)?;
            self.replace_overlap_face(face, &rings, &mut result, ch)?;
        }
        self.heal_face_overlaps(0.5, true, 1e-2, h, compensated, ch, &mut result)?;
        Ok(result)
    }

    fn heal_face_overlaps(
        &mut self,
        coverage: f64,
        partial: bool,
        pad: f64,
        h: Hypot,
        compensated: bool,
        ch: &mut Change,
        result: &mut Healed,
    ) -> Result<(), OpError> {
        for f in self.live_slots(Kind::Face) {
            let n = self.state.frings.get(f as usize).n_rings as usize;
            if n > 2 {
                let mut rings = self.position_rings(f);
                filter_holes(&mut rings, h)?;
                if rings.len() < n {
                    self.replace_overlap_face(f, &rings, result, ch)?;
                }
            }
        }
        let (removed, punches) = self.heal_plan(coverage, partial, pad, h, compensated);
        for &key in &removed {
            let f = self.slot(Kind::Face, key).unwrap();
            self.remove_face(f, ch);
        }
        result.bumps += removed.len();
        for (key, smalls) in punches {
            let f = self.slot(Kind::Face, key).unwrap();
            let mut rings = self.position_rings(f);
            for small in smalls {
                // An earlier punch may already have replaced this face.
                let slot = self
                    .slot_any(Kind::Face, small)
                    .ok_or(OpError::Fallback("overlap heal lost a punch boundary"))?;
                let ring = self
                    .outer_ring(slot)
                    .into_iter()
                    .map(|v| *self.state.vpos.get(v as usize))
                    .collect();
                rings.push(ring);
            }
            filter_holes(&mut rings, h)?;
            self.replace_overlap_face(f, &rings, result, ch)?;
        }
        Ok(())
    }
}
