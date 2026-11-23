// SPDX-License-Identifier: GPL-3.0-or-later
//! Ordered geometry between a push/pull outline and its translated ring.
use super::{edit::Change, topo::OpError, Store};

type Point = [f64; 3];

pub(super) struct Band {
    pub edges: Vec<[Point; 2]>,
    pub faces: Vec<Vec<Point>>,
}

impl Band {
    pub fn new(
        base: &[Point],
        top: &[Point],
        rim: bool,
        rails: bool,
        walls: bool,
        cap: bool,
    ) -> Option<Self> {
        if base.len() != top.len() {
            return None;
        }
        let mut edges = Vec::new();
        let mut faces = Vec::new();
        let sides = |n: usize| (0..n).map(move |i| (i, (i + 1) % n));
        if rim {
            edges.extend(sides(top.len()).map(|(i, j)| [top[i], top[j]]));
        }
        if rails {
            edges.extend(base.iter().zip(top).map(|(&a, &b)| [a, b]));
        }
        if walls {
            faces.extend(sides(base.len()).map(|(i, j)| vec![base[i], base[j], top[j], top[i]]));
        }
        if cap {
            faces.push(top.to_vec());
        }
        Some(Self { edges, faces })
    }
}
impl Store {
    /// Reused rim edges cost no mutation. Face creation marks each corner
    /// and clears the interior bit, matching the individual drawing steps.
    pub(super) fn apply_push_band(&mut self, band: Band) -> Result<(usize, usize), OpError> {
        let steps = band.edges.len() + band.faces.len();
        let mut bumps = 0;
        for [a, b] in band.edges {
            if self
                .lookup(a)
                .zip(self.lookup(b))
                .and_then(|(v, w)| self.find_edge(v, w))
                .is_some()
            {
                continue;
            }
            let mut change = Change::default();
            self.add_edge(a, b, &mut change)?;
            self.finish(change);
            bumps += 1;
        }
        for ring in band.faces {
            let mut change = Change::default();
            bumps += 1 + ring.len();
            let f = self.add_face(&[ring], &mut change)?;
            self.finish(change);
            self.set_face_flags(f, 0);
        }
        Ok((bumps, steps))
    }
}
