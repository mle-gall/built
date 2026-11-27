// SPDX-License-Identifier: GPL-3.0-or-later
//! The root's journalled attribute holds a frozen, structurally shared table.
//! Definition replacements and placement markers are ordinary document ops.
mod cut;
pub(crate) use cut::{assign_cut_roles, hosted, migrate_wall, pushed_row};
mod drawn_profile;
mod file;
mod geometry;
mod joints;
mod presets;
pub(crate) mod profile;
mod reference;
mod repeat;
pub mod sketch;
mod sketch_scene;
mod sweep;
pub(crate) use file::{load, save};
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
    let value = if table.rows.is_empty() && table.presets.is_empty() {
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
) -> PyResult<&'static str> {
    let py = scene.py();
    if row.kind == "repeat" {
        return repeat::evaluate(scene, t, row);
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
    let stamp = {
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
        geometry::stamp(&s.inner)
    };
    let body_cut = row.cuts.iter().any(|id| {
        t.rows.get(id).is_some_and(|r| {
            r.on != Some(false)
                && ((row.kind == "cut_stack" && matches!(r.cutter, Some(cut::Cutter::Plane(_))))
                    || matches!(
                        r.cutter,
                        Some(
                            cut::Cutter::Body(_)
                                | cut::Cutter::Repeat(_)
                                | cut::Cutter::Section(_)
                                | cut::Cutter::Opening(_)
                        )
                    ))
        })
    });
    let (mesh, stamp) = if body_cut {
        match cut::bodies(scene, t, row, mesh, active, skipped)? {
            Ok(mesh) => {
                let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
                let stamp = geometry::stamp(&store.try_borrow()?.inner);
                (mesh, stamp)
            }
            Err(state) => return Ok(state),
        }
    } else {
        (mesh, stamp)
    };
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
    for uid in t.schedule(seeds).map_err(PyValueError::new_err)? {
        reference::bind_frames(t, &uid);
        let row = &t.rows[&uid];
        if row.state == "stale" || row.state == "cycle" {
            continue;
        }
        let mut active = None;
        let mut skipped = Vec::new();
        let state = evaluate(scene, t, row, &mut active, &mut skipped)?;
        let repeat = row.kind == "repeat";
        let cut_ids = row.cuts.clone();
        let initial = row.state.is_empty();
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
    let mut seeds: HashSet<_> = std::mem::take(&mut t.pending).into_iter().collect();
    let mut structure = false;
    let mut records = HashSet::new();
    let ifc = doc::attr_id("ifc").map_err(PyValueError::new_err)?;
    {
        let d = doc::lock();
        let mut moved = HashSet::new();
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
        for key in moved {
            seeds.extend(t.frames.get(&key).into_iter().flatten().cloned());
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
        t.groups = Arc::new(next);
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
                    for cut_id in &mut copy.cuts {
                        let mut cut = (*t
                            .rows
                            .get(cut_id)
                            .ok_or_else(|| PyValueError::new_err("Missing cut"))?)
                        .as_ref()
                        .clone();
                        cut.uid = eid::new_uid();
                        cut.owner = uid.clone();
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
                    for cut in &row.cuts {
                        t.remove(cut);
                    }
                }
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
        let Some(uid) = owner.as_ref().and_then(|id| t.owners.get(id)).cloned() else {
            continue;
        };
        let row = &t.rows[&uid];
        let Some(p) = row
            .sweep
            .as_ref()
            .and_then(|s| s.preset_uid.as_ref())
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
                let row = Arc::make_mut(t.rows.get_mut(&uid).unwrap())
                    .sweep
                    .as_mut()
                    .unwrap();
                row.preset = None;
                row.preset_uid = None;
                row.preset_rev = 0;
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
    let result = run(scene, &mut t, seeds).and_then(|()| put(scene, t));
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

#[derive(Clone)]
enum Edit {
    Presets(Vec<presets::Preset>),
    DeletePreset(String),
    Create(Vec<sweep::Sweep>),
    Set(Row),
    Release(String),
    Reevaluate(String),
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
                Edit::Presets(_) | Edit::DeletePreset(_) | Edit::Create(_)
            )
        {
            return Err(PyValueError::new_err("Unrecognised preset extension"));
        }
        match &self.0 {
            Edit::Presets(rows) => presets::update(scene, &mut t, rows)?,
            Edit::DeletePreset(id) => {
                if t.rows.values().any(|r| {
                    r.sweep
                        .as_ref()
                        .is_some_and(|s| s.preset_uid.as_ref() == Some(id))
                }) {
                    return Err(PyValueError::new_err("Preset is in use"));
                }
                Arc::make_mut(&mut t.presets)
                    .remove(id)
                    .ok_or_else(|| PyValueError::new_err("Unknown project preset"))?;
            }
            Edit::Create(sweeps) => {
                for spec in sweeps {
                    let key = &spec.profile.library;
                    let name = profile::label(key).unwrap_or("Drawn profile");
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
                    crate::structure::file_sweep(scene, &g, &destination)?;
                    let owner: String = g.getattr("uid")?.extract()?;
                    let uid = eid::new_uid();
                    Arc::make_mut(&mut t.groups)
                        .insert(owner.clone(), g.getattr("_doc_key")?.extract()?);
                    let definitions = std::mem::take(&mut spec.cut_definitions);
                    let target_uid = owner.clone();
                    t.insert(Row {
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
                    t.pending.insert(uid);
                }
            }
            Edit::Set(request) => {
                let mut row = request.clone();
                if row.kind == "repeat" {
                    repeat::prepare(scene, &mut t, &mut row)?;
                }
                if row.kind != "sketch" && group(py, &t, &row.owner).is_none() {
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
                    if let Some(g) = group(py, &t, &row.owner) {
                        if old.preset_uid != new.preset_uid {
                            if let Some(p) = presets::link(&mut t, new)? {
                                p.record(&g, false)?;
                            }
                        } else if let Some(p) = old
                            .preset_uid
                            .as_deref()
                            .and_then(|id| presets::lookup(&t, id))
                        {
                            if let Some(note) = presets::detach(old, new, &p) {
                                g.getattr("ifc")?.set_item("preset_note", note)?;
                            }
                        }
                    }
                }
                if row.kind == "cut" {
                    cut::attach(scene, &mut t, &mut row)?;
                } else if let Some(old) = t.rows.get(&row.uid) {
                    row.cuts.clone_from(&old.cuts);
                } else {
                    row.cuts.clear();
                }
                if let Some(spec) = row.sweep.as_mut() {
                    spec.preset_definition = None;
                    bind_drawing(&mut t, &mut spec.profile)?;
                }
                row.state.clear();
                let new_repeat = row.kind == "repeat" && !t.rows.contains_key(&row.uid);
                t.insert(row.clone()).map_err(PyValueError::new_err)?;
                if new_repeat {
                    let source = row.source.as_ref().unwrap();
                    let users: Vec<_> = t.users(&Key::Placement(source.clone())).cloned().collect();
                    for uid in users {
                        let Some(mut cut) = t.rows.get(&uid).map(|r| (**r).clone()) else {
                            continue;
                        };
                        if matches!(&cut.cutter, Some(cut::Cutter::Body(id)) if id == source) {
                            cut.cutter = Some(cut::Cutter::Repeat(row.uid.clone()));
                            cut::attach(scene, &mut t, &mut cut)?;
                            t.insert(cut).map_err(PyValueError::new_err)?;
                        }
                    }
                }
                t.schedule([row.uid.clone()])
                    .map_err(PyValueError::new_err)?;
                t.pending.insert(row.uid);
            }
            Edit::Release(id) => {
                let mut row = t
                    .remove(id)
                    .ok_or_else(|| PyValueError::new_err("unknown recipe"))?;
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
                    if evaluate(scene, &t, &target, &mut None, &mut Vec::new())? != "ok" {
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
                t.pending.extend(users);
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
    let command = Bound::new(py, Mutation(Edit::Set(row)))?.into_any();
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
    value.set_item(
        "parent_frame",
        parent
            .map(reference::frame)
            .unwrap_or(DMat4::IDENTITY)
            .to_cols_array(),
    )?;
    if let Some(spec) = &row.sweep {
        let name = spec
            .profile
            .sketch
            .as_ref()
            .and_then(|uid| t.rows.get(uid))
            .and_then(|r| r.sketch_fields.name.as_str())
            .or_else(|| profile::label(&spec.profile.library))
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
    let profile = profile::Profile {
        sketch: None,
        drawing: None,
        library: key,
        params: Default::default(),
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
        &serde_json::json!({"family":outline.family,"web":outline.web,"flange":outline.flange,"radius":outline.radius,"width":outline.width,"height":outline.height,"rings":rings,"offset":outline.attachment(&spec.attachment).map_err(PyValueError::new_err)?,"name":profile::label(&spec.profile.library)}),
    )
}
#[pyfunction]
fn create_sweeps<'p>(history: &Obj<'p>, sweeps: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
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
    edit(history, Edit::Create(specs))
}
#[pyfunction]
fn live_set<'p>(history: &Obj<'p>, row: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let row: Row = json(row)?;
    for uid in std::iter::once(&row.uid)
        .chain((!matches!(row.kind.as_str(), "sketch" | "repeat") || !row.owner.is_empty()).then_some(&row.owner))
    {
        if uid.len() != 16 || !uid.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(PyValueError::new_err("expected 16-hex uid"));
        }
    }
    if !matches!(
        row.kind.as_str(),
        "test_box" | "sweep" | "sketch" | "cut" | "repeat"
    ) || row.v != 1
    {
        return Err(PyValueError::new_err("unsupported recipe"));
    }
    if row.kind == "sketch" {
        sketch::validate(&row).map_err(PyValueError::new_err)?;
    }
    edit(history, Edit::Set(row))
}
/// Done in the Sketch workspace: the sketch row and every user of it
/// re-evaluated, one History step. A new sketch gets a fresh uid.
#[pyfunction]
fn sketch_commit<'p>(history: &Obj<'p>, row: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let row = row.call_method0("copy")?.cast_into::<PyDict>()?;
    row.set_item("kind", "sketch")?;
    if row.get_item("uid")?.is_none_or(|u| u.is_none()) {
        row.set_item("uid", eid::new_uid())?;
    }
    let _ = row.del_item("state");
    let reply = live_set(history, row.as_any())?;
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
    let names: Vec<String> = built_draft2d::sketch::ATTACH.iter().map(|n| n.to_string()).chain(s.points.iter().map(|p| p.name.clone())).collect();
    let attach: serde_json::Map<String, serde_json::Value> =
        names.iter().filter_map(|n| s.attachment(n).map(|at| (n.clone(), serde_json::json!([at.x, at.y])))).collect();
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
#[pyfunction]
fn live_release<'p>(history: &Obj<'p>, uid: String) -> PyResult<Bound<'p, PyDict>> {
    edit(history, Edit::Release(uid))
}
#[pyfunction]
fn live_reevaluate<'p>(history: &Obj<'p>, uid: String) -> PyResult<Bound<'p, PyDict>> {
    edit(history, Edit::Reevaluate(uid))
}
#[pyfunction]
fn live_rows<'p>(scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    let t = current(scene.py(), root(scene)?).unwrap_or_default();
    to_py(scene.py(), &t.by_creation())
}
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    presets::register(m)?;
    m.add_function(wrap_pyfunction!(live_cuts, m)?)?;
    m.add_function(wrap_pyfunction!(cut::live_openings, m)?)?;
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
    m.add_function(wrap_pyfunction!(live_reevaluate, m)?)?;
    m.add_function(wrap_pyfunction!(live_rows, m)?)
}
pub(crate) fn starter_preset() -> serde_json::Value {
    presets::starter()
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
