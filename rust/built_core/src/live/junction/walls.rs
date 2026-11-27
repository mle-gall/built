// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

fn priority(scene: &Obj<'_>, t: &LiveTable, row: &Row) -> (bool, i64, std::cmp::Reverse<u64>) {
    let bearing = group(scene.py(), t, &row.owner)
        .and_then(|g| g.getattr("ifc").ok())
        .and_then(|record| {
            record
                .get_item("psets")
                .and_then(|v| v.get_item("Pset_WallCommon"))
                .and_then(|v| v.get_item("LoadBearing"))
                .or_else(|_| record.get_item("load_bearing"))
                .ok()
        })
        .and_then(|v| {
            v.extract::<bool>()
                .ok()
                .or_else(|| v.get_item("value").ok()?.extract::<bool>().ok())
        })
        .unwrap_or(false);
    let width = row
        .sweep
        .as_ref()
        .and_then(|s| s.profile.outline(s.segments).ok())
        .map(|o| (o.width * 1000.).round() as i64)
        .unwrap_or(0);
    (bearing, width, std::cmp::Reverse(t.order[&row.uid]))
}

pub(in crate::live) fn apply_ends(scene: &Obj<'_>, t: &mut LiveTable, uid: &str) -> PyResult<()> {
    let new = t.rows[uid].clone();
    let spec = new.sweep.as_ref().unwrap();
    if spec.profile.library != "basic/wall" || spec.path.points.len() < 2 {
        return Ok(());
    }
    let m = reference::frame(t.groups[&new.owner]);
    let points = &spec.path.points;
    let mut connected = false;
    for (end, i, k) in [("start", 0, 1), ("end", points.len() - 1, points.len() - 2)] {
        let snapped =
            spec.path
                .ends
                .as_ref()
                .and_then(|ends| match &ends[usize::from(end == "end")] {
                    sweep::End::Anchored {
                        to: Reference::Element { path, .. },
                        ..
                    } => path.last(),
                    _ => None,
                });
        let tip = m.transform_point3(points[i].into());
        let dir = m
            .transform_vector3(DVec3::from(points[i]) - DVec3::from(points[k]))
            .normalize_or_zero();
        let mut best = None;
        for other in t.rows.values().filter(|r| {
            r.uid != uid
                && r.sweep
                    .as_ref()
                    .is_some_and(|s| s.profile.library == "basic/wall")
        }) {
            let s = other.sweep.as_ref().unwrap();
            let matrix = reference::frame(t.groups[&other.owner]);
            for (span, ab) in s.path.points.windows(2).enumerate() {
                let a = matrix.transform_point3(ab[0].into());
                let b = matrix.transform_point3(ab[1].into());
                let run = b - a;
                if run.length_squared() < 1e-12
                    || dir.cross(run.normalize()).length_squared() < 1e-8
                {
                    continue;
                }
                let at = ((tip - a).dot(run) / run.length_squared()).clamp(0., 1.);
                let distance = tip.distance(a + at * run);
                if distance > 1e-5 && snapped != Some(&other.owner) {
                    continue;
                }
                let terminal = if span == 0 && tip.distance(a) < 1e-5 {
                    Some("start")
                } else if span + 2 == s.path.points.len() && tip.distance(b) < 1e-5 {
                    Some("end")
                } else {
                    None
                };
                let rank = (terminal.is_none(), t.order[&other.uid]);
                if best.as_ref().is_none_or(|(old, _, _)| rank < *old) {
                    best = Some((rank, other.clone(), terminal));
                }
            }
        }
        if let Some((_, old, terminal)) = best {
            let mut main = json!({"placement":old.owner,"end":terminal.unwrap_or("end")});
            let mut joining = json!({"placement":new.owner,"end":end});
            let detail = if terminal.is_some() {
                if priority(scene, t, &new) > priority(scene, t, &old) {
                    std::mem::swap(&mut main, &mut joining);
                }
                "f4f093aed0396c6f"
            } else {
                "a2ff384d2a3b97b3"
            };
            let preset = presets::lookup(t, detail).ok_or_else(|| err("Missing wall detail"))?;
            let mut j:Row=serde_json::from_value(json!({"uid":eid::new_uid(),"kind":"junction","detail":{"uid":detail},"roles":{"main":main,"joining":joining}})).map_err(err)?;
            prepare(scene, t, &mut j, &preset)?;
            Arc::make_mut(&mut t.presets).insert(preset.uid.clone(), preset);
            t.pending.insert(j.uid.clone());
            t.insert(j).map_err(err)?;
            connected = true;
        }
    }
    if connected {
        if let Some(wall) = group(scene.py(), t, &new.owner) {
            scene.call_method1("select", (vec![wall],))?;
        }
    }
    Ok(())
}
