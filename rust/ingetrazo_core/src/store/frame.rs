// SPDX-License-Identifier: GPL-3.0-or-later
use super::{cow::CowVec, State};

impl State {
    /// Only the position column changes representation. Registry cells always
    /// use model coordinates, including displaced and alternate cell owners.
    pub fn in_frame(&self, origin: [f64; 3], original: Option<&State>) -> State {
        let mut out = self.clone();
        if origin == self.frame_origin {
            return out;
        }
        let delta: [f64; 3] = std::array::from_fn(|i| self.frame_origin[i] - origin[i]);
        let original = original.filter(|s| s.frame_origin == origin && s.ns == self.ns);
        out.vpos = CowVec::from_vec(self.vpos.iter().enumerate().map(|(slot, p)| {
            let mut result = std::array::from_fn(|i| p[i] + delta[i]);
            // Retain untouched components exactly when the mesh also contains
            // islands far away from the active coordinate frame.
            if let Some(s) = original.filter(|s| slot < s.vpos.len()
                    && s.slots[0].gen(slot as u32) == self.slots[0].gen(slot as u32)) {
                let old = s.vpos.get(slot);
                for i in 0..3 {
                    if old[i] - delta[i] == p[i] {
                        result[i] = old[i];
                    }
                }
            }
            result
        }).collect());
        out.frame_origin = origin;
        out
    }
}
