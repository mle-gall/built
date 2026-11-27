// SPDX-License-Identifier: GPL-3.0-or-later
//! The root's journalled attribute holds a frozen, structurally shared table.
//! Definition replacements and placement markers are ordinary document ops.
mod file;
mod geometry;
mod reference;
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
    let value = if table.rows.is_empty() {
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
fn evaluate(scene: &Obj<'_>, t: &LiveTable, row: &Row) -> PyResult<&'static str> {
    let py = scene.py();
    let Some(g) = group(py, t, &row.owner) else {
        return Ok("ref_lost");
    };
    if row.kind != "test_box" || row.v != 1 {
        return Ok("unsupported");
    }
    let mut origin = DVec3::ZERO;
    for r in &row.inputs {
        let Some(point) = reference::origin(scene, t, r, row)? else {
            return Ok("ref_lost");
        };
        origin += point;
    }
    let mesh = py.import("built_core.mesh")?.call_method0("Mesh")?;
    let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
    let stamp = {
        let mut s = store.try_borrow_mut()?;
        if let Err(state) = geometry::generate(&mut s.inner, row, origin.to_array()) {
            return Ok(state);
        }
        geometry::stamp(&s.inner)
    };
    let marker = PyDict::new(py);
    marker.set_item("row", &row.uid)?;
    marker.set_item("key", &stamp)?;
    marker.set_item("stamp", &stamp)?;
    let ext = metadata(&g)?;
    ext.set_item(EXT, marker)?;
    eid::set_definition_uid(
        &mesh,
        &eid::derived_uid(&format!("built:live:v1:{}", row.uid)),
    )?;
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
        let state = evaluate(scene, t, row)?;
        t.set_state(&uid, state);
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
                t.remove(&id);
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
    if seeds.is_empty() && !structure {
        return Ok(());
    }
    if journalled {
        run(scene, &mut t, seeds)?;
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
    fn doc_after(&self, scene: &Obj<'_>, _phase: &str) -> PyResult<()> {
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
        match &self.0 {
            Edit::Set(row) => {
                if group(py, &t, &row.owner).is_none() {
                    return Err(PyValueError::new_err("unknown recipe owner"));
                }
                if t.rows.get(&row.uid).is_some_and(|r| r.state == "stale") {
                    return Err(PyValueError::new_err(
                        "stale recipe requires re-evaluation or release",
                    ));
                }
                let mut row = row.clone();
                row.state.clear();
                t.insert(row.clone()).map_err(PyValueError::new_err)?;
                t.schedule([row.uid.clone()])
                    .map_err(PyValueError::new_err)?;
                t.pending.insert(row.uid);
            }
            Edit::Release(id) => {
                let row = t
                    .remove(id)
                    .ok_or_else(|| PyValueError::new_err("unknown recipe"))?;
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
fn live_set<'p>(history: &Obj<'p>, row: &Obj<'p>) -> PyResult<Bound<'p, PyDict>> {
    let row: Row = json(row)?;
    for uid in [&row.uid, &row.owner] {
        if uid.len() != 16 || !uid.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(PyValueError::new_err("expected 16-hex uid"));
        }
    }
    if row.kind != "test_box" || row.v != 1 {
        return Err(PyValueError::new_err("unsupported recipe"));
    }
    edit(history, Edit::Set(row))
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
    m.add_function(wrap_pyfunction!(live_set, m)?)?;
    m.add_function(wrap_pyfunction!(live_release, m)?)?;
    m.add_function(wrap_pyfunction!(live_reevaluate, m)?)?;
    m.add_function(wrap_pyfunction!(live_rows, m)?)
}
pub(crate) fn clear(scene: &Obj<'_>) -> PyResult<()> {
    if let Some(root) = scene.getattr_opt("_root")? {
        root.call_method1("_set_attr", (TABLE, scene.py().None()))?;
    }
    Ok(())
}
