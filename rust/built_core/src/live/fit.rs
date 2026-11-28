// SPDX-License-Identifier: GPL-3.0-or-later
//! Sized from a host member: a footing from the column or wall above it, a
//! capping from its wall. The member's `fit` names the host and gives each
//! fitted field as an expression over the host's stock dimensions, in the
//! junction parameters' grammar (`host.width + 300`). The values are
//! written into the row before it is evaluated, so the saved row, the
//! record and the exports always carry the resolved numbers.
use super::*;

/// Write `value[field] = n`; false when the field is not one a fit may set.
fn slot<'a>(value: &'a mut serde_json::Value, field: &str) -> Option<&'a mut serde_json::Value> {
    match field {
        "path.depth" => Some(&mut value["path"]["depth"]),
        "rotation" => Some(&mut value["rotation"]),
        "shift.x" | "shift.y" => {
            if !value["shift"].is_array() {
                value["shift"] = serde_json::json!([0., 0.]);
            }
            Some(&mut value["shift"][usize::from(field == "shift.y")])
        }
        _ => {
            let name = field.strip_prefix("profile.params.")?;
            if name.is_empty() || name.contains('.') {
                return None;
            }
            Some(&mut value["profile"]["params"][name])
        }
    }
}

/// The fields a fit may set.
pub(super) fn validate(fit: &sweep::Fit) -> Result<(), String> {
    for (field, e) in &fit.values {
        if slot(&mut serde_json::json!({}), field).is_none() {
            return Err(format!("Cannot size {field} from a host"));
        }
        for name in expr::names(&serde_json::json!(e))? {
            if !name.starts_with("host.") {
                return Err(format!("Unknown name {name}"));
            }
        }
    }
    Ok(())
}

/// Resolve `uid`'s fitted fields from its host's stock. A host that is gone
/// or does not resolve leaves the member as it was.
pub(super) fn apply(scene: &Obj<'_>, t: &mut LiveTable, uid: &str) -> PyResult<()> {
    let Some(row) = t.rows.get(uid) else {
        return Ok(());
    };
    let Some(fit) = row.sweep.as_ref().and_then(|s| s.fit.as_ref()) else {
        return Ok(());
    };
    let Some(Reference::Axis { sweep: host, .. }) = &fit.host else {
        return Ok(());
    };
    let Some(source) = t.rows.get(host) else {
        return Ok(());
    };
    let stock = match t.stock.get(host) {
        Some(s) => s.clone(),
        None => Arc::new(junction::stock(scene, t, source)?),
    };
    if stock.state != "ok" {
        return Ok(());
    }
    let dims: expr::Values = stock.dims.iter().map(|(k, v)| (format!("host.{k}"), *v)).collect();
    let mut value = serde_json::json!(row.sweep);
    let mut changed = false;
    for (field, e) in &fit.values {
        let Ok(n) = expr::field(&serde_json::json!(e), &expr::Values::new(), &dims) else {
            continue;
        };
        let Some(at) = slot(&mut value, field) else {
            continue;
        };
        if at.as_f64().is_none_or(|old| (old - n).abs() > 1e-9) {
            *at = serde_json::json!(n);
            changed = true;
        }
    }
    if changed {
        let Ok(resolved) = serde_json::from_value::<sweep::Sweep>(value) else {
            return Ok(());
        };
        // The references are the same: the table's indexes stay valid.
        Arc::make_mut(t.rows.get_mut(uid).unwrap()).sweep = Some(Box::new(resolved));
    }
    Ok(())
}

/// A fitted field edited by hand stops following the host: its relation is
/// dropped, the others stay.
pub(super) fn release_edited(old: &sweep::Sweep, new: &mut sweep::Sweep) {
    let (Some(before), Some(fit)) = (&old.fit, &new.fit) else {
        return;
    };
    if before != fit {
        return;
    }
    let (mut a, mut b) = (serde_json::json!(old), serde_json::json!(new));
    let edited: Vec<String> = fit
        .values
        .keys()
        .filter(|field| match (slot(&mut a, field).map(|v| v.clone()), slot(&mut b, field).map(|v| v.clone())) {
            (Some(x), Some(y)) => match (x.as_f64(), y.as_f64()) {
                (Some(x), Some(y)) => (x - y).abs() > 1e-9,
                _ => x != y,
            },
            _ => false,
        })
        .cloned()
        .collect();
    if let Some(fit) = &mut new.fit {
        for field in edited {
            fit.values.remove(&field);
        }
        if fit.values.is_empty() {
            new.fit = None;
        }
    }
}

/// A preset's fitted fields worked out for given host dimensions
/// (`{"width": mm, …}`): what a tool previews before the member exists.
pub(crate) fn resolve(values: &serde_json::Value, dims: &serde_json::Value) -> serde_json::Value {
    let dims: expr::Values = dims
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| Some((format!("host.{k}"), v.as_f64()?)))
        .collect();
    let mut out = serde_json::Map::new();
    for (field, e) in values.as_object().into_iter().flatten() {
        if let Ok(n) = expr::field(e, &expr::Values::new(), &dims) {
            out.insert(field.clone(), serde_json::json!(n));
        }
    }
    out.into()
}

/// What a member offers something placed against it (a footing under it,
/// a capping on it): its row, its axis and section frame in the world, its
/// section's size and where the section's centre sits off the axis.
pub(crate) fn host(scene: &Obj<'_>, owner: &str) -> PyResult<serde_json::Value> {
    let py = scene.py();
    let none = serde_json::Value::Null;
    let Some(t) = current(py, root(scene)?) else {
        return Ok(none);
    };
    let Some(row) = t.owners.get(owner).and_then(|uid| t.rows.get(uid)).filter(|r| r.kind == "sweep" && r.state == "ok") else {
        return Ok(none);
    };
    let (Some(s), Some(g), Some(k)) = (row.sweep.as_deref(), group(py, &t, owner), t.groups.get(owner)) else {
        return Ok(none);
    };
    let Some(path) = metadata(&g)?
        .get_item(EXT)?
        .and_then(|m| m.get_item("path").ok())
        .and_then(|v| v.extract::<Vec<[f64; 3]>>().ok())
        .filter(|p| p.len() >= 2)
    else {
        return Ok(none);
    };
    let mut profile = s.profile.clone();
    if let Some(id) = &profile.sketch {
        profile.drawing = t.rows.get(id).filter(|r| r.state == "ok").map(|r| serde_json::json!(r));
    }
    let Ok(o) = profile.outline(s.segments) else {
        return Ok(none);
    };
    let Ok(origin) = s.origin(&o) else {
        return Ok(none);
    };
    let world = reference::frame(*k);
    let points: Vec<DVec3> = path.iter().map(|p| world.transform_point3(DVec3::from(*p))).collect();
    let Some(dir) = (DVec3::from(path[1]) - DVec3::from(path[0])).try_normalize() else {
        return Ok(none);
    };
    let across = DVec3::Z.cross(dir).try_normalize().unwrap_or(DVec3::X);
    let up = dir.cross(across);
    let mx = if s.mirror { -1. } else { 1. };
    let turn = glam::DVec2::from_angle(s.rotation.to_radians());
    let section = |v: glam::DVec2| {
        let v = turn.rotate(glam::DVec2::new(v.x * mx, v.y));
        world.transform_vector3(across * v.x + up * v.y)
    };
    let centre = section((glam::DVec2::new(o.width, o.height) * 0.5 - origin) / 1000.);
    let class = g
        .getattr("ifc")
        .ok()
        .filter(|r| !r.is_none())
        .and_then(|r| r.get_item("class").ok())
        .and_then(|c| c.extract::<String>().ok())
        .unwrap_or_default();
    let nine = |name: &str| -> Option<[f64; 2]> {
        Some(match name {
            "centre" => [0.5, 0.5],
            "bottom" | "bottom_centre" => [0.5, 0.],
            "top" | "top_centre" => [0.5, 1.],
            "left" | "middle_left" => [0., 0.5],
            "right" | "middle_right" => [1., 0.5],
            "bottom_left" => [0., 0.],
            "bottom_right" => [1., 0.],
            "top_left" => [0., 1.],
            "top_right" => [1., 1.],
            _ => return None,
        })
    };
    Ok(serde_json::json!({
        "row": row.uid, "placement": owner, "class": class,
        "points": points.iter().map(|p| p.to_array()).collect::<Vec<_>>(),
        "x": section(glam::DVec2::X).normalize_or_zero().to_array(),
        "y": section(glam::DVec2::Y).normalize_or_zero().to_array(),
        "centre": centre.to_array(),
        "dims": {"width": o.width, "height": o.height, "thickness": o.width,
                 "length": points.windows(2).map(|w| w[0].distance(w[1])).sum::<f64>() * 1000.},
        // Where the path sits on the section, as fractions of its box, when
        // the attachment is one of the nine and nothing shifts it.
        "attach": if s.shift == [0.; 2] { nine(&s.attachment) } else { None },
        "rotation": s.rotation, "mirror": s.mirror,
        "extruded": s.path.depth.is_some(),
    }))
}
