// SPDX-License-Identifier: GPL-3.0-or-later
//! Click selection follows topology in the current editing store.
use crate::store::{keys::Kind, Store, SOFT};
use std::collections::HashSet;

fn boundary(s: &Store, face: u32) -> Vec<u32> {
    let mut edges = Vec::new();
    for ring in s.face_rings(face) {
        for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            edges.extend(s.find_edge(a, b));
        }
    }
    edges
}

fn surface(s: &Store, face: u32) -> Vec<u32> {
    let mut faces = vec![face];
    let mut seen = HashSet::from([face]);
    let mut i = 0;
    while i < faces.len() {
        for e in boundary(s, faces[i]) {
            if *s.state.eflags.get(e as usize) & SOFT == 0 {
                continue;
            }
            for &f in s.edge_faces(e) {
                if seen.insert(f) {
                    faces.push(f);
                }
            }
        }
        i += 1;
    }
    faces
}

fn connected(s: &Store, kind: Kind, slot: u32) -> (Vec<u32>, Vec<u32>) {
    let mut todo: Vec<_> = if kind == Kind::Edge {
        s.state.ends.get(slot as usize).to_vec()
    } else {
        s.face_rings(slot).into_iter().flatten().collect()
    };
    let mut visited = HashSet::new();
    let (mut edges, mut faces) = (HashSet::new(), HashSet::new());
    while let Some(v) = todo.pop() {
        if !visited.insert(v) {
            continue;
        }
        for e in s.edges_at(v) {
            if !edges.insert(e) {
                continue;
            }
            todo.extend(s.state.ends.get(e as usize));
            for &f in s.edge_faces(e) {
                if faces.insert(f) {
                    todo.extend(s.face_rings(f).into_iter().flatten());
                }
            }
        }
    }
    let (mut es, mut fs) = (
        edges.into_iter().collect::<Vec<_>>(),
        faces.into_iter().collect::<Vec<_>>(),
    );
    es.sort_unstable();
    fs.sort_unstable();
    (es, fs)
}

pub(crate) fn selection(s: &Store, key: u128, grow: &str) -> Option<(Vec<u128>, Vec<u128>)> {
    let (kind, slot) = s
        .slot(Kind::Edge, key)
        .map(|e| (Kind::Edge, e))
        .or_else(|| s.slot(Kind::Face, key).map(|f| (Kind::Face, f)))?;
    let (es, fs) = if grow == "connected" {
        connected(s, kind, slot)
    } else if kind == Kind::Face {
        let fs = surface(s, slot);
        let es = if grow == "bounded" {
            fs.iter().flat_map(|&f| boundary(s, f)).collect()
        } else {
            vec![]
        };
        (es, fs)
    } else {
        let curve = *s.state.ecurve.get(slot as usize);
        let keys = if curve == -1 {
            vec![key]
        } else {
            s.curve_keys(curve)
        };
        let faces = if grow == "bounded" {
            keys.iter()
                .filter_map(|e| s.slot(Kind::Edge, *e))
                .flat_map(|e| s.edge_faces(e))
                .map(|&f| s.key(Kind::Face, f))
                .collect()
        } else {
            vec![]
        };
        return Some((keys, faces));
    };
    Some((
        es.into_iter().map(|e| s.key(Kind::Edge, e)).collect(),
        fs.into_iter().map(|f| s.key(Kind::Face, f)).collect(),
    ))
}
