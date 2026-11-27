// SPDX-License-Identifier: GPL-3.0-or-later
//! Erase a selection in pick order. Incidence alone cannot identify a dying
//! face: a separate, coincident boundary segment has the same position keys.
use super::{
    commands::cell, drawing::vector, edit::Change, geometry::Hypot, keys::Kind, stitch::Plan,
    topo::OpError, Store,
};
use std::collections::{HashMap, HashSet, VecDeque};

type Point = [f64; 3];
type Segment = [[u64; 3]; 2];
pub(super) type Plane = (Point, Point);

#[derive(Default)]
pub(super) struct Erasure {
    pub dirty: usize,
    pub edges: Vec<u128>,
    pub faces: Vec<u128>,
    pub merges: Vec<(u128, Vec<u128>, bool)>,
    pub slits: Vec<Plane>,
    pub across: Vec<u128>,
}

fn signature(points: impl Iterator<Item = Point>) -> Vec<[u64; 3]> {
    let mut cells: Vec<_> = points.map(cell).collect();
    cells.sort_unstable();
    cells.dedup();
    cells
}

fn segment(a: Point, b: Point) -> Segment {
    let mut pair = [cell(a), cell(b)];
    pair.sort_unstable();
    pair
}

fn walks(ring: &[u32], a: u32, b: u32) -> bool {
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .any(|(&x, &y)| (x == a && y == b) || (x == b && y == a))
}

impl Store {
    fn erase_face(&mut self, f: u32) -> Result<(), OpError> {
        if self.remove_face_blocked(f) {
            return Err(OpError::Fallback("unsupported incidence"));
        }
        let mut change = Change::default();
        self.remove_face(f, &mut change);
        self.finish(change);
        Ok(())
    }

    fn erase_edge(&mut self, e: u32) {
        let mut change = Change::default();
        self.remove_edge(e, &mut change);
        self.finish(change);
    }

    fn erase_pair(&self, faces: &[u32], h: Hypot) -> Option<[u32; 2]> {
        if faces.len() < 2 {
            return None;
        }
        let normals: Vec<_> = faces.iter().map(|&f| self.face_normal(f, h)).collect();
        let mut pair = None;
        for (i, &a) in normals.iter().enumerate() {
            for (j, &b) in normals.iter().enumerate().skip(i + 1) {
                if a.dot(b, h) > 0.999 {
                    if pair.is_some() {
                        return None;
                    }
                    pair = Some((i, j));
                }
            }
        }
        let (i, j) = pair?;
        if normals.iter().enumerate().any(|(k, &n)| {
            faces[k] != faces[i] && faces[k] != faces[j] && normals[i].dot(n, h).abs() > 0.999
        }) {
            return None;
        }
        Some([faces[i], faces[j]])
    }

    fn erase_merge(
        &mut self,
        pair: [u32; 2],
        h: Hypot,
        out: &mut Erasure,
    ) -> Result<Option<u32>, OpError> {
        let plan = self.plan_dissolve_region(&pair, h);
        let mut change = Change::default();
        let (merged, bumps, stacked) = match plan {
            Plan::No => {
                self.finish(change);
                return Ok(None);
            }
            Plan::Stacked(fs) => {
                for &f in &fs[1..] {
                    self.remove_face(f, &mut change);
                }
                (fs[0], fs.len() - 1, true)
            }
            Plan::Merge(m) => {
                let mut keep = Vec::new();
                if m.holes.len() > 1 {
                    let rings: Vec<_> = m
                        .holes
                        .iter()
                        .map(|r| {
                            r.iter()
                                .map(|&v| vector(*self.state.vpos.get(v as usize)))
                                .collect()
                        })
                        .collect();
                    keep = super::fillet::maximal_holes(&rings, h);
                }
                let (f, bumps) = self.apply_merge(&m, &keep, &mut change)?;
                (f, bumps, false)
            }
        };
        if self.dangling(&pair) {
            return Err(OpError::Fallback("unsupported merge incidence"));
        }
        self.finish(change);
        out.dirty += bumps;
        out.merges.push((
            self.key(Kind::Face, merged),
            pair.iter().map(|&f| self.key(Kind::Face, f)).collect(),
            stacked,
        ));
        Ok(Some(merged))
    }

    fn index_erase_face(
        &self,
        f: u32,
        targets: &HashSet<Segment>,
        index: &mut HashMap<Segment, Vec<u32>>,
    ) {
        let ring = self.outer_ring(f);
        for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            let key = segment(
                *self.state.vpos.get(a as usize),
                *self.state.vpos.get(b as usize),
            );
            if targets.contains(&key) {
                index.entry(key).or_default().push(f);
            }
        }
    }

    pub(super) fn erase_selection(
        &mut self,
        loops: &[Vec<Point>],
        ends: &[[Point; 2]],
        h: Hypot,
    ) -> Result<Erasure, OpError> {
        let mut out = Erasure::default();
        if !loops.is_empty() {
            let mut requested: HashMap<_, VecDeque<_>> = loops
                .iter()
                .map(|r| (signature(r.iter().copied()), VecDeque::new()))
                .collect();
            for f in self.live_slots(Kind::Face) {
                let key = signature(
                    self.outer_ring(f)
                        .into_iter()
                        .map(|v| *self.state.vpos.get(v as usize)),
                );
                if let Some(bucket) = requested.get_mut(&key) {
                    bucket.push_back(f);
                }
            }
            for r in loops {
                if let Some(f) = requested
                    .get_mut(&signature(r.iter().copied()))
                    .and_then(VecDeque::pop_front)
                {
                    out.faces.push(self.key(Kind::Face, f));
                    self.erase_face(f)?;
                    out.dirty += 1;
                }
            }
        }
        let mut targets = HashSet::new();
        for &[p, q] in ends {
            if let Some((a, b)) = self.lookup(p).zip(self.lookup(q)) {
                targets.insert(segment(
                    *self.state.vpos.get(a as usize),
                    *self.state.vpos.get(b as usize),
                ));
            }
        }
        let mut index = HashMap::new();
        if !targets.is_empty() {
            for f in self.live_slots(Kind::Face) {
                self.index_erase_face(f, &targets, &mut index);
            }
        }
        let mut endpoints = std::collections::BTreeSet::new();
        let mut dead = HashSet::new();
        for &[p, q] in ends {
            let Some(e) = self
                .lookup(p)
                .zip(self.lookup(q))
                .and_then(|(a, b)| self.find_edge(a, b))
            else {
                continue;
            };
            out.edges.push(self.key(Kind::Edge, e));
            let ends = *self.state.ends.get(e as usize);
            let faces = self.edge_faces(e).to_vec();
            if faces.len() == 2 && faces[0] == faces[1] {
                let f = faces[0];
                let origin = *self.state.vpos.get(self.outer_ring(f)[0] as usize);
                endpoints.extend(ends);
                out.slits.push((origin, self.face_normal(f, h).into()));
                self.erase_edge(e);
                out.dirty += 1;
                continue;
            }
            if let Some(pair) = self.erase_pair(&faces, h) {
                if let Some(merged) = self.erase_merge(pair, h, &mut out)? {
                    dead.extend(pair);
                    self.index_erase_face(merged, &targets, &mut index);
                    if faces.len() > 2 {
                        out.across.extend(
                            faces
                                .iter()
                                .filter(|f| !pair.contains(f))
                                .map(|&f| self.key(Kind::Face, f)),
                        );
                        self.erase_edge(e);
                        out.dirty += 1;
                    }
                    continue;
                }
            }
            // A successfully dissolved coplanar seam is already simplified
            // by the normal face-healing pass, in its original store order.
            endpoints.extend(ends);
            let [a, b] = ends;
            let key = segment(
                *self.state.vpos.get(a as usize),
                *self.state.vpos.get(b as usize),
            );
            let dying: Vec<_> = index
                .get(&key)
                .into_iter()
                .flatten()
                .copied()
                .filter(|f| !dead.contains(f))
                .collect();
            for f in faces {
                if dead.contains(&f) || dying.contains(&f) {
                    continue;
                }
                let rings = self.face_rings(f);
                if walks(&rings[0], a, b) {
                    continue;
                }
                for hole in rings.iter().skip(1).filter(|r| walks(r, a, b)) {
                    let mut change = Change::default();
                    self.remove_hole(f, hole, &mut change);
                    self.finish(change);
                }
            }
            for f in dying {
                dead.insert(f);
                out.faces.push(self.key(Kind::Face, f));
                self.erase_face(f)?;
                out.dirty += 1;
            }
            self.erase_edge(e);
            out.dirty += 1;
        }
        self.heal_erased_endpoints(endpoints, h)?;
        Ok(out)
    }

    fn curve_corner_can_heal(&self, v: u32, e: u32, f: u32) -> bool {
        let faces = self.edge_faces(e);
        let other = self.edge_faces(f);
        faces.len() == other.len()
            && faces.iter().all(|face| {
                other.contains(face)
                    && self
                        .face_rings(*face)
                        .iter()
                        .all(|ring| !ring.contains(&v) || ring.len() > 3)
            })
    }

    /// Inspect only junctions at the ends of erased edges. A segment of
    /// an authored polygon or arc keeps both its original corner vertices.
    pub(super) fn heal_erased_endpoints(
        &mut self,
        endpoints: std::collections::BTreeSet<u32>,
        h: Hypot,
    ) -> Result<(), OpError> {
        let mut counts = None;
        for v in endpoints {
            if !self.alive(Kind::Vertex, v) {
                continue;
            }
            let pair: Vec<_> = self.edges_at(v).collect();
            let [e, f] = pair[..] else { continue };
            let [a, b] = [e, f].map(|s| self.other_end(s, v));
            if self.find_edge(a, b).is_some() {
                continue;
            }
            let mut tags = [e, f].map(|s| *self.state.ecurve.get(s as usize));
            tags.sort();
            let vk = self.key(Kind::Vertex, v);
            let split = self.state.split_runs.get(&vk) == Some(&tags);
            if tags[0] != tags[1] && !split {
                continue;
            }
            let flags = *self.state.eflags.get(e as usize);
            let layer = *self.state.elayer.get(e as usize);
            if flags != *self.state.eflags.get(f as usize)
                || layer != *self.state.elayer.get(f as usize)
            {
                continue;
            }
            let p = self.def_point(v);
            let (u, w) = (self.def_point(a) - p, self.def_point(b) - p);
            let straight = u.dot(w) < 0.
                && u.cross(w).length()
                    <= crate::exact::bound::tolerance(p.abs().max_element()) * (u - w).length();
            if straight {
                if !self.collapsible(v, h) {
                    continue;
                }
            } else if !split || !self.curve_corner_can_heal(v, e, f) {
                continue;
            }
            let id = self.pid(
                Kind::Edge,
                if u.length_squared() >= w.length_squared() {
                    e
                } else {
                    f
                },
            );
            // A stroke splits the displayed chord. Its middle point can
            // be off the support, so recover the circle from a neighbour
            // of the same curve when both temporary intervals fell away.
            let curve = [e, f]
                .into_iter()
                .chain(self.edges_at(a))
                .chain(self.edges_at(b))
                .filter(|&s| {
                    (s == e || s == f)
                        || (tags[0] >= 0 && tags.contains(self.state.ecurve.get(s as usize)))
                })
                .filter_map(|s| self.curved_edge(s))
                .find(|bind| {
                    [a, b].iter().all(|&v| {
                        let p = self.def_point(v);
                        bind.curve
                            .contains(p, crate::exact::bound::tolerance(p.abs().max_element()))
                    })
                })
                .map(|b| b.curve.clone());
            if !straight && curve.is_none() {
                continue;
            }
            let mut change = Change::default();
            let occurrences = counts.get_or_insert_with(|| self.corner_counts());
            let joined = match self.collapse_vertex(v, occurrences, &mut change) {
                Ok(c) => c.edge,
                Err(OpError::Fallback(_)) => continue,
                Err(e) => return Err(e),
            };
            self.state.eflags.set(joined as usize, flags);
            self.state.elayer.set(joined as usize, layer);
            self.set_edge_curve(joined, tags[0]);
            self.set_pid(Kind::Edge, joined, id);
            if let Some(curve) = curve {
                let _ = self.bind_circle(&[joined], curve);
            }
            if tags[0] != tags[1] {
                for s in self.live_slots(Kind::Edge) {
                    if *self.state.ecurve.get(s as usize) == tags[1] {
                        self.set_edge_curve(s, tags[0]);
                    }
                }
            }
            if self.state.split_runs.contains_key(&vk) {
                std::sync::Arc::make_mut(&mut self.state.split_runs).remove(&vk);
            }
            self.finish(change);
        }
        Ok(())
    }

    pub(super) fn erase_soft_seams(&mut self) -> usize {
        let mut count = 0;
        for edge in self.live_slots(Kind::Edge) {
            if self.state.eflags.get(edge as usize) & super::SOFT != 0
                && self.edge_faces(edge).is_empty()
            {
                self.erase_edge(edge);
                count += 1;
            }
        }
        count
    }
}
