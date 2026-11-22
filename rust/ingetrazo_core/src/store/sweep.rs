// SPDX-License-Identifier: GPL-3.0-or-later
//! Follow Me station projection, wall/cap construction and loose profile cleanup.
use super::{edit::Change, topo::OpError, Store};
use crate::facegeom::{length, normalized, Hypot, V3};

type Ring = Vec<[f64; 3]>;
pub type Station = Vec<Ring>;

fn vector(p: [f64; 3]) -> V3 {
    V3(p[0] as f32, p[1] as f32, p[2] as f32)
}
fn coords(v: V3) -> [f64; 3] {
    [v.0 as f64, v.1 as f64, v.2 as f64]
}

impl Store {
    pub fn sweep_stations(
        &self,
        face: u32,
        path: &[[f64; 3]],
        closed: bool,
        h: Hypot,
    ) -> Option<Vec<Station>> {
        if path.len() < 2 {
            return None;
        }
        let spans = if closed { path.len() } else { path.len() - 1 };
        let mut dirs = Vec::with_capacity(spans);
        for i in 0..spans {
            let v = vector(path[(i + 1) % path.len()]).sub(vector(path[i]));
            if length(v, h) < 1e-9 {
                return None;
            }
            dirs.push(normalized(v, h));
        }
        let mut rings: Station = self
            .face_rings(face)
            .iter()
            .map(|r| {
                r.iter()
                    .map(|&v| *self.state.vpos.get(v as usize))
                    .collect()
            })
            .collect();
        let mut stations = Vec::new();
        for (i, &anchor) in path.iter().enumerate() {
            let dir = dirs[i.saturating_sub(1)];
            let normal = if !closed && (i == 0 || i == spans) {
                dir
            } else {
                let sum = dirs[(i + spans - 1) % spans].add(dirs[i % spans]);
                if length(sum, h) < 1e-9 {
                    return None;
                }
                normalized(sum, h)
            };
            let denom = dir.dot(normal, h);
            if denom.abs() < 1e-9 {
                return None;
            }
            for p in rings.iter_mut().flatten() {
                let v = vector(*p);
                let t = vector(anchor).sub(v).dot(normal, h) / denom;
                *p = coords(v.add(dir.mul(t)));
            }
            stations.push(rings.clone());
        }
        Some(stations)
    }

    pub fn sweep_face(
        &mut self,
        face: u32,
        stations: &[Station],
        closed: bool,
        h: Hypot,
        ch: &mut Change,
    ) -> Result<(), OpError> {
        if stations.len() < 2
            || stations[0].is_empty()
            || stations.iter().any(|s| {
                s.len() != stations[0].len()
                    || s.iter()
                        .zip(&stations[0])
                        .any(|(a, b)| a.len() < 3 || a.len() != b.len())
            })
        {
            return Err(OpError::Fallback("incompatible sweep station rings"));
        }
        let profile = self.face_rings(face);
        self.remove_face(face, ch);
        let spans = if closed {
            stations.len()
        } else {
            stations.len() - 1
        };
        for i in 0..spans {
            for (a, b) in stations[i].iter().zip(&stations[(i + 1) % stations.len()]) {
                for j in 0..a.len() {
                    let k = (j + 1) % a.len();
                    let mut wall = Vec::with_capacity(4);
                    let distinct = |p, q| length(vector(p).sub(vector(q)), h) >= 1e-9;
                    for p in [a[j], a[k], b[k], b[j]] {
                        if wall.last().is_none_or(|&q| distinct(p, q)) {
                            wall.push(p);
                        }
                    }
                    if wall.len() > 1 && !distinct(wall[0], *wall.last().unwrap()) {
                        wall.pop();
                    }
                    if wall.len() >= 3 {
                        self.add_face(&[wall], ch)?;
                    }
                }
            }
        }
        if !closed {
            let start = stations[0]
                .iter()
                .map(|r| r.iter().copied().rev().collect())
                .collect::<Vec<_>>();
            self.add_face(&start, ch)?;
            self.add_face(stations.last().unwrap(), ch)?;
        }
        for ring in profile {
            for (i, &v) in ring.iter().enumerate() {
                if let Some(e) = self.find_edge(v, ring[(i + 1) % ring.len()]) {
                    if self.edge_faces(e).is_empty() {
                        self.remove_edge(e, ch);
                    }
                }
            }
        }
        Ok(())
    }
}
