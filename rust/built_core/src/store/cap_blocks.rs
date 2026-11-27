// SPDX-License-Identifier: GPL-3.0-or-later
//! Edge connectivity independent of list order. A change in a cap can cut
//! a component. Only union edges whose incidence is still current.
use super::{cow::CHUNK, State, Store};
use crate::orient::component_root;
use std::collections::HashMap;

pub(super) struct CapBlocks {
    pub(super) base: State,
    pub(super) faces: super::push_rebuild::FaceBlocks,
    // Each group starts with its length. Singletons add no connectivity.
    groups: Vec<Vec<u32>>,
}

impl CapBlocks {
    pub(super) fn new(st: &Store) -> Self {
        let edges = &st.state.slots[1];
        let mut parent: Vec<u32> = (0..st.state.slots[2].len() as u32).collect();
        let groups = (0..edges.len().div_ceil(CHUNK))
            .map(|c| {
                let mut touched = Vec::new();
                for e in c * CHUNK..((c + 1) * CHUNK).min(edges.len()) {
                    if !edges.alive(e as u32) {
                        continue;
                    }
                    let fs = st.edge_faces(e as u32);
                    if fs.len() > 1 {
                        Self::join(&mut parent, fs);
                        touched.extend_from_slice(fs);
                    }
                }
                touched.sort_unstable();
                touched.dedup();
                let mut by_root = HashMap::<u32, Vec<u32>>::new();
                for &f in &touched {
                    let r = component_root(&mut parent, f);
                    by_root.entry(r).or_default().push(f);
                }
                for f in touched {
                    parent[f as usize] = f;
                }
                let mut packed = Vec::new();
                for faces in by_root.into_values() {
                    packed.push(faces.len() as u32);
                    packed.extend(faces);
                }
                packed
            })
            .collect();
        Self {
            base: st.state.clone(),
            faces: super::push_rebuild::FaceBlocks::new(st),
            groups,
        }
    }

    fn join(parent: &mut [u32], fs: &[u32]) {
        if let Some((&first, rest)) = fs.split_first() {
            let root = component_root(parent, first);
            for &f in rest {
                let other = component_root(parent, f);
                parent[other as usize] = root;
            }
        }
    }

    pub(super) fn labels(&self, store: &Store) -> Vec<u32> {
        let st = &store.state;
        let mut parent: Vec<u32> = (0..st.slots[2].len() as u32).collect();
        let spill = st.ef_spill.n_chunks() == self.base.ef_spill.n_chunks()
            && (0..st.ef_spill.n_chunks()).all(|c| st.ef_spill.same_chunk(&self.base.ef_spill, c));
        for c in 0..st.slots[1].len().div_ceil(CHUNK) {
            if spill
                && st.slots[1].state.same_chunk(&self.base.slots[1].state, c)
                && st.efaces.same_chunk(&self.base.efaces, c)
            {
                let mut groups = self.groups[c].as_slice();
                while let Some((&n, tail)) = groups.split_first() {
                    let (faces, next) = tail.split_at(n as usize);
                    Self::join(&mut parent, faces);
                    groups = next;
                }
            } else {
                for e in c * CHUNK..((c + 1) * CHUNK).min(st.slots[1].len()) {
                    if st.slots[1].alive(e as u32) {
                        Self::join(&mut parent, store.edge_faces(e as u32));
                    }
                }
            }
        }
        for f in st.slots[2].live_slots() {
            parent[f as usize] = component_root(&mut parent, f);
        }
        parent
    }
}
