// SPDX-License-Identifier: GPL-3.0-or-later
//! Install the cap planner's replacement rings in order; old perimeter keys survive.
use super::{
    edit::Change,
    keys::{Kind, NONE},
    topo::OpError,
    Store, INTERIOR,
};
use crate::facegeom::{Hypot, V3};

pub type Cap = (Vec<Vec<[f64; 3]>>, bool);
impl Store {
    pub fn replace_caps(
        &mut self,
        faces: &[u32],
        caps: &[Cap],
        ch: &mut Change,
    ) -> Result<Vec<u128>, OpError> {
        for &f in faces {
            self.remove_face(f, ch);
        }
        let mut keys = Vec::with_capacity(caps.len());
        for (rings, interior) in caps {
            let f = self.add_face(rings, ch)?;
            self.state
                .fflags
                .set(f as usize, if *interior { INTERIOR } else { 0 });
            keys.push(self.key(Kind::Face, f));
        }
        Ok(keys)
    }

    pub fn edge_on_cap_plane(&self, edge: u32, o: V3, n: V3, tol: f64, h: Hypot) -> bool {
        self.state.ends.get(edge as usize).iter().all(|&v| {
            let [x, y, z] = *self.state.vpos.get(v as usize);
            (V3(x as f32, y as f32, z as f32).sub(o).dot(n, h).abs() as f64) < tol
        })
    }

    pub fn prune_plane_debris(
        &mut self,
        origin: [f32; 3],
        normal: [f32; 3],
        tol: f64,
        h: Hypot,
        ch: &mut Change,
    ) -> Result<(), OpError> {
        let o = V3(origin[0], origin[1], origin[2]);
        let n = V3(normal[0], normal[1], normal[2]);
        for e in self.live_slots(Kind::Edge) {
            if self.edge_faces(e).is_empty() && self.edge_on_cap_plane(e, o, n, tol, h) {
                self.remove_edge(e, ch);
            }
        }
        let mut orphans: Vec<_> = self
            .live_slots(Kind::Vertex)
            .into_iter()
            .filter(|&v| *self.state.vedge.get(v as usize) == NONE)
            .collect();
        if !orphans.is_empty() {
            let used = self.face_corners();
            orphans.retain(|&v| !used[v as usize]);
            self.drop_orphan_vertices(&orphans, ch)?;
        }
        Ok(())
    }
}
