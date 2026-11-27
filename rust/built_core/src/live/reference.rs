// SPDX-License-Identifier: GPL-3.0-or-later
//! Resolve stable references into the local coordinates of the generator.
use super::*;
pub(super) fn frame(mut k: doc::Key) -> DMat4 {
    let d = doc::lock();
    let mut m = DMat4::IDENTITY;
    while let Some(n) = d.node(k) {
        let Data::Placement(p) = &n.data else { break };
        if let Some(x) = &p.xform {
            m = DMat4::from_cols_array(&x.doubles()) * m;
        }
        let Some(parent) = n.owners.first() else {
            break;
        };
        k = *parent;
    }
    m
}
pub(super) fn origin(
    scene: &Obj<'_>,
    table: &LiveTable,
    r: &Reference,
    row: &Row,
) -> PyResult<Option<DVec3>> {
    let py = scene.py();
    let owner = table.groups[&row.owner];
    let mut reference = r;
    let mut seen = HashSet::new();
    while let Reference::Slot { slot } = reference {
        if !seen.insert(slot) {
            return Ok(None);
        }
        let Some(bound) = row.slots.get(slot) else {
            return Ok(None);
        };
        reference = bound;
    }
    let (g, local) = match reference {
        Reference::Slot { .. } => unreachable!(),
        Reference::Point(p) => {
            let local = {
                let d = doc::lock();
                let Some(Data::Placement(placement)) = d.node(owner).map(|n| &n.data) else {
                    return Ok(None);
                };
                placement
                    .xform
                    .as_ref()
                    .map(|x| DMat4::from_cols_array(&x.doubles()))
                    .unwrap_or(DMat4::IDENTITY)
            };
            return Ok(Some(local.inverse().transform_point3(DVec3::from(*p))));
        }
        Reference::Axis { sweep, at, point } => {
            let Some(source) = table.rows.get(sweep).filter(|s| s.state == "ok") else {
                return Ok(None);
            };
            let Some(host) = group(py, table, &source.owner) else {
                return Ok(None);
            };
            if source.kind == "sweep" {
                let p = host
                    .getattr("ext")?
                    .get_item(EXT)?
                    .get_item("path")?
                    .extract::<Vec<[f64; 3]>>()?;
                let len: f64 = p
                    .windows(2)
                    .map(|v| DVec3::from(v[0]).distance(v[1].into()))
                    .sum();
                let mut station = match at {
                    table::Station::Fraction(t) => len * t,
                    table::Station::End(s) if s == "start" => 0.,
                    table::Station::End(s) if s == "end" => len,
                    table::Station::Offset { from, mm } if from == "start" => mm / 1000.,
                    table::Station::Offset { from, mm } if from == "end" => len - mm / 1000.,
                    _ => return Ok(None),
                };
                if !matches!(point.as_deref(), None | Some("origin")) {
                    return Ok(None);
                }
                for (i, pair) in p.windows(2).enumerate() {
                    let (a, b) = (DVec3::from(pair[0]), DVec3::from(pair[1]));
                    let span = a.distance(b);
                    if station <= span || i + 2 == p.len() {
                        let local = a + (b - a) * (station / span);
                        return Ok(Some(frame(owner).inverse().transform_point3(
                            frame(table.groups[&source.owner]).transform_point3(local),
                        )));
                    }
                    station -= span;
                }
                return Ok(None);
            }
            // The test generator exposes its bottom left corner as the path
            // origin, local X as its axis, and a named profile centre.
            let store = host
                .getattr("mesh")?
                .getattr("_store")?
                .cast_into::<PyMeshStore>()?;
            let s = store.try_borrow()?;
            let Some(start) = s
                .inner
                .live_slots(Kind::Vertex)
                .iter()
                .map(|v| DVec3::from(*s.inner.state.vpos.get(*v as usize)))
                .reduce(DVec3::min)
            else {
                return Ok(None);
            };
            let len = source.size[0];
            let x = match at {
                table::Station::Fraction(t) => t * len,
                table::Station::End(end) if end == "start" => 0.,
                table::Station::End(end) if end == "end" => len,
                table::Station::Offset { from, mm } if from == "end" => len - mm / 1000.,
                table::Station::Offset { from, mm } if from == "start" => mm / 1000.,
                _ => return Ok(None),
            };
            let local = match point.as_deref() {
                None | Some("origin") => DVec3::new(x, 0., 0.),
                Some("centre") => DVec3::new(x, source.size[1] * 0.5, source.size[2] * 0.5),
                _ => return Ok(None),
            };
            (Some(host), start + local)
        }
        Reference::Placement { placement } => (group(py, table, placement), DVec3::ZERO),
        Reference::Row { row } => {
            let Some(source) = table.rows.get(row).filter(|s| s.state == "ok") else {
                return Ok(None);
            };
            (group(py, table, &source.owner), DVec3::ZERO)
        }
        Reference::Element { path, id } => {
            let (g, mesh) = if path.is_empty() {
                (None, loose_mesh(scene)?)
            } else {
                let g = scene.call_method1("resolve_occurrence", (path.clone(),))?;
                if g.is_none() {
                    return Ok(None);
                }
                let m = g.getattr("mesh")?;
                (Some(g), m)
            };
            let Ok((letter, id)) = eid::from_text(id) else {
                return Ok(None);
            };
            let k = Kind::parse(eid::kind_name(letter).unwrap()).unwrap();
            let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
            let borrow = store.try_borrow()?;
            let s = &borrow.inner;
            let Some(slot) = s.slot_of_pid(k, id) else {
                return Ok(None);
            };
            let vertices = match k {
                Kind::Vertex => vec![slot],
                Kind::Edge => s.state.ends.get(slot as usize).to_vec(),
                Kind::Face => s.face_rings(slot).remove(0),
            };
            let point = vertices
                .iter()
                .map(|v| {
                    DVec3::from(
                        s.state
                            .frame
                            .definition_point(*s.state.vpos.get(*v as usize)),
                    )
                })
                .sum::<DVec3>()
                / vertices.len() as f64;
            if path.is_empty() {
                return Ok(Some(frame(owner).inverse().transform_point3(point)));
            }
            (g, point)
        }
    };
    let Some(g) = g else { return Ok(None) };
    let k = g.getattr("_doc_key")?.extract()?;
    Ok(Some(
        frame(owner)
            .inverse()
            .transform_point3(frame(k).transform_point3(local)),
    ))
}

/// Only transforms between the two frames affect an anchor. Moving their
/// common assembly preserves the relation and schedules no work.
pub(super) fn bind_frames(t: &mut LiveTable, uid: &str) -> bool {
    let row = &t.rows[uid];
    let mut result = HashSet::new();
    if row.references().next().is_some() {
        let d = doc::lock();
        let chain = |mut key: Option<doc::Key>| {
            let mut keys = HashSet::new();
            while let Some(k) = key {
                let Some(n) = d.node(k) else { break };
                if !matches!(&n.data, Data::Placement(_)) {
                    break;
                }
                keys.insert(k);
                key = n.owners.first().copied();
            }
            keys
        };
        let owner = t.groups.get(&row.owner).copied();
        let local = chain(owner);
        let root = owner.and_then(|mut k| loop {
            let node = d.node(k)?;
            if matches!(node.data, Data::Root { .. }) {
                break Some(k);
            }
            k = *node.owners.first()?;
        });
        let resolve_path = |path: &[String]| {
            let mut key = root?;
            for uid in path {
                let mut children=d.node(key)?.children.iter().filter(|&&child|
                    matches!(d.node(child).map(|n|&n.data),Some(Data::Placement(p)) if p.uid.as_ref()==Some(uid)));
                key = *children.next()?;
                if children.next().is_some() {
                    return None;
                }
            }
            Some(key)
        };
        for input in row.references() {
            let target = match input {
                Reference::Point(_) => {
                    result.extend(owner);
                    continue;
                }
                Reference::Slot { .. } => continue,
                Reference::Placement { placement } => t.groups.get(placement).copied(),
                Reference::Element { path, .. } => resolve_path(path),
                Reference::Axis { sweep, .. } | Reference::Row { row: sweep } => t
                    .rows
                    .get(sweep)
                    .and_then(|r| t.groups.get(&r.owner))
                    .copied(),
            };
            result.extend(local.symmetric_difference(&chain(target)).copied());
        }
    }
    t.bind_frames(uid, result.into_iter().collect())
}

/// Resolve an occurrence's edge or face vertices in the generated definition.
fn element_points(
    scene: &Obj<'_>,
    t: &LiveTable,
    row: &Row,
    r: &Reference,
    kind: Kind,
) -> PyResult<Option<Vec<DVec3>>> {
    let mut r = r;
    let mut visited = HashSet::new();
    while let Reference::Slot { slot } = r {
        if !visited.insert(slot) {
            return Ok(None);
        }
        let Some(next) = row.slots.get(slot) else {
            return Ok(None);
        };
        r = next;
    }
    let Reference::Element { path, id } = r else {
        return Ok(None);
    };
    let Ok((letter, pid)) = eid::from_text(id) else {
        return Ok(None);
    };
    if eid::kind_name(letter).and_then(Kind::parse) != Some(kind) {
        return Ok(None);
    }
    let (mesh, m) = if path.is_empty() {
        (loose_mesh(scene)?, DMat4::IDENTITY)
    } else {
        let g = scene.call_method1("resolve_occurrence", (path,))?;
        if g.is_none() {
            return Ok(None);
        }
        (g.getattr("mesh")?, frame(g.getattr("_doc_key")?.extract()?))
    };
    let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
    let borrow = store.try_borrow()?;
    let s = &borrow.inner;
    let Some(slot) = s.slot_of_pid(kind, pid) else {
        return Ok(None);
    };
    let ring = if kind == Kind::Edge {
        s.state.ends.get(slot as usize).to_vec()
    } else {
        s.face_rings(slot).remove(0)
    };
    let m = frame(t.groups[&row.owner]).inverse() * m;
    Ok(Some(
        ring.into_iter()
            .map(|v| {
                m.transform_point3(
                    s.state
                        .frame
                        .definition_point(*s.state.vpos.get(v as usize))
                        .into(),
                )
            })
            .collect(),
    ))
}

pub(super) fn sweep_path(
    scene: &Obj<'_>,
    t: &LiveTable,
    row: &Row,
    path: &sweep::Path,
) -> PyResult<Option<Vec<[f64; 3]>>> {
    let mut points = Vec::<DVec3>::new();
    if !path.chain.is_empty() {
        for r in &path.chain {
            let Some(edge) = element_points(scene, t, row, r, Kind::Edge)? else {
                return Ok(None);
            };
            let [a, b] = [edge[0], edge[1]];
            if points.is_empty() {
                points.extend([a, b]);
                continue;
            }
            if points.len() == 2 && (points[0].distance(a) < 1e-9 || points[0].distance(b) < 1e-9) {
                points.reverse()
            }
            let end = *points.last().unwrap();
            if end.distance(a) < 1e-9 {
                points.push(b)
            } else if end.distance(b) < 1e-9 {
                points.push(a)
            } else {
                return Ok(None);
            }
        }
        if path.closed && points.last().is_some_and(|p| p.distance(points[0]) < 1e-9) {
            points.pop();
        }
    } else {
        for p in &path.points {
            let Some(p) = origin(scene, t, &Reference::Point(*p), row)? else {
                return Ok(None);
            };
            points.push(p);
        }
    }
    if let Some(ends) = &path.ends {
        for (i, end) in ends.iter().enumerate() {
            if let sweep::End::Point(p) = end {
                let Some(p) = origin(scene, t, &Reference::Point(*p), row)? else {
                    return Ok(None);
                };
                if i < points.len() {
                    points[i] = p
                } else {
                    points.push(p)
                }
            }
        }
        if points.len() != 2 {
            return Ok(None);
        }
        let Some(dir) = (points[1] - points[0]).try_normalize() else {
            return Ok(None);
        };
        let original = [points[0], points[1]];
        for (i, end) in ends.iter().enumerate() {
            if let sweep::End::Anchored { to, offset, extend } = end {
                let Some(face) = element_points(scene, t, row, to, Kind::Face)? else {
                    return Ok(None);
                };
                let Some(n) =
                    crate::snap_planes::face_plane(face.iter().map(|v| v.to_array()).collect())
                        .map(|(_, n)| DVec3::from(n))
                else {
                    return Ok(None);
                };
                let dot = n.dot(dir);
                if dot.abs() < 1e-10 {
                    return Ok(None);
                }
                let dist = n.dot(face[0] - original[i]) / dot
                    + if i == 0 {
                        offset / 1000.
                    } else {
                        -offset / 1000.
                    };
                if !extend && ((i == 0 && dist < -1e-9) || (i == 1 && dist > 1e-9)) {
                    return Ok(None);
                }
                points[i] = original[i] + dist * dir;
            }
        }
        if (points[1] - points[0]).dot(dir) <= 1e-9 {
            return Ok(None);
        }
    }
    Ok(Some(points.into_iter().map(|p| p.to_array()).collect()))
}
