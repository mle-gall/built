// SPDX-License-Identifier: GPL-3.0-or-later
//! Reuse the triangulation and interior probe of unchanged faces across
//! neighbouring pushes. Triangle corners are stored as vertex slots so a
//! deletion or relink can change the scan's dense indices independently.
use super::{
    geometry::{self, Hypot, V3},
    keys::Kind,
    State, Store,
};
use crate::{
    orient::{interior_probe, Geom},
    scans::MeshScan,
};
use rayon::prelude::*;

pub(super) struct OrientationCache {
    state: State,
    h: Hypot,
    compensated: bool,
    faces: Vec<CachedFace>,
    corners: Vec<[u32; 3]>,
}

#[derive(Clone, Copy)]
struct CachedFace {
    normal: V3,
    center: V3,
    probe: V3,
    start: usize,
    end: usize,
}

impl OrientationCache {
    pub(super) fn new(
        store: &Store,
        scan: &MeshScan,
        faces: &[u32],
        h: Hypot,
        compensated: bool,
    ) -> Option<Self> {
        let geom = Geom::of(scan, h, compensated);
        let vertices = store.live_slots(Kind::Vertex);
        if vertices.len() != geom.pos.len() {
            return None;
        }
        let corners = geom
            .tris
            .iter()
            .map(|t| t.map(|i| vertices[i as usize]))
            .collect();
        let empty = V3(0., 0., 0.);
        let mut by_slot = vec![
            CachedFace {
                normal: empty,
                center: empty,
                probe: empty,
                start: 0,
                end: 0
            };
            store.state.slots[2].len()
        ];
        let probes: Vec<_> = (0..faces.len())
            .into_par_iter()
            .map(|f| interior_probe(&geom, f, &scan.rings_of(f)))
            .collect();
        for (i, &slot) in faces.iter().enumerate() {
            by_slot[slot as usize] = CachedFace {
                normal: geom.normal[i],
                center: geom.centroid[i],
                probe: probes[i],
                start: geom.tstart[i] as usize,
                end: geom.tstart[i + 1] as usize,
            };
        }
        Some(Self {
            state: store.state.clone(),
            h,
            compensated,
            faces: by_slot,
            corners,
        })
    }

    pub(super) fn geometry(
        &self,
        store: &Store,
        scan: &MeshScan,
        faces: &[u32],
        h: Hypot,
        compensated: bool,
    ) -> Option<(Geom, Vec<V3>)> {
        if self.h != h || self.compensated != compensated {
            return None;
        }
        let mut dirty = vec![false; store.state.slots[2].len()];
        for f in store.state.changed_face_geometry(&self.state)? {
            dirty[f as usize] = true;
        }
        let dense = store.state.order[0].dense_of_slot(&store.state.slots[0]);
        if scan.positions().len() != store.n(Kind::Vertex) {
            return None;
        }
        let mut geom = Geom {
            h,
            pos: scan
                .positions()
                .iter()
                .map(|p| V3(p[0], p[1], p[2]))
                .collect(),
            normal: Vec::with_capacity(faces.len()),
            centroid: Vec::with_capacity(faces.len()),
            tstart: vec![0],
            tris: Vec::with_capacity(self.corners.len()),
        };
        let mut probes = Vec::with_capacity(faces.len());
        for (i, &slot) in faces.iter().enumerate() {
            if dirty[slot as usize] {
                let rings = scan.rings_of(i);
                let outer = rings.first().copied().unwrap_or_default();
                let n = geometry::face_normal(&geom.pos, outer, h);
                geom.normal.push(n);
                geom.centroid
                    .push(geometry::face_centroid(&geom.pos, outer, compensated));
                if outer.len() >= 3 {
                    geom.tris
                        .extend(geometry::triangles(&geom.pos, &rings, n, h));
                }
                geom.tstart.push(geom.tris.len() as u32);
                probes.push(interior_probe(&geom, i, &rings));
            } else {
                let face = self.faces[slot as usize];
                geom.normal.push(face.normal);
                geom.centroid.push(face.center);
                probes.push(face.probe);
                geom.tris.extend(
                    self.corners[face.start..face.end]
                        .iter()
                        .map(|t| t.map(|v| dense[v as usize])),
                );
                geom.tstart.push(geom.tris.len() as u32);
            }
        }
        Some((geom, probes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scans::{EFACES, FFLAGS, RINGS};
    use crate::store::edit::Change;

    #[test]
    fn retained_face_probes_match_a_new_scan_after_slot_reuse() {
        let mut store = Store::empty(15, Default::default());
        let mut change = Change::default();
        let face = |x: f64, z: f64| vec![[x, 0., z], [x + 1., 0., z], [x, 2., z]];
        for i in 0..16 {
            store.add_face(&[face(i as f64, 0.)], &mut change).unwrap();
        }
        let h = Hypot::LibStdCxx;
        let scan = |s: &Store| MeshScan::from_state(s.state.clone(), RINGS | EFACES | FFLAGS);
        let cache = OrientationCache::new(
            &store,
            &scan(&store),
            &store.live_slots(Kind::Face),
            h,
            true,
        )
        .unwrap();
        store.remove_face(3, &mut change);
        store.add_face(&[face(3., 2.)], &mut change).unwrap();
        let scan = scan(&store);
        let faces = store.live_slots(Kind::Face);
        let (kept, probes) = cache.geometry(&store, &scan, &faces, h, true).unwrap();
        let fresh = Geom::of(&scan, h, true);
        let bits = |v: V3| [v.0, v.1, v.2].map(f64::to_bits);
        assert_eq!(kept.tris, fresh.tris);
        assert_eq!(kept.tstart, fresh.tstart);
        for i in 0..faces.len() {
            assert_eq!(bits(kept.centroid[i]), bits(fresh.centroid[i]));
            assert_eq!(bits(kept.normal[i]), bits(fresh.normal[i]));
            assert_eq!(
                bits(probes[i]),
                bits(interior_probe(&fresh, i, &scan.rings_of(i)))
            );
        }
    }
}
