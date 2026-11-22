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
        compensated: bool,
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
        if closed {
            if let Some(stations) = super::revolution::stations(path, &rings, h, compensated) {
                return Some(stations);
            }
        }
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
        cap_holes: Option<&[Vec<bool>; 2]>,
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
        if cap_holes.is_some_and(|ends| ends.iter().any(|mask| mask.len() + 1 != stations[0].len()))
        {
            return Err(OpError::Fallback("incompatible cap hole mask"));
        }
        let profile = self.face_rings(face);
        // Orient each ring against its motion. A pole of a lathe can stay
        // fixed, so use the whole ring for the direction of travel.
        let reverse: Vec<bool> = stations[0]
            .iter()
            .zip(&stations[1])
            .enumerate()
            .map(|(i, (start, end))| {
                let mut area = V3(0., 0., 0.);
                let mut travel = V3(0., 0., 0.);
                for j in 0..start.len() {
                    let a = vector(start[j]).sub(vector(start[0]));
                    let b = vector(start[(j + 1) % start.len()]).sub(vector(start[0]));
                    area = area.add(a.cross(b, h));
                    travel = travel.add(vector(end[j]).sub(vector(start[j])));
                }
                (area.dot(travel, h) < 0.) != (i > 0)
            })
            .collect();
        self.remove_face(face, ch);
        let spans = if closed {
            stations.len()
        } else {
            stations.len() - 1
        };
        for i in 0..spans {
            for (ring, (a, b)) in stations[i]
                .iter()
                .zip(&stations[(i + 1) % stations.len()])
                .enumerate()
            {
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
                        if reverse[ring] {
                            wall.reverse();
                        }
                        self.add_face(&[wall], ch)?;
                    }
                }
            }
        }
        if !closed {
            for (end, station) in [&stations[0], stations.last().unwrap()]
                .into_iter()
                .enumerate()
            {
                let mut cap = Vec::new();
                for (i, ring) in station.iter().enumerate() {
                    if i > 0 && cap_holes.is_some_and(|m| !m[end][i - 1]) {
                        continue;
                    }
                    cap.push(if (end == 0) != reverse[i] {
                        ring.iter().copied().rev().collect()
                    } else {
                        ring.clone()
                    });
                }
                self.add_face(&cap, ch)?;
            }
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

impl Store {
    fn sweep_edge(&self, a: [f64; 3], b: [f64; 3]) -> Option<u32> {
        let (a, b) = (self.lookup(a)?, self.lookup(b)?);
        if a == b {
            None
        } else {
            self.find_edge(a, b)
        }
    }

    /// Post-stitch seams. Only an edge with two sweep faces, or a newly
    /// created edge, can become soft. The original flags otherwise survive.
    pub fn soften_sweep(&mut self, before: &Store, h: Hypot, ch: &mut Change) {
        use super::{keys::Kind, SOFT};
        for e in self.live_slots(Kind::Edge) {
            let key = self.key(Kind::Edge, e);
            let flags = *self.state.eflags.get(e as usize);
            let [a, b] = *self.edge_faces(e) else {
                continue;
            };
            if flags & SOFT != 0
                || (before.slot(Kind::Edge, key).is_some()
                    && [a, b]
                        .iter()
                        .any(|&f| before.slot(Kind::Face, self.key(Kind::Face, f)).is_some()))
            {
                continue;
            }
            let dot = normalized(self.face_normal(a, h), h)
                .dot(normalized(self.face_normal(b, h), h), h) as f64;
            if dot > 0.85 && dot < 0.99995 {
                self.state.eflags.set(e as usize, flags | SOFT);
                ch.modified[1].push(key);
            }
        }
    }

    /// Order of curve allocations is observable. End-cap runs come first,
    /// then the rails from each profile corner in station order.
    pub fn sweep_curves(
        &self,
        before: &Store,
        face: u32,
        path: &[[f64; 3]],
        stations: &[Station],
        closed: bool,
    ) -> Vec<Vec<u128>> {
        use super::{keys::Kind, SOFT};
        let mut out = Vec::new();
        let mut tagged = std::collections::HashSet::new();
        let mut tag = |edges: Vec<u32>| {
            let group: Vec<_> = edges
                .into_iter()
                .filter(|&e| {
                    *self.state.ecurve.get(e as usize) < 0
                        && self.state.eflags.get(e as usize) & SOFT == 0
                        && tagged.insert(e)
                })
                .map(|e| self.key(Kind::Edge, e))
                .collect();
            if !group.is_empty() {
                out.push(group);
            }
        };
        let curve = |e: Option<u32>| e.map_or(-1, |e| *before.state.ecurve.get(e as usize));
        if !closed {
            let rings = before.face_rings(face);
            for cap in [&stations[0], stations.last().unwrap()] {
                for (ring, source) in cap.iter().zip(&rings) {
                    let mut groups: Vec<(i64, Vec<u32>)> = Vec::new();
                    for (i, &v) in ring.iter().enumerate().take(source.len()) {
                        let c = curve(before.find_edge(source[i], source[(i + 1) % source.len()]));
                        if c < 0 {
                            continue;
                        }
                        let at = groups.iter().position(|g| g.0 == c).unwrap_or_else(|| {
                            groups.push((c, Vec::new()));
                            groups.len() - 1
                        });
                        if let Some(e) = self.sweep_edge(v, ring[(i + 1) % ring.len()]) {
                            groups[at].1.push(e);
                        }
                    }
                    for (_, edges) in groups {
                        tag(edges);
                    }
                }
            }
        }
        let spans = if closed { path.len() } else { path.len() - 1 };
        let curves: Vec<_> = (0..spans)
            .map(|i| curve(before.sweep_edge(path[i], path[(i + 1) % path.len()])))
            .collect();
        if curves.iter().all(|c| *c < 0) {
            return out;
        }
        let start = if closed {
            (0..spans)
                .find(|&i| curves[i] != curves[(i + spans - 1) % spans])
                .unwrap_or(0)
        } else {
            0
        };
        for (l, ring) in stations[0].iter().enumerate() {
            for j in 0..ring.len() {
                let mut run = Vec::new();
                let mut current = -1;
                for k in 0..spans {
                    let i = (start + k) % spans;
                    if current != curves[i] {
                        tag(std::mem::take(&mut run));
                        current = curves[i];
                    }
                    if current >= 0 {
                        if let Some(e) = self
                            .sweep_edge(stations[i][l][j], stations[(i + 1) % stations.len()][l][j])
                        {
                            run.push(e);
                        }
                    }
                }
                tag(run);
            }
        }
        out
    }
}
