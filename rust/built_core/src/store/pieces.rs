// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! `core/pieces.py` over a store: the connected pieces of a mesh, its
//! physical pieces (solids that do not touch, fragments joined to the
//! solid they lie on), and the copy of one piece into a new store (Split
//! into Pieces). Reads only: nothing here changes the store it reads.
//!
//! Each function is its Python namesake step for step, so the pieces, their
//! order and the order of faces and edges inside each are Python's:
//! - `connected_parts`: union of every face's corners and every edge's
//!   ends; a part per root in order of its first face (then of its first
//!   edge), largest first (stable).
//! - `solid_parts`: faces paired around each edge with the face reached
//!   turning into the material (`_edge_partners`, QVector3D arithmetic in
//!   float32 with this Qt's `length()`, angles in double), mutual pairs
//!   only where three or more faces meet; closed results with thickness
//!   and four faces are solids; the rest cluster through shared corners
//!   and join the solid most of their corners lie on (ties: the first
//!   solid, `core/pieces.py` since this port) when at least half do.
//! - `extract`: `copy_part` — points (each edge's ends, then each face's
//!   rings), the bulk weld, edges with the LAST copy's flags winning where
//!   two weld onto one pair (`stamp_edge_flags` in input order), faces
//!   with the attribute rows given — built by `Store::from_block`, which
//!   is that very sequence.

use super::build::Block;
use super::keys::{Kind, NONE};
use super::{Store, Weld, FLAT, HIDDEN, SOFT};
use crate::store::geometry::{self as facegeom, Hypot, V3};
use std::collections::HashMap;
use std::f64::consts::TAU;

/// A piece: face slots and edge slots, each in Python's order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Part {
    pub faces: Vec<u32>,
    pub edges: Vec<u32>,
}

/// A result thinner than this is a sheet, not a board (`MIN_THICKNESS`).
const MIN_THICKNESS: f64 = 0.001;
/// Share of a fragment's corners that must lie on a solid (`ATTACH_SHARE`).
const ATTACH_SHARE: f64 = 0.5;
/// Faces closer than this in angle about an edge are one plane (`_COINCIDENT`).
const COINCIDENT: f64 = 1e-3;

struct Uf(Vec<u32>);

impl Uf {
    fn new(n: usize) -> Self {
        Uf((0..n as u32).collect())
    }
    fn find(&mut self, x: u32) -> u32 {
        let mut root = x;
        while self.0[root as usize] != root {
            root = self.0[root as usize];
        }
        let mut x = x;
        while self.0[x as usize] != root {
            let next = self.0[x as usize];
            self.0[x as usize] = root;
            x = next;
        }
        root
    }
    fn join(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.0[ra as usize] = rb;
        }
    }
}

fn sort_parts(parts: &mut [Part]) {
    // `sorted(..., key=lambda p: (-len(p[0]), -len(p[1])))`: stable.
    parts.sort_by(|a, b| b.faces.len().cmp(&a.faces.len()).then(b.edges.len().cmp(&a.edges.len())));
}

/// `QVector3D::normalize()` (in place: a vector of length ~1 or ~0 is
/// left as it is, unlike `normalized()`).
fn normalize(v: V3, h: Hypot) -> V3 {
    let len = facegeom::length(v, h);
    if (len - 1.0).abs() <= 0.00001 || len.abs() <= 0.00001 {
        return v;
    }
    V3(v.0 / len, v.1 / len, v.2 / len)
}

fn is_null(v: V3) -> bool {
    v.0 == 0.0 && v.1 == 0.0 && v.2 == 0.0
}

impl Store {
    /// `connected_parts` over the given faces and edges (all live).
    pub fn connected_parts_of(&self, faces: &[u32], edges: &[u32]) -> Vec<Part> {
        let mut uf = Uf::new(self.state.slots[Kind::Vertex as usize].len());
        let mut rings_of = Vec::with_capacity(faces.len());
        for &f in faces {
            let rings = self.face_rings(f);
            let first = rings.first().and_then(|r| r.first()).copied();
            if let Some(r) = first {
                for ring in &rings {
                    for &v in ring {
                        uf.join(v, r);
                    }
                }
            }
            rings_of.push(first);
        }
        for &e in edges {
            let [a, b] = *self.state.ends.get(e as usize);
            uf.join(b, a);
        }
        let mut index: HashMap<u32, usize> = HashMap::new();
        let mut parts: Vec<Part> = Vec::new();
        for (i, &f) in faces.iter().enumerate() {
            let Some(r) = rings_of[i] else { continue };
            let root = uf.find(r);
            let n = *index.entry(root).or_insert_with(|| {
                parts.push(Part::default());
                parts.len() - 1
            });
            parts[n].faces.push(f);
        }
        for &e in edges {
            let root = uf.find(self.state.ends.get(e as usize)[0]);
            let n = *index.entry(root).or_insert_with(|| {
                parts.push(Part::default());
                parts.len() - 1
            });
            parts[n].edges.push(e);
        }
        sort_parts(&mut parts);
        parts
    }

    /// `connected_parts(mesh)`.
    pub fn connected_parts(&self) -> Vec<Part> {
        let faces = self.live_slots(Kind::Face);
        let edges = self.live_slots(Kind::Edge);
        self.connected_parts_of(&faces, &edges)
    }

    /// `pieces._newell`: the sums in double, the vector in float32.
    fn ring_newell(&self, ring: &[u32]) -> V3 {
        let (mut nx, mut ny, mut nz) = (0.0f64, 0.0f64, 0.0f64);
        let m = ring.len();
        for i in 0..m {
            let p = self.state.vpos.get(ring[i] as usize);
            let q = self.state.vpos.get(ring[(i + 1) % m] as usize);
            nx += (p[1] - q[1]) * (p[2] + q[2]);
            ny += (p[2] - q[2]) * (p[0] + q[0]);
            nz += (p[0] - q[0]) * (p[1] + q[1]);
        }
        V3(nx, ny, nz)
    }

    /// `_inward(face, a, b, n)`.
    fn inward(&self, f: u32, a: u32, b: u32, n: V3, h: Hypot) -> Option<V3> {
        for (k, ring) in self.face_rings(f).iter().enumerate() {
            let m = ring.len();
            for i in 0..m {
                let (p, q) = (ring[i], ring[(i + 1) % m]);
                if (p == a && q == b) || (p == b && q == a) {
                    let mut w = n.cross(self.position_vec(q).sub(self.position_vec(p)), h);
                    if k > 0 && self.ring_newell(ring).dot(n, h) > 0.0 {
                        w = V3(w.0 * -1.0, w.1 * -1.0, w.2 * -1.0);
                    }
                    if w.dot(w, h) == 0.0 {
                        return None;
                    }
                    return Some(facegeom::normalized(w, h));
                }
            }
        }
        None
    }

    /// `_edge_partners(edge)`: `(face, partner)` slot pairs.
    fn edge_partners(&self, e: u32, h: Hypot) -> Vec<(u32, u32)> {
        let faces = self.edge_faces(e);
        if faces.len() < 3 {
            return if faces.len() == 2 { vec![(faces[0], faces[1])] } else { Vec::new() };
        }
        let [a, b] = *self.state.ends.get(e as usize);
        let axis = self.position_vec(b).sub(self.position_vec(a));
        if axis.dot(axis, h) == 0.0 {
            return Vec::new();
        }
        let axis = normalize(axis, h);
        let mut refs: Option<(V3, V3)> = None;
        // (face, theta, s, w, n)
        let mut info: Vec<(u32, f64, f64, V3, V3)> = Vec::with_capacity(faces.len());
        for &f in faces {
            let n = self.face_normal(f, h);
            if is_null(n) {
                continue;
            }
            let Some(w) = self.inward(f, a, b, n, h) else { continue };
            let (ru, rv) = *refs.get_or_insert_with(|| (w, axis.cross(w, h)));
            let theta = w.dot(rv, h).atan2(w.dot(ru, h));
            let spin = axis.cross(w, h).dot(n, h);
            info.push((f, theta, if spin > 0.0 { -1.0 } else { 1.0 }, w, n));
        }
        let mut pairs = Vec::with_capacity(info.len());
        for &(f, theta, s, w_f, _) in &info {
            let mut best: Option<(u32, (i64, u8))> = None;
            for &(g, phi, _, _, n_g) in &info {
                if g == f {
                    continue;
                }
                let mut d = ((phi - theta) * s).rem_euclid(TAU);
                if d < COINCIDENT || TAU - d < COINCIDENT {
                    d = TAU;
                }
                let facing = if n_g.dot(w_f, h) < 0.0 { 0 } else { 1 };
                let key = ((d / COINCIDENT).round_ties_even() as i64, facing);
                if best.is_none_or(|(_, k)| key < k) {
                    best = Some((g, key));
                }
            }
            if let Some((g, _)) = best {
                pairs.push((f, g));
            }
        }
        pairs
    }

    /// `_is_solid(faces)`.
    pub(super) fn is_solid(&self, faces: &[u32]) -> bool {
        if faces.len() < 4 {
            return false;
        }
        let mut uses: HashMap<(u32, u32), u32> = HashMap::new();
        for &f in faces {
            for ring in self.face_rings(f) {
                let n = ring.len();
                for i in 0..n {
                    let (a, b) = (ring[i], ring[(i + 1) % n]);
                    *uses.entry(if a < b { (a, b) } else { (b, a) }).or_insert(0) += 1;
                }
            }
        }
        if uses.values().any(|c| c % 2 == 1) {
            return false;
        }
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for &f in faces {
            for v in self.outer_ring(f) {
                let p = self.state.vpos.get(v as usize);
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
        }
        (0..3).map(|k| hi[k] - lo[k]).fold(f64::INFINITY, f64::min) >= MIN_THICKNESS
    }

    fn corners_of(&self, f: u32, out: &mut Vec<u32>) {
        for ring in self.face_rings(f) {
            out.extend(ring);
        }
    }

    /// `solid_parts(mesh)`.
    pub fn solid_parts(&self, h: Hypot) -> Vec<Part> {
        let faces = self.live_slots(Kind::Face);
        let edges = self.live_slots(Kind::Edge);
        let nfs = self.state.slots[Kind::Face as usize].len();
        let nvs = self.state.slots[Kind::Vertex as usize].len();
        let mut uf = Uf::new(nfs);
        for &e in &edges {
            let mut pairs = self.edge_partners(e, h);
            if self.edge_faces(e).len() > 2 {
                let mut chosen: HashMap<u32, u32> = HashMap::new();
                for &(f, g) in &pairs {
                    chosen.insert(f, g);
                }
                pairs.retain(|&(f, g)| chosen.get(&g) == Some(&f));
            }
            for (f, g) in pairs {
                uf.join(f, g);
            }
        }
        let mut comp_of_root: HashMap<u32, usize> = HashMap::new();
        let mut comps: Vec<Vec<u32>> = Vec::new();
        for &f in &faces {
            let r = uf.find(f);
            let i = *comp_of_root.entry(r).or_insert_with(|| {
                comps.push(Vec::new());
                comps.len() - 1
            });
            comps[i].push(f);
        }
        let solid: Vec<bool> = comps.iter().map(|c| self.is_solid(c)).collect();
        let mut owner = vec![NONE; nvs];
        let mut scratch = Vec::new();
        for (i, c) in comps.iter().enumerate() {
            if !solid[i] {
                continue;
            }
            for &f in c {
                scratch.clear();
                self.corners_of(f, &mut scratch);
                for &v in &scratch {
                    if owner[v as usize] == NONE {
                        owner[v as usize] = i as u32;
                    }
                }
            }
        }
        // Fragments cluster through shared vertices: nodes are vertices,
        // then one per component.
        let mut cuf = Uf::new(nvs + comps.len());
        for (i, c) in comps.iter().enumerate() {
            if solid[i] {
                continue;
            }
            for &f in c {
                scratch.clear();
                self.corners_of(f, &mut scratch);
                for &v in &scratch {
                    cuf.join(v, (nvs + i) as u32);
                }
            }
        }
        let mut cluster_of_root: HashMap<u32, usize> = HashMap::new();
        let mut clusters: Vec<Vec<usize>> = Vec::new();
        for i in 0..comps.len() {
            if solid[i] {
                continue;
            }
            let r = cuf.find((nvs + i) as u32);
            let k = *cluster_of_root.entry(r).or_insert_with(|| {
                clusters.push(Vec::new());
                clusters.len() - 1
            });
            clusters[k].push(i);
        }
        // Pieces: the solids in component order, then the fragments that
        // joined none, in cluster order.
        let mut piece_of_comp = vec![usize::MAX; comps.len()];
        let mut pieces: Vec<Vec<u32>> = Vec::new();
        for (i, c) in comps.iter().enumerate() {
            if solid[i] {
                piece_of_comp[i] = pieces.len();
                pieces.push(c.clone());
            }
        }
        for members in &clusters {
            let fs: Vec<u32> = members.iter().flat_map(|&i| comps[i].iter().copied()).collect();
            let mut corners: Vec<u32> = Vec::new();
            for &f in &fs {
                self.corners_of(f, &mut corners);
            }
            corners.sort_unstable();
            corners.dedup();
            let mut votes: HashMap<u32, usize> = HashMap::new();
            for &v in &corners {
                let k = owner[v as usize];
                if k != NONE {
                    *votes.entry(k).or_insert(0) += 1;
                }
            }
            // The most votes; a tie goes to the first solid.
            let best = votes.iter().map(|(&k, &n)| (k, n)).max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)));
            match best {
                Some((k, n)) if n as f64 >= ATTACH_SHARE * corners.len() as f64 => {
                    pieces[piece_of_comp[k as usize]].extend(fs);
                }
                _ => pieces.push(fs),
            }
        }
        let mut face_piece = vec![usize::MAX; nfs];
        let mut vertex_piece = vec![usize::MAX; nvs];
        for (n, fs) in pieces.iter().enumerate() {
            for &f in fs {
                face_piece[f as usize] = n;
                scratch.clear();
                self.corners_of(f, &mut scratch);
                for &v in &scratch {
                    if vertex_piece[v as usize] == usize::MAX {
                        vertex_piece[v as usize] = n;
                    }
                }
            }
        }
        let mut out: Vec<Part> = pieces.into_iter().map(|faces| Part { faces, edges: Vec::new() }).collect();
        let mut wires = Vec::new();
        for &e in &edges {
            let fs = self.edge_faces(e);
            if !fs.is_empty() {
                let mut ns: Vec<usize> =
                    fs.iter().map(|&f| face_piece[f as usize]).filter(|&n| n != usize::MAX).collect();
                ns.sort_unstable();
                ns.dedup();
                for n in ns {
                    out[n].edges.push(e);
                }
                continue;
            }
            let [a, b] = *self.state.ends.get(e as usize);
            let n = if vertex_piece[a as usize] != usize::MAX { vertex_piece[a as usize] } else { vertex_piece[b as usize] };
            if n != usize::MAX {
                out[n].edges.push(e);
            } else {
                wires.push(e);
            }
        }
        if !wires.is_empty() {
            out.extend(self.connected_parts_of(&[], &wires).into_iter().map(|p| Part { faces: Vec::new(), edges: p.edges }));
        }
        sort_parts(&mut out);
        out
    }

    /// `copy_part(faces, edges)`: a new store in namespace `ns` holding a
    /// copy of the given faces and edges. `keep[f]`: which holes of a face
    /// with several survive `Mesh.add_face`'s nested-hole filter (absent:
    /// all); `face_attr`: per face, its row in the new mesh's attribute
    /// table (−1: none).
    pub fn extract(
        &self,
        ns: u64,
        faces: &[u32],
        edges: &[u32],
        keep: &HashMap<u32, Vec<bool>>,
        face_attr: &[i64],
        w: Weld,
    ) -> Result<Store, String> {
        if face_attr.len() != faces.len() {
            return Err("face_attr: one entry per face".into());
        }
        let st = &self.state;
        let position = |v: u32| *st.vpos.get(v as usize);
        let mut pos: Vec<[f64; 3]> = Vec::with_capacity(edges.len() * 2 + faces.len() * 4);
        for &e in edges {
            let [a, b] = *st.ends.get(e as usize);
            pos.push(position(a));
            pos.push(position(b));
        }
        let (mut rings, mut sizes, mut counts) = (Vec::new(), Vec::new(), Vec::new());
        for &f in faces {
            let all = self.face_rings(f);
            let mask = keep.get(&f);
            let mut n = 0;
            for (k, ring) in all.iter().enumerate() {
                if k > 0 && mask.is_some_and(|m| !m.get(k - 1).copied().unwrap_or(true)) {
                    continue;
                }
                sizes.push(ring.len() as i64);
                for &v in ring {
                    rings.push(pos.len() as i64);
                    pos.push(position(v));
                }
                n += 1;
            }
            counts.push(n);
        }
        // Where two copied edges weld onto one pair the last one's flags
        // win (`stamp_edge_flags` in input order): give every copy those.
        let (_v, ids, _o) = crate::weld::weld(&pos, w.scale, w.tol2);
        let mut last: HashMap<(i64, i64), usize> = HashMap::new();
        for i in 0..edges.len() {
            let (a, b) = (ids[2 * i], ids[2 * i + 1]);
            last.insert(if a <= b { (a, b) } else { (b, a) }, i);
        }
        let mut names: Vec<String> = Vec::new();
        let (mut eix, mut flags, mut curves, mut layers) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for i in 0..edges.len() {
            let (a, b) = (ids[2 * i], ids[2 * i + 1]);
            let j = last.get(&if a <= b { (a, b) } else { (b, a) }).copied().unwrap_or(i);
            let src = edges[j] as usize;
            eix.push([2 * i as i64, 2 * i as i64 + 1]);
            flags.push(*st.eflags.get(src) & (SOFT | HIDDEN | FLAT));
            curves.push(*st.ecurve.get(src));
            let l = *st.elayer.get(src);
            layers.push(if l == NONE {
                -1
            } else {
                let name = &st.layers[l as usize];
                match names.iter().position(|n| n == name) {
                    Some(k) => k as i64,
                    None => {
                        names.push(name.clone());
                        (names.len() - 1) as i64
                    }
                }
            });
        }
        let block = Block {
            pos: super::build::Points::Double(&pos),
            edges: &eix,
            flags: Some(&flags),
            curves: Some(&curves),
            layers: Some(&layers),
            layer_names: names,
            rings: &rings,
            sizes: &sizes,
            counts: &counts,
            face_attr: Some(face_attr),
            sequential: false,
        };
        Store::from_block(ns, &block, w)
    }
}

/// Per face with several holes (slots in `faces`), which ones
/// `Mesh.add_face`'s nested-hole filter keeps: `filter(holes)` is called
/// with the holes as position rings (`at` maps a vertex slot) and answers
/// a bool per hole (`built_core.store_mesh._hole_filter`).
pub(super) fn hole_masks(
    st: &Store,
    faces: &[u32],
    at: impl Fn(u32) -> [f64; 3],
    filter: &pyo3::Bound<'_, pyo3::PyAny>,
) -> pyo3::PyResult<HashMap<u32, Vec<bool>>> {
    use pyo3::prelude::*;
    let mut out = HashMap::new();
    for &f in faces {
        if st.state.frings.get(f as usize).n_rings <= 2 {
            continue;
        }
        let holes: Vec<Vec<(f64, f64, f64)>> = st.face_rings(f)[1..]
            .iter()
            .map(|r| r.iter().map(|&v| { let p = at(v); (p[0], p[1], p[2]) }).collect())
            .collect();
        let keep: Vec<bool> = filter.call1((holes.clone(),))?.extract()?;
        if keep.len() != holes.len() {
            return Err(pyo3::exceptions::PyValueError::new_err("hole_filter: one bool per hole"));
        }
        out.insert(f, keep);
    }
    Ok(out)
}

/// Parts as `(face keys, edge keys)`.
pub(super) fn keyed(st: &Store, parts: Vec<Part>) -> Vec<(Vec<u128>, Vec<u128>)> {
    parts
        .into_iter()
        .map(|p| {
            (p.faces.iter().map(|&f| st.key(Kind::Face, f)).collect(), p.edges.iter().map(|&e| st.key(Kind::Edge, e)).collect())
        })
        .collect()
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::store::edit::Change;

    const QUADS: [[usize; 4]; 6] = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];

    /// A store holding unit cubes at the given corners (faces added one
    /// by one, as `Mesh.add_face` would).
    pub fn cubes(at: &[[f64; 3]]) -> Store {
        let mut s = Store::empty(7, Weld::default());
        let mut ch = Change::default();
        for o in at {
            let c: Vec<[f64; 3]> = (0..8)
                .map(|i| [o[0] + (i & 1) as f64, o[1] + ((i >> 1) & 1) as f64, o[2] + ((i >> 2) & 1) as f64])
                .collect();
            for q in QUADS {
                s.add_face(&[q.iter().map(|&k| c[k]).collect()], &mut ch).unwrap();
            }
        }
        s
    }

    #[test]
    fn connected_and_solid_parts() {
        // Two cubes stacked (four shared corners), one apart, and a wire.
        let mut s = cubes(&[[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [5.0, 0.0, 0.0]]);
        let mut ch = Change::default();
        s.add_edge([9.0, 0.0, 0.0], [9.0, 1.0, 0.0], &mut ch).unwrap();
        let conn: Vec<(usize, usize)> = s.connected_parts().iter().map(|p| (p.faces.len(), p.edges.len())).collect();
        assert_eq!(conn, vec![(12, 20), (6, 12), (0, 1)]);
        let solid: Vec<(usize, usize)> =
            s.solid_parts(Hypot::LibStdCxx).iter().map(|p| (p.faces.len(), p.edges.len())).collect();
        assert_eq!(solid, vec![(6, 12), (6, 12), (6, 12), (0, 1)]);
    }

    #[test]
    fn extract_copies_one_piece_with_its_flags() {
        let mut s = cubes(&[[0.0, 0.0, 0.0], [5.0, 0.0, 0.0]]);
        let parts = s.solid_parts(Hypot::LibStdCxx);
        let e = parts[1].edges[0];
        s.set_edge_flags(e, SOFT);
        s.set_edge_layer(e, Some("T"));
        let p = &parts[1];
        let out = s.extract(9, &p.faces, &p.edges, &HashMap::new(), &vec![-1; p.faces.len()], Weld::default()).unwrap();
        assert_eq!((out.n(Kind::Vertex), out.n(Kind::Edge), out.n(Kind::Face)), (8, 12, 6));
        assert_eq!(*out.state.eflags.get(0), SOFT);
        assert_eq!(out.state.layers.as_ref(), &vec!["T".to_string()]);
        // The copy sits where the piece was.
        assert!(out.state.vpos.get(0)[0] >= 5.0);
        out.check().unwrap();
    }
}
