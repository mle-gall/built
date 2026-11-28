// SPDX-License-Identifier: GPL-3.0-or-later
//! The meeting index (docs/junctions.md §4): members read from stock and
//! cached boxes, never faces; meetings with keys and signatures; rule
//! matching with near misses; skips and Follow. Built when first asked,
//! then updated from the document's touched-key log. Never saved, never
//! journalled: only skips and Follow (on the live table) are.
mod classify;
use super::*;
use classify::{Dims, Found, Member, Shape};
use doc::value::Value as DocValue;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

/// Skips and Follow opt-ins, journalled with the live table and saved as
/// `ext["built.junctions"]`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Prefs {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub skips: BTreeMap<String, BTreeSet<String>>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub follow: BTreeSet<String>,
}
impl Prefs {
    pub fn is_empty(&self) -> bool {
        self.skips.is_empty() && self.follow.is_empty()
    }
    pub(super) fn skip(&mut self, detail: &str, keys: &[String], skip: bool) {
        let set = self.skips.entry(detail.into()).or_default();
        for k in keys {
            if skip {
                set.insert(k.clone());
            } else {
                set.remove(k);
            }
        }
        if set.is_empty() {
            self.skips.remove(detail);
        }
    }
}
pub(super) const PREFS_EXT: &str = "built.junctions";
pub(super) fn save_prefs(t: &LiveTable) -> Option<Value> {
    (!t.prefs.is_empty()).then(|| {
        let mut v = json!(*t.prefs);
        v["v"] = json!(1);
        v
    })
}
pub(super) fn load_prefs(v: Value) -> Option<Prefs> {
    (v["v"] == 1).then(|| serde_json::from_value(v).ok())?
}

const BAR_CLASSES: &[&str] = &[
    "IfcBeam", "IfcColumn", "IfcMember", "IfcPile", "IfcRailing", "IfcPipeSegment",
    "IfcDuctSegment",
];
const SLAB_CLASSES: &[&str] = &["IfcSlab", "IfcFooting", "IfcRoof", "IfcPlate", "IfcCovering"];
const WALL_CLASSES: &[&str] = &["IfcWall", "IfcWallStandardCase", "IfcCurtainWall"];
const HOSTED_CLASSES: &[&str] = &["IfcDoor", "IfcWindow", "IfcOpeningElement"];

/// A sweep's stock in its owner's coordinates.
#[derive(Clone)]
struct Template {
    path: Vec<DVec3>,
    sections: Vec<[DVec3; 3]>,
    half: [f64; 2],
    profile: String,
    profile_label: String,
    family: String,
    dims: Dims,
    wall: bool,
}

/// Canonical direction: a member drawn either way reads the same.
fn canonical(d: DVec3) -> (DVec3, bool) {
    let key = if d.z.abs() > 1e-9 {
        d.z
    } else if d.y.abs() > 1e-9 {
        d.y
    } else {
        d.x
    };
    if key < 0. {
        (-d, true)
    } else {
        (d, false)
    }
}

fn template(t: &LiveTable, row: &Row, path: Vec<DVec3>, class: &str) -> Option<Template> {
    let s = row.sweep.as_deref()?;
    if path.len() < 2 {
        return None;
    }
    let o = match &s.profile.sketch {
        Some(id) => {
            let mut profile = s.profile.clone();
            profile.drawing = Some(json!(t.rows.get(id).filter(|r| r.state == "ok")?));
            profile.outline(s.segments).ok()?
        }
        None => s.profile.outline(s.segments).ok()?,
    };
    Template::new(s, &o, path, class)
}
impl Template {
    fn new(s: &sweep::Sweep, o: &profile::Outline, path: Vec<DVec3>, class: &str) -> Option<Self> {
        let origin = o.attachment(&s.attachment).ok()?;
        let (sin, cos) = s.rotation.to_radians().sin_cos();
        let mx = if s.mirror { -1. } else { 1. };
        let mut sections = Vec::new();
        for w in path.windows(2) {
            let dir = (w[1] - w[0]).try_normalize()?;
            let frame = |d: DVec3| {
                let across = DVec3::Z.cross(d).try_normalize().unwrap_or(DVec3::X);
                (across, d.cross(across))
            };
            let (across, up) = frame(dir);
            let mut v = (glam::DVec2::new(o.width, o.height) * 0.5 - origin) / 1000.;
            v.x *= mx;
            let v = glam::DVec2::from_angle(s.rotation.to_radians()).rotate(v);
            let offset = across * v.x + up * v.y;
            let (ca, cu) = frame(canonical(dir).0);
            sections.push([
                ca * (mx * cos) + cu * (mx * sin),
                -ca * sin + cu * cos,
                offset,
            ]);
        }
        let dims = Dims::of(&[
            ("width", o.width),
            ("height", o.height),
            ("thickness", o.width),
            ("web", o.web),
            ("flange", o.flange),
            ("radius", o.radius),
        ]);
        Some(Template {
            path,
            sections,
            half: [o.width / 2000., o.height / 2000.],
            profile: s.profile.library.clone(),
            profile_label: s.profile.name().unwrap_or(&s.profile.library).into(),
            family: o.family.clone(),
            dims,
            wall: WALL_CLASSES.contains(&class) || s.profile.library == "basic/wall",
        })
    }
    /// One member per span; only the path's first and last points are ends.
    fn members(&self, world: DMat4, base: &Member) -> Vec<Member> {
        let n = self.path.len() - 1;
        let length: f64 = self.path.windows(2).map(|w| w[0].distance(w[1])).sum();
        (0..n)
            .map(|i| {
                let [across, up, offset] = self.sections[i];
                let mut dims = self.dims;
                dims.set("length", length * 1000.);
                Member {
                    uid: if n == 1 { base.uid.clone() } else { format!("{}#{i}", base.uid) },
                    a: world.transform_point3(self.path[i] + offset),
                    b: world.transform_point3(self.path[i + 1] + offset),
                    ends: [i == 0, i + 1 == n],
                    across: world.transform_vector3(across).normalize_or_zero(),
                    up: world.transform_vector3(up).normalize_or_zero(),
                    half: self.half,
                    shape: if self.wall { Shape::Wall } else { Shape::Bar },
                    profile: self.profile.clone(),
                    profile_label: self.profile_label.clone(),
                    family: self.family.clone(),
                    dims,
                    ..base.clone()
                }
            })
            .collect()
    }
}

/// Members found in the tree that still need their geometry's bounds.
enum Pending {
    Ready(Vec<Member>),
    Box {
        base: Member,
        def: u64,
        geometry: Py<PyAny>,
        world: DMat4,
        along: Option<(Template, DMat4)>,
    },
}

fn field<'a>(v: &'a DocValue, name: &str) -> Option<&'a DocValue> {
    let DocValue::Dict(items) = v else {
        return None;
    };
    items
        .iter()
        .find_map(|(k, v)| matches!(k, DocValue::Str(s) if s == name).then_some(v))
}
fn text<'a>(v: Option<&'a DocValue>) -> Option<&'a str> {
    match v? {
        DocValue::Str(s) => Some(s),
        _ => None,
    }
}
fn number(v: &DocValue) -> Option<f64> {
    match v {
        DocValue::Float(f) => Some(*f),
        DocValue::Int(i) => Some(*i as f64),
        _ => None,
    }
}
fn points(v: Option<&DocValue>) -> Option<Vec<DVec3>> {
    let (DocValue::List(items) | DocValue::Tuple(items)) = v? else {
        return None;
    };
    items
        .iter()
        .map(|p| match p {
            DocValue::List(c) | DocValue::Tuple(c) if c.len() == 3 => {
                Some(DVec3::new(number(&c[0])?, number(&c[1])?, number(&c[2])?))
            }
            DocValue::Vector(c) => Some(DVec3::from(*c)),
            _ => None,
        })
        .collect()
}
fn xform(n: &doc::Node) -> DMat4 {
    match &n.data {
        Data::Placement(p) => p
            .xform
            .as_ref()
            .map(|x| DMat4::from_cols_array(&x.doubles()))
            .unwrap_or(DMat4::IDENTITY),
        _ => DMat4::IDENTITY,
    }
}
/// World matrix and uid path of a node, read under the held lock.
fn locate(d: &doc::Doc, k: doc::Key) -> Option<(DMat4, Vec<doc::Key>, Vec<String>, doc::Key)> {
    let mut m = DMat4::IDENTITY;
    let mut chain = Vec::new();
    let mut path = Vec::new();
    let mut at = k;
    for _ in 0..256 {
        let n = d.node(at)?;
        if n.kind == doc::Kind::Root {
            chain.reverse();
            path.reverse();
            return Some((m, chain, path, at));
        }
        m = xform(n) * m;
        chain.push(at);
        if let Data::Placement(p) = &n.data {
            if let Some(uid) = &p.uid {
                path.push(uid.clone());
            }
        }
        at = *n.owners.first()?;
    }
    None
}

struct Walk<'a> {
    t: &'a LiveTable,
    /// Definition key → the sweep owner it was generated for (built when a
    /// copy first needs it).
    defs: Option<HashMap<u64, String>>,
    ext: u16,
    ifc: u16,
    names: HashMap<String, String>,
    out: Vec<(Vec<doc::Key>, Pending)>,
}
impl Walk<'_> {
    fn marker_path(&self, d: &doc::Doc, uid: &str) -> Option<Vec<DVec3>> {
        let n = d.node(*self.t.groups.get(uid)?)?;
        points(field(field(n.attr(self.ext)?, EXT)?, "path"))
    }
    fn sweep_template(&self, d: &doc::Doc, owner: &str, class: &str) -> Option<Template> {
        let row = self.t.rows.get(self.t.owners.get(owner)?)?;
        if row.kind != "sweep" || row.junction.is_some() {
            return None;
        }
        let path = match self.t.stock.get(&row.uid).filter(|s| s.state == "ok") {
            Some(s) => s.path.iter().map(|p| DVec3::from(*p)).collect(),
            None => self.marker_path(d, owner)?,
        };
        template(self.t, row, path, class)
    }
    fn node(
        &mut self,
        py: Python<'_>,
        d: &doc::Doc,
        k: doc::Key,
        world: DMat4,
        chain: &mut Vec<doc::Key>,
        path: &mut Vec<String>,
    ) {
        let Some(n) = d.node(k) else { return };
        let Data::Placement(p) = &n.data else { return };
        let world = world * xform(n);
        let ext = n.attr(self.ext);
        if ext.and_then(|e| field(e, "link")).is_some() {
            return;
        }
        let marker = ext.and_then(|e| field(e, EXT));
        if let Some(row) = text(marker.and_then(|m| field(m, "row"))).and_then(|r| self.t.rows.get(r)) {
            if row.kind == "junction" || row.junction.is_some() {
                return;
            }
        }
        chain.push(k);
        let pushed = p.uid.is_some();
        if let Some(uid) = &p.uid {
            path.push(uid.clone());
            if let Some(name) = &p.name {
                self.names.insert(uid.clone(), name.clone());
            }
            let class = text(n.attr(self.ifc).and_then(|r| field(r, "class"))).unwrap_or("");
            let base = Member {
                uid: uid.clone(),
                path: path.clone(),
                row: None,
                station: None,
                a: DVec3::ZERO,
                b: DVec3::ZERO,
                ends: [true, true],
                across: DVec3::X,
                up: DVec3::Z,
                half: [0., 0.],
                shape: Shape::Bar,
                class: class.to_string(),
                profile: String::new(),
                profile_label: String::new(),
                family: String::new(),
                dims: Dims::default(),
            };
            let def = p.definition.as_ref().map(|r| r.0.get());
            let copy = text(ext.and_then(|e| field(e, "built.repeat")).and_then(|m| field(m, "row")))
                .and_then(|r| self.t.rows.get(r))
                .filter(|r| r.kind == "repeat");
            if let Some(tpl) = self.sweep_template(d, uid, class) {
                let row = self.t.owners.get(uid).cloned();
                self.out.push((chain.clone(), Pending::Ready(tpl.members(world, &Member { row, ..base }))));
            } else if let Some((station, source, tpl)) = copy
                .and_then(|r| Some((r, r.source.clone()?)))
                .and_then(|(r, source)| {
                    // A direct copy binds as its Repeat's station; a copy
                    // inside a nested copy binds by placement.
                    let direct = chain.len() >= 2
                        && self.t.groups.get(&r.owner) == Some(&chain[chain.len() - 2]);
                    let station = direct.then(|| (r.uid.clone(), uid.clone()));
                    Some((station, source.clone(), self.sweep_template(d, &source, class)?))
                })
                .or_else(|| {
                    // A copy of a sweep's placement sharing its definition.
                    let defs = self.defs.get_or_insert_with(|| {
                        let mut defs = HashMap::new();
                        for row in self.t.rows.values().filter(|r| r.kind == "sweep" && r.junction.is_none()) {
                            let def = self.t.groups.get(&row.owner).and_then(|k| match &d.node(*k)?.data {
                                Data::Placement(p) => Some(p.definition.as_ref()?.0.get().key),
                                _ => None,
                            });
                            if let Some(def) = def {
                                defs.insert(def, row.owner.clone());
                            }
                        }
                        defs
                    });
                    let source = defs.get(&def?.key)?.clone();
                    let tpl = self.sweep_template(d, &source, class)?;
                    Some((None, source, tpl))
                })
            {
                let source_def = self.t.groups.get(&source).and_then(|sk| {
                    let Data::Placement(sp) = &d.node(*sk)?.data else { return None };
                    Some(sp.definition.as_ref()?.0.get().key)
                });
                let base = Member { station, ..base };
                match def.filter(|def| Some(def.key) != source_def) {
                    Some(def) => self.out.push((
                        chain.clone(),
                        Pending::Box {
                            base,
                            def: def.key,
                            geometry: def.geometry.clone_ref(py),
                            world,
                            along: Some((tpl, world)),
                        },
                    )),
                    None => self
                        .out
                        .push((chain.clone(), Pending::Ready(tpl.members(world, &base)))),
                }
            } else if let Some(def) = def.filter(|_| {
                [BAR_CLASSES, SLAB_CLASSES, WALL_CLASSES, HOSTED_CLASSES]
                    .iter()
                    .any(|l| l.contains(&class))
            }) {
                if !self.t.owners.contains_key(uid) || n.children.is_empty() {
                    self.out.push((
                        chain.clone(),
                        Pending::Box {
                            base,
                            def: def.key,
                            geometry: def.geometry.clone_ref(py),
                            world,
                            along: None,
                        },
                    ));
                }
            }
        }
        for &c in &n.children {
            self.node(py, d, c, world, chain, path);
        }
        chain.pop();
        if pushed {
            path.pop();
        }
    }
}

type Bounds = Option<(DVec3, DVec3)>;
fn local_bounds(py: Python<'_>, geometry: &Py<PyAny>) -> PyResult<(u64, Bounds)> {
    let store = geometry.bind(py).getattr("_store")?.cast_into::<PyMeshStore>()?;
    let s = store.try_borrow()?;
    let s = &s.inner;
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = -lo;
    for v in s.live_slots(Kind::Vertex) {
        let p = DVec3::from(s.state.frame.definition_point(*s.state.vpos.get(v as usize)));
        lo = lo.min(p);
        hi = hi.max(p);
    }
    Ok((s.revision, lo.is_finite().then_some((lo, hi))))
}
fn store_revision(py: Python<'_>, geometry: &Py<PyAny>) -> PyResult<u64> {
    let store = geometry.bind(py).getattr("_store")?.cast_into::<PyMeshStore>()?;
    let revision = store.try_borrow()?.inner.revision;
    Ok(revision)
}

/// A classified element's oriented box as a member.
fn box_member(base: Member, (lo, hi): (DVec3, DVec3), world: DMat4) -> Option<Member> {
    let centre = world.transform_point3((lo + hi) * 0.5);
    let size = hi - lo;
    let mut axes: Vec<(f64, DVec3)> = (0..3)
        .map(|i| {
            let v = world.transform_vector3(DVec3::AXES[i] * size[i]);
            (v.length(), v.normalize_or_zero())
        })
        .collect();
    axes.sort_by(|a, b| b.0.total_cmp(&a.0));
    let [(l, long), (w, wide), (h, thin)] = [axes[0], axes[1], axes[2]];
    let class = base.class.as_str();
    let (shape, (length, axis), across, up, half) = if HOSTED_CLASSES.contains(&class) {
        (Shape::Hostable, (l, long), wide, thin, [w / 2., h / 2.])
    } else if WALL_CLASSES.contains(&class) {
        // Run along the more horizontal of the two large extents.
        let (run, rise) = if long.z.abs() <= wide.z.abs() { ((l, long), (w, wide)) } else { ((w, wide), (l, long)) };
        (Shape::Wall, run, thin, rise.1, [h / 2., rise.0 / 2.])
    } else if l >= 3. * w {
        (Shape::Bar, (l, long), wide, thin, [w / 2., h / 2.])
    } else if SLAB_CLASSES.contains(&class) && h <= w / 3. {
        (Shape::Slab, (l, long), wide, thin, [w / 2., h / 2.])
    } else {
        return None;
    };
    let (dir, _) = canonical(axis);
    let half_len = length / 2.;
    let mut dims = Dims::of(&[
        ("length", length * 1000.),
        ("width", half[0] * 2000.),
        ("height", half[1] * 2000.),
        ("thickness", h * 1000.),
    ]);
    if shape == Shape::Wall {
        dims.set("width", h * 1000.);
    }
    Some(Member {
        a: centre - dir * half_len,
        b: centre + dir * half_len,
        across,
        up,
        half,
        shape,
        dims,
        ..base
    })
}

/// A Repeat copy with its own geometry: the source's section, spanning the
/// copy's extent along the source axis.
fn along_member(base: &Member, (lo, hi): (DVec3, DVec3), tpl: &Template, world: DMat4) -> Vec<Member> {
    let mut members = tpl.members(world, base);
    if let ([first], Some(dir)) = (members.as_mut_slice(), (tpl.path[1] - tpl.path[0]).try_normalize()) {
        let corners = (0..8).map(|i| {
            DVec3::new(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            )
            .dot(dir)
        });
        let (min, max) = corners.fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(v), b.max(v)));
        let o = tpl.path[0] + tpl.sections[0][2];
        let s0 = o.dot(dir);
        first.a = world.transform_point3(o + dir * (min - s0));
        first.b = world.transform_point3(o + dir * (max - s0));
        first.dims.set("length", (max - min) * 1000.);
    }
    members
}

#[derive(Default)]
struct DocIndex {
    index: classify::Index,
    chains: HashMap<String, Vec<doc::Key>>,
    names: HashMap<String, String>,
    top: Vec<doc::Key>,
    cursor: (u64, usize),
    boxes: HashMap<u64, (u64, Bounds)>,
    build_ms: f64,
    update_ms: f64,
    updated: usize,
}

static INDEXES: LazyLock<Mutex<HashMap<doc::Key, DocIndex>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

impl DocIndex {
    /// Read members under `keys` (whole subtrees), in one pass under the lock.
    fn read(&mut self, py: Python<'_>, t: &LiveTable, keys: &[doc::Key]) -> PyResult<Vec<(Vec<doc::Key>, Vec<Member>)>> {
        let mut walk = Walk {
            t,
            defs: None,
            ext: doc::attr_id("ext").map_err(PyValueError::new_err)?,
            ifc: doc::attr_id("ifc").map_err(PyValueError::new_err)?,
            names: HashMap::new(),
            out: Vec::new(),
        };
        {
            let d = doc::lock();
            for &k in keys {
                let Some(n) = d.node(k) else { continue };
                let Some(parent) = n.owners.first() else { continue };
                let Some((world, mut chain, mut path, _)) = locate(&d, *parent) else { continue };
                walk.node(py, &d, k, world, &mut chain, &mut path);
            }
        }
        self.names.extend(walk.names);
        let mut out = Vec::new();
        for (chain, pending) in walk.out {
            let members = match pending {
                Pending::Ready(m) => m,
                Pending::Box { base, def, geometry, world, along } => {
                    let revision = store_revision(py, &geometry)?;
                    let bounds = match self.boxes.get(&def).filter(|(r, _)| *r == revision) {
                        Some((_, b)) => *b,
                        None => {
                            let (r, b) = local_bounds(py, &geometry)?;
                            self.boxes.insert(def, (r, b));
                            b
                        }
                    };
                    let Some(bounds) = bounds else { continue };
                    match along {
                        Some((tpl, world)) => along_member(&base, bounds, &tpl, world),
                        None => box_member(base, bounds, world).into_iter().collect(),
                    }
                }
            };
            out.push((chain, members));
        }
        Ok(out)
    }

    fn apply(&mut self, removed: impl Fn(&[doc::Key]) -> bool, found: Vec<(Vec<doc::Key>, Vec<Member>)>) -> usize {
        let mut fresh = HashSet::new();
        let mut changed = 0;
        for (_, members) in &found {
            fresh.extend(members.iter().map(|m| m.uid.clone()));
        }
        let gone: Vec<String> = self
            .chains
            .iter()
            .filter(|(uid, chain)| removed(chain) && !fresh.contains(*uid))
            .map(|(uid, _)| uid.clone())
            .collect();
        for uid in gone {
            self.chains.remove(&uid);
            changed += usize::from(self.index.remove(&uid));
        }
        for (chain, members) in found {
            for m in members {
                self.chains.insert(m.uid.clone(), chain.clone());
                changed += usize::from(self.index.upsert(m));
            }
        }
        changed
    }

    fn build(py: Python<'_>, t: &LiveTable, root: doc::Key) -> PyResult<Self> {
        let started = Instant::now();
        let mut me = DocIndex::default();
        let top = {
            let mut d = doc::lock();
            d.touched.get_or_insert_with(Vec::new);
            d.reader_epochs.entry(root).or_default();
            me.cursor = (d.touched_gen, d.touched.as_ref().map_or(0, Vec::len));
            d.node(root).map(|n| n.children.clone()).unwrap_or_default()
        };
        let found = me.read(py, t, &top)?;
        let mut all = Vec::new();
        for (chain, members) in found {
            for m in members {
                me.chains.insert(m.uid.clone(), chain.clone());
                all.push(m);
            }
        }
        me.index = classify::Index::build(all);
        me.top = top;
        me.build_ms = started.elapsed().as_secs_f64() * 1000.;
        Ok(me)
    }

    /// Bring the index up to date with what was written since last time.
    fn refresh(&mut self, py: Python<'_>, t: &LiveTable, root: doc::Key) -> PyResult<()> {
        let started = Instant::now();
        let (keys, rescan, top) = {
            let mut d = doc::lock();
            let gen = d.touched_gen;
            let log = d.touched.get_or_insert_with(Vec::new);
            let (keys, rescan) = if gen != self.cursor.0 || log.len() < self.cursor.1 {
                (Vec::new(), true)
            } else {
                let mut keys: Vec<_> = log[self.cursor.1..].to_vec();
                keys.sort_unstable();
                keys.dedup();
                let many = keys.len() > 512;
                (keys, many)
            };
            self.cursor = (gen, log.len());
            // Trim what every reader has consumed when this is the only one.
            if INDEX_COUNT.load(std::sync::atomic::Ordering::Relaxed) <= 1 && log.len() > 4096 {
                log.clear();
                self.cursor.1 = 0;
            }
            let top = d.node(root).map(|n| n.children.clone()).unwrap_or_default();
            (keys, rescan, top)
        };
        if !rescan && keys.is_empty() {
            return Ok(());
        }
        let changed = if rescan {
            let found = self.read(py, t, &top)?;
            self.apply(|_| true, found)
        } else {
            let mut walk = Vec::new();
            let mut dropped = Vec::new();
            for k in &keys {
                if *k == root {
                    for c in self.top.iter().filter(|c| !top.contains(c)) {
                        dropped.push(*c);
                    }
                    for c in top.iter().filter(|c| !self.top.contains(c)) {
                        walk.push(*c);
                    }
                    continue;
                }
                let inside = {
                    let d = doc::lock();
                    d.node(*k).is_some() && locate(&d, *k).is_some_and(|(_, _, _, r)| r == root)
                };
                if inside {
                    walk.push(*k);
                } else {
                    dropped.push(*k);
                }
            }
            // A subtree walked from its top covers the keys inside it.
            let walk: Vec<_> = {
                let d = doc::lock();
                let set: HashSet<_> = walk.iter().copied().collect();
                walk.iter()
                    .copied()
                    .filter(|k| {
                        locate(&d, *k).is_none_or(|(_, chain, _, _)| {
                            !chain[..chain.len().saturating_sub(1)].iter().any(|a| set.contains(a))
                        })
                    })
                    .collect()
            };
            let found = self.read(py, t, &walk)?;
            let touched: HashSet<_> = walk.iter().chain(&dropped).copied().collect();
            self.apply(|chain| chain.iter().any(|k| touched.contains(k)), found)
        };
        self.top = top;
        self.updated = changed;
        self.update_ms = started.elapsed().as_secs_f64() * 1000.;
        Ok(())
    }
}
static INDEX_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Run `f` on the document's up-to-date index (built on first use).
fn with_index<R>(scene: &Obj<'_>, f: impl FnOnce(&LiveTable, &DocIndex) -> PyResult<R>) -> PyResult<R> {
    let py = scene.py();
    let document = root(scene)?;
    let t = current(py, document).unwrap_or_else(|| LiveTable {
        groups: Arc::new(index(document)),
        ..Default::default()
    });
    let mut all = INDEXES.lock().unwrap_or_else(|e| e.into_inner());
    // Indexes of documents that went away.
    {
        let d = doc::lock();
        all.retain(|k, _| d.node(*k).is_some());
    }
    match all.get_mut(&document) {
        Some(ix) => ix.refresh(py, &t, document)?,
        None => {
            let ix = DocIndex::build(py, &t, document)?;
            all.insert(document, ix);
        }
    }
    INDEX_COUNT.store(all.len(), std::sync::atomic::Ordering::Relaxed);
    f(&t, &all[&document])
}

/// Forget the document's index (tests measure a cold build).
#[pyfunction]
pub(super) fn meeting_index_reset(scene: &Obj<'_>) -> PyResult<()> {
    let document = root(scene)?;
    INDEXES.lock().unwrap_or_else(|e| e.into_inner()).remove(&document);
    Ok(())
}

// ------------------------------------------------------------ keys, roles

/// `h64("built:meeting:v1:<kind>:" + sorted [(uid path, end or face)])`.
pub(super) fn meeting_key(kind: &str, roles: [(&Member, &str); 2]) -> String {
    let mut parts: Vec<String> = roles
        .iter()
        .map(|(m, tag)| format!("{}|{}", m.path.join("/"), tag))
        .collect();
    parts.sort();
    eid::derived_uid(&format!("built:meeting:v1:{kind}:{}", parts.join(",")))
}
fn symmetric(f: &Found) -> bool {
    f.kind == "end_end"
        || (f.kind == "crossing" && f.tags[0] == "mid")
        || (f.kind == "corner" && f.tags[0] != "side")
}
fn ptr_eq(a: &Member, b: &Member) -> bool {
    std::ptr::eq(a, b)
}
fn leaf(uid: &str) -> &str {
    uid.split('#').next().unwrap_or(uid)
}
fn end_of(m: &Member, tag: &str, at: DVec3) -> &'static str {
    match tag {
        "start" if m.ends[0] => "start",
        "end" if m.ends[1] => "end",
        _ => {
            if m.a.distance_squared(at) <= m.b.distance_squared(at) {
                "start"
            } else {
                "end"
            }
        }
    }
}
/// A J0 role binding for a member at this meeting.
fn binding(m: &Member, end: &str) -> Value {
    let uid = leaf(&m.uid);
    match (&m.station, &m.row) {
        (Some((row, station)), _) => json!({"row": row, "station": station, "end": end}),
        (None, Some(row)) => json!({"placement": uid, "row": row, "end": end}),
        (None, None) => json!({"placement": uid, "end": end}),
    }
}
/// The member a junction role binding names.
fn bound_member(t: &LiveTable, b: &Value) -> Option<String> {
    if let Some(station) = b.get("station") {
        let row = b["row"].as_str()?;
        return Some(match station {
            Value::String(s) if s.len() == 16 => s.clone(),
            s => eid::derived_uid(&format!(
                "built:repeat:v1:{row}:{}",
                s.as_u64().map(|i| i.to_string()).or_else(|| s.as_str().map(String::from))?
            )),
        });
    }
    b["placement"]
        .as_str()
        .or_else(|| b["placement"].as_array()?.last()?.as_str())
        .map(String::from)
        .or_else(|| Some(t.rows.get(b["row"].as_str()?)?.owner.clone()))
}
/// Existing top-level junctions by the pair of members they join.
fn existing(t: &LiveTable) -> HashMap<(String, String), Vec<Arc<Row>>> {
    let mut out: HashMap<_, Vec<_>> = HashMap::new();
    for id in &t.junctions {
        let j = &t.rows[id];
        if j.joint.parent.is_some() {
            continue;
        }
        let members: Vec<String> = j
            .joint
            .roles
            .as_object()
            .into_iter()
            .flat_map(|r| r.values())
            .filter_map(|b| bound_member(t, b))
            .collect();
        for (i, a) in members.iter().enumerate() {
            for b in &members[i + 1..] {
                let pair = if a <= b { (a.clone(), b.clone()) } else { (b.clone(), a.clone()) };
                out.entry(pair).or_default().push(j.clone());
            }
        }
    }
    out
}
fn pair(x: &Member, y: &Member) -> (String, String) {
    let (a, b) = (leaf(&x.uid).to_string(), leaf(&y.uid).to_string());
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

fn descriptor(m: &Member, tag: &str) -> String {
    let profile = if m.profile.is_empty() {
        if m.family.is_empty() {
            format!("{:.0}x{:.0}", m.dims.get("width").unwrap_or(0.), m.dims.get("height").unwrap_or(0.))
        } else {
            m.family.clone()
        }
    } else {
        m.profile.clone()
    };
    let tag = if matches!(tag, "start" | "end") { "end" } else { tag };
    format!("{}:{}:{}", m.class, profile, tag)
}
fn label_of(m: &Member) -> String {
    if m.profile.is_empty() {
        m.class.trim_start_matches("Ifc").to_string()
    } else {
        m.profile_label.clone()
    }
}
/// Grouping signature, independent of modelling direction and hand.
fn signature(main: &Member, other: &Member, f: &Found) -> (String, String) {
    let mut roles = [descriptor(main, if f.tags[0] == "side" { f.face } else { f.tags[0] }), descriptor(other, f.tags[1])];
    let mut labels = [label_of(main), label_of(other)];
    if symmetric(f) && roles[0] > roles[1] {
        roles.swap(0, 1);
        labels.swap(0, 1);
    }
    let bucket = f.angle.round();
    let how = match f.kind {
        "end_face" => format!("end to {}", f.face.replace('_', " ")),
        "end_end" if f.angle < 5. => "splice".into(),
        "end_end" => "end to end".into(),
        "crossing" if f.tags[0] == "side" => format!("seated on {}", f.face.replace('_', " ")),
        "corner" => arrangement(f).replace("corner", "L corner"),
        k => k.replace('_', " "),
    };
    (
        format!("{}|{}|{}|{bucket}", f.kind, roles[0], roles[1]),
        format!("{} × {} · {how} · {bucket}°", labels[0], labels[1]),
    )
}

// ------------------------------------------------------------- rule match

#[derive(Clone, Debug, Serialize)]
struct Miss {
    criterion: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    value: Value,
    limit: Value,
    widen_to: Value,
}
fn miss(criterion: &str, role: Option<&str>, value: Value, limit: Value, widen_to: Value) -> Miss {
    Miss { criterion: criterion.into(), role: role.map(String::from), value, limit, widen_to }
}
fn role_misses(name: &str, m: &Member, crit: &Value, face: Option<&str>, end: &str, exact: bool, out: &mut Vec<Miss>) {
    if let Some(classes) = crit["class"].as_array().filter(|c| !c.is_empty()) {
        if !classes.iter().any(|c| c == m.class.as_str()) {
            let mut wider = classes.clone();
            wider.push(json!(m.class));
            out.push(miss("class", Some(name), json!(m.class), json!(classes), json!(wider)));
        }
    }
    if let Some(profiles) = crit["profile"].as_array().filter(|p| !p.is_empty()) {
        let fits = profiles.iter().any(|p| {
            p["library"].as_str().is_none_or(|prefix| !m.profile.is_empty() && m.profile.starts_with(prefix))
                && p["family"].as_str().is_none_or(|f| f == m.family)
                && (!p["library"].is_string() || !m.profile.is_empty())
        });
        if !fits {
            let mut wider = profiles.clone();
            wider.push(if m.profile.is_empty() { json!({"family": m.family}) } else { json!({"library": m.profile}) });
            out.push(miss("profile", Some(name), json!(m.profile), json!(profiles), json!(wider)));
        }
    }
    for (dim, range) in crit["size"].as_object().into_iter().flatten() {
        let Some(v) = m.dims.get(dim) else { continue };
        let v = &v;
        let (lo, hi) = (range[0].as_f64(), range[1].as_f64());
        if lo.is_some_and(|l| *v < l - 1e-6) || hi.is_some_and(|h| *v > h + 1e-6) {
            let wider = json!([lo.map(|l| l.min(*v)), hi.map(|h| h.max(*v))]);
            out.push(miss("size", Some(name), json!({dim: v}), json!({dim: range}), json!({dim: wider})));
        }
    }
    if let (Some(face), Some(allowed)) = (face, crit["face"].as_array().filter(|f| !f.is_empty())) {
        if !allowed.iter().any(|f| f == face) {
            let mut wider = allowed.clone();
            wider.push(json!(face));
            out.push(miss("face", Some(name), json!(face), json!(allowed), json!(wider)));
        }
    }
    if exact {
        if let Some(want) = crit["end"].as_str().filter(|e| *e != "any") {
            if want != end {
                out.push(miss("end", Some(name), json!(end), json!(want), json!("any")));
            }
        }
    }
}
/// What keeps this meeting out of the rule (empty: it fits).
fn misses(rule: &Value, names: [&str; 2], roles: [(&Member, &str); 2], f: &Found) -> Vec<Miss> {
    let mut out = Vec::new();
    let exact = rule["direction"] == "exact";
    for (i, (m, end)) in roles.iter().enumerate() {
        let face = (i == 0 && matches!(f.kind, "end_face" | "crossing") && f.tags[0] == "side").then_some(f.face);
        role_misses(names[i], m, &rule["roles"][names[i]], face, end, exact, &mut out);
    }
    if let Some(value) = rule["angle"]["value"].as_f64() {
        let tol = rule["angle"]["tol"].as_f64().unwrap_or(2.);
        let off = (f.angle - value).abs();
        if off > tol + 1e-4 {
            let mut wider = rule["angle"].clone();
            wider["tol"] = json!((off - 1e-3).ceil());
            out.push(miss("angle", None, json!((f.angle * 10.).round() / 10.), rule["angle"].clone(), wider));
        }
    }
    if rule["mirror"] == "exact" {
        if let Some(hand) = rule["hand"].as_i64().filter(|h| *h != 0 && f.hand != 0) {
            if hand != i64::from(f.hand) {
                out.push(miss("mirror", None, json!("mirrored"), json!("exact"), json!("allow")));
            }
        }
    }
    if rule["offset"].is_object() {
        let want = rule["offset"]["value"].as_f64().unwrap_or(0.);
        let tol = rule["offset"]["tol_mm"].as_f64().unwrap_or(5.);
        let off = (f.offset - want).abs();
        if off > tol + 1e-6 {
            let mut wider = rule["offset"].clone();
            wider["tol_mm"] = json!((off - 1e-3).ceil());
            out.push(miss("offset", None, json!((f.offset * 10.).round() / 10.), rule["offset"].clone(), wider));
        }
    }
    out
}

fn role_names(p: &presets::Preset) -> [String; 2] {
    let mut names: Vec<String> = p
        .values
        .get("roles")
        .and_then(Value::as_object)
        .map(|r| r.keys().cloned().collect())
        .unwrap_or_default();
    names.sort_by_key(|n| (!matches!(n.as_str(), "main" | "host" | "a" | "ply_1"), n.clone()));
    let first = names.first().cloned().unwrap_or_else(|| "main".into());
    let second = names.get(1).cloned().unwrap_or_else(|| "joining".into());
    [first, second]
}
fn rule_of(p: &presets::Preset) -> Value {
    let mut rule = p.values.get("rule").cloned().unwrap_or(Value::Null);
    if rule.is_null() {
        rule = json!({});
    }
    if rule["meeting"].is_null() {
        rule["meeting"] = p.values.get("meeting").cloned().unwrap_or(Value::Null);
    }
    if rule["arrangement"].is_null() {
        rule["arrangement"] = p.values.get("arrangement").cloned().unwrap_or(Value::Null);
    }
    rule
}
/// A corner is an L (two ends) or a tee (an end on the other's run).
fn arrangement(f: &Found) -> &'static str {
    match (f.kind, f.tags[0]) {
        ("corner", "side") => "tee",
        ("corner", _) => "corner",
        _ => "",
    }
}

#[derive(Serialize)]
struct Suggestion {
    key: String,
    kind: &'static str,
    at: [f64; 3],
    label: String,
    roles: BTreeMap<String, Value>,
    mirrored: bool,
    fit: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    near: Option<Miss>,
    #[serde(skip_serializing_if = "Option::is_none")]
    existing: Option<Value>,
    why: Vec<String>,
}

/// One meeting against one detail's rule.
fn suggestion(
    t: &LiveTable,
    ix: &DocIndex,
    p: &presets::Preset,
    rule: &Value,
    names: &[String; 2],
    existing: &HashMap<(String, String), Vec<Arc<Row>>>,
    (main, other, f): (&Member, &Member, &Found),
) -> Option<Suggestion> {
    let kind = rule["meeting"].as_str();
    if kind.is_some_and(|k| k != f.kind) {
        return None;
    }
    // A corner detail without an arrangement is for L corners.
    if f.kind == "corner" && rule["arrangement"].as_str().unwrap_or("corner") != arrangement(f) {
        return None;
    }
    let names = [names[0].as_str(), names[1].as_str()];
    let mut orders = vec![[(main, f.tags[0]), (other, f.tags[1])]];
    if symmetric(f) {
        orders.push([(other, f.tags[1]), (main, f.tags[0])]);
    }
    let (roles, failed) = orders
        .into_iter()
        .map(|o| {
            let ends = [end_of(o[0].0, if o[0].1 == "side" { f.main_end } else { o[0].1 }, f.at), end_of(o[1].0, o[1].1, f.at)];
            let m = misses(rule, names, [(o[0].0, ends[0]), (o[1].0, ends[1])], f);
            ([(o[0].0, ends[0]), (o[1].0, ends[1])], m)
        })
        // Equal fits: the thicker member, then the older one, is main
        // (a wall corner's priority without the load-bearing flag).
        .min_by_key(|(roles, m)| {
            let width = |m: &Member| (m.dims.get("width").unwrap_or(0.) * 1000.) as i64;
            let order = |m: &Member| m.row.as_ref().and_then(|r| t.order.get(r)).copied().unwrap_or(u64::MAX);
            (m.len(), -width(roles[0].0), order(roles[0].0))
        })?;
    if failed.len() > 1 {
        return None;
    }
    let key = meeting_key(f.kind, [(main, f.tags[0]), (other, f.tags[1])]);
    let hand = rule["hand"].as_i64().unwrap_or(0);
    let mirrored = hand != 0 && f.hand != 0 && hand != i64::from(f.hand);
    let mut why = Vec::new();
    if mirrored {
        why.push("mirrored".to_string());
    }
    for (i, (m, _)) in roles.iter().enumerate() {
        let wanted: Vec<&str> = rule["roles"][names[i]]["profile"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p["library"].as_str())
            .collect();
        // Only a rule naming one catalogue size says another size differs.
        if wanted.len() == 1
            && profile::label(wanted[0]).is_some()
            && wanted[0] != m.profile
            && !m.profile.is_empty()
            && failed.is_empty()
        {
            why.push(format!("sized differently: {}", label_of(m)));
        }
    }
    if let Some(value) = rule["angle"]["value"].as_f64() {
        if (f.angle - value).abs() >= 0.05 {
            why.push(format!("angle {:.1}°", f.angle));
        }
    }
    let name = |m: &Member| ix.names.get(leaf(&m.uid)).cloned().unwrap_or_else(|| label_of(m));
    let detailed = existing
        .get(&pair(main, other))
        .and_then(|js| js.first())
        .map(|j| json!({"uid": j.uid, "detail": j.joint.detail.get("uid"), "name": j.joint.detail.get("name"), "state": j.state, "same": j.joint.detail.get("uid").and_then(Value::as_str) == Some(p.uid.as_str())}));
    let skipped = t.prefs.skips.get(&p.uid).is_some_and(|s| s.contains(&key));
    let fit = match (&detailed, failed.is_empty(), skipped) {
        (Some(d), _, _) if d["same"] == true => "applied",
        (Some(_), _, _) => "detailed",
        (None, _, true) => "skipped",
        (None, true, false) => "match",
        (None, false, false) => "near",
    };
    Some(Suggestion {
        key,
        kind: f.kind,
        at: f.at.to_array(),
        label: format!("{} × {}", name(roles[0].0), name(roles[1].0)),
        roles: BTreeMap::from([
            (names[0].to_string(), {
                let mut b = binding(roles[0].0, roles[0].1);
                // The plies a junction measures through need the flange
                // named; other faces are found by facing (J0).
                if f.face == "flange_outer" && f.tags[0] == "side" && ptr_eq(roles[0].0, main) {
                    b["face"] = json!(f.face);
                }
                b
            }),
            (names[1].to_string(), binding(roles[1].0, roles[1].1)),
        ]),
        mirrored,
        fit,
        near: failed.into_iter().next(),
        existing: detailed,
        why,
    })
}

fn junction_detail(t: &LiveTable, uid: &str) -> PyResult<presets::Preset> {
    presets::lookup(t, uid)
        .filter(|p| p.primitive == "junction")
        .ok_or_else(|| PyValueError::new_err("Unknown junction detail"))
}

fn preview_detail(t: &LiveTable, detail: &Obj<'_>) -> PyResult<presets::Preset> {
    if let Ok(uid) = detail.extract::<String>() {
        junction_detail(t, &uid)
    } else {
        json(detail)
    }
}

fn suggest(t: &LiveTable, ix: &DocIndex, p: &presets::Preset) -> Vec<Suggestion> {
    let rule = rule_of(p);
    let names = role_names(p);
    let existing = existing(t);
    let mut out: Vec<_> = ix
        .index
        .all()
        .filter_map(|(a, b, f)| suggestion(t, ix, p, &rule, &names, &existing, (a, b, &f)))
        .collect();
    out.sort_by(|a, b| a.label.cmp(&b.label).then(a.key.cmp(&b.key)));
    out
}

/// Meetings that fit a detail, its near misses, skipped and detailed ones.
#[pyfunction]
pub(super) fn junction_suggest<'p>(scene: &Obj<'p>, detail: &Obj<'p>) -> PyResult<Obj<'p>> {
    let started = Instant::now();
    let reply = with_index(scene, |t, ix| {
        let p = preview_detail(t, detail)?;
        let list = suggest(t, ix, &p);
        let mut counts = BTreeMap::<&str, usize>::new();
        for s in &list {
            *counts.entry(s.fit).or_default() += 1;
        }
        Ok(json!({
            "detail": {"uid": p.uid, "rev": p.rev, "name": p.name},
            "follow": t.prefs.follow.contains(&p.uid),
            "suggestions": list,
            "counts": counts,
        }))
    })?;
    let mut reply = reply;
    reply["ms"] = json!(started.elapsed().as_secs_f64() * 1000.);
    to_py(scene.py(), &reply)
}

/// Per project detail: uses, matches waiting, near misses, skips, Follow.
#[pyfunction]
pub(super) fn junction_counts<'p>(scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    let started = Instant::now();
    let mut reply = with_index(scene, |t, ix| {
        let mut out = serde_json::Map::new();
        for p in t.presets.values().filter(|p| p.primitive == "junction") {
            let mut counts = BTreeMap::<&str, usize>::new();
            for s in suggest(t, ix, p) {
                *counts.entry(s.fit).or_default() += 1;
            }
            let follow = t.prefs.follow.contains(&p.uid);
            out.insert(
                p.uid.clone(),
                json!({"name": p.name, "counts": counts, "follow": follow,
                       "new": if follow { counts.get("match").copied().unwrap_or(0) } else { 0 }}),
            );
        }
        Ok(json!({"details": out}))
    })?;
    reply["ms"] = json!(started.elapsed().as_secs_f64() * 1000.);
    to_py(scene.py(), &reply)
}

/// Every meeting in the model, or those matching `filter` (`kind`,
/// `member`: a placement uid).
#[pyfunction(name = "meetings")]
#[pyo3(signature = (scene, filter=None))]
pub(super) fn meetings_query<'p>(scene: &Obj<'p>, filter: Option<&Obj<'p>>) -> PyResult<Obj<'p>> {
    let filter: Value = match filter {
        Some(f) if !f.is_none() => json(f)?,
        _ => Value::Null,
    };
    let list = with_index(scene, |_, ix| {
        let mut out = Vec::new();
        for (main, other, f) in ix.index.all() {
            let f = &f;
            if filter["kind"].as_str().is_some_and(|k| k != f.kind) {
                continue;
            }
            if let Some(uid) = filter["member"].as_str() {
                if leaf(&main.uid) != uid && leaf(&other.uid) != uid {
                    continue;
                }
            }
            let (sig, words) = signature(main, other, f);
            let mut main_binding = binding(main, end_of(main, if f.tags[0] == "side" { f.main_end } else { f.tags[0] }, f.at));
            if f.face == "flange_outer" {
                main_binding["face"] = json!(f.face);
            }
            out.push(json!({
                "key": meeting_key(f.kind, [(main, f.tags[0]), (other, f.tags[1])]),
                "kind": f.kind,
                "at": f.at.to_array(),
                "angle": (f.angle * 100.).round() / 100.,
                "offset": (f.offset * 10.).round() / 10.,
                "face": f.face,
                "hand": f.hand,
                "signature": sig,
                "arrangement": words,
                "roles": {"main": main_binding, "other": binding(other, end_of(other, f.tags[1], f.at))},
                "axes": [[main.a.to_array(), main.b.to_array()], [other.a.to_array(), other.b.to_array()]],
                "members": [leaf(&main.uid), leaf(&other.uid)],
            }));
        }
        out.sort_by(|a, b| a["key"].as_str().cmp(&b["key"].as_str()));
        Ok(out)
    })?;
    to_py(scene.py(), &list)
}

/// The "By arrangement" view's data: meetings grouped by signature, each
/// with how many are detailed, by which details, and how many are outside.
#[pyfunction]
pub(super) fn junction_arrangements<'p>(scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    let groups = with_index(scene, |t, ix| {
        let existing = existing(t);
        let mut groups: BTreeMap<String, (String, &'static str, usize, BTreeMap<String, usize>, usize, Vec<String>)> = BTreeMap::new();
        for (main, other, f) in ix.index.all() {
            let f = &f;
            let (sig, words) = signature(main, other, f);
            let g = groups.entry(sig).or_insert_with(|| (words, f.kind, 0, BTreeMap::new(), 0, Vec::new()));
            g.2 += 1;
            g.5.push(meeting_key(f.kind, [(main, f.tags[0]), (other, f.tags[1])]));
            if let Some(j) = existing.get(&pair(main, other)).and_then(|js| js.first()) {
                let name = j.joint.detail["name"].as_str().unwrap_or("Custom").to_string();
                *g.3.entry(name).or_default() += 1;
                g.4 += usize::from(j.state == "outside");
            }
        }
        let mut out: Vec<Value> = groups
            .into_iter()
            .map(|(sig, (words, kind, count, details, outside, keys))| {
                let detailed: usize = details.values().sum();
                let status = if detailed == 0 { "none" } else if detailed == count && outside == 0 { "all" } else { "some" };
                json!({"signature": sig, "arrangement": words, "kind": kind, "count": count,
                       "detailed": detailed, "details": details, "outside": outside,
                       "status": status, "meetings": keys})
            })
            .collect();
        out.sort_by(|a, b| b["count"].as_u64().cmp(&a["count"].as_u64()).then(a["arrangement"].as_str().cmp(&b["arrangement"].as_str())));
        Ok(out)
    })?;
    to_py(scene.py(), &groups)
}

/// Index sizes and timings, for budgets and tests.
#[pyfunction]
pub(super) fn meeting_index_stats<'p>(scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    let stats = with_index(scene, |_, ix| {
        Ok(json!({
            "members": ix.index.len(),
            "meetings": ix.index.all().count(),
            "bytes": ix.index.heap_bytes() + ix.chains.len() * 64 + ix.names.len() * 64,
            "build_ms": ix.build_ms,
            "update_ms": ix.update_ms,
            "updated": ix.updated,
        }))
    })?;
    to_py(scene.py(), &stats)
}

/// Ghost lines for a detail at meetings: the meet cross and each owned
/// plane cut's outline, from the junction's own frames (not committed).
#[pyfunction]
pub(super) fn junction_ghosts<'p>(scene: &Obj<'p>, detail: &Obj<'p>, keys: Vec<String>) -> PyResult<Obj<'p>> {
    let py = scene.py();
    let (p, chosen) = with_index(scene, |t, ix| {
        let p = preview_detail(t, detail)?;
        let wanted: HashSet<&String> = keys.iter().collect();
        let chosen: Vec<_> = suggest(t, ix, &p)
            .into_iter()
            .filter(|s| wanted.contains(&s.key))
            .map(|s| {
                let size = s
                    .roles
                    .values()
                    .filter_map(|b| bound_member(t, b))
                    .filter_map(|uid| ix.index.by_uid(&uid).map(|m| m.radius()))
                    .fold(0.05f64, f64::max);
                (s.key, s.kind, s.at, s.roles, size)
            })
            .collect();
        Ok((p, chosen))
    })?;
    let t = current(py, root(scene)?).unwrap_or_default();
    let mut out = Vec::new();
    for (key, kind, at, roles, size) in chosen {
        let at = DVec3::from(at);
        let mut lines = Vec::new();
        let r = size * 0.6;
        for axis in [DVec3::X, DVec3::Y, DVec3::Z] {
            lines.push([(at - axis * r).to_array(), (at + axis * r).to_array()]);
        }
        let owner = roles
            .values()
            .find_map(|b| bound_member(&t, b).filter(|u| t.groups.contains_key(u)));
        if let Some(owner) = owner {
            let row: Option<Row> = serde_json::from_value(json!({
                "kind": "junction", "uid": "0000000000000000", "owner": owner,
                "roles": roles, "meeting": kind,
            }))
            .ok();
            if let Some((frames, dims)) = row
                .as_ref()
                .map(|row| junction::geometry(scene, &t, row))
                .transpose()?
                .flatten()
            {
                let defs = p.values.get("params").and_then(Value::as_array).cloned().unwrap_or_default();
                let params = expr::resolve(&defs, &Value::Null, &dims).map(|r| r.0).unwrap_or_default();
                let world = reference::frame(t.groups[&owner]);
                for item in p.values.get("contents").and_then(Value::as_array).into_iter().flatten() {
                    let row = &item["row"];
                    let Some(frame) = row["anchor"]["ref"].as_str().and_then(|s| s.strip_prefix('@')) else { continue };
                    if !row["cutter"]["plane"].is_object() {
                        continue;
                    }
                    let Some(m) = frames.get(frame) else { continue };
                    let m = world * DMat4::from_cols_array(m);
                    let offset = match &row["anchor"]["offset"] {
                        Value::Null => 0.,
                        v => expr::field(v, &params, &dims).unwrap_or(0.),
                    };
                    let (x, y, z) = (m.x_axis.truncate().normalize_or_zero(), m.y_axis.truncate().normalize_or_zero(), m.z_axis.truncate().normalize_or_zero());
                    let o = m.w_axis.truncate() + z * offset / 1000.;
                    let s = size * 1.4;
                    let c = [o - x * s - y * s, o + x * s - y * s, o + x * s + y * s, o - x * s + y * s];
                    for i in 0..4 {
                        lines.push([c[i].to_array(), c[(i + 1) % 4].to_array()]);
                    }
                }
            }
        }
        out.push(json!({"key": key, "at": at.to_array(), "lines": lines}));
    }
    to_py(py, &out)
}

/// One meeting as capture and the Junction tool read it (J2).
#[derive(Clone, Debug, Serialize)]
pub(in crate::live) struct Lite {
    pub key: String,
    pub kind: &'static str,
    pub at: [f64; 3],
    pub angle: f64,
    pub offset: f64,
    pub face: &'static str,
    pub hand: i8,
    /// Main then the other: placement leaf uids.
    pub members: [String; 2],
    /// Their occurrence paths (a Repeat copy's ancestors included).
    pub paths: [Vec<String>; 2],
    /// J0 role bindings, main then the other.
    pub roles: [Value; 2],
    pub profiles: [String; 2],
    pub classes: [String; 2],
    pub dims: [BTreeMap<String, f64>; 2],
    pub label: String,
    pub arrangement: String,
}
fn dims_map(m: &Member) -> BTreeMap<String, f64> {
    ["width", "height", "thickness", "web", "flange", "radius", "length"]
        .iter()
        .filter_map(|k| Some((k.to_string(), m.dims.get(k)?)))
        .collect()
}
/// Every meeting of the document, from the up-to-date index.
pub(in crate::live) fn lite(scene: &Obj<'_>) -> PyResult<Vec<Lite>> {
    with_index(scene, |_, ix| {
        Ok(ix
            .index
            .all()
            .map(|(main, other, f)| {
                let mut main_binding = binding(main, end_of(main, if f.tags[0] == "side" { f.main_end } else { f.tags[0] }, f.at));
                if f.face == "flange_outer" {
                    main_binding["face"] = json!(f.face);
                }
                let name = |m: &Member| ix.names.get(leaf(&m.uid)).cloned().unwrap_or_else(|| label_of(m));
                Lite {
                    key: meeting_key(f.kind, [(main, f.tags[0]), (other, f.tags[1])]),
                    kind: f.kind,
                    at: f.at.to_array(),
                    angle: f.angle,
                    offset: f.offset,
                    face: f.face,
                    hand: f.hand,
                    members: [leaf(&main.uid).to_string(), leaf(&other.uid).to_string()],
                    paths: [main.path.clone(), other.path.clone()],
                    roles: [main_binding, binding(other, end_of(other, f.tags[1], f.at))],
                    profiles: [main.profile.clone(), other.profile.clone()],
                    classes: [main.class.clone(), other.class.clone()],
                    dims: [dims_map(main), dims_map(other)],
                    label: format!("{} × {}", name(main), name(other)),
                    arrangement: signature(main, other, &f).1,
                }
            })
            .collect())
    })
}
/// A draft detail's suggestions (not saved): fit per meeting key, with the
/// role bindings `junction_apply` takes.
pub(in crate::live) fn draft_fits(scene: &Obj<'_>, p: &presets::Preset) -> PyResult<Vec<(String, &'static str, BTreeMap<String, Value>, [f64; 3])>> {
    with_index(scene, |t, ix| {
        Ok(suggest(t, ix, p).into_iter().map(|s| (s.key, s.fit, s.roles, s.at)).collect())
    })
}

pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(meetings_query, m)?)?;
    m.add_function(wrap_pyfunction!(junction_suggest, m)?)?;
    m.add_function(wrap_pyfunction!(junction_counts, m)?)?;
    m.add_function(wrap_pyfunction!(junction_arrangements, m)?)?;
    m.add_function(wrap_pyfunction!(junction_ghosts, m)?)?;
    m.add_function(wrap_pyfunction!(meeting_index_stats, m)?)?;
    m.add_function(wrap_pyfunction!(meeting_index_reset, m)?)
}
