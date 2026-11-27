// SPDX-License-Identifier: GPL-3.0-or-later
use super::table::Row;
use crate::{
    element_ids as eid,
    store::{edit::Change, keys::Kind, Store},
};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

fn h64(bytes: &[u8]) -> u64 {
    u64::from_be_bytes(Sha256::digest(bytes)[..8].try_into().unwrap())
}

pub fn generate(store: &mut Store, row: &Row, origin: [f64; 3]) -> Result<(), &'static str> {
    if row.size.iter().any(|x| !x.is_finite() || *x <= 0.) {
        return Err("invalid_size");
    }
    let mut points = [[0.; 3]; 8];
    for (i, p) in points.iter_mut().enumerate() {
        *p = std::array::from_fn(|j| origin[j] + if i & (1 << j) != 0 { row.size[j] } else { 0. });
        if p.iter().any(|x| !x.is_finite()) {
            return Err("invalid_size");
        }
    }
    let faces = [
        ("bottom", [0, 2, 3, 1]),
        ("left", [0, 4, 6, 2]),
        ("right", [1, 3, 7, 5]),
        ("front", [0, 1, 5, 4]),
        ("back", [2, 6, 7, 3]),
        ("top", [4, 5, 7, 6]),
    ];
    let mut roles = Vec::new();
    let mut change = Change::default();
    for (role, ring) in faces {
        if role == "top" && row.open_top {
            continue;
        }
        let f = store
            .add_face(&[ring.map(|i| points[i]).to_vec()], &mut change)
            .map_err(|_| "invalid_size")?;
        roles.push((format!("face/{role}"), Kind::Face, f));
    }
    if store.n(Kind::Vertex) != 8 {
        return Err("invalid_size");
    }
    let corner = |v: u32| {
        points
            .iter()
            .position(|p| p == store.state.vpos.get(v as usize))
            .ok_or("invalid_size")
    };
    for v in store.live_slots(Kind::Vertex) {
        roles.push((format!("vertex/{}", corner(v)?), Kind::Vertex, v));
    }
    for e in store.live_slots(Kind::Edge) {
        let [a, b] = *store.state.ends.get(e as usize);
        let (a, b) = (corner(a)?, corner(b)?);
        roles.push((format!("edge/{}/{}", a.min(b), a.max(b)), Kind::Edge, e));
    }
    assign_roles(store, roles)
}

pub(super) fn assign_roles(
    store: &mut Store,
    mut roles: Vec<(String, Kind, u32)>,
) -> Result<(), &'static str> {
    roles.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let index = eid::intern(h64(b"built:generated:v1")).map_err(|_| "origin_limit")?;
    let mut used = HashSet::new();
    for (role, kind, slot) in roles {
        let mut counter = h64(role.as_bytes()) & eid::COUNTER_MASK;
        while !used.insert(counter) {
            counter = (counter + 1) & eid::COUNTER_MASK;
        }
        store.set_pid(kind, slot, eid::pack(index, counter));
    }
    Ok(())
}

/// Hash logical ids, canonical positions and connectivity in id order.
/// Slot indices, process origin indices and file row order are transient.
pub fn stamp(store: &Store) -> String {
    content_hash(store, false)
}
pub(super) fn part_key(store: &Store) -> String {
    content_hash(store, true)
}
fn content_hash(store: &Store, quantize: bool) -> String {
    let mut hash = Sha256::new();
    // Adjacent generated elements normally share an origin. Resolve it once
    // instead of locking the process origin table for every face corner.
    let mut last = (u32::MAX, 0u64);
    let mut logical = |kind, slot| {
        let id = store.pid(kind, slot);
        let index = eid::origin_index(id);
        if index != last.0 {
            last = (index, eid::origin_of(index).unwrap_or(0));
        }
        [last.1, eid::counter(id)]
    };
    let mut words = Vec::new();
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    let mut bytes = Vec::new();
    for kind in Kind::ALL {
        words.clear();
        ranges.clear();
        bytes.clear();
        for slot in store.live_slots(kind) {
            let start = words.len();
            words.extend(logical(kind, slot));
            match kind {
                Kind::Vertex => words.extend(
                    store
                        .state
                        .frame
                        .definition_point(*store.state.vpos.get(slot as usize))
                        .map(|v| {
                            if quantize {
                                (v * 1e6).round() as i64 as u64
                            } else if v == 0. {
                                0
                            } else {
                                v.to_bits()
                            }
                        }),
                ),
                Kind::Edge => {
                    let mut ends = store
                        .state
                        .ends
                        .get(slot as usize)
                        .map(|v| logical(Kind::Vertex, v));
                    ends.sort();
                    words.extend(ends.into_iter().flatten());
                }
                Kind::Face => {
                    for ring in store.face_rings(slot) {
                        words.push(ring.len() as u64);
                        words.extend(ring.into_iter().flat_map(|v| logical(Kind::Vertex, v)));
                    }
                }
            }
            ranges.push(start..words.len());
        }
        ranges.sort_unstable_by(|a, b| words[a.clone()].cmp(&words[b.clone()]));
        bytes.extend_from_slice(&(ranges.len() as u64).to_le_bytes());
        for range in &ranges {
            for word in &words[range.clone()] {
                bytes.extend_from_slice(&word.to_le_bytes());
            }
        }
        hash.update(&bytes);
    }
    format!(
        "{:016x}",
        u64::from_be_bytes(hash.finalize()[..8].try_into().unwrap())
    )
}

pub(super) fn matches_role(pid:u64,role:&str)->bool {
    eid::origin_of(eid::origin_index(pid))==Some(h64(b"built:generated:v1")) &&
        eid::counter(pid)==h64(role.as_bytes()) & eid::COUNTER_MASK
}
