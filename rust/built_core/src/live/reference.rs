// SPDX-License-Identifier: GPL-3.0-or-later
//! Resolve stable references into the local coordinates of the generator.
use super::*;
fn frame(mut k: doc::Key) -> DMat4 {
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
    if !row.inputs.is_empty() {
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
        for input in row.inputs.iter().chain(row.slots.values()) {
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
