// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

/// Grid station zero is the source. One cut per ply references the Repeat.
pub(super) fn prepare(
    scene: &Obj<'_>,
    t: &mut LiveTable,
    j: &Row,
    child: &Row,
    spec: &Value,
    bindings: &mut Vec<Value>,
) -> PyResult<Row> {
    let key = format!(
        "{}:repeat",
        child
            .key
            .as_deref()
            .ok_or_else(|| err("Missing bolt key"))?
    );
    let uid = id(&j.uid, &key);
    let axis = spec["run"]
        .as_str()
        .unwrap_or("@meet.y")
        .strip_prefix('@')
        .ok_or_else(|| err("Expected grid frame axis"))?;
    let (frame, axis) = axis
        .rsplit_once('.')
        .ok_or_else(|| err("Expected grid frame axis"))?;
    if !matches!(axis, "x" | "y") {
        return Err(err("Expected in-plane grid axis"));
    }
    let mut rule = json!({"grid":{"rows":spec["rule"]["count"],"pitch":spec["rule"]["step"],
        "columns":spec["nest"].get("count").cloned().unwrap_or(json!(1)),
        "gauge":spec["nest"].get("step").cloned().unwrap_or(json!(0))}});
    for (name, v) in rule["grid"].as_object_mut().unwrap() {
        if v.is_string() {
            bindings.push(json!({"param":v,"row":uid,"field":format!("/rule/grid/{name}")}));
            *v = json!(1);
        }
    }
    let mut row: Row = serde_json::from_value(
        json!({"uid":uid,"key":key,"kind":"repeat","source":child.owner,"junction":j.uid,
        "run":{"frame":frame,"junction":j.uid,"axis":axis},"rule":rule,"adapt":"adaptive"}),
    )
    .map_err(err)?;
    repeat::prepare(scene, t, &mut row)?;
    t.insert(row.clone()).map_err(err)?;
    t.pending.insert(uid.clone());
    attach_holes(scene, t, &child.uid, &uid)?;
    Ok(row)
}

pub(in crate::live) fn attach_holes(
    scene: &Obj<'_>,
    t: &mut LiveTable,
    source: &str,
    repeat: &str,
) -> PyResult<()> {
    for uid in t.owned.get(source).cloned().unwrap_or_default() {
        let mut row = (*t.rows[&uid]).clone();
        if let Some(cut::Cutter::Hole(h)) = &mut row.cutter {
            if h.repeat.as_deref().is_some_and(|r| r != repeat) {
                return Err(err("Bolt holes already follow a Repeat"));
            }
            h.repeat = Some(repeat.to_owned());
            cut::attach(scene, t, &mut row)?;
            t.insert(row).map_err(err)?;
            t.pending.insert(uid);
        }
    }
    Ok(())
}

/// A released Repeat leaves the drilled stations on each still-live ply.
pub(in crate::live) fn detach_holes(
    scene: &Obj<'_>,
    t: &mut LiveTable,
    row: &Row,
    bake: bool,
) -> PyResult<()> {
    let users: Vec<_> = t.users(&Key::Row(row.uid.clone())).cloned().collect();
    for uid in users {
        let mut cut = (*t.rows[&uid]).clone();
        let Some(cut::Cutter::Hole(mut hole)) = cut.cutter.take() else {
            continue;
        };
        if hole.repeat.as_deref() != Some(&row.uid) {
            continue;
        }
        if bake {
            let layout = repeat::layout(scene, t, row)?.map_err(err)?;
            let target = reference::frame(t.groups[&cut.owner]).inverse()
                * reference::frame(t.groups[&row.owner]);
            let pose = hole
                .pose(scene, t, &cut)?
                .ok_or_else(|| err("Missing hole frame"))?;
            let into_hole = pose.inverse() * target;
            hole.stations = layout
                .poses
                .iter()
                .map(|(i, _, copy)| {
                    (
                        format!("{uid}/{i}"),
                        (into_hole * *copy * layout.base.inverse() * into_hole.inverse())
                            .to_cols_array(),
                    )
                })
                .collect();
        }
        hole.repeat = None;
        cut.cutter = Some(cut::Cutter::Hole(hole));
        cut.inputs
            .retain(|r| !matches!(r,Reference::Row{row:r} if r==&row.uid));
        t.insert(cut).map_err(err)?;
        t.pending.insert(uid);
    }
    Ok(())
}
