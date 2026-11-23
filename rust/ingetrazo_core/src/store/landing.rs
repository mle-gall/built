// SPDX-License-Identifier: GPL-3.0-or-later
//! Local admission check for a move that welds onto another vertex.
use super::{
    drawing::vector,
    drawing_face::Outline,
    geometry::{face_normal_length, Hypot},
    Store,
};
use std::collections::HashSet;

type Point = [f64; 3];
type XY = [f64; 2];
pub(super) type Corner = (i128, Point);
pub(super) type Rings = Vec<Vec<Corner>>;
pub(super) type Edge = (Corner, Corner, Vec<i128>);
pub(super) const DEFECTS: [&str; 6] = [
    "duplicate_face",
    "overlapping_face",
    "pinched_face",
    "crossed_face",
    "flat_face",
    "zero_edge",
];

pub(super) fn increased<T: PartialOrd>(
    before: [T; 6],
    after: [T; 6],
) -> impl Iterator<Item = &'static str> {
    DEFECTS
        .into_iter()
        .zip(before.into_iter().zip(after))
        .filter_map(|(kind, (a, b))| (b > a).then_some(kind))
}

fn orient(a: XY, b: XY, c: XY) -> i8 {
    let v = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    if v.abs() < 1e-9 {
        0
    } else if v > 0. {
        1
    } else {
        -1
    }
}
fn on_segment(a: XY, b: XY, c: XY) -> bool {
    (0..2).all(|i| a[i].min(b[i]) - 1e-9 <= c[i] && c[i] <= a[i].max(b[i]) + 1e-9)
}
fn cross([a, b]: [XY; 2], [c, d]: [XY; 2], touching: bool) -> bool {
    let [p, q, r, s] = [
        orient(a, b, c),
        orient(a, b, d),
        orient(c, d, a),
        orient(c, d, b),
    ];
    (p * q < 0 && r * s < 0)
        || (touching
            && ((p == 0 && on_segment(a, b, c))
                || (q == 0 && on_segment(a, b, d))
                || (r == 0 && on_segment(c, d, a))
                || (s == 0 && on_segment(c, d, b))))
}
fn sides<T: Copy>(ring: &[T]) -> impl Iterator<Item = [T; 2]> + '_ {
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .map(|(&a, &b)| [a, b])
}
fn crosses_itself(ring: &[Corner], normal: Point) -> bool {
    if ring.len() < 4 {
        return false;
    }
    let mut axis = 0;
    for i in 1..3 {
        if normal[i].abs() > normal[axis].abs() {
            axis = i;
        }
    }
    let axes = match axis {
        0 => [1, 2],
        1 => [0, 2],
        _ => [0, 1],
    };
    let points: Vec<_> = ring.iter().map(|(_, p)| [p[axes[0]], p[axes[1]]]).collect();
    let crosses = sides(&points).enumerate().any(|(i, ab)| {
        sides(&points)
            .enumerate()
            .skip(i + 2)
            .any(|(j, cd)| (i != 0 || j + 1 != points.len()) && cross(ab, cd, true))
    });
    crosses
}

impl Outline {
    fn overlaps(&self, other: &Self, h: Hypot) -> bool {
        let normal = self.normal;
        if normal.dot(other.normal, h).abs() < 1. - 1e-6 {
            return false;
        }
        let origin = vector(self.rings[0][0]);
        if other.rings[0]
            .iter()
            .any(|&p| vector(p).sub(origin).dot(normal, h).abs() > 1e-4)
        {
            return false;
        }
        for face in [self, other] {
            if face
                .interior()
                .is_some_and(|p| self.solid_at(p) && other.solid_at(p))
            {
                return true;
            }
        }
        let a = self.polygon(&self.rings[0]);
        let b = self.polygon(&other.rings[0]);
        let crossed = sides(&a).any(|ab| sides(&b).any(|cd| cross(ab, cd, false)));
        crossed
    }
}

pub(super) fn counts(edges: Vec<Edge>, faces: Vec<Rings>, area_eps: f64, h: Hypot) -> [usize; 6] {
    let mut counts = [0; 6];
    for ((a, p), (b, q), faces) in edges {
        let delta = vector(p).sub(vector(q));
        if a == b || delta.dot(delta, h) < 1e-18 {
            counts[5] += 1;
        }
        if faces.iter().copied().collect::<HashSet<_>>().len() < faces.len() {
            counts[2] += 1;
        }
    }
    let mut seen = HashSet::new();
    let mut regions = Vec::new();
    for rings in faces {
        let outer = &rings[0];
        let points: Vec<_> = outer.iter().map(|&(_, p)| vector(p)).collect();
        let indices: Vec<_> = (0..points.len() as u32).collect();
        let (normal, length) = face_normal_length(&points, &indices, h);
        let distinct = outer.iter().map(|&(k, _)| k).collect::<HashSet<_>>().len();
        for ring in &rings {
            let unique = ring.iter().map(|&(k, _)| k).collect::<HashSet<_>>();
            if unique.len() < ring.len() {
                counts[2] += 1;
            } else if crosses_itself(ring, normal.into()) {
                counts[3] += 1;
            }
        }
        let mut signature: Vec<_> = sides(outer)
            .map(|[(a, _), (b, _)]| [a.min(b), a.max(b)])
            .collect();
        signature.sort_unstable();
        signature.dedup();
        if !seen.insert(signature) {
            counts[0] += 1;
        }
        if distinct < 3 || length * 0.5 < area_eps {
            counts[4] += 1;
        }
        if distinct >= 3 && length * 0.5 >= area_eps {
            let positions = rings
                .into_iter()
                .map(|r| r.into_iter().map(|(_, p)| p).collect())
                .collect();
            let face = Outline::from_rings(positions, normal, h);
            counts[1] += regions
                .iter()
                .filter(|prior: &&Outline| prior.overlaps(&face, h))
                .count();
            regions.push(face);
        }
    }
    counts
}
impl Store {
    pub(super) fn landing_counts(&self, edges: &[u32], eps: f64, h: Hypot) -> [usize; 6] {
        let corner = |v: u32| (v as i128, *self.state.vpos.get(v as usize));
        let mut seen = HashSet::new();
        let mut faces = Vec::new();
        let edges = edges
            .iter()
            .map(|&e| {
                let [a, b] = *self.state.ends.get(e as usize);
                let near = self.edge_faces(e);
                for &f in near {
                    if seen.insert(f) {
                        faces.push(
                            self.face_rings(f)
                                .into_iter()
                                .map(|r| r.into_iter().map(corner).collect())
                                .collect(),
                        );
                    }
                }
                (
                    corner(a),
                    corner(b),
                    near.iter().map(|&f| f as i128).collect(),
                )
            })
            .collect();
        counts(edges, faces, eps, h)
    }
}
