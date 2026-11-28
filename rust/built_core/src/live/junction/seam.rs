// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::store::edit::Change;
use glam::DVec2;
use i_overlay::{
    core::{fill_rule::FillRule, overlay_rule::OverlayRule},
    float::single::SingleFloatOverlay,
};

/// Clip the projected section to the finite stock faces on the contact plane.
fn contact_area(
    scene: &Obj<'_>,
    t: &LiveTable,
    j: &Row,
    contact: DMat4,
    projected: &[Vec<[f64; 2]>],
    exact_area: f64,
) -> PyResult<f64> {
    let uid = placement(&j.joint.roles["b"]).ok_or_else(|| err("Missing glue substrate"))?;
    let mesh = uncut_mesh(scene, t, uid)?.ok_or_else(|| err("Missing glue substrate"))?;
    let handle = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
    let borrow = handle.try_borrow()?;
    let st = &borrow.inner;
    let plane = contact.inverse()
        * reference::frame(t.groups[&j.owner]).inverse()
        * reference::frame(t.groups[uid]);
    let mut substrate = Vec::new();
    for face in st.live_slots(Kind::Face) {
        let mut contours = Vec::new();
        let mut coplanar = true;
        for ring in st.face_rings(face) {
            let mut points = Vec::with_capacity(ring.len());
            for slot in ring {
                let pos = st
                    .state
                    .frame
                    .definition_point(*st.state.vpos.get(slot as usize));
                let p = plane.transform_point3(pos.into()) * 1000.;
                if p.z.abs() > 1e-5 {
                    coplanar = false;
                    break;
                }
                points.push([p.x, p.y]);
            }
            if !coplanar {
                break;
            }
            contours.push(points);
        }
        if coplanar {
            substrate.extend(contours);
        }
    }
    if projected
        .overlay(&substrate, OverlayRule::Difference, FillRule::EvenOdd)
        .is_empty()
    {
        return Ok(exact_area);
    }
    let clipped = projected.overlay(&substrate, OverlayRule::Intersect, FillRule::EvenOdd);
    let area = clipped
        .iter()
        .flatten()
        .map(|ring| {
            ring.iter()
                .zip(ring.iter().cycle().skip(1))
                .map(|(a, b)| a[0] * b[1] - a[1] * b[0])
                .sum::<f64>()
        })
        .sum::<f64>()
        .abs()
        * 0.5;
    if area <= 0. {
        return Err(err("No glue contact area"));
    }
    Ok(area)
}

/// Intersect the section generators with the other ply's stock contact plane.
pub(super) fn update(
    scene: &Obj<'_>,
    t: &LiveTable,
    j: &Row,
    g: &Obj<'_>,
    spec: &Value,
    glue: bool,
    params: &expr::Values,
    dims: &expr::Values,
) -> PyResult<f64> {
    let binding = &j.joint.roles["a"];
    let a = role(scene, t, j, binding)?.ok_or_else(|| err("Missing weld member"))?;
    let row = binding["row"]
        .as_str()
        .and_then(|s| t.rows.get(s))
        .ok_or_else(|| err("Missing weld section"))?;
    let sweep = row
        .sweep
        .as_ref()
        .ok_or_else(|| err("Expected weld section"))?;
    let mut profile = sweep.profile.clone();
    if let Some(id) = &profile.sketch {
        profile.drawing = t.rows.get(id).map(|r| json!(r));
    }
    let outline = profile.outline(sweep.segments).map_err(err)?;
    let origin = sweep.origin(&outline).map_err(err)?;
    let contact = DMat4::from_cols_array(
        j.frames
            .get("b.contact")
            .ok_or_else(|| err("Missing weld contact plane"))?,
    );
    let n = contact.z_axis.truncate().normalize();
    let den = n.dot(a.axis);
    if den.abs() < 1e-9 {
        return Err(err("Weld section parallel to contact plane"));
    }
    let project = |p: DVec2| {
        let p = a.pos + (a.across * (p.x - origin.x) + a.up * (p.y - origin.y)) / 1000.;
        p + a.axis * (n.dot(contact.w_axis.truncate() - p) / den)
    };
    let mut length = 0.;
    let mut area = 0.;
    let mut paths = Vec::new();
    let throat = expr::field(
        spec.get("throat")
            .unwrap_or(&json!(params.get("throat").copied().unwrap_or(0.))),
        params,
        dims,
    )
    .map_err(err)?;
    let leg = throat * 2f64.sqrt() / 1000.;
    for (r, ring) in outline.rings.iter().enumerate() {
        let mut path = Vec::new();
        let mut signed_area = 0.;
        for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            let start = project(a.p);
            let end = project(b.p);
            let include = if let Some(edges) = spec["edges"].as_array() {
                edges.contains(&json!(a.role))
            } else {
                let top =
                    (a.p.y - outline.height).abs() < 1e-8 && (b.p.y - outline.height).abs() < 1e-8;
                let bottom = a.p.y.abs() < 1e-8 && b.p.y.abs() < 1e-8;
                match spec["sides"].as_str().unwrap_or("all_round") {
                    "one" => top,
                    "both" => top || bottom,
                    "all_round" => true,
                    _ => return Err(err("Unknown weld sides")),
                }
            };
            if let Some(c) = a.arc {
                let u = a.p - c;
                let theta = u.angle_to(b.p - c);
                signed_area += c.perp_dot(b.p - a.p) + u.length_squared() * theta;
                if include {
                    // Simpson quadrature of the projected circle's speed (an ellipse).
                    let speed = |i: usize| {
                        let v = DVec2::from_angle(theta * i as f64 / 16.).rotate(u).perp();
                        project(c + v).distance(project(c)) * 1000.
                    };
                    length += theta.abs() / 48.
                        * (speed(0)
                            + speed(16)
                            + (1..16)
                                .map(|i| {
                                    if i % 2 == 1 {
                                        4. * speed(i)
                                    } else {
                                        2. * speed(i)
                                    }
                                })
                                .sum::<f64>());
                }
            } else {
                signed_area += a.p.perp_dot(b.p);
                if include {
                    length += 1000. * start.distance(end);
                }
            }
            path.push((start, end, include, a.role.clone()));
        }
        area += if r == 0 { 1. } else { -1. } * signed_area.abs() * 0.5 / den.abs();
        paths.push(path);
    }
    if glue {
        let plane = contact.inverse();
        let projected: Vec<Vec<[f64; 2]>> = paths
            .iter()
            .map(|ring| {
                ring.iter()
                    .map(|(p, _, _, _)| (plane.transform_point3(*p).truncate() * 1000.).to_array())
                    .collect()
            })
            .collect();
        area = contact_area(scene, t, j, contact, &projected, area)?;
    }
    let py = scene.py();
    let mesh = py.import("built_core.mesh")?.call_method0("Mesh")?;
    if !glue {
        let handle = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
        let mut b = handle.try_borrow_mut()?;
        let st = &mut b.inner;
        let mut change = Change::default();
        let mut roles = Vec::new();
        for (r, path) in paths.iter().enumerate() {
            for (i, (a, b, include, name)) in path.iter().enumerate() {
                if !include {
                    continue;
                }
                let edge = st
                    .add_edge(a.to_array(), b.to_array(), &mut change)
                    .map_err(|_| err("Invalid weld path"))?;
                roles.push((format!("weld/{r}/{name}"), Kind::Edge, edge));
                if spec["bead"] == true {
                    if leg <= 0. {
                        return Err(err("Expected positive weld throat"));
                    }
                    let side = |i: usize| {
                        let prev = &path[(i + path.len() - 1) % path.len()];
                        let curr = &path[i % path.len()];
                        let p = (prev.1 - prev.0).normalize().cross(n);
                        let q = (curr.1 - curr.0).normalize().cross(n);
                        (p + q) * leg / (1. + p.dot(q))
                    };
                    let section = |p: DVec3, s: DVec3| [p, p + s, p + n * leg];
                    let (start, end) = (section(*a, side(i)), section(*b, side(i + 1)));
                    for k in 0..3 {
                        let next = (k + 1) % 3;
                        let f = st
                            .add_face(
                                &[vec![start[k], start[next], end[next], end[k]]
                                    .into_iter()
                                    .map(|p| p.to_array())
                                    .collect()],
                                &mut change,
                            )
                            .map_err(|_| err("Invalid weld bead"))?;
                        roles.push((format!("weld/{r}/{name}/side/{k}"), Kind::Face, f));
                    }
                    for (index, cap, reverse) in [(i, start, true), (i + 1, end, false)] {
                        let neighbor = if reverse {
                            (index + path.len() - 1) % path.len()
                        } else {
                            index % path.len()
                        };
                        if !path[neighbor].2 {
                            let mut ring = cap.map(|p| p.to_array()).to_vec();
                            if reverse {
                                ring.reverse();
                            }
                            st.add_face(&[ring], &mut change)
                                .map_err(|_| err("Invalid weld bead end"))?;
                        }
                    }
                }
            }
        }
        geometry::assign_roles(st, roles).map_err(err)?;
    }
    g.setattr("mesh", mesh)?;
    let mut record = json!({"class":"IfcFastener","predefined_type":if glue{"GLUE"}else{"WELD"}});
    if glue {
        record["area"] = json!(area);
    } else {
        record["length"] = json!(length);
        record["throat"] = json!(throat);
        for key in ["type", "sides", "process"] {
            if let Some(v) = spec.get(key) {
                record[if key == "type" { "weld_type" } else { key }] = v.clone();
            }
        }
    }
    g.setattr("ifc", to_py(py, &record)?)?;
    Ok(if glue { area } else { length })
}
