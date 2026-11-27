// SPDX-License-Identifier: GPL-3.0-or-later
//! Construct the shell or tunnel, then repair and orient its connected components.
use super::{
    commands::{cell, Cell},
    drawing::{point, vector},
    edit::Change,
    geometry::{face_normal_length, Hypot},
    keys::Kind,
    push_rebuild::{Fresh, Paint},
    push_through::centroid,
    stitch::Plan,
    topo::OpError,
    Store, INTERIOR,
};
use crate::{
    orient::Geom,
    pushscan::sweeps_free_sheet,
    pyset::PySet,
    scans::{MeshScan, EFACES, FFLAGS, RINGS},
};
use glam::DVec3;
use std::collections::HashSet;
type Point = [f64; 3];

#[derive(Default)]
pub(super) struct Extruded {
    pub bumps: usize,
    pub steps: usize,
    pub painted: Vec<(u128, u32)>,
    pub flipped: Vec<u128>,
    pub faces: Vec<u128>,
    pub edges: Vec<u128>,
}

impl Extruded {
    fn give(&mut self, paint: &mut Paint, face: u128, class: u32) {
        paint.own.insert(face, class);
        self.painted.push((face, class));
    }
    fn turn_paint(&mut self, st: &Store, paint: &mut Paint, flipped: Vec<u128>, swapped: &[u32]) {
        for key in flipped {
            let f = st.slot(Kind::Face, key).unwrap();
            let old = paint.class(st, f);
            let new = swapped[old as usize];
            if new != old {
                self.give(paint, key, new);
            }
        }
    }
    fn dedupe(&mut self, st: &Store, paint: &mut Paint, drop: u32, keep: u32) {
        let class = paint.class(st, drop);
        if class != 0 && paint.class(st, keep) == 0 {
            self.give(paint, st.key(Kind::Face, keep), class);
        }
    }
}

impl Store {
    fn push_closed(&self) -> bool {
        self.n(Kind::Face) != 0
            && self
                .state.slots[Kind::Edge as usize]
                .live_slots()
                .all(|e| self.edge_faces(e).len() >= 2)
    }
    fn merge_push_sheet(
        &mut self,
        seeds: &[Point],
        mut fresh: HashSet<u128>,
        paint: &mut Paint,
        h: Hypot,
        result: &mut Extruded,
        ch: &mut Change,
    ) -> Result<(), OpError> {
        let mut start = 0;
        while let Some((next, keys)) =
            self.stitch_region(seeds.to_vec(), Some(fresh.clone()), start, h)
        {
            start = next;
            // The old sheet pass passed a Python set of face views to dissolve.
            let slots: Vec<_> = PySet::from_keys(keys)
                .iter()
                .filter_map(|k| self.slot(Kind::Face, k))
                .collect();
            let plan = self.plan_dissolve_region(&slots, h);
            if matches!(plan, Plan::No) {
                continue;
            }
            let mut donor = 0;
            let mut best = -1.;
            for &f in &slots {
                let class = paint.class(self, f);
                if class == 0 {
                    continue;
                }
                let pos: Vec<_> = self
                    .outer_ring(f)
                    .iter()
                    .map(|&v| vector(*self.state.vpos.get(v as usize)))
                    .collect();
                let area =
                    face_normal_length(&pos, &(0..pos.len() as u32).collect::<Vec<_>>(), h).1 * 0.5;
                if area > best {
                    best = area;
                    donor = class;
                }
            }
            let (merged, bumps) = match plan {
                Plan::Stacked(fs) => {
                    for &f in &fs[1..] {
                        self.remove_face(f, ch);
                    }
                    let own = paint.class(self, fs[0]);
                    if own != 0 {
                        donor = own;
                    }
                    (fs[0], fs.len() - 1)
                }
                Plan::Merge(m) => {
                    let holes: Vec<_> = m
                        .holes
                        .iter()
                        .map(|r| {
                            r.iter()
                                .map(|&v| vector(*self.state.vpos.get(v as usize)))
                                .collect()
                        })
                        .collect();
                    let keep = if holes.len() > 1 {
                        super::fillet::maximal_holes(&holes, h)
                    } else {
                        Vec::new()
                    };
                    self.apply_merge(&m, &keep, ch)?
                }
                Plan::No => unreachable!(),
            };
            result.bumps += bumps;
            let key = self.key(Kind::Face, merged);
            result.give(paint, key, donor);
            for f in slots {
                fresh.remove(&self.key(Kind::Face, f));
            }
            fresh.insert(key);
            start = 0;
        }
        Ok(())
    }
    pub(super) fn push_extrude(
        &mut self,
        face: u32,
        mut distance: f64,
        drag_normal: Option<Point>,
        keep: bool,
        oriented: bool,
        preview: bool,
        paint: &mut Paint,
        swapped: &[u32],
        surface: &[u32],
        h: Hypot,
        compensated: bool,
        index: Option<&std::sync::Arc<super::face_index::FaceIndex>>,
        ch: &mut Change,
    ) -> Result<Extruded, OpError> {
        let mut result = Extruded::default();
        let faces = self.live_slots(Kind::Face);
        let idx = faces
            .iter()
            .position(|&f| f == face)
            .ok_or(OpError::Fallback("missing push base"))? as u32;
        let pre =
            DVec3::from_array(drag_normal.unwrap_or_else(|| point(self.face_normal(face, h))));
        let closed = self.push_closed();
        if closed && !oriented {
            let flips = self.orient_push_faces(&faces, Some(&[idx]), h, compensated);
            result.turn_paint(self, paint, flips, swapped);
        }
        let normal = DVec3::from_array(point(self.face_normal(face, h)));
        if normal.dot(pre) < 0. {
            distance = -distance;
        }
        let index = index.map(|tree| tree.refresh(self));
        let (mut base, backed, sheet, was_solid) = self.push_region(face, closed, h, index.as_deref())?;
        let attached = backed && !keep;
        let opening = if attached {
            self.push_opening(face, distance, h, compensated, index.as_deref())
        } else {
            None
        };
        if was_solid && !closed && !oriented {
            let flips = self.orient_push_faces(&faces, Some(&[idx]), h, compensated);
            result.turn_paint(self, paint, flips, swapped);
        }
        let original = paint.class(self, face);
        let far_class = opening.as_ref().map(|(f, _, _)| paint.class(self, *f));
        if opening.is_none() && !backed {
            let scan = MeshScan::from_state(self.state.clone(), RINGS | EFACES | FFLAGS);
            let geom = Geom::of(&scan, h, compensated);
            if sweeps_free_sheet(&scan, &geom, idx, &base) {
                if distance > 0. {
                    self.apply_orientation(&[], &[face]);
                    result.turn_paint(self, paint, vec![self.key(Kind::Face, face)], swapped);
                } else {
                    for ring in &mut base {
                        ring.reverse();
                    }
                }
            }
        }
        let delta = normal * distance;
        let top: Vec<Vec<_>> = base
            .iter()
            .map(|r| {
                r.iter()
                    .map(|&p| (DVec3::from_array(p) + delta).to_array())
                    .collect()
            })
            .collect();
        let cap_point = *top
            .first()
            .and_then(|r| r.first())
            .ok_or(OpError::Fallback("empty push base"))?;
        let seed_set: HashSet<Cell> = base
            .iter()
            .chain(&top)
            .flatten()
            .copied()
            .map(cell)
            .collect();
        let mut seeds: Vec<_> = seed_set.iter().map(|k| k.map(f64::from_bits)).collect();
        seeds.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let upper = if let Some((far, back, rest)) = &opening {
            let mut caps = vec![back.clone()];
            if rest.is_none() {
                if let Some(islands) =
                    self.push_islands(face, &base[1..], *far, back, h, compensated)
                {
                    caps.extend(islands);
                }
            }
            caps
        } else {
            top
        };
        let before = self.state.clone();
        let shell = self.push_shell(face, base, upper, delta.to_array(), attached, opening, h)?;
        result.steps = shell.steps;
        result.bumps = shell.dirty;
        result.faces = shell.removed_faces;
        result.edges = shell.removed_edges;
        if let Some(key) = shell.source {
            let f = self
                .slot_any(Kind::Face, key)
                .ok_or(OpError::Fallback("missing cap paint donor"))?;
            let class = paint.class(self, f);
            for k in shell.remainders {
                result.give(paint, k, class);
            }
            if class != 0 {
                result.give(paint, shell.cap, surface[class as usize]);
            }
        }
        let class = far_class.unwrap_or(original);
        if class != 0 {
            result.give(paint, shell.cap, class);
        }
        let fresh: HashSet<_> = self.state.changes_since(&before).created[2].iter().copied().collect();
        // Preserve Python set iteration order for the solid rebuild. The
        // sheet dissolve only tests membership in its fresh-face set.
        let solid = backed && was_solid;
        let rebuild_keys = solid.then(|| Fresh::Delta(self.new_push_faces(&before).copy()));
        if keep && backed && distance < 0. {
            for &key in &fresh {
                let f = self.slot(Kind::Face, key).unwrap();
                let n = DVec3::from_array(point(self.face_normal(f, h))).normalize_or_zero();
                if n.dot(normal).abs() > 0.999
                    && (centroid(&self.position_rings(f)[0], compensated)
                        - DVec3::from_array(cap_point))
                    .dot(normal)
                    .abs()
                        < 1e-4
                {
                    self.set_face_flags(f, *self.state.fflags.get(f as usize) | INTERIOR);
                }
            }
        }
        let connected = self.stitch_connect(&seeds, !solid, 1e-4, h, ch)?;
        result.bumps += connected.bumps;
        for (d, k) in connected.deduped {
            result.dedupe(self, paint, d, k);
        }
        if let Some(keys) = rebuild_keys {
            let rebuilt = self.rebuild_push_planes(
                keys,
                &seeds,
                keep,
                distance < 0.,
                paint,
                compensated,
                h,
                ch,
            )?;
            result.bumps += rebuilt.bumps;
            result.painted.extend(rebuilt.painted);
            for (d, k) in rebuilt.deduped {
                if let (Some(d), Some(k)) =
                    (self.slot_any(Kind::Face, d), self.slot_any(Kind::Face, k))
                {
                    result.dedupe(self, paint, d, k);
                }
            }
        } else if !keep {
            self.merge_push_sheet(&seeds, fresh, paint, h, &mut result, ch)?;
        }
        if !preview {
            let orient = !sheet
                || self.push_closed()
                || {
                    let vertices = self.push_seed_vertices(&seed_set);
                    self.live_slots(Kind::Face).into_iter()
                        .find(|&f| self.touches_push_vertex(f, &vertices))
                        .is_some_and(|f| self.push_component_closed(f))
                };
            if orient {
                result.flipped = self.orient_push(&seed_set, h, compensated);
            }
        }
        Ok(result)
    }
}
