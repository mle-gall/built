// SPDX-License-Identifier: GPL-3.0-or-later
//! Position-addressed edits for the History drawing commands.
use super::{edit::Change, keys::Kind, Store};
use std::collections::HashSet;

type Point = [f64; 3];
pub(crate) type Cell = [u64; 3];

/// NumPy float64 to int64 maps a floor coordinate outside int64 to MIN.
pub(super) fn floor_cell(p: Point) -> [i64; 3] {
    p.map(|x| {
        let n = (x * 1e4).floor();
        if n >= i64::MIN as f64 && n < -(i64::MIN as f64) { n as i64 } else { i64::MIN }
    })
}

/// A position key supplied by a caller has already been rounded.
pub(crate) fn supplied_cell(p: Point) -> Cell {
    p.map(|x| if x == 0. { 0 } else { x.to_bits() })
}

/// Bits of Python's round(x, 4), canonicalising signed zero. When the
/// multiplication itself can change a rounding decision, round the original
/// binary64 value directly to four decimal places.
pub(crate) fn cell(point: Point) -> Cell {
    point.map(|x| {
        let scaled = x * 10000.;
        let rounded = if scaled.abs() >= (1u64 << 52) as f64 || scaled.fract().abs() == 0.5 {
            format!("{x:.4}").parse::<f64>().unwrap()
        } else {
            scaled.round_ties_even() / 10000.
        };
        if rounded == 0. {
            0
        } else {
            rounded.to_bits()
        }
    })
}

fn segment(a: Cell, b: Cell) -> [Cell; 2] {
    if a < b {
        [a, b]
    } else {
        [b, a]
    }
}

/// A landing may leave a face walking out to a tip and straight back.
pub(super) fn trim_spurs<T: Copy>(
    ring: &mut Vec<T>,
    same: impl Fn(T, T) -> bool,
    landed: impl Fn(T) -> bool,
) {
    while ring.len() >= 3 {
        let n = ring.len();
        let Some(i) = (0..n).find(|&i| {
            let prev = ring[(i + n - 1) % n];
            same(ring[i], prev) || (same(prev, ring[(i + 1) % n]) && landed(ring[i]))
        }) else {
            break;
        };
        if same(ring[i], ring[(i + n - 1) % n]) {
            ring.remove(i);
        } else {
            let j = (i + 1) % n;
            ring.remove(i.max(j));
            ring.remove(i.min(j));
        }
    }
}

impl Store {
    /// Apply an affine point transform in mesh order. The facade moves by
    /// a delta, including its float32 rounding for older meshes.
    pub(super) fn transform_vertices(
        &mut self,
        source: &[[f64; 3]],
        matrix: [f64; 16],
        collect: bool,
    ) -> (usize, Vec<u128>) {
        let source: HashSet<_> = source.iter().copied().map(supplied_cell).collect();
        let matrix = glam::DMat4::from_cols_array(&matrix);
        let mut count = 0;
        let mut keys = Vec::new();
        for v in self.live_slots(Kind::Vertex) {
            let point = *self.state.vpos.get(v as usize);
            if !source.contains(&cell(point)) {
                continue;
            }
            let p = glam::DVec3::from_array(point);
            let q = matrix * p.extend(1.);
            self.move_vertex(v, (q.truncate() / q.w - p).to_array());
            count += 1;
            if collect {
                keys.push(self.key(Kind::Vertex, v));
            }
        }
        (count, keys)
    }

    /// Move the selected position cells together. A landing needs the weld
    /// and defect checks. Detect it before writing so that path can take over.
    pub fn move_without_join(&mut self, source: &[Point], delta: Point) -> Option<usize> {
        let source: HashSet<_> = source.iter().copied().map(cell).collect();
        let vertices = self.live_slots(Kind::Vertex);
        let mut moving = Vec::new();
        let mut landing = HashSet::new();
        for &v in &vertices {
            let p = *self.state.vpos.get(v as usize);
            // Outside the range of the facade's int64 floor-cell index the
            // candidate scan has its own overflow semantics.
            if p.iter().any(|x| !x.is_finite() || x.abs() >= 1e11) {
                return None;
            }
            if source.contains(&cell(p)) {
                moving.push(v);
                let dest = self.translated_position(p, delta);
                if dest.iter().any(|x| !x.is_finite() || x.abs() >= 1e11) {
                    return None;
                }
                landing.insert(cell(dest));
            }
        }
        if vertices.iter().any(|&v| {
            let position = cell(*self.state.vpos.get(v as usize));
            landing.contains(&position) && !source.contains(&position)
        }) {
            return None;
        }
        for &v in &moving {
            self.move_vertex(v, delta);
        }
        Some(moving.len())
    }

    pub(super) fn command_remove_face(&mut self, face: u32) {
        let mut change = Change::default();
        self.remove_face(face, &mut change);
        self.finish(change);
    }

    /// Outer loop matching ignores winding, repeated corners and holes.
    /// None requests the materialising facade for unusual incidence.
    pub fn delete_command_face(
        &mut self,
        points: &[Point],
        candidates: &[u32],
    ) -> Option<Option<u128>> {
        let target: HashSet<_> = points.iter().copied().map(cell).collect();
        let found = candidates.iter().copied().find(|&f| {
            let corners: HashSet<_> = self
                .outer_ring(f)
                .into_iter()
                .map(|v| cell(*self.state.vpos.get(v as usize)))
                .collect();
            corners == target
        });
        match found {
            None => Some(None),
            Some(f) if self.remove_face_blocked(f) => None,
            Some(f) => {
                let key = self.key(Kind::Face, f);
                self.command_remove_face(f);
                Some(Some(key))
            }
        }
    }

    pub fn delete_command_edges(
        &mut self,
        endpoints: &[[Point; 2]],
        candidates: &[u32],
    ) -> Option<(Vec<u128>, Vec<u128>)> {
        let edges: Vec<_> = endpoints
            .iter()
            .filter_map(|&[a, b]| self.find_edge(self.lookup(a)?, self.lookup(b)?))
            .collect();
        let gone: HashSet<_> = endpoints
            .iter()
            .map(|&[a, b]| segment(cell(a), cell(b)))
            .collect();
        let mut faces = Vec::new();
        for &f in candidates {
            let corners: Vec<_> = self
                .outer_ring(f)
                .into_iter()
                .map(|v| cell(*self.state.vpos.get(v as usize)))
                .collect();
            if corners
                .iter()
                .zip(corners.iter().cycle().skip(1))
                .any(|(&a, &b)| gone.contains(&segment(a, b)))
            {
                if self.remove_face_blocked(f) {
                    return None;
                }
                faces.push(f);
            }
        }
        let face_keys = faces.iter().map(|&f| self.key(Kind::Face, f)).collect();
        let edge_keys = edges.iter().map(|&e| self.key(Kind::Edge, e)).collect();
        for f in faces {
            self.command_remove_face(f);
        }
        for e in edges {
            let mut change = Change::default();
            self.remove_edge(e, &mut change);
            self.finish(change);
        }
        Some((edge_keys, face_keys))
    }
}

impl Store {
    /// Remove spurs after a landing weld. Only their tips must have been
    /// moved. Other folds in a neighbouring wall are preserved.
    pub(super) fn trim_landing_spurs(
        &mut self,
        landed: &HashSet<Cell>,
    ) -> Result<(usize, Vec<(u128, u128)>), super::topo::OpError> {
        let mut copies = Vec::new();
        let mut dirty = 0;
        for face in self.live_slots(Kind::Face) {
            let rings = self.face_rings(face);
            if rings.len() != 1 {
                continue;
            }
            let mut ring = rings[0].clone();
            let at_landing = |v: u32| landed.contains(&cell(*self.state.vpos.get(v as usize)));
            if !ring.iter().any(|&v| at_landing(v)) {
                continue;
            }
            trim_spurs(&mut ring, |a, b| a == b, at_landing);
            if ring.len() == rings[0].len() {
                continue;
            }
            if self.remove_face_blocked(face) {
                return Err(super::topo::OpError::Fallback("spur incidence"));
            }
            let original = self.key(Kind::Face, face);
            self.command_remove_face(face);
            dirty += 1;
            if ring.len() >= 3 {
                let points = ring
                    .iter()
                    .map(|&v| *self.state.vpos.get(v as usize))
                    .collect();
                let mut change = Change::default();
                let new = self.add_face(&[points], &mut change)?;
                self.finish(change);
                copies.push((original, self.key(Kind::Face, new)));
                dirty += 1 + ring.len();
            }
        }
        Ok((dirty, copies))
    }

    /// Return None before any write if the facade has to materialise this mesh.
    pub(super) fn weld_landing(
        &mut self,
        moving: &HashSet<u32>,
    ) -> Result<Option<(bool, usize, Vec<(u128, u128)>)>, super::topo::OpError> {
        let vertices = self.live_slots(Kind::Vertex);
        if vertices.iter().any(|&v| {
            self.state
                .vpos
                .get(v as usize)
                .iter()
                .any(|x| !x.is_finite() || x.abs() >= 1e11)
        }) {
            return Ok(None);
        }
        let landed = moving
            .iter()
            .map(|&v| cell(*self.state.vpos.get(v as usize)))
            .collect::<HashSet<_>>();
        if landed.is_empty()
            || !vertices.into_iter().any(|v| {
                !moving.contains(&v) && landed.contains(&cell(*self.state.vpos.get(v as usize)))
            })
        {
            return Ok(Some((false, 0, Vec::new())));
        }

        let mut work = self.clone();
        let mut change = Change::default();
        let (_, dirty) = work.weld_coincident(&mut change)?;
        let removed = change.removed[2]
            .iter()
            .filter_map(|&key| work.slot_any(Kind::Face, key))
            .collect::<Vec<_>>();
        if work.dangling(&removed) {
            return Ok(None);
        }
        work.finish(change);
        let (trim, copies) = work.trim_landing_spurs(&landed)?;
        *self = work;
        Ok(Some((true, dirty + trim, copies)))
    }
}
