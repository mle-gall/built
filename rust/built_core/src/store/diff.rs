// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! The change between two states of one store, as a change record
//! (`edit::Change`): what a History step did to a mesh, by key, whatever
//! made it — ops, field writes (`set_edge_flags`, …), a restore (undo,
//! redo), or several of them in a row. The core protocol's change feed
//! (`core/protocol/feed.py`, `docs/core-protocol.md`) publishes it.
//!
//! Net, not a log: an element made and removed again inside the step is
//! in neither list; one removed and relinked under its own key is
//! modified only if its data differ. Undo gives the inverse of its step
//! for free (the states swap roles).
//!
//! Cost: chunks both states still share are skipped unread, so a step
//! costs O(chunks it wrote), like `State::same_as`.
//!
//! What each list means:
//! - `created` / `removed`: keys live in one state and not in the other
//!   (a tombstone relinked counts as created, as the ops report it);
//! - `moved`: vertices live in both whose position differs;
//! - `modified[vertex]`: vertices live in both whose edge set differs
//!   (an edge on it made, removed, or re-ended);
//! - `modified[edge]`: edges live in both whose ends, flags, curve, tag or
//!   face list (with multiplicity, in order) differ;
//! - `modified[face]`: faces live in both whose rings (corners, in
//!   order), attribute-table index or flags differ.
//!
//! Lists are in slot order. Two states of different namespaces (a
//! foreign restore adopted under fresh keys) share no key: everything in
//! `before` is removed and everything in `self` created.

use super::cow::{CowVec, CHUNK};
use super::edit::Change;
use super::keys::{full_key, local_key, Kind, Slots, NONE};
use super::State;

impl State {
    /// What changed from `before` to `self` (see the module docs).
    pub fn changes_since(&self, before: &State) -> Change {
        let (a, b) = (self, before);
        let mut ch = Change::default();
        if a.ns != b.ns {
            for k in Kind::ALL {
                let i = k as usize;
                ch.removed[i] = b.slots[i].live_slots().map(|s| b.key_of(k, s)).collect();
                ch.created[i] = a.slots[i].live_slots().map(|s| a.key_of(k, s)).collect();
            }
            return ch;
        }
        // Vertex slots whose edge set may have changed: the ends of every
        // edge whose liveness or ends differ.
        let mut touched: Vec<u32> = Vec::new();
        let spill_shared = same_col(&a.ef_spill, &b.ef_spill);
        let layers_shared = std::sync::Arc::ptr_eq(&a.layers, &b.layers) || a.layers == b.layers;
        diff_kind(a, b, Kind::Edge, &mut ch, |c| {
            spill_shared
                && a.ends.same_chunk(&b.ends, c)
                && a.eflags.same_chunk(&b.eflags, c)
                && a.ecurve.same_chunk(&b.ecurve, c)
                && a.elayer.same_chunk(&b.elayer, c)
                && a.efaces.same_chunk(&b.efaces, c)
        }, |s, live_b, live_a, same_key| {
            let (eb, ea) = (live_b.then(|| *b.ends.get(s as usize)), live_a.then(|| *a.ends.get(s as usize)));
            if !same_key {
                touched.extend(eb.into_iter().flatten());
                touched.extend(ea.into_iter().flatten());
            } else if let (Some(old), Some(new)) = (eb, ea) {
                // A split keeps the first half's edge key. Its shared end
                // still has exactly the same incident edge.
                touched.extend(old.into_iter().filter(|v| !new.contains(v)));
                touched.extend(new.into_iter().filter(|v| !old.contains(v)));
            }
            if !same_key {
                return false;
            }
            let i = s as usize;
            eb != ea
                || a.eflags.get(i) != b.eflags.get(i)
                || a.ecurve.get(i) != b.ecurve.get(i)
                || if layers_shared {
                    a.elayer.get(i) != b.elayer.get(i)
                } else {
                    layer_name(a, *a.elayer.get(i)) != layer_name(b, *b.elayer.get(i))
                }
                || edge_faces(a, s) != edge_faces(b, s)
        });
        let pools_shared = same_col(&a.corners, &b.corners) && same_col(&a.ring_sizes, &b.ring_sizes);
        diff_kind(a, b, Kind::Face, &mut ch, |c| {
            pools_shared
                && a.frings.same_chunk(&b.frings, c)
                && a.fattr.same_chunk(&b.fattr, c)
                && a.fflags.same_chunk(&b.fflags, c)
        }, |s, _, _, same_key| {
            let i = s as usize;
            same_key
                && (a.fattr.get(i) != b.fattr.get(i)
                    || a.fflags.get(i) != b.fflags.get(i)
                    || !same_rings(a, b, s))
        });
        for kind in [Kind::Edge, Kind::Face] {
            let k = kind as usize;
            if a.exact.shared(&b.exact, kind) {
                continue;
            }
            let modified = &mut ch.modified[k];
            for slot in a.exact.changed_slots(&b.exact, kind) {
                if a.slots[k].alive(slot) && b.slots[k].alive(slot) && a.slots[k].gen(slot) == b.slots[k].gen(slot) {
                    modified.push(a.key_of(kind, slot));
                }
            }
            modified.sort_unstable_by_key(|key| (*key as u32) & (super::keys::MAX_SLOTS - 1));
            modified.dedup();
        }
        let mut moved = Vec::new();
        diff_kind(a, b, Kind::Vertex, &mut ch, |c| a.vpos.same_chunk(&b.vpos, c), |s, _, _, same_key| {
            if same_key && a.vpos.get(s as usize) != b.vpos.get(s as usize) {
                moved.push(a.key_of(Kind::Vertex, s));
            }
            false
        });
        ch.moved = moved;
        touched.sort_unstable();
        touched.dedup();
        let (va, vb) = (&a.slots[0], &b.slots[0]);
        ch.modified[0] = touched
            .into_iter()
            .filter(|&v| v != NONE && va.alive(v) && vb.alive(v) && va.gen(v) == vb.gen(v))
            .map(|v| a.key_of(Kind::Vertex, v))
            .collect();
        ch
    }

    /// The key of slot `s` of `kind` under its generation in this state.
    fn key_of(&self, kind: Kind, s: u32) -> u128 {
        full_key(self.ns, local_key(kind, self.slots[kind as usize].gen(s), s))
    }
}

/// Created / removed / (by `changed`) modified keys of one kind, every
/// chunk whose slot table and data (`data_shared(chunk)`) both states
/// still share skipped. `changed(slot, live before, live after, same
/// key)` is called for every slot of an unshared chunk live in either.
fn diff_kind(
    a: &State,
    b: &State,
    kind: Kind,
    ch: &mut Change,
    data_shared: impl Fn(usize) -> bool,
    mut changed: impl FnMut(u32, bool, bool, bool) -> bool,
) {
    let k = kind as usize;
    let (sa, sb): (&Slots, &Slots) = (&a.slots[k], &b.slots[k]);
    let n = sa.len().max(sb.len());
    for c in 0..n.div_ceil(CHUNK) {
        if sa.state.same_chunk(&sb.state, c) && data_shared(c) {
            continue;
        }
        let lo = c * CHUNK;
        for s in lo as u32..(lo + CHUNK).min(n) as u32 {
            let (lb, la) = (sb.alive(s), sa.alive(s));
            if !lb && !la {
                continue;
            }
            let same = lb && la && sb.gen(s) == sa.gen(s);
            if !same {
                if lb {
                    ch.removed[k].push(b.key_of(kind, s));
                }
                if la {
                    ch.created[k].push(a.key_of(kind, s));
                }
            }
            if changed(s, lb, la, same) {
                ch.modified[k].push(a.key_of(kind, s));
            }
        }
    }
}

fn same_col<T: Clone>(a: &CowVec<T>, b: &CowVec<T>) -> bool {
    a.len() == b.len() && (0..a.n_chunks()).all(|c| a.same_chunk(b, c))
}

fn layer_name(st: &State, l: u32) -> Option<&str> {
    if l == NONE { None } else { st.layers.get(l as usize).map(|s| s.as_str()) }
}

fn edge_faces(st: &State, e: u32) -> &[u32] {
    let inc = st.efaces.get(e as usize);
    if inc.len <= 2 {
        &inc.a[..inc.len as usize]
    } else {
        st.ef_spill.get(inc.a[0] as usize)
    }
}

/// Whether chunks `start..start + n` of both columns are the same
/// allocations (then the entries are equal unread).
fn range_shared<T: Clone>(x: &CowVec<T>, y: &CowVec<T>, start: usize, n: usize) -> bool {
    n == 0 || (start / CHUNK..=(start + n - 1) / CHUNK).all(|c| x.same_chunk(y, c))
}

/// `rings(a, f) == rings(b, f)` without building either: unread where the
/// face points at the same pool range in chunks both states share (an
/// edit appends to the pool, so most of it is still shared), compared
/// entry by entry otherwise.
fn same_rings(a: &State, b: &State, f: u32) -> bool {
    let (ra, rb) = (*a.frings.get(f as usize), *b.frings.get(f as usize));
    if ra.n_rings != rb.n_rings {
        return false;
    }
    let n = ra.n_rings as usize;
    if ra.ring_start == rb.ring_start
        && ra.corner_start == rb.corner_start
        && range_shared(&a.ring_sizes, &b.ring_sizes, ra.ring_start as usize, n)
    {
        let total: usize = (0..n).map(|k| *a.ring_sizes.get(ra.ring_start as usize + k) as usize).sum();
        if range_shared(&a.corners, &b.corners, ra.corner_start as usize, total) {
            return true;
        }
    }
    let (mut ca, mut cb) = (ra.corner_start as usize, rb.corner_start as usize);
    for k in 0..n {
        let size = *a.ring_sizes.get(ra.ring_start as usize + k);
        if size != *b.ring_sizes.get(rb.ring_start as usize + k) {
            return false;
        }
        let size = size as usize;
        if (0..size).any(|j| a.corners.get(ca + j) != b.corners.get(cb + j)) {
            return false;
        }
        ca += size;
        cb += size;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::super::{Store, Weld};
    use super::*;

    fn sorted(mut v: Vec<u128>) -> Vec<u128> {
        v.sort_unstable();
        v
    }

    fn quad(st: &mut Store, x: f64) -> u32 {
        let ring = vec![[x, 0.0, 0.0], [x + 1.0, 0.0, 0.0], [x + 1.0, 1.0, 0.0], [x, 1.0, 0.0]];
        st.add_face(&[ring], &mut Change::default()).unwrap()
    }

    #[test]
    fn a_step_reports_what_its_ops_did_and_undo_the_inverse() {
        let mut st = Store::empty(9, Weld::default());
        quad(&mut st, 0.0);
        let s0 = st.snapshot();
        // Nothing done: nothing reported.
        assert_eq!(st.state.changes_since(&s0.state), Change::default());
        let mut op = Change::default();
        let f = quad(&mut st, 5.0);
        op.created[2].push(st.key(Kind::Face, f));
        let mv = st.move_vertex(0, [0.0, 0.0, 1.0]);
        let s1 = st.snapshot();
        let ch = st.state.changes_since(&s0.state);
        assert_eq!(ch.created[2], op.created[2]);
        assert_eq!((ch.created[0].len(), ch.created[1].len()), (4, 4));
        assert_eq!(ch.moved, mv.moved);
        assert!(ch.removed.iter().all(|r| r.is_empty()));
        // Undo (a restore) is the inverse.
        st.restore(&s0, None).unwrap();
        let back = st.state.changes_since(&s1.state);
        assert_eq!(back.removed, ch.created);
        assert_eq!(back.created, ch.removed);
        assert_eq!(back.moved, ch.moved);
    }

    #[test]
    fn removals_and_field_writes_are_modifications_of_the_survivors() {
        let mut st = Store::empty(9, Weld::default());
        let f = quad(&mut st, 0.0);
        let s0 = st.snapshot();
        let e = st.find_edge(0, 1).unwrap();
        let ek = st.key(Kind::Edge, e);
        let fk = st.key(Kind::Face, f);
        st.remove_face(f, &mut Change::default());
        let ch = st.state.changes_since(&s0.state);
        assert_eq!(ch.removed[2], vec![fk]);
        assert_eq!(sorted(ch.modified[1].clone()).len(), 4); // each lost the face
        assert!(ch.modified[1].contains(&ek));
        assert!(ch.modified[0].is_empty()); // no edge set changed
        let s1 = st.snapshot();
        st.remove_edge(e, &mut Change::default());
        let ch = st.state.changes_since(&s1.state);
        assert_eq!(ch.removed[1], vec![ek]);
        let ends = sorted(vec![st.key(Kind::Vertex, 0), st.key(Kind::Vertex, 1)]);
        assert_eq!(sorted(ch.modified[0].clone()), ends);
        // A relink under its own key: created again, as the op reports.
        let s2 = st.snapshot();
        st.relink_edge(e, &mut Change::default()).unwrap();
        assert_eq!(st.state.changes_since(&s2.state).created[1], vec![ek]);
        // A flag write is a modification of that edge only.
        let s3 = st.snapshot();
        *st.state.eflags.get_mut(e as usize) |= super::super::SOFT;
        let ch = st.state.changes_since(&s3.state);
        assert_eq!(ch.modified[1], vec![ek]);
        assert!(ch.created.iter().chain(ch.removed.iter()).all(|r| r.is_empty()));
    }

    #[test]
    fn another_namespace_replaces_everything() {
        let mut st = Store::empty(9, Weld::default());
        quad(&mut st, 0.0);
        let s0 = st.snapshot();
        let other = st.copy_as(10);
        let ch = other.state.changes_since(&s0.state);
        assert_eq!(ch.removed[2].len(), 1);
        assert_eq!(ch.created[2].len(), 1);
        assert_ne!(ch.removed[2], ch.created[2]);
    }

    #[test]
    fn untouched_chunks_are_not_read() {
        // A big store and a one-vertex move: the diff touches one chunk
        // per column (checked by timing nothing, by the result's size).
        let mut st = Store::empty(9, Weld::default());
        for i in 0..3000 {
            quad(&mut st, 2.0 * i as f64);
        }
        let s0 = st.snapshot();
        let ch0 = st.move_vertex(7, [0.0, 0.0, 0.5]);
        let ch = st.state.changes_since(&s0.state);
        assert_eq!(ch.moved, ch0.moved);
        assert!(ch.modified.iter().all(|m| m.is_empty()));
    }

    #[test]
    fn the_chunked_diff_is_the_element_by_element_one() {
        // Draws, a removal, a relink, a move and a restore; the chunked
        // diff and the full walk name the same keys.
        let mut st = Store::empty(9, Weld::default());
        for i in 0..40 {
            quad(&mut st, 0.5 * i as f64); // overlapping: shared edges, welds
        }
        let s0 = st.snapshot();
        let r0 = Store { state: s0.state.clone(), ..st.clone() };
        let e = st.find_edge(0, 1).unwrap();
        st.remove_edge(e, &mut Change::default());
        quad(&mut st, 100.0);
        st.move_vertex(5, [0.0, 0.0, 0.25]);
        st.relink_edge(e, &mut Change::default()).unwrap();
        for (now, then) in [(&st, &r0), (&r0, &st)] {
            let (a, b) = (now.changes_since(then), now.changes_since_full(then));
            for k in 0..3 {
                assert_eq!(sorted(a.created[k].clone()), sorted(b.created[k].clone()));
                assert_eq!(sorted(a.removed[k].clone()), sorted(b.removed[k].clone()));
                assert_eq!(sorted(a.modified[k].clone()), sorted(b.modified[k].clone()), "kind {k}");
            }
            assert_eq!(sorted(a.moved), sorted(b.moved));
        }
    }
}
