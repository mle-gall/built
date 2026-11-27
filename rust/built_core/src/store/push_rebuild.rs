// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! The plane-rebuild fixpoint of a push in the store, with no Python
//! (`PushPullOperation._rebuild_planes_fixpoint` and `core/cap_rebuild.py`:
//! `RebuildCache`, `rebuild_plane`, `apply_rebuild`, `_FacesBefore`,
//! `seam_planes`, `crack_planes`, `_seam_planes_along`), then the trailing
//! connectivity stitch.
//!
//! It gives what the Python composition gave, to the bit: the same planes in
//! the same order with the same origins (the fresh faces iterate as the
//! Python set of views did, `crate::pyset`), the same parity votes (Python's
//! jitter stream, `orient::PyRandom`), the same caches with the same
//! staleness (the scope is memoised on the cache token only, as Python's).
//! Face paint stays a Python dict: the caller hands each face's paint class
//! in (`Paint`, 0 = no paint) and gets back the class each new face takes.
use super::{
    arrangement,
    cap_outline::{self, on_segment},
    edit::Change,
    fillet::maximal_holes,
    geometry::{normalized, plane_basis, Hypot, V3},
    keys::Kind,
    offset::contains,
    planar::{self, Metrics, Point, Ring},
    topo::OpError,
    Store, INTERIOR,
};
use crate::{orient::PyRandom, parity::Tris, pyset::PySet};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// `cap_rebuild._ON_PLANE`.
const ON_PLANE: f64 = 1e-4;
/// `cap_rebuild._SAMPLE_OFF`.
const SAMPLE_OFF: f64 = 1e-3;
/// `built_core.arrangement._TOL`.
const ARR_TOL: f64 = 1e-4;
/// `built_core.orient._RAYS`, `_JITTER`.
const RAYS: usize = 7;
const JITTER: f64 = 3e-4;
/// `cap_rebuild._STORE_HAIR_COS2`.
fn store_hair_cos2() -> f64 {
    let x = 1.0 - 2f64.powi(-25);
    x * x
}

type XY = (f64, f64);

/// Paint classes: equal dicts share a class, 0 is no paint (`{}`).
pub struct Paint {
    /// Faces with their own dict, by key.
    pub own: HashMap<u128, u32>,
    /// The class of each attribute-table row.
    pub rows: Vec<u32>,
    /// Faces this rebuild made and gave a donor's paint.
    made: HashMap<u128, u32>,
    order: Vec<u128>,
}

impl Paint {
    pub fn new(own: HashMap<u128, u32>, rows: Vec<u32>) -> Self {
        Self { own, rows, made: HashMap::new(), order: Vec::new() }
    }
    pub(super) fn class(&self, st: &Store, f: u32) -> u32 {
        let k = st.key(Kind::Face, f);
        if let Some(&c) = self.made.get(&k).or_else(|| self.own.get(&k)) {
            return c;
        }
        match *st.state.fattr.get(f as usize) {
            0 => 0,
            i => self.rows.get(i as usize - 1).copied().unwrap_or(0),
        }
    }
    fn give(&mut self, key: u128, class: u32) {
        if self.made.insert(key, class).is_none() {
            self.order.push(key);
        }
    }
}

/// How the fresh faces start: the faces carrying a seed (`{f for f in
/// mesh.faces if …}`: their keys in list order), or the faces a push made
/// (`set(set_minus(now, set(before)))`).
pub enum Fresh {
    Listed(Vec<u128>),
    Since { now: Vec<u128>, before: Vec<u128> },
}

/// What a rebuild left to the facade: the paint each new face takes (key,
/// class, in the order given), the trailing stitch's dedupe pairs, the
/// dirty bumps.
pub struct Rebuilt {
    pub painted: Vec<(u128, u32)>,
    pub deduped: Vec<(u128, u128)>,
    pub bumps: usize,
}

/// `RebuildCache`, store path: the memo fields Python keeps, with the same
/// keys (the token), so a stale scope is stale here too.
type Scope = (std::sync::Arc<[u32]>, std::sync::Arc<[u32]>);

#[derive(Default)]
struct Cache {
    token: i64,
    seeds: Vec<u128>,
    comp: Option<Vec<u32>>,
    comp_token: i64,
    scope: Option<Scope>,
    scope_token: i64,
    packed: Option<(i64, Option<Vec<u128>>, Option<BTreeSet<u32>>, Option<RayWindow>, Tris)>,
    dropped: Vec<[V3; 2]>,
    frames: HashMap<u128, (V3, V3)>,
}

/// A conservative envelope of all parity rays for an axis-aligned cap.
/// Geometry outside this slab cannot contribute a crossing, including on
/// the remote side of a closed component.
#[derive(Clone, Copy, PartialEq)]
struct RayWindow {
    axes: [usize; 2],
    lo: [f64; 2],
    hi: [f64; 2],
}

impl RayWindow {
    fn new(store: &Store, n: V3, probes: &[V3]) -> Option<Self> {
        if store.state.exact.native.is_some() || probes.is_empty() {
            return None;
        }
        let dir = arr(n);
        let axis = (0..3)
            .find(|&k| dir[k].abs() == 1. && dir[(k + 1) % 3] == 0. && dir[(k + 2) % 3] == 0.)?;
        let axes = [(axis + 1) % 3, (axis + 2) % 3];
        let mut magnitude = 0f64;
        for p in store.state.vpos.iter() {
            for x in p {
                if !x.is_finite() {
                    return None;
                }
                magnitude = magnitude.max(x.abs());
            }
        }
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for p in probes {
            let p = arr(*p);
            if !p.iter().all(|x| x.is_finite()) {
                return None;
            }
            magnitude = magnitude.max(p[axis].abs());
            for (i, k) in axes.into_iter().enumerate() {
                lo[i] = lo[i].min(p[k]);
                hi[i] = hi[i].max(p[k]);
            }
        }
        // The maximum travel along the plane normal is twice the largest
        // coordinate plus the sample offset. The two tangent slopes of a
        // jittered axis ray are at most JITTER; include the parity BVH's
        // coordinate/extent allowance as well.
        let pad = (2. * magnitude + SAMPLE_OFF) * JITTER + 1e-4 * (1. + magnitude);
        Some(Self {
            axes,
            lo: lo.map(|x| x - pad),
            hi: hi.map(|x| x + pad),
        })
    }

    fn intersects(&self, store: &Store, f: u32) -> bool {
        let st = &store.state;
        let rings = *st.frings.get(f as usize);
        let count: usize = (0..rings.n_rings)
            .map(|i| *st.ring_sizes.get((rings.ring_start + i) as usize) as usize)
            .sum();
        let mut sides = [false; 4];
        for i in rings.corner_start as usize..rings.corner_start as usize + count {
            let p = st.vpos.get(*st.corners.get(i) as usize);
            for (j, k) in self.axes.into_iter().enumerate() {
                sides[2 * j] |= p[k] >= self.lo[j];
                sides[2 * j + 1] |= p[k] <= self.hi[j];
            }
        }
        sides.into_iter().all(|yes| yes)
    }
}

struct Ctx<'a> {
    window_min_faces: usize,
    standalone: bool,
    h: Hypot,
    compensated: bool,
    keep: bool,
    removing: bool,
    metrics: Metrics<'static>,
    paint: &'a mut Paint,
}

fn v(p: [f64; 3]) -> V3 {
    V3(p[0], p[1], p[2])
}
fn arr(p: V3) -> [f64; 3] {
    [p.0, p.1, p.2]
}
fn dvec(p: V3) -> glam::DVec3 {
    glam::DVec3::new(p.0, p.1, p.2)
}

/// `plane_key`: the normal turned so its largest component is positive.
fn plane_key(point: V3, normal: V3, h: Hypot) -> ([u64; 4], V3, V3) {
    let mut n = normalized(normal, h);
    let c = [n.0, n.1, n.2];
    let mut axis = 0;
    for i in 1..3 {
        if c[i].abs() > c[axis].abs() {
            axis = i;
        }
    }
    if c[axis] < 0. {
        n = V3(-n.0, -n.1, -n.2);
    }
    let d = n.dot(point, h);
    (round_key([n.0, n.1, n.2, d]), point, n)
}

/// Python's `round(x, 3)` of each, as ordered bits.
fn round_key(xs: [f64; 4]) -> [u64; 4] {
    xs.map(|x| {
        let x = format!("{x:.3}").parse::<f64>().unwrap();
        let bits = if x == 0. { 0 } else { x.to_bits() };
        if bits >> 63 == 1 {
            !bits
        } else {
            bits ^ (1u64 << 63)
        }
    })
}

/// An ordered map that keeps first-insertion order (a Python dict).
struct Seen<V> {
    order: Vec<[u64; 4]>,
    map: HashMap<[u64; 4], V>,
}
impl<V> Seen<V> {
    fn new() -> Self {
        Self { order: Vec::new(), map: HashMap::new() }
    }
    fn set(&mut self, k: [u64; 4], val: V) {
        if self.map.insert(k, val).is_none() {
            self.order.push(k);
        }
    }
    fn setdefault(&mut self, k: [u64; 4], val: V) {
        if !self.map.contains_key(&k) {
            self.set(k, val);
        }
    }
    fn values(self) -> Vec<V> {
        let mut map = self.map;
        self.order.into_iter().map(|k| map.remove(&k).unwrap()).collect()
    }
}

/// `_canon_loop` / `plane_cycle`: the 1e-4 cells, from the smallest.
fn cycle(pts: &[[f64; 3]]) -> Vec<[i128; 3]> {
    let keys: Vec<[f64; 3]> = pts.iter().map(|p| p.map(|x| (x / 1e-4).round_ties_even())).collect();
    let mut start = 0;
    for i in 1..keys.len() {
        if keys[i].partial_cmp(&keys[start]) == Some(std::cmp::Ordering::Less) {
            start = i;
        }
    }
    let mut out: Vec<[i128; 3]> = keys.iter().map(|k| k.map(|x| x as i128)).collect();
    out.rotate_left(start);
    out
}

/// `_face_shape`'s loop: either winding.
fn either_way(pts: &[[f64; 3]]) -> Vec<[i128; 3]> {
    let a = cycle(pts);
    let mut rev = pts.to_vec();
    rev.reverse();
    let b = cycle(&rev);
    a.min(b)
}

type Shape = (Vec<[i128; 3]>, BTreeSet<Vec<[i128; 3]>>, bool);

impl Store {
    fn ring_positions(&self, ring: &[u32]) -> Vec<[f64; 3]> {
        ring.iter().map(|&v| *self.state.vpos.get(v as usize)).collect()
    }

    fn shape(&self, f: u32) -> Shape {
        let rings = self.face_rings(f);
        let outer = rings.first().map(|r| either_way(&self.ring_positions(r))).unwrap_or_default();
        let holes = rings[1.min(rings.len())..].iter().map(|r| either_way(&self.ring_positions(r))).collect();
        (outer, holes, self.state.fflags.get(f as usize) & INTERIOR != 0)
    }

    /// `(cache.centroid(f), cache.normal(f))`.
    fn frame_of(&self, f: u32, cache: &mut Cache, ctx: &Ctx<'_>) -> (V3, V3) {
        let k = self.key(Kind::Face, f);
        if let Some(&fr) = cache.frames.get(&k) {
            return fr;
        }
        let rings = self.face_rings(f);
        let outer = rings.first().map(Vec::as_slice).unwrap_or_default();
        let (c, n, _) = self.cap_frame(outer, ctx.h, ctx.compensated);
        let fr = (c, normalized(n, ctx.h));
        cache.frames.insert(k, fr);
        fr
    }

    fn coplanar_on(&self, f: u32, o: V3, n: V3, cache: &mut Cache, ctx: &Ctx<'_>) -> bool {
        if !self.may_meet_cap(f, o, n, ON_PLANE) { return false; }
        let (c, fnrm) = self.frame_of(f, cache, ctx);
        if fnrm.dot(n, ctx.h).abs() <= 0.999 {
            return false;
        }
        c.sub(o).dot(n, ctx.h).abs() < ON_PLANE
    }

    fn live_face(&self, k: u128) -> Option<u32> {
        self.slot(Kind::Face, k).filter(|&f| self.alive(Kind::Face, f))
    }

    // ---- RebuildCache ------------------------------------------------------

    fn components(&self, cache: &mut Cache) {
        if cache.comp.is_some() && cache.comp_token == cache.token {
            return;
        }
        let mut edges = self.live_slots(Kind::Edge);
        let comp = self.cap_labels(&edges);
        let wanted = self.cap_members(&cache.seeds,&comp);
        cache.scope = wanted.iter().any(|w| *w).then(|| {
            let mut faces = self.live_slots(Kind::Face);
            faces.retain(|&f| wanted[comp[f as usize] as usize]);
            edges.retain(|&e| self.scoped_edge(e,&comp,&wanted));
            (faces.into(),edges.into())
        });
        cache.scope_token = cache.token;
        cache.comp = Some(comp);
        cache.comp_token = cache.token;
    }

    /// `cache.scope(mesh)`: `None` for the whole mesh.
    fn scope(&self, cache: &mut Cache) -> Option<Scope> {
        if cache.seeds.is_empty() {
            return None;
        }
        if cache.scope_token != cache.token {
            self.components(cache);
        }
        cache.scope.clone()
    }

    /// `cache.packed(mesh, shadowed, want)`.
    fn packed(&self, cache: &mut Cache, shadowed: &HashSet<u32>, want: Option<BTreeSet<u32>>, window: Option<RayWindow>, h: Hypot) {
        let skey = if shadowed.is_empty() {
            None
        } else {
            let mut ks: Vec<u128> = shadowed.iter().map(|&f| self.key(Kind::Face, f)).collect();
            ks.sort_unstable();
            Some(ks)
        };
        if let Some((tok, k, w, old_window, _)) = &cache.packed {
            if *tok == cache.token && *k == skey && *w == want && *old_window == window {
                return;
            }
        }
        if want.is_some() {
            self.components(cache);
        }
        let comp = cache.comp.as_ref();
        let faces: Vec<u32> = self
            .live_slots(Kind::Face)
            .into_iter()
            .filter(|&f| window.is_none_or(|w| w.intersects(self, f)) && !shadowed.contains(&f))
            .filter(|&f| match &want {
                None => true,
                Some(w) => comp
                    .and_then(|c| c.get(f as usize))
                    .is_some_and(|c| w.contains(c)),
            })
            .collect();
        let s = self.indexed_surface(&faces, h);
        let (mut a, mut e1, mut e2) = (Vec::new(), Vec::new(), Vec::new());
        for t in &s.triangles {
            let [p, q, r] = t.map(|i| s.positions[i as usize]);
            a.push(p);
            e1.push([q[0] - p[0], q[1] - p[1], q[2] - p[2]]);
            e2.push([r[0] - p[0], r[1] - p[1], r[2] - p[2]]);
        }
        let owners = s.owners.iter().map(|&o| o as i64).collect();
        cache.packed = Some((cache.token, skey, want, window, Tris::new(a, e1, e2, owners)));
    }

    // ---- rebuild_plane -----------------------------------------------------

    #[allow(clippy::type_complexity)]
    fn rebuild_cap_plane(
        &self,
        origin: V3,
        normal: V3,
        fresh: &PySet,
        op: &mut Vec<[V3; 2]>,
        cache: &mut Cache,
        ctx: &mut Ctx<'_>,
    ) -> Result<Option<Vec<(Vec<V3>, Vec<Vec<V3>>, bool)>>, OpError> {
        let h = ctx.h;
        let n = normalized(normal, h);
        let (u, w) = plane_basis(n, h);
        let (scope_faces, scope_edges) = match self.scope(cache) {
            Some(pair) => pair,
            None => (self.live_slots(Kind::Face).into(), self.live_slots(Kind::Edge).into()),
        };
        let to2d = |p: V3| {
            let d = p.sub(origin);
            [d.dot(u, h), d.dot(w, h)]
        };
        let plane_edges: Vec<u32> = scope_edges
            .iter().copied()
            .filter(|&e| self.edge_on_cap_plane(e, origin, n, ON_PLANE, h))
            .collect();
        if plane_edges.len() < 3 {
            return Ok(None);
        }
        let ends = |e: u32| self.state.ends.get(e as usize).map(|x| v(*self.state.vpos.get(x as usize)));
        // planar_arrangement: projected, the zero-length dropped, arranged,
        // lifted back (and projected again below, as Python does).
        let lines: Vec<[Point; 2]> = plane_edges
            .iter()
            .map(|&e| ends(e).map(to2d))
            .filter(|[a, b]| {
                let (dx, dy) = (a[0] - b[0], a[1] - b[1]);
                dx * dx + dy * dy > ARR_TOL * ARR_TOL
            })
            .collect();
        let lift = |p: Point| origin.add(u.mul(p[0])).add(w.mul(p[1]));
        let (_, faces2d) = arrangement::arrange(&lines, false, &ctx.metrics).map_err(|_| OpError::Fallback("plane not arranged"))?;
        if faces2d.is_empty() {
            return Ok(None);
        }
        let regions: Vec<(Ring, Vec<Ring>)> = faces2d
            .into_iter()
            .map(|(r, hs)| {
                let again = |r: Ring| r.into_iter().map(|p| to2d(lift(p))).collect::<Ring>();
                (again(r), hs.into_iter().map(again).collect())
            })
            .collect();
        let fresh_set: Vec<u32> = fresh.iter().filter_map(|k| self.live_face(k)).collect();
        let fresh_slots: HashSet<u32> = fresh_set.iter().copied().collect();
        let shadowed = if ctx.keep { self.shadowed(&fresh_set, &scope_faces, cache, ctx) } else { HashSet::new() };
        let mut probes = Vec::with_capacity(regions.len());
        for (outer, holes) in &regions {
            probes.push(planar::region_point(&ctx.metrics, outer, holes).map_err(|_| OpError::Fallback("no region point"))?);
        }
        let probe_points: Vec<_> = probes.iter().copied().map(lift).collect();
        let window = if self.state.slots[2].live >= ctx.window_min_faces {
            RayWindow::new(self, n, &probe_points)
        } else { None };
        if ctx.standalone {
            self.packed(cache, &shadowed, None, window, h);
        } else {
            self.components(cache);
            let mut want = BTreeSet::new();
            for &f in &fresh_set {
                if let Some(&c) = cache.comp.as_ref().and_then(|c| c.get(f as usize)) {
                    want.insert(c);
                }
            }
            for &f in scope_faces.iter() {
                let c = cache.comp.as_ref().and_then(|c| c.get(f as usize).copied());
                if let Some(c) = c {
                    if !want.contains(&c) && self.coplanar_on(f, origin, n, cache, ctx) {
                        want.insert(c);
                    }
                }
            }
            self.packed(cache, &shadowed, (!want.is_empty()).then_some(want), window, h);
        }
        let probe_xy: Vec<XY> = probes.iter().map(|p| (p[0], p[1])).collect();
        let coverage = self.cap_coverage(
            &scope_faces,
            &fresh_set,
            arr(origin),
            arr(n),
            &probe_xy,
            ON_PLANE,
            h,
            ctx.compensated,
        );
        let tris = &cache.packed.as_ref().unwrap().4;
        let mut rng = PyRandom::new(54321);
        let mut groups: [Vec<(Ring, Vec<Ring>)>; 4] = Default::default();
        for (((outer, holes), probe), (old, skin, new)) in regions.into_iter().zip(probes).zip(coverage) {
            let p = lift(probe);
            let plus = occupied(p.add(n.mul(SAMPLE_OFF)), n, &mut rng, tris, h);
            let minus = occupied(p.sub(n.mul(SAMPLE_OFF)), V3(-n.0, -n.1, -n.2), &mut rng, tris, h);
            match surface(plus, minus, old, skin, new, ctx.keep, ctx.removing) {
                Surface::Boundary(true) => groups[0].push((outer, holes)),
                Surface::Boundary(false) => groups[1].push((outer, holes)),
                Surface::Partition => groups[2].push((outer, holes)),
                Surface::Sheet => groups[3].push((outer, holes)),
                Surface::Discard => {}
                Surface::InteriorSeam => {
                    for ring in std::iter::once(&outer).chain(&holes) {
                        for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                            let seg = [lift(a), lift(b)];
                            op.push(seg);
                            cache.dropped.push(seg);
                        }
                    }
                }
            }
        }
        let seams = self.creases(origin, n, (u, w), &plane_edges, op, &fresh_slots, cache, ctx);
        let mut result = Vec::new();
        for (group, regions) in groups.into_iter().enumerate() {
            if regions.is_empty() {
                continue;
            }
            let loops = cap_outline::trace_native(regions, &seams, ARR_TOL).ok_or(OpError::Fallback("plane outline not traced"))?;
            let nested = planar::nesting(&ctx.metrics, &loops, false).map_err(|_| OpError::Fallback("plane outline not nested"))?;
            for (i, hs) in nested {
                let ring = |r: &Ring| {
                    let mut out: Vec<V3> = r.iter().map(|&p| lift(p)).collect();
                    if group == 0 {
                        out.reverse();
                    }
                    out
                };
                result.push((ring(&loops[i]), hs.iter().map(|&j| ring(&loops[j])).collect(), group == 2));
            }
        }
        Ok(Some(result))
    }

    /// `shadowed_push_faces`.
    fn shadowed(&self, fresh: &[u32], scope: &[u32], cache: &mut Cache, ctx: &Ctx<'_>) -> HashSet<u32> {
        let h = ctx.h;
        let fresh_set: HashSet<u32> = fresh.iter().copied().collect();
        let mut out = HashSet::new();
        for &f in fresh {
            if self.state.fflags.get(f as usize) & INTERIOR != 0 {
                continue;
            }
            let (o, n) = self.frame_of(f, cache, ctx);
            let (u, w) = plane_basis(n, h);
            for &g in scope {
                if fresh_set.contains(&g) || !self.may_meet_cap(g, o, n, ON_PLANE) {
                    continue;
                }
                let (gc, gn) = self.frame_of(g, cache, ctx);
                if gn.dot(n, h).abs() < 0.999 || gc.sub(o).dot(n, h).abs() > ON_PLANE {
                    continue;
                }
                let rings = self.face_rings(g);
                let proj = |r: &Vec<u32>| -> Vec<XY> {
                    r.iter()
                        .map(|&x| {
                            let d = v(*self.state.vpos.get(x as usize)).sub(o);
                            (d.dot(u, h), d.dot(w, h))
                        })
                        .collect()
                };
                let rim = proj(&rings[0]);
                if contains(&rim, (0., 0.)) && !rings[1..].iter().any(|r| contains(&proj(r), (0., 0.))) {
                    out.insert(f);
                    break;
                }
            }
        }
        out
    }

    /// `rebuild_region_faces`' creases: plane edges a perpendicular face
    /// stands on, edges no push rim lies along, and edges between paints.
    #[allow(clippy::too_many_arguments)]
    fn creases(
        &self,
        o: V3,
        n: V3,
        (u, w): (V3, V3),
        edges: &[u32],
        op: &[[V3; 2]],
        _fresh: &HashSet<u32>,
        cache: &mut Cache,
        ctx: &Ctx<'_>,
    ) -> Vec<[Point; 2]> {
        let h = ctx.h;
        let project = |p: V3| {
            let d = p.sub(o);
            [d.dot(u, h), d.dot(w, h)]
        };
        let rim: Vec<[Point; 2]> = op
            .iter()
            .filter(|s| s.iter().all(|p| p.sub(o).dot(n, h).abs() < ON_PLANE))
            .map(|s| [project(s[0]), project(s[1])])
            .collect();
        let mut out = Vec::new();
        for &e in edges {
            let fs = self.edge_faces(e).to_vec();
            if fs.is_empty() {
                continue;
            }
            let crease = fs.iter().any(|&f| self.frame_of(f, cache, ctx).1.dot(n, h).abs() < 0.999);
            let [a, b] = self.state.ends.get(e as usize).map(|x| project(v(*self.state.vpos.get(x as usize))));
            let along = rim.iter().any(|&s| [a, b].iter().all(|&p| on_segment(p, s, ARR_TOL / 2.)));
            let first = ctx.paint.class(self, fs[0]);
            let paints = fs[1..].iter().any(|&f| ctx.paint.class(self, f) != first);
            if crease || (!ctx.standalone && (!along || paints)) {
                out.push([a, b]);
            }
        }
        out
    }

    // ---- apply_rebuild -----------------------------------------------------

    fn apply_cap_rebuild(
        &mut self,
        origin: V3,
        normal: V3,
        fresh: &PySet,
        op: &mut Vec<[V3; 2]>,
        cache: &mut Cache,
        ctx: &mut Ctx<'_>,
        ch: &mut Change,
        bumps: &mut usize,
    ) -> Result<bool, OpError> {
        let h = ctx.h;
        let n = normalized(normal, h);
        let Some(rebuilt) = self.rebuild_cap_plane(origin, n, fresh, op, cache, ctx)? else {
            return Ok(false);
        };
        let scope_faces = match self.scope(cache) {
            Some((f, _)) => f,
            None => self.live_slots(Kind::Face).into(),
        };
        let old: Vec<u32> = scope_faces.iter().copied().filter(|&f| self.coplanar_on(f, origin, n, cache, ctx)).collect();
        let print_new: BTreeSet<_> = rebuilt
            .iter()
            .map(|(outer, holes, part)| {
                let c = |r: &Vec<V3>| cycle(&r.iter().map(|&p| arr(p)).collect::<Vec<_>>());
                (c(outer), holes.iter().map(c).collect::<BTreeSet<_>>(), *part)
            })
            .collect();
        let print_old: BTreeSet<_> = old
            .iter()
            .map(|&f| {
                let rings = self.face_rings(f);
                let c = |r: &Vec<u32>| cycle(&self.ring_positions(r));
                (c(&rings[0]), rings[1..].iter().map(c).collect::<BTreeSet<_>>(), self.state.fflags.get(f as usize) & INTERIOR != 0)
            })
            .collect();
        if print_new == print_old {
            return Ok(false);
        }
        cache.token += 1;
        let (u, w) = plane_basis(n, h);
        let project = |p: V3| -> XY {
            let d = p.sub(origin);
            (d.dot(u, h), d.dot(w, h))
        };
        let mut donors = Vec::new();
        for &f in &old {
            let class = ctx.paint.class(self, f);
            if class != 0 {
                let rings = self.face_rings(f);
                let proj = |r: &Vec<u32>| -> Vec<XY> { r.iter().map(|&x| project(v(*self.state.vpos.get(x as usize)))).collect() };
                donors.push((proj(&rings[0]), rings[1..].iter().map(proj).collect::<Vec<_>>(), class));
            }
        }
        let caps: Vec<super::caps::Cap> = rebuilt
            .iter()
            .map(|(outer, holes, interior)| {
                let keep = if holes.len() > 1 { maximal_holes(holes, h) } else { vec![true; holes.len()] };
                let mut rings = vec![outer.iter().map(|&p| arr(p)).collect::<Vec<_>>()];
                rings.extend(holes.iter().zip(keep).filter(|(_, k)| *k).map(|(r, _)| r.iter().map(|&p| arr(p)).collect()));
                (rings, *interior)
            })
            .collect();
        let keys = self.replace_caps(&old, &caps, ch)?;
        *bumps += 1;
        if !donors.is_empty() {
            for ((outer, holes, _), &key) in rebuilt.iter().zip(&keys) {
                let rim: Ring = outer.iter().map(|&p| { let q = project(p); [q.0, q.1] }).collect();
                let openings: Vec<Ring> = holes.iter().map(|r| r.iter().map(|&p| { let q = project(p); [q.0, q.1] }).collect()).collect();
                let p = planar::region_point(&ctx.metrics, &rim, &openings).map_err(|_| OpError::Fallback("no region point"))?;
                let pt = (p[0], p[1]);
                if let Some((_, _, class)) = donors
                    .iter()
                    .find(|(r, hs, _)| contains(r, pt) && !hs.iter().any(|hole| contains(hole, pt)))
                {
                    ctx.paint.give(key, *class);
                }
            }
        }
        let before = ch.removed.iter().map(Vec::len).sum::<usize>();
        self.prune_plane_debris(arr(origin), arr(n), ON_PLANE, h, ch)?;
        if ch.removed.iter().map(Vec::len).sum::<usize>() > before {
            *bumps += 1;
        }
        Ok(true)
    }

    // ---- the fixpoint ------------------------------------------------------

    fn seam_candidates(&self, fresh: &PySet, ctx: &Ctx<'_>, out: &mut Vec<(V3, V3)>) {
        let h = ctx.h;
        let faces: Vec<u32> = fresh.iter().filter_map(|k| self.slot(Kind::Face, k)).collect();
        let rows = self.seam_planes(&faces, ON_PLANE / 2., store_hair_cos2(), h, ctx.compensated);
        let mut seen: Seen<(bool, V3, V3)> = Seen::new();
        for (point, normal, correction) in rows {
            let (key, origin, cn) = plane_key(v(point), v(normal), h);
            let corrected = seen.map.get(&key).is_some_and(|e| e.0);
            match correction {
                Some(c) if !corrected => {
                    let (_, _, cn) = plane_key(origin, v(c), h);
                    seen.set(key, (true, origin, cn));
                }
                _ => seen.setdefault(key, (false, origin, cn)),
            }
        }
        out.extend(seen.values().into_iter().map(|(_, o, n)| (o, n)));
    }

    fn window(&self, fresh: &PySet) -> Option<[f64; 6]> {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        let mut any = false;
        for k in fresh.iter() {
            let Some(f) = self.slot(Kind::Face, k) else { continue };
            for ring in self.face_rings(f) {
                for p in self.ring_positions(&ring) {
                    any = true;
                    for i in 0..3 {
                        lo[i] = lo[i].min(p[i]);
                        hi[i] = hi[i].max(p[i]);
                    }
                }
            }
        }
        if !any {
            return None;
        }
        let big = lo.iter().chain(&hi).fold(0f64, |m, c| m.max(c.abs()));
        let m = 0.01 + 1e-6 * big;
        Some([lo[0] - m, lo[1] - m, lo[2] - m, hi[0] + m, hi[1] + m, hi[2] + m])
    }

    fn crack_candidates(&self, fresh: &PySet, ctx: &Ctx<'_>, out: &mut Vec<(V3, V3)>) {
        let mut seen: Seen<(V3, V3)> = Seen::new();
        for (p, n) in self.crack_planes(self.window(fresh), ctx.h, ctx.compensated) {
            let (key, origin, normal) = plane_key(v(p), v(n), ctx.h);
            seen.setdefault(key, (origin, normal));
        }
        out.extend(seen.values());
    }

    /// `_seam_planes_along(mesh, cache.dropped)`.
    fn dropped_candidates(&self, cache: &mut Cache, ctx: &Ctx<'_>, out: &mut Vec<(V3, V3)>) {
        if cache.dropped.is_empty() {
            return;
        }
        let h = ctx.h;
        let segments: Vec<[[f64; 3]; 2]> = cache.dropped.iter().map(|s| [arr(s[0]), arr(s[1])]).collect();
        for k in self.edges_along(&segments, ON_PLANE, h) {
            let Some(e) = self.slot(Kind::Edge, k) else { continue };
            let &[fa, fb] = self.edge_faces(e) else { continue };
            let interior = |f: u32| self.state.fflags.get(f as usize) & INTERIOR != 0;
            if interior(fa) || interior(fb) || ctx.paint.class(self, fa) != ctx.paint.class(self, fb) {
                continue;
            }
            let rings = |f: u32| self.face_rings(f).into_iter().next().unwrap_or_default();
            let (ca, na, _) = self.cap_frame(&rings(fa), h, ctx.compensated);
            let (_, nb, _) = self.cap_frame(&rings(fb), h, ctx.compensated);
            if normalized(na, h).dot(normalized(nb, h), h) < 0.9999 {
                continue;
            }
            out.push((ca, na));
        }
    }

    /// `_FacesBefore.kept`: a face of before with the new face's shape,
    /// not fresh.
    fn kept_before(before: &Store, shape: &Shape, corners: &[[f64; 3]], fresh: &PySet) -> bool {
        // An equal shape has its corners in the new face's cells: its first
        // corner is within a cell of the new face's box.
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for p in corners {
            for i in 0..3 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
        let big = lo.iter().chain(&hi).fold(0f64, |m, c| m.max(c.abs()));
        let m = 2e-4 + 1e-6 * big;
        before.live_slots(Kind::Face).into_iter().any(|g| {
            let r = *before.state.frings.get(g as usize);
            if r.n_rings == 0 || *before.state.ring_sizes.get(r.ring_start as usize) as usize != corners.len() {
                return false;
            }
            let first = *before.state.vpos.get(*before.state.corners.get(r.corner_start as usize) as usize);
            (0..3).all(|i| first[i] >= lo[i] - m && first[i] <= hi[i] + m)
                && !fresh.contains(before.key(Kind::Face, g))
                && &before.shape(g) == shape
        })
    }

    /// The push's plane rebuild to a fixpoint, then the trailing stitch.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn single_plane(&self, origin: [f64;3], normal: [f64;3], h: Hypot, compensated: bool)
        -> Result<Option<Vec<super::caps::Cap>>, OpError> {
        let mut paint = Paint::new(HashMap::new(), Vec::new());
        let mut ctx = Ctx { window_min_faces: 8192, standalone: true, h, compensated, keep: false, removing: true, metrics: Metrics::native(compensated), paint: &mut paint };
        let plan = self.rebuild_cap_plane(v(origin), v(normal), &PySet::from_keys([]), &mut Vec::new(), &mut Cache::default(), &mut ctx)?;
        Ok(plan.map(|caps| caps.into_iter().map(|(outer, holes, interior)| {
            let rings = std::iter::once(outer).chain(holes).map(|r| r.into_iter().map(arr).collect()).collect();
            (rings, interior)
        }).collect()))
    }

    pub(super) fn rebuild_single_plane(&mut self, origin: [f64;3], normal: [f64;3], paint: &mut Paint, h: Hypot, compensated: bool, ch: &mut Change)
        -> Result<(usize, Vec<(u128, u32)>), OpError> {
        let mut ctx = Ctx { window_min_faces: 8192, standalone: true, h, compensated, keep: false, removing: true, metrics: Metrics::native(compensated), paint };
        let mut bumps = 0;
        self.apply_cap_rebuild(v(origin),v(normal),&PySet::from_keys([]),&mut Vec::new(),&mut Cache::default(),&mut ctx,ch,&mut bumps)?;
        let painted = ctx.paint.order.iter().map(|k| (*k, ctx.paint.made[k])).collect();
        Ok((bumps, painted))
    }

    pub fn rebuild_push_planes(
        &mut self,
        fresh: Fresh,
        seeds: &[[f64; 3]],
        keep: bool,
        removing: bool,
        paint: &mut Paint,
        compensated: bool,
        h: Hypot,
        ch: &mut Change,
    ) -> Result<Rebuilt, OpError> {
        let mut fresh: PySet = match fresh {
            Fresh::Listed(keys) => PySet::from_keys(keys),
            Fresh::Since { now, before } => {
                let before: HashSet<u128> = before.into_iter().collect();
                PySet::from_keys(now).difference(before.len(), |k| before.contains(&k)).copy()
            }
        };
        // The push's own rims, by position, before any rebuild.
        let mut op: Vec<[V3; 2]> = Vec::new();
        for k in fresh.iter() {
            let Some(f) = self.slot(Kind::Face, k) else { continue };
            for ring in self.face_rings(f) {
                let pts = self.ring_positions(&ring);
                for i in 0..pts.len() {
                    op.push([v(pts[i]), v(pts[(i + 1) % pts.len()])]);
                }
            }
        }
        let mut cache = Cache { comp_token: -1, scope_token: -1, ..Cache::default() };
        let mut ctx = Ctx { window_min_faces: 8192, standalone: false, h, compensated, keep, removing, metrics: Metrics::native(compensated), paint };
        let mut bumps = 0;
        for round in 0..4 {
            if round > 0 {
                fresh = PySet::from_keys(fresh.iter().filter(|&k| self.live_face(k).is_some()).collect::<Vec<_>>());
            }
            cache.seeds = fresh.iter().collect();
            let mut candidates = Vec::new();
            self.seam_candidates(&fresh, &ctx, &mut candidates);
            self.crack_candidates(&fresh, &ctx, &mut candidates);
            self.dropped_candidates(&mut cache, &ctx, &mut candidates);
            cache.dropped.clear();
            // rebuild_push_planes: one representative per canonical plane,
            // in key order.
            let mut unique: BTreeMap<[u64; 4], (V3, V3)> = BTreeMap::new();
            for (origin, normal) in candidates {
                let mut n = dvec(normal).normalize_or_zero();
                let mut axis = 0;
                for i in 1..3 {
                    if n[i].abs() > n[axis].abs() {
                        axis = i;
                    }
                }
                if n[axis] < 0. {
                    n = -n;
                }
                let offset = n.dot(dvec(origin));
                unique.entry(round_key([n.x, n.y, n.z, offset])).or_insert((origin, normal));
            }
            let mut changed = false;
            for (origin, normal) in unique.into_values() {
                let before_state = self.state.clone();
                let did = self.apply_cap_rebuild(origin, normal, &fresh, &mut op, &mut cache, &mut ctx, ch, &mut bumps)?;
                if did {
                    let now: Vec<u128> = self.live_slots(Kind::Face).into_iter().map(|f| self.key(Kind::Face, f)).collect();
                    let mut before = Store::empty(self.state.ns, Default::default());
                    before.state = before_state;
                    let new = PySet::from_keys(now).difference(before.state.slots[2].live, |k| before.live_face(k).is_some());
                    let mut add = Vec::new();
                    for k in new.iter() {
                        let Some(f) = self.slot(Kind::Face, k) else { continue };
                        let shape = self.shape(f);
                        let corners = self.face_rings(f).first().map(|r| self.ring_positions(r)).unwrap_or_default();
                        if !Self::kept_before(&before, &shape, &corners, &fresh) {
                            add.push(k);
                        }
                    }
                    fresh.merge(&PySet::from_keys(add));
                }
                changed |= did;
            }
            if !changed {
                break;
            }
        }
        let stitched = self.stitch_connect(seeds, true, 1e-4, h, ch)?;
        bumps += stitched.bumps;
        let painted = ctx.paint.order.iter().map(|k| (*k, ctx.paint.made[k])).collect();
        let deduped = stitched.deduped.iter().map(|&(d, k)| (self.key(Kind::Face, d), self.key(Kind::Face, k))).collect();
        Ok(Rebuilt { painted, deduped, bumps })
    }
}

/// `ray_parity_outside(...) is False`: most of seven jittered rays cross
/// the counted triangles an odd number of times.
fn occupied(origin: V3, direction: V3, rng: &mut PyRandom, tris: &Tris, h: Hypot) -> bool {
    let n = dvec(normalized(direction, h));
    let mut r = glam::DVec3::new(1., 0., 0.);
    if r.dot(n).abs() > 0.9 {
        r = glam::DVec3::new(0., 1., 0.);
    }
    let u = n.cross(r).normalize_or_zero();
    let w = n.cross(u).normalize_or_zero();
    let mut dirs = Vec::with_capacity(RAYS);
    for i in 0..RAYS {
        if i == 0 {
            dirs.push(n.to_array());
            continue;
        }
        let a = -JITTER + (JITTER - -JITTER) * rng.random();
        let b = -JITTER + (JITTER - -JITTER) * rng.random();
        dirs.push((n + u * a + w * b).normalize_or_zero().to_array());
    }
    let counts = tris.crossings(arr(origin), &dirs, None, None);
    let inside = counts.iter().filter(|&&c| c % 2 != 0).count();
    let outside = counts.len() - inside;
    inside > outside
}

enum Surface {
    Boundary(bool),
    Partition,
    Sheet,
    InteriorSeam,
    Discard,
}

/// `plane_regions::surface`.
fn surface(plus: bool, minus: bool, old: bool, skin: u8, fresh: u8, keep: bool, removing: bool) -> Surface {
    if plus != minus {
        return Surface::Boundary(plus);
    }
    let decl = if keep { 0 } else { fresh };
    if (1..=2).contains(&decl) && plus == removing {
        return Surface::Boundary(decl == 2);
    }
    if plus {
        if !removing && !keep && ((skin & 1 != 0 && decl & 2 != 0) || (skin & 2 != 0 && decl & 1 != 0)) {
            Surface::InteriorSeam
        } else if (!removing || decl == 0) && (old || (keep && fresh != 0)) {
            Surface::Partition
        } else {
            Surface::Discard
        }
    } else if decl == 0 && old {
        Surface::Sheet
    } else {
        Surface::Discard
    }
}

#[cfg(test)]
mod window_tests {
    use super::*;

    #[test]
    fn pruned_and_full_parity_give_identical_cap_edits() {
        let mut random = PyRandom::new(421);
        for round in 0..32 {
            let mut model = Store::empty(2, Default::default());
            let mut fresh = PySet::new();
            let mut ch = Change::default();
            // Closed solids on either side, intersecting sheets and remote
            // walls. Their faces can contribute to parity without touching
            // the cap loop.
            for i in 0..7 {
                let x = (random.random() * 7.).floor() + (i / 4 * 40) as f64;
                let y = (random.random() * 7.).floor();
                let z = if i % 2 == 0 { 0. } else { -2. };
                let w = 1. + random.random();
                let verts: [[f64; 3]; 8] = std::array::from_fn(|k| {
                    [
                        x + if k & 1 != 0 { w } else { 0. },
                        y + if k & 2 != 0 { w } else { 0. },
                        z + if k & 4 != 0 { 2. } else { 0. },
                    ]
                });
                for corners in [
                    [0, 2, 3, 1],
                    [4, 5, 7, 6],
                    [0, 1, 5, 4],
                    [2, 6, 7, 3],
                    [0, 4, 6, 2],
                    [1, 3, 7, 5],
                ] {
                    let f = model
                        .add_face(&[corners.map(|i| verts[i]).to_vec()], &mut ch)
                        .unwrap();
                    if i == 0 {
                        fresh.add(model.key(Kind::Face, f));
                    }
                }
            }
            let apply = |mut model: Store, window_min_faces| {
                let mut ch = Change::default();
                let mut paint = Paint::new(HashMap::new(), Vec::new());
                let mut ctx = Ctx {
                    window_min_faces,
                    standalone: false,
                    h: Hypot::LibStdCxx,
                    compensated: true,
                    keep: round % 2 == 0,
                    removing: round % 3 == 0,
                    metrics: Metrics::native(true),
                    paint: &mut paint,
                };
                let mut bumps = 0;
                let result = model
                    .apply_cap_rebuild(
                        V3(0., 0., 0.),
                        V3(0., 0., 1.),
                        &fresh,
                        &mut Vec::new(),
                        &mut Cache::default(),
                        &mut ctx,
                        &mut ch,
                        &mut bumps,
                    )
                    .unwrap();
                (model, ch, result, bumps)
            };
            let (reference, a, ra, ba) = apply(model.clone(), usize::MAX);
            let (local, b, rb, bb) = apply(model, 0);
            assert_eq!((a, ra, ba), (b, rb, bb));
            assert!(reference.state.same_as(&local.state));
        }
    }
}
