// SPDX-License-Identifier: GPL-3.0-or-later
//! Display of bound exact geometry. The segment count of a drawn circle no
//! longer decides how it looks: every bound edge is drawn along its curve
//! and every face with a curved support or curved sides is triangulated
//! from its support at the display step, with the support's own normals.
//! Each edge is sampled once and both faces along it use those very
//! samples, so neighbouring patches meet without cracks or T-junctions.
//! Edges between two faces of one support (a cylinder's generators, its
//! seam) draw soft.
//!
//! Face tessellations are cached by (support, boundary samples, step,
//! tolerance): the key holds everything the result depends on in
//! definition coordinates, so an edit invalidates exactly the faces whose
//! support or boundary it changed, and a placement never does.
use super::{keys::Kind, read::Dense, Store, SOFT};
use crate::exact::bound::{Support, Tessellation};
use crate::exact::Surface;
use glam::DVec3;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

/// Angular display step of curves and curved surfaces (radians).
pub(crate) const DISPLAY_STEP: f64 = std::f64::consts::PI / 60.;

/// Display arrays plus, per display face and edge, the logical dense index
/// it draws (-1: a synthetic interior side), and per ring corner the exact
/// surface normal (NaN where the face has none).
pub(crate) struct Display {
    pub dense: Dense,
    pub face_owner: Vec<i64>,
    pub edge_owner: Vec<i64>,
    pub corner_normals: Vec<[f64; 3]>,
}

struct Entry {
    support: Support,
    rings: Vec<Vec<DVec3>>,
    result: Option<Arc<Tessellation>>,
}

/// Process-wide tessellation cache, bounded; whole generations drop when
/// it is full (an entry is cheap to rebuild).
struct Cache {
    map: HashMap<u64, Entry>,
    hits: u64,
    misses: u64,
}
const CACHE_LIMIT: usize = 65_536;

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(Cache {
            map: HashMap::new(),
            hits: 0,
            misses: 0,
        })
    })
}

/// (hits, misses, entries) of the tessellation cache, for tests and timing.
pub(crate) fn cache_stats() -> (u64, u64, usize) {
    let c = cache().lock().unwrap();
    (c.hits, c.misses, c.map.len())
}

fn key(s: &Support, rings: &[Vec<DVec3>], tol: f64) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bincode::serialize(s).unwrap_or_default().hash(&mut h);
    DISPLAY_STEP.to_bits().hash(&mut h);
    tol.to_bits().hash(&mut h);
    for r in rings {
        r.len().hash(&mut h);
        for p in r {
            p.to_array().map(f64::to_bits).hash(&mut h);
        }
    }
    h.finish()
}

/// The cached tessellation of one trimmed region.
pub(crate) fn tessellation(s: &Support, rings: &[Vec<DVec3>], tol: f64) -> Option<Arc<Tessellation>> {
    let k = key(s, rings, tol);
    {
        let mut c = cache().lock().unwrap();
        if let Some(e) = c.map.get(&k) {
            if e.support == *s && e.rings == rings {
                let out = e.result.clone();
                c.hits += 1;
                return out;
            }
        }
        c.misses += 1;
    }
    let result = crate::exact::bound::tessellate(s, rings, DISPLAY_STEP, tol).map(Arc::new);
    let mut c = cache().lock().unwrap();
    if c.map.len() >= CACHE_LIMIT {
        c.map.clear();
    }
    c.map.insert(
        k,
        Entry {
            support: s.clone(),
            rings: rings.to_vec(),
            result: result.clone(),
        },
    );
    result
}

impl Store {
    /// Whether display needs the curved path at all.
    pub(crate) fn has_curved(&self) -> bool {
        self.state.exact.curved.is_some() && self.state.exact.native.is_none()
    }

    /// Faces of one support meet along these edges: draw them soft.
    pub(crate) fn same_support_edge(&self, e: u32) -> bool {
        let fs = self.edge_faces(e);
        if fs.len() != 2 {
            return false;
        }
        match (self.curved_face(fs[0]), self.curved_face(fs[1])) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }

    /// A definition-space direction in this store's frame.
    fn display_direction(&self, origin: DVec3, n: DVec3) -> DVec3 {
        let a = DVec3::from(self.model_point(origin.to_array()));
        let b = DVec3::from(self.model_point((origin + n).to_array()));
        (b - a).try_normalize().unwrap_or(n)
    }

    pub(crate) fn curved_display(&self, bake: bool) -> Display {
        let logical = self.dense();
        let [dv, de, df] = self.dense_maps();
        let st = &self.state;
        let mut out = Dense {
            pos: logical.pos.clone(),
            ..Dense::default()
        };
        let mut edge_owner = Vec::new();
        let mut corner_normals: Vec<[f64; 3]> = Vec::new();
        let to_display = |p: DVec3| self.model_point(p.to_array());
        // Sample every bound edge once, in its stored direction.
        let mut samples: HashMap<u32, Vec<(i64, DVec3)>> = HashMap::new();
        for e in st.order[Kind::Edge as usize].live(&st.slots[Kind::Edge as usize]) {
            let [a, b] = *st.ends.get(e as usize);
            let mut chain = vec![(dv[a as usize] as i64, self.def_point(a))];
            if let Some(bind) = self.curved_edge(e) {
                let n = bind.curve.samples(bind.interval, DISPLAY_STEP);
                for i in 1..n {
                    let t = bind.interval[0] + (bind.interval[1] - bind.interval[0]) * i as f64 / n as f64;
                    if let Ok(p) = bind.curve.point(t) {
                        chain.push((out.pos.len() as i64, p));
                        out.pos.push(to_display(p));
                    }
                }
            }
            chain.push((dv[b as usize] as i64, self.def_point(b)));
            let d = de[e as usize] as usize;
            // A straight generator between two faces of one support is part
            // of the surface: soft. A curve drawn on it (a line around a
            // cylinder) keeps its own flags, so it stays visible.
            let mut flags = logical.flags[d];
            if self.curved_edge(e).is_none() && self.same_support_edge(e) {
                flags |= SOFT;
            }
            for w in chain.windows(2) {
                out.edges.push([w[0].0, w[1].0]);
                out.flags.push(flags);
                out.layer.push(logical.layer[d]);
                out.curve.push(logical.curve[d]);
                edge_owner.push(d as i64);
            }
            if chain.len() > 2 {
                samples.insert(e, chain);
            }
        }
        let mut face_owner = Vec::new();
        let mut interior = Vec::new();
        let nan = [f64::NAN; 3];
        for f in st.order[Kind::Face as usize].live(&st.slots[Kind::Face as usize]) {
            let d = df[f as usize] as i64;
            let attr = logical.face_attr[d as usize];
            let sides = self.sides(f);
            let curved_sides = sides.iter().flatten().any(|s| s.edge.is_some_and(|e| samples.contains_key(&e)));
            // A planar face draws curved sides only when they lie in its
            // plane (a cap, a disk); a facet beside a curved support whose
            // side bends out of it stays the facet.
            let support = self.curved_face(f).cloned().or_else(|| {
                let plane = curved_sides.then(|| self.exact_face_plane(f)).flatten()?;
                let s = Support::new(Surface::Plane { frame: plane });
                let in_plane = sides.iter().flatten().all(|x| match x.edge.and_then(|e| self.curved_edge(e)) {
                    Some(b) => {
                        let tol = crate::exact::bound::tolerance(self.def_point(x.a).abs().max_element()) * 16.;
                        s.holds_curve(&b.curve, b.interval, tol)
                    }
                    None => true,
                });
                in_plane.then_some(s)
            });
            let refined = support.and_then(|s| {
                let mut rings = Vec::new();
                let mut ids = Vec::new();
                for ring in &sides {
                    let mut pts = Vec::new();
                    for side in ring {
                        match side.edge.and_then(|e| samples.get(&e).map(|c| (e, c))) {
                            Some((e, chain)) => {
                                let forward = st.ends.get(e as usize)[0] == side.a;
                                let n = chain.len();
                                for k in 0..n - 1 {
                                    let (id, p) = chain[if forward { k } else { n - 1 - k }];
                                    pts.push(p);
                                    ids.push(id);
                                }
                            }
                            None => {
                                pts.push(self.def_point(side.a));
                                ids.push(dv[side.a as usize] as i64);
                            }
                        }
                    }
                    rings.push(pts);
                }
                let tol = crate::exact::bound::tolerance(
                    rings.iter().flatten().fold(0., |m: f64, p| m.max(p.abs().max_element())),
                );
                let t = tessellation(&s, &rings, tol)?;
                for p in &t.points[ids.len()..] {
                    ids.push(out.pos.len() as i64);
                    out.pos.push(to_display(*p));
                }
                Some((ids, t))
            });
            match refined {
                Some((ids, t)) => {
                    // Orient like the store face (its winding in this frame).
                    let ring = &self.face_rings(f)[0];
                    let v = |i: i64| DVec3::from(out.pos[i as usize]);
                    let mut normal = DVec3::ZERO;
                    for k in 0..ring.len() {
                        let (a, b) = (v(dv[ring[k] as usize] as i64), v(dv[ring[(k + 1) % ring.len()] as usize] as i64));
                        normal += a.cross(b);
                    }
                    let mut sum = DVec3::ZERO;
                    for tri in &t.triangles {
                        let [a, b, c] = tri.map(|i| v(ids[i]));
                        sum += (b - a).cross(c - a);
                    }
                    let flip = sum.dot(normal) < 0.;
                    let sign = if flip { -1. } else { 1. };
                    for tri in &t.triangles {
                        let mut tri = *tri;
                        if flip {
                            tri.swap(1, 2);
                        }
                        out.corners.extend(tri.map(|i| ids[i]));
                        for i in tri {
                            let n = t.normals[i];
                            corner_normals.push(if n.is_finite() {
                                (self.display_direction(t.points[i], n) * sign).to_array()
                            } else {
                                nan
                            });
                        }
                        out.ring_counts.push(1);
                        out.ring_sizes.push(3);
                        out.face_attr.push(attr);
                        face_owner.push(d);
                        if bake {
                            interior.push(tri.map(|i| ids[i]));
                        }
                    }
                }
                None => {
                    let rings = self.face_rings(f);
                    out.ring_counts.push(rings.len() as i64);
                    for r in rings {
                        out.ring_sizes.push(r.len() as i64);
                        out.corners.extend(r.iter().map(|&v| dv[v as usize] as i64));
                        corner_normals.extend(std::iter::repeat(nan).take(r.len()));
                    }
                    out.face_attr.push(attr);
                    face_owner.push(d);
                }
            }
        }
        if bake {
            // Interior sides of a refined face are soft and suppressed (8):
            // they neither draw nor stop smoothing.
            let key = |a: i64, b: i64| (a.min(b), a.max(b));
            let mut sides: std::collections::HashSet<_> = out.edges.iter().map(|&[a, b]| key(a, b)).collect();
            for t in interior {
                for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                    if sides.insert(key(a, b)) {
                        out.edges.push([a, b]);
                        out.flags.push(9);
                        out.layer.push(-1);
                        out.curve.push(-1);
                        edge_owner.push(-1);
                    }
                }
            }
        }
        out.face_flags = vec![0; face_owner.len()];
        Display {
            dense: out,
            face_owner,
            edge_owner,
            corner_normals,
        }
    }
}
