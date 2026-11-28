// SPDX-License-Identifier: GPL-3.0-or-later
//! Members as capsules, a uniform grid over them, and the meeting classifier.
//! Pure geometry: no document, no faces.
use glam::DVec3;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Contact tolerance in metres (docs/junctions.md §4.2: envelope + 5 mm).
pub const TOL: f64 = 0.005;
/// End to face needs this much angle between the axes (degrees).
const MIN_ANGLE: f64 = 10.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A member with an axis: beams, columns, purlins, braces.
    Bar,
    /// A planar member with an axis: a wall (height ≫ thickness).
    Wall,
    /// A slab, footing or plate box: members stand on its top face.
    Slab,
    /// A door, window or sleeve box: hosted by a wall or slab.
    Hostable,
}

/// Role dimensions in millimetres, inline (no allocation per member).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Dims([f64; 7]);
impl Dims {
    const NAMES: [&'static str; 7] = ["width", "height", "thickness", "web", "flange", "radius", "length"];
    pub fn of(pairs: &[(&str, f64)]) -> Self {
        let mut d = Dims::default();
        for (k, v) in pairs {
            d.set(k, *v);
        }
        d
    }
    pub fn set(&mut self, name: &str, v: f64) {
        if let Some(i) = Self::NAMES.iter().position(|n| *n == name) {
            self.0[i] = v;
        }
    }
    /// A dimension the member has (zero: it has none, a web on a box).
    pub fn get(&self, name: &str) -> Option<f64> {
        Self::NAMES.iter().position(|n| *n == name).map(|i| self.0[i]).filter(|v| *v != 0.)
    }
}

/// One element as the index sees it. `a`→`b` is the envelope's centre line
/// in world coordinates; `across` and `up` are the section axes of the
/// member drawn in its canonical direction, so a reversed member reads the
/// same.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub uid: String,
    pub path: Vec<String>,
    /// The sweep row, when the member has a recipe.
    pub row: Option<String>,
    /// `(repeat row, station key)` for a Repeat copy.
    pub station: Option<(String, String)>,
    pub a: DVec3,
    pub b: DVec3,
    /// Whether `a` and `b` are real member ends (a polyline span's inner
    /// points are not).
    pub ends: [bool; 2],
    pub across: DVec3,
    pub up: DVec3,
    /// Half extents along `across` and `up`, metres.
    pub half: [f64; 2],
    pub shape: Shape,
    pub class: String,
    pub profile: String,
    pub family: String,
    /// Role dimensions in millimetres (`width`, `height`, `length`…).
    pub dims: Dims,
}

impl Member {
    pub fn len(&self) -> f64 {
        self.a.distance(self.b)
    }
    pub fn dir(&self) -> DVec3 {
        (self.b - self.a).normalize_or_zero()
    }
    pub fn radius(&self) -> f64 {
        self.half[0].hypot(self.half[1])
    }
    pub fn end(&self, i: usize) -> DVec3 {
        if i == 0 {
            self.a
        } else {
            self.b
        }
    }
    /// Distance along the axis from `a`, and the offsets across and up.
    fn local(&self, p: DVec3) -> (f64, f64, f64) {
        let d = p - self.a;
        let dir = self.dir();
        let t = d.dot(dir);
        let v = d - dir * t;
        (t, v.dot(self.across), v.dot(self.up))
    }
    /// Half the envelope's depth seen along `n`.
    fn depth(&self, n: DVec3) -> f64 {
        n.dot(self.across).abs() * self.half[0] + n.dot(self.up).abs() * self.half[1]
    }
    /// The end tag of end `i` as the recipe names it.
    fn tag(i: usize) -> &'static str {
        if i == 0 {
            "start"
        } else {
            "end"
        }
    }
    fn bounds(&self, pad: f64) -> (DVec3, DVec3) {
        let r = DVec3::splat(self.radius() + pad);
        (self.a.min(self.b) - r, self.a.max(self.b) + r)
    }
}

/// A classified meeting between two members. `main` indexes the pair
/// `(x, y)` given to [`classify`]: 0 when `x` is the main role.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub kind: &'static str,
    pub main: usize,
    /// Main's tag then the other's: `start`/`end` for an end, `side` for a
    /// face met by an end, `mid` for a crossing, `top` for a slab, `host`
    /// and `hosted`.
    pub tags: [&'static str; 2],
    pub at: DVec3,
    /// Angle between the axes, 0–90°.
    pub angle: f64,
    /// Distance between the two axis lines, millimetres.
    pub offset: f64,
    /// The contact face role on the main member.
    pub face: &'static str,
    /// Mirror class relative to the main member's section: ±1, or 0 when
    /// the meeting has no hand.
    pub hand: i8,
    /// The main member's end nearest the meeting (what its binding names).
    pub main_end: &'static str,
}

/// Every name a [`Found`] uses, so a stored meeting keeps one byte each.
const NAMES: [&str; 15] = [
    "end_face", "end_end", "crossing", "corner", "base", "hosted", "start", "end", "side", "mid",
    "top", "bottom", "host", "flange_outer", "web",
];
fn code(name: &str) -> u8 {
    NAMES.iter().position(|n| *n == name).expect("a meeting name") as u8
}
/// A meeting as the index keeps it (a fifth of a [`Found`]).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Stored {
    names: [u8; 5],
    main: u8,
    hand: i8,
    angle: f32,
    offset: f32,
    at: [f64; 3],
}
impl Stored {
    fn pack(f: &Found) -> Self {
        Stored {
            names: [code(f.kind), code(f.tags[0]), code(f.tags[1]), code(f.face), code(f.main_end)],
            main: f.main as u8,
            hand: f.hand,
            angle: f.angle as f32,
            offset: f.offset as f32,
            at: f.at.to_array(),
        }
    }
    fn unpack(&self) -> Found {
        let n = |i: usize| NAMES[self.names[i] as usize];
        Found {
            kind: n(0),
            main: self.main as usize,
            tags: [n(1), n(2)],
            at: DVec3::from(self.at),
            angle: f64::from(self.angle),
            offset: f64::from(self.offset),
            face: n(3),
            hand: self.hand,
            main_end: n(4),
        }
    }
    fn kind(&self) -> &'static str {
        NAMES[self.names[0] as usize]
    }
    fn tag(&self, i: usize) -> &'static str {
        NAMES[self.names[1 + i] as usize]
    }
}

fn angle(x: &Member, y: &Member) -> f64 {
    x.dir().dot(y.dir()).abs().clamp(0., 1.).acos().to_degrees()
}

fn line_distance(x: &Member, y: &Member) -> f64 {
    let (dx, dy) = (x.dir(), y.dir());
    let n = dx.cross(dy);
    let w = y.a - x.a;
    if n.length_squared() < 1e-12 {
        (w - dx * w.dot(dx)).length() * 1000.
    } else {
        w.dot(n.normalize()).abs() * 1000.
    }
}

/// An end of the other member touching `m`: inside its span (with the
/// tolerance) and within its envelope inflated by the other's radius.
fn touches(m: &Member, p: DVec3, other_r: f64) -> bool {
    let (t, x, y) = m.local(p);
    t >= -TOL
        && t <= m.len() + TOL
        && x.abs() <= m.half[0] + other_r + TOL
        && y.abs() <= m.half[1] + other_r + TOL
}

fn nearest_end(m: &Member, p: DVec3) -> &'static str {
    if m.a.distance_squared(p) <= m.b.distance_squared(p) {
        "start"
    } else {
        "end"
    }
}

/// Which face of `main` an end of `other` meets, and its hand.
/// Which face of `main` an end of `other` meets, its hand, and its outward
/// normal.
fn face(main: &Member, other: &Member, end: usize) -> (&'static str, i8, DVec3) {
    let p = other.end(end);
    let (_, x, y) = main.local(p);
    let (sx, sy) = (x / (main.half[0] + 1e-6), y / (main.half[1] + 1e-6));
    let (across, sign) = if sx.abs().max(sy.abs()) > 0.25 {
        (sx.abs() >= sy.abs(), if sx.abs() >= sy.abs() { sx } else { sy })
    } else {
        // Butting on the axis: the side the other member comes from.
        let inward = if end == 0 { other.dir() } else { -other.dir() };
        let (wx, wy) = (inward.dot(main.across), inward.dot(main.up));
        (wx.abs() >= wy.abs(), if wx.abs() >= wy.abs() { wx } else { wy })
    };
    let hand = if sign >= 0. { 1 } else { -1 };
    let name = match (across, main.family.as_str()) {
        (false, "i") => "flange_outer",
        (true, "i") => "web",
        (false, _) => "top",
        (true, _) => "side",
    };
    let normal = if across { main.across } else { main.up } * f64::from(hand);
    (name, hand, normal)
}

fn segment_closest(x: &Member, y: &Member) -> (f64, f64, f64) {
    let (dx, dy) = (x.b - x.a, y.b - y.a);
    let r = x.a - y.a;
    let (a, e, f) = (dx.dot(dx), dy.dot(dy), dy.dot(r));
    let (c, b) = (dx.dot(r), dx.dot(dy));
    let den = a * e - b * b;
    let mut s = if den > 1e-14 {
        ((b * f - c * e) / den).clamp(0., 1.)
    } else {
        0.
    };
    let mut t = (b * s + f) / e.max(1e-14);
    if t < 0. {
        t = 0.;
        s = (-c / a.max(1e-14)).clamp(0., 1.);
    } else if t > 1. {
        t = 1.;
        s = ((b - c) / a.max(1e-14)).clamp(0., 1.);
    }
    let d = (x.a + dx * s).distance(y.a + dy * t);
    (s, t, d)
}

fn base(slab: &Member, m: &Member) -> Option<(usize, DVec3)> {
    // The slab's top: its `up` side facing world +Z.
    let top = if slab.up.z >= 0. { 1. } else { -1. };
    if slab.up.z.abs() < 0.7 {
        return None;
    }
    let on = |p: DVec3, below: f64| {
        let (t, x, y) = slab.local(p);
        t >= -TOL
            && t <= slab.len() + TOL
            && x.abs() <= slab.half[0] + TOL
            && {
                let h = y * top - below;
                h <= slab.half[1] + TOL && h >= -slab.half[1] - TOL
            }
    };
    if m.shape == Shape::Wall {
        let mid = (m.a + m.b) * 0.5;
        let below = m.depth(DVec3::Z);
        if m.dir().z.abs() < 0.3 && on(mid, below) {
            return Some((usize::MAX, mid - DVec3::Z * below));
        }
        return None;
    }
    if m.dir().z.abs() < 0.7 {
        return None;
    }
    (0..2)
        .filter(|&i| m.ends[i])
        .find(|&i| on(m.end(i), 0.))
        .map(|i| (i, m.end(i)))
}

fn hosted(host: &Member, fill: &Member) -> Option<DVec3> {
    let centre = (fill.a + fill.b) * 0.5;
    let (t, x, y) = host.local(centre);
    (t >= 0. && t <= host.len() && x.abs() <= host.half[0] + TOL && y.abs() <= host.half[1] + TOL)
        .then_some(centre)
}

/// Classify the pair, or `None` when the members do not meet.
pub fn classify(x: &Member, y: &Member) -> Option<Found> {
    use Shape::*;
    let found = |kind, main: usize, tags, at, face, hand, main_end| Found {
        kind,
        main,
        tags,
        at,
        angle: angle(x, y),
        offset: line_distance(x, y),
        face,
        hand,
        main_end,
    };
    match (x.shape, y.shape) {
        (Hostable, Hostable) | (Slab, Slab) | (Slab, Hostable) | (Hostable, Slab) => {
            return None
        }
        (Wall, Hostable) | (Hostable, Wall) => {
            let (host, fill, main) = if x.shape == Wall { (x, y, 0) } else { (y, x, 1) };
            let at = hosted(host, fill)?;
            let mut f = found("hosted", main, ["host", "hosted"], at, "side", 0, "start");
            f.main_end = nearest_end(host, at);
            return Some(f);
        }
        (Hostable, _) | (_, Hostable) => return None,
        (Slab, _) | (_, Slab) => {
            let (slab, m, main) = if x.shape == Slab { (x, y, 0) } else { (y, x, 1) };
            let (end, at) = base(slab, m)?;
            let tag = if end == usize::MAX { "bottom" } else { Member::tag(end) };
            let mut f = found("base", main, ["top", tag], at, "top", 0, "start");
            f.main_end = nearest_end(slab, at);
            return Some(f);
        }
        _ => (),
    }
    let walls = x.shape == Wall && y.shape == Wall;
    let (rx, ry) = (x.radius(), y.radius());
    // End to end: two ends together, each touching the other member.
    let mut best: Option<(f64, usize, usize)> = None;
    for i in (0..2).filter(|&i| x.ends[i]) {
        for j in (0..2).filter(|&j| y.ends[j]) {
            let d = x.end(i).distance(y.end(j));
            if d <= rx.max(ry) + TOL
                && touches(y, x.end(i), rx)
                && touches(x, y.end(j), ry)
                && best.is_none_or(|b| d < b.0)
            {
                best = Some((d, i, j));
            }
        }
    }
    if let Some((_, i, j)) = best {
        let at = (x.end(i) + y.end(j)) * 0.5;
        let kind = if walls { "corner" } else { "end_end" };
        return Some(found(kind, 0, [Member::tag(i), Member::tag(j)], at, "end", 0, Member::tag(i)));
    }
    // End to face: an end of one on the other's envelope, at an angle.
    if angle(x, y) >= MIN_ANGLE {
        let mut pick: Option<(f64, usize, usize)> = None;
        for (main, m, o, ro) in [(0, x, y, ry), (1, y, x, rx)] {
            for j in (0..2).filter(|&j| o.ends[j]) {
                if touches(m, o.end(j), ro) {
                    let (t, _, _) = m.local(o.end(j));
                    let interior = t.min(m.len() - t);
                    if pick.is_none_or(|p| interior > p.0) {
                        pick = Some((interior, main, j));
                    }
                }
            }
        }
        if let Some((_, main, j)) = pick {
            let (m, o) = if main == 0 { (x, y) } else { (y, x) };
            let (face, hand, normal) = face(m, o, j);
            let at = o.end(j);
            // An end lying on the face (a purlin seated on a rafter's top
            // flange) bears on it: a stacked crossing, not an end into it.
            let along = o.dir().dot(normal).abs() < MIN_ANGLE.to_radians().sin();
            let kind = if walls { "corner" } else if along { "crossing" } else { "end_face" };
            return Some(found(kind, main, ["side", Member::tag(j)], at, face, hand, nearest_end(m, at)));
        }
        // Crossing: closest points inside both spans, envelopes in contact.
        let (s, t, d) = segment_closest(x, y);
        let (lx, ly) = (x.len(), y.len());
        let n = x.dir().cross(y.dir()).normalize_or_zero();
        if s * lx > TOL
            && s * lx < lx - TOL
            && t * ly > TOL
            && t * ly < ly - TOL
            && d <= x.depth(n) + y.depth(n) + TOL
        {
            let at = (x.a.lerp(x.b, s) + y.a.lerp(y.b, t)) * 0.5;
            return Some(found("crossing", 0, ["mid", "mid"], at, "mid", 0, nearest_end(x, at)));
        }
    }
    None
}

/// The members, a uniform grid of capsule bounds, and the meetings found,
/// kept up to date one member at a time.
#[derive(Default)]
pub struct Index {
    slots: Vec<Option<Arc<Member>>>,
    free: Vec<u32>,
    ids: HashMap<String, u32>,
    cell: f64,
    cells: HashMap<[i32; 3], Vec<u32>>,
    placed: Vec<Vec<[i32; 3]>>,
    pub(crate) meetings: HashMap<(u32, u32), Stored>,
    near: Vec<Vec<u32>>,
}

impl Index {
    /// Grid cells sized from the median member length.
    pub fn build(members: Vec<Member>) -> Self {
        let mut lengths: Vec<f64> = members
            .iter()
            .map(|m| m.len() + 2. * m.radius())
            .collect();
        lengths.sort_by(f64::total_cmp);
        let median = lengths.get(lengths.len() / 2).copied().unwrap_or(4.);
        let mut index = Index {
            cell: median.clamp(0.5, 20.),
            ..Default::default()
        };
        for m in members {
            index.insert(m, false);
        }
        let mut pairs = HashSet::new();
        for list in index.cells.values() {
            for (k, &i) in list.iter().enumerate() {
                for &j in &list[k + 1..] {
                    pairs.insert((i.min(j), i.max(j)));
                }
            }
        }
        for (i, j) in pairs {
            index.test(i, j);
        }
        index
    }
    pub fn len(&self) -> usize {
        self.ids.len()
    }
    pub fn member(&self, id: u32) -> &Member {
        self.slots[id as usize].as_ref().expect("live member")
    }
    pub fn by_uid(&self, uid: &str) -> Option<&Member> {
        self.ids.get(uid).map(|&i| self.member(i))
    }
    pub fn members(&self) -> impl Iterator<Item = &Member> {
        self.slots.iter().flatten().map(|m| m.as_ref())
    }
    fn cells_of(&self, m: &Member) -> Vec<[i32; 3]> {
        let (lo, hi) = m.bounds(TOL);
        let c = |v: f64| (v / self.cell).floor() as i32;
        let mut out = Vec::new();
        for i in c(lo.x)..=c(hi.x) {
            for j in c(lo.y)..=c(hi.y) {
                for k in c(lo.z)..=c(hi.z) {
                    out.push([i, j, k]);
                }
            }
        }
        out
    }
    fn test(&mut self, i: u32, j: u32) {
        let (x, y) = (self.member(i), self.member(j));
        if x.uid == y.uid {
            return;
        }
        let (_, _, d) = segment_closest(x, y);
        if d > x.radius() + y.radius() + 2. * TOL {
            return;
        }
        if let Some(f) = classify(x, y) {
            self.meetings.insert((i, j), Stored::pack(&f));
            self.near[i as usize].push(j);
            self.near[j as usize].push(i);
        }
    }
    fn insert(&mut self, m: Member, query: bool) -> u32 {
        let cells = self.cells_of(&m);
        let uid = m.uid.clone();
        let id = match self.free.pop() {
            Some(id) => {
                self.slots[id as usize] = Some(Arc::new(m));
                id
            }
            None => {
                self.slots.push(Some(Arc::new(m)));
                self.placed.push(Vec::new());
                self.near.push(Vec::new());
                (self.slots.len() - 1) as u32
            }
        };
        self.ids.insert(uid, id);
        let mut others = HashSet::new();
        for c in &cells {
            let list = self.cells.entry(*c).or_default();
            if query {
                others.extend(list.iter().copied());
            }
            list.push(id);
        }
        self.placed[id as usize] = cells;
        let mut others: Vec<_> = others.into_iter().collect();
        others.sort_unstable();
        for o in others {
            self.test(id.min(o), id.max(o));
        }
        id
    }
    pub fn remove(&mut self, uid: &str) -> bool {
        let Some(id) = self.ids.remove(uid) else {
            return false;
        };
        for c in std::mem::take(&mut self.placed[id as usize]) {
            if let Some(list) = self.cells.get_mut(&c) {
                list.retain(|&k| k != id);
                if list.is_empty() {
                    self.cells.remove(&c);
                }
            }
        }
        for o in std::mem::take(&mut self.near[id as usize]) {
            self.meetings.remove(&(id.min(o), id.max(o)));
            self.near[o as usize].retain(|&k| k != id);
        }
        self.slots[id as usize] = None;
        self.free.push(id);
        true
    }
    /// Replace or add a member; unchanged members cost one comparison.
    pub fn upsert(&mut self, m: Member) -> bool {
        if self.by_uid(&m.uid).is_some_and(|old| *old == m) {
            return false;
        }
        if self.cell == 0. {
            self.cell = (m.len() + 2. * m.radius()).clamp(0.5, 20.);
        }
        self.remove(&m.uid);
        self.insert(m, true);
        true
    }
    /// Every meeting as `(main, other, found)`.
    pub fn all(&self) -> impl Iterator<Item = (&Member, &Member, Found)> {
        self.meetings
            .iter()
            .filter(|(&(i, j), f)| !self.lapped(i, j, f))
            .map(|(&(i, j), f)| {
                let (x, y) = (self.member(i), self.member(j));
                let f = f.unpack();
                if f.main == 0 {
                    (x, y, f)
                } else {
                    (y, x, f)
                }
            })
    }
    /// Two ends that both sit on a third member's face (purlins butting over
    /// a rafter) meet that member, not each other.
    fn lapped(&self, i: u32, j: u32, f: &Stored) -> bool {
        if f.kind() != "end_end" {
            return false;
        }
        let on = |end: u32, tag: &str, host: u32| {
            self.meetings
                .get(&(end.min(host), end.max(host)))
                .is_some_and(|g| {
                    matches!(g.kind(), "end_face" | "crossing")
                        && g.main == u8::from(host > end)
                        && g.tag(1) == tag
                })
        };
        self.near[i as usize]
            .iter()
            .any(|&k| k != j && on(i, f.tag(0), k) && on(j, f.tag(1), k))
    }
    /// Approximate heap use in bytes (members, grid and meetings).
    pub fn heap_bytes(&self) -> usize {
        let member = |m: &Member| {
            std::mem::size_of::<Member>()
                + m.uid.len()
                + m.path.iter().map(|s| s.len() + 24).sum::<usize>()
                + m.row.as_ref().map_or(0, String::len)
                + m.station.as_ref().map_or(0, |(a, b)| a.len() + b.len())
                + m.class.len()
                + m.profile.len()
                + m.family.len()
        };
        self.members().map(member).sum::<usize>()
            + self.ids.keys().map(|k| k.len() + 40).sum::<usize>()
            + self.cells.values().map(|v| 48 + v.len() * 4).sum::<usize>()
            + self.placed.iter().map(|v| 24 + v.len() * 12).sum::<usize>()
            + self.near.iter().map(|v| 24 + v.len() * 4).sum::<usize>()
            + self.meetings.len() * (std::mem::size_of::<Stored>() + 16)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// An I-section bar from `a` to `b`: `w`×`h` mm, `h` along `up`.
    pub fn bar(uid: &str, a: [f64; 3], b: [f64; 3], w: f64, h: f64, up: [f64; 3]) -> Member {
        let (a, b) = (DVec3::from(a), DVec3::from(b));
        let dir = (b - a).normalize();
        let up = (DVec3::from(up) - dir * DVec3::from(up).dot(dir)).normalize();
        Member {
            uid: uid.into(),
            path: vec![uid.into()],
            row: None,
            station: None,
            a,
            b,
            ends: [true, true],
            across: up.cross(dir).normalize(),
            up,
            half: [w / 2000., h / 2000.],
            shape: Shape::Bar,
            class: "IfcBeam".into(),
            profile: "en/ipe240".into(),
            family: "i".into(),
            dims: Dims::of(&[("height", h), ("width", w)]),
        }
    }

    fn knee(mirror: f64) -> (Member, Member) {
        let column = bar("c", [0., 0., 0.], [0., 0., 3.], 200., 190., [mirror, 0., 0.]);
        let slope = 15f64.to_radians();
        let start = DVec3::new(mirror * 0.11, 0., 2.85);
        let end = start + DVec3::new(mirror * slope.cos(), 0., slope.sin()) * 3.;
        let rafter = bar("r", start.into(), end.into(), 120., 240., [0., 0., 1.]);
        (column, rafter)
    }

    #[test]
    fn a_knee_is_end_to_flange_and_the_other_side_is_mirrored() {
        let (c, r) = knee(1.);
        let f = classify(&c, &r).unwrap();
        assert_eq!((f.kind, f.main, f.tags), ("end_face", 0, ["side", "start"]));
        assert_eq!(f.face, "flange_outer");
        assert!((f.angle - 75.).abs() < 1e-6, "{}", f.angle);
        assert!(f.offset < 1e-6);
        let (c2, r2) = knee(-1.);
        let g = classify(&r2, &c2).unwrap();
        assert_eq!((g.kind, g.main, g.tags), ("end_face", 1, ["side", "start"]));
        // The second column's section axes point the same way as the first's.
        let c2 = Member { up: c.up, across: c.across, ..c2 };
        assert_eq!(classify(&c2, &r2).unwrap().hand, -f.hand);
    }

    #[test]
    fn reversed_members_meet_the_same_way() {
        let (c, r) = knee(1.);
        let reversed = Member { a: r.b, b: r.a, ..r.clone() };
        let f = classify(&c, &r).unwrap();
        let g = classify(&c, &reversed).unwrap();
        assert_eq!((g.kind, g.tags[1]), ("end_face", "end"));
        assert_eq!((f.face, f.hand), (g.face, g.hand));
    }

    #[test]
    fn apex_purlins_and_far_members() {
        let s = 15f64.to_radians();
        let ridge = DVec3::new(3., 0., 3.8);
        let left = bar("l", (ridge - DVec3::new(s.cos(), 0., s.sin()) * 3.).into(), ridge.into(), 120., 240., [0., 0., 1.]);
        let right = bar("rr", (ridge + DVec3::new(s.cos(), 0., -s.sin()) * 3.).into(), ridge.into(), 120., 240., [0., 0., 1.]);
        let f = classify(&left, &right).unwrap();
        assert_eq!((f.kind, f.tags), ("end_end", ["end", "end"]));
        // A purlin sitting on the rafter's top flange, ending there.
        let top = left.a.lerp(left.b, 0.5) + left.up * (0.12 + 0.05);
        let purlin = bar("p", top.into(), (top + DVec3::Y * 5.).into(), 100., 100., [0., 0., 1.]);
        let g = classify(&left, &purlin).unwrap();
        assert_eq!((g.kind, g.main, g.face, g.tags), ("crossing", 0, "flange_outer", ["side", "start"]));
        let far = bar("f", [10., 10., 0.], [10., 12., 0.], 100., 100., [0., 0., 1.]);
        assert!(classify(&left, &far).is_none());
        // A continuous purlin over the rafter crosses it.
        let over = bar("o", (top - DVec3::Y * 2.).into(), (top + DVec3::Y * 2.).into(), 100., 100., [0., 0., 1.]);
        assert_eq!(classify(&left, &over).unwrap().kind, "crossing");
    }

    #[test]
    fn columns_stand_on_a_slab_and_walls_make_corners() {
        let mut slab = bar("s", [0., 2., -0.1], [10., 2., -0.1], 4000., 200., [0., 0., 1.]);
        slab.shape = Shape::Slab;
        let column = bar("c", [1., 1., 0.], [1., 1., 3.], 200., 200., [1., 0., 0.]);
        assert_eq!(classify(&slab, &column).unwrap().kind, "base");
        let mut a = bar("a", [0., 0., 1.5], [5., 0., 1.5], 200., 3000., [0., 0., 1.]);
        let mut b = bar("b", [5., 0., 1.5], [5., 4., 1.5], 200., 3000., [0., 0., 1.]);
        a.shape = Shape::Wall;
        b.shape = Shape::Wall;
        assert_eq!(classify(&a, &b).unwrap().tags, ["end", "start"]);
        b.a = DVec3::new(2.5, 0.1, 1.5);
        let t = classify(&a, &b).unwrap();
        assert_eq!((t.kind, t.tags), ("corner", ["side", "start"]));
    }

    #[test]
    fn purlins_butting_over_a_rafter_meet_the_rafter_only() {
        let rafter = bar("r", [0., 0., 0.], [0., 6., 0.], 120., 240., [0., 0., 1.]);
        let top = DVec3::new(0., 3., 0.17);
        let p1 = bar("p1", (top - DVec3::X * 5.).into(), top.into(), 100., 100., [0., 0., 1.]);
        let p2 = bar("p2", top.into(), (top + DVec3::X * 5.).into(), 100., 100., [0., 0., 1.]);
        assert_eq!(classify(&p1, &p2).unwrap().kind, "end_end");
        let index = Index::build(vec![rafter, p1, p2]);
        let kinds: Vec<_> = index.all().map(|(m, _, f)| (m.uid.as_str(), f.kind)).collect();
        assert_eq!(kinds, [("r", "crossing"); 2]);
    }

    #[test]
    fn the_index_updates_one_member() {
        let (c, r) = knee(1.);
        let mut index = Index::build(vec![c.clone(), r.clone()]);
        assert_eq!(index.meetings.len(), 1);
        let moved = Member { a: r.a + DVec3::X * 2., b: r.b + DVec3::X * 2., ..r.clone() };
        assert!(index.upsert(moved));
        assert!(index.meetings.is_empty());
        assert!(!index.upsert(Member { ..index.by_uid("r").unwrap().clone() }));
        assert!(index.upsert(r));
        assert_eq!(index.meetings.len(), 1);
        assert!(index.remove("c"));
        assert!(index.meetings.is_empty() && index.len() == 1);
    }
}
