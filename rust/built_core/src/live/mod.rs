// SPDX-License-Identifier: GPL-3.0-or-later
//! The root's journalled attribute holds a frozen, structurally shared table.
//! Definition replacements and placement markers are ordinary document ops.
mod cut;
pub(crate) mod cut_display;
pub(crate) mod cut_fit;
pub(crate) use cut::{assign_cut_roles, hosted, migrate_wall, pushed_row, solid_axis};
pub(crate) use cut_display::live_cut_display;
mod drawn_profile;
mod expr;
mod file;
mod geometry;
mod hole;
mod joints;
pub(crate) mod junction;
pub(crate) use junction::hosting::{opening_conflict,wall_faces_changed};
mod meetings;
mod pack;
mod presets;
pub(crate) mod profile;
mod reference;
mod repeat;
pub mod sketch;
mod sketch_scene;
mod sweep;
pub(crate) use file::{load, save, FileLive};
mod table;
use crate::{
    doc::{self, Data, Field, Op},
    element_ids as eid,
    store::{keys::Kind, py::PyMeshStore},
};
use glam::{DMat4, DVec3};
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyModule},
};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, OnceLock},
};
use table::{Key, LiveTable, Reference, Row};
type Obj<'p> = Bound<'p, PyAny>;
const EXT: &str = "built.live";
const TABLE: &str = "_live_table";
fn table_attr() -> u16 {
    static ID: OnceLock<u16> = OnceLock::new();
    *ID.get_or_init(|| doc::attr_id(TABLE).expect("registered live table attribute"))
}
fn current(py: Python<'_>, root: doc::Key) -> Option<LiveTable> {
    let value = {
        let d = doc::lock();
        let doc::value::Value::Opaque(obj) = d.node(root)?.attr(table_attr())? else {
            return None;
        };
        obj.0.clone_ref(py)
    };
    Some(value.bind(py).cast::<LiveTable>().ok()?.get().clone())
}
fn put(scene: &Obj<'_>, table: LiveTable) -> PyResult<()> {
    let py = scene.py();
    let value = if table.rows.is_empty() && table.presets.is_empty() && table.prefs.is_empty() {
        py.None()
    } else {
        Py::new(py, table)?.into_any()
    };
    scene
        .getattr("_root")?
        .call_method1("_set_attr", (TABLE, value))?;
    Ok(())
}
fn root(scene: &Obj<'_>) -> PyResult<doc::Key> {
    scene.getattr("_root")?.getattr("_doc_key")?.extract()
}
fn index(root: doc::Key) -> HashMap<String, doc::Key> {
    let d = doc::lock();
    let mut result = HashMap::new();
    let mut ambiguous = HashSet::new();
    let mut todo = vec![root];
    while let Some(k) = todo.pop() {
        let Some(node) = d.node(k) else { continue };
        if let Data::Placement(p) = &node.data {
            if let Some(uid) = &p.uid {
                if result.insert(uid.clone(), k).is_some() {
                    ambiguous.insert(uid.clone());
                }
            }
        }
        todo.extend(node.children.iter().copied());
    }
    for uid in ambiguous {
        result.remove(&uid);
    }
    result
}
fn group<'p>(py: Python<'p>, table: &LiveTable, uid: &str) -> Option<Obj<'p>> {
    doc::lock()
        .handle_ref(py, *table.groups.get(uid)?)
        .map(|g| g.into_bound(py))
}
fn loose_mesh<'p>(scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    match scene
        .getattr_opt("_loose_mesh")?
        .filter(|mesh| !mesh.is_none())
    {
        Some(mesh) => Ok(mesh),
        None => scene.getattr("mesh"),
    }
}
fn metadata<'p>(g: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let ext = g.getattr("ext")?;
    if ext.is_none() {
        Ok(PyDict::new(g.py()))
    } else {
        ext.cast::<PyDict>()?.copy()
    }
}
fn evaluate(
    scene: &Obj<'_>,
    t: &LiveTable,
    row: &Row,
    active: &mut Option<String>,
    skipped: &mut Vec<(String, &'static str)>,
    stamps: &mut repeat::MeshStamps,
) -> PyResult<&'static str> {
    let py = scene.py();
    if row.kind == "repeat" {
        return repeat::evaluate(scene, t, row, stamps);
    }
    if row.kind == "cut" {
        return Ok("ok");
    }
    if row.kind == "sketch" {
        // Open ends and crossings first (the Sketch workspace shows them);
        // then what a sweep needs from the drawing (one region).
        if let Some(state) = sketch::evaluate(row) {
            return Ok(state);
        }
        return Ok(drawn_profile::outline(&serde_json::json!(row), 24)
            .err()
            .unwrap_or("ok"));
    }
    let Some(g) = group(py, t, &row.owner) else {
        return Ok("ref_lost");
    };
    if !matches!(
        row.kind.as_str(),
        "test_box" | "sweep" | "sketch" | "cut" | "cut_stack"
    ) || row.v != 1
    {
        return Ok("unsupported");
    }
    let mut origin = DVec3::ZERO;
    for r in &row.inputs {
        let Some(point) = reference::origin(scene, t, r, row)? else {
            return Ok("ref_lost");
        };
        origin += point;
    }
    let mesh = if row.kind == "cut_stack" {
        let Some(source) = cut::source(scene, row.source.as_deref().unwrap_or(""))? else {
            return Ok("ref_lost");
        };
        source.getattr("mesh")?
    } else {
        py.import("built_core.mesh")?.call_method0("Mesh")?
    };
    let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
    let mut resolved_path = None;
    let mut span_geometry = None;
    {
        let mut s = store.try_borrow_mut()?;
        let result = if row.kind == "cut_stack" {
            Ok(())
        } else if row.kind == "sweep" {
            let Some(spec) = &row.sweep else {
                return Ok("invalid_profile");
            };
            let Some(points) = reference::sweep_path(scene, t, row, &spec.path)? else {
                return Ok("end_unresolved");
            };
            let mut spec = (**spec).clone();
            if let Some(uid) = &spec.profile.sketch {
                let Some(source) = t.rows.get(uid).filter(|r| r.kind == "sketch") else {
                    return Ok("profile_missing");
                };
                if source.state != "ok" {
                    return Ok("unchanged");
                }
                spec.profile.drawing = Some(serde_json::json!(source));
            }
            let planes = match cut::end_planes(scene, t, row, &points, active)? {
                Ok(planes) => planes,
                Err(state) => return Ok(state),
            };
            let result = sweep::generate(&mut s.inner, &spec, &points, &planes)
                .map(|geometry| span_geometry = Some(geometry));
            resolved_path = Some(points);
            result
        } else {
            geometry::generate(&mut s.inner, row, origin.to_array())
        };
        if let Err(state) = result {
            return Ok(state);
        }
    }
    let body_cut = row.cuts.iter().any(|id| {
        t.rows.get(id).is_some_and(|r| {
            r.on != Some(false)
                && ((row.kind == "cut_stack" && matches!(r.cutter, Some(cut::Cutter::Plane(_) | cut::Cutter::Bisector(_))))
                    || matches!(
                        r.cutter,
                        Some(
                            cut::Cutter::Body(_)
                                | cut::Cutter::Repeat(_)
                                | cut::Cutter::Section(_)
                                | cut::Cutter::Hole(_)
                                | cut::Cutter::Opening(_)
                                | cut::Cutter::Block(_)
                        )
                    ))
        })
    });
    let mesh = if body_cut {
        match cut::bodies(scene, t, row, mesh, active, skipped)? {
            Ok(mesh) => mesh,
            Err(state) => return Ok(state),
        }
    } else {
        mesh
    };
    let result_store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
    let stamp = geometry::stamp(&result_store.try_borrow()?.inner);
    if row.junction.is_some() {
        stamps.insert(
            mesh.as_ptr() as usize,
            (mesh.clone().unbind(), stamp.clone()),
        );
    }
    let marker = PyDict::new(py);
    marker.set_item("row", &row.uid)?;
    if body_cut {
        marker.set_item("body_cut", true)?;
    }
    if let Some(points) = resolved_path {
        marker.set_item("path", points)?;
    }
    if let Some(geometry) = span_geometry {
        marker.set_item("zones", geometry.zones)?;
        if !geometry.butt_planes.is_empty() {
            marker.set_item("butt_planes", geometry.butt_planes)?;
        }
    }
    marker.set_item("key", &stamp)?;
    marker.set_item("stamp", &stamp)?;
    let ext = metadata(&g)?;
    ext.set_item(EXT, marker)?;
    if row.kind != "cut_stack" || body_cut {
        eid::set_definition_uid(
            &mesh,
            &eid::derived_uid(&format!("built:live:v1:{}", row.uid)),
        )?;
    }
    g.setattr("mesh", mesh)?;
    g.setattr("ext", ext)?;
    Ok("ok")
}
fn run(scene: &Obj<'_>, t: &mut LiveTable, seeds: HashSet<String>) -> PyResult<()> {
    static TIMING: OnceLock<bool> = OnceLock::new();
    let timer = (!t.junctions.is_empty()
        && *TIMING.get_or_init(|| std::env::var_os("BUILT_PERF").is_some()))
    .then(std::time::Instant::now);
    let plan = t
        .plan(seeds.iter().cloned())
        .map_err(PyValueError::new_err)?;
    let mut dirty: HashSet<_> = seeds
        .iter()
        .filter(|s| t.rows.contains_key(*s))
        .flat_map(|id| [t.entry(id), Key::Row(id.clone())])
        .collect();
    let mut checked = HashSet::new();
    let mut stamps = repeat::MeshStamps::new();
    for node in plan {
        if !dirty.contains(&node) {
            continue;
        }
        let uid = match &node {
            Key::Stock(id) => {
                if matches!(t.rows[id].state.as_str(), "stale" | "cycle") {
                    continue;
                }
                let downstream = t.next_nodes(&node);
                if t.rows[id].kind != "repeat" && downstream.len() == 1 {
                    t.stock.remove(id);
                    dirty.extend(downstream);
                    continue;
                }
                let stock = junction::stock(scene, t, &t.rows[id])?;
                if t.stock.get(id).is_none_or(|old| **old != stock) {
                    t.stock.insert(id.clone(), Arc::new(stock));
                    dirty.extend(downstream);
                }
                continue;
            }
            Key::Row(id) => id.clone(),
            _ => unreachable!(),
        };
        if matches!(t.rows[&uid].state.as_str(), "cycle" | "stale") {
            continue;
        }
        if t.rows[&uid]
            .junction
            .as_deref()
            .and_then(|p| t.rows.get(p))
            .is_some_and(|p| p.state == "lost")
        {
            continue;
        }
        if let Some(owner) = t.rows[&uid].junction.as_ref().or(t.rows[&uid].joint.parent.as_ref()) {
            checked.insert(owner.clone());
        }
        reference::bind_frames(t, &uid);
        if t.rows[&uid].kind == "junction" {
            if junction::evaluate(scene, t, &uid)? {
                dirty.extend(t.next_nodes(&node));
            }
            checked.insert(uid);
            continue;
        }
        if t.rows[&uid].kind == "repeat" && !t.stock.contains_key(&uid) {
            t.stock.insert(
                uid.clone(),
                Arc::new(junction::stock(scene, t, &t.rows[&uid])?),
            );
        }
        let downstream = t.next_nodes(&node);
        let marker_stamp = |t: &LiveTable, id: &str| -> Option<String> {
            let r = t.rows.get(id)?;
            let g = group(scene.py(), t, &r.owner)?;
            metadata(&g)
                .ok()?
                .get_item(EXT)
                .ok()??
                .get_item("stamp")
                .ok()?
                .extract()
                .ok()
        };
        let old_stamp = if !downstream.is_empty()
            && matches!(t.rows[&uid].kind.as_str(), "sweep" | "repeat" | "test_box")
        {
            marker_stamp(t, &uid)
        } else {
            None
        };
        let old_state = t.rows[&uid].state.clone();
        let row = &t.rows[&uid];
        let mut active = None;
        let mut skipped = Vec::new();
        let state = evaluate(scene, t, row, &mut active, &mut skipped, &mut stamps)?;
        let repeat = row.kind == "repeat";
        let cut_ids = row.cuts.clone();
        let initial = row.state.is_empty();
        for id in &cut_ids {
            if let Some(parent) = t.rows.get(id).and_then(|r| r.junction.as_ref()) {
                checked.insert(parent.clone());
            }
        }
        if repeat && state == "ok" {
            t.groups = Arc::new(index(root(scene)?));
        }
        if state == "ok" {
            for id in cut_ids {
                if t.rows.contains_key(&id) {
                    t.set_state(&id, "ok");
                }
            }
        }
        for (id, state) in skipped {
            t.set_state(&id, state);
        }
        if let Some(id) = active.filter(|_| state != "ok") {
            t.set_state(&id, state);
        }
        if state != sketch::UNCHANGED {
            t.set_state(&uid, state);
        } else if initial {
            // A user of an open sketch with nothing good to keep yet.
            t.set_state(&uid, "profile_open");
        }
        if old_stamp.is_none()
            || old_state != t.rows[&uid].state
            || old_stamp != marker_stamp(t, &uid)
        {
            dirty.extend(downstream);
        }
    }
    // Check leaves before their owning assemblies. A part change affects every
    // enclosing assembly stamp even when the detail's frames did not move.
    let mut owners = Vec::new();
    let mut seen = HashSet::new();
    for id in checked {
        let mut chain = Vec::new();
        let mut next = Some(id);
        while let Some(id) = next.filter(|id| seen.insert(id.clone())) {
            next = t.rows.get(&id).and_then(|r| r.joint.parent.as_ref().or(r.junction.as_ref())).cloned();
            chain.push(id);
        }
        owners.extend(chain.into_iter().rev());
    }
    for id in owners.into_iter().rev() {
        let Some(row) = t.rows.get(&id) else { continue };
        if let Some(g) = group(scene.py(), t, &row.owner) {
            let stamp = repeat::stamp_shared(&g, &mut stamps)?;
            let ext = metadata(&g)?;
            if !ext
                .get_item(EXT)?
                .and_then(|m| m.get_item("stamp").ok())
                .is_some_and(|v| v.eq(&stamp).unwrap_or(false))
            {
                ext.set_item(
                    EXT,
                    to_py(scene.py(), &serde_json::json!({"row":id,"stamp":stamp}))?,
                )?;
                g.setattr("ext", ext)?;
            }
        }
        if row.state == "ok" || (row.state == "failed" && row.joint.reason.get("child").is_some()) {
            let failure = t
                .owned
                .get(&id)
                .into_iter()
                .flatten()
                .filter_map(|c| t.rows.get(c))
                .find(|r| r.state != "ok")
                .map(|r| serde_json::json!({"child":r.uid,"state":r.state}));
            let row = Arc::make_mut(t.rows.get_mut(&id).unwrap());
            row.state = if failure.is_some() { "failed" } else { "ok" }.into();
            row.joint.reason = failure.unwrap_or_default();
        }
    }
    if let Some(start) = timer {
        eprintln!(
            "Junction live evaluation {:.3} ms",
            start.elapsed().as_secs_f64() * 1000.
        );
    }
    Ok(())
}

pub(crate) fn finish(
    scene: &Obj<'_>,
    document: doc::Key,
    command: &Obj<'_>,
    journalled: bool,
) -> PyResult<()> {
    let py = scene.py();
    let Some(mut t) = current(py, document) else {
        return Ok(());
    };
    let mut moved = HashSet::new();
    let mut seeds: HashSet<_> = std::mem::take(&mut t.pending).into_iter().collect();
    let mut structure = false;
    let mut records = HashSet::new();
    let ifc = doc::attr_id("ifc").map_err(PyValueError::new_err)?;
    {
        let d = doc::lock();
        let mut add = |k| {
            if let Some(Data::Placement(p)) = d.node(k).map(|n| &n.data) {
                if let Some(uid) = &p.uid {
                    seeds.extend(t.users(&Key::Placement(uid.clone())).cloned());
                }
            }
        };
        for op in d.open.iter().flat_map(|o| o.ops.iter()) {
            match op {
                Op::Set {
                    k,
                    f: Field::Definition,
                    ..
                } => add(*k),
                Op::SetMany {
                    keys,
                    f: Field::Definition,
                    ..
                } => keys.iter().copied().for_each(&mut add),
                Op::Set {
                    k,
                    f: Field::Xform,
                    old,
                    new,
                } if old != new => {
                    moved.insert(*k);
                }
                Op::SetMany {
                    keys,
                    f: Field::Xform,
                    ..
                } => moved.extend(keys.iter().copied()),
                Op::Ins {
                    k,
                    list: doc::ListId::Children,
                    ..
                }
                | Op::Rem {
                    k,
                    list: doc::ListId::Children,
                    ..
                } => {
                    add(*k);
                    structure = true;
                }
                Op::Rep {
                    list: doc::ListId::Children,
                    old,
                    new,
                    ..
                } => {
                    old.iter().chain(new).copied().for_each(&mut add);
                    structure = true;
                }
                Op::Set {
                    k,
                    f: Field::Attr(a),
                    ..
                }
                | Op::Patch { k, a, .. }
                    if *a == ifc =>
                {
                    records.insert(*k);
                }
                Op::SetMany {
                    keys,
                    f: Field::Attr(a),
                    ..
                } if *a == ifc => records.extend(keys.iter().copied()),
                _ => (),
            }
        }
        for key in &moved {
            seeds.extend(t.frames.get(key).into_iter().flatten().cloned());
        }
    }
    if !moved.is_empty() && !t.junctions.is_empty() {
        // The complete transform transaction is visible here, including a
        // selection containing both the host and its fill in either order.
        if junction::hosting::sync_poses(scene, &t, &moved)? {
            t = current(py, document).unwrap_or(t);
            seeds.extend(std::mem::take(&mut t.pending));
        }
    }
    if structure {
        let next = index(document);
        // A copied placement inherits its marker and baked definition. Give
        // its recipe a fresh identity before another edit can write either.
        let added: Vec<_> = next
            .iter()
            .filter(|(uid, k)| t.groups.get(*uid) != Some(*k))
            .map(|(uid, _)| uid.clone())
            .collect();
        let removed: Vec<_> = t
            .groups
            .keys()
            .filter(|uid| !next.contains_key(*uid))
            .cloned()
            .collect();
        if !t.junctions.is_empty() {
            junction::forget_removed_parts(py, &mut t, &removed, &mut seeds)?;
        }
        t.groups = Arc::new(next);
        // Copies made in one step that copied a member together with the
        // member it is anchored to (a frame copied as column and beam): each
        // copy follows its own copied partner, not the original. Partners are
        // paired by the same move (the copy's frame relative to its source).
        let mut copied: HashMap<String, Vec<(String, DMat4)>> = HashMap::new();
        for uid in &added {
            let g = group(py, &t, uid).expect("indexed placement");
            if let Some(source) = metadata(&g)?
                .get_item(EXT)?
                .and_then(|m| m.get_item("row").ok())
                .and_then(|id| id.extract::<String>().ok())
                .and_then(|id| t.rows.get(&id).map(|r| r.owner.clone()))
                .filter(|owner| owner != uid && t.groups.contains_key(owner))
            {
                let delta = reference::frame(t.groups[uid]) * reference::frame(t.groups[&source]).inverse();
                copied.entry(source).or_default().push((uid.clone(), delta));
            }
        }
        for uid in added {
            let g = group(py, &t, &uid).expect("indexed placement");
            let ext = metadata(&g)?;
            let marker = ext.get_item(EXT)?;
            if let Some(source) = marker
                .and_then(|m| m.get_item("row").ok())
                .and_then(|id| id.extract::<String>().ok())
                .and_then(|id| t.rows.get(&id).cloned())
            {
                if source.owner != uid && !t.owners.contains_key(&uid) {
                    let mut copy = (*source).clone();
                    copy.uid = eid::new_uid();
                    copy.owner = uid.clone();
                    let delta = reference::frame(t.groups[&uid]) * reference::frame(t.groups[&source.owner]).inverse();
                    let partners: HashMap<String, String> = copied
                        .iter()
                        .filter(|(from, _)| **from != source.owner)
                        .filter_map(|(from, copies)| {
                            copies.iter().find(|(_, d)| d.abs_diff_eq(delta, 1e-9)).map(|(to, _)| (from.clone(), to.clone()))
                        })
                        .collect();
                    remap_copy(&mut copy, &partners)?;
                    for cut_id in &mut copy.cuts {
                        let mut cut = (*t
                            .rows
                            .get(cut_id)
                            .ok_or_else(|| PyValueError::new_err("Missing cut"))?)
                        .as_ref()
                        .clone();
                        cut.uid = eid::new_uid();
                        cut.owner = uid.clone();
                        remap_copy(&mut cut, &partners)?;
                        cut.junction = None;
                        cut.key = None;
                        *cut_id = cut.uid.clone();
                        seeds.insert(cut.uid.clone());
                        t.insert(cut).map_err(PyValueError::new_err)?;
                    }
                    let marker = ext.get_item(EXT)?.unwrap().cast_into::<PyDict>()?.copy()?;
                    marker.set_item("row", &copy.uid)?;
                    ext.set_item(EXT, marker)?;
                    g.setattr("ext", ext)?;
                    t.insert(copy).map_err(PyValueError::new_err)?;
                }
            }
            seeds.extend(t.users(&Key::Placement(uid)).cloned());
        }
        for uid in removed {
            seeds.extend(t.users(&Key::Placement(uid.clone())).cloned());
            if let Some(id) = t.owners.get(&uid).cloned() {
                if let Some(row) = t.remove(&id) {
                    if let Some(parent) = row.joint.parent.as_ref().or(row.junction.as_ref()) {
                        seeds.insert(parent.clone());
                    }
                    junction::forget_content(&mut t, &row);
                    if row.kind == "repeat" {
                        junction::detach_holes(scene, &mut t, &row, false)?;
                    }
                    if row.kind == "junction" {
                        junction::remove_descendants(&mut t, &id)?;
                        seeds.extend(std::mem::take(&mut t.pending));
                    }
                    for cut in &row.cuts {
                        t.remove(cut);
                    }
                }
                seeds.extend(t.users(&Key::Stock(id.clone())).cloned());
                seeds.extend(t.users(&Key::Row(id)).cloned());
            }
        }
    }
    if structure {
        for uid in t.rows.keys().cloned().collect::<Vec<_>>() {
            if reference::bind_frames(&mut t, &uid) {
                seeds.insert(uid);
            }
        }
    }
    for mesh in doc::geometry::touched_meshes(py) {
        let root = scene.getattr("_root")?;
        if mesh.bind(py).is(&loose_mesh(scene)?) {
            seeds.extend(t.users(&Key::Root).cloned());
        }
        for uid in root
            .call_method1("_definition_uids", (mesh,))?
            .extract::<Vec<String>>()?
        {
            seeds.extend(t.users(&Key::Placement(uid)).cloned());
        }
    }
    let mut detached = false;
    for k in records {
        let owner = doc::lock().node(k).and_then(|node| match &node.data {
            Data::Placement(p) => p.uid.clone(),
            _ => None,
        });
        if let Some(owner) = &owner {
            seeds.extend(
                t.users(&Key::Placement(owner.clone()))
                    .filter(|id| t.junctions.contains(*id))
                    .cloned(),
            );
            if let Some(id) = t.owners.get(owner) {
                seeds.extend(
                    t.users(&Key::Stock(id.clone()))
                        .filter(|id| t.junctions.contains(*id))
                        .cloned(),
                );
            }
        }
        let Some(uid) = owner.as_ref().and_then(|id| t.owners.get(id)).cloned() else {
            continue;
        };
        let row = &t.rows[&uid];
        let Some(p) = presets::follower(row)
            .and_then(|id| t.presets.get(id))
        else {
            continue;
        };
        let Some(g) = group(py, &t, &row.owner) else {
            continue;
        };
        let record = g.getattr("ifc")?;
        for (key, expected) in &p.record {
            if p.locks.get(key).is_none_or(|v| v != "fixed") {
                continue;
            }
            if !record
                .get_item(key)
                .is_ok_and(|v| to_py(py, expected).and_then(|e| v.eq(e)).unwrap_or(false))
            {
                let note = format!("Detached from {}: {key}", p.name);
                let row = Arc::make_mut(t.rows.get_mut(&uid).unwrap());
                if let Some(sweep) = &mut row.sweep {
                    sweep.preset = None;
                    sweep.preset_uid = None;
                    sweep.preset_rev = 0;
                }
                row.repeat.repeat_preset = None;
                let record = if record.is_none() {
                    PyDict::new(py)
                } else {
                    record.cast::<PyDict>()?.copy()?
                };
                record.set_item("preset_note", note)?;
                g.setattr("ifc", record)?;
                detached = true;
                break;
            }
        }
    }
    if seeds.is_empty() && !structure && !detached {
        return Ok(());
    }
    if journalled {
        run(scene, &mut t, seeds)?;
        along_sync(scene, &mut t)?;
        if let Some(uid) = command.getattr_opt("_repeat_selection_row")? {
            if let Some(row) = t.rows.get(&uid.extract::<String>()?) {
                if let Some(owner) = group(py, &t, &row.owner) {
                    let before = pyo3::types::PyList::new(
                        py,
                        scene
                            .getattr("selection")?
                            .try_iter()?
                            .collect::<PyResult<Vec<_>>>()?,
                    )?;
                    let copies = owner.getattr("children")?;
                    command.setattr("_repeat_selection_before", before)?;
                    command.setattr("_repeat_selection_after", &copies)?;
                    command.setattr("_repeat_edit_context", scene.getattr("edit_group")?)?;
                    scene.call_method1("select", (copies,))?;
                }
            }
        }
        return put(scene, t);
    }
    // Opaque commands own their inverse. Only the evaluator's writes must
    // accompany it on replay. A savepoint prevents coalescing with the input.
    let api = py.import("built_core.doc_model")?;
    api.call_method1("begin", (document,))?;
    let result = run(scene, &mut t, seeds)
        .and_then(|()| along_sync(scene, &mut t))
        .and_then(|()| put(scene, t));
    if let Err(error) = result {
        api.call_method0("rollback")?;
        return Err(error);
    }
    let (ops, pins) = {
        let d = doc::lock();
        let open = d.open.as_ref().expect("live savepoint");
        (
            open.ops[*open.marks.last().unwrap()..].to_vec(),
            open.pins.iter().map(|p| p.clone_ref(py)).collect(),
        )
    };
    api.call_method0("commit")?;
    command.setattr(
        "_live_txn",
        Py::new(py, doc::py::PyTransaction::from_parts(ops, pins))?,
    )
}

/// A Repeat's copies changed: junctions applied along it follow in the same
/// step (docs/junctions.md §2.8).
fn along_sync(scene: &Obj<'_>, t: &mut LiveTable) -> PyResult<()> {
    if t.junctions.iter().any(|id| t.rows.get(id).is_some_and(|r| !r.joint.along.is_null()))
        && junction::along::sync(scene, t)?
    {
        let more: HashSet<String> = std::mem::take(&mut t.pending).into_iter().collect();
        run(scene, t, more)?;
    }
    Ok(())
}
#[derive(Clone)]
enum Edit {
    Presets(Vec<presets::Preset>),
    DeletePreset(String),
    Create(Vec<sweep::Sweep>, bool),
    /// Rows to set, after embedding the presets they need that the
    /// model does not hold yet (a Library detail applied in one step).
    Set(Vec<Row>, Vec<presets::Preset>),
    /// Done in the Sketch workspace: the sketch row, and the sweeps whose
    /// profile becomes it (a catalogue size edited as a drawing).
    Commit(Row, Vec<String>),
    Release(String),
    ReleaseTree(String),
    Reevaluate(String),
    /// Skip (or un-skip) meetings for a detail.
    Skip(String, Vec<String>, bool),
    /// Follow a detail: new matching meetings are listed, never applied.
    Follow(String, bool),
    /// Capture a meeting's items as a detail and its first use (§5).
    Capture(serde_json::Value, serde_json::Value),
    /// Adopt rows or parts into a junction: this use only, or its detail.
    Adopt(String, Vec<String>, bool),
    /// A use's overrides pushed into its detail.
    Upgrade(String),
}
#[pyclass(frozen, dict, name = "LiveCommand")]
struct Mutation(Edit);
#[pymethods]
#[allow(non_upper_case_globals)]
impl Mutation {
    #[classattr]
    const doc_undo: bool = true;
    #[classattr]
    const keeps_rows: bool = true;
    #[classattr]
    const history_name: &'static str = "Live recipe";
    fn bind_transaction(slf: &Bound<'_, Self>, txn: &Obj<'_>) -> PyResult<()> {
        slf.as_any().setattr("_doc_txn", txn)
    }
    fn doc_after(slf: &Bound<'_, Self>, scene: &Obj<'_>, phase: &str) -> PyResult<()> {
        let cmd = slf.as_any();
        if cmd
            .getattr_opt("_repeat_edit_context")?
            .is_some_and(|c| scene.getattr("edit_group").is_ok_and(|open| c.is(&open)))
        {
            let key = if phase == "undo" {
                "_repeat_selection_before"
            } else {
                "_repeat_selection_after"
            };
            if let Some(items) = cmd.getattr_opt(key)? {
                scene.call_method1("select", (items,))?;
            }
        }
        scene.setattr("version", scene.getattr("version")?.add(1)?)
    }
    #[pyo3(name = "do")]
    fn apply(&self, scene: &Obj<'_>) -> PyResult<()> {
        let py = scene.py();
        match self.apply_edit(scene) {
            Err(e) if e.is_instance_of::<PyValueError>(py) => {
                let rejection = py
                    .import("built_core.history")?
                    .getattr("CommandRejected")?
                    .call1((e.value(py).str()?,))?;
                Err(PyErr::from_value(rejection))
            }
            result => result,
        }
    }
}

fn bind_drawing(t: &mut LiveTable, profile: &mut profile::Profile) -> PyResult<()> {
    if let Some(mut drawing) = profile.drawing.take() {
        if profile.sketch.is_none() {
            let uid = eid::new_uid();
            drawing["uid"] = serde_json::json!(uid);
            drawing["kind"] = serde_json::json!("sketch");
            let source = serde_json::from_value(drawing)
                .map_err(|e| PyValueError::new_err(e.to_string()))?;
            t.insert(source).map_err(PyValueError::new_err)?;
            t.pending.insert(uid.clone());
            profile.sketch = Some(uid);
        }
    }
    Ok(())
}
/// Point a copied row's references at the copies of the placements they
/// named (`partners`: original uid → its copy made in the same step).
fn remap_copy(row: &mut Row, partners: &HashMap<String, String>) -> PyResult<()> {
    if partners.is_empty() {
        return Ok(());
    }
    fn walk(v: &mut serde_json::Value, partners: &HashMap<String, String>) {
        match v {
            serde_json::Value::Object(m) => {
                for (k, x) in m.iter_mut() {
                    if let (true, serde_json::Value::Array(path)) = (k == "path" || k == "rests_on", &mut *x) {
                        for e in path.iter_mut() {
                            if let Some(to) = e.as_str().and_then(|s| partners.get(s)) {
                                *e = serde_json::json!(to);
                            }
                        }
                    } else if let (true, Some(to)) = (k == "rests_on", x.as_str().and_then(|s| partners.get(s))) {
                        *x = serde_json::json!(to);
                    } else {
                        walk(x, partners);
                    }
                }
            }
            serde_json::Value::Array(a) => a.iter_mut().for_each(|x| walk(x, partners)),
            _ => {}
        }
    }
    let mut v = serde_json::to_value(&*row).map_err(|e| PyValueError::new_err(e.to_string()))?;
    walk(&mut v, partners);
    *row = serde_json::from_value(v).map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(())
}
impl Mutation {
    fn apply_edit(&self, scene: &Obj<'_>) -> PyResult<()> {
        let py = scene.py();
        let document = root(scene)?;
        if scene
            .getattr_opt("doc_ext")?
            .filter(|ext| !ext.is_none())
            .is_some_and(|ext| ext.contains(EXT).unwrap_or(false))
        {
            return Err(PyValueError::new_err("unsupported live extension version"));
        }
        let mut t = current(py, document).unwrap_or_else(|| LiveTable {
            groups: Arc::new(index(document)),
            ..Default::default()
        });
        if scene
            .getattr_opt("doc_ext")?
            .filter(|ext| !ext.is_none())
            .is_some_and(|ext| ext.contains("built.presets").unwrap_or(false))
            && matches!(
                self.0,
                Edit::Presets(_) | Edit::DeletePreset(_) | Edit::Create(..)
            )
        {
            return Err(PyValueError::new_err("Unrecognised preset extension"));
        }
        match &self.0 {
            Edit::Presets(rows) => {
                presets::update(scene, &mut t, rows)?;
                // The view offers a review of similar meetings after the step
                // (never inside it: the search is not part of the History).
                let saved: Vec<(String, String)> = rows
                    .iter()
                    .filter(|p| p.primitive == "junction")
                    .filter(|p| p.values.get("meeting").and_then(|m| m.as_str()) != Some("parts"))
                    .map(|p| (p.uid.clone(), p.name.clone()))
                    .collect();
                if !saved.is_empty() {
                    scene.setattr("_junction_saved", saved)?;
                }
            }
            Edit::DeletePreset(id) => {
                if t.rows.values().any(|r| {
                    presets::follower(r) == Some(id.as_str())
                }) {
                    return Err(PyValueError::new_err("Preset is in use"));
                }
                Arc::make_mut(&mut t.presets)
                    .remove(id)
                    .ok_or_else(|| PyValueError::new_err("Unknown project preset"))?;
            }
            Edit::Create(sweeps, wall_junctions) => {
                for spec in sweeps {
                    let key = &spec.profile.library;
                    // A drawn member is named after its drawing.
                    let drawn = spec
                        .profile
                        .sketch
                        .as_ref()
                        .and_then(|uid| t.rows.get(uid))
                        .and_then(|r| r.sketch_fields.name.as_str().map(str::to_owned))
                        .or_else(|| spec.profile.drawing.as_ref().and_then(|d| d["name"].as_str().map(str::to_owned)))
                        .filter(|n| !n.is_empty());
                    let name = spec.profile.name().map(str::to_owned).or(drawn).unwrap_or_else(|| "Drawn profile".into());
                    let name = name.as_str();
                    let g = py.import("built_core.group")?.getattr("Group")?.call0()?;
                    g.setattr("name", name)?;
                    g.setattr("xform", py.import("built_core")?.call_method0("Matrix4")?)?;
                    let record = PyDict::new(py);
                    record.set_item(
                        "class",
                        if key == "basic/wall" {
                            "IfcWall"
                        } else {
                            "IfcBeam"
                        },
                    )?;
                    record.set_item("name", name)?;
                    g.setattr("ifc", record)?;
                    let parent = scene.getattr("edit_group")?;
                    let mut spec = spec.clone();
                    let mut destination = String::new();
                    if let Some(p) = presets::link(&mut t, &mut spec)? {
                        if spec.cut_definitions.is_empty() {
                            spec.cut_definitions = p
                                .values
                                .get("cut_definitions")
                                .and_then(|cuts| cuts.as_array())
                                .cloned()
                                .unwrap_or_default();
                        }
                        destination = p.node.clone();
                        crate::structure::sweep_default(scene, Some(&p.uid))?;
                        p.record(&g, false)?;
                        let expected = p.apply_values(serde_json::json!(spec), true)?;
                        if let Some(note) = presets::detach(&expected, &mut spec, &p) {
                            g.getattr("ifc")?.set_item("preset_note", note)?;
                        }

                        let name = if p.naming.contains("{n}") {
                            p.naming.replace("{n}", &(t.serial + 1).to_string())
                        } else {
                            p.name.clone()
                        };
                        g.setattr("name", &name)?;
                        g.getattr("ifc")?.set_item("name", name)?;
                    }
                    bind_drawing(&mut t, &mut spec.profile)?;
                    if parent.is_none() {
                        scene.getattr("groups")?.call_method1("append", (&g,))?;
                    } else {
                        let inverse =
                            reference::frame(parent.getattr("_doc_key")?.extract()?).inverse();
                        for point in &mut spec.path.points {
                            *point = inverse.transform_point3((*point).into()).to_array();
                        }
                        for end in spec.path.ends.iter_mut().flatten() {
                            if let sweep::End::Point(p) = end {
                                *p = inverse.transform_point3((*p).into()).to_array();
                            }
                        }
                        parent.getattr("children")?.call_method1("append", (&g,))?;
                    }
                    // The member's origin and axes follow its profile, so its
                    // box, axes and Move/Rotate grips sit on the member.
                    if !spec.path.definition_space && spec.path.chain.is_empty() {
                        if let Some(frame) = sweep::member_frame(&spec.path.points) {
                            let inverse = frame.inverse();
                            for point in &mut spec.path.points {
                                *point = inverse.transform_point3((*point).into()).to_array();
                            }
                            for end in spec.path.ends.iter_mut().flatten() {
                                if let sweep::End::Point(p) = end {
                                    *p = inverse.transform_point3((*p).into()).to_array();
                                }
                            }
                            spec.path.definition_space = true;
                            g.setattr("xform", Py::new(py, crate::math::Matrix4::from(frame))?)?;
                        }
                    }
                    crate::structure::file_sweep(scene, &g, &destination)?;
                    let owner: String = g.getattr("uid")?.extract()?;
                    let uid = eid::new_uid();
                    Arc::make_mut(&mut t.groups)
                        .insert(owner.clone(), g.getattr("_doc_key")?.extract()?);
                    let definitions = std::mem::take(&mut spec.cut_definitions);
                    let target_uid = owner.clone();
                    t.insert(Row {
                        frames: Default::default(),
                        joint: Default::default(),
                        junction: None,
                        key: None,
                        repeat: Default::default(),
                        kind: "sweep".into(),
                        uid: uid.clone(),
                        owner,
                        v: 1,
                        size: [1.; 3],
                        inputs: vec![],
                        slots: Default::default(),
                        open_top: false,
                        state: String::new(),
                        curves: None,
                        sketch_fields: Default::default(),
                        cuts: vec![],
                        source: None,
                        clearance: None,
                        cutter: None,
                        anchor: None,
                        operation: None,
                        end: None,
                        span: None,
                        offset: None,
                        on: None,
                        sweep: Some(Box::new(spec)),
                    })
                    .map_err(PyValueError::new_err)?;
                    for mut definition in definitions {
                        definition["uid"] = serde_json::json!(eid::new_uid());
                        definition["owner"] = serde_json::json!(target_uid);
                        let mut cut: Row = serde_json::from_value(definition)
                            .map_err(|e| PyValueError::new_err(e.to_string()))?;
                        cut::attach(scene, &mut t, &mut cut)?;
                        t.pending.insert(cut.uid.clone());
                        t.insert(cut).map_err(PyValueError::new_err)?;
                    }
                    if *wall_junctions {
                        junction::walls::apply_ends(scene, &mut t, &uid)?;
                    }
                    t.pending.insert(uid);
                }
            }
            Edit::Set(requests, definitions) => {
                presets::embed(&mut t, definitions)?;
                for request in requests {
                    set_row(scene, &mut t, request)?;
                }
            }
            Edit::Commit(row, adopt) => {
                set_row(scene, &mut t, row)?;
                for id in adopt {
                    let mut user = t
                        .rows
                        .get(id)
                        .filter(|r| r.sweep.is_some())
                        .map(|r| (**r).clone())
                        .ok_or_else(|| PyValueError::new_err("not a swept member"))?;
                    let spec = user.sweep.as_mut().unwrap();
                    spec.profile = profile::Profile {
                        catalogue: None,
                        sketch: Some(row.uid.clone()),
                        drawing: None,
                        library: String::new(),
                        family: String::new(),
                        params: Default::default(),
                    };
                    user.state.clear();
                    t.insert(user).map_err(PyValueError::new_err)?;
                    t.pending.insert(id.clone());
                }
            }
            Edit::ReleaseTree(id) => {
                let mut nodes = vec![id.clone()];
                let mut seen = HashSet::new();
                let mut order = Vec::new();
                while let Some(uid) = nodes.pop() {
                    if !seen.insert(uid.clone()) {
                        return Err(PyValueError::new_err("ownership cycle"));
                    }
                    nodes.extend(t.owned.get(&uid).into_iter().flatten().cloned());
                    order.push(uid);
                }
                for uid in order {
                    if current(py, document).is_some_and(|t| t.rows.contains_key(&uid)) {
                        Mutation(Edit::Release(uid)).apply_edit(scene)?;
                    }
                }
                return Ok(());
            }
            Edit::Release(id) => {
                if let Some(repeat) = t.rows.get(id).filter(|r| r.kind == "repeat").cloned() {
                    junction::detach_holes(scene, &mut t, &repeat, true)?;
                    if repeat.source.as_ref().is_some_and(|s| t.owners.get(s).and_then(|u| t.rows.get(u)).is_some_and(|r| r.kind == "junction")) {
                        if let Some(g) = group(py, &t, &repeat.owner) {
                            let record = g.getattr("ifc")?.call_method0("copy")?;
                            record.set_item("junction_source", repeat.source.as_deref())?;
                            g.setattr("ifc", record)?;
                        }
                    }
                }
                if let Some(j) = t.rows.get(id).filter(|r| r.kind == "junction").cloned() {
                    junction::release(scene, &mut t, &j)?;
                }
                let mut row = t
                    .remove(id)
                    .ok_or_else(|| PyValueError::new_err("unknown recipe"))?;
                junction::forget_content(&mut t, &row);
                if row.kind == "cut" {
                    let target_uid = t.owners[&row.owner].clone();
                    let mut target = (*t.rows[&target_uid]).clone();
                    let at = target
                        .cuts
                        .iter()
                        .position(|u| u == id)
                        .ok_or_else(|| PyValueError::new_err("Detached cut"))?;
                    // Bake the prefix through the selected cut. Later operations
                    // continue to use the baked result as their editable source.
                    t.insert((*row).clone()).map_err(PyValueError::new_err)?;
                    let later = target.cuts.split_off(at + 1);
                    let state = evaluate(
                        scene, &t, &target, &mut None, &mut Vec::new(), &mut repeat::MeshStamps::new(),
                    )?;
                    if state != "ok" {
                        return Err(PyValueError::new_err("Could not bake the cut"));
                    }
                    for uid in &target.cuts {
                        t.remove(uid);
                    }
                    if !later.is_empty() {
                        let g = group(py, &t, &target.owner).unwrap();
                        let source = cut::keep_source(scene, &g.getattr("mesh")?)?;
                        target.kind = "cut_stack".into();
                        target.source = Some(source);
                        target.cuts = later;
                        target.sweep = None;
                        target.inputs.clear();
                        target.slots.clear();
                        t.remove(&target_uid);
                        t.insert(target).map_err(PyValueError::new_err)?;
                        t.pending.insert(target_uid);
                        return put(scene, t);
                    }
                    row = t.remove(&target_uid).unwrap();
                }
                if row.kind == "repeat" {
                    if let Some(g) = group(py, &t, &row.owner) {
                        for copy in g.getattr("children")?.try_iter()? {
                            let copy = copy?;
                            let ext = metadata(&copy)?;
                            for key in ["built.repeat", "built.repeat_state"] {
                                if ext.contains(key)? {
                                    ext.del_item(key)?;
                                }
                            }
                            copy.setattr("ext", ext)?;
                        }
                    }
                }
                if let Some(uid) = row.source.as_ref().filter(|_| row.kind == "cut_stack") {
                    if !t.rows.values().any(|r| r.source.as_ref() == Some(uid)) {
                        if let Some(source) = cut::source(scene, uid)? {
                            scene
                                .getattr("kept_components")?
                                .call_method1("remove", (source,))?;
                        }
                    }
                }
                for uid in &row.cuts {
                    t.remove(uid);
                }
                if let Some(g) = group(py, &t, &row.owner) {
                    let ext = metadata(&g)?;
                    if ext.contains(EXT)? {
                        ext.del_item(EXT)?;
                        g.setattr("ext", ext)?;
                    }
                }
                let users: Vec<_> = t.users(&Key::Row(id.clone())).cloned().collect();
                if row.kind == "sketch" {
                    let frozen = freeze_users(&mut t, &row)?;
                    t.pending.extend(frozen);
                }
                t.pending.extend(users);
            }
            Edit::Skip(detail, keys, skip) => {
                if presets::lookup(&t, detail).is_none_or(|p| p.primitive != "junction") {
                    return Err(PyValueError::new_err("Unknown junction detail"));
                }
                if keys.iter().any(|k| k.len() != 16 || !k.bytes().all(|c| c.is_ascii_hexdigit())) {
                    return Err(PyValueError::new_err("expected 16-hex meeting keys"));
                }
                let before = t.prefs.clone();
                Arc::make_mut(&mut t.prefs).skip(detail, keys, *skip);
                if t.prefs == before {
                    return Err(PyValueError::new_err("Nothing to change"));
                }
            }
            Edit::Follow(detail, on) => {
                if presets::lookup(&t, detail).is_none_or(|p| p.primitive != "junction") {
                    return Err(PyValueError::new_err("Unknown junction detail"));
                }
                let prefs = Arc::make_mut(&mut t.prefs);
                let changed = if *on {
                    prefs.follow.insert(detail.clone())
                } else {
                    prefs.follow.remove(detail)
                };
                if !changed {
                    return Err(PyValueError::new_err("Nothing to change"));
                }
            }
            Edit::Capture(seed, options) => {
                let opts: junction::capture::Options = serde_json::from_value(options.clone())
                    .map_err(|e| PyValueError::new_err(e.to_string()))?;
                let draft = junction::capture::draft(scene, &t, seed, &opts)?;
                let uid = junction::capture::convert(scene, &mut t, &draft)?;
                scene.setattr("_junction_saved", vec![(draft.detail.uid.clone(), draft.detail.name.clone())])?;
                scene.setattr("_junction_captured", (uid, draft.detail.uid.clone()))?;
            }
            Edit::Adopt(junction_uid, items, to_detail) => {
                junction::capture::adopt(scene, &mut t, junction_uid, items, *to_detail)?;
            }
            Edit::Upgrade(uid) => {
                let j = t.rows.get(uid).filter(|r| r.kind == "junction").cloned()
                    .ok_or_else(|| PyValueError::new_err("Unknown junction"))?;
                let overrides = j.joint.overrides.as_object().cloned().unwrap_or_default();
                if overrides.is_empty() {
                    return Err(PyValueError::new_err("This use has no overrides"));
                }
                let mut p = junction::detail(&t, &j)?;
                if p.uid.is_empty() {
                    return Err(PyValueError::new_err("A custom junction has no detail"));
                }
                if let Some(defs) = p.values.get_mut("params").and_then(|v| v.as_array_mut()) {
                    for d in defs {
                        if let Some(v) = d["key"].as_str().and_then(|k| overrides.get(k)) {
                            d["expr"] = v.clone();
                        }
                    }
                }
                let mut row = (*j).clone();
                row.joint.overrides = serde_json::Value::Null;
                t.insert(row).map_err(PyValueError::new_err)?;
                presets::update(scene, &mut t, &[p])?;
            }
            Edit::Reevaluate(id) => {
                if !t.rows.contains_key(id) {
                    return Err(PyValueError::new_err("unknown recipe"));
                }
                t.set_state(id, "");
                t.pending.insert(id.clone());
            }
        }
        put(scene, t)?;
        scene.setattr("version", scene.getattr("version")?.add(1)?)
    }
}
/// A released sketch's users each keep its drawing as their own frozen
/// copy: a document-local sketch row per member (a sweep's profile, a
/// section cut's cutter), so none of them loses its profile
/// (`profile_missing`) and none follows another's later edits.
fn freeze_users(t: &mut LiveTable, sketch: &Row) -> PyResult<Vec<String>> {
    let users: Vec<String> = t
        .by_creation()
        .into_iter()
        .filter(|r| {
            r.sweep.as_ref().is_some_and(|s| s.profile.sketch.as_deref() == Some(sketch.uid.as_str()))
                || matches!(&r.cutter, Some(cut::Cutter::Section(uid)) if *uid == sketch.uid)
        })
        .map(|r| r.uid.clone())
        .collect();
    for id in &users {
        let mut user = (*t.rows[id]).clone();
        let mut copy = sketch.clone();
        copy.uid = eid::new_uid();
        copy.sketch_fields.frozen = serde_json::json!(true);
        // A frozen copy is the member's profile, not a picked plane's.
        copy.sketch_fields.plane = serde_json::Value::Null;
        if let Some(spec) = user.sweep.as_mut() {
            spec.profile.sketch = Some(copy.uid.clone());
        } else {
            user.cutter = Some(cut::Cutter::Section(copy.uid.clone()));
        }
        t.insert(copy).map_err(PyValueError::new_err)?;
        t.insert(user).map_err(PyValueError::new_err)?;
    }
    Ok(users)
}
fn set_row(scene: &Obj<'_>, t: &mut LiveTable, request: &Row) -> PyResult<()> {
    let mut row = request.clone();
    let py = scene.py();
    junction::validate_ownership(t, &row)?;
    junction::place_owned_sweep(scene, t, &row)?;
    if row.kind == "junction" {
        let preset = junction::detail(t, &row)?;
        junction::prepare(scene, t, &mut row, &preset)?;
    }
    if row.kind == "repeat" {
        repeat::prepare(scene, t, &mut row)?;
        presets::bind_repeat(scene, t, &mut row)?;
    }
    if row.kind != "sketch" && group(py, t, &row.owner).is_none() {
        return Err(PyValueError::new_err("unknown recipe owner"));
    }
    if t.rows.get(&row.uid).is_some_and(|r| r.state == "stale") {
        return Err(PyValueError::new_err(
            "stale recipe requires re-evaluation or release",
        ));
    }
    if let (Some(old), Some(new)) = (
        t.rows.get(&row.uid).and_then(|r| r.sweep.as_deref()),
        row.sweep.as_deref_mut(),
    ) {
        if let Some(g) = group(py, t, &row.owner) {
            if old.preset_uid != new.preset_uid {
                if let Some(p) = presets::link(t, new)? {
                    p.record(&g, false)?;
                }
            } else if let Some(p) = old
                .preset_uid
                .as_deref()
                .and_then(|id| presets::lookup(t, id))
            {
                if let Some(note) = presets::detach(old, new, &p) {
                    g.getattr("ifc")?.set_item("preset_note", note)?;
                }
            }
        }
    }
    if row.kind == "cut" {
        cut::attach(scene, t, &mut row)?;
    } else if let Some(old) = t.rows.get(&row.uid) {
        row.cuts.clone_from(&old.cuts);
    } else {
        row.cuts.clear();
    }
    if let Some(spec) = row.sweep.as_mut() {
        spec.preset_definition = None;
        bind_drawing(t, &mut spec.profile)?;
    }
    // Editing a member's frozen copy keeps it that member's.
    if row.kind == "sketch" && row.sketch_fields.frozen.is_null() {
        if let Some(old) = t.rows.get(&row.uid) {
            row.sketch_fields.frozen = old.sketch_fields.frozen.clone();
        }
    }
    row.state = t
        .rows
        .get(&row.uid)
        .filter(|r| r.state != "cycle")
        .map(|r| r.state.clone())
        .unwrap_or_default();
    let new_repeat = row.kind == "repeat" && !t.rows.contains_key(&row.uid);
    t.insert(row.clone()).map_err(PyValueError::new_err)?;
    if new_repeat {
        let source = row.source.as_ref().unwrap();
        if let Some(junction) = t.owners.get(source)
            .filter(|id| t.junctions.contains(*id)).cloned()
        {
            junction::attach_holes(scene, t, &junction, &row.uid)?;
        }
        let users: Vec<_> = t.users(&Key::Placement(source.clone())).cloned().collect();
        for uid in users {
            let Some(mut cut) = t.rows.get(&uid).map(|r| (**r).clone()) else {
                continue;
            };
            if matches!(&cut.cutter, Some(cut::Cutter::Body(id)) if id == source) {
                cut.cutter = Some(cut::Cutter::Repeat(row.uid.clone()));
                cut::attach(scene, t, &mut cut)?;
                t.insert(cut).map_err(PyValueError::new_err)?;
            }
        }
    }
    t.schedule([row.uid.clone()])
        .map_err(PyValueError::new_err)?;
    t.pending.insert(row.uid);
    Ok(())
}
fn edit<'p>(history: &Obj<'p>, action: Edit) -> PyResult<Bound<'p, PyDict>> {
    let py = history.py();
    let cmd = Bound::new(py, Mutation(action))?;
    crate::protocol::execute(history, cmd.as_any())
}
fn json<T: serde::de::DeserializeOwned>(v: &Obj<'_>) -> PyResult<T> {
    let raw = v
        .py()
        .import("json")?
        .call_method1("dumps", (v,))?
        .extract::<String>()?;
    serde_json::from_str(&raw).map_err(|e| PyValueError::new_err(e.to_string()))
}
fn to_py<'p>(py: Python<'p>, v: &impl serde::Serialize) -> PyResult<Obj<'p>> {
    let raw = serde_json::to_string(v).map_err(|e| PyValueError::new_err(e.to_string()))?;
    py.import("json")?.call_method1("loads", (raw,))
}
#[pyfunction]
fn live_repeat<'p>(scene: &Obj<'p>, owner: &str) -> PyResult<Obj<'p>> {
    if let Some(t) = current(scene.py(), root(scene)?) {
        let copy_row = group(scene.py(), &t, owner).and_then(|g| {
            g.getattr("ext")
                .ok()?
                .get_item("built.repeat")
                .ok()?
                .get_item("row")
                .ok()?
                .extract::<String>()
                .ok()
        });
        if let Some(row) = t
            .owners
            .get(owner)
            .or(copy_row.as_ref())
            .and_then(|uid| t.rows.get(uid))
            .filter(|r| r.kind == "repeat")
        {
            let owner = &row.owner;
            let result = to_py(scene.py(), row)?;
            let mut breadcrumb = Vec::new();
            let mut next = Some(row.as_ref());
            let mut seen = HashSet::new();
            while let Some(r) = next.filter(|r| seen.insert(r.uid.clone())) {
                if let Some(g) = group(scene.py(), &t, &r.owner) {
                    breadcrumb.push(g.getattr("name")?.extract::<String>()?);
                }
                next = r
                    .source
                    .as_ref()
                    .and_then(|s| t.owners.get(s))
                    .and_then(|id| t.rows.get(id))
                    .filter(|r| r.kind == "repeat")
                    .map(Arc::as_ref);
            }
            breadcrumb.reverse();
            result.set_item("breadcrumb", breadcrumb)?;
            let mut hosts = Vec::new();
            let blocked: HashSet<_> = t
                .schedule([row.uid.clone()])
                .unwrap_or_default()
                .into_iter()
                .collect();
            for id in t.repeats.iter().filter(|id| !blocked.contains(*id)) {
                if let Some(g) = group(scene.py(), &t, &t.rows[id].owner) {
                    hosts.push((id.clone(), g.getattr("name")?.extract::<String>()?));
                }
            }
            result.set_item("between_options", hosts)?;
            if let Some(source) = row.source.as_ref() {
                if let Some(g) = group(scene.py(), &t, source) {
                    result.set_item("source_path", scene.call_method1("occurrence_path", (g,))?)?;
                }
                result.set_item(
                    "can_adapt",
                    t.owners
                        .get(source)
                        .and_then(|id| t.rows.get(id))
                        .is_some_and(|r| r.kind == "sweep"),
                )?;
            }
            if let Some(g) = group(scene.py(), &t, owner) {
                if let Some(marker) = metadata(&g)?.get_item(EXT)? {
                    result.set_item(
                        "owner_frame",
                        reference::frame(t.groups[owner]).to_cols_array(),
                    )?;
                    if let (Some(source), Ok(last)) = (
                        row.source
                            .as_deref()
                            .and_then(|uid| group(scene.py(), &t, uid)),
                        g.getattr("children")?.get_item(-1),
                    ) {
                        let inverse = reference::frame(t.groups[owner]).inverse();
                        result.set_item(
                            "grips",
                            [
                                repeat::grip(&source, inverse)?,
                                repeat::grip(&last, inverse)?,
                            ],
                        )?;
                    }
                    for field in ["copies", "distinct", "stations"] {
                        if let Ok(value) = marker.get_item(field) {
                            result.set_item(field, value)?;
                        }
                    }
                }
            }
            return Ok(result);
        }
    }
    Ok(scene.py().None().into_bound(scene.py()))
}
/// Resolve the editable source for a double-click on a Repeat occurrence.
pub(crate) fn repeat_source<'p>(scene: &Obj<'p>, picked: &Obj<'p>) -> PyResult<Option<Obj<'p>>> {
    let Some(ext) = picked.getattr_opt("ext")?.filter(|ext| !ext.is_none()) else {
        return Ok(None);
    };
    let Some(marker) = ext
        .get_item("built.repeat")
        .ok()
        .or_else(|| ext.get_item(EXT).ok())
    else {
        return Ok(None);
    };
    let Some(id) = marker
        .get_item("row")
        .ok()
        .and_then(|id| id.extract::<String>().ok())
    else {
        return Ok(None);
    };
    let Some(t) = current(scene.py(), root(scene)?) else {
        return Ok(None);
    };
    Ok(t.rows
        .get(&id)
        .filter(|r| r.kind == "repeat")
        .and_then(|r| r.source.as_deref())
        .and_then(|uid| group(scene.py(), &t, uid)))
}

pub(crate) fn repeat_command<'p>(
    py: Python<'p>,
    source: &Obj<'p>,
    run: serde_json::Value,
    rule: serde_json::Value,
    uid: String,
) -> PyResult<Obj<'p>> {
    let source = source.getattr("uid")?.extract::<String>()?;
    let row = serde_json::from_value(serde_json::json!({
        "uid": uid, "kind":"repeat", "source":source, "run":run, "rule":rule
    }))
    .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let command = Bound::new(py, Mutation(Edit::Set(vec![row], Vec::new())))?.into_any();
    command.setattr("_repeat_selection_row", uid)?;
    Ok(command)
}
#[pyfunction]
fn live_sweep<'p>(scene: &Obj<'p>, owner: &str) -> PyResult<Obj<'p>> {
    let Some(t) = current(scene.py(), root(scene)?) else {
        return Ok(scene.py().None().into_bound(scene.py()));
    };
    let Some(row) = t
        .owners
        .get(owner)
        .and_then(|uid| t.rows.get(uid))
        .filter(|r| r.kind == "sweep")
    else {
        return Ok(scene.py().None().into_bound(scene.py()));
    };
    let value = to_py(scene.py(), row)?;
    let cuts: Vec<_> = row.cuts.iter().filter_map(|id| t.rows.get(id)).collect();
    value.set_item("cut_rows", to_py(scene.py(), &cuts)?)?;
    let k = t.groups[owner];
    let parent = doc::lock().node(k).and_then(|n| n.owners.first().copied());
    // The frame the path's points are written in: the member's own when the
    // path is in definition space (new members: their axes follow the
    // profile), else its parent's.
    let path_frame = if row.sweep.as_ref().is_some_and(|s| s.path.definition_space) {
        reference::frame(k)
    } else {
        parent.map(reference::frame).unwrap_or(DMat4::IDENTITY)
    };
    value.set_item("parent_frame", path_frame.to_cols_array())?;
    if let Some(spec) = &row.sweep {
        let name = spec
            .profile
            .sketch
            .as_ref()
            .and_then(|uid| t.rows.get(uid))
            .and_then(|r| r.sketch_fields.name.as_str())
            .or_else(|| spec.profile.name())
            .unwrap_or("Drawn profile");
        value.set_item("profile_name", name)?;
        if let Some(g) = group(scene.py(), &t, owner) {
            if let Ok(path) = g.getattr("ext")?.get_item(EXT)?.get_item("path") {
                value.set_item("resolved_points", path)?;
                value.set_item("owner_frame", reference::frame(k).to_cols_array())?;
            }
        }
    }
    Ok(value)
}
#[pyfunction]
fn sweep_thumbnail<'p>(
    py: Python<'p>,
    key: String,
    pixels: u32,
) -> PyResult<Bound<'p, pyo3::types::PyBytes>> {
    if !(1..=512).contains(&pixels) {
        return Err(PyValueError::new_err("invalid thumbnail size"));
    }
    let profile = if key.starts_with('{') {
        serde_json::from_str(&key).map_err(|e| PyValueError::new_err(format!("Invalid profile: {e}")))?
    } else {
        profile::Profile { catalogue: None, sketch: None, drawing: None, library: key,
                           family: String::new(), params: Default::default() }
    };
    let outline = profile.outline(48).map_err(PyValueError::new_err)?;
    let scale = pixels as f64 * 0.8 / outline.height.max(outline.width);
    let mut paths = tiny_skia::PathBuilder::new();
    for ring in outline.rings {
        for (i, n) in ring.iter().enumerate() {
            let x = (pixels as f64 / 2. + (n.p.x - outline.width / 2.) * scale) as f32;
            let y = (pixels as f64 / 2. - (n.p.y - outline.height / 2.) * scale) as f32;
            if i == 0 {
                paths.move_to(x, y)
            } else {
                paths.line_to(x, y)
            }
        }
        paths.close();
    }
    let path = paths
        .finish()
        .ok_or_else(|| PyValueError::new_err("invalid profile"))?;
    let mut pix = tiny_skia::Pixmap::new(pixels, pixels).unwrap();
    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(196, 200, 203, 255);
    paint.anti_alias = true;
    pix.fill_path(
        &path,
        &paint,
        tiny_skia::FillRule::EvenOdd,
        tiny_skia::Transform::identity(),
        None,
    );
    Ok(pyo3::types::PyBytes::new(py, pix.data()))
}
#[pyfunction]
fn sweep_profile<'p>(row: &Obj<'p>) -> PyResult<Obj<'p>> {
    let spec: sweep::Sweep = json(row)?;
    let outline = spec
        .profile
        .outline(spec.segments)
        .map_err(PyValueError::new_err)?;
    let rings: Vec<Vec<_>> = outline
        .rings
        .iter()
        .map(|ring| {
            ring.iter()
                .map(|n| serde_json::json!([n.p, n.arc]))
                .collect()
        })
        .collect();
    to_py(
        row.py(),
        &serde_json::json!({"family":outline.family,"web":outline.web,"flange":outline.flange,"radius":outline.radius,"width":outline.width,"height":outline.height,"rings":rings,"offset":outline.attachment(&spec.attachment).map_err(PyValueError::new_err)?,"name":spec.profile.name()}),
    )
}
#[pyfunction(signature = (history, sweeps, wall_junctions=false))]
fn create_sweeps<'p>(
    history: &Obj<'p>,
    sweeps: &Obj<'p>,
    wall_junctions: bool,
) -> PyResult<Bound<'p, PyDict>> {
    let raw: Vec<serde_json::Value> = json(sweeps)?;
    let mut specs = Vec::with_capacity(raw.len());
    for mut value in raw {
        if let Some(uid) = value["preset_uid"].as_str() {
            let scene = history.getattr("scene")?;
            let t = current(scene.py(), root(&scene)?).unwrap_or_default();
            let p = presets::lookup(&t, uid)
                .or_else(|| serde_json::from_value(value["preset_definition"].clone()).ok())
                .ok_or_else(|| PyValueError::new_err("Unknown preset"))?;
            let obj = value
                .as_object_mut()
                .ok_or_else(|| PyValueError::new_err("Expected sweep fields"))?;
            for (k, v) in p.values {
                obj.entry(k).or_insert(v);
            }
        }
        specs.push(
            serde_json::from_value::<sweep::Sweep>(value)
                .map_err(|e| PyValueError::new_err(e.to_string()))?,
        );
    }
    if specs.is_empty() {
        return Err(PyValueError::new_err("expected a sweep"));
    }
    for spec in &specs {
        if spec.profile.sketch.is_none() {
            spec.profile
                .outline(spec.segments)
                .map_err(PyValueError::new_err)?;
        }
        if (spec.path.chain.is_empty() && spec.path.ends.is_none() && spec.path.points.len() < 2)
            || spec
                .path
                .points
                .windows(2)
                .any(|ab| DVec3::from(ab[0]).distance(DVec3::from(ab[1])) < 1e-9)
        {
            return Err(PyValueError::new_err("invalid_path"));
        }
    }
    edit(history, Edit::Create(specs, wall_junctions))
}
#[pyfunction]
fn live_set<'p>(history: &Obj<'p>, row: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let mut rows: Vec<Row> = if row.is_instance_of::<pyo3::types::PyList>() {
        json(row)?
    } else {
        vec![json(row)?]
    };
    if rows.is_empty() {
        return Err(PyValueError::new_err("Expected live rows"));
    }
    for row in &mut rows {
        if row.kind == "junction" && row.joint.parent.is_none() {
            row.joint.parent.clone_from(&row.junction);
        }
        if let (Some(owner), Some(key)) = (
            row.junction.as_ref().or(row.joint.parent.as_ref()), &row.key,
        ) {
            if row.uid.is_empty() {
                row.uid = junction::id(owner, key);
            }
            if row.kind == "sweep" && row.owner.is_empty() {
                row.owner = junction::id(owner, &format!("{key}:owner"));
            }
        }
        for uid in std::iter::once(&row.uid).chain(
            (!matches!(row.kind.as_str(), "sketch" | "repeat" | "junction")
                || !row.owner.is_empty())
            .then_some(&row.owner),
        ) {
            if uid.len() != 16 || !uid.bytes().all(|c| c.is_ascii_hexdigit()) {
                return Err(PyValueError::new_err("expected 16-hex uid"));
            }
        }
        if !matches!(
            row.kind.as_str(),
            "test_box" | "sweep" | "sketch" | "cut" | "repeat" | "junction"
        ) || row.v != 1
        {
            return Err(PyValueError::new_err("unsupported recipe"));
        }
        if row.kind == "sketch" {
            sketch::validate(&row).map_err(PyValueError::new_err)?;
        }
    }
    edit(history, Edit::Set(rows, Vec::new()))
}
/// Done in the Sketch workspace: the sketch row and every user of it
/// re-evaluated, one History step. A new sketch gets a fresh uid.
///
/// `adopt`: sweep row uids whose profile becomes this sketch in the same
/// step (a member's catalogue size opened as a drawing).
#[pyfunction]
#[pyo3(signature = (history, row, adopt=None))]
fn sketch_commit<'p>(history: &Obj<'p>, row: &Obj<'p>, adopt: Option<Vec<String>>) -> PyResult<Bound<'p, PyDict>> {
    let row = row.call_method0("copy")?.cast_into::<PyDict>()?;
    row.set_item("kind", "sketch")?;
    if row.get_item("uid")?.is_none_or(|u| u.is_none()) {
        row.set_item("uid", eid::new_uid())?;
    }
    let _ = row.del_item("state");
    let adopt = adopt.unwrap_or_default();
    let reply = if adopt.is_empty() {
        live_set(history, row.as_any())?
    } else {
        let parsed: Row = json(row.as_any())?;
        if parsed.uid.len() != 16 || !parsed.uid.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(PyValueError::new_err("expected 16-hex uid"));
        }
        sketch::validate(&parsed).map_err(PyValueError::new_err)?;
        edit(history, Edit::Commit(parsed, adopt))?
    };
    reply.set_item("uid", row.get_item("uid")?)?;
    Ok(reply)
}
/// The regions, area and problems of a sketch row, and its attachment
/// points: what a user of the sketch reads.
#[pyfunction]
fn sketch_profile<'p>(scene: &Obj<'p>, uid: &str) -> PyResult<Obj<'p>> {
    let py = scene.py();
    let t = current(py, root(scene)?).unwrap_or_default();
    let Some(row) = t.rows.get(uid).filter(|r| r.kind == "sketch") else {
        return Ok(py.None().into_bound(py));
    };
    let s = sketch::body(row).map_err(PyValueError::new_err)?;
    let p = built_draft2d::profile(&s);
    let names: Vec<String> = built_draft2d::sketch::ATTACH
        .iter()
        .map(|n| n.to_string())
        .chain(s.points.iter().map(|p| p.name.clone()))
        .collect();
    let attach: serde_json::Map<String, serde_json::Value> = names
        .iter()
        .filter_map(|n| {
            s.attachment(n)
                .map(|at| (n.clone(), serde_json::json!([at.x, at.y])))
        })
        .collect();
    to_py(
        py,
        &serde_json::json!({
            "state": row.state, "ok": p.ok(), "area": p.area,
            "regions": p.regions.len(),
            "holes": p.regions.iter().map(|r| r.holes.len()).sum::<usize>(),
            "problems": p.problems, "attach": attach, "default": s.attach,
            "curves": s.curves.iter().map(|c| c.id).collect::<Vec<_>>(),
        }),
    )
}
/// A catalogue size as a drawing (Sketch workspace JSON), or None.
#[pyfunction]
fn profile_sketch<'p>(py: Python<'p>, key: &Obj<'p>) -> PyResult<Obj<'p>> {
    let sketch = if let Ok(key) = key.extract::<String>() {
        profile::catalogue_sketch(&key)
    } else {
        profile::sketch_of(&json(key)?)
    };
    match sketch {
        Some(sketch) => to_py(py, &sketch),
        None => Ok(py.None().into_bound(py)),
    }
}
#[pyfunction]
fn live_release<'p>(history: &Obj<'p>, uid: String) -> PyResult<Bound<'p, PyDict>> {
    edit(history, Edit::Release(uid))
}
#[pyfunction]
fn live_reevaluate<'p>(history: &Obj<'p>, uid: String) -> PyResult<Bound<'p, PyDict>> {
    edit(history, Edit::Reevaluate(uid))
}
pub(crate) fn cut_shortcut(scene: &Obj<'_>, key: &str) -> PyResult<serde_json::Value> {
    let uid = match key {
        "c" => "78d69d1a1cd30f54", "m" => "81f3c1507efa12f3",
        "p" => "95f3d998bf6c7707", "b" => "f21af74966307f91", _ => return Ok(serde_json::Value::Null),
    };
    let t = current(scene.py(), root(scene)?).unwrap_or_default();
    if let Some(preset) = presets::lookup(&t, uid) {
        return Ok(serde_json::json!(preset));
    }
    let library = scene.py().import("built_render.wgpu_library")?;
    for item in library.call_method1("library_items", ("presets", scene))?.try_iter()? {
        let preset = item?.get_item("preset")?;
        if preset.get_item("uid")?.eq(uid)? && !preset.contains("needs")? {
            return json(&preset);
        }
    }
    Ok(serde_json::Value::Null)
}
pub(crate) fn cut_feedback(scene: &Obj<'_>, ids: &[String]) -> PyResult<Option<&'static str>> {
    let Some(table) = current(scene.py(), root(scene)?) else { return Ok(None); };
    Ok(ids.iter().filter_map(|id| table.rows.get(id)).find_map(|row| match row.state.as_str() {
        "ok" => None,
        "cut_misses" => Some("Live cut saved · the solids do not overlap"),
        "empty_cut" => Some("Live cut saved · the cutter removes the entire solid"),
        "end_unresolved" => Some("Cannot resolve the member ends for this cut"),
        _ => Some("Live cut needs attention · check the cut in the project tree"),
    }))
}

#[pyfunction]
fn live_rows<'p>(scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    let t = current(scene.py(), root(scene)?).unwrap_or_default();
    to_py(scene.py(), &t.by_creation())
}
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    presets::register(m)?;
    m.add_function(wrap_pyfunction!(junction::junction_markers, m)?)?;
    m.add_function(wrap_pyfunction!(junction_apply, m)?)?;
    m.add_function(wrap_pyfunction!(junction_skip, m)?)?;
    m.add_function(wrap_pyfunction!(junction_follow, m)?)?;
    meetings::register(m)?;
    m.add_function(wrap_pyfunction!(pack::pack_read, m)?)?;
    m.add_function(wrap_pyfunction!(pack::pack_export, m)?)?;
    m.add_function(wrap_pyfunction!(pack::pack_row_hash, m)?)?;
    m.add_function(wrap_pyfunction!(pack::junction_words, m)?)?;
    m.add_function(wrap_pyfunction!(pack::junction_thumbnail, m)?)?;
    m.add_function(wrap_pyfunction!(pack::junction_bind, m)?)?;
    m.add_function(wrap_pyfunction!(pack::junction_definitions, m)?)?;
    m.add_function(wrap_pyfunction!(junction_set, m)?)?;
    m.add_function(wrap_pyfunction!(junction_frames, m)?)?;
    m.add_function(wrap_pyfunction!(junction_release, m)?)?;
    m.add_function(wrap_pyfunction!(junction_capture, m)?)?;
    m.add_function(wrap_pyfunction!(junction_capture_preview, m)?)?;
    m.add_function(wrap_pyfunction!(junction_capture_offers, m)?)?;
    m.add_function(wrap_pyfunction!(junction_adopt, m)?)?;
    m.add_function(wrap_pyfunction!(junction_record, m)?)?;
    m.add_function(wrap_pyfunction!(junction_upgrade, m)?)?;
    m.add_function(wrap_pyfunction!(live_cut_display, m)?)?;
    m.add_function(wrap_pyfunction!(live_cuts, m)?)?;
    m.add_function(wrap_pyfunction!(cut::live_openings, m)?)?;
    m.add_function(wrap_pyfunction!(cut::live_holes, m)?)?;
    m.add_function(wrap_pyfunction!(sweep_thumbnail, m)?)?;
    m.add_function(wrap_pyfunction!(live_repeat, m)?)?;
    m.add_function(wrap_pyfunction!(live_sweep, m)?)?;
    m.add_function(wrap_pyfunction!(sweep_profile, m)?)?;
    m.add_function(wrap_pyfunction!(create_sweeps, m)?)?;
    m.add_function(wrap_pyfunction!(live_set, m)?)?;
    m.add_function(wrap_pyfunction!(sketch_commit, m)?)?;
    m.add_function(wrap_pyfunction!(sketch_profile, m)?)?;
    m.add_function(wrap_pyfunction!(sketch_scene::sketch_plane, m)?)?;
    m.add_function(wrap_pyfunction!(sketch_scene::sketch_context, m)?)?;
    m.add_function(wrap_pyfunction!(live_release, m)?)?;
    m.add_function(wrap_pyfunction!(profile_sketch, m)?)?;
    m.add_function(wrap_pyfunction!(live_reevaluate, m)?)?;
    m.add_function(wrap_pyfunction!(live_rows, m)?)
}
pub(crate) fn default_preset(scene: &Obj<'_>) -> PyResult<serde_json::Value> {
    let t = current(scene.py(), root(scene)?).unwrap_or_default();
    Ok(serde_json::json!(crate::structure::sweep_default(
        scene, None
    )?
    .and_then(|id| presets::lookup(&t, &id))))
}
pub(crate) fn clear(scene: &Obj<'_>) -> PyResult<()> {
    if let Some(root) = scene.getattr_opt("_root")? {
        root.call_method1("_set_attr", (TABLE, scene.py().None()))?;
    }
    Ok(())
}

pub(crate) fn repeat_labels(scene: &Obj<'_>) -> PyResult<HashMap<String, String>> {
    let mut labels = HashMap::new();
    let Some(k) = scene
        .getattr_opt("_root")?
        .and_then(|r| r.getattr("_doc_key").ok())
        .and_then(|v| v.extract().ok())
    else {
        return Ok(labels);
    };
    if let Some(t) = current(scene.py(), k) {
        for id in &t.repeats {
            let row = &t.rows[id];
            let rule = &row.repeat.rule;
            let label = if let Some(n) = rule.get("count") {
                format!("Repeat · {n}×")
            } else if let Some(n) = rule.get("divide") {
                format!("Repeat · /{n}")
            } else if let Some(n) = rule.get("fit_max") {
                format!("Repeat · ≤{n}")
            } else {
                format!("Repeat · {}", rule["spacing"])
            };
            labels.insert(row.owner.clone(), label);
        }
    }
    Ok(labels)
}

pub(crate) fn cut_labels(
    scene: &Obj<'_>,
) -> PyResult<HashMap<String, Vec<(String, String, bool)>>> {
    let mut labels = HashMap::new();
    let Some(r) = scene
        .getattr_opt("_root")?
        .and_then(|g| g.getattr("_doc_key").ok())
        .and_then(|k| k.extract().ok())
    else {
        return Ok(labels);
    };
    let Some(t) = current(scene.py(), r) else {
        return Ok(labels);
    };
    for id in &t.cut_targets {
        let row = &t.rows[id];
        let items = row
            .cuts
            .iter()
            .filter_map(|id| {
                let c = t.rows.get(id)?;
                let kind = match c.cutter.as_ref()? {
                    cut::Cutter::Plane(_) => "plane",
                    cut::Cutter::Bisector(_) => "joint bisector",
                    cut::Cutter::Body(_) => "body",
                    cut::Cutter::Repeat(_) => "Repeat · cutter",
                    cut::Cutter::Section(_) => "section",
                    cut::Cutter::Opening(_) => "Opening",
                    cut::Cutter::Hole(_) => "Hole",
                    cut::Cutter::Block(_) => "Block",
                };
                Some((
                    id.clone(),
                    format!(
                        "{} · {kind}",
                        c.sketch_fields
                            .name
                            .as_str()
                            .or(c.operation.as_deref())
                            .unwrap_or("subtract")
                    ),
                    c.on != Some(false),
                ))
            })
            .collect();
        labels.insert(row.owner.clone(), items);
    }
    Ok(labels)
}

pub(crate) fn finish_migration(scene: &Obj<'_>) -> PyResult<()> {
    if let Some(mut t) = current(scene.py(), root(scene)?) {
        let dirty = std::mem::take(&mut t.pending).into_iter().collect();
        run(scene, &mut t, dirty)?;
        put(scene, t)?;
    }
    Ok(())
}

#[pyfunction]
fn live_cuts<'p>(scene: &Obj<'p>, owner: &str) -> PyResult<Obj<'p>> {
    let t = current(scene.py(), root(scene)?).unwrap_or_default();
    let cuts: Vec<_> = t
        .owners
        .get(owner)
        .and_then(|uid| t.rows.get(uid))
        .into_iter()
        .flat_map(|r| &r.cuts)
        .filter_map(|id| t.rows.get(id))
        .collect();
    to_py(scene.py(), &cuts)
}

/// Apply a detail to bound members: one binding map or a list (one step).
/// `definitions` are the detail and the presets it needs, embedded in the
/// same step when the model lacks them (a Library or pack detail). `force`:
/// outside its rule (the tool's Alt-click). `along`: `{"end": "start"|"end"|
/// "both"}` applies it to every copy of the Repeat the binding's member
/// belongs to.
#[pyfunction]
#[pyo3(signature=(history, detail, bindings, definitions=None, force=false, along=None))]
fn junction_apply<'p>(
    history: &Obj<'p>,
    detail: String,
    bindings: &Obj<'p>,
    definitions: Option<&Obj<'p>>,
    force: bool,
    along: Option<&Obj<'p>>,
) -> PyResult<Bound<'p, PyDict>> {
    let roles: serde_json::Value = json(bindings)?;
    if let Some(along) = along.filter(|a| !a.is_none()) {
        let mut along: serde_json::Value = json(along)?;
        if along["repeat"].is_null() {
            let scene = history.getattr("scene")?;
            let t = current(scene.py(), root(&scene)?).unwrap_or_default();
            along["repeat"] = serde_json::json!(junction::along_repeat(&scene, &t, &roles)
                .ok_or_else(|| PyValueError::new_err("Not a copy of a Repeat"))?);
        }
        let uid = eid::new_uid();
        let row: Row = serde_json::from_value(serde_json::json!({"kind":"junction","uid":uid,"detail":{"uid":detail},"along":along}))
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        let definitions: Vec<presets::Preset> = match definitions {
            Some(d) if !d.is_none() => json(d)?,
            _ => Vec::new(),
        };
        let reply = edit(history, Edit::Set(vec![row], definitions))?;
        reply.set_item("uid", uid)?;
        return Ok(reply);
    }
    // One use, or several applied in one History step (a review's Enter).
    let many = roles.is_array();
    let uses: Vec<serde_json::Value> = match roles {
        serde_json::Value::Array(list) => list,
        one => vec![one],
    };
    if uses.is_empty() {
        return Err(PyValueError::new_err("Expected junction bindings"));
    }
    let uids: Vec<String> = uses.iter().map(|_| eid::new_uid()).collect();
    let rows = uses
        .into_iter()
        .zip(&uids)
        .map(|(roles, uid)| {
            let mut row = serde_json::json!({"kind":"junction","uid":uid,"detail":{"uid":detail},"roles":roles});
            if force {
                row["local"] = serde_json::json!({"forced": true});
            }
            serde_json::from_value::<Row>(row).map_err(|e| PyValueError::new_err(e.to_string()))
        })
        .collect::<PyResult<Vec<_>>>()?;
    // The detail and the presets it needs, embedded in the same step when
    // the model lacks them (a Library or pack detail).
    let definitions: Vec<presets::Preset> = match definitions {
        Some(d) if !d.is_none() => json(d)?,
        _ => Vec::new(),
    };
    let reply = edit(history, Edit::Set(rows, definitions))?;
    if many {
        reply.set_item("uids", uids)?;
    } else {
        reply.set_item("uid", &uids[0])?;
    }
    Ok(reply)
}
/// Skip a meeting for a detail (remembered, undoable), or un-skip it.
#[pyfunction]
#[pyo3(signature=(history, detail, meeting, skip=true))]
fn junction_skip<'p>(history: &Obj<'p>, detail: String, meeting: &Obj<'p>, skip: bool) -> PyResult<Bound<'p, PyDict>> {
    let keys: Vec<String> = match meeting.extract::<String>() {
        Ok(one) => vec![one],
        Err(_) => meeting.extract()?,
    };
    if keys.is_empty() {
        return Err(PyValueError::new_err("Expected meeting keys"));
    }
    edit(history, Edit::Skip(detail, keys, skip))
}
/// Follow a detail: matching meetings that appear are listed on it.
#[pyfunction]
#[pyo3(signature=(history, detail, on=true))]
fn junction_follow<'p>(history: &Obj<'p>, detail: String, on: bool) -> PyResult<Bound<'p, PyDict>> {
    edit(history, Edit::Follow(detail, on))
}
#[pyfunction]
fn junction_set<'p>(history: &Obj<'p>, row: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let copy = row.call_method0("copy")?;
    copy.set_item("kind", "junction")?;
    if !copy.contains("uid")? {
        let uid = match (copy.get_item("parent"), copy.get_item("key")) {
            (Ok(parent), Ok(key)) => {
                junction::id(&parent.extract::<String>()?, &key.extract::<String>()?)
            }
            _ => eid::new_uid(),
        };
        copy.set_item("uid", uid)?;
    }
    let reply = live_set(history, &copy)?;
    reply.set_item("uid", copy.get_item("uid")?)?;
    Ok(reply)
}
#[pyfunction]
#[pyo3(signature=(history,uid,deep=false))]
fn junction_release<'p>(history: &Obj<'p>, uid: String, deep: bool) -> PyResult<Bound<'p, PyDict>> {
    let scene = history.getattr("scene")?;
    if current(scene.py(), root(&scene)?).is_none_or(|t| !t.junctions.contains(&uid)) {
        return Err(PyValueError::new_err("Unknown junction"));
    }
    if deep {
        edit(history, Edit::ReleaseTree(uid))
    } else {
        live_release(history, uid)
    }
}
/// Capture the items at a meeting as a detail and its first use, one
/// History step (docs/junctions.md §5). `seed`: a cut row, a part or a
/// junction uid, or the two member placements.
#[pyfunction]
#[pyo3(signature=(history, seed, options=None))]
fn junction_capture<'p>(history: &Obj<'p>, seed: &Obj<'p>, options: Option<&Obj<'p>>) -> PyResult<Bound<'p, PyDict>> {
    let seed: serde_json::Value = json(seed)?;
    let options = match options {
        Some(o) if !o.is_none() => json(o)?,
        _ => serde_json::json!({}),
    };
    let scene = history.getattr("scene")?;
    let reply = edit(history, Edit::Capture(seed, options))?;
    if let Some(v) = scene.getattr_opt("_junction_captured")?.filter(|v| !v.is_none()) {
        reply.set_item("uid", v.get_item(0)?)?;
        reply.set_item("detail", v.get_item(1)?)?;
        scene.setattr("_junction_captured", scene.py().None())?;
    }
    Ok(reply)
}
/// What capture would gather and save, without writing anything: the
/// popover's data (members, Includes, parameters, rule, similar count).
#[pyfunction]
#[pyo3(signature=(scene, seed, options=None))]
fn junction_capture_preview<'p>(scene: &Obj<'p>, seed: &Obj<'p>, options: Option<&Obj<'p>>) -> PyResult<Obj<'p>> {
    let seed: serde_json::Value = json(seed)?;
    let opts: junction::capture::Options = match options {
        Some(o) if !o.is_none() => json(o)?,
        _ => Default::default(),
    };
    let t = current(scene.py(), root(scene)?).unwrap_or_default();
    let d = junction::capture::draft(scene, &t, &seed, &opts)?;
    to_py(scene.py(), &d)
}
/// Meetings where two or more items no junction owns follow the members:
/// where the capture chip is offered.
#[pyfunction]
fn junction_capture_offers<'p>(scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    let t = current(scene.py(), root(scene)?).unwrap_or_default();
    let offers: Vec<_> = junction::capture::offers(scene, &t)?
        .into_iter()
        .map(|(m, items)| serde_json::json!({"key": m.key, "at": m.at, "label": m.label, "members": m.members, "items": items}))
        .collect();
    to_py(scene.py(), &offers)
}
/// The junction record for the inspector (`uid`: the row or its assembly).
#[pyfunction]
fn junction_record<'p>(scene: &Obj<'p>, uid: &str) -> PyResult<Obj<'p>> {
    let t = current(scene.py(), root(scene)?).unwrap_or_default();
    let v = junction::capture::record_of(scene, &t, uid)?;
    to_py(scene.py(), &v)
}
/// Upgrade to detail: this use's overrides become the detail's values (a
/// revision every use follows); the use then has none (§6.1).
#[pyfunction]
fn junction_upgrade<'p>(history: &Obj<'p>, uid: String) -> PyResult<Bound<'p, PyDict>> {
    edit(history, Edit::Upgrade(uid))
}
/// Adopt cut rows or parts into a junction (a cut dragged onto it, or what
/// was made inside it): this use only (`detail=False`), or its detail.
#[pyfunction]
#[pyo3(signature=(history, junction, rows, detail=false))]
fn junction_adopt<'p>(history: &Obj<'p>, junction: String, rows: Vec<String>, detail: bool) -> PyResult<Bound<'p, PyDict>> {
    if rows.is_empty() {
        return Err(PyValueError::new_err("Nothing to adopt"));
    }
    edit(history, Edit::Adopt(junction, rows, detail))
}
#[pyfunction]
fn junction_frames<'p>(scene: &Obj<'p>, uid: &str) -> PyResult<Obj<'p>> {
    let t = current(scene.py(), root(scene)?).unwrap_or_default();
    let row = t
        .rows
        .get(uid)
        .filter(|r| r.kind == "junction")
        .ok_or_else(|| PyValueError::new_err("Unknown junction"))?;
    to_py(scene.py(), &row.frames)
}
