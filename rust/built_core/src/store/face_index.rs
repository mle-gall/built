// SPDX-License-Identifier: GPL-3.0-or-later
//! A BVH over a store's faces' outer-loop boxes, for the box searches of
//! the drawing commands (which faces a drawn loop may replace or divide).
//!
//! A line across a 200k-face mesh runs a dozen steps (split the edges it
//! crosses, divide each face it cuts, ...), and each step searched every
//! face's box for its candidates: a drawing scan (`MeshScan`) made of the
//! whole mesh per step, ~7 ms each. This tree is built once per mesh
//! state and, like `EdgeIndex`, not rebuilt after an edit: the faces it
//! changed (the store's chunked diff from one refresh to the next, gathered
//! since the tree was built) are left out of the leaves and tested one by
//! one, until they are many. The answers are the scan's, face for face and in list order: the
//! same boxes and planes (`scans::face_box`, `scans::face_plane`) from the
//! same positions.
use super::keys::{split_key, Kind};
use super::{State, Store};
use crate::instances::{build_bvh, Node, LEAF};
use crate::scans::{face_box, face_plane, Box6};
type FxHashSet<T> = std::collections::HashSet<T, super::cow::FxBuild>;
use std::sync::Arc;

pub struct FaceIndex {
    revision: u64,
    lineage: u64,
    /// The last state the tree was brought to: the next refresh diffs
    /// against it (one step's change), adding to what changed before.
    last: State,
    /// Face slots in leaf order, and the tree over their boxes.
    order: Arc<Vec<u32>>,
    nodes: Arc<Vec<Node>>,
    /// Per face slot at the build: outer box, plane reach and vagueness.
    boxes: Arc<Vec<Box6>>,
    planes: Arc<Vec<(f64, bool)>>,
    /// The largest reach of a face at the build, and its vague faces.
    reach: f64,
    vague: Arc<Vec<u32>>,
    /// Faces changed since the build: left out of the leaves, tested directly.
    stale: Arc<FxHashSet<u32>>,
    extra: Arc<Vec<u32>>,
}

/// Past this many changed faces the tree is built again.
fn rebuild_past(n: usize) -> usize {
    4096 + n / 16
}

/// The outer loop's corner positions of face `f`.
fn outer(st: &State, f: u32) -> Vec<[f64; 3]> {
    let r = *st.frings.get(f as usize);
    if r.n_rings == 0 {
        return Vec::new();
    }
    let size = *st.ring_sizes.get(r.ring_start as usize) as usize;
    (0..size).map(|j| *st.vpos.get(*st.corners.get(r.corner_start as usize + j) as usize)).collect()
}


/// How a query tests a face's box and plane.
#[derive(Clone, Copy)]
pub enum Query {
    /// The box lies within the window.
    Within,
    /// The box meets the window.
    Meeting,
    /// The box grown by `pad` + the face's reach holds the window, or the
    /// face's plane is vague (`MeshScan.faces_holding`).
    Holding(f64),
}

impl FaceIndex {
    pub fn build(store: &Store) -> Self {
        let st = &store.state;
        let slots = store.live_slots(Kind::Face);
        let n = st.slots[Kind::Face as usize].len();
        let mut boxes = vec![[0.0; 6]; n];
        let mut planes = vec![(0.0, false); n];
        let (mut lo, mut hi, mut cen) = (Vec::with_capacity(slots.len()), Vec::with_capacity(slots.len()), Vec::with_capacity(slots.len()));
        let (mut reach, mut vague) = (0.0f64, Vec::new());
        for &f in &slots {
            let pts = outer(st, f);
            let b = face_box(&pts);
            let p = face_plane(&pts);
            boxes[f as usize] = b;
            planes[f as usize] = (p.reach, p.vague);
            reach = reach.max(p.reach);
            if p.vague {
                vague.push(f);
            }
            let (l, h) = ([b[0], b[1], b[2]], [b[3], b[4], b[5]]);
            cen.push(std::array::from_fn(|k| (l[k] + h[k]) * 0.5));
            lo.push(l);
            hi.push(h);
        }
        let (order, nodes) = if slots.len() > 4 * LEAF {
            let (order, nodes) = build_bvh(&lo, &hi, &cen);
            (order.into_iter().map(|i| slots[i as usize]).collect(), nodes)
        } else {
            (slots, Vec::new())
        };
        FaceIndex {
            revision: store.revision,
            lineage: store.lineage,
            last: st.clone(),
            order: Arc::new(order),
            nodes: Arc::new(nodes),
            boxes: Arc::new(boxes),
            planes: Arc::new(planes),
            reach,
            vague: Arc::new(vague),
            stale: Default::default(),
            extra: Default::default(),
        }
    }

    /// This index brought up to `store`: the same one when nothing changed,
    /// the same tree with the changed faces set aside after a small edit,
    /// a new tree otherwise.
    pub fn refresh(self: &Arc<Self>, store: &Store) -> Arc<Self> {
        if store.revision == self.revision && store.lineage == self.lineage {
            return self.clone();
        }
        if store.lineage != self.lineage || store.state.ns != self.last.ns {
            return Arc::new(Self::build(store));
        }
        let st = &store.state;
        // Changed since the last refresh, added to what had changed before.
        let change = st.changes_since(&self.last);
        let k = Kind::Face as usize;
        let mut touched: FxHashSet<u32> = (*self.stale).clone();
        let slot = |key: u128| split_key(key).map(|(_, _, _, s)| s);
        for &key in change.created[k].iter().chain(&change.removed[k]).chain(&change.modified[k]) {
            touched.extend(slot(key));
        }
        let vertices = &st.slots[Kind::Vertex as usize];
        for &key in &change.moved {
            if let Some(v) = slot(key).filter(|&v| vertices.alive(v)) {
                for e in st.edges_at(v) {
                    touched.extend(st.edge_faces(e).iter().copied());
                }
            }
        }
        if touched.len() > rebuild_past(self.order.len()) {
            return Arc::new(Self::build(store));
        }
        let faces = &st.slots[k];
        let mut extra: Vec<u32> = touched.iter().copied().filter(|&f| faces.alive(f)).collect();
        extra.sort_unstable();
        Arc::new(FaceIndex {
            revision: store.revision,
            lineage: store.lineage,
            last: st.clone(),
            order: self.order.clone(),
            nodes: self.nodes.clone(),
            boxes: self.boxes.clone(),
            planes: self.planes.clone(),
            reach: self.reach,
            vague: self.vague.clone(),
            stale: Arc::new(touched),
            extra: Arc::new(extra),
        })
    }

    /// `MeshScan::drawn_face_candidates` of the face `slot` of `store` (the
    /// state this index was refreshed to): the faces that may hold it and
    /// those it may hold, in list order. None if `slot` is not live.
    pub fn drawn_candidates(&self, store: &Store, slot: u32) -> Option<(Vec<u32>, Vec<u32>)> {
        let st = &store.state;
        if !st.slots[Kind::Face as usize].alive(slot) {
            return None;
        }
        let pts = outer(st, slot);
        let all = || st.order[Kind::Face as usize].live(&st.slots[Kind::Face as usize]);
        if pts.is_empty() {
            return Some((all(), all()));
        }
        let b = face_box(&pts);
        let pad = 0.002 + 1e-6 * b.into_iter().map(f64::abs).fold(0.0, f64::max);
        let plane = face_plane(&pts);
        let holding = self.faces(store, b, Query::Holding(pad));
        let inside = if pts.len() < 3 || plane.vague {
            all()
        } else {
            let g = pad + plane.reach;
            self.faces(store, [b[0] - g, b[1] - g, b[2] - g, b[3] + g, b[4] + g, b[5] + g], Query::Within)
        };
        Some((holding, inside))
    }

    /// The live faces of `store` (the state this index was refreshed to)
    /// whose outer box answers `query` for the window `w`, in list order.
    pub fn faces(&self, store: &Store, w: Box6, query: Query) -> Vec<u32> {
        let st = &store.state;
        let faces = &st.slots[Kind::Face as usize];
        let test = |b: &Box6, reach: f64, vague: bool| -> bool {
            match query {
                Query::Within => (0..3).all(|k| b[k] >= w[k] && b[k + 3] <= w[k + 3]),
                Query::Meeting => b[3] >= w[0] && b[0] <= w[3] && b[4] >= w[1] && b[1] <= w[4] && b[5] >= w[2] && b[2] <= w[5],
                Query::Holding(pad) => {
                    let g = pad + reach;
                    vague || (0..3).all(|k| b[k] - g <= w[k] && b[k + 3] + g >= w[k + 3])
                }
            }
        };
        // A branch can hold an answer only if its box, grown as far as
        // any base face's own test grows it, meets the window.
        let grow = match query {
            Query::Holding(pad) => pad + self.reach,
            _ => 0.0,
        };
        let may = |n: &Node| (0..3).all(|k| n.hi[k] + grow >= w[k] && n.lo[k] - grow <= w[k + 3]);
        let mut out = Vec::new();
        let base_face = |f: u32, out: &mut Vec<u32>| {
            if faces.alive(f) && !self.stale.contains(&f) {
                let (reach, vague) = self.planes[f as usize];
                if test(&self.boxes[f as usize], reach, vague) {
                    out.push(f);
                }
            }
        };
        if self.nodes.is_empty() {
            for &f in self.order.iter() {
                base_face(f, &mut out);
            }
        } else {
            let mut stack = vec![0u32];
            while let Some(ni) = stack.pop() {
                let n = &self.nodes[ni as usize];
                if !may(n) {
                    continue;
                }
                if n.leaf {
                    for &f in &self.order[n.start as usize..n.end as usize] {
                        base_face(f, &mut out);
                    }
                } else {
                    stack.push(n.left);
                    stack.push(n.left + 1);
                }
            }
            if matches!(query, Query::Holding(_)) {
                // Vague faces hold any window, wherever their box lies.
                for &f in self.vague.iter() {
                    base_face(f, &mut out);
                }
            }
        }
        for &f in self.extra.iter() {
            let pts = outer(st, f);
            let p = face_plane(&pts);
            if test(&face_box(&pts), p.reach, p.vague) {
                out.push(f);
            }
        }
        out.sort_unstable_by_key(|&f| (st.list_rank(Kind::Face, f), f));
        out.dedup();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scans::MeshScan;
    use crate::store::edit::Change;
    use crate::store::Weld;

    fn quad(st: &mut Store, x: f64, y: f64, z: f64) -> u32 {
        let ring = vec![[x, y, z], [x + 1.0, y, z], [x + 1.0, y + 1.0, z], [x, y + 1.0, z]];
        st.add_face(&[ring], &mut Change::default()).unwrap()
    }

    /// The scan's answer for a window, as face slots in list order.
    fn scanned(st: &Store, w: Box6, query: Query) -> Vec<u32> {
        let mut scan = MeshScan::from_state(st.state.clone(), 0);
        let rows = Python::attach(|py| match query {
            Query::Within => scan.faces_within_rows(py, w),
            Query::Meeting => scan.faces_meeting_rows(py, w),
            Query::Holding(pad) => scan.faces_holding_rows(py, w, pad),
        });
        let live = st.state.order[2].live(&st.state.slots[2]);
        rows.into_iter().map(|r| live[r as usize]).collect()
    }

    use pyo3::Python;

    #[test]
    fn the_box_tree_answers_as_a_scan() {
        Python::initialize();
        let mut st = Store::empty(9, Weld::default());
        for i in 0..20 {
            for j in 0..20 {
                quad(&mut st, i as f64, j as f64, 0.0);
            }
        }
        // A sliver (vague plane) far away, and a tilted face.
        st.add_face(&[vec![[90.0, 0.0, 0.0], [91.0, 0.0, 0.0], [92.0, 0.0, 1e-12]]], &mut Change::default()).unwrap();
        st.add_face(&[vec![[50.0, 50.0, 0.0], [51.0, 50.0, 0.0], [51.0, 51.0, 3.0], [50.0, 51.0, 3.0]]], &mut Change::default()).unwrap();
        let mut index = Arc::new(FaceIndex::build(&st));
        let windows: Vec<Box6> = (0..30)
            .map(|k| {
                let (x, y) = ((k * 7 % 23) as f64 - 1.5, (k * 11 % 19) as f64 - 0.5);
                [x, y, -0.5, x + (k % 5) as f64 + 0.25, y + (k % 3) as f64 + 0.25, 0.5]
            })
            .chain([[49.9, 49.9, -1.0, 51.1, 51.1, 4.0], [10.2, 10.2, -0.1, 10.4, 10.4, 0.1]])
            .collect();
        // The facade's ops move the revision; these raw ones do not.
        let check = |st: &mut Store, index: &mut Arc<FaceIndex>| {
            st.revision += 1;
            let st = &*st;
            *index = index.refresh(st);
            for &w in &windows {
                for query in [Query::Within, Query::Meeting, Query::Holding(0.002), Query::Holding(0.5)] {
                    let (a, b) = (index.faces(st, w, query), scanned(st, w, query));
                    if a != b {
                        let q = match query { Query::Within => "within".to_string(), Query::Meeting => "meeting".into(), Query::Holding(p) => format!("holding {p}") };
                        let f = *b.iter().chain(&a).find(|f| !a.contains(f) || !b.contains(f)).unwrap();
                        let pts = outer(&st.state, f);
                        panic!("{q} w={w:?} index={a:?} scan={b:?} face {f}: {pts:?} plane {:?} nodes {}", face_plane(&pts), index.nodes.len());
                    }
                }
            }
        };
        check(&mut st, &mut index);
        let e = st.find_edge(0, 1).unwrap();
        st.split_edge(e, [0.5, 0.0, 0.0], &mut Change::default()).unwrap();
        check(&mut st, &mut index);
        let s0 = st.snapshot();
        st.remove_face(3, &mut Change::default());
        quad(&mut st, 10.25, 10.25, 0.0);
        check(&mut st, &mut index);
        st.move_vertex(30, [0.0, 0.0, 0.5]);
        check(&mut st, &mut index);
        // A face drawn over others: the candidates its division reads.
        let drawn = st.add_face(&[vec![[2.5, 2.5, 0.0], [4.5, 2.5, 0.0], [4.5, 4.5, 0.0]]], &mut Change::default()).unwrap();
        st.revision += 1;
        index = index.refresh(&st);
        let mut scan = MeshScan::from_state(st.state.clone(), 0);
        let by_scan = Python::attach(|py| scan.drawn_face_candidates(py, drawn).unwrap());
        assert_eq!(index.drawn_candidates(&st, drawn).unwrap(), by_scan);
        st.restore(&s0, None).unwrap();
        check(&mut st, &mut index);
    }
}
