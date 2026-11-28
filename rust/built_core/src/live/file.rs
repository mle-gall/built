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
    if let Some(prefs) = meetings::save_prefs(&t) {
        ext.set_item(meetings::PREFS_EXT, to_py(py, &prefs)?)?;
    }
    if !t.presets.is_empty() {
        ext.set_item(
            "built.presets",
            to_py(
                py,
                &serde_json::json!({"v":1,"rows":t.presets.values().collect::<Vec<_>>()}),
            )?,
        )?;
    }
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
    let raw = data.extract::<PyRef<FileLive>>().ok();
    if let Some(raw) = raw {
        if let Ok(pack) = serde_json::from_str::<FileRows>(raw.0.get()) {
            if pack.v == 1 && load_rows(scene, ext, pack.rows)? {
                return Ok(());
            }
        }
        let foreign = scene
            .py()
            .import("json")?
            .call_method1("loads", (raw.0.get(),))?;
        return ext.set_item(EXT, foreign);
    }
    let Ok(data) = data.cast::<PyDict>() else {
        return Ok(());
    };
    if data.get_item("v")?.and_then(|v| v.extract::<u32>().ok()) != Some(1) {
        return Ok(());
    }
    if let Some(raw) = data.get_item("rows")? {
        if let Ok(rows) = json::<Vec<Row>>(&raw) {
            load_rows(scene, ext, rows)?;
        }
    }
    Ok(())
}

#[pyclass(frozen)]
pub(crate) struct FileLive(pub Box<serde_json::value::RawValue>);

#[derive(serde::Deserialize)]
struct FileRows {
    v: u32,
    rows: Vec<Row>,
}

fn load_rows(scene: &Obj<'_>, ext: &Bound<'_, PyDict>, rows: Vec<Row>) -> PyResult<bool> {
    let py = scene.py();
    let mut t = LiveTable::default();
    t.groups = Arc::new(index(root(scene)?));
    let mut stamps = HashMap::new();
    let attr = doc::attr_id("ext").map_err(PyValueError::new_err)?;
    for mut row in rows {
        let id = row.uid.clone();
        if row.uid.is_empty()
            || row.v != 1
            || !matches!(
                row.kind.as_str(),
                "test_box" | "sweep" | "sketch" | "cut" | "cut_stack" | "repeat" | "junction"
            )
            || t.rows.contains_key(&id)
        {
            return Ok(false);
        }
        if row.kind == "cut" {
            if !t.groups.contains_key(&row.owner) {
                row.state = "ref_lost".into();
            }
        } else if matches!(row.kind.as_str(), "repeat" | "junction") {
            if let Some(g) = group(py, &t, &row.owner) {
                if row.state == "ok" {
                    let saved = metadata(&g)?
                        .get_item(EXT)?
                        .and_then(|v| v.get_item("stamp").ok())
                        .and_then(|v| v.extract::<String>().ok());
                    if saved.as_deref() != Some(&repeat::stamp(&g)?) {
                        row.state = "stale".into();
                    }
                }
            } else {
                row.state = if row.kind == "junction" {
                    "lost"
                } else {
                    "ref_lost"
                }
                .into();
            }
        } else if let Some(k) = t.groups.get(&row.owner) {
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
        } else if row.kind != "sketch" {
            row.state = "ref_lost".into();
        }
        let has_inputs = row.references().next().is_some();
        if t.insert(row).is_err() {
            return Ok(false);
        }
        if has_inputs && t.schedule([id.clone()]).is_err() {
            t.set_state(&id, "cycle");
        }
    }
    for uid in t.rows.keys().cloned().collect::<Vec<_>>() {
        if t.rows[&uid].kind == "cut" {
            cut::bind_placements(scene, &t, &t.rows[&uid])?;
        }
        super::reference::bind_frames(&mut t, &uid);
    }
    if let Some(pack) = ext.get_item("built.presets")? {
        if pack.get_item("v").is_ok_and(|v| v.eq(1).unwrap_or(false)) {
            if let Some(rows) = pack
                .get_item("rows")
                .ok()
                .and_then(|r| json::<Vec<presets::Preset>>(&r).ok())
            {
                for p in rows {
                    Arc::make_mut(&mut t.presets).insert(p.uid.clone(), p);
                }
                ext.del_item("built.presets")?;
            }
        }
    }
    let openings: Vec<_> = t
        .rows
        .values()
        .filter(|r| r.junction.is_none() && matches!(r.cutter, Some(cut::Cutter::Opening(_))))
        .cloned()
        .collect();
    for row in openings {
        let Some(cut::Cutter::Opening(o)) = &row.cutter else {
            continue;
        };
        let host_link =
            group(py, &t, &o.fill).and_then(|g| g.getattr("ext").ok()?.get_item("wall_host").ok());
        if host_link.is_some_and(|h| {
            h.get_item("uid")
                .is_ok_and(|v| v.eq(&row.owner).unwrap_or(false))
                && h.get_item("profile")
                    .is_ok_and(|v| v.extract::<[[f64; 3]; 4]>().is_ok())
        }) && DMat4::from_cols_array(&o.frame).inverse().is_finite()
            && t.owners
                .get(&row.owner)
                .and_then(|id| t.rows.get(id))
                .is_some_and(|target| target.cuts.contains(&row.uid))
            && !t.groups.contains_key(&junction::id(
                &eid::derived_uid(&format!(
                    "built:junction:v1:hosted:{}:{}",
                    row.owner, o.fill
                )),
                "owner",
            ))
        {
            junction::hosting::adopt(scene, &mut t, &row, &o.fill, true)?;
        }
    }
    if let Some(raw) = ext.get_item(meetings::PREFS_EXT)? {
        if let Some(prefs) = json::<serde_json::Value>(&raw).ok().and_then(meetings::load_prefs) {
            t.prefs = Arc::new(prefs);
            ext.del_item(meetings::PREFS_EXT)?;
        }
    }
    put(scene, t)?;
    ext.del_item(EXT)?;
    Ok(true)
}
