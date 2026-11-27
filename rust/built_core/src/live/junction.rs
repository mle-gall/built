// SPDX-License-Identifier: GPL-3.0-or-later
//! Junctions read member stock and own ordinary recipe rows.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
pub(super) type Frames = BTreeMap<String, [f64; 16]>;
fn err(s: impl ToString) -> PyErr {
    PyValueError::new_err(s.to_string())
}
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Fields {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meeting: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub detail: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub roles: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub overrides: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub reason: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub contents: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub bindings: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub local: Value,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Stock {
    pub path: Vec<[f64; 3]>,
    pub dims: expr::Values,
    pub layout: Option<repeat::Layout>,
    pub state: &'static str,
    section: Value,
}
pub(super) fn stock(scene: &Obj<'_>, t: &LiveTable, r: &Row) -> PyResult<Stock> {
    let mut stock = Stock {
        state: "ok",
        ..Default::default()
    };
    if group(scene.py(), t, &r.owner).is_none() {
        stock.state = "ref_lost";
        return Ok(stock);
    }
    if r.kind == "repeat" {
        match repeat::layout(scene, t, r)? {
            Ok(layout) => {
                stock.path = std::iter::once(layout.base.w_axis.truncate().to_array())
                    .chain(
                        layout
                            .poses
                            .iter()
                            .map(|(_, _, m)| m.w_axis.truncate().to_array()),
                    )
                    .collect();
                stock.layout = Some(layout);
            }
            Err(s) => stock.state = s,
        }
    } else if let Some(s) = &r.sweep {
        stock.path = reference::sweep_path(scene, t, r, &s.path)?.unwrap_or_default();
        let mut profile = s.profile.clone();
        if let Some(id) = &profile.sketch {
            profile.drawing = t.rows.get(id).filter(|r| r.state == "ok").map(|r| json!(r));
        }
        stock.section = json!([profile, s.attachment, s.rotation, s.mirror, s.segments]);
        if let Ok(o) = profile.outline(s.segments) {
            let area: f64 = o
                .rings
                .iter()
                .map(|ring| {
                    ring.iter()
                        .zip(ring.iter().cycle().skip(1))
                        .map(|(a, b)| a.p.perp_dot(b.p))
                        .sum::<f64>()
                        * 0.5
                })
                .sum();
            stock.dims.insert("area".into(), area.abs());
            stock.dims.extend(
                [
                    ("width", o.width),
                    ("height", o.height),
                    ("thickness", o.width),
                    ("web", o.web),
                    ("flange", o.flange),
                    ("radius", o.radius),
                ]
                .map(|(k, v)| (k.into(), v)),
            );
        } else {
            stock.state = "invalid_profile"
        }
        if stock.path.len() < 2 {
            stock.state = "end_unresolved"
        }
    }
    stock.dims.insert(
        "length".into(),
        stock
            .path
            .windows(2)
            .map(|p| DVec3::from(p[0]).distance(p[1].into()) * 1000.)
            .sum(),
    );
    Ok(stock)
}
/// Return geometry before the member's cut list, in its definition coordinates.
pub(super) fn uncut_mesh<'p>(
    scene: &Obj<'p>,
    t: &LiveTable,
    uid: &str,
) -> PyResult<Option<Obj<'p>>> {
    let Some(member) = group(scene.py(), t, uid) else {
        return Ok(None);
    };
    let Some(row) = t.owners.get(uid).and_then(|id| t.rows.get(id)) else {
        return member.getattr("mesh").map(Some);
    };
    if row.kind == "cut_stack" {
        return cut::source(scene, row.source.as_deref().unwrap_or(""))?
            .map(|g| g.getattr("mesh"))
            .transpose();
    }
    let Some(spec) = row.sweep.as_deref() else {
        return member.getattr("mesh").map(Some);
    };
    let mut spec = spec.clone();
    if let Some(id) = &spec.profile.sketch {
        spec.profile.drawing = t.rows.get(id).map(|r| json!(r));
    }
    let Some(path) = reference::sweep_path(scene, t, row, &spec.path)? else {
        return Ok(None);
    };
    let mesh = scene.py().import("built_core.mesh")?.call_method0("Mesh")?;
    let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
    if sweep::generate(&mut store.try_borrow_mut()?.inner, &spec, &path, &[]).is_err() {
        return Ok(None);
    }
    Ok(Some(mesh))
}
pub(super) fn id(uid: &str, key: &str) -> String {
    eid::derived_uid(&format!("built:junction:v1:{uid}:{key}"))
}
fn pose(p: DVec3, z: DVec3, x: DVec3) -> Option<DMat4> {
    let z = z.try_normalize()?;
    let x = (x - z * x.dot(z))
        .try_normalize()
        .unwrap_or_else(|| z.any_orthonormal_vector());
    Some(DMat4::from_cols(
        x.extend(0.),
        z.cross(x).extend(0.),
        z.extend(0.),
        p.extend(1.),
    ))
}
pub(super) fn local_frame(t: &LiveTable, owner: &str, junction: &str, name: &str) -> Option<DMat4> {
    let j = t.rows.get(junction)?;
    Some(
        reference::frame(*t.groups.get(owner)?).inverse()
            * reference::frame(*t.groups.get(&j.owner)?)
            * DMat4::from_cols_array(j.frames.get(name)?),
    )
}
struct Role {
    pos: DVec3,
    axis: DVec3,
    across: DVec3,
    up: DVec3,
    dims: expr::Values,
    datums: BTreeMap<String, DVec3>,
    face: Option<(DVec3, DVec3)>,
}
fn placement(b: &Value) -> Option<&str> {
    b["placement"]
        .as_str()
        .or_else(|| b["placement"].as_array()?.last()?.as_str())
}
fn box_role(scene: &Obj<'_>, t: &LiveTable, j: &Row, b: &Value) -> PyResult<Option<Role>> {
    let Some(uid) = placement(b) else {
        return Ok(None);
    };
    let Some(mesh) = uncut_mesh(scene, t, uid)? else {
        return Ok(None);
    };
    let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
    let borrow = store.try_borrow()?;
    let mesh = &borrow.inner;
    let mut lo = DVec3::splat(f64::INFINITY);
    let mut hi = -lo;
    for v in mesh.live_slots(Kind::Vertex) {
        let p = DVec3::from(
            mesh.state
                .frame
                .definition_point(*mesh.state.vpos.get(v as usize)),
        );
        lo = lo.min(p);
        hi = hi.max(p);
    }
    if !lo.is_finite() {
        return Ok(None);
    }
    let size = hi - lo;
    let mut axes = [0, 1, 2];
    axes.sort_by(|&a, &b| size[b].total_cmp(&size[a]));
    let [long, wide, thin] = axes;
    let m = reference::frame(t.groups[&j.owner]).inverse() * reference::frame(t.groups[uid]);
    let axis = |i: usize| m.transform_vector3([DVec3::X, DVec3::Y, DVec3::Z][i]);
    let dir = axis(long);
    let across = axis(wide);
    let up = axis(thin);
    let sign = if b["end"] == "start" { -1. } else { 1. };
    let pos = m.transform_point3((hi + lo) * 0.5) + sign * dir * size[long] * 0.5;
    let dims = expr::Values::from([
        ("length".into(), size[long] * dir.length() * 1000.),
        ("width".into(), size[wide] * across.length() * 1000.),
        ("height".into(), size[thin] * up.length() * 1000.),
        ("thickness".into(), size[thin] * up.length() * 1000.),
    ]);
    let half = across * size[wide] * 0.5;
    let rise = up * size[thin] * 0.5;
    let datums = BTreeMap::from([
        ("left".into(), -half),
        ("right".into(), half),
        ("top".into(), rise),
        ("bottom".into(), -rise),
        ("centre".into(), DVec3::ZERO),
    ]);
    Ok(Some(Role {
        pos,
        axis: sign * dir.normalize_or_zero(),
        across,
        up,
        dims,
        datums,
        face: None,
    }))
}
fn role(scene: &Obj<'_>, t: &LiveTable, j: &Row, b: &Value) -> PyResult<Option<Role>> {
    let Some(r) = b["row"].as_str().and_then(|id| t.rows.get(id)) else {
        return box_role(scene, t, j, b);
    };
    if r.kind == "cut_stack" {
        return box_role(scene, t, j, b);
    }
    if matches!(r.state.as_str(), "stale" | "cycle") {
        return Ok(None);
    }
    let Some(k) = t.groups.get(&r.owner) else {
        return Ok(None);
    };
    let stock = match t.stock.get(&r.uid) {
        Some(s) => s.clone(),
        None => Arc::new(stock(scene, t, r)?),
    };
    if let Some(station) = b.get("station") {
        let Some(layout) = &stock.layout else {
            return Ok(None);
        };
        let Some(source) = r.source.as_deref() else {
            return Ok(None);
        };
        let mut binding = b.clone();
        binding.as_object_mut().unwrap().remove("station");
        binding["placement"] = json!(source);
        binding["row"] = json!(t.owners.get(source));
        let Some(mut role) = role(scene, t, j, &binding)? else {
            return Ok(None);
        };
        let index = station.as_u64().or_else(|| station.as_str()?.parse().ok());
        let pose = layout.poses.iter().find(|(i, _, _)| {
            index == Some(*i as u64)
                || station.as_str()
                    == Some(&eid::derived_uid(&format!("built:repeat:v1:{}:{i}", r.uid)))
        });
        let Some((_, _, pose)) = pose else {
            return Ok(None);
        };
        let owner = reference::frame(t.groups[&j.owner]);
        let frame = owner.inverse()
            * reference::frame(*k)
            * *pose
            * reference::frame(t.groups[source]).inverse()
            * owner;
        role.pos = frame.transform_point3(role.pos);
        role.axis = frame.transform_vector3(role.axis).normalize_or_zero();
        role.across = frame.transform_vector3(role.across).normalize_or_zero();
        role.up = frame.transform_vector3(role.up).normalize_or_zero();
        for offset in role.datums.values_mut() {
            *offset = frame.transform_vector3(*offset);
        }
        return Ok(Some(role));
    }
    if stock.state != "ok" || stock.path.len() < 2 {
        return Ok(None);
    }
    let path = &stock.path;
    let end = b["end"] != "start";
    let n = if end { path.len() - 2 } else { 0 };
    let mat = reference::frame(t.groups[&j.owner]).inverse() * reference::frame(*k);
    let pos = mat.transform_point3(path[n + usize::from(end)].into());
    let Some(dir) = (DVec3::from(path[n + 1]) - DVec3::from(path[n])).try_normalize() else {
        return Ok(None);
    };
    let across = DVec3::Z.cross(dir).try_normalize().unwrap_or(DVec3::X);
    let up = dir.cross(across);
    let mut datums = BTreeMap::new();
    let mut section_axes = [mat.transform_vector3(across), mat.transform_vector3(up)];
    if let Some(s) = &r.sweep {
        let mut profile = s.profile.clone();
        if let Some(id) = &profile.sketch {
            profile.drawing = t.rows.get(id).map(|r| json!(r));
        }
        if let Ok(o) = profile.outline(s.segments) {
            let Ok(origin) = o.attachment(&s.attachment) else {
                return Ok(None);
            };
            let convert = |p: glam::DVec2| {
                let mut v = (p - origin) / 1000.;
                if s.mirror {
                    v.x = -v.x
                }
                v = glam::DVec2::from_angle(s.rotation.to_radians()).rotate(v);
                mat.transform_vector3(across * v.x + up * v.y)
            };
            section_axes = [
                convert(origin + glam::DVec2::X) - convert(origin),
                convert(origin + glam::DVec2::Y) - convert(origin),
            ];
            if o.web > 0. {
                for (name, sign) in [("web_left", -1.), ("web_right", 1.)] {
                    datums.insert(
                        name.into(),
                        convert(glam::DVec2::new(
                            (o.width + sign * o.web) * 0.5,
                            o.height * 0.5,
                        )),
                    );
                }
            }
            for name in [
                "centre",
                "bottom",
                "top",
                "left",
                "right",
                "bottom_left",
                "bottom_right",
                "top_left",
                "top_right",
            ]
            .into_iter()
            .chain(o.named.keys().map(String::as_str))
            {
                if let Ok(p) = o.attachment(name) {
                    datums.insert(name.into(), convert(p));
                }
            }
        }
    }
    let axis = mat
        .transform_vector3(if end { dir } else { -dir })
        .normalize_or_zero();
    Ok(Some(Role {
        pos,
        axis,
        across: section_axes[0].normalize_or_zero(),
        up: section_axes[1].normalize_or_zero(),
        dims: stock.dims.clone(),
        datums,
        face: None,
    }))
}
fn face_plane(
    scene: &Obj<'_>,
    t: &LiveTable,
    j: &Row,
    binding: &Value,
    pid: u64,
) -> PyResult<Option<(DVec3, DVec3)>> {
    let Some(uid) = placement(binding) else {
        return Ok(None);
    };
    let Some(mesh) = uncut_mesh(scene, t, uid)? else {
        return Ok(None);
    };
    let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
    let borrow = store.try_borrow()?;
    let s = &borrow.inner;
    let Some(slot) = s.slot_of_pid(Kind::Face, pid) else {
        return Ok(None);
    };
    let m = reference::frame(t.groups[&j.owner]).inverse() * reference::frame(t.groups[uid]);
    let ring = s.face_rings(slot);
    let points: Vec<_> = ring[0]
        .iter()
        .map(|v| {
            m.transform_point3(
                s.state
                    .frame
                    .definition_point(*s.state.vpos.get(*v as usize))
                    .into(),
            )
        })
        .collect();
    let Some(origin) = points.first().copied() else {
        return Ok(None);
    };
    let normal: DVec3 = points
        .windows(2)
        .map(|ab| (ab[0] - origin).cross(ab[1] - origin))
        .sum();
    Ok(normal.try_normalize().map(|n| (origin, n)))
}

fn geometry(scene: &Obj<'_>, t: &LiveTable, j: &Row) -> PyResult<Option<(Frames, expr::Values)>> {
    let mut roles = BTreeMap::new();
    let Some(bindings) = j.joint.roles.as_object() else {
        return Ok(None);
    };
    for (name, binding) in bindings {
        let Some(mut r) = role(scene, t, j, binding)? else {
            return Ok(None);
        };
        if let Some((letter, pid)) = binding["face"]
            .as_str()
            .and_then(|id| eid::from_text(id).ok())
        {
            if eid::kind_name(letter).and_then(Kind::parse) != Some(Kind::Face) {
                return Ok(None);
            }
            r.face = face_plane(scene, t, j, binding, pid)?;
            if r.face.is_none() {
                return Ok(None);
            }
        }
        roles.insert(name.as_str(), r);
    }
    let main = if roles.contains_key("main") {
        "main"
    } else if roles.contains_key("host") {
        "host"
    } else {
        roles.keys().next().copied().unwrap_or("")
    };
    let Some(a) = roles.get(main) else {
        return Ok(None);
    };
    let Some(b) = roles
        .get("joining")
        .or_else(|| roles.iter().find(|(k, _)| **k != main).map(|(_, r)| r))
    else {
        return Ok(None);
    };
    let facing = |a: &Role, b: &Role, face: Option<&str>| {
        if let Some(plane) = a.face {
            return Some(plane);
        }
        let aim = b.pos - a.pos - b.axis * 0.001;
        let choices: &[&str] = match face {
            Some("flange_outer") => &["top", "bottom"],
            Some("web_left") => &["web_left"],
            Some("web_right") => &["web_right"],
            Some("left") => &["left"],
            Some("right") => &["right"],
            Some("top") => &["top"],
            Some("bottom") => &["bottom"],
            Some(_) => return None,
            None => &["left", "right", "top", "bottom"],
        };
        choices
            .iter()
            .filter_map(|&name| {
                let offset = *a.datums.get(name)?;
                let normal = match name {
                    "left" | "web_left" => -a.across,
                    "right" | "web_right" => a.across,
                    "top" => a.up,
                    _ => -a.up,
                }
                .normalize_or_zero();
                Some((a.pos + offset, normal))
            })
            .max_by(|(_, n), (_, m)| n.dot(aim).total_cmp(&m.dot(aim)))
            .or_else(|| face.is_none().then_some((a.pos, a.axis)))
    };
    let Some((contact, normal)) = facing(a, b, bindings[main]["face"].as_str()) else {
        return Ok(None);
    };
    let delta = a.pos - b.pos;
    let (meet, z) = match j.joint.meeting.as_deref() {
        Some("corner") => ((a.pos + b.pos) * 0.5, (a.axis - b.axis).normalize_or_zero()),
        Some("end_end" | "parts") => ((a.pos + b.pos) * 0.5, a.axis),
        Some("crossing") => {
            let dot = a.axis.dot(b.axis);
            let den = 1. - dot * dot;
            if den.abs() < 1e-10 {
                return Ok(None);
            }
            let s = (dot * delta.dot(b.axis) - delta.dot(a.axis)) / den;
            let q = a.pos + s * a.axis;
            (
                (q + b.pos + b.axis.dot(q - b.pos) * b.axis) * 0.5,
                a.axis.cross(b.axis),
            )
        }
        Some("hosted") => (b.pos + normal * (contact - b.pos).dot(normal), normal),
        _ => {
            let den = b.axis.dot(normal);
            if den.abs() < 1e-10 {
                return Ok(None);
            }
            (
                b.pos + b.axis * ((contact - b.pos).dot(normal) / den),
                normal,
            )
        }
    };
    let Some(m) = pose(
        meet,
        z,
        if b.axis.cross(z).length_squared() > 1e-12 {
            b.axis
        } else {
            a.axis
        },
    ) else {
        return Ok(None);
    };
    let mut frames = Frames::from([("meet".into(), m.to_cols_array())]);
    let mut dims = expr::Values::new();
    for (name, r) in &roles {
        let at = r.pos + r.axis * (meet - r.pos).dot(r.axis);
        let (pt, n) = if *name == main {
            (contact, normal)
        } else {
            let Some(contact) = facing(r, a, bindings[*name]["face"].as_str()) else {
                return Ok(None);
            };
            contact
        };
        let end_frame = DMat4::from_cols(
            r.axis.extend(0.),
            r.up.cross(r.axis).normalize_or_zero().extend(0.),
            r.up.normalize_or_zero().extend(0.),
            r.pos.extend(1.),
        );
        for (suffix, frame) in [
            ("end", end_frame),
            ("at", DMat4::from_translation(at - r.pos) * end_frame),
            ("origin", DMat4::from_translation(at - r.pos) * end_frame),
            (
                "contact",
                pose(at + n * (pt - at).dot(n), n, r.axis).unwrap_or(end_frame),
            ),
        ] {
            frames.insert(format!("{name}.{suffix}"), frame.to_cols_array());
        }
        for (k, v) in &r.datums {
            frames.insert(
                format!("{name}.{k}"),
                (DMat4::from_translation(at + *v - r.pos) * end_frame).to_cols_array(),
            );
        }
        for (k, v) in &r.dims {
            dims.insert(format!("{name}.{k}"), *v);
        }
        dims.insert(
            format!("{name}.angle"),
            a.axis.dot(r.axis).abs().clamp(0., 1.).acos().to_degrees(),
        );
        let project = |axis: DVec3| glam::DVec2::new(axis.x, axis.y).normalize_or_zero();
        dims.insert(
            format!("{name}.skew"),
            project(a.axis)
                .dot(project(r.axis))
                .abs()
                .clamp(0., 1.)
                .acos()
                .to_degrees(),
        );
        dims.insert(
            format!("{name}.slope"),
            r.axis.z.clamp(-1., 1.).asin().to_degrees(),
        );
    }
    if placement(&bindings[main])
        .and_then(|uid| t.groups.get(uid))
        .is_some_and(|k| reference::frame(*k).determinant() < 0.)
    {
        for frame in frames.values_mut() {
            for v in &mut frame[4..7] {
                *v = -*v;
            }
        }
    }
    Ok(Some((frames, dims)))
}
pub(super) fn detail(t: &LiveTable, j: &Row) -> PyResult<presets::Preset> {
    if j.joint.detail.is_null() {
        let params: Vec<_> = j
            .joint
            .params
            .as_object()
            .into_iter()
            .flatten()
            .map(|(key, value)| json!({"key":key,"expr":value}))
            .collect();
        return Ok(presets::Preset {
            uid: String::new(),
            name: "Junction".into(),
            pack: String::new(),
            primitive: "junction".into(),
            values: BTreeMap::from([("params".into(), json!(params))]),
            record: BTreeMap::new(),
            naming: String::new(),
            node: String::new(),
            locks: BTreeMap::new(),
            rev: 1,
        });
    }
    j.joint.detail["uid"]
        .as_str()
        .or_else(|| j.joint.detail.as_str())
        .and_then(|id| presets::lookup(t, id))
        .filter(|p| p.primitive == "junction")
        .ok_or_else(|| err("Unknown junction detail"))
}
fn set_reason(row: &mut Row, state: &str, reason: Value) {
    row.state = state.into();
    row.joint.reason = reason;
}
fn outside_role(
    scene: &Obj<'_>,
    t: &LiveTable,
    binding: &Value,
    name: &str,
    rule: &Value,
) -> PyResult<Option<Value>> {
    let problem = |what: &str, value: Value| {
        Some(json!({"role":name,"criterion":what,"value":value,"limit":rule[what]}))
    };
    if let Some(classes) = rule["class"].as_array() {
        let class = placement(binding)
            .and_then(|id| group(scene.py(), t, id))
            .and_then(|g| {
                g.getattr("ifc")
                    .ok()?
                    .get_item("class")
                    .ok()?
                    .extract::<String>()
                    .ok()
            });
        let value = json!(class);
        if !classes.contains(&value) {
            return Ok(problem("class", value));
        }
    }
    if let Some(profiles) = rule["profile"].as_array() {
        let sweep = binding["row"]
            .as_str()
            .and_then(|id| t.rows.get(id))
            .and_then(|r| r.sweep.as_deref());
        let fits = sweep.is_some_and(|s| {
            profiles.iter().any(|p| {
                let library = p["library"]
                    .as_str()
                    .is_none_or(|prefix| s.profile.library.starts_with(prefix));
                let family = p["family"].as_str().is_none_or(|wanted| {
                    s.profile
                        .outline(s.segments)
                        .is_ok_and(|o| o.family == wanted)
                });
                library && family
            })
        });
        if !fits {
            return Ok(problem("profile", json!(sweep.map(|s| &s.profile.library))));
        }
    }
    if rule["end"]
        .as_str()
        .is_some_and(|end| end != "any" && binding["end"] != end)
    {
        return Ok(problem("end", binding["end"].clone()));
    }
    if !binding["face"].is_null()
        && rule["face"]
            .as_array()
            .is_some_and(|allowed| !allowed.contains(&binding["face"]))
    {
        return Ok(problem("face", binding["face"].clone()));
    }
    Ok(None)
}

pub(super) fn evaluate(scene: &Obj<'_>, t: &mut LiveTable, uid: &str) -> PyResult<bool> {
    let mut row = (*t.rows[uid]).clone();
    let before = (
        row.frames.clone(),
        row.joint.params.clone(),
        row.state.clone(),
    );
    let mut fields_changed = false;
    let Ok(p) = detail(t, &row) else {
        set_reason(&mut row, "failed", json!({"criterion":"detail_missing"}));
        t.rows.insert(uid.into(), Arc::new(row));
        return Ok(before.2 != "failed");
    };
    if let Some((frames, dims)) = geometry(scene, t, &row)? {
        row.frames = frames;
        let rule = p.values.get("rule").unwrap_or(&Value::Null);
        let mut outside = None;
        for (role, crit) in rule["roles"].as_object().into_iter().flatten() {
            if let Some(reason) = outside_role(scene, t, &row.joint.roles[role], role, crit)? {
                outside = Some(reason);
                break;
            }
            for (key, range) in crit["size"].as_object().into_iter().flatten() {
                if let Some(v) = dims.get(&format!("{role}.{key}")) {
                    if range[0].as_f64().is_some_and(|l| *v < l)
                        || range[1].as_f64().is_some_and(|h| *v > h)
                    {
                        outside =
                            Some(json!({"role":role,"criterion":"size","value":v,"limit":range}));
                    }
                }
            }
        }
        if let (Some(a), Some(v)) = (rule["angle"]["value"].as_f64(), dims.get("joining.angle")) {
            if (a - v).abs() > rule["angle"]["tol"].as_f64().unwrap_or(2.) {
                outside = Some(json!({"criterion":"angle","value":v,"limit":rule["angle"]}));
            }
        }
        if let Some(reason) = outside {
            set_reason(&mut row, "outside", reason);
        } else {
            let defs = p
                .values
                .get("params")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            match expr::resolve(defs, &row.joint.overrides, &dims) {
                Err(e) => {
                    if e.contains("cycle") {
                        return Err(err(e));
                    }
                    set_reason(
                        &mut row,
                        "failed",
                        json!({"criterion":"expression","message":e}),
                    );
                }
                Ok((params, limited)) => {
                    let mut changes = BTreeMap::new();
                    for bind in row.joint.bindings.as_array().into_iter().flatten() {
                        let id = bind["row"]
                            .as_str()
                            .ok_or_else(|| err("Missing binding row"))?;
                        let child = t
                            .rows
                            .get(id)
                            .filter(|r| r.junction.as_deref() == Some(uid))
                            .ok_or_else(|| err("Binding outside junction"))?;
                        let path = bind["field"]
                            .as_str()
                            .ok_or_else(|| err("Missing binding field"))?;
                        let value = expr::field(&bind["param"], &params, &dims).map_err(err)?;
                        let obj = changes
                            .entry(id.to_string())
                            .or_insert_with(|| json!(child));
                        *obj.pointer_mut(path)
                            .ok_or_else(|| err("Missing bound field"))? = json!(value);
                    }
                    for (id, v) in changes {
                        if v != json!(&t.rows[&id]) {
                            fields_changed = true;
                            let r = serde_json::from_value(v).map_err(err)?;
                            t.rows.insert(id, Arc::new(r));
                        }
                    }
                    row.joint.params = json!(params);
                    if limited {
                        set_reason(&mut row, "failed", json!({"criterion":"limit"}));
                    } else {
                        set_reason(&mut row, "ok", Value::Null);
                    }
                }
            }
        }
    } else {
        let mut reason = json!({"criterion":"role"});
        for (name, binding) in row.joint.roles.as_object().into_iter().flatten() {
            if role(scene, t, &row, binding)?.is_none() {
                reason["role"] = json!(name);
                break;
            }
        }
        set_reason(&mut row, "lost", reason);
    }
    let changed = fields_changed
        || before.1 != row.joint.params
        || before.2 != row.state
        || before.0.len() != row.frames.len()
        || row.frames.iter().any(|(k, v)| {
            before.0.get(k).is_none_or(|o| {
                o.iter()
                    .zip(v)
                    .any(|(a, b)| (a * 1e6).round() != (b * 1e6).round())
            })
        });
    t.rows.insert(uid.into(), Arc::new(row));
    Ok(changed)
}
fn expand_field(
    v: &mut Value,
    path: &str,
    row: &Row,
    id: &str,
    bindings: &mut Vec<Value>,
) -> PyResult<()> {
    if let Some(obj) = v.as_object_mut() {
        for (k, val) in obj {
            expand_field(val, &format!("{path}/{k}"), row, id, bindings)?;
        }
    } else if let Some(s) = v.as_str() {
        if let Some(frame) = s.strip_prefix('@') {
            *v = if path == "/end" {
                let role = frame
                    .strip_suffix(".end")
                    .ok_or_else(|| err("Expected role end"))?;
                row.joint.roles[role]
                    .get("end")
                    .cloned()
                    .unwrap_or(json!("end"))
            } else if path == "/anchor/ref" {
                json!({"junction":row.uid,"frame":frame})
            } else {
                return Err(err("Unknown symbolic cut field"));
            };
        } else if matches!(
            path,
            "/offset" | "/anchor/offset" | "/clearance" | "/cutter/plane/angle"
        ) {
            bindings.push(json!({"row":id,"field":path,"param":s}));
            *v = json!(0.);
        }
    }
    Ok(())
}
pub(super) fn remove_cut(t: &mut LiveTable, uid: &str) -> PyResult<()> {
    if let Some(cut) = t.remove(uid) {
        if let Some(id) = t.owners.get(&cut.owner).cloned() {
            let mut target = (*t.rows[&id]).clone();
            target.cuts.retain(|s| s != uid);
            t.insert(target).map_err(err)?;
            t.pending.insert(id);
        }
    }
    Ok(())
}
pub(super) fn prepare(
    scene: &Obj<'_>,
    t: &mut LiveTable,
    row: &mut Row,
    p: &presets::Preset,
) -> PyResult<()> {
    if let Some(defs) = p.values.get("params").and_then(Value::as_array) {
        let mut defs = defs.clone();
        for d in &mut defs {
            if let Some(v) = d["key"].as_str().and_then(|k| row.joint.overrides.get(k)) {
                d["expr"] = v.clone();
            }
        }
        expr::validate(&defs).map_err(err)?;
    }
    let roles = row
        .joint
        .roles
        .as_object_mut()
        .filter(|r| r.len() >= 2)
        .ok_or_else(|| err("Bind at least two junction roles"))?;
    if let Some(required) = p.values.get("roles").and_then(Value::as_object) {
        if let Some(key) = required.keys().find(|key| !roles.contains_key(*key)) {
            return Err(err(format!("Missing junction role {key}")));
        }
    }
    row.inputs.clear();
    for binding in roles.values_mut() {
        if binding["end"]
            .as_str()
            .is_some_and(|s| !matches!(s, "start" | "end"))
        {
            return Err(err("Unknown member end"));
        }
        if let Some(path) = binding["placement"].as_array() {
            let occurrence =
                scene.call_method1("resolve_occurrence", (to_py(scene.py(), path)?,))?;
            if occurrence.is_none() {
                return Err(err("Unknown member occurrence"));
            }
        }
        let member = binding["placement"]
            .as_str()
            .or_else(|| binding["placement"].as_array()?.last()?.as_str());
        let r = binding["row"]
            .as_str()
            .or_else(|| t.owners.get(member?).map(String::as_str))
            .and_then(|id| t.rows.get(id));
        let Some(r) = r else {
            let uid = member
                .filter(|id| t.groups.contains_key(*id))
                .ok_or_else(|| err("Unknown role placement"))?;
            row.inputs.push(Reference::Placement {
                placement: uid.into(),
            });
            continue;
        };
        if !matches!(r.kind.as_str(), "sweep" | "repeat" | "cut_stack") {
            return Err(err("Unknown member stock"));
        }
        if member.is_some_and(|s| s != r.owner) {
            return Err(err("Role owner mismatch"));
        }
        binding["row"] = json!(r.uid);
        if binding["placement"].is_null() {
            binding["placement"] = json!(r.owner)
        }
        row.inputs.push(Reference::Axis {
            sweep: r.uid.clone(),
            at: table::Station::End("end".into()),
            point: None,
        });
    }
    let first = roles
        .get("main")
        .or_else(|| roles.get("host"))
        .unwrap_or_else(|| roles.values().next().unwrap());
    let main = placement(first)
        .ok_or_else(|| err("Missing main placement"))?
        .to_owned();
    let owner = id(&row.uid, "owner");
    if let Some(old) = t.rows.get(&row.uid) {
        if old.kind != "junction" || old.owner != owner {
            return Err(err("Junction identity collision"));
        }
        row.frames = old.frames.clone();
        row.joint.params = old.joint.params.clone();
    } else {
        if t.groups.contains_key(&owner) {
            return Err(err("Junction assembly already exists"));
        }
        let py = scene.py();
        let g = py.import("built_core.group")?.getattr("Group")?.call0()?;
        g.setattr("uid", &owner)?;
        g.setattr("name", &p.name)?;
        g.setattr("xform", crate::math::Matrix4::from(DMat4::IDENTITY))?;
        g.setattr(
            "ifc",
            to_py(py, &json!({"class":"IfcElementAssembly","name":p.name}))?,
        )?;
        p.record(&g, false)?;
        g.setattr("ext", to_py(py, &json!({"built.live":{"row":row.uid}}))?)?;
        let parent = if let Some(parent) = row.joint.parent.as_deref() {
            t.rows
                .get(parent)
                .and_then(|p| t.groups.get(&p.owner))
                .copied()
        } else {
            doc::lock()
                .node(t.groups[&main])
                .and_then(|n| n.owners.first().copied())
        };
        let siblings = match parent
            .and_then(|k| doc::lock().handle_ref(py, k))
            .and_then(|g| g.bind(py).getattr("children").ok())
        {
            Some(children) => children,
            None => scene.getattr("groups")?,
        };
        siblings.call_method1("append", (&g,))?;
        crate::structure::file_sweep(scene, &g, "")?;
        Arc::make_mut(&mut t.groups).insert(owner.clone(), g.getattr("_doc_key")?.extract()?);
    }
    row.owner = owner;
    if row.joint.detail.is_null() {
        return Ok(());
    }
    row.joint.detail = json!({"uid":p.uid,"rev":p.rev,"name":p.name});
    row.joint.meeting = p
        .values
        .get("meeting")
        .and_then(Value::as_str)
        .map(String::from);
    let mut contents = Vec::new();
    let mut bindings = Vec::new();
    let mut keep = HashSet::new();
    for item in p
        .values
        .get("contents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let key = item["key"]
            .as_str()
            .filter(|k| !k.is_empty() && *k != "owner")
            .ok_or_else(|| err("Invalid content key"))?;
        if row.joint.local["removed"]
            .as_array()
            .is_some_and(|a| a.contains(&json!(key)))
        {
            continue;
        }
        let uid = id(&row.uid, key);
        if !keep.insert(uid.clone()) {
            return Err(err("Duplicate junction content key"));
        }
        let on = item["on"].as_str().ok_or_else(|| err("Missing cut role"))?;
        let target = placement(&row.joint.roles[on])
            .ok_or_else(|| err("Unknown cut role"))?
            .to_owned();
        let mut v = item["row"].clone();
        if v["kind"] != "cut" {
            return Err(err("Expected cut content"));
        }
        expand_field(&mut v, "", row, &uid, &mut bindings)?;
        v["uid"] = json!(uid);
        v["owner"] = json!(target);
        v["key"] = json!(key);
        v["junction"] = json!(row.uid);
        if let Some(old) = t.rows.get(&uid) {
            if old.junction.as_deref() != Some(&row.uid) {
                return Err(err("Junction content identity collision"));
            }
            if old.owner != target {
                remove_cut(t, &uid)?;
            }
        }
        if let Some(old) = t.rows.get(&uid) {
            let previous = json!(old);
            for path in bindings
                .iter()
                .filter(|b| b["row"] == uid)
                .filter_map(|b| b["field"].as_str())
            {
                if let (Some(old), Some(next)) = (previous.pointer(path), v.pointer_mut(path)) {
                    *next = old.clone();
                }
            }
        }
        let mut cut: Row = serde_json::from_value(v).map_err(err)?;
        cut::attach(scene, t, &mut cut)?;
        t.insert(cut).map_err(err)?;
        t.pending.insert(uid.clone());
        contents.push(json!({"key":key,"row":uid}));
    }
    for uid in t.owned.get(&row.uid).cloned().unwrap_or_default() {
        if !keep.contains(&uid)
            && !row.joint.local["added"]
                .as_array()
                .is_some_and(|a| a.contains(&json!(uid)))
        {
            remove_cut(t, &uid)?;
        }
    }
    // A plain solid acquires a stock source when its first cut is attached.
    // Bind its new stock node before checking the expanded graph.
    for b in row
        .joint
        .roles
        .as_object_mut()
        .into_iter()
        .flat_map(|m| m.values_mut())
    {
        if b["row"].is_null() {
            if let Some((uid, r)) = placement(b).and_then(|uid| Some((uid, t.owners.get(uid)?))) {
                row.inputs
                    .retain(|i| !matches!(i,Reference::Placement{placement} if placement==uid));
                row.inputs.push(Reference::Axis {
                    sweep: r.clone(),
                    at: table::Station::End("end".into()),
                    point: None,
                });
                b["row"] = json!(r);
            }
        }
    }
    for uid in row.joint.local["added"].as_array().into_iter().flatten() {
        if let Some(child) = uid.as_str().and_then(|id| t.rows.get(id)) {
            contents.push(local_content(child));
        }
    }
    row.joint.contents = json!(contents);
    row.joint.bindings = json!(bindings);
    Ok(())
}
pub(super) fn release(scene: &Obj<'_>, t: &mut LiveTable, j: &Row) -> PyResult<()> {
    for uid in t.owned.get(&j.uid).cloned().unwrap_or_default() {
        let mut r = (*t.rows[&uid]).clone();
        if let Some(anchor) = &r.anchor {
            if let Reference::Frame { junction, frame } = &anchor.reference {
                let m = local_frame(t, &r.owner, junction, frame)
                    .or_else(|| {
                        let frame = j.frames.get(frame)?;
                        Some(
                            reference::frame(*t.groups.get(&r.owner)?).inverse()
                                * reference::frame(*t.groups.get(&j.owner)?)
                                * DMat4::from_cols_array(frame),
                        )
                    })
                    .ok_or_else(|| err("Missing last good frame"))?;
                if let Some(cut::Cutter::Plane(p)) = &mut r.cutter {
                    p.origin = m.w_axis.truncate().to_array();
                    p.normal = Some(m.z_axis.truncate().normalize().to_array());
                }
                r.offset = Some(anchor.offset);
                r.anchor = None;
            }
        }
        r.junction = None;
        r.key = None;
        r.joint.parent = None;
        t.insert(r).map_err(err)?;
        t.pending.insert(uid);
    }
    if let Some(g) = group(scene.py(), t, &j.owner) {
        let record = g.getattr("ifc")?.call_method0("copy")?;
        let role_owners: BTreeMap<_, _> = j
            .joint
            .roles
            .as_object()
            .into_iter()
            .flat_map(|r| r.iter())
            .filter_map(|(name, b)| {
                let uid = b["placement"]
                    .as_str()
                    .or_else(|| b["placement"].as_array()?.last()?.as_str())?;
                Some((name.as_str(), uid))
            })
            .collect();
        let main = role_owners
            .get("main")
            .or_else(|| role_owners.get("host"))
            .or_else(|| role_owners.values().next());
        let related: Vec<_> = role_owners.values().filter(|u| Some(*u) != main).collect();
        let realizing: Vec<_> = j
            .joint
            .contents
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c["placement"].as_str())
            .collect();
        record.set_item("connects",to_py(scene.py(),&json!({"relating":main,"related":related,"realizing":realizing,"type":j.joint.detail["name"]}))?)?;
        g.setattr("ifc", record)?;
    }
    Ok(())
}

pub(crate) fn labels(scene: &Obj<'_>) -> PyResult<HashMap<String, Vec<(String, String, String)>>> {
    let mut labels = HashMap::<String, Vec<_>>::new();
    let Some(t) = root(scene).ok().and_then(|k| current(scene.py(), k)) else {
        return Ok(labels);
    };
    for id in &t.junctions {
        let j = &t.rows[id];
        let Some(g) = group(scene.py(), &t, &j.owner) else {
            continue;
        };
        let address = scene.call_method1("occurrence_path", (&g,))?;
        let path: String = scene
            .py()
            .import("built_core.link_structure")?
            .call_method1("row_id", (address,))?
            .extract()?;
        let label: String = g.getattr("name")?.extract()?;
        for member in j
            .joint
            .roles
            .as_object()
            .into_iter()
            .flat_map(|r| r.values())
            .filter_map(|b| t.rows.get(b["row"].as_str()?))
        {
            labels.entry(member.owner.clone()).or_default().push((
                path.clone(),
                label.clone(),
                j.state.clone(),
            ));
        }
    }
    Ok(labels)
}
#[pyfunction]
pub(super) fn junction_markers<'p>(scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    let mut marks = Vec::new();
    if let Some(t) = root(scene).ok().and_then(|k| current(scene.py(), k)) {
        let selected = scene.getattr("selection")?;
        for uid in &t.junctions {
            let j = &t.rows[uid];
            let visible = std::iter::once(&j.owner)
                .chain(
                    j.joint
                        .roles
                        .as_object()
                        .into_iter()
                        .flat_map(|m| m.values())
                        .filter_map(|b| Some(&t.rows.get(b["row"].as_str()?)?.owner)),
                )
                .filter_map(|id| group(scene.py(), &t, id))
                .any(|g| selected.contains(g).unwrap_or(false));
            if visible {
                if let Some(m) = local_frame(&t, &j.owner, uid, "meet") {
                    marks.push((
                        reference::frame(t.groups[&j.owner])
                            .transform_point3(m.w_axis.truncate())
                            .to_array(),
                        j.state == "ok",
                    ));
                }
            }
        }
    }
    to_py(scene.py(), &marks)
}

fn local_content(row: &Row) -> Value {
    match row.kind.as_str() {
        "junction" => json!({"key":row.key,"junction":row.uid,"placement":id(&row.uid,"owner")}),
        "repeat" => json!({"key":row.key,"repeat":row.uid,"placement":row.owner}),
        "cut" => json!({"key":row.key,"row":row.uid}),
        _ => json!({"key":row.key,"row":row.uid,"placement":row.owner}),
    }
}

pub(super) fn forget_content(t: &mut LiveTable, row: &Row) {
    let owner = row.junction.as_ref().or(row.joint.parent.as_ref());
    let Some(parent) = owner.and_then(|id| t.rows.get_mut(id)) else {
        return;
    };
    let parent = Arc::make_mut(parent);
    let local = &mut parent.joint.local;
    let added = local["added"].as_array_mut().and_then(|items| {
        let index = items.iter().position(|id| id == &row.uid)?;
        Some(items.remove(index))
    });
    if added.is_none() {
        let mut removed = local["removed"].as_array().cloned().unwrap_or_default();
        removed.push(json!(row.key));
        local["removed"] = json!(removed);
    }
    if let Some(items) = parent.joint.contents.as_array_mut() {
        items.retain(|item| item["key"] != json!(row.key));
    }
}

pub(super) fn remove_descendants(t: &mut LiveTable, uid: &str) -> PyResult<()> {
    let mut pending = vec![uid.to_owned()];
    while let Some(parent) = pending.pop() {
        for child in t.owned.get(&parent).cloned().unwrap_or_default() {
            pending.push(child.clone());
            remove_cut(t, &child)?;
        }
    }
    Ok(())
}

/// Keep an owned sweep under its assembly, using the same placement on re-expansion.
pub(super) fn place_owned_sweep(scene: &Obj<'_>, t: &mut LiveTable, row: &Row) -> PyResult<()> {
    let Some(parent) = row.junction.as_deref().filter(|_| row.kind == "sweep") else {
        return Ok(());
    };
    let key = row.key.as_deref().ok_or_else(|| err("Missing part key"))?;
    if row.owner != id(parent, &format!("{key}:owner")) {
        return Err(err("Invalid junction part placement"));
    }
    let owner = &t
        .rows
        .get(parent)
        .ok_or_else(|| err("Missing junction"))?
        .owner;
    let parent_key = *t
        .groups
        .get(owner)
        .ok_or_else(|| err("Missing junction assembly"))?;
    if let Some(existing) = t.groups.get(&row.owner) {
        if doc::lock()
            .node(*existing)
            .is_none_or(|n| !n.owners.contains(&parent_key))
        {
            return Err(err("Part must be under its junction assembly"));
        }
        return Ok(());
    }
    let py = scene.py();
    let g = py.import("built_core.group")?.getattr("Group")?.call0()?;
    g.setattr("uid", &row.owner)?;
    g.setattr("name", key)?;
    g.setattr("xform", crate::math::Matrix4::from(DMat4::IDENTITY))?;
    let parent = group(py, t, owner).ok_or_else(|| err("Missing junction assembly"))?;
    parent.getattr("children")?.call_method1("append", (&g,))?;
    Arc::make_mut(&mut t.groups).insert(row.owner.clone(), g.getattr("_doc_key")?.extract()?);
    Ok(())
}

pub(super) fn validate_ownership(t: &mut LiveTable, row: &Row) -> PyResult<()> {
    let owner = row.junction.as_ref().or(row.joint.parent.as_ref());
    if let Some(old) = t.rows.get(&row.uid) {
        if old.junction != row.junction
            || old.joint.parent != row.joint.parent
            || old.key != row.key
        {
            return Err(err("Ownership change requires junction edit"));
        }
        if let Some(parent) = owner.and_then(|id| t.rows.get(id)) {
            let before = json!(old);
            let next = json!(row);
            if parent
                .joint
                .bindings
                .as_array()
                .into_iter()
                .flatten()
                .filter(|b| b["row"] == row.uid)
                .filter_map(|b| b["field"].as_str())
                .any(|path| before.pointer(path) != next.pointer(path))
            {
                return Err(err("Override bound fields on the junction"));
            }
        }
        return Ok(());
    }
    let Some(owner) = owner else { return Ok(()) };
    if owner == &row.uid {
        return Err(err("Junction ownership cycle"));
    }
    let key = row
        .key
        .as_deref()
        .filter(|key| !key.is_empty())
        .ok_or_else(|| err("Missing content key"))?;
    if !t.junctions.contains(owner) || row.uid != id(owner, key) {
        return Err(err("Invalid junction content identity"));
    }
    if row.joint.parent.is_some()
        && (row.kind != "junction"
            || row
                .joint
                .parent
                .as_ref()
                .is_some_and(|parent| parent != owner))
    {
        return Err(err("Invalid junction parent"));
    }
    let parent = Arc::make_mut(t.rows.get_mut(owner).unwrap());
    let mut added = parent.joint.local["added"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    added.push(json!(row.uid));
    parent.joint.local["added"] = json!(added);
    let mut contents = parent
        .joint
        .contents
        .as_array()
        .cloned()
        .unwrap_or_default();
    contents.push(local_content(row));
    parent.joint.contents = json!(contents);
    Ok(())
}
