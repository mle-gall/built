// SPDX-License-Identifier: GPL-3.0-or-later
//! A junction applied along a Repeat (docs/junctions.md §2.8, owner question
//! Q1): one row with `along`, one child junction per copy and end, keyed by
//! the copy, so a new copy gets its junction in the same step and a removed
//! copy's parts go. A station's own values are its child's overrides.
use super::*;
use std::collections::BTreeSet;

/// The Repeat's copies, nested copies and source: the placements whose
/// meetings the row details.
fn members_of(t: &LiveTable, repeat: &Row) -> BTreeSet<String> {
    let mut set = BTreeSet::from([repeat.owner.clone()]);
    let mut source = repeat.source.clone();
    while let Some(s) = source {
        if !set.insert(s.clone()) {
            break;
        }
        source = t.owners.get(&s).and_then(|id| t.rows.get(id)).filter(|r| r.kind == "repeat").and_then(|r| r.source.clone());
    }
    set
}

fn ends(along: &Value) -> Vec<&'static str> {
    match along["end"].as_str() {
        Some("start") => vec!["start"],
        Some("end") => vec!["end"],
        _ => vec!["start", "end"],
    }
}

/// Child junctions wanted now: key → (roles, meeting key).
fn wanted(scene: &Obj<'_>, t: &LiveTable, row: &Row, p: &presets::Preset) -> PyResult<BTreeMap<String, Value>> {
    let along = &row.joint.along;
    let repeat = along["repeat"]
        .as_str()
        .and_then(|id| t.rows.get(id))
        .filter(|r| r.kind == "repeat")
        .ok_or_else(|| err("Unknown Repeat for the junction"))?;
    let set = members_of(t, repeat);
    let kind = p.values.get("rule").and_then(|r| r["meeting"].as_str()).or_else(|| p.values.get("meeting").and_then(Value::as_str));
    let mut names: Vec<String> = p.values.get("roles").and_then(Value::as_object).map(|r| r.keys().cloned().collect()).unwrap_or_default();
    names.sort_by_key(|n| (n != "main", n.clone()));
    let (main_name, other_name) = (names.first().cloned().unwrap_or_else(|| "main".into()), names.get(1).cloned().unwrap_or_else(|| "joining".into()));
    let skips: BTreeSet<String> = along["skips"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(String::from)).collect();
    let ends = ends(along);
    let mut out = BTreeMap::new();
    for m in meetings::lite(scene)? {
        if kind.is_some_and(|k| k != m.kind) {
            continue;
        }
        // The copy side: a member inside the Repeat; the other is main.
        let inside = |i: usize| m.paths[i].iter().any(|u| set.contains(u)) || set.contains(&m.members[i]);
        let copy = match (inside(0), inside(1)) {
            (false, true) => 1,
            (true, false) => 0,
            _ => continue,
        };
        let end = m.roles[copy]["end"].as_str().unwrap_or("end");
        if !ends.contains(&end) {
            continue;
        }
        let station = &m.members[copy];
        let key = format!("{station}:{end}");
        if skips.contains(station) || skips.contains(&key) {
            continue;
        }
        out.insert(key, json!({main_name.clone(): m.roles[1 - copy], other_name.clone(): m.roles[copy], "meeting": m.key}));
    }
    Ok(out)
}

/// Expand the row: its assembly beside the Repeat, a child per station.
pub(super) fn expand(scene: &Obj<'_>, t: &mut LiveTable, row: &mut Row, p: &presets::Preset) -> PyResult<()> {
    let repeat = row.joint.along["repeat"].as_str().and_then(|id| t.rows.get(id)).cloned().ok_or_else(|| err("Unknown Repeat for the junction"))?;
    row.joint.detail = json!({"uid": p.uid, "rev": p.rev, "name": p.name});
    row.joint.meeting = p.values.get("meeting").and_then(Value::as_str).map(String::from);
    row.inputs = vec![Reference::Axis { sweep: repeat.uid.clone(), at: table::Station::End("end".into()), point: None }];
    ensure_assembly(scene, t, row, p, &repeat.owner)?;
    let want = wanted(scene, t, row, p)?;
    if want.is_empty() {
        return Err(err("No copy of this Repeat meets a member the detail fits"));
    }
    t.insert(row.clone()).map_err(err)?;
    let mut keep = HashSet::new();
    let mut contents = Vec::new();
    for (key, roles) in &want {
        let uid = id(&row.uid, key);
        let mut roles = roles.clone();
        roles.as_object_mut().map(|o| o.remove("meeting"));
        let mut child: Row = serde_json::from_value(json!({
            "kind": "junction", "uid": uid, "junction": row.uid, "parent": row.uid, "key": key,
            "detail": {"uid": p.uid}, "roles": roles,
            "overrides": row.joint.along["stations"][key]["overrides"],
        }))
        .map_err(err)?;
        if let Some(old) = t.rows.get(&uid) {
            child.joint.local = old.joint.local.clone();
            child.joint.contents = old.joint.contents.clone();
            if child.joint.overrides.is_null() {
                child.joint.overrides = old.joint.overrides.clone();
            }
        }
        prepare(scene, t, &mut child, p)?;
        contents.push(local_content(&child));
        t.insert(child).map_err(err)?;
        t.pending.insert(uid.clone());
        keep.insert(uid);
    }
    for uid in t.owned.get(&row.uid).cloned().unwrap_or_default() {
        if keep.contains(&uid) {
            continue;
        }
        let Some(child) = t.rows.get(&uid).cloned() else { continue };
        remove_descendants(t, &uid)?;
        if let (Some(assembly), Some(part)) = (group(scene.py(), t, &row.owner), group(scene.py(), t, &child.owner)) {
            assembly.getattr("children")?.call_method1("remove", (part,))?;
        }
        Arc::make_mut(&mut t.groups).remove(&child.owner);
        t.remove(&uid);
    }
    row.joint.contents = json!(contents);
    Ok(())
}

/// After a step: re-expand along rows whose stations changed (a copy
/// added or removed). True when something was re-expanded.
pub(in crate::live) fn sync(scene: &Obj<'_>, t: &mut LiveTable) -> PyResult<bool> {
    let rows: Vec<Arc<Row>> = t.junctions.iter().filter_map(|id| t.rows.get(id)).filter(|r| !r.joint.along.is_null()).cloned().collect();
    let mut changed = false;
    for r in rows {
        let Ok(p) = detail(t, &r) else { continue };
        let Ok(want) = wanted(scene, t, &r, &p) else { continue };
        let have: BTreeSet<String> = r.joint.contents.as_array().into_iter().flatten().filter_map(|c| c["key"].as_str().map(String::from)).collect();
        if want.keys().cloned().collect::<BTreeSet<_>>() == have || want.is_empty() {
            continue;
        }
        let mut row = (*r).clone();
        expand(scene, t, &mut row, &p)?;
        t.pending.insert(row.uid.clone());
        t.insert(row).map_err(err)?;
        changed = true;
    }
    Ok(changed)
}
