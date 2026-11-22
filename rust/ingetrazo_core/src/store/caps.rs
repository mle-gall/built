// SPDX-License-Identifier: GPL-3.0-or-later
//! Install the cap planner's replacement rings in order; old perimeter keys survive.
use super::{
    edit::Change,
    keys::{Kind, NONE},
    topo::OpError,
    Store, INTERIOR,
};
use crate::store::geometry::{Hypot, V3};

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
            V3(x, y, z).sub(o).dot(n, h).abs() < tol
        })
    }

    pub fn prune_plane_debris(
        &mut self,
        origin: [f64; 3],
        normal: [f64; 3],
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

/// Coverage bits distinguish old skin (+/-), old structure and fresh skin (+/-).
/// A fresh interior face counts as old structure, just as a kept partition does.
impl Store {
    pub fn cap_coverage(
        &self,
        scope: &[u32],
        fresh: &[u32],
        origin: [f64; 3],
        normal: [f64; 3],
        probes: &[(f64, f64)],
        tol: f64,
        h: Hypot,
        compensated: bool,
    ) -> Vec<(bool, u8, u8)> {
        use super::offset::contains;
        use crate::store::geometry::normalized;
        use crate::store::geometry::plane_basis;
        use std::collections::HashSet;
        let fresh: HashSet<_> = fresh.iter().copied().collect();
        let mut seen = HashSet::new();
        let mut coverage = vec![(false, 0, 0); probes.len()];
        let n = V3(normal[0], normal[1], normal[2]);
        let o = V3(origin[0], origin[1], origin[2]);
        let (u, v) = plane_basis(n, h);
        for &f in scope.iter().chain(
            fresh
                .iter()
                .filter(|&&f| self.state.fflags.get(f as usize) & INTERIOR == 0),
        ) {
            if !seen.insert(f) {
                continue;
            }
            let rings = self.face_rings(f);
            let Some(outer) = rings.first() else { continue };
            let (center, face_normal, _) = self.cap_frame(outer, h, compensated);
            let alignment = normalized(face_normal, h).dot(n, h);
            if alignment.abs() <= 0.999 || center.sub(o).dot(n, h).abs() >= tol {
                continue;
            }
            let xy: Vec<Vec<_>> = rings
                .iter()
                .map(|r| {
                    r.iter()
                        .map(|&s| {
                            let d = self.cap_point(s).sub(o);
                            (d.dot(u, h), d.dot(v, h))
                        })
                        .collect()
                })
                .collect();
            let interior = self.state.fflags.get(f as usize) & INTERIOR != 0;
            let sign = if alignment > 0.0 { 1 } else { 2 };
            for (&probe, (old, skin, push)) in probes.iter().zip(coverage.iter_mut()) {
                if contains(&xy[0], probe) && !xy[1..].iter().any(|hole| contains(hole, probe)) {
                    if fresh.contains(&f) && !interior {
                        *push |= sign;
                    } else {
                        *old = true;
                        if !interior {
                            *skin |= sign;
                        }
                    }
                }
            }
        }
        coverage
    }
}

use crate::store::geometry::{length, normalized};
use crate::store::geometry::{face_centroid, face_normal_length};
pub type Seam = ([f64; 3], [f64; 3], Option<[f64; 3]>);

fn vector([x, y, z]: [f64; 3]) -> V3 {
    V3(x, y, z)
}
fn xyz(V3(x, y, z): V3) -> [f64; 3] {
    [x, y, z]
}

impl Store {
    fn cap_point(&self, slot: u32) -> V3 {
        vector(self.state.vpos.get(slot as usize).map(|p| p))
    }

    fn cap_frame(&self, ring: &[u32], h: Hypot, compensated: bool) -> (V3, V3, f64) {
        let pos: Vec<_> = ring.iter().map(|&s| self.cap_point(s)).collect();
        let order: Vec<_> = (0..pos.len() as u32).collect();
        let (normal, len) = face_normal_length(&pos, &order, h);
        (face_centroid(&pos, &order, compensated), normal, len)
    }

    pub fn crack_planes(
        &self,
        near: Option<[f64; 6]>,
        h: Hypot,
        compensated: bool,
    ) -> Vec<([f64; 3], [f64; 3])> {
        let mut seen = std::collections::HashSet::new();
        let mut planes = vec![];
        for e in self.live_slots(Kind::Edge) {
            let [f] = self.edge_faces(e) else { continue };
            if seen.contains(f) {
                continue;
            }
            if let Some(bounds) = near {
                if !self.state.ends.get(e as usize).iter().any(|&v| {
                    let p = xyz(self.cap_point(v)).map(f64::from);
                    (0..3).all(|i| bounds[i] <= p[i] && p[i] <= bounds[i + 3])
                }) {
                    continue;
                }
            }
            seen.insert(*f);
            let rings = self.face_rings(*f);
            let Some(outer) = rings.first() else { continue };
            let (center, n, _) = self.cap_frame(outer, h, compensated);
            planes.push((xyz(center), xyz(n)));
        }
        planes
    }

    pub fn edges_along(&self, segments: &[[[f64; 3]; 2]], tolerance: f64, h: Hypot) -> Vec<u128> {
        let on = |p: V3, a: V3, b: V3| {
            let ab = b.sub(a);
            let l2 = ab.dot(ab, h);
            if l2 < 1e-18 {
                return length(p.sub(a), h) < tolerance;
            }
            let t = p.sub(a).dot(ab, h) / l2;
            (-1e-9..=1. + 1e-9).contains(&t)
                && length(a.add(ab.mul(t)).sub(p), h) < tolerance
        };
        self.live_slots(Kind::Edge)
            .into_iter()
            .filter(|&e| {
                let ends = self.state.ends.get(e as usize).map(|v| self.cap_point(v));
                segments
                    .iter()
                    .any(|&[a, b]| ends.iter().all(|&p| on(p, vector(a), vector(b))))
            })
            .map(|e| self.key(Kind::Edge, e))
            .collect()
    }

    pub fn seam_planes(
        &self,
        faces: &[u32],
        half: f64,
        hair_cos2: f64,
        h: Hypot,
        compensated: bool,
    ) -> Vec<Seam> {
        let mut out = vec![];
        for &f in faces {
            let rings = self.face_rings(f);
            let Some(outer) = rings.first() else { continue };
            let (center, n, size) = self.cap_frame(outer, h, compensated);
            let nf = normalized(n, h);
            let off = |rings: &[Vec<u32>], normal: V3| {
                rings
                    .iter()
                    .flatten()
                    .any(|&v| self.cap_point(v).sub(center).dot(normal, h).abs() > half)
            };
            let mut coplanar = false;
            let mut correction = None;
            let mut visited = std::collections::HashSet::from([f]);
            'boundary: for ring in &rings {
                for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                    let Some(e) = self.find_edge(a, b) else {
                        continue;
                    };
                    for &g in self.edge_faces(e) {
                        if !visited.insert(g) {
                            continue;
                        }
                        let gr = self.face_rings(g);
                        let Some(outer) = gr.first() else { continue };
                        let (_, gn, gsize) = self.cap_frame(outer, h, compensated);
                        let unit = normalized(gn, h);
                        let align = nf.dot(unit, h);
                        if align.abs() <= 0.999 {
                            continue;
                        }
                        coplanar = true;
                        if gsize > size
                            && align * align >= hair_cos2
                            && off(&gr, nf)
                            && !off(&gr, unit)
                            && !off(&rings, unit)
                        {
                            correction = Some(xyz(gn));
                            break 'boundary;
                        }
                    }
                }
            }
            if coplanar {
                out.push((xyz(center), xyz(nf), correction));
            }
        }
        out
    }
}

impl Store {
    /// Each face's component label and the scoped edge keys. Face labels are
    /// dense list indices, matching the Python union through edges in list order.
    pub fn cap_scope(&self, seeds: &[u128]) -> (Vec<(u128, u32)>, Vec<u128>) {
        use crate::orient::component_root;
        let faces = self.live_slots(Kind::Face);
        let edges = self.live_slots(Kind::Edge);
        let dense = self.state.order[2].dense_of_slot(&self.state.slots[2]);
        let mut parent: Vec<_> = (0..faces.len() as u32).collect();
        for &e in &edges {
            if let Some((&first, rest)) = self.edge_faces(e).split_first() {
                for &f in rest {
                    let a = component_root(&mut parent, dense[first as usize]);
                    let b = component_root(&mut parent, dense[f as usize]);
                    parent[a as usize] = b;
                }
            }
        }
        let labels: Vec<_> = (0..faces.len() as u32)
            .map(|f| component_root(&mut parent, f))
            .collect();
        let wanted: std::collections::HashSet<_> = seeds
            .iter()
            .filter_map(|&k| self.slot(Kind::Face, k))
            .map(|f| labels[dense[f as usize] as usize])
            .collect();
        let scoped = if wanted.is_empty() {
            vec![]
        } else {
            edges
                .into_iter()
                .filter(|&e| {
                    let fs = self.edge_faces(e);
                    fs.is_empty()
                        || fs
                            .iter()
                            .any(|&f| wanted.contains(&labels[dense[f as usize] as usize]))
                })
                .map(|e| self.key(Kind::Edge, e))
                .collect()
        };
        (
            faces
                .iter()
                .zip(labels)
                .map(|(&f, label)| (self.key(Kind::Face, f), label))
                .collect(),
            scoped,
        )
    }
}
