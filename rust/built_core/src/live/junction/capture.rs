// SPDX-License-Identifier: GPL-3.0-or-later
//! Capture (docs/junctions.md §5): gather the cuts, parts and child
//! junctions made at one meeting, find their parameters and *sized from*
//! relations, draft the detail and its rule, and convert the gathered items
//! into the first use of the detail, keeping every uid.
use super::*;
use std::collections::BTreeSet;

/// Gathering stops here (§5).
const CAP: usize = 200;
const PART_CLASSES: &[&str] = &[
    "IfcPlate",
    "IfcFastener",
    "IfcMechanicalFastener",
    "IfcDiscreteAccessory",
    "IfcBuildingElementPart",
];

/// One gathered item, as the popover's Includes list shows it.
#[derive(Clone, Debug, Serialize)]
pub(in crate::live) struct Item {
    pub id: String,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placement: Option<String>,
    pub label: String,
    pub meta: String,
    /// `anchored` (follows an anchor to the members) or `near` (beside the
    /// meeting, offered unticked unless it rests on a member).
    pub how: &'static str,
    pub ticked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub key: String,
}

/// What the popover changes before Save.
#[derive(Clone, Debug, Default, Deserialize)]
pub(in crate::live) struct Options {
    #[serde(default)]
    pub include: Option<Vec<String>>,
    #[serde(default)]
    pub swap: bool,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub tol: Option<f64>,
    #[serde(default)]
    pub mirror: Option<String>,
    /// Parameter expressions the user chose (a relation, or a literal).
    #[serde(default)]
    pub exprs: BTreeMap<String, Value>,
    /// Size ranges per role and dimension the user typed.
    #[serde(default)]
    pub sizes: BTreeMap<String, BTreeMap<String, [f64; 2]>>,
    /// Detail uid to write (a re-save of the same draft keeps it).
    #[serde(default)]
    pub uid: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(in crate::live) struct Param {
    pub key: String,
    pub name: String,
    pub group: String,
    pub unit: &'static str,
    pub value: f64,
    pub expr: Value,
    /// The relation in words ("= R-1 width + 30"), when sized from one.
    pub relation: String,
    /// Other relations that give the same value, best first.
    pub alternatives: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(in crate::live) struct Draft {
    pub detail: presets::Preset,
    pub roles: serde_json::Map<String, Value>,
    pub contents: Vec<Value>,
    pub items: Vec<Item>,
    pub params: Vec<Param>,
    pub meeting: meetings::Lite,
    pub members: [String; 2],
    pub names: [String; 2],
    /// World points where the roles are painted (main, joining).
    pub paint: [[f64; 3]; 2],
    pub similar: usize,
    pub near: usize,
}

fn err(s: impl ToString) -> PyErr {
    PyValueError::new_err(s.to_string())
}

/// The placements a row references (its anchors, cutter, path ends).
pub(in crate::live) fn refs(t: &LiveTable, r: &Row) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let owner_of = |id: &str| t.rows.get(id).map(|r| r.owner.clone());
    for reference in r.references() {
        match reference {
            Reference::Placement { placement } => {
                out.insert(placement.clone());
            }
            Reference::Element { path, .. } => out.extend(path.last().cloned()),
            Reference::Axis { sweep, .. } | Reference::Row { row: sweep } => out.extend(owner_of(sweep)),
            Reference::Frame { junction, .. } => {
                if let Some(j) = t.rows.get(junction) {
                    out.extend(j.joint.roles.as_object().into_iter().flatten().filter_map(|(_, b)| placement(b).map(String::from)));
                }
            }
            Reference::Point(_) | Reference::Slot { .. } => {}
        }
    }
    match &r.cutter {
        Some(cut::Cutter::Body(uid) | cut::Cutter::Bisector(uid)) => {
            out.insert(uid.clone());
        }
        _ => {}
    }
    out.extend(r.sweep.as_ref().and_then(|s| s.path.rests_on.clone()));
    out.remove(&r.owner);
    out
}

fn record(py: Python<'_>, t: &LiveTable, uid: &str) -> Value {
    group(py, t, uid)
        .and_then(|g| g.getattr("ifc").ok())
        .filter(|r| !r.is_none())
        .and_then(|r| {
            let raw: String = py.import("json").ok()?.call_method1("dumps", (r, )).ok()?.extract().ok()?;
            serde_json::from_str(&raw).ok()
        })
        .unwrap_or(Value::Null)
}
fn name_of(py: Python<'_>, t: &LiveTable, uid: &str) -> String {
    group(py, t, uid)
        .and_then(|g| g.getattr("name").ok()?.extract::<String>().ok())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Part".into())
}
fn world(t: &LiveTable, uid: &str) -> DMat4 {
    t.groups.get(uid).map(|k| reference::frame(*k)).unwrap_or(DMat4::IDENTITY)
}
/// A sweep's stock path in world coordinates.
fn world_path(scene: &Obj<'_>, t: &LiveTable, r: &Row) -> PyResult<Vec<DVec3>> {
    let Some(s) = &r.sweep else { return Ok(Vec::new()) };
    let m = world(t, &r.owner);
    Ok(reference::sweep_path(scene, t, r, &s.path)?
        .unwrap_or_default()
        .into_iter()
        .map(|p| m.transform_point3(p.into()))
        .collect())
}
fn is_part(py: Python<'_>, t: &LiveTable, r: &Row) -> bool {
    r.kind == "sweep"
        && r.junction.is_none()
        && record(py, t, &r.owner)["class"]
            .as_str()
            .is_some_and(|c| PART_CLASSES.contains(&c))
}
fn snake(name: &str) -> String {
    let mut s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    while s.contains("__") {
        s = s.replace("__", "_");
    }
    let s = s.trim_matches('_').to_string();
    if s.is_empty() || s.starts_with(|c: char| c.is_ascii_digit()) {
        format!("p_{s}")
    } else {
        s
    }
}

/// The seed's members and where it is, to choose its meeting.
fn seed_info(scene: &Obj<'_>, t: &LiveTable, seed: &Value) -> PyResult<(BTreeSet<String>, Option<DVec3>, bool)> {
    let py = scene.py();
    if let Some(list) = seed.as_array().or_else(|| seed["members"].as_array()) {
        let set: BTreeSet<String> = list.iter().filter_map(|v| v.as_str().map(String::from)).collect();
        return Ok((set, None, true));
    }
    let id = seed.as_str().ok_or_else(|| err("Expected a capture seed"))?;
    let row = t.rows.get(id).or_else(|| t.owners.get(id).and_then(|r| t.rows.get(r)));
    if let Some(r) = row {
        match r.kind.as_str() {
            "cut" => {
                let mut set = refs(t, r);
                set.insert(r.owner.clone());
                return Ok((set, None, false));
            }
            "junction" => {
                let set: BTreeSet<String> = r.joint.roles.as_object().into_iter().flatten().filter_map(|(_, b)| placement(b).map(String::from)).collect();
                let at = r.frames.get("meet").map(|m| world(t, &r.owner).transform_point3(DMat4::from_cols_array(m).w_axis.truncate()));
                return Ok((set, at, false));
            }
            _ => {
                let mut set = refs(t, r);
                let path = world_path(scene, t, r)?;
                let at = (!path.is_empty()).then(|| path.iter().copied().sum::<DVec3>() / path.len() as f64);
                if !is_part(py, t, r) {
                    set.insert(r.owner.clone());
                }
                return Ok((set, at, false));
            }
        }
    }
    // A fastener or other part placement: its junction.
    if let Some(g) = group(py, t, id) {
        if let Some(parent) = metadata(&g)?
            .get_item("built.junction_part")?
            .and_then(|p| p.get_item("junction").ok())
            .and_then(|j| j.extract::<String>().ok())
        {
            let mut top = parent;
            while let Some(up) = t.rows.get(&top).and_then(|r| r.joint.parent.clone()) {
                top = up;
            }
            return seed_info(scene, t, &json!(top));
        }
    }
    Err(err("Nothing to capture here"))
}

/// The meeting a seed belongs to: both its members first, then the nearest.
fn choose(meetings: &[meetings::Lite], set: &BTreeSet<String>, at: Option<DVec3>, exact: bool) -> Option<usize> {
    meetings
        .iter()
        .enumerate()
        .filter_map(|(i, m)| {
            // A plate on a member is a part of a meeting, not one.
            if m.classes.iter().any(|c| PART_CLASSES.contains(&c.as_str())) {
                return None;
            }
            let hits = m.members.iter().filter(|u| set.contains(*u)).count();
            if hits == 0 || (exact && hits < 2) {
                return None;
            }
            let d = at.map_or(0., |p| p.distance(DVec3::from(m.at)));
            Some((i, hits, d))
        })
        .min_by(|a, b| b.1.cmp(&a.1).then(a.2.total_cmp(&b.2)))
        .map(|(i, _, _)| i)
}

/// Gather the items at a meeting (§5): cuts on either member that follow
/// the other or a gathered part, parts that follow the members, child
/// junctions among them; until nothing is added, at most 200.
fn gather(scene: &Obj<'_>, t: &LiveTable, m: &meetings::Lite) -> PyResult<Vec<Item>> {
    let py = scene.py();
    let members: BTreeSet<String> = m.members.iter().cloned().collect();
    let size = m.dims.iter().flat_map(|d| [d.get("width"), d.get("height")]).flatten().fold(0f64, |a, b| a.max(*b)) / 1000.;
    let reach = (size * 1.5).max(0.4);
    let at = DVec3::from(m.at);
    let mut parts: BTreeSet<String> = BTreeSet::new();
    let mut items: Vec<Item> = Vec::new();
    let mut taken: HashSet<String> = HashSet::new();
    let name = |uid: &str| name_of(py, t, uid);
    loop {
        let mut added = false;
        let known: BTreeSet<String> = members.union(&parts).cloned().collect();
        for r in t.by_creation() {
            if items.len() >= CAP {
                break;
            }
            if taken.contains(&r.uid) || r.junction.is_some() || r.joint.parent.is_some() {
                continue;
            }
            match r.kind.as_str() {
                "cut" if members.contains(&r.owner) => {
                    let other = refs(t, &r);
                    if other.is_disjoint(&known) {
                        continue;
                    }
                    let partner = other.iter().find(|u| known.contains(*u)).cloned().unwrap_or_default();
                    let what = match &r.cutter {
                        Some(cut::Cutter::Plane(_)) => "plane",
                        Some(cut::Cutter::Bisector(_)) => "mitre",
                        Some(cut::Cutter::Body(_)) => "body",
                        Some(cut::Cutter::Hole(_)) => "hole",
                        _ => "cut",
                    };
                    let offset = r.anchor.as_ref().map(|a| a.offset).or(r.clearance).or(r.offset).unwrap_or(0.);
                    let label = r.sketch_fields.name.as_str().filter(|n| !n.is_empty()).unwrap_or("Cut");
                    items.push(Item {
                        id: r.uid.clone(),
                        kind: "cut",
                        row: Some(r.uid.clone()),
                        placement: None,
                        label: format!("{label} on {}", name(&r.owner)),
                        meta: format!("{what} · {} off {}", fmt(offset), name(&partner)),
                        how: "anchored",
                        ticked: true,
                        problem: None,
                        key: String::new(),
                    });
                    taken.insert(r.uid.clone());
                    added = true;
                }
                "sweep" if !members.contains(&r.owner) && is_part(py, t, &r) => {
                    let other = refs(t, &r);
                    let path = world_path(scene, t, &r)?;
                    let centre = if path.is_empty() { None } else { Some(path.iter().copied().sum::<DVec3>() / path.len() as f64) };
                    let close = centre.is_some_and(|c| c.distance(at) <= reach);
                    let both = other.iter().filter(|u| known.contains(*u)).count();
                    let anchored = both >= 2 || (both == 1 && other.iter().any(|u| parts.contains(u)));
                    if !(anchored || (close && (both >= 1 || other.is_empty()))) {
                        continue;
                    }
                    let rests = both >= 1;
                    let s = r.sweep.as_ref().unwrap();
                    let length = path.windows(2).map(|p| p[0].distance(p[1])).sum::<f64>() * 1000.;
                    let dims = s.profile.outline(s.segments).map(|o| format!("{} × {}", fmt(o.width), fmt(o.height))).unwrap_or_default();
                    items.push(Item {
                        id: r.owner.clone(),
                        kind: "part",
                        row: Some(r.uid.clone()),
                        placement: Some(r.owner.clone()),
                        label: name(&r.owner),
                        meta: format!("{} · {dims} × {}", s.preset.clone().unwrap_or_else(|| "Part".into()), fmt(length)),
                        how: if anchored { "anchored" } else { "near" },
                        ticked: anchored || rests,
                        problem: None,
                        key: String::new(),
                    });
                    taken.insert(r.uid.clone());
                    parts.insert(r.owner.clone());
                    added = true;
                }
                "junction" => {
                    let placed: BTreeSet<String> = r.joint.roles.as_object().into_iter().flatten().filter_map(|(_, b)| placement(b).map(String::from)).collect();
                    if placed.is_empty() || !placed.is_subset(&known) || placed.is_disjoint(&parts) {
                        continue;
                    }
                    let detail = r.joint.detail["name"].as_str().unwrap_or("Junction").to_string();
                    let roles: Vec<String> = placed.iter().map(|u| name(u)).collect();
                    items.push(Item {
                        id: r.uid.clone(),
                        kind: "junction",
                        row: Some(r.uid.clone()),
                        placement: Some(r.owner.clone()),
                        label: detail,
                        meta: roles.join(" + "),
                        how: "anchored",
                        ticked: true,
                        problem: None,
                        key: String::new(),
                    });
                    taken.insert(r.uid.clone());
                    added = true;
                }
                _ => {}
            }
        }
        if !added || items.len() >= CAP {
            break;
        }
    }
    Ok(items)
}

fn fmt(v: f64) -> String {
    let r = (v * 10.).round() / 10.;
    if r.fract() == 0. {
        format!("{r:.0}")
    } else {
        format!("{r:.1}")
    }
}

/// The rule's catalogue steps for one role (± 1 step, §5).
fn role_rule(profile: &str, dims: &BTreeMap<String, f64>, sizes: Option<&BTreeMap<String, [f64; 2]>>) -> Value {
    let mut out = json!({});
    let series = profile::series(profile);
    if let Some(i) = series.iter().position(|(k, _)| k == profile) {
        let lo = i.saturating_sub(1);
        let hi = (i + 1).min(series.len() - 1);
        out["profile"] = json!(series[lo..=hi].iter().map(|(k, _)| json!({"library": k})).collect::<Vec<_>>());
        out["size"] = json!({"height": [series[lo].1, series[hi].1]});
    } else if let (Some(w), Some(h)) = (dims.get("width"), dims.get("height")) {
        let step = |v: f64| [(v * 0.9 * 10.).round() / 10., (v * 1.1 * 10.).round() / 10.];
        out["size"] = json!({"width": step(*w), "height": step(*h)});
    }
    for (dim, range) in sizes.into_iter().flatten() {
        out["size"][dim] = json!(range);
    }
    out
}

/// Relations that give `v` from a role dimension or an earlier parameter,
/// best first (§2.5): exact, then the smallest multiple-of-5 constant, then
/// × ½ or × 2.
fn relations(v: f64, roles: &[(String, String, BTreeMap<String, f64>)], earlier: &[Param]) -> Vec<(String, String)> {
    let mut found: Vec<((u8, i64), String, String)> = Vec::new();
    if v.abs() < 1e-9 {
        return Vec::new();
    }
    for p in earlier {
        if (p.value - v).abs() < 0.01 {
            found.push(((0, 1), p.key.clone(), format!("= {}", p.name)));
        }
    }
    for (role, label, dims) in roles {
        for dim in ["width", "height", "web", "flange", "thickness"] {
            let Some(d) = dims.get(dim).copied().filter(|d| *d > 0.) else { continue };
            let c = v - d;
            let words = format!("{label} {dim}");
            if c.abs() < 0.01 {
                found.push(((0, 0), format!("{role}.{dim}"), format!("= {words}")));
            } else if c.abs() <= d / 2. + 1e-9 && ((c / 5.) - (c / 5.).round()).abs() < 1e-6 {
                let sign = if c > 0. { "+" } else { "-" };
                found.push((
                    (1, (c.abs() * 10.) as i64),
                    format!("{role}.{dim} {sign} {}", fmt(c.abs())),
                    format!("= {words} {sign} {}", fmt(c.abs())),
                ));
            } else {
                for (k, text) in [(0.5, "½"), (2., "2")] {
                    if (v - d * k).abs() < 0.01 {
                        found.push(((2, 0), format!("{role}.{dim} * {k}"), format!("= {words} × {text}")));
                    }
                }
            }
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found.dedup_by(|a, b| a.1 == b.1);
    found.into_iter().map(|(_, e, w)| (e, w)).collect()
}

struct Builder<'a> {
    params: Vec<Param>,
    roles: &'a [(String, String, BTreeMap<String, f64>)],
    exprs: &'a BTreeMap<String, Value>,
}
impl Builder<'_> {
    /// A number of a captured item becomes a parameter (merged with an equal
    /// value of the same field), and its key is written in its place.
    fn param(&mut self, field: &str, name: &str, group: &str, unit: &'static str, value: f64) -> Value {
        let value = (value * 1000.).round() / 1000.;
        if let Some(p) = self.params.iter().find(|p| p.name == name && (p.value - value).abs() < 1e-6) {
            return json!(p.key);
        }
        let mut key = snake(field);
        let mut n = 2;
        while self.params.iter().any(|p| p.key == key) {
            key = format!("{}_{n}", snake(field));
            n += 1;
        }
        let found = relations(value, self.roles, &self.params);
        let (expr, relation) = match self.exprs.get(&key) {
            Some(Value::String(e)) if e.trim().is_empty() => (json!(value), String::new()),
            Some(chosen) => {
                let words = found.iter().find(|(e, _)| Value::String(e.clone()) == *chosen).map(|(_, w)| w.clone()).unwrap_or_default();
                (chosen.clone(), words)
            }
            None => match found.first() {
                Some((e, w)) => (json!(e), w.clone()),
                None => (json!(value), String::new()),
            },
        };
        self.params.push(Param {
            key: key.clone(),
            name: name.into(),
            group: group.into(),
            unit,
            value,
            expr,
            relation,
            alternatives: found.into_iter().map(|(e, _)| e).collect(),
        });
        json!(key)
    }
}

fn axis_of(m: &DMat4, i: usize) -> DVec3 {
    [m.x_axis, m.y_axis, m.z_axis][i].truncate().normalize_or_zero()
}

/// Convert a gathered plane cut: the frame whose Z is the cut's normal and
/// the offset that keeps the plane where it is.
fn plane_anchor(scene: &Obj<'_>, t: &LiveTable, r: &Row, frames: &BTreeMap<String, DMat4>, prefer: &[String]) -> PyResult<Result<(String, f64), String>> {
    let Some(anchor) = &r.anchor else {
        return Ok(Err("not anchored".into()));
    };
    let Some(o) = reference::origin(scene, t, &anchor.reference, r)? else {
        return Ok(Err("anchor not found".into()));
    };
    let n = match reference::element_points(scene, t, r, &anchor.reference, Kind::Face)? {
        Some(face) => crate::snap_planes::face_plane(face.iter().map(|p| p.to_array()).collect()).map(|(_, n)| DVec3::from(n)),
        None => match &anchor.reference {
            Reference::Frame { junction, frame } => local_frame(t, &r.owner, junction, frame).map(|m| m.z_axis.truncate()),
            _ => match &r.cutter {
                Some(cut::Cutter::Plane(p)) => p.normal.map(DVec3::from),
                _ => None,
            },
        },
    };
    let Some(n) = n.and_then(|n| n.try_normalize()) else {
        return Ok(Err("plane not found".into()));
    };
    let m = world(t, &r.owner);
    let (o, n) = (m.transform_point3(o), m.transform_vector3(n).normalize_or_zero());
    for name in prefer {
        let Some(f) = frames.get(name) else { continue };
        let z = axis_of(f, 2);
        let dot = z.dot(n);
        if dot > 1. - 1e-6 {
            return Ok(Ok((name.clone(), (o - f.w_axis.truncate()).dot(z) * 1000. + anchor.offset)));
        }
    }
    for name in prefer {
        let Some(f) = frames.get(name) else { continue };
        let z = axis_of(f, 2);
        if z.dot(n) < -1. + 1e-6 && r.operation.as_deref() == Some("trim") {
            return Ok(Ok((name.clone(), (o - f.w_axis.truncate()).dot(z) * 1000. - anchor.offset)));
        }
    }
    Ok(Err("its plane is not a face of the meeting".into()))
}

/// Converting gathered items into template contents against role frames.
struct Convert<'a> {
    frames: &'a BTreeMap<String, DMat4>,
    members: &'a [String; 2],
    roles: &'a serde_json::Map<String, Value>,
    /// Part placement → its content key.
    keys: HashMap<String, String>,
    used: HashSet<String>,
}
impl Convert<'_> {
    fn unique(&mut self, base: &str) -> String {
        let mut k = base.to_string();
        let mut n = 2;
        while !self.used.insert(k.clone()) {
            k = format!("{base}_{n}");
            n += 1;
        }
        k
    }
    fn role_of(&self, uid: &str) -> Option<&'static str> {
        if uid == self.members[0] {
            Some("main")
        } else if uid == self.members[1] {
            Some("joining")
        } else {
            None
        }
    }
    /// One item: its key, its template content and this use's entry.
    #[allow(clippy::type_complexity)]
    fn item(&mut self, scene: &Obj<'_>, t: &LiveTable, row: &Row, kind: &str, b: &mut Builder<'_>) -> PyResult<Result<(String, Value, Value), String>> {
        let py = scene.py();
        let meet = self.frames.get("meet").copied().ok_or_else(|| err("No meeting frame"))?;
        match kind {
            "part" => {
                let Some(s) = row.sweep.as_deref() else { return Ok(Err("not a swept part".into())) };
                let path = world_path(scene, t, row)?;
                if path.len() != 2 {
                    return Ok(Err("only straight two-point parts".into()));
                }
                let local = reference::sweep_path(scene, t, row, &s.path)?.unwrap_or_default();
                let dir_l = (DVec3::from(local[1]) - DVec3::from(local[0])).normalize_or_zero();
                let across_l = DVec3::Z.cross(dir_l).try_normalize().unwrap_or(DVec3::X);
                let up_l = dir_l.cross(across_l);
                let pm = world(t, &row.owner);
                let inv = meet.inverse();
                let basis = [across_l, up_l, dir_l].map(|v| inv.transform_vector3(pm.transform_vector3(v)).normalize_or_zero());
                let offset = inv.transform_point3(path[0]) * 1000.;
                let preset = s.preset_uid.as_deref().and_then(|u| presets::lookup(t, u)).filter(|p| p.primitive == "sweep");
                let base_name = preset.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| "Part".into());
                let key = self.unique(&snake(&base_name));
                self.keys.insert(row.owner.clone(), key.clone());
                let group = base_name.clone();
                let mut fields = serde_json::Map::new();
                let thickness = path[0].distance(path[1]) * 1000.;
                fields.insert("thickness".into(), b.param(&format!("{key}_t"), &format!("{group} thickness"), &group, "mm", thickness));
                for (k, v) in &s.profile.params {
                    fields.insert(format!("profile.params.{k}"), b.param(&format!("{key}_{k}"), &format!("{group} {k}"), &group, "mm", *v));
                }
                let mut at_offset = Vec::new();
                for (axis, v) in ["x", "y", "z"].iter().zip(offset.to_array()) {
                    at_offset.push(if v.abs() < 0.05 { json!(0.) } else { b.param(&format!("{key}_{axis}"), &format!("{group} offset {axis}"), &group, "mm", v) });
                }
                let mut part = json!({
                    "fields": fields,
                    "at": {"frame": "@meet", "offset": at_offset,
                           "basis": basis.iter().flat_map(|v| v.to_array()).map(|x| (x * 1e9).round() / 1e9).collect::<Vec<_>>()},
                    "rotation": s.rotation, "mirror": s.mirror, "attachment": s.attachment,
                });
                match &preset {
                    Some(p) => part["preset"] = json!(p.uid),
                    None => {
                        let mut profile = json!(s.profile);
                        profile.as_object_mut().map(|o| o.remove("params"));
                        part["sweep"] = json!({"profile": profile, "segments": s.segments});
                        let r = record(py, t, &row.owner);
                        let keep: serde_json::Map<String, Value> = ["class", "predefined_type", "material", "object_type"]
                            .iter()
                            .filter_map(|k| Some((k.to_string(), r.get(*k)?.clone())))
                            .collect();
                        part["record"] = json!(keep);
                    }
                }
                Ok(Ok((key.clone(), json!({"key": key, "part": part}), json!({"key": key, "row": row.uid, "placement": row.owner}))))
            }
            "cut" => {
                let Some(on) = self.role_of(&row.owner) else { return Ok(Err("it cuts something outside this meeting".into())) };
                let base_key = if row.operation.as_deref() == Some("trim") && row.end.is_some() { "end_cut" } else { "cut" };
                let group = if base_key == "end_cut" { "End cut" } else { "Cut" };
                let mut v = json!({"kind": "cut", "operation": row.operation, "on": row.on.unwrap_or(true)});
                if let Some(end) = &row.end {
                    v["end"] = if self.roles[on]["end"].as_str() == Some(end.as_str()) { json!(format!("@{on}.end")) } else { json!(end) };
                }
                if let Some(name) = row.sketch_fields.name.as_str() {
                    v["name"] = json!(name);
                }
                let other = if on == "main" { "joining" } else { "main" };
                match &row.cutter {
                    Some(cut::Cutter::Plane(p)) => {
                        let prefer: Vec<String> = ["meet".to_string(), format!("{other}.contact"), format!("{on}.contact"), format!("{on}.end")].to_vec();
                        match plane_anchor(scene, t, row, self.frames, &prefer)? {
                            Ok((frame, offset)) => {
                                v["cutter"] = json!({"plane": {"angle": p.angle}});
                                v["anchor"] = json!({"ref": format!("@{frame}"), "offset": b.param("gap", &format!("{group} gap"), group, "mm", offset)});
                            }
                            Err(why) => return Ok(Err(why)),
                        }
                    }
                    Some(cut::Cutter::Bisector(uid) | cut::Cutter::Body(uid)) => {
                        let kind = if matches!(row.cutter, Some(cut::Cutter::Bisector(_))) { "bisector" } else { "body" };
                        let Some(with) = self.role_of(uid) else {
                            return Ok(Err("its cutter is not a member of this meeting".into()));
                        };
                        v["cutter"] = json!({kind: format!("@{with}")});
                        v["clearance"] = b.param("clearance", &format!("{group} clearance"), group, "mm", row.clearance.unwrap_or(0.));
                    }
                    _ => return Ok(Err("this cutter kind is not portable".into())),
                }
                let key = self.unique(base_key);
                Ok(Ok((key.clone(), json!({"key": key, "on": on, "row": v}), json!({"key": key, "row": row.uid}))))
            }
            "junction" => {
                let Some(detail) = row.joint.detail["uid"].as_str().and_then(|u| presets::lookup(t, u)) else {
                    return Ok(Err("a custom junction: save it as a detail first".into()));
                };
                let mut child_roles = serde_json::Map::new();
                for (name, binding) in row.joint.roles.as_object().into_iter().flatten() {
                    let uid = placement(binding).unwrap_or_default().to_string();
                    let target = match (self.role_of(&uid), self.keys.get(&uid)) {
                        (Some(role), _) => role.to_string(),
                        (None, Some(k)) => k.clone(),
                        _ => return Ok(Err("it joins something outside this meeting".into())),
                    };
                    let mut extra = json!({"placement": format!("@{target}")});
                    for f in ["face", "end"] {
                        if !binding[f].is_null() && self.role_of(&uid).is_none_or(|r| self.roles[r][f] != binding[f]) {
                            extra[f] = binding[f].clone();
                        }
                    }
                    child_roles.insert(name.clone(), if extra.as_object().is_some_and(|o| o.len() == 1) { json!(format!("@{target}")) } else { extra });
                }
                let key = self.unique(&snake(&detail.name));
                let mut params = serde_json::Map::new();
                for def in detail.values.get("params").and_then(Value::as_array).into_iter().flatten() {
                    let Some(pk) = def["key"].as_str() else { continue };
                    let value = row.joint.overrides.get(pk).and_then(Value::as_f64).or_else(|| row.joint.params.get(pk).and_then(Value::as_f64));
                    if let Some(value) = value {
                        let label = def["name"].as_str().unwrap_or(pk);
                        let unit = match def["unit"].as_str() { Some("deg") => "deg", Some("count") => "count", _ => "mm" };
                        params.insert(pk.into(), b.param(&format!("{key}_{pk}"), &format!("{} {label}", detail.name), &detail.name, unit, value));
                    }
                }
                Ok(Ok((key.clone(), json!({"key": key, "junction": {"detail": detail.uid, "roles": child_roles, "params": params}}), json!({"key": key, "junction": row.uid, "placement": row.owner}))))
            }
            _ => Ok(Err("not a cut, part or junction".into())),
        }
    }
}

/// Gather and draft, without writing anything.
pub(in crate::live) fn draft(scene: &Obj<'_>, t: &LiveTable, seed: &Value, opts: &Options) -> PyResult<Draft> {
    let py = scene.py();
    let all = meetings::lite(scene)?;
    let (set, at, exact) = seed_info(scene, t, seed)?;
    let i = choose(&all, &set, at, exact).ok_or_else(|| err("No meeting of two members here"))?;
    let meeting = all[i].clone();
    let existing = t.junctions.iter().filter_map(|id| t.rows.get(id)).filter(|j| j.joint.parent.is_none()).any(|j| {
        let placed: BTreeSet<String> = j.joint.roles.as_object().into_iter().flatten().filter_map(|(_, b)| placement(b).map(String::from)).collect();
        meeting.members.iter().all(|u| placed.contains(u)) && placed.len() == 2
    });
    if existing {
        return Err(err("These members already have a junction: edit it instead"));
    }
    let mut items = gather(scene, t, &meeting)?;
    if let Some(include) = &opts.include {
        for item in &mut items {
            item.ticked = include.contains(&item.id);
        }
    }
    let (mi, ji) = if opts.swap { (1, 0) } else { (0, 1) };
    let members = [meeting.members[mi].clone(), meeting.members[ji].clone()];
    let names = [name_of(py, t, &members[0]), name_of(py, t, &members[1])];
    let mut roles = serde_json::Map::new();
    let mut main = meeting.roles[mi].clone();
    let mut joining = meeting.roles[ji].clone();
    if opts.swap {
        main.as_object_mut().map(|o| o.remove("face"));
        joining.as_object_mut().map(|o| o.remove("face"));
    }
    roles.insert("main".into(), main);
    roles.insert("joining".into(), joining);
    // Frames from the members' stock, as the junction will resolve them.
    let probe: Row = serde_json::from_value(json!({
        "kind": "junction", "uid": "", "owner": members[0], "roles": roles, "meeting": meeting.kind,
    }))
    .map_err(err)?;
    let (frames, dims) = geometry(scene, t, &probe)?.ok_or_else(|| err("The members' stock does not resolve"))?;
    let base = world(t, &members[0]);
    let frames: BTreeMap<String, DMat4> = frames.iter().map(|(k, v)| (k.clone(), base * DMat4::from_cols_array(v))).collect();
    let role_dims = |role: &str| -> BTreeMap<String, f64> {
        dims.iter().filter_map(|(k, v)| Some((k.strip_prefix(&format!("{role}."))?.to_string(), *v))).collect()
    };
    let role_list = vec![
        ("main".to_string(), names[0].clone(), role_dims("main")),
        ("joining".to_string(), names[1].clone(), role_dims("joining")),
    ];
    let mut b = Builder { params: Vec::new(), roles: &role_list, exprs: &opts.exprs };
    let mut contents = Vec::new();
    let mut template = Vec::new();
    let mut ctx = Convert { frames: &frames, members: &members, roles: &roles, keys: HashMap::new(), used: HashSet::new() };
    // Parts first: cuts and child junctions may refer to them by key.
    let order: Vec<usize> = (0..items.len())
        .filter(|i| items[*i].kind == "part")
        .chain((0..items.len()).filter(|i| items[*i].kind == "cut"))
        .chain((0..items.len()).filter(|i| items[*i].kind == "junction"))
        .collect();
    for i in order {
        if !items[i].ticked {
            continue;
        }
        let row = t.rows.get(items[i].row.as_deref().unwrap_or("")).cloned().ok_or_else(|| err("Missing gathered row"))?;
        match ctx.item(scene, t, &row, items[i].kind, &mut b)? {
            Ok((key, tpl, content)) => {
                items[i].key = key;
                template.push(tpl);
                contents.push(content);
            }
            Err(why) => {
                items[i].problem = Some(why);
                items[i].ticked = false;
            }
        }
    }
    let kept = template.len();
    let what = if template.iter().any(|c| c.get("part").is_some()) {
        "End plate"
    } else if template.iter().any(|c| c.get("junction").is_some()) {
        "Fastened joint"
    } else {
        "End cut"
    };
    let label = |p: &str, n: &str| profile::label(p).map(String::from).unwrap_or_else(|| n.to_string());
    let default_name = format!("{what} · {} to {}", label(&meeting.profiles[ji], &names[1]), label(&meeting.profiles[mi], &names[0]));
    let name = opts.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or(default_name);
    let mut rule = json!({
        "meeting": meeting.kind,
        "roles": {
            "main": role_rule(&meeting.profiles[mi], &meeting.dims[mi], opts.sizes.get("main")),
            "joining": role_rule(&meeting.profiles[ji], &meeting.dims[ji], opts.sizes.get("joining")),
        },
        "angle": {"value": (meeting.angle * 10.).round() / 10., "tol": opts.tol.unwrap_or(2.)},
        "mirror": opts.mirror.clone().unwrap_or_else(|| "allow".into()),
        "hand": meeting.hand,
        "direction": "ignore",
        "offset": {"value": (meeting.offset * 10.).round() / 10., "tol_mm": 5},
    });
    if !opts.swap && !meeting.face.is_empty() && meeting.kind == "end_face" {
        rule["roles"]["main"]["face"] = json!([meeting.face]);
    }
    let params: Vec<Value> = b
        .params
        .iter()
        .map(|p| json!({"key": p.key, "name": p.name, "group": p.group, "expr": p.expr, "unit": p.unit}))
        .collect();
    let uid = opts.uid.clone().unwrap_or_else(eid::new_uid);
    let detail = presets::Preset {
        uid,
        name,
        pack: "Project".into(),
        primitive: "junction".into(),
        values: BTreeMap::from([
            ("meeting".into(), json!(meeting.kind)),
            ("roles".into(), json!({"main": {"name": names[0]}, "joining": {"name": names[1]}})),
            ("params".into(), json!(params)),
            ("contents".into(), json!(template)),
            ("rule".into(), rule),
        ]),
        record: BTreeMap::from([("class".into(), json!("IfcElementAssembly"))]),
        naming: String::new(),
        node: String::new(),
        locks: BTreeMap::new(),
        source: None,
        rev: 1,
    };
    let paint = [
        frames.get("main.contact").map(|f| f.w_axis.truncate().to_array()).unwrap_or(meeting.at),
        frames.get("joining.end").map(|f| f.w_axis.truncate().to_array()).unwrap_or(meeting.at),
    ];
    let mut draft = Draft {
        detail,
        roles,
        contents,
        items,
        params: b.params,
        members: members.clone(),
        names,
        paint,
        meeting,
        similar: 0,
        near: 0,
    };
    if kept > 0 {
        for (key, fit, _, _) in meetings::draft_fits(scene, &draft.detail)? {
            if key == draft.meeting.key {
                continue;
            }
            match fit {
                "match" => draft.similar += 1,
                "near" => draft.near += 1,
                _ => {}
            }
        }
    }
    Ok(draft)
}

/// Write each moved placement's current GlobalId into its record, so moving
/// it under the junction's assembly keeps it (§5, persistent-ids §7.1).
fn keep_guid(scene: &Obj<'_>, g: &Obj<'_>) -> PyResult<()> {
    let py = scene.py();
    let record = g.getattr("ifc")?;
    let record = if record.is_none() { PyDict::new(py) } else { record.cast::<PyDict>()?.copy()? };
    if record.get_item("guid")?.is_some_and(|v| !v.is_none()) {
        return Ok(());
    }
    let path = scene.call_method1("occurrence_path", (g,))?;
    let guid = py.import("core.ifc_identity")?.call_method1("occurrence_guid", (scene, path))?;
    record.set_item("guid", guid)?;
    g.setattr("ifc", record)
}

fn keep_guids(scene: &Obj<'_>, g: &Obj<'_>) -> PyResult<()> {
    keep_guid(scene, g)?;
    for child in g.getattr("children")?.try_iter()? {
        keep_guids(scene, &child?)?;
    }
    Ok(())
}

/// Move a placement under the junction's assembly, keeping its world pose.
pub(super) fn reparent(scene: &Obj<'_>, t: &mut LiveTable, uid: &str, assembly: &str) -> PyResult<()> {
    let py = scene.py();
    let (Some(g), Some(a)) = (group(py, t, uid), group(py, t, assembly)) else {
        return Err(err("Missing placement to move"));
    };
    let parent = doc::lock().node(t.groups[uid]).and_then(|n| n.owners.first().copied());
    if parent == Some(t.groups[assembly]) {
        return Ok(());
    }
    keep_guids(scene, &g)?;
    let pose = reference::frame(t.groups[uid]);
    let siblings = match parent.and_then(|k| doc::lock().handle_ref(py, k)) {
        Some(p) => p.bind(py).getattr("children")?,
        None => scene.getattr("groups")?,
    };
    siblings.call_method1("remove", (&g,))?;
    let local = reference::frame(t.groups[assembly]).inverse() * pose;
    g.setattr("xform", crate::math::Matrix4::from(local))?;
    a.getattr("children")?.call_method1("append", (&g,))?;
    Arc::make_mut(&mut t.groups).insert(uid.to_string(), g.getattr("_doc_key")?.extract()?);
    Ok(())
}

/// Conversion (§5), one History step: the detail, the junction row owning
/// the gathered items under their template keys, their uids kept.
pub(in crate::live) fn convert(scene: &Obj<'_>, t: &mut LiveTable, d: &Draft) -> PyResult<String> {
    if d.contents.is_empty() {
        return Err(err("Tick at least one cut or part to save"));
    }
    d.detail.validate()?;
    Arc::make_mut(&mut t.presets).insert(d.detail.uid.clone(), d.detail.clone());
    let uid = eid::new_uid();
    // The gathered rows are owned before expansion, so it adopts them.
    for c in &d.contents {
        let key = c["key"].as_str().unwrap_or_default();
        let id = c["row"].as_str().or_else(|| c["junction"].as_str()).unwrap_or_default();
        let Some(r) = t.rows.get(id).cloned() else { continue };
        let mut r = (*r).clone();
        r.junction = Some(uid.clone());
        r.key = Some(key.into());
        if r.kind == "junction" {
            r.joint.parent = Some(uid.clone());
        }
        t.insert(r).map_err(err)?;
    }
    let row: Row = serde_json::from_value(json!({
        "kind": "junction", "uid": uid, "detail": {"uid": d.detail.uid},
        "roles": d.roles, "contents": d.contents,
    }))
    .map_err(err)?;
    t.adopt = d.contents.iter().filter_map(|c| c["placement"].as_str().map(String::from)).collect();
    let result = super::super::set_row(scene, t, &row);
    t.adopt.clear();
    result?;
    Ok(uid)
}

/// The capture chip's candidates: meetings with two or more items that no
/// junction owns, keyed by meeting, with their item ids.
pub(in crate::live) fn offers(scene: &Obj<'_>, t: &LiveTable) -> PyResult<Vec<(meetings::Lite, Vec<String>)>> {
    let py = scene.py();
    let loose: Vec<&Arc<Row>> = t
        .rows
        .values()
        .filter(|r| r.junction.is_none() && r.joint.parent.is_none())
        .filter(|r| r.kind == "cut" || is_part(py, t, r))
        .collect();
    if loose.is_empty() {
        return Ok(Vec::new());
    }
    let all = meetings::lite(scene)?;
    let mut by_member: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, m) in all.iter().enumerate() {
        for u in &m.members {
            by_member.entry(u.as_str()).or_default().push(i);
        }
    }
    let mut found: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for r in loose {
        let (set, at, exact) = seed_info(scene, t, &json!(r.uid))?;
        let candidates: Vec<meetings::Lite> = set
            .iter()
            .flat_map(|u| by_member.get(u.as_str()).into_iter().flatten())
            .map(|i| all[*i].clone())
            .collect();
        if let Some(j) = choose(&candidates, &set, at, exact) {
            let key = &candidates[j].key;
            if let Some(i) = all.iter().position(|m| &m.key == key) {
                found.entry(i).or_default().push(r.uid.clone());
            }
        }
    }
    let detailed: Vec<BTreeSet<String>> = t
        .junctions
        .iter()
        .filter_map(|id| t.rows.get(id))
        .filter(|j| j.joint.parent.is_none())
        .map(|j| j.joint.roles.as_object().into_iter().flatten().filter_map(|(_, b)| placement(b).map(String::from)).collect())
        .collect();
    Ok(found
        .into_iter()
        .filter(|(i, ids)| ids.len() >= 2 && !detailed.iter().any(|p| all[*i].members.iter().all(|u| p.contains(u))))
        .map(|(i, ids)| (all[i].clone(), ids))
        .collect())
}

/// Adopt rows or parts into a junction (§2.4, §6.3): a cut dragged onto it
/// or what was made inside it. This use only (`local.added`, a cut's anchor
/// rewritten to the junction's frame), or its detail (a revision every use
/// follows; this use keeps the items' uids).
pub(in crate::live) fn adopt(scene: &Obj<'_>, t: &mut LiveTable, uid: &str, ids: &[String], to_detail: bool) -> PyResult<()> {
    let j = t.rows.get(uid).filter(|r| r.kind == "junction").cloned().ok_or_else(|| err("Unknown junction"))?;
    let p = detail(t, &j)?;
    if to_detail && j.joint.detail.is_null() {
        return Err(err("A custom junction has no detail: save it as one first"));
    }
    let main = j.joint.roles.get("main").or_else(|| j.joint.roles.get("host")).and_then(placement).map(String::from).unwrap_or_default();
    let other = j
        .joint
        .roles
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(_, b)| placement(b))
        .find(|u| *u != main)
        .map(String::from)
        .unwrap_or_default();
    let members = [main, other];
    let (frames, dims) = geometry(scene, t, &j)?.ok_or_else(|| err("The junction's members do not resolve"))?;
    let base = world(t, &j.owner);
    let frames: BTreeMap<String, DMat4> = frames.iter().map(|(k, v)| (k.clone(), base * DMat4::from_cols_array(v))).collect();
    let py = scene.py();
    let role_dims = |role: &str| -> BTreeMap<String, f64> {
        dims.iter().filter_map(|(k, v)| Some((k.strip_prefix(&format!("{role}."))?.to_string(), *v))).collect()
    };
    let role_list = vec![
        ("main".to_string(), name_of(py, t, &members[0]), role_dims("main")),
        ("joining".to_string(), name_of(py, t, &members[1]), role_dims("joining")),
    ];
    let defs: Vec<Value> = p.values.get("params").and_then(Value::as_array).cloned().unwrap_or_default();
    let earlier: Vec<Param> = defs
        .iter()
        .filter_map(|d| {
            let key = d["key"].as_str()?.to_string();
            Some(Param {
                value: j.joint.params.get(&key).and_then(Value::as_f64).unwrap_or(f64::NAN),
                name: d["name"].as_str().unwrap_or(&key).to_string(),
                group: d["group"].as_str().unwrap_or_default().to_string(),
                unit: "mm",
                expr: d["expr"].clone(),
                relation: String::new(),
                alternatives: Vec::new(),
                key,
            })
        })
        .collect();
    let exprs = BTreeMap::new();
    let mut b = Builder { params: earlier.clone(), roles: &role_list, exprs: &exprs };
    let roles = j.joint.roles.as_object().cloned().unwrap_or_default();
    let mut ctx = Convert { frames: &frames, members: &members, roles: &roles, keys: HashMap::new(), used: HashSet::new() };
    let mut template: Vec<Value> = p.values.get("contents").and_then(Value::as_array).cloned().unwrap_or_default();
    for c in template.iter().chain(j.joint.contents.as_array().into_iter().flatten()) {
        if let Some(k) = c["key"].as_str() {
            ctx.used.insert(k.to_string());
        }
        if let (Some(k), Some(pl)) = (c["key"].as_str(), c["placement"].as_str()) {
            ctx.keys.insert(pl.to_string(), k.to_string());
        }
    }
    let mut jr = (*j).clone();
    let mut contents = jr.joint.contents.as_array().cloned().unwrap_or_default();
    let mut added = jr.joint.local["added"].as_array().cloned().unwrap_or_default();
    for id in ids {
        let (kind, row) = match t.rows.get(id) {
            Some(r) if r.kind == "cut" => ("cut", r.clone()),
            Some(r) if r.kind == "junction" => ("junction", r.clone()),
            _ => match t.owners.get(id).and_then(|r| t.rows.get(r)) {
                Some(r) if r.kind == "sweep" => ("part", r.clone()),
                _ => return Err(err("Only cuts, swept parts and junctions can join a junction")),
            },
        };
        if row.junction.as_deref().is_some_and(|o| o != uid) || row.joint.parent.as_deref().is_some_and(|o| o != uid) {
            return Err(err("It already belongs to another junction"));
        }
        let local = added.iter().position(|a| a == &json!(row.uid));
        if row.junction.as_deref() == Some(uid) {
            // Made inside this use (local): "Apply to all" moves it into the detail.
            let (Some(at), true) = (local, to_detail) else { continue };
            added.remove(at);
            contents.retain(|c| c["row"] != json!(row.uid) && c["junction"] != json!(row.uid));
            ctx.used.remove(row.key.as_deref().unwrap_or_default());
        }
        if row.uid == uid {
            return Err(err("Junction ownership cycle"));
        }
        let (key, tpl, content) = match ctx.item(scene, t, &row, kind, &mut b)? {
            Ok(done) => done,
            Err(why) => return Err(err(format!("Cannot adopt it: {why}"))),
        };
        let mut owned = (*row).clone();
        owned.junction = Some(uid.into());
        owned.key = Some(key.clone());
        if kind == "junction" {
            owned.joint.parent = Some(uid.into());
        }
        if to_detail {
            template.push(tpl);
            if let Some(pl) = content["placement"].as_str() {
                t.adopt.insert(pl.into());
            }
        } else {
            if kind == "cut" {
                // This use only: the cut follows the junction's frame.
                if let Some(frame) = tpl["row"]["anchor"]["ref"].as_str() {
                    let frame = frame.trim_start_matches('@').to_string();
                    let offset = match &tpl["row"]["anchor"]["offset"] {
                        Value::String(k) => b.params.iter().find(|p| &p.key == k).map_or(0., |p| p.value),
                        v => v.as_f64().unwrap_or(0.),
                    };
                    owned.anchor = Some(cut::Anchor { reference: Reference::Frame { junction: uid.into(), frame }, offset });
                    owned.inputs.clear();
                }
            } else if let Some(pl) = content["placement"].as_str() {
                let assembly = jr.owner.clone();
                reparent(scene, t, pl, &assembly)?;
            }
            added.push(json!(row.uid));
        }
        if owned.kind == "cut" {
            cut::attach(scene, t, &mut owned)?;
        }
        t.insert(owned).map_err(err)?;
        t.pending.insert(row.uid.clone());
        contents.push(content);
    }
    jr.joint.contents = json!(contents);
    if !added.is_empty() {
        jr.joint.local["added"] = json!(added);
    }
    t.insert(jr).map_err(err)?;
    t.pending.insert(uid.into());
    if to_detail {
        let mut next = p.clone();
        next.values.insert("contents".into(), json!(template));
        let mut defs = defs;
        for q in b.params.iter().skip(earlier.len()) {
            defs.push(json!({"key": q.key, "name": q.name, "group": q.group, "expr": q.expr, "unit": q.unit}));
        }
        next.values.insert("params".into(), json!(defs));
        let result = presets::update(scene, t, &[next]);
        t.adopt.clear();
        result?;
    }
    Ok(())
}

/// Words for an expression: role names instead of `main.`/`joining.`.
fn words(expr: &Value, names: &BTreeMap<String, String>, params: &[Value]) -> String {
    let Some(text) = expr.as_str() else { return String::new() };
    let mut out = text.to_string();
    for (role, name) in names {
        out = out.replace(&format!("{role}."), &format!("{name} "));
    }
    for p in params {
        if let (Some(k), Some(n)) = (p["key"].as_str(), p["name"].as_str()) {
            if out.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == k) {
                out = out.replace(k, n);
            }
        }
    }
    format!("= {}", out.replace('*', "×"))
}

/// The junction record (§11): detail, uses, roles, parameters with their
/// relation and override, contents, status with its ways out. `uid`: the
/// junction row or its assembly placement.
pub(in crate::live) fn record_of(scene: &Obj<'_>, t: &LiveTable, uid: &str) -> PyResult<Value> {
    let py = scene.py();
    let j = t
        .rows
        .get(uid)
        .or_else(|| t.owners.get(uid).and_then(|r| t.rows.get(r)))
        .filter(|r| r.kind == "junction")
        .cloned()
        .ok_or_else(|| err("Unknown junction"))?;
    let p = detail(t, &j).ok();
    let detail_uid = j.joint.detail["uid"].as_str().unwrap_or_default().to_string();
    let uses: Vec<&Arc<Row>> = t
        .junctions
        .iter()
        .filter_map(|id| t.rows.get(id))
        .filter(|r| !detail_uid.is_empty() && r.joint.detail["uid"] == detail_uid.as_str())
        .collect();
    let mut names = BTreeMap::new();
    let mut roles = Vec::new();
    for (name, b) in j.joint.roles.as_object().into_iter().flatten() {
        let member = placement(b).map(String::from).or_else(|| {
            let row = b["row"].as_str()?;
            Some(t.rows.get(row)?.owner.clone())
        });
        let label = member.as_deref().map(|m| name_of(py, t, m)).unwrap_or_else(|| "—".into());
        let profile = member
            .as_deref()
            .and_then(|m| t.owners.get(m))
            .and_then(|r| t.rows.get(r))
            .and_then(|r| r.sweep.as_ref())
            .and_then(|s| profile::label(&s.profile.library).map(String::from))
            .unwrap_or_default();
        let how = match (b["face"].as_str(), b["end"].as_str()) {
            (Some(f), _) => f.replace('_', " "),
            (None, Some(e)) => format!("{e} end"),
            _ => String::new(),
        };
        names.insert(name.clone(), label.clone());
        roles.push(json!({"role": name, "member": member, "name": label, "profile": profile, "how": how}));
    }
    let defs: Vec<Value> = p.as_ref().and_then(|p| p.values.get("params")).and_then(Value::as_array).cloned().unwrap_or_default();
    let overridden: Vec<&String> = j.joint.overrides.as_object().into_iter().flatten().map(|(k, _)| k).collect();
    let mut params = Vec::new();
    for d in &defs {
        let Some(key) = d["key"].as_str() else { continue };
        if d["hidden"] == true {
            continue;
        }
        let over = j.joint.overrides.get(key).filter(|v| !v.is_null());
        let differing = uses.iter().filter(|u| u.joint.overrides.get(key).is_some()).count();
        params.push(json!({
            "key": key, "name": d["name"].as_str().unwrap_or(key), "group": d["group"],
            "unit": d["unit"].as_str().unwrap_or("mm"), "expr": d["expr"],
            "relation": words(&d["expr"], &names, &defs),
            "value": j.joint.params.get(key), "override": over, "overridden_uses": differing,
            "lock": d["lock"],
        }));
    }
    for derived in ["length", "area"] {
        if let Some(v) = j.joint.params.get(derived) {
            params.push(json!({"key": derived, "name": derived, "unit": if derived == "area" {"mm²"} else {"mm"}, "value": v, "derived": true}));
        }
    }
    let mut contents = Vec::new();
    for c in j.joint.contents.as_array().into_iter().flatten() {
        let key = c["key"].as_str().unwrap_or_default();
        let (label, class) = if let Some(child) = c["junction"].as_str().and_then(|id| t.rows.get(id)) {
            (child.joint.detail["name"].as_str().unwrap_or("Junction").to_string(), "IfcElementAssembly".to_string())
        } else if let Some(pl) = c["placement"].as_str() {
            (name_of(py, t, pl), record(py, t, pl)["class"].as_str().unwrap_or_default().to_string())
        } else if let Some(row) = c["row"].as_str().and_then(|id| t.rows.get(id)) {
            let target = name_of(py, t, &row.owner);
            (format!("{} on {target}", row.sketch_fields.name.as_str().unwrap_or("Cut")), "cut".to_string())
        } else {
            (key.to_string(), String::new())
        };
        let local = j.joint.local["added"].as_array().is_some_and(|a| a.iter().any(|v| *v == c["row"] || *v == c["junction"]));
        contents.push(json!({"key": key, "label": label, "class": class, "placement": c["placement"], "row": c["row"], "junction": c["junction"], "local": local}));
    }
    let removed = j.joint.local["removed"].clone();
    let along = &j.joint.along;
    let stations = if along.is_null() { Value::Null } else { json!(t.owned.get(&j.uid).map_or(0, |o| o.len())) };
    let parent = j.joint.parent.as_deref().and_then(|id| t.rows.get(id)).map(|r| json!({"uid": r.uid, "name": r.joint.detail["name"], "along": !r.joint.along.is_null()}));
    let rule = p.as_ref().and_then(|p| p.values.get("rule")).cloned().unwrap_or(Value::Null);
    Ok(json!({
        "uid": j.uid, "owner": j.owner, "state": j.state, "reason": j.joint.reason,
        "detail": j.joint.detail, "rev": p.as_ref().map(|p| p.rev), "uses": uses.len(),
        "outside_uses": uses.iter().filter(|u| u.state == "outside").count(),
        "roles": roles, "params": params, "overrides": overridden, "contents": contents,
        "removed": removed, "forced": j.joint.local["forced"] == true, "along": along,
        "stations": stations, "parent": parent, "rule": rule, "meeting": j.joint.meeting,
    }))
}
