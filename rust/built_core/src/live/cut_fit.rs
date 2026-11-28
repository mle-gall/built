//! Match the end of a swept section to a finite face on the picked member.
use super::*;
use crate::store::Store;
use serde_json::{json, Value};

fn rings(store: &Store, face: u32, world: DMat4) -> Vec<Vec<DVec3>> {
    store
        .face_rings(face)
        .into_iter()
        .map(|ring| {
            ring.into_iter()
                .map(|v| {
                    let p = store
                        .state
                        .frame
                        .definition_point(*store.state.vpos.get(v as usize));
                    world.transform_point3(p.into())
                })
                .collect()
        })
        .collect()
}

fn plane(ring: &[DVec3]) -> Option<(DVec3, DVec3)> {
    let (origin, normal) =
        crate::snap_planes::face_plane(ring.iter().map(|p| p.to_array()).collect())?;
    Some((origin.into(), DVec3::from(normal).try_normalize()?))
}

fn on_face(point: DVec3, normal: DVec3, rings: &[Vec<DVec3>]) -> bool {
    let u = normal.any_orthonormal_vector();
    let v = normal.cross(u);
    let xy = |p: DVec3| [p.dot(u), p.dot(v)];
    let contains = |r: &Vec<DVec3>| {
        crate::object_outline::polygon_contains_2d(
            xy(point),
            r.iter().map(|p| xy(*p)).collect(),
            false,
        )
    };
    rings.first().is_some_and(contains) && !rings.iter().skip(1).any(contains)
}

pub(crate) fn offer(scene: &Obj<'_>, args: &Value) -> PyResult<Value> {
    let Some(t) = current(scene.py(), root(scene)?) else {
        return Ok(Value::Null);
    };
    let Some(target) = args["target"]
        .as_str()
        .and_then(|id| t.owners.get(id))
        .and_then(|id| t.rows.get(id))
    else {
        return Ok(Value::Null);
    };
    let Some(sweep) = &target.sweep else {
        return Ok(Value::Null);
    };
    let Some(body) = args["body"].as_str() else {
        return Ok(Value::Null);
    };
    let Some(cutter) = group(scene.py(), &t, body) else {
        return Ok(Value::Null);
    };
    let Some(owner) = group(scene.py(), &t, &target.owner) else {
        return Ok(Value::Null);
    };
    let explicit = args["explicit"] == true;
    let Some(path) = reference::sweep_path(scene, &t, target, &sweep.path)? else {
        return Ok(Value::Null);
    };
    if path.len() < 2 || body == target.owner {
        return Ok(Value::Null);
    }
    // A closed or multi-span sweep needs an explicit span from the first pick.
    let span = args["span"].as_u64().unwrap_or(0) as usize;
    let count = if sweep.path.closed {
        path.len()
    } else {
        path.len() - 1
    };
    if span >= count || (count > 1 && args["span"].is_null()) {
        return Ok(Value::Null);
    }
    let world = reference::frame(t.groups[&target.owner]);
    let ends =
        [path[span], path[(span + 1) % path.len()]].map(|p| world.transform_point3(p.into()));
    let Some(axis) = (ends[1] - ends[0]).try_normalize() else {
        return Ok(Value::Null);
    };
    let target_store = owner
        .getattr("mesh")?
        .getattr("_store")?
        .cast_into::<PyMeshStore>()?;
    let target_store = target_store.try_borrow()?;
    let stock = &target_store.inner;
    let cut_store = cutter
        .getattr("mesh")?
        .getattr("_store")?
        .cast_into::<PyMeshStore>()?;
    let cut_store = cut_store.try_borrow()?;
    let tool = &cut_store.inner;
    let cutter_world = reference::frame(t.groups[body]);
    let mut best = None;
    let mut distance = f64::INFINITY;
    let picked = args["face"]["id"].as_str().unwrap_or_default();
    let section = sweep.profile.outline(sweep.segments);
    let reach = section
        .map(|s| s.width.max(s.height) / 1000.)
        .unwrap_or(0.)
        .min(ends[0].distance(ends[1]) * 0.45);
    for face in tool.live_slots(Kind::Face) {
        let id = eid::to_text('f', tool.pid(Kind::Face, face));
        if explicit && id != picked {
            continue;
        }
        let rings = rings(tool, face, cutter_world);
        let Some((at, normal)) = rings.first().and_then(|r| plane(r)) else {
            continue;
        };
        for (i, tip) in ends.iter().enumerate() {
            let outward = if i == 1 { axis } else { -axis };
            let den = normal.dot(outward);
            if den.abs() < 1e-6 || (!explicit && den > 0.) {
                continue;
            }
            let travel = (at - *tip).dot(normal) / den;
            if travel.abs() >= distance
                || (!explicit
                    && (travel.abs() > reach || !on_face(*tip + travel * outward, normal, &rings)))
            {
                continue;
            }
            distance = travel.abs();
            best = Some((i, *tip, outward, at, normal, id.clone(), travel));
        }
    }
    let Some((i, tip, outward, mut at, normal, id, travel)) = best else {
        return Ok(if explicit {
            json!({"reason":"Pick a face that crosses the beam axis"})
        } else {
            Value::Null
        });
    };
    // Project the actual cap outline along the member axis onto the offered plane.
    let cap = stock
        .live_slots(Kind::Face)
        .into_iter()
        .filter_map(|f| {
            let rings = rings(stock, f, world);
            let (centre, n) = plane(rings.first()?)?;
            (n.dot(outward) > 0.95).then(|| ((centre - tip).dot(outward).abs(), rings))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0));
    // A square end already on the web has nothing left to trim. Offer the
    // project's Cope clearance explicitly and allow a typed zero for flush.
    let flush = cap.as_ref().is_some_and(|(_, rings)| {
        rings
            .iter()
            .flatten()
            .all(|p| (*p - at).dot(normal).abs() < 1e-5)
    });
    let allowance = (!explicit
        && flush
        && args["typed"] != true
        && t.owners
            .get(body)
            .is_some_and(|id| t.rows[id].sweep.is_some()))
    .then(|| args["flush_clearance"].as_f64())
    .flatten()
    .filter(|mm| *mm > 0.);
    let offset = allowance.or_else(|| args["offset"].as_f64()).unwrap_or(0.);
    at += normal * offset / 1000.;
    let mut lines = Vec::new();
    if let Some((_, cap)) = cap {
        for ring in cap {
            let project =
                |p: DVec3| (p + outward * ((at - p).dot(normal) / outward.dot(normal))).to_array();
            for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                lines.push([project(*a), project(*b)]);
            }
        }
    }
    let change = travel + offset / 1000. / outward.dot(normal);
    let prompt = if let Some(mm) = allowance {
        format!("Click for {mm} mm clearance · type 0 for flush")
    } else if change.abs() < 0.00001 {
        "End is flush · click to fit or press C to cope".to_owned()
    } else {
        format!(
            "Click to {} end by {:.1} mm",
            if change > 0. { "extend" } else { "trim" },
            change.abs() * 1000.
        )
    };
    Ok(
        json!({"face":{"path":args["face"]["path"],"id":id},"span":span,"end":if i==1 {"end"} else {"start"},"lines":lines,"offset":offset,"prompt":prompt,"flush":change.abs()<0.00001}),
    )
}

/// A corner has two nearby ends and intersecting stock axes. A beam meeting
/// the middle of a column cuts only the selected beam.
pub(crate) fn corner(scene: &Obj<'_>, args: &Value) -> PyResult<Value> {
    let Some(t) = current(scene.py(), root(scene)?) else {
        return Ok(Value::Null);
    };
    let mut axes = Vec::new();
    for key in ["target", "body"] {
        let Some(row) = args[key]
            .as_str()
            .and_then(|u| t.owners.get(u))
            .and_then(|u| t.rows.get(u))
        else {
            return Ok(Value::Null);
        };
        let Some(sweep) = &row.sweep else {
            return Ok(Value::Null);
        };
        let Some(path) = reference::sweep_path(scene, &t, row, &sweep.path)? else {
            return Ok(Value::Null);
        };
        if path.len() != 2 || sweep.path.closed {
            return Ok(Value::Null);
        }
        let frame = reference::frame(t.groups[&row.owner]);
        let ends = [path[0], path[1]].map(|p| frame.transform_point3(p.into()));
        let Ok(section) = sweep.profile.outline(sweep.segments) else {
            return Ok(Value::Null);
        };
        axes.push((ends, section.width.max(section.height) / 1000., &row.uid));
    }
    let (a, b) = (&axes[0], &axes[1]);
    let (i, j) = [(0, 0), (0, 1), (1, 0), (1, 1)]
        .into_iter()
        .min_by(|&(i, j), &(k, l)| {
            a.0[i]
                .distance_squared(b.0[j])
                .total_cmp(&a.0[k].distance_squared(b.0[l]))
        })
        .unwrap();
    let reach =
        a.1.max(b.1)
            .min(0.45 * a.0[0].distance(a.0[1]))
            .min(0.45 * b.0[0].distance(b.0[1]));
    let x = (a.0[1 - i] - a.0[i]).normalize_or_zero();
    let y = (b.0[1 - j] - b.0[j]).normalize_or_zero();
    let n = x.cross(y);
    let diff = b.0[j] - a.0[i];
    if n.length_squared() < 1e-8 || diff.length() > reach || diff.dot(n.normalize()).abs() > 1e-5 {
        return Ok(Value::Null);
    }
    let end = |i| if i == 1 { "end" } else { "start" };
    Ok(
        json!({"target_row":a.2,"partner_row":b.2,"end":end(i),"partner_end":end(j),
        "prompt":"Click to mitre both beam ends"}),
    )
}
