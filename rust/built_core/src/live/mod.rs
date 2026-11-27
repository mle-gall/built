// SPDX-License-Identifier: GPL-3.0-or-later
//! The root's journalled attribute holds a frozen, structurally shared table.
//! Definition replacements and placement markers are ordinary document ops.
mod geometry;
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
    Some(
        value
            .bind(py)
            .cast::<LiveTable>()
            .expect("native live table")
            .get()
            .clone(),
    )
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
    let mut todo = vec![root];
    while let Some(k) = todo.pop() {
        let Some(node) = d.node(k) else { continue };
        if let Data::Placement(p) = &node.data {
            if let Some(uid) = &p.uid {
                result.insert(uid.clone(), k);
            }
        }
        todo.extend(node.children.iter().copied());
    }
    result
}
fn group<'p>(py: Python<'p>, table: &LiveTable, uid: &str) -> Option<Obj<'p>> {
    doc::lock()
        .handle_ref(py, *table.groups.get(uid)?)
        .map(|g| g.into_bound(py))
}
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
fn reference(
    scene: &Obj<'_>,
    table: &LiveTable,
    r: &Reference,
    owner: doc::Key,
) -> PyResult<Option<DVec3>> {
    let py = scene.py();
    let (g, local) = match r {
        Reference::Point(p) => return Ok(Some(DVec3::from(*p))),
        Reference::Placement { placement } => (group(py, table, placement), DVec3::ZERO),
        Reference::Row { row } => {
            let Some(source) = table.rows.get(row).filter(|s| s.state == "ok") else {
                return Ok(None);
            };
            (group(py, table, &source.owner), DVec3::ZERO)
        }
        Reference::Element { path, id } => {
            let (g, mesh) = if path.is_empty() {
                (None, scene.getattr("mesh")?)
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
        let Some(point) = reference(scene, t, r, t.groups[&row.owner])? else {
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
        let row = &t.rows[&uid];
        if row.state == "stale" || row.state == "cycle" {
            continue;
        }
        let state = evaluate(scene, t, row)?;
        t.set_state(&uid, state);
    }
    Ok(())
}

pub(crate) fn finish(scene: &Obj<'_>, document: doc::Key) -> PyResult<()> {
    let py = scene.py();
    let Some(mut t) = current(py, document) else {
        return Ok(());
    };
    let mut seeds: HashSet<_> = std::mem::take(&mut t.pending).into_iter().collect();
    let mut structure = false;
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
                    f: Field::Definition | Field::Xform,
                    ..
                } => add(*k),
                Op::SetMany {
                    keys,
                    f: Field::Definition | Field::Xform,
                    ..
                } => keys.iter().copied().for_each(&mut add),
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
    }
    if structure {
        t.groups = Arc::new(index(document));
    }
    for mesh in doc::geometry::touched_meshes(py) {
        let root = scene.getattr("_root")?;
        if mesh.bind(py).is(&scene.getattr("mesh")?) {
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
    run(scene, &mut t, seeds)?;
    put(scene, t)
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
        let document = root(scene)?;
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
pub(crate) fn save(scene: &Obj<'_>, out: &Bound<'_, PyDict>) -> PyResult<()> {
    let py = scene.py();
    let Some(t) = current(py, root(scene)?) else {
        return Ok(());
    };
    let ext = match out.get_item("_doc_ext")? {
        Some(e) => e.cast_into::<PyDict>()?,
        None => PyDict::new(py),
    };
    let data = PyDict::new(py);
    data.set_item("v", 1)?;
    data.set_item("rows", to_py(py, &t.by_creation())?)?;
    ext.set_item(EXT, data)?;
    out.set_item("_doc_ext", ext)
}
pub(crate) fn load(scene: &Obj<'_>, ext: &Bound<'_, PyDict>) -> PyResult<()> {
    let Some(data) = ext.get_item(EXT)? else {
        return Ok(());
    };
    // Preserve an unrecognised extension verbatim like other optional data.
    let Ok(data) = data.cast::<PyDict>() else {
        return Ok(());
    };
    if data.get_item("v")?.and_then(|v| v.extract::<u32>().ok()) != Some(1) {
        return Ok(());
    }
    let Some(raw) = data.get_item("rows")? else {
        return Ok(());
    };
    let Ok(rows) = json::<Vec<Row>>(&raw) else {
        return Ok(());
    };
    let py = scene.py();
    let mut t = LiveTable::default();
    t.groups = Arc::new(index(root(scene)?));
    for mut row in rows {
        let id = row.uid.clone();
        if t.rows.contains_key(&id) {
            return Ok(());
        }
        if let Some(g) = group(py, &t, &row.owner) {
            let marker = metadata(&g)?.get_item(EXT)?;
            let stamp = marker
                .as_ref()
                .and_then(|m| m.get_item("stamp").ok())
                .and_then(|v| v.extract::<String>().ok());
            if let Some(stamp) = stamp {
                let store = g
                    .getattr("mesh")?
                    .getattr("_store")?
                    .cast_into::<PyMeshStore>()?;
                if stamp != geometry::stamp(&store.try_borrow()?.inner) {
                    row.state = "stale".into();
                }
            } else if row.state == "ok" {
                row.state = "stale".into();
            }
        } else {
            row.state = "ref_lost".into();
        }
        if t.insert(row).is_err() {
            return Ok(());
        }
        if t.schedule([id.clone()]).is_err() {
            t.set_state(&id, "cycle");
        }
    }
    put(scene, t)?;
    ext.del_item(EXT)
}
