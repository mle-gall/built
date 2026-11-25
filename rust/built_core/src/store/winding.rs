// SPDX-License-Identifier: GPL-3.0-or-later
use super::{edit::Change, keys::Kind, Store};

impl Store {
    /// Apply orient_decision in mesh order. Keep the same revision steps
    /// as assigning each interior flag and reversing each face separately.
    pub fn apply_orientation(&mut self, marks: &[(u32, bool)], flips: &[u32]) {
        for &(face, interior) in marks {
            let flags = *self.state.fflags.get(face as usize);
            let bit = super::INTERIOR;
            self.set_face_flags(face, if interior { flags | bit } else { flags & !bit });
        }
        for &face in flips {
            let mut change = Change::default();
            self.reverse_face(face, &mut change);
            self.finish(change);
        }
    }

    /// Repeated occurrences flip the same face repeatedly. Only odd
    /// multiplicities can change the winding or its feed record.
    pub fn reverse_faces(&mut self, mut slots: Vec<u32>, change: &mut Change) {
        slots.sort_unstable();
        for run in slots.chunk_by(|a, b| a == b) {
            if run.len() % 2 == 0 {
                continue;
            }
            let face = run[0];
            let rings = *self.state.frings.get(face as usize);
            let mut start = rings.corner_start as usize;
            let mut changed = false;
            for ring in rings.ring_start..rings.ring_start + rings.n_rings {
                let len = *self.state.ring_sizes.get(ring as usize) as usize;
                for offset in 0..len / 2 {
                    let left = start + offset;
                    let right = start + len - offset - 1;
                    let a = *self.state.corners.get(left);
                    let b = *self.state.corners.get(right);
                    if a != b {
                        self.state.corners.set(left, b);
                        self.state.corners.set(right, a);
                        changed = true;
                    }
                }
                start += len;
            }
            if changed {
                change.modified[2].push(self.key(Kind::Face, face));
            }
        }
    }
}
