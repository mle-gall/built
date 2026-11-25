// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! What Python cannot reach cheaply: chunk sharing, key allocation across
//! restores, list order bookkeeping. Semantics against `core/mesh.py` are
//! tested from Python (tests/test_meshstore.py), where `Mesh` is the oracle.

use super::build::Block;
use super::cow::{CellMap, CowVec, CHUNK};
use super::keys::{self, Hwm, Kind, Order, Slots};
use super::edit::Change;
use super::stitch;
use super::topo::OpError;
use super::{Store, Weld, HIDDEN, SOFT};

fn block<'a>(
    pos: &'a [[f64; 3]],
    edges: &'a [[i64; 2]],
    flags: Option<&'a [u8]>,
    curves: Option<&'a [i128]>,
    rings: &'a [i64],
    sizes: &'a [i64],
    counts: &'a [i64],
) -> Block<'a> {
    Block {
        pos: super::build::Points::Double(pos),
        edges,
        flags,
        curves,
        layers: None,
        layer_names: vec![],
        rings,
        sizes,
        counts,
        face_attr: None,
        sequential: false,
    }
}

#[test]
fn a_snapshot_never_sees_later_writes_and_shares_untouched_chunks() {
    let mut a = CowVec::from_vec((0..3 * CHUNK as u32).collect::<Vec<_>>());
    let snap = a.clone();
    assert_eq!(a.shared_with(&snap), (3, 3));
    a.set(CHUNK + 5, 7);
    a.push(99);
    assert_eq!(*snap.get(CHUNK + 5), (CHUNK + 5) as u32);
    assert_eq!(snap.len(), 3 * CHUNK);
    assert_eq!(*a.get(CHUNK + 5), 7);
    // One chunk copied by the write, the new one is a's alone.
    assert_eq!(a.shared_with(&snap), (2, 4));
}

#[test]
fn cell_map_shards_copy_on_write() {
    let mut m = CellMap::with_expected(100_000);
    for i in 0..100_000i64 {
        m.insert((i, -i, 3), i as u32);
    }
    let snap = m.clone();
    m.remove(&(5, -5, 3));
    m.insert((5, -5, 4), 5);
    assert_eq!(snap.get(&(5, -5, 3)), Some(5));
    assert_eq!(m.get(&(5, -5, 3)), None);
    let (shared, total) = m.shared_with(&snap);
    assert!(total >= 16 && shared >= total - 2, "{shared}/{total}");
    assert!(!m.same_as(&snap));
    m.remove(&(5, -5, 4));
    m.insert((5, -5, 3), 5);
    assert!(m.same_as(&snap));
}

#[test]
fn keys_round_trip() {
    for (kind, gen, slot) in [(Kind::Vertex, 0, 0), (Kind::Edge, 4095, 67_108_863), (Kind::Face, 17, 12345)] {
        let k = keys::full_key(123_456_789_012, keys::local_key(kind, gen, slot));
        assert_eq!(keys::split_key(k), Some((123_456_789_012, kind, gen, slot)));
        assert!(keys::local_key(kind, gen, slot) < 1 << 40);
    }
    assert_eq!(keys::split_key(3u128 << 38), None);
}

#[test]
fn restoring_never_rewinds_allocation() {
    let (mut slots, mut hwm) = Slots::fresh(2);
    let snap = slots.clone();
    let (s, g, top) = slots.alloc(&mut hwm).unwrap();
    assert_eq!((s, g, top), (2, 0, true));
    slots.release(0);
    let (s0, g0, top0) = slots.alloc(&mut hwm).unwrap();
    assert_eq!((s0, g0, top0), (0, 1, false)); // the freed slot, next generation
    // Undo: back to the snapshot (slots 0, 1 live, gen 0); the floor of
    // what the discarded state issued is folded in first.
    hwm.fold(&slots, &snap);
    slots = snap;
    assert_eq!(hwm.over.get(&2), Some(&0));
    let (s2, g2, _) = slots.alloc(&mut hwm).unwrap();
    assert_eq!((s2, g2), (2, 1), "slot 2 was issued gen 0 before the undo");
    slots.release(0);
    let (s3, g3, _) = slots.alloc(&mut hwm).unwrap();
    assert_eq!((s3, g3), (0, 2), "slot 0 was issued gen 1 before the undo");
    assert!(hwm.over.is_empty(), "floors are dropped once passed");
}

#[test]
fn an_exhausted_slot_retires() {
    let (mut slots, mut hwm) = Slots::fresh(1);
    slots.state.set(0, keys::GEN_MAX << 1 | 1);
    slots.release(0);
    assert!(slots.free.is_empty());
    let (s, _, _) = slots.alloc(&mut hwm).unwrap();
    assert_eq!(s, 1);
    let _ = Hwm::default();
}

#[test]
fn list_order_matches_a_python_list() {
    // Python: remove keeps the others' order, relink appends at the end.
    let (mut slots, mut hwm) = Slots::fresh(5);
    let mut o = Order::identity();
    let mut model: Vec<u32> = (0..5).collect();
    slots.release(4);
    o.remove(4);
    model.retain(|&s| s != 4);
    assert!(o.explicit.is_none(), "removal keeps the order implicit");
    let (s, _, top) = slots.alloc(&mut hwm).unwrap(); // reuses slot 4 ...
    assert_eq!(s, 4);
    o.append(s, &slots, top); // ... but it lists last: still ascending
    model.push(4);
    assert_eq!(o.live(&slots), model);
    slots.release(1);
    o.remove(1);
    model.retain(|&s| s != 1);
    let (s, _, top) = slots.alloc(&mut hwm).unwrap(); // slot 1, listed last
    assert_eq!((s, top), (1, false));
    o.append(s, &slots, top);
    model.push(1);
    assert!(o.explicit.is_some());
    slots.release(3);
    o.remove(3);
    model.retain(|&s| s != 3);
    let (s, _, top) = slots.alloc(&mut hwm).unwrap();
    o.append(s, &slots, top);
    model.push(s);
    let (s, _, top) = slots.alloc(&mut hwm).unwrap(); // a brand new slot
    o.append(s, &slots, top);
    model.push(s);
    assert_eq!(o.live(&slots), model);
    let dense = o.dense_of_slot(&slots);
    for (i, &s) in model.iter().enumerate() {
        assert_eq!(dense[s as usize], i as u32);
    }
    o.compact(&slots);
    assert_eq!(o.live(&slots), model);
    assert_eq!(o.stale(), 0);
}

#[test]
fn block_order_rules() {
    // A square and a triangle sharing the edge (1, 2); edge input lists
    // (2, 1) twice (soft, then curve 9), a pair welded shut by a 2e-6
    // nudge, and a face whose ring has two welded corners (self-loop).
    let pos = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [2.0, 0.5, 0.0],
        [5.0, 0.0, 0.0],
        [5.000002, 0.0, 0.0],
    ];
    let edges = [[2, 1], [5, 6], [1, 2]];
    let flags = [SOFT, 0, HIDDEN];
    let curves = [-1, -1, 9];
    let rings = [0, 1, 2, 3, 1, 4, 2, 5, 6, 0];
    let sizes = [4, 3, 3];
    let counts = [1, 1, 1];
    let b = block(&pos, &edges, Some(&flags), Some(&curves), &rings, &sizes, &counts);
    let s = Store::from_block(1, &b, Weld::default()).unwrap();
    let d = s.dense();
    assert_eq!(d.pos.len(), 6, "5 and 6 weld");
    // Edge 0 is the input's first pair in its direction; the shut pair
    // made nothing; the square's segments come next in ring order.
    assert_eq!(d.edges[0], [2, 1]);
    assert_eq!(d.flags[0], SOFT | HIDDEN);
    assert_eq!(d.curve[0], 9);
    assert_eq!(&d.edges[1..4], &[[0, 1], [2, 3], [3, 0]]);
    // Triangle: (1,4), (4,2), then (2,1) exists.
    assert_eq!(&d.edges[4..6], &[[1, 4], [4, 2]]);
    // Third face: (5,5) self-loop, (5,0).
    assert_eq!(&d.edges[6..8], &[[5, 5], [5, 0]]);
    let faces_of = |e: usize| d.ef_face[d.ef_start[e] as usize..d.ef_start[e + 1] as usize].to_vec();
    assert_eq!(faces_of(0), vec![0, 1]);
    assert_eq!(faces_of(3), vec![0]);
    assert_eq!(faces_of(6), vec![2]);
    assert_eq!(faces_of(7), vec![2, 2], "walked twice: listed twice");
    assert_eq!(d.corners, vec![0, 1, 2, 3, 1, 4, 2, 5, 5, 0]);
    // The self-loop sits once in its vertex's disk.
    assert_eq!(s.edges_at(5).filter(|&e| e == 6).count(), 1);
    assert_eq!(s.degree(1), 3);
    assert_eq!(s.faces_around(&[4]), vec![1]);
}

fn chain(n: usize) -> Store {
    let pos: Vec<[f64; 3]> = (0..n).map(|i| [i as f64, 0.0, 0.0]).collect();
    let edges: Vec<[i64; 2]> = (0..n as i64 - 1).map(|i| [i, i + 1]).collect();
    let b = block(&pos, &edges, None, None, &[], &[], &[]);
    Store::from_block(7, &b, Weld::default()).unwrap()
}

#[test]
fn moving_a_chain_along_itself_queues_then_promotes() {
    // Mesh._rekey's docstring case: each vertex lands where the next one
    // still is, waits in `displaced`, and takes the cell when it leaves.
    let mut s = chain(3);
    for v in 0..3 {
        let c = s.move_vertex(v, [1.0, 0.0, 0.0]);
        assert_eq!(c.moved, vec![s.key(Kind::Vertex, v)]);
        if v < 2 {
            assert_eq!(s.state.displaced.len(), 1);
        }
    }
    assert!(s.state.displaced.is_empty());
    for v in 0..3u32 {
        assert_eq!(s.state.registry.get(&(10_000 * (v as i64 + 1), 0, 0)), Some(v));
        assert!(s.registers(v));
    }
    assert_eq!(s.state.registry.len(), 3);
    assert_eq!(s.revision, 3);
}

#[test]
fn restore_brings_the_registry_back_and_clears_displaced() {
    let mut s = chain(3);
    let snap = s.snapshot();
    s.move_vertex(0, [1.0, 0.0, 0.0]); // onto vertex 1: waits
    assert!(!s.state.displaced.is_empty());
    assert!(!s.state.same_as(&snap.state));
    s.restore(&snap, None).unwrap();
    assert!(s.state.displaced.is_empty());
    assert!(s.state.same_as(&snap.state));
    assert_eq!(s.lookup([0.0, 0.0, 0.0]), Some(0));
    // Moved and put back by hand: different chunks, same content.
    s.move_vertex(2, [0.5, 0.0, 0.0]);
    s.place_vertex(2, [2.0, 0.0, 0.0]);
    assert!(s.state.same_as(&snap.state));
}

#[test]
fn a_foreign_snapshot_is_adopted_under_a_fresh_namespace() {
    let mut proto = chain(2);
    let local = chain(4);
    let snap = local.snapshot();
    assert!(proto.restore(&snap, None).is_err());
    proto.restore(&snap, Some(99)).unwrap();
    assert_eq!(proto.state.ns, 99);
    assert_eq!(proto.n(Kind::Vertex), 4);
    assert_eq!(proto.hwm[0].len, 4);
    let k = proto.key(Kind::Vertex, 3);
    assert_eq!(proto.slot(Kind::Vertex, k), Some(3));
    assert_eq!(local.slot(Kind::Vertex, k), None, "other namespace");
}

#[test]
fn snapshots_are_cheap_on_a_big_store() {
    // 1M points in a 100^3 lattice, no topology: the snapshot must not
    // walk them, and one move must copy a sliver.
    let n = 100usize;
    let mut pos = Vec::with_capacity(n * n * n);
    for i in 0..n {
        for j in 0..n {
            for k in 0..n {
                pos.push([i as f64 * 0.1, j as f64 * 0.1, k as f64 * 0.1]);
            }
        }
    }
    let b = block(&pos, &[], None, None, &[], &[], &[]);
    let mut s = Store::from_block(3, &b, Weld::default()).unwrap();
    let t = std::time::Instant::now();
    let snap = s.snapshot();
    let took = t.elapsed();
    assert!(took.as_millis() < 5, "snapshot took {took:?}");
    s.move_vertex(500_000, [0.05, 0.0, 0.0]);
    let (shared, total) = s.state.vpos.shared_with(&snap.state.vpos);
    assert_eq!(total - shared, 1);
    let (rs, rt) = s.state.registry.shared_with(&snap.state.registry);
    assert!(rt - rs <= 2, "{rs}/{rt}");
}

// ---- port step 2: incidence ops ------------------------------------------

fn grid_store(n: usize) -> Store {
    let mut st = Store::empty(7, Weld::default());
    let mut ch = super::edit::Change::default();
    for i in 0..n {
        for j in 0..n {
            let (x, y) = (i as f64, j as f64);
            let ring = vec![[x, y, 0.0], [x + 1.0, y, 0.0], [x + 1.0, y + 1.0, 0.0], [x, y + 1.0, 0.0]];
            st.add_face(&[ring], &mut ch).unwrap();
        }
    }
    st.check().unwrap();
    st
}

#[test]
fn removing_the_last_entries_keeps_an_explicit_order_free_of_stale_ones() {
    let mut st = grid_store(3);
    let mut ch = super::edit::Change::default();
    // Relinking a low face makes the order explicit (it goes last)...
    st.remove_face(0, &mut ch);
    st.relink_face(0, &mut ch).unwrap();
    assert!(st.state.order[2].explicit.is_some());
    assert_eq!(*st.live_slots(Kind::Face).last().unwrap(), 0);
    // ...and LIFO removals pop, so indexing stays O(1).
    let n = st.n(Kind::Face);
    st.add_face(&[vec![[50.0, 0.0, 0.0], [51.0, 0.0, 0.0], [51.0, 1.0, 0.0]]], &mut ch).unwrap();
    let top = *st.live_slots(Kind::Face).last().unwrap();
    st.remove_face(top, &mut ch);
    st.remove_face(0, &mut ch);
    assert_eq!(st.state.order[2].stale(), 0);
    assert_eq!(st.n(Kind::Face), n - 1);
    assert!(st.state.order[2].nth_fast(&st.state.slots[2], 0).is_some());
    st.check().unwrap();
}

#[test]
fn a_tombstone_keeps_its_key_and_data_and_its_slot_is_never_reused() {
    let mut st = grid_store(2);
    let mut ch = super::edit::Change::default();
    let e = st.find_edge(0, 1).unwrap();
    let key = st.key(Kind::Edge, e);
    st.remove_edge(e, &mut ch);
    assert_eq!(st.slot(Kind::Edge, key), None);
    assert_eq!(st.slot_any(Kind::Edge, key), Some(e));
    assert_eq!(st.edge_faces(e), &[0]); // its faces list stays
    let before = st.state.slots[1].len();
    st.add_edge([9.0, 9.0, 0.0], [9.0, 8.0, 0.0], &mut ch).unwrap();
    assert_eq!(st.state.slots[1].len(), before + 1); // a new slot, not e's
    // Back while face 0 is listed: its faces list is as it was.
    assert_eq!(st.relink_edge(e, &mut ch), Ok(true));
    // Removed again, then face 0 goes: e would name a gone face.
    st.remove_edge(e, &mut ch);
    st.remove_face(0, &mut ch);
    assert_eq!(st.relink_edge(e, &mut ch), Err(super::topo::OpError::Fallback("relink_edge listing a gone face")));
    // relink_face makes the missing edge again: e would be a second one.
    st.relink_face(0, &mut ch).unwrap();
    assert_eq!(
        st.relink_edge(e, &mut ch),
        Err(super::topo::OpError::Fallback("relink_edge beside another edge between its ends"))
    );
    st.check().unwrap();
    assert_eq!(st.slot(Kind::Edge, key), None);
    assert_eq!(st.slot_any(Kind::Edge, key), Some(e));
}

#[test]
fn a_relinked_edge_is_its_own_key_again() {
    let mut st = grid_store(2);
    let mut ch = super::edit::Change::default();
    let e = st.find_edge(0, 1).unwrap();
    let key = st.key(Kind::Edge, e);
    st.remove_edge(e, &mut ch);
    assert_eq!(st.relink_edge(e, &mut ch), Ok(true));
    assert_eq!(st.slot(Kind::Edge, key), Some(e));
    st.check().unwrap();
}

#[test]
fn unlinking_from_the_middle_of_a_disk_cycle() {
    let mut st = grid_store(3);
    let mut ch = super::edit::Change::default();
    // The centre-ish vertex (1, 1) has four edges.
    let v = st.lookup([1.0, 1.0, 0.0]).unwrap();
    let around: Vec<u32> = st.edges_at(v).collect();
    assert_eq!(around.len(), 4);
    st.remove_edge(around[2], &mut ch);
    st.remove_edge(around[0], &mut ch);
    let left: Vec<u32> = st.edges_at(v).collect();
    assert_eq!(left, vec![around[1], around[3]]);
    st.check().unwrap();
}

#[test]
fn rewritten_rings_are_compacted_with_the_tombstones() {
    let mut st = grid_store(1);
    let mut ch = super::edit::Change::default();
    let f = 0;
    for k in 0..40_000 {
        let x = 0.1 + (k % 7) as f64 * 1e-3;
        let ring = st
            .add_hole(f, &[[x, 0.2, 0.0], [x + 0.01, 0.2, 0.0], [x + 0.01, 0.3, 0.0]], &mut ch)
            .unwrap();
        st.remove_hole(f, &ring, &mut ch);
    }
    assert!(st.state.pool_garbage < 200_000);
    assert_eq!(st.face_rings(f).len(), 1);
    st.check().unwrap();
}

#[test]
fn cowvec_pop_drops_empty_chunks() {
    let mut a = CowVec::from_vec((0..CHUNK as u32 + 2).collect::<Vec<_>>());
    let snap = a.clone();
    assert_eq!(a.pop(), Some(CHUNK as u32 + 1));
    assert_eq!(a.pop(), Some(CHUNK as u32));
    assert_eq!(a.n_chunks(), 1);
    a.push(5);
    assert_eq!(*a.get(CHUNK), 5);
    assert_eq!(*snap.get(CHUNK + 1), CHUNK as u32 + 1);
}

// ---- port step 3 -----------------------------------------------------------

fn square(st: &mut Store, x: f64, ch: &mut Change) -> u32 {
    st.add_face(&[vec![[x, 0.0, 0.0], [x + 1.0, 0.0, 0.0], [x + 1.0, 1.0, 0.0], [x, 1.0, 0.0]]], ch).unwrap()
}

#[test]
fn split_first_half_takes_the_edge_slot_and_key() {
    let mut st = Store::empty(9, Weld::default());
    let mut ch = Change::default();
    let f = square(&mut st, 0.0, &mut ch);
    let e = st.find_edge(0, 1).unwrap();
    let key = st.key(Kind::Edge, e);
    let mut ch = Change::default();
    let (e0, e1, bumps) = st.split_edge(e, [0.5, 0.0, 0.0], &mut ch).unwrap();
    assert_eq!((e0, bumps), (e, 3));
    assert_eq!(st.key(Kind::Edge, e0), key);
    assert_ne!(e1, e0);
    assert_eq!(st.face_rings(f)[0].len(), 5);
    assert_eq!(st.edge_faces(e0), &[f]);
    assert_eq!(st.edge_faces(e1), &[f]);
    // The record: the inherited key is modified, not removed and created.
    let ch = st.finish(ch);
    assert!(!ch.removed[1].contains(&key) && !ch.created[1].contains(&key));
    assert!(ch.modified[1].contains(&key));
    st.check().unwrap();
}

#[test]
fn a_cross_weld_hands_the_cell_over_and_takes_it_back_on_death() {
    let mut st = Store::empty(9, Weld::default());
    let mut ch = Change::default();
    // Two vertices a hair apart across a cell border, each with an edge.
    let a = st.vertex([0.000_049, 0.0, 0.0], &mut ch).unwrap();
    let b = st.vertex([5.0, 0.0, 0.0], &mut ch).unwrap();
    st.place_vertex(b, [0.000_051, 0.0, 0.0]); // a move never welds
    assert_ne!(st.cell(st.state.vpos.get(a as usize)), st.cell(st.state.vpos.get(b as usize)));
    let c = st.vertex([1.0, 0.0, 0.0], &mut ch).unwrap();
    st.link_edge(a, c, &mut ch).unwrap();
    st.link_edge(b, c, &mut ch).unwrap();
    let (changed, _) = st.weld_coincident(&mut ch).unwrap();
    assert!(changed);
    let (keep, gone) = if st.alive(Kind::Vertex, a) { (a, b) } else { (b, a) };
    assert!(!st.alive(Kind::Vertex, gone));
    let cell = st.cell(st.state.vpos.get(gone as usize));
    assert_eq!(st.state.registry.get(&cell), Some(keep));
    assert_eq!(st.state.extra.get(&keep).map(|v| v.len()), Some(1));
    // The survivor's death takes both its cells: no ghost entry.
    let e = st.find_edge(keep, c).unwrap();
    st.remove_edge(e, &mut ch);
    assert_eq!(st.drop_orphan_vertices(&[keep], &mut ch).unwrap(), 1);
    assert_eq!(st.state.registry.get(&cell), None);
    st.check().unwrap();
}

#[test]
fn trace_and_splice_follow_the_python_rules() {
    // Start at the first outline segment, in its direction.
    let outline = [(3, 4), (1, 2), (2, 3), (4, 1)];
    assert_eq!(stitch::trace_loops(&outline), Some(vec![vec![3, 4, 1, 2]]));
    assert_eq!(stitch::trace_loops(&[(1, 1)]), None);
    assert_eq!(stitch::trace_loops(&[(1, 2), (2, 3), (3, 1), (1, 4)]), None);
    // A runs u→w, B w→u: the splice drops the shared edge.
    assert_eq!(stitch::splice_loops(&[1, 2, 3, 4], &[2, 1, 5, 6], 1, 2), Some(vec![2, 3, 4, 1, 5, 6]));
    assert_eq!(stitch::splice_loops(&[1, 2, 3, 4], &[1, 2, 5, 6], 1, 2), None);
}

#[test]
fn a_refused_weld_changes_nothing() {
    let mut st = Store::empty(9, Weld::default());
    let mut ch = Change::default();
    let a = st.vertex([0.0, 0.0, 0.0], &mut ch).unwrap();
    let b = st.vertex([1.0, 0.0, 0.0], &mut ch).unwrap();
    st.link_edge(b, b, &mut ch).unwrap(); // a self-loop on the vertex to go
    let before = st.snapshot();
    let mut occ = st.corner_counts();
    let r = st.weld_into(a, b, &mut occ, &mut ch);
    assert!(matches!(r, Err(OpError::Fallback(_))));
    assert!(st.state.same_as(&before.state));
}
