// SPDX-License-Identifier: GPL-3.0-or-later
//! A world-space BVH over a store's edges, for the edges near the cursor.
//!
//! Select's hover asked the store for the loose edges near the pointer on
//! every move, and the store projected every edge to answer: ~14 ms a move
//! on a 200k-face mesh. The tree is camera-independent, so it is built once
//! per mesh state and a query walks only the branches whose screen box comes
//! within the radius. An edit does not rebuild it: the edges it touched (the
//! store's chunked diff against the state the tree was built from) are left
//! out of the tree's leaves and tested one by one, until they are many.
use super::keys::{split_key, Kind};
use super::{State, Store};
use crate::instances::{box_corner, build_bvh, rect_gap, Node, LEAF};
use crate::pick::Projector;
type FxHashSet<T> = std::collections::HashSet<T, super::cow::FxBuild>;
use std::sync::Arc;

pub struct EdgeIndex {
    revision: u64,
    lineage: u64,
    /// The state the tree was built from, kept to diff later states against.
    base: State,
    /// Edge slots in leaf order, and the tree over them.
    order: Arc<Vec<u32>>,
    nodes: Arc<Vec<Node>>,
    /// Edges changed since `base`: left out of the leaves, tested directly.
    stale: Arc<FxHashSet<u32>>,
    extra: Arc<Vec<u32>>,
}

/// Past this many changed edges the tree is built again.
fn rebuild_past(n: usize) -> usize {
    4096 + n / 16
}

impl EdgeIndex {
    pub fn build(store: &Store) -> Self {
        let st = &store.state;
        let slots = store.live_slots(Kind::Edge);
        let mut lo = Vec::with_capacity(slots.len());
        let mut hi = Vec::with_capacity(slots.len());
        let mut cen = Vec::with_capacity(slots.len());
        for &e in &slots {
            let [a, b] = st.ends.get(e as usize).map(|v| *st.vpos.get(v as usize));
            let l: [f64; 3] = std::array::from_fn(|k| a[k].min(b[k]));
            let h: [f64; 3] = std::array::from_fn(|k| a[k].max(b[k]));
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
        EdgeIndex {
            revision: store.revision,
            lineage: store.lineage,
            base: st.clone(),
            order: Arc::new(order),
            nodes: Arc::new(nodes),
            stale: Default::default(),
            extra: Default::default(),
        }
    }

    /// This index brought up to `store`: the same one when nothing changed,
    /// the same tree with the changed edges set aside after a small edit,
    /// a new tree otherwise.
    pub fn refresh(self: &Arc<Self>, store: &Store) -> Arc<Self> {
        if store.revision == self.revision && store.lineage == self.lineage {
            return self.clone();
        }
        if store.lineage != self.lineage || store.state.ns != self.base.ns {
            return Arc::new(Self::build(store));
        }
        let change = store.state.changes_since(&self.base);
        let k = Kind::Edge as usize;
        let mut touched: FxHashSet<u32> = FxHashSet::default();
        let slot = |key: u128| split_key(key).map(|(_, _, _, s)| s);
        for &key in change.created[k].iter().chain(&change.removed[k]).chain(&change.modified[k]) {
            touched.extend(slot(key));
        }
        let vertices = &store.state.slots[Kind::Vertex as usize];
        for &key in change.moved.iter().chain(&change.modified[Kind::Vertex as usize]) {
            if let Some(v) = slot(key).filter(|&v| vertices.alive(v)) {
                touched.extend(store.edges_at(v));
            }
        }
        if touched.len() > rebuild_past(self.order.len()) {
            return Arc::new(Self::build(store));
        }
        let edges = &store.state.slots[k];
        let mut extra: Vec<u32> = touched.iter().copied().filter(|&e| edges.alive(e)).collect();
        extra.sort_unstable();
        Arc::new(EdgeIndex {
            revision: store.revision,
            lineage: store.lineage,
            base: self.base.clone(),
            order: self.order.clone(),
            nodes: self.nodes.clone(),
            stale: Arc::new(touched),
            extra: Arc::new(extra),
        })
    }

    /// `(edge slot, pixels, t)` of the live edges within `radius` px of
    /// `(x, y)`, nearest first (ties by slot).
    pub fn near(&self, store: &Store, camera: &Projector, mvp: &[f64; 16], w: f64, h: f64,
                x: f64, y: f64, radius: f64) -> Vec<(u32, f64, f64)> {
        let st = &store.state;
        let edges = &st.slots[Kind::Edge as usize];
        let mut out = Vec::new();
        let mut test = |e: u32| {
            let [a, b] = st.ends.get(e as usize).map(|v| *st.vpos.get(v as usize));
            let (pa, pb) = camera.segment(a, b);
            if let Some(&(_, d, t)) = crate::pick::nearest_projected(std::iter::once((pa, pb)), x, y, radius).first() {
                out.push((e, d, t));
            }
        };
        let leaf = |slots: &[u32], test: &mut dyn FnMut(u32)| {
            for &e in slots {
                if edges.alive(e) && !self.stale.contains(&e) {
                    test(e);
                }
            }
        };
        if self.nodes.is_empty() {
            leaf(&self.order, &mut test);
        } else {
            let mut stack = vec![0u32];
            while let Some(ni) = stack.pop() {
                let n = &self.nodes[ni as usize];
                if rect_gap(|i| box_corner(n.lo, n.hi, i), mvp, w, h, x, y) >= radius {
                    continue;
                }
                if n.leaf {
                    leaf(&self.order[n.start as usize..n.end as usize], &mut test);
                } else {
                    stack.push(n.left);
                    stack.push(n.left + 1);
                }
            }
        }
        for &e in self.extra.iter() {
            test(e);
        }
        out.sort_by(|p, q| p.1.total_cmp(&q.1).then(p.0.cmp(&q.0)));
        out
    }
}
