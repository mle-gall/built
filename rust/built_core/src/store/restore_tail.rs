// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! A restore keeps the tombstones a newer state made.
//!
//! History relinks elements by key (`relink_edge` / `relink_face` /
//! `relink_vertices`), and its snapshot steps (`SnapshotCompound`,
//! `MeshSnapshotCommand`) restore whole states. Undo is LIFO, but redo
//! crosses those restores: draw a line `a`, draw a line through the
//! snapshot planner `P`, draw a line `c`; undo `c` (tombstone), undo `P`
//! (restore before `P`), redo `P` (restore after `P`), redo `c`: both
//! states predate `c`, so a restore that took the snapshot's slot table
//! as it is forgot `c`'s tombstone and the relink found nothing.
//!
//! So a restore within one lineage keeps every slot the current state
//! has past the snapshot's end, dead, with its data: what a Python mesh
//! does by holding the element objects outside the mesh. Slots are not
//! reused within a state (§15.2), so a slot past the snapshot's end is
//! the same element in both. Live there, it becomes a tombstone (the
//! restore removed it). Cost: O(rows past the snapshot's end), the edits
//! being undone.

use super::cow::CowVec;
use super::keys::{Kind, NONE};
use super::{FaceRings, Inc, State, Store};
use std::sync::Arc;

fn put<T: Clone>(col: &mut CowVec<T>, i: usize, v: T, fill: T) {
    while col.len() < i {
        col.push(fill.clone());
    }
    if col.len() == i {
        col.push(v);
    } else {
        col.set(i, v);
    }
}

impl Store {
    /// Append to `next` (a same-lineage snapshot state about to become
    /// the live one) every slot of the live state past `next`'s end, as a
    /// tombstone.
    pub(super) fn carry_tombstones(&self, next: &mut State) {
        let cur = &self.state;
        if cur.ns != next.ns {
            return;
        }
        // Curved bindings of the carried tombstones come along for relink.
        if let Some(c) = cur.exact.curved.as_deref() {
            let (ne, nf) = (next.slots[Kind::Edge as usize].len() as u32, next.slots[Kind::Face as usize].len() as u32);
            let carried_e: Vec<_> = c.edges.iter().filter(|r| *r.0 >= ne).map(|(k, v)| (*k, v.clone())).collect();
            let carried_f: Vec<_> = c.faces.iter().filter(|r| *r.0 >= nf).map(|(k, v)| (*k, v.clone())).collect();
            if !carried_e.is_empty() || !carried_f.is_empty() {
                let mut merged = next.exact.curved.as_deref().cloned().unwrap_or_default();
                let stale_e: Vec<u32> = merged.edges.keys().filter(|&k| k >= ne).collect();
                let stale_f: Vec<u32> = merged.faces.keys().filter(|&k| k >= nf).collect();
                for k in stale_e {
                    merged.edges.remove(&k);
                }
                for k in stale_f {
                    merged.faces.remove(&k);
                }
                merged.edges.extend(carried_e);
                merged.faces.extend(carried_f);
                next.exact.curved = Some(Arc::new(merged));
            }
        }
        let v = Kind::Vertex as usize;
        for s in next.slots[v].len()..cur.slots[v].len() {
            let g = cur.slots[v].gen(s as u32);
            next.slots[v].state.push(g << 1);
            next.ids[v].put(s as u32, s, cur.ids[v].get(s as u32));
            put(&mut next.vpos, s, *cur.vpos.get(s), [0.0; 3]);
            put(&mut next.vedge, s, NONE, NONE);
        }
        let e = Kind::Edge as usize;
        for s in next.slots[e].len()..cur.slots[e].len() {
            let g = cur.slots[e].gen(s as u32);
            next.slots[e].state.push(g << 1);
            next.ids[e].put(s as u32, s, cur.ids[e].get(s as u32));
            next.exact.inherit(Kind::Edge, s as u32, cur.exact.get(Kind::Edge, s as u32));
            put(&mut next.ends, s, *cur.ends.get(s), [NONE, NONE]);
            put(&mut next.enext, s, [NONE, NONE], [NONE, NONE]);
            put(&mut next.eflags, s, *cur.eflags.get(s), 0);
            put(&mut next.ecurve, s, *cur.ecurve.get(s), -1);
            let l = *cur.elayer.get(s);
            let layer = if l == NONE {
                NONE
            } else {
                let name = &cur.layers[l as usize];
                match next.layers.iter().position(|x| x == name) {
                    Some(i) => i as u32,
                    None => {
                        let layers = Arc::make_mut(&mut next.layers);
                        layers.push(name.clone());
                        (layers.len() - 1) as u32
                    }
                }
            };
            put(&mut next.elayer, s, layer, NONE);
            let inc = *cur.efaces.get(s);
            let inc = if inc.len > 2 {
                next.ef_spill.push(cur.ef_spill.get(inc.a[0] as usize).clone());
                Inc { len: inc.len, a: [(next.ef_spill.len() - 1) as u32, NONE] }
            } else {
                inc
            };
            put(&mut next.efaces, s, inc, Inc::default());
        }
        let f = Kind::Face as usize;
        for s in next.slots[f].len()..cur.slots[f].len() {
            let g = cur.slots[f].gen(s as u32);
            next.slots[f].state.push(g << 1);
            next.ids[f].put(s as u32, s, cur.ids[f].get(s as u32));
            let r = cur.exact.get(Kind::Face, s as u32);
            if let Some(p) = cur.exact.planes.as_ref().and_then(|v| v.get(r as usize)) {
                next.exact
                    .install_plane(s as u32, p.clone())
                    .expect("existing plane fits a store slot");
            } else {
                next.exact.inherit(Kind::Face, s as u32, r);
            }
            let rings = self.face_rings(s as u32);
            let fr = FaceRings {
                corner_start: next.corners.len() as u32,
                ring_start: next.ring_sizes.len() as u32,
                n_rings: rings.len() as u32,
            };
            for r in &rings {
                next.ring_sizes.push(r.len() as u32);
                for &c in r {
                    next.corners.push(c);
                }
            }
            put(&mut next.frings, s, fr, FaceRings::default());
            put(&mut next.fattr, s, *cur.fattr.get(s), 0);
            put(&mut next.fflags, s, *cur.fflags.get(s), 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::edit::Change;
    use super::super::keys::Kind;
    use super::super::{Store, Weld};

    /// Line, snapshot step, line; undo all; redo all. The second redo
    /// restores a state older than the last line, whose relink still finds
    /// its tombstone (and a face's, with its rings).
    #[test]
    fn redo_across_snapshot_restores_relinks_the_newer_elements() {
        let mut st = Store::empty(3, Weld::default());
        let mut ch = Change::default();
        st.add_edge([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], &mut ch).unwrap();
        let before = st.snapshot();
        st.add_edge([1.0, 0.0, 0.0], [1.0, 1.0, 0.0], &mut ch).unwrap();
        let after = st.snapshot();
        let e = st.add_edge([1.0, 1.0, 0.0], [0.0, 1.0, 0.0], &mut ch).unwrap();
        let f = st
            .add_face(&[vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]]], &mut ch)
            .unwrap();
        let (ek, fk) = (st.key(Kind::Edge, e), st.key(Kind::Face, f));
        let rings = st.face_rings(f);
        st.remove_face(f, &mut ch);
        st.remove_edge(e, &mut ch);
        st.restore(&before, None).unwrap();
        st.check().unwrap();
        assert!(st.state.same_as(&before.state), "tombstones are not content");
        st.restore(&after, None).unwrap();
        assert_eq!(st.slot_any(Kind::Edge, ek), Some(e));
        assert_eq!(st.slot_any(Kind::Face, fk), Some(f));
        assert!(st.relink_edge(e, &mut ch).unwrap());
        assert!(st.relink_face(f, &mut ch).unwrap());
        assert_eq!(st.key(Kind::Edge, e), ek);
        assert_eq!(st.face_rings(f), rings);
        st.check().unwrap();
        // New elements after the restore never take a kept slot.
        let n = st.state.slots[1].len();
        let e2 = st.add_edge([5.0, 0.0, 0.0], [6.0, 0.0, 0.0], &mut ch).unwrap();
        assert_eq!(e2 as usize, n);
    }
}
