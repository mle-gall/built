// SPDX-License-Identifier: GPL-3.0-or-later
//! Geometry reads in face order, independent of dense vertex indices.
use super::{cow::FxHashMap, keys::Kind, Store, INTERIOR};
use crate::store::geometry::{triangles, Hypot, V3};

impl Store {
    pub fn face_triangles(&self, f: u32, normal: Option<[f64; 3]>, h: Hypot) -> Vec<[[f64; 3]; 3]> {
        if let Some(triangles) = self.native_triangles(f) {
            return triangles;
        }
        let rings = self.face_rings(f);
        if rings.first().is_none_or(|r| r.len() < 3) {
            return Vec::new();
        }
        let mut pos = Vec::new();
        let local: Vec<Vec<u32>> = rings
            .iter()
            .map(|ring| {
                ring.iter()
                    .map(|&v| {
                        let [x, y, z] = *self.state.vpos.get(v as usize);
                        let i = pos.len() as u32;
                        pos.push(V3(x, y, z));
                        i
                    })
                    .collect()
            })
            .collect();
        let refs: Vec<_> = local.iter().map(Vec::as_slice).collect();
        let n = normal
            .map(|[x, y, z]| V3(x, y, z))
            .unwrap_or_else(|| crate::store::geometry::face_normal(&pos, refs[0], h));
        triangles(&pos, &refs, n, h)
            .iter()
            .map(|t| {
                t.map(|i| {
                    let V3(x, y, z) = pos[i as usize];
                    [x, y, z]
                })
            })
            .collect()
    }

    pub fn solid_volume(&self, h: Hypot) -> Option<f64> {
        let exterior = |f: u32| *self.state.fflags.get(f as usize) & INTERIOR == 0;
        for e in self.live_slots(Kind::Edge) {
            if self.edge_faces(e).iter().filter(|&&f| exterior(f)).count() != 2 {
                return None;
            }
        }
        let faces: Vec<_> = self
            .live_slots(Kind::Face)
            .into_iter()
            .filter(|&f| exterior(f))
            .collect();
        self.face_set_volume(&faces, h).filter(|v| *v > 1e-12)
    }

    pub fn face_set_volume(&self, faces: &[u32], h: Hypot) -> Option<f64> {
        let mut uses = FxHashMap::default();
        for &f in faces {
            if self.state.exact.native.is_some() {
                for (e, _) in self.boundary_edges(f).into_iter().flatten().flatten() {
                    *uses.entry((e, e)).or_insert(0usize) += 1;
                }
                continue;
            }
            for r in self.face_rings(f) {
                for (&a, &b) in r.iter().zip(r.iter().cycle().skip(1)) {
                    *uses.entry((a.min(b), a.max(b))).or_insert(0usize) += 1;
                }
            }
        }
        if uses.values().any(|&n| n != 2) {
            return None;
        }
        let rings = self.face_rings(*faces.first()?);
        let anchor = self.state.vpos.get(*rings.first()?.first()? as usize);
        let rel = |p: [f64; 3]| {
            [
                p[0] - anchor[0],
                p[1] - anchor[1],
                p[2] - anchor[2],
            ]
        };
        let terms = faces
            .iter()
            .flat_map(|&f| self.face_triangles(f, None, h))
            .map(|tri| {
                let [a, b, c] = tri.map(rel);
                (a[0] * (b[1] * c[2] - b[2] * c[1])
                    + a[1] * (b[2] * c[0] - b[0] * c[2])
                    + a[2] * (b[0] * c[1] - b[1] * c[0]))
                    / 6.0
            });
        Some(fsum(terms).abs())
    }
}

/// Sum finite tetrahedron terms with non-overlapping partials, like math.fsum.
/// The final half-even correction matters when two partials straddle a tie.
fn fsum(terms: impl Iterator<Item = f64>) -> f64 {
    let mut partials = Vec::<f64>::new();
    for mut x in terms {
        let mut kept = 0;
        for i in 0..partials.len() {
            let mut y = partials[i];
            if x.abs() < y.abs() {
                std::mem::swap(&mut x, &mut y);
            }
            let hi = x + y;
            let lo = y - (hi - x);
            if lo != 0.0 {
                partials[kept] = lo;
                kept += 1;
            }
            x = hi;
        }
        partials.truncate(kept);
        if x != 0.0 {
            partials.push(x);
        }
    }
    let mut hi = partials.pop().unwrap_or(0.);
    while let Some(y) = partials.pop() {
        let old = hi;
        hi += y;
        let lo = y - (hi - old);
        if lo != 0.0 {
            if partials.last().is_some_and(|p| p.signum() == lo.signum()) {
                let twice = lo * 2.;
                let rounded = hi + twice;
                if rounded - hi == twice {
                    hi = rounded;
                }
            }
            break;
        }
    }
    hi
}

pub struct Surface {
    pub positions: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
    pub owners: Vec<u32>,
    pub faces: Vec<u128>,
}

impl Store {
    /// Manifold3D needs equal positions welded even if their store keys differ.
    /// Vertices follow first triangle use. Owners index the returned face list.
    pub fn indexed_surface(&self, faces: &[u32], h: Hypot) -> Surface {
        let mut surface = Surface {
            positions: vec![],
            triangles: vec![],
            owners: vec![],
            faces: vec![],
        };
        let mut ids = FxHashMap::default();
        for &f in faces {
            if self.state.fflags.get(f as usize) & INTERIOR != 0 {
                continue;
            }
            let fi = surface.faces.len() as u32;
            surface.faces.push(self.key(Kind::Face, f));
            for tri in self.face_triangles(f, None, h) {
                surface.triangles.push(tri.map(|p| {
                    let bits = p.map(|x| if x == 0.0 { 0 } else { x.to_bits() });
                    *ids.entry(bits).or_insert_with(|| {
                        let i = surface.positions.len() as u32;
                        surface.positions.push(p.map(f64::from));
                        i
                    })
                }));
                surface.owners.push(fi);
            }
        }
        surface
    }

    pub fn edge_defects(&self) -> [Vec<u128>; 3] {
        let mut issues = [vec![], vec![], vec![]];
        for edge in self.live_slots(Kind::Edge) {
            let uses = self
                .edge_faces(edge)
                .iter()
                .filter(|&&f| self.state.fflags.get(f as usize) & INTERIOR == 0)
                .count();
            match uses {
                2 => continue,
                n => issues[n.min(2)].push(self.key(Kind::Edge, edge)),
            }
        }
        issues
    }
}
