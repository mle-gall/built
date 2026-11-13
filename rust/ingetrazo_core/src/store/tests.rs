// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! What Python cannot reach cheaply: chunk sharing, key allocation across
//! restores, list order bookkeeping. Semantics against `core/mesh.py` are
//! tested from Python (tests/test_meshstore.py), where `Mesh` is the oracle.

use super::build::Block;
use super::cow::{CellMap, CowVec, CHUNK};
use super::keys::{self, Hwm, Kind, Order, Slots};
use super::{Store, Weld, HIDDEN, SOFT};

fn block<'a>(
    pos: &'a [[f32; 3]],
    edges: &'a [[i64; 2]],
    flags: Option<&'a [u8]>,
    curves: Option<&'a [i64]>,
    rings: &'a [i64],
    sizes: &'a [i64],
    counts: &'a [i64],
) -> Block<'a> {
    Block {
        pos,
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
    let (s, g) = slots.alloc(&mut hwm).unwrap();
    assert_eq!((s, g), (2, 0));
    slots.release(0, &hwm);
    let (s0, g0) = slots.alloc(&mut hwm).unwrap();
    assert_eq!((s0, g0), (0, 1)); // the freed slot, next generation
    // Undo: back to the snapshot (slots 0, 1 live, gen 0); hwm survives.
    slots = snap;
    let (s2, g2) = slots.alloc(&mut hwm).unwrap();
    assert_eq!((s2, g2), (2, 1), "slot 2 was issued gen 0 before the undo");
    let mut seen = std::collections::HashSet::new();
    seen.insert((2, 0));
    seen.insert((0, 1));
    assert!(seen.insert((s2, g2)));
}

#[test]
fn an_exhausted_slot_retires() {
    let (mut slots, mut hwm) = Slots::fresh(1);
    hwm.0[0] = keys::GEN_MAX;
    slots.release(0, &hwm);
    assert!(slots.free.is_empty());
    let (s, _) = slots.alloc(&mut hwm).unwrap();
    assert_eq!(s, 1);
    let _ = Hwm::default();
}

#[test]
fn list_order_matches_a_python_list() {
    // Python: remove keeps the others' order, relink appends at the end.
    let mut o = Order::identity(5);
    let mut model: Vec<u32> = (0..5).collect();
    o.remove(1);
    model.retain(|&s| s != 1);
    o.append(1); // relinked: last
    model.push(1);
    o.remove(3);
    model.retain(|&s| s != 3);
    o.append(7); // a new slot
    model.push(7);
    o.append(3);
    model.push(3);
    assert_eq!(o.iter().collect::<Vec<_>>(), model);
    let dense = o.dense_of_slot();
    for (i, &s) in model.iter().enumerate() {
        assert_eq!(dense[s as usize], i as u32);
    }
    o.compact();
    assert_eq!(o.iter().collect::<Vec<_>>(), model);
    assert_eq!(o.stale, 0);
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
    let pos: Vec<[f32; 3]> = (0..n).map(|i| [i as f32, 0.0, 0.0]).collect();
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
    assert_eq!(proto.hwm[0].0.len(), 4);
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
                pos.push([i as f32 * 0.1, j as f32 * 0.1, k as f32 * 0.1]);
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
