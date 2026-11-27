// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

const OPENING_DETAIL: &str = "8ccdd727386a50c6";

pub(in crate::live) fn adopt(
    scene: &Obj<'_>,
    t: &mut LiveTable,
    cut: &Row,
    fill: &str,
    migration: bool,
) -> PyResult<()> {
    let uid = eid::derived_uid(&format!("built:junction:v1:hosted:{}:{fill}", cut.owner));
    let wall_key = t.groups[&cut.owner];
    let fill_key = t.groups[fill];
    let relative =
        (reference::frame(wall_key).inverse() * reference::frame(fill_key)).to_cols_array();
    if let Some(existing) = t.rows.get(&uid) {
        if existing.kind != "junction" || existing.joint.roles["hosted"]["placement"] != fill {
            return Err(err("Hosted junction identity collision"));
        }
        let mut row = cut.clone();
        row.junction = Some(uid.clone());
        row.key = Some("opening".into());
        row.inputs.clear();
        t.insert(row).map_err(err)?;
        let j = Arc::make_mut(t.rows.get_mut(&uid).unwrap());
        j.joint.reason = Value::Null;
        j.joint.roles["hosted"]["relative"] = json!(relative);
        t.pending.insert(uid);
        return Ok(());
    }
    let hosted = group(scene.py(), t, fill).ok_or_else(|| err("Missing hosted placement"))?;
    let selected = if migration {
        None
    } else {
        metadata(&hosted)?
            .get_item("junction")?
            .and_then(|v| v.extract::<String>().ok())
    };
    let p = presets::lookup(t, selected.as_deref().unwrap_or(OPENING_DETAIL))
        .filter(|p| p.values.get("meeting") == Some(&json!("hosted")))
        .ok_or_else(|| err("Missing hosted detail"))?;
    let mut j: Row =
        serde_json::from_value(json!({"kind":"junction","uid":uid,"detail":{"uid":p.uid},
        "roles":{"host":{"placement":cut.owner},"hosted":{"placement":fill,"attached":true,"relative":relative}}}))
        .map_err(err)?;
    prepare(scene, t, &mut j, &p)?;
    if migration {
        // Opening a file only establishes ownership and reads the saved stock.
        // Leave its evaluated geometry and every existing element id intact.
        if let Some((frames, dims)) = geometry(t, &j) {
            let defs = p.values["params"]
                .as_array()
                .ok_or_else(|| err("Missing opening parameters"))?;
            let (params, _) = expr::resolve(defs, &Value::Null, &dims).map_err(err)?;
            j.frames = frames;
            j.joint.params = json!(params);
            j.state = "ok".into();
        }
    }
    Arc::make_mut(&mut t.presets).insert(p.uid.clone(), p);
    t.insert(j).map_err(err)?;
    if migration {
        reference::bind_frames(t, &uid);
        let g = group(scene.py(), t, &t.rows[&uid].owner)
            .ok_or_else(|| err("Missing hosted assembly"))?;
        let ext = metadata(&g)?;
        ext.set_item(
            EXT,
            to_py(scene.py(), &json!({"row":uid,"stamp":repeat::stamp(&g)?}))?,
        )?;
        g.setattr("ext", ext)?;
    } else {
        t.pending.insert(uid);
    }
    Ok(())
}

pub(super) fn geometry(t: &LiveTable, j: &Row) -> Option<(Frames, expr::Values)> {
    let host = placement(&j.joint.roles["host"])?;
    let fill = placement(&j.joint.roles["hosted"])?;
    let owner = reference::frame(*t.groups.get(&j.owner)?);
    let host_pose = reference::frame(*t.groups.get(host)?);
    let fill_pose = reference::frame(*t.groups.get(fill)?);
    let entry = j
        .joint
        .contents
        .as_array()?
        .iter()
        .find(|v| v["key"] == "opening")?;
    let cut = t.rows.get(entry["row"].as_str()?)?;
    let cut::Cutter::Opening(open) = cut.cutter.as_ref()? else {
        return None;
    };
    let f = host_pose * DMat4::from_cols_array(&open.frame);
    let aperture = host_pose * open.pose();
    let x = f.x_axis.truncate().try_normalize()?;
    let y = f.z_axis.truncate().try_normalize()?;
    let z = x.cross(y).try_normalize()?;
    let mut origin = aperture.w_axis.truncate();
    let front = if f.inverse().transform_point3(fill_pose.w_axis.truncate()).y > 0.5 {
        1.
    } else {
        0.
    };
    origin += front * f.y_axis.truncate();
    let meet = owner.inverse()
        * DMat4::from_cols(x.extend(0.), y.extend(0.), z.extend(0.), origin.extend(1.));
    let frames = ["meet", "host.contact", "host.at", "hosted.at"]
        .into_iter()
        .map(|k| (k.into(), meet.to_cols_array()))
        .collect();
    let dims = [
        ("host.thickness", f.y_axis.truncate().length()),
        ("hosted.width", aperture.x_axis.truncate().length()),
        ("hosted.height", aperture.z_axis.truncate().length()),
        ("hosted.sill", open.rect[1] * f.z_axis.truncate().length()),
        (
            "hosted.offset",
            (fill_pose.w_axis.truncate() - origin).dot(-z),
        ),
    ]
    .map(|(k, v)| (k.into(), v * 1000.))
    .into();
    Some((frames, dims))
}

pub(in crate::live) fn sync_poses(
    scene: &Obj<'_>,
    t: &LiveTable,
    moved: &HashSet<doc::Key>,
) -> PyResult<bool> {
    let affected: HashSet<_> = moved
        .iter()
        .filter_map(|k| t.frames.get(k))
        .flatten()
        .filter(|uid| t.junctions.contains(*uid))
        .collect();
    let mut recut = false;
    for uid in affected {
        let j = &t.rows[uid];
        if j.joint.meeting.as_deref() != Some("hosted") {
            continue;
        }
        let binding = &j.joint.roles["hosted"];
        let Some(fill) = placement(binding).and_then(|id| group(scene.py(), t, id)) else {
            continue;
        };
        let Some(host_key) = placement(&j.joint.roles["host"]).and_then(|id| t.groups.get(id))
        else {
            continue;
        };
        let fill_key: doc::Key = fill.getattr("_doc_key")?.extract()?;
        if moved.contains(&fill_key) {
            crate::hosted_openings::place(scene, &fill)?;
            recut = true;
        } else if let Ok(relative) =
            serde_json::from_value::<[f64; 16]>(binding["relative"].clone())
        {
            let parent = doc::lock()
                .node(fill_key)
                .and_then(|n| n.owners.first().copied());
            let local = parent
                .map(reference::frame)
                .unwrap_or(DMat4::IDENTITY)
                .inverse()
                * reference::frame(*host_key)
                * DMat4::from_cols_array(&relative);
            fill.setattr("xform", crate::math::Matrix4::from(local))?;
        }
    }
    Ok(recut)
}

/// A stock edit can move the reference face without changing the wall placement.
/// Preserve metric jamb and sill distances from the reference corner.
pub(super) fn resize(
    scene: &Obj<'_>,
    t: &mut LiveTable,
    j: &mut Row,
) -> PyResult<Result<bool, &'static str>> {
    let host = &j.joint.roles["host"];
    let Some(row) = host["row"].as_str().and_then(|id| t.rows.get(id)) else {
        return Ok(Ok(false));
    };
    let Some(s) = row
        .sweep
        .as_ref()
        .filter(|s| s.profile.library == "basic/wall")
    else {
        return Ok(Ok(false));
    };
    let Some(path) = reference::sweep_path(scene, t, row, &s.path)? else {
        return Ok(Err("wall_path"));
    };
    let [start, end] = path.as_slice() else {
        return Ok(Err("wall_path"));
    };
    let Some(entry) = j
        .joint
        .contents
        .as_array()
        .and_then(|a| a.iter().find(|v| v["key"] == "opening"))
    else {
        return Ok(Ok(false));
    };
    let cut_id = entry["row"]
        .as_str()
        .ok_or_else(|| err("Missing opening"))?;
    let Some(cut) = t.rows.get(cut_id) else {
        return Ok(Ok(false));
    };
    let Some(cut::Cutter::Opening(open)) = &cut.cutter else {
        return Ok(Ok(false));
    };
    let old = DMat4::from_cols_array(&open.frame);
    let run = DVec3::from(*end) - DVec3::from(*start);
    let Some(axis) = run.try_normalize() else {
        return Ok(Err("wall_path"));
    };
    let across = DVec3::Z.cross(axis).try_normalize().unwrap_or(DVec3::X);
    let up = axis.cross(across);
    let section = s.profile.outline(s.segments).map_err(err)?;
    let attachment = section.attachment(&s.attachment).map_err(err)?;
    let section_point = |u: f64, v: f64| {
        let mut p = (glam::DVec2::new(u, v) - attachment) / 1000.;
        if s.mirror {
            p.x = -p.x
        }
        p = glam::DVec2::from_angle(s.rotation.to_radians()).rotate(p);
        DVec3::from(*start) + across * p.x + up * p.y
    };
    let mut base = section_point(0., 0.);
    let mut cols = [
        run,
        section_point(section.width, 0.) - base,
        section_point(0., section.height) - base,
    ];
    for (col, prev) in cols.iter_mut().zip([old.x_axis, old.y_axis, old.z_axis]) {
        if col.dot(prev.truncate()) < 0. {
            base += *col;
            *col = -*col
        }
    }
    let next = DMat4::from_cols(
        cols[0].extend(0.),
        cols[1].extend(0.),
        cols[2].extend(0.),
        base.extend(1.),
    );
    if old.abs_diff_eq(next, 1e-8) {
        return Ok(Ok(false));
    }
    let [x, z, r, h] = open.rect;
    let wx = old.x_axis.truncate().length() / cols[0].length();
    let wz = old.z_axis.truncate().length() / cols[2].length();
    let rect = [x * wx, z * wz, r * wx, h * wz];
    if rect[0] <= 0. || rect[1] < -1e-7 || rect[2] >= 1. || rect[3] >= 1. {
        return Ok(Err("wall_edge"));
    }
    let fill_uid = open.fill.clone();
    let Some(wall) = group(scene.py(), t, &row.owner) else {
        return Ok(Ok(false));
    };
    let Some(fill) = group(scene.py(), t, &fill_uid) else {
        return Ok(Ok(false));
    };
    let basis = |m: DMat4| {
        DMat4::from_cols(
            m.x_axis.truncate().normalize().extend(0.),
            m.y_axis.truncate().normalize().extend(0.),
            m.z_axis.truncate().normalize().extend(0.),
            m.w_axis,
        )
    };
    let delta = basis(next) * basis(old).inverse();
    let binding = &mut j.joint.roles["hosted"];
    let relative: [f64; 16] = serde_json::from_value(binding["relative"].clone()).map_err(err)?;
    let relative = delta * DMat4::from_cols_array(&relative);
    let wall_pose = reference::frame(t.groups[&row.owner]);
    let parent = doc::lock()
        .node(t.groups[&fill_uid])
        .and_then(|n| n.owners.first().copied());
    let local = parent
        .map(reference::frame)
        .unwrap_or(DMat4::IDENTITY)
        .inverse()
        * wall_pose
        * relative;
    fill.setattr("xform", crate::math::Matrix4::from(local))?;
    binding["relative"] = json!(relative.to_cols_array());
    let fill_host = fill.getattr("ext")?.get_item("wall_host")?;
    fill_host.set_item("relative", relative.to_cols_array())?;
    wall.getattr("ext")?
        .set_item("opening_frame", next.to_cols_array())?;
    for entry in wall.getattr("ifc")?.get_item("openings")?.try_iter()? {
        let entry = entry?;
        if entry
            .get_item("fill_uid")
            .is_ok_and(|v| v.eq(&fill_uid).unwrap_or(false))
        {
            entry.set_item("wall_rect", rect)?;
        }
    }
    let cut = Arc::make_mut(t.rows.get_mut(cut_id).unwrap());
    if let Some(cut::Cutter::Opening(open)) = &mut cut.cutter {
        open.frame = next.to_cols_array();
        open.rect = rect;
    }
    Ok(Ok(true))
}

pub(crate) fn opening_conflict(scene: &Obj<'_>, wall: &Obj<'_>, fill: &Obj<'_>) -> PyResult<bool> {
    let Some(mut t) = current(scene.py(), root(scene)?) else {
        return Ok(false);
    };
    let wall: String = wall.getattr("uid")?.extract()?;
    let fill: String = fill.getattr("uid")?.extract()?;
    let uid = eid::derived_uid(&format!("built:junction:v1:hosted:{wall}:{fill}"));
    let Some(row) = t.rows.get_mut(&uid) else {
        return Ok(false);
    };
    let row = Arc::make_mut(row);
    row.state = "failed".into();
    row.joint.reason = serde_json::json!({"criterion":"opening_overlap","role":"hosted"});
    put(scene, t)?;
    Ok(true)
}
