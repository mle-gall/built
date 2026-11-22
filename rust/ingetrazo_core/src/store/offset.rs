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

fn arrangement_axes(n: V3, h: Hypot) -> (V3, V3) {
    let basis_ref = if n.0.abs() > 0.9 {
        V3(0., 1., 0.)
    } else {
        V3(1., 0., 0.)
    };
    let bu = normalized(basis_ref.sub(n.mul(basis_ref.dot(n, h))), h);
    let bv = normalized(n.cross(bu, h), h);
    (bu, bv)
}

/// The largest triangulated ear provides an interior witness even for concave
/// regions. Measure its signed clearance from the original boundary.
pub fn keep_regions(
    store: &Store,
    face: u32,
    regions: Vec<Vec<[f32; 3]>>,
    normal: [f32; 3],
    distance: f64,
    h: Hypot,
) -> Vec<bool> {
    use crate::earcut::earcut;
    use crate::facegeom::{is_convex, plane_axes};
    let n = normalized(V3(normal[0], normal[1], normal[2]), h);
    let (u, v) = plane_axes(n, h);
    let (bu, bv) = arrangement_axes(n, h);
    let rings = store.face_rings(face);
    let boundary: Vec<_> = rings
        .first()
        .into_iter()
        .flatten()
        .map(|&i| {
            let p = store.state.vpos.get(i as usize);
            V3(p[0] as f32, p[1] as f32, p[2] as f32)
        })
        .collect();
    if boundary.len() < 3 {
        return vec![false; regions.len()];
    }
    let flat = |p: V3| {
        let rel = p.sub(boundary[0]);
        (rel.dot(bu, h) as f64, rel.dot(bv, h) as f64)
    };
    let outline: Vec<_> = boundary.iter().copied().map(flat).collect();
    regions
        .into_iter()
        .map(|region| {
            if region.len() < 3 {
                return false;
            }
            let points: Vec<_> = region.iter().map(|p| V3(p[0], p[1], p[2])).collect();
            let xy: Vec<_> = points
                .iter()
                .map(|&p| {
                    let rel = p.sub(points[0]);
                    (rel.dot(u, h) as f64, rel.dot(v, h) as f64)
                })
                .collect();
            let triangles = if is_convex(&xy) {
                (1..points.len() - 1).map(|i| [0, i, i + 1]).collect()
            } else {
                earcut(&xy.iter().map(|&(x, y)| [x, y]).collect::<Vec<_>>(), &[])
            };
            let mut best = 0.;
            let mut witness = None;
            for [a, b, c] in triangles {
                let (a, b, c) = (points[a], points[b], points[c]);
                let cross = b.sub(a).cross(c.sub(a), h);
                let area = cross.dot(cross, h);
                if area > best {
                    best = area;
                    witness = Some(a.add(b).add(c).div(3.));
                }
            }
            let Some(p) = witness else {
                return false;
            };
            let inside = contains(&outline, flat(p));
            let mut depth = f32::INFINITY;
            for (i, &a) in boundary.iter().enumerate() {
                let j = (i + 1) % boundary.len();
                let delta = boundary[j].sub(a);
                let l2 = delta.dot(delta, h);
                let t = if l2 < 1e-18 {
                    0.
                } else {
                    (p.sub(a).dot(delta, h) / l2).clamp(0., 1.)
                };
                depth = depth.min(length(p.sub(a.add(delta.mul(t))), h));
            }
            let signed = if inside { depth } else { -depth };
            signed as f64
                >= if distance > 0. {
                    distance * 0.75
                } else {
                    distance
                }
        })
        .collect()
}

type XY = (f64, f64);

fn edges(ring: &[XY]) -> impl Iterator<Item = (XY, XY)> + '_ {
    ring.iter()
        .copied()
        .zip(ring.iter().copied().cycle().skip(1))
}

pub(super) fn contains(ring: &[XY], (x, y): XY) -> bool {
    edges(ring).fold(false, |inside, ((xj, yj), (xi, yi))| {
        inside ^ ((yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi + 1e-30) + xi)
    })
}

fn meet(p: XY, q: XY, r: XY, s: XY) -> bool {
    let orient = |a: XY, b: XY, c: XY| {
        let v = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
        if v.abs() < 1e-6 {
            0
        } else if v > 0. {
            1
        } else {
            -1
        }
    };
    let between = |a: f64, b: f64, c: f64| a.min(c) - 1e-6 <= b && b <= a.max(c) + 1e-6;
    let on = |a: XY, b: XY, c: XY| between(a.0, b.0, c.0) && between(a.1, b.1, c.1);
    let (a, b, c, d) = (
        orient(p, q, r),
        orient(p, q, s),
        orient(r, s, p),
        orient(r, s, q),
    );
    (a != b && c != d)
        || (a == 0 && on(p, r, q))
        || (b == 0 && on(p, s, q))
        || (c == 0 && on(r, p, s))
        || (d == 0 && on(r, q, s))
}

pub fn place_holes(
    store: &Store,
    face: u32,
    regions: &[Vec<[f32; 3]>],
    normal: [f32; 3],
    h: Hypot,
) -> Option<Vec<i64>> {
    let n = normalized(V3(normal[0], normal[1], normal[2]), h);
    let (u, v) = arrangement_axes(n, h);
    let p = regions.first()?.first()?;
    let origin = V3(p[0], p[1], p[2]);
    let flat = |p: V3| {
        let r = p.sub(origin);
        (r.dot(u, h) as f64, r.dot(v, h) as f64)
    };
    let flat_regions: Vec<Vec<_>> = regions
        .iter()
        .map(|ring| ring.iter().map(|p| flat(V3(p[0], p[1], p[2]))).collect())
        .collect();
    let mut targets = Vec::new();
    for ring in store.face_rings(face).iter().skip(1) {
        let hole: Vec<_> = ring
            .iter()
            .map(|&s| {
                let p = store.state.vpos.get(s as usize);
                flat(V3(p[0] as f32, p[1] as f32, p[2] as f32))
            })
            .collect();
        if flat_regions
            .iter()
            .any(|r| edges(&hole).any(|(a, b)| edges(r).any(|(c, d)| meet(a, b, c, d))))
        {
            return None;
        }
        let mut target = -1;
        for (i, r) in flat_regions.iter().enumerate() {
            if contains(r, *hole.first()?) {
                target = i as i64;
                break;
            }
            if contains(&hole, *r.first()?) {
                return None;
            }
        }
        targets.push(target);
    }
    Some(targets)
}
