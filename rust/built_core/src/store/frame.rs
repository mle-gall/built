// SPDX-License-Identifier: GPL-3.0-or-later
use super::{cow::CowVec, State};

use glam::{DMat3, DMat4, DVec3};
use std::sync::Arc;

#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
struct Placement {
    basis: DMat3,
    inverse: DMat3,
    translation: DVec3,
}

/// Stored points are in occurrence world coordinates relative to origin.
/// Weld cells always belong to the underlying definition.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub origin: [f64; 3],
    placement: Option<Arc<Placement>>,
}

impl Frame {
    pub fn instance(matrix: [f64; 16], origin: [f64; 3]) -> Option<Self> {
        let m = DMat4::from_cols_array(&matrix);
        if !m.is_finite()
            || !DVec3::from(origin).is_finite()
            || m.row(3).to_array() != [0., 0., 0., 1.]
        {
            return None;
        }
        let basis = DMat3::from_mat4(m);
        if basis.determinant() == 0. {
            return None;
        }
        let inverse = basis.inverse();
        if !inverse.is_finite() {
            return None;
        }
        let placement = Placement {
            basis,
            inverse,
            translation: m.w_axis.truncate(),
        };
        Some(Self {
            origin,
            placement: Some(Arc::new(placement)),
        })
    }

    pub fn rebased(&self, origin: [f64; 3]) -> Self {
        Self {
            origin,
            placement: self.placement.clone(),
        }
    }

    fn offset(&self) -> DVec3 {
        self.placement
            .as_ref()
            .map_or(DVec3::ZERO, |p| p.translation)
            - DVec3::from(self.origin)
    }

    pub fn matrix(&self) -> Option<[f64; 16]> {
        self.placement.as_ref().map(|p| {
            DMat4::from_cols(
                p.basis.x_axis.extend(0.),
                p.basis.y_axis.extend(0.),
                p.basis.z_axis.extend(0.),
                p.translation.extend(1.),
            )
            .to_cols_array()
        })
    }

    pub(crate) fn mirrored(&self) -> bool {
        self.placement
            .as_ref()
            .is_some_and(|p| p.basis.determinant() < 0.)
    }

    pub fn is_model(&self) -> bool {
        self.placement.is_none() && self.origin == [0.; 3]
    }

    pub fn definition_delta(&self, delta: DVec3) -> DVec3 {
        self.placement.as_ref().map_or(delta, |p| p.inverse * delta)
    }

    pub fn definition_point(&self, point: [f64; 3]) -> [f64; 3] {
        let v = DVec3::from(point) - self.offset();
        self.placement
            .as_ref()
            .map_or(v, |p| p.inverse * v)
            .to_array()
    }

    pub fn convert(&self, point: [f64; 3], to: &Self) -> [f64; 3] {
        if self.placement == to.placement {
            let delta = DVec3::from(self.origin) - DVec3::from(to.origin);
            return (DVec3::from(point) + delta).to_array();
        }
        let v = DVec3::from(self.definition_point(point));
        (to.placement.as_ref().map_or(v, |p| p.basis * v) + to.offset()).to_array()
    }
}

impl State {
    /// Only the position column changes representation. Registry cells always
    /// use model coordinates, including displaced and alternate cell owners.
    pub fn in_frame(&self, frame: Frame, original: Option<&State>) -> State {
        let mut out = self.clone();
        if frame == self.frame {
            return out;
        }
        let original = original.filter(|s| s.frame == frame && s.ns == self.ns);
        out.vpos = CowVec::from_vec(
            self.vpos
                .iter()
                .enumerate()
                .map(|(slot, p)| {
                    let mut result = self.frame.convert(*p, &frame);
                    // Retain untouched components exactly when the mesh also contains
                    // islands far away from the active coordinate frame.
                    if let Some(s) = original.filter(|s| {
                        slot < s.vpos.len()
                            && s.slots[0].gen(slot as u32) == self.slots[0].gen(slot as u32)
                    }) {
                        let old = s.vpos.get(slot);
                        let previous = frame.convert(*old, &self.frame);
                        if previous == *p {
                            return *old;
                        }
                        for i in 0..3 {
                            if frame.placement == self.frame.placement && previous[i] == p[i] {
                                result[i] = old[i];
                            }
                        }
                    }
                    result
                })
                .collect(),
        );
        if frame.mirrored() != self.frame.mirrored() {
            // The ring and corner pools are appended in lockstep, including
            // tombstones. A second reflection restores their exact order.
            let mut corners: Vec<_> = self.corners.iter().copied().collect();
            let mut start = 0;
            for size in self.ring_sizes.iter() {
                let end = start + *size as usize;
                corners[start..end].reverse();
                start = end;
            }
            out.corners = CowVec::from_vec(corners);
        }
        out.frame = frame;
        out
    }
}
