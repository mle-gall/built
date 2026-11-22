// SPDX-License-Identifier: GPL-3.0-or-later
//! Offset outline segments and install the planar arrangement's surviving faces.
use super::{edit::Change, keys::Kind, topo::OpError, Store};
use crate::facegeom::{length, normalized, Hypot, V3};

impl Store {
    pub fn offset_outline(
        &self,
        face: u32,
        normal: [f32; 3],
        distance: f64,
        h: Hypot,
    ) -> Vec<([f32; 3], [f32; 3])> {
        let rings = self.face_rings(face);
        let Some(outer) = rings.first() else {
            return Vec::new();
        };
        let point = |v: u32| {
            let p = self.state.vpos.get(v as usize);
            V3(p[0] as f32, p[1] as f32, p[2] as f32)
        };
        let normal = normalized(V3(normal[0], normal[1], normal[2]), h);
        let mut lines = Vec::new();
        for (i, &v) in outer.iter().enumerate() {
            let a = point(v);
            let b = point(outer[(i + 1) % outer.len()]);
            let delta = b.sub(a);
            if length(delta, h) < 1e-9 {
                continue;
            }
            let dir = normalized(delta, h);
            let push = normalized(normal.cross(dir, h), h).mul(distance as f32);
            lines.push((a.add(push), b.add(push), dir));
        }
        if lines.len() < 3 {
            return Vec::new();
        }
        let corners: Vec<_> = lines
            .iter()
            .enumerate()
            .map(|(i, &(_, end, u))| {
                let (start, _, v) = lines[(i + 1) % lines.len()];
                let den = u.cross(v, h).dot(normal, h);
                let corner = if den.abs() < 1e-9 {
                    start
                } else {
                    end.add(u.mul(start.sub(end).cross(v, h).dot(normal, h) / den))
                };
                ((length(corner.sub(end), h) as f64) <= 2. * distance.abs() + 1e-9)
                    .then_some(corner)
            })
            .collect();
        let xyz = |v: V3| [v.0, v.1, v.2];
        let mut segments = Vec::new();
        for (i, &(start, end, _)) in lines.iter().enumerate() {
            let back = corners[(i + lines.len() - 1) % lines.len()].unwrap_or(start);
            segments.push((xyz(back), xyz(corners[i].unwrap_or(end))));
            if corners[i].is_none() {
                segments.push((xyz(end), xyz(lines[(i + 1) % lines.len()].0)));
            }
        }
        segments
    }

    pub fn offset_face(
        &mut self,
        face: u32,
        regions: &[Vec<Vec<[f64; 3]>>],
        ch: &mut Change,
    ) -> Result<Vec<u128>, OpError> {
        if regions.len() < 2
            || regions
                .iter()
                .any(|r| r.is_empty() || r.iter().any(|loop_| loop_.len() < 3))
        {
            return Err(OpError::Fallback("offset needs a strip and valid regions"));
        }
        self.remove_face(face, ch);
        let mut faces = Vec::new();
        for rings in regions {
            let f = self.add_face(rings, ch)?;
            faces.push(self.key(Kind::Face, f));
        }
        Ok(faces)
    }
}
