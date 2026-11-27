// SPDX-License-Identifier: GPL-3.0-or-later
//! Optional format-2 data. Opening validates the baked result in place.
use super::*;
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
// Read placement extension data from the arena. Building ten thousand
// tracked Python dictionaries just to compare their stamps dominates open.
fn field<'a>(v: &'a doc::value::Value, name: &str) -> Option<&'a doc::value::Value> {
    let doc::value::Value::Dict(items) = v else {
        return None;
    };
    items
        .iter()
        .find_map(|(k, v)| matches!(k,doc::value::Value::Str(s) if s==name).then_some(v))
}
fn marker(py: Python<'_>, k: doc::Key, attr: u16, uid: &str) -> Option<(String, Py<PyAny>)> {
    let d = doc::lock();
    let node = d.node(k)?;
    let Data::Placement(p) = &node.data else {
        return None;
    };
    let data = field(node.attr(attr)?, EXT)?;
    let doc::value::Value::Str(row) = field(data, "row")? else {
        return None;
    };
    let doc::value::Value::Str(stamp) = field(data, "stamp")? else {
        return None;
    };
    (row == uid).then(|| {
        Some((
            stamp.clone(),
            p.definition.as_ref()?.0.get().geometry.clone_ref(py),
        ))
    })?
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
    let mut stamps = HashMap::new();
    let attr = doc::attr_id("ext").map_err(PyValueError::new_err)?;
    for mut row in rows {
        let id = row.uid.clone();
        if row.v != 1 || row.kind != "test_box" || t.rows.contains_key(&id) {
            return Ok(());
        }
        if let Some(k) = t.groups.get(&row.owner) {
            if let Some((stamp, mesh)) = marker(py, *k, attr, &id) {
                let mesh = mesh.bind(py);
                let actual = match stamps.entry(mesh.as_ptr() as usize) {
                    std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        let store = mesh.getattr("_store")?.cast_into::<PyMeshStore>()?;
                        let digest = geometry::stamp(&store.try_borrow()?.inner);
                        entry.insert(digest)
                    }
                };
                if &stamp != actual {
                    row.state = "stale".into();
                }
            } else if row.state == "ok" {
                row.state = "stale".into();
            }
        } else {
            row.state = "ref_lost".into();
        }
        let has_inputs = !row.inputs.is_empty();
        if t.insert(row).is_err() {
            return Ok(());
        }
        if has_inputs && t.schedule([id.clone()]).is_err() {
            t.set_state(&id, "cycle");
        }
    }
    for uid in t.rows.keys().cloned().collect::<Vec<_>>() {
        super::reference::bind_frames(&mut t, &uid);
    }
    put(scene, t)?;
    ext.del_item(EXT)
}
