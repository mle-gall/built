// SPDX-License-Identifier: GPL-3.0-or-later
//! Emit the naive shell whose seams the connectivity and plane passes repair.
use super::{
    drawing::vector, drawing_face::Division, edit::Change, geometry::Hypot, keys::Kind,
    push_band::Band, topo::OpError, Store,
};
use std::collections::HashMap;
type Point = [f64; 3];
type Ring = Vec<Point>;

#[derive(Default)]
pub(super) struct Shell {
    pub dirty: usize,
    pub steps: usize,
    pub cap: u128,
    pub removed_faces: Vec<u128>,
    pub removed_edges: Vec<u128>,
    pub source: Option<u128>,
    pub remainders: Vec<u128>,
}

impl Shell {
    fn delete(&mut self, st: &mut Store, ring: &[Point]) -> Result<(), OpError> {
        let candidates = st.live_slots(Kind::Face);
        let gone = st
            .delete_command_face(ring, &candidates)
            .ok_or(OpError::Fallback(
                "push would leave a deleted face on an edge",
            ))?;
        if let Some(f) = gone {
            self.dirty += 1;
            self.removed_faces.push(f);
        }
        self.steps += 1;
        Ok(())
    }
    fn band(
        &mut self,
        st: &mut Store,
        lower: &[Point],
        upper: &[Point],
        rim: bool,
        rails: bool,
        walls: bool,
        cap: bool,
    ) -> Result<(), OpError> {
        let band = Band::new(lower, upper, rim, rails, walls, cap)
            .ok_or(OpError::Fallback("push rings have different lengths"))?;
        let (dirty, steps) = st.apply_push_band(band)?;
        self.dirty += dirty;
        self.steps += steps;
        Ok(())
    }
    fn face(&mut self, st: &mut Store, mut rings: Vec<Ring>, h: Hypot) -> Result<u32, OpError> {
        if rings.len() > 2 {
            if rings[1..].iter().any(|r| r.len() < 3) {
                return Err(OpError::Fallback("short hole in pushed face"));
            }
            let hs: Vec<_> = rings[1..]
                .iter()
                .map(|r| r.iter().copied().map(vector).collect())
                .collect();
            let mut keep = std::iter::once(true).chain(super::fillet::maximal_holes(&hs, h));
            rings.retain(|_| keep.next() == Some(true));
        }
        self.dirty += 1 + rings.iter().map(Vec::len).sum::<usize>();
        let mut ch = Change::default();
        let f = st.add_face(&rings, &mut ch)?;
        st.finish(ch);
        Ok(f)
    }
    fn hole(&mut self, st: &mut Store, f: u32, ring: &[Point]) -> Result<(), OpError> {
        let mut ch = Change::default();
        st.add_hole(f, ring, &mut ch)?;
        st.finish(ch);
        self.dirty += ring.len();
        Ok(())
    }
    fn divide_cap(&mut self, st: &mut Store, cap: u32, h: Hypot) -> Result<(), OpError> {
        let faces = st.live_slots(Kind::Face);
        match st
            .face_division(cap, &faces, &faces, h)
            .ok_or(OpError::Fallback(
                "push cap division could not be represented",
            ))? {
            Division::Holes {
                mother,
                swap,
                enclosed,
            } => {
                if let Some(m) = mother {
                    let ring = if let Some((i, ring)) = swap {
                        let mut ch = Change::default();
                        st.remove_hole(m, &st.face_rings(m)[i + 1], &mut ch);
                        st.finish(ch);
                        ring
                    } else {
                        st.position_rings(cap).remove(0)
                    };
                    self.hole(st, m, &ring)?;
                    self.source = Some(st.key(Kind::Face, m));
                }
                for ring in enclosed {
                    self.hole(st, cap, &ring)?;
                }
            }
            Division::Carve { mother, pieces } => {
                self.source = Some(st.key(Kind::Face, mother));
                st.command_remove_face(mother);
                self.dirty += 1;
                for rings in pieces {
                    let f = self.face(st, rings, h)?;
                    self.remainders.push(st.key(Kind::Face, f));
                }
            }
        }
        Ok(())
    }
    fn prune(&mut self, st: &mut Store, rim: &[Point]) {
        self.removed_edges =
            st.orphan_edges(&st.live_slots(Kind::Edge), &st.live_slots(Kind::Face), rim);
        for &key in &self.removed_edges {
            let mut ch = Change::default();
            st.remove_edge(st.slot(Kind::Edge, key).unwrap(), &mut ch);
            st.finish(ch);
        }
        self.dirty += self.removed_edges.len();
        self.steps += usize::from(!self.removed_edges.is_empty());
    }
}

impl Store {
    pub(super) fn push_shell(
        &mut self,
        face: u32,
        mut lower: Vec<Ring>,
        mut upper: Vec<Ring>,
        delta: Point,
        attached: bool,
        far: Option<(u32, Ring, Option<Ring>)>,
        h: Hypot,
    ) -> Result<Shell, OpError> {
        let mut result = Shell::default();
        if lower.is_empty() || upper.is_empty() {
            return Err(OpError::Fallback("empty push outline"));
        }
        let outer = self.position_rings(face).remove(0);
        if let Some((far, back, rest)) = far {
            let mut rings = self.position_rings(far);
            let old_far = rings[0].clone();
            let punch = rest.is_none();
            if let Some(rest) = rest {
                rings[0] = rest;
            } else {
                rings.push(back.clone());
            }
            result.delete(self, &outer)?;
            result.delete(self, &old_far)?;
            let f = result.face(self, rings, h)?;
            self.set_face_flags(f, 0);
            result.cap = self.key(Kind::Face, f);
            result.steps += 1;
            result.band(self, &lower[0], &back, punch, true, true, false)?;
            for (a, b) in lower[1..].iter().zip(&upper[1..]) {
                result.band(self, a, b, true, true, true, true)?;
            }
            lower[0].extend(back);
            result.prune(self, &lower[0]);
        } else {
            if lower.len() != upper.len()
                || lower.iter().zip(&upper).any(|(a, b)| a.len() != b.len())
            {
                return Err(OpError::Fallback("push rings have different lengths"));
            }
            // Resolve the planner's exact coordinates before edits can change the registry.
            let bits = |p: Point| p.map(|x| if x == 0. { 0 } else { x.to_bits() });
            let mut exact = HashMap::new();
            for (&p, &q) in lower.iter().flatten().zip(upper.iter().flatten()) {
                if let Some(v) = self.lookup(p) {
                    let source = *self.state.vpos.get(v as usize);
                    exact.insert(bits(p), source);
                    exact.insert(bits(q), std::array::from_fn(|i| source[i] + delta[i]));
                }
            }
            for p in lower.iter_mut().chain(&mut upper).flatten() {
                if let Some(&precise) = exact.get(&bits(*p)) {
                    *p = precise;
                }
            }
            if attached {
                result.delete(self, &outer)?;
            }
            result.band(self, &lower[0], &upper[0], true, true, false, false)?;
            let f = result.face(self, upper.clone(), h)?;
            self.set_face_flags(f, 0);
            result.cap = self.key(Kind::Face, f);
            result.steps += 1;
            if !attached {
                result.divide_cap(self, f, h)?;
            }
            for (a, b) in lower[1..].iter().zip(&upper[1..]) {
                result.band(self, a, b, true, true, true, false)?;
            }
            result.band(self, &lower[0], &upper[0], false, false, true, false)?;
            if attached {
                result.prune(self, &lower[0]);
            }
        }
        Ok(result)
    }
}
