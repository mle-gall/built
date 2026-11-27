// SPDX-License-Identifier: GPL-3.0-or-later
//! Persistent ids in `.built` format 2 (docs/persistent-ids.md §5).
//!
//! The document header `doc` names the document (`id`, 32 hex) and lists
//! the origins its element ids use (`origins`, hex, index 0 = "0"). A
//! geometry block names its definition (`uid`) and carries, per kind,
//! either `ids[kind]` (implicit: the packed id of the first element, hex;
//! the rest follow by +1) or an array `<kind>_id` of wrapping differences
//! (`<u8`). Packed ids in the file index the file's origin table; in
//! memory they index the process table (`element_ids`).
use crate::element_ids as eid;
use numpy::{IntoPyArray, PyReadonlyArray1};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyString, PyTuple};

type Obj<'p> = Bound<'p, PyAny>;

pub const KINDS: [&str; 3] = ["vertex", "edge", "face"];
pub const FORMAT: i64 = 2;
/// Reserved for exact geometry (docs/persistent-ids.md §11,
/// docs/exact-geometry.md §2): the critical feature, the document `ext`
/// namespace holding its pools, and the block array families binding
/// elements (by packed id) to pool entries.
pub const EXACT_GEOMETRY: &str = "built.exact_geometry.v1";
pub const EXACT_NAMESPACE: &str = "built.exact_geometry";
pub const RESERVED_ARRAY_PREFIXES: &[&str] = &["face_surface_", "edge_exact_curve_", "face_loop_", "coedge_"];

fn bad(what: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(format!("damaged element ids ({what})"))
}

/// Wrapping prefix sum of the file's differences.
pub fn undelta(d: &[u64]) -> Vec<u64> {
    let mut acc = 0u64;
    d.iter()
        .map(|&x| {
            acc = acc.wrapping_add(x);
            acc
        })
        .collect()
}

pub fn delta(ids: &[u64]) -> Vec<u64> {
    let mut prev = 0u64;
    ids.iter()
        .map(|&x| {
            let d = x.wrapping_sub(prev);
            prev = x;
            d
        })
        .collect()
}

fn parse_hex(v: &Obj<'_>, what: &str) -> PyResult<u64> {
    let s: String = v.extract().map_err(|_| bad(what))?;
    u64::from_str_radix(&s, 16).map_err(|_| bad(what))
}

/// The process-space index map of a document's `doc.origins` (None:
/// identity, or no ids at all).
#[pyfunction]
pub fn file_origin_map(doc: &Obj<'_>) -> PyResult<Option<Vec<u32>>> {
    if doc.is_none() {
        return Ok(None);
    }
    let origins = doc.call_method1("get", ("origins",))?;
    if origins.is_none() {
        return Ok(None);
    }
    let mut values = Vec::new();
    for o in origins.try_iter()? {
        values.push(parse_hex(&o?, "origins")?);
    }
    eid::file_to_process(&values).map_err(bad)
}

fn remap(id: u64, map: Option<&[u32]>) -> PyResult<u64> {
    match map {
        None => Ok(id),
        Some(m) => eid::remap(id, m).map_err(bad),
    }
}

/// The `ids` a deferred source carries (`store_views.store_from_source`):
/// `"legacy"` for a block without ids, else a 3-tuple of None / int /
/// uint64 array in process space.
#[pyfunction]
#[pyo3(signature=(meta, arrays, id_map=None))]
pub fn source_ids<'p>(
    meta: &Obj<'p>,
    arrays: &Obj<'p>,
    id_map: Option<Vec<u32>>,
) -> PyResult<Obj<'p>> {
    source_ids_checked(meta, arrays, id_map, false)
}

pub(crate) fn source_ids_checked<'p>(
    meta: &Obj<'p>,
    arrays: &Obj<'p>,
    id_map: Option<Vec<u32>>,
    planes: bool,
) -> PyResult<Obj<'p>> {
    let py = meta.py();
    // Only accept paired plane columns after the critical header is validated.
    if let Ok(names) = meta.get_item("arrays").and_then(|a| a.call_method0("keys")) {
        for name in names.try_iter()? {
            let name: String = name?.extract()?;
            if RESERVED_ARRAY_PREFIXES.iter().any(|p| name.starts_with(p))
                && !(planes
                    && matches!(
                        name.as_str(),
                        "face_surface_id"
                            | "face_surface_ref"
                            | "face_loop_patch"
                            | "coedge_topology"
                            | "coedge_edge_id"
                            | "coedge_vertex_id"
                            | "edge_exact_curve_id"
                            | "edge_exact_curve_ref"
                    ))
            {
                return Err(PyValueError::new_err(format!(
                    "This document needs features this build of Built lacks: {EXACT_GEOMETRY} ({name})"
                )));
            }
        }
    }
    let _ = arrays;
    let implicit = meta.call_method1("get", ("ids",))?;
    let map = id_map.as_deref();
    let mut out = Vec::new();
    let mut any = false;
    for k in KINDS {
        let name = format!("{k}_id");
        let col = arrays.call_method1("get", (name.as_str(),))?;
        if !col.is_none() {
            let np = py.import("numpy")?;
            let a = np.call_method1("ascontiguousarray", (col, np.getattr("uint64")?))?;
            let a: PyReadonlyArray1<u64> = a.extract().map_err(|_| bad(&name))?;
            let mut ids = undelta(a.as_slice()?);
            if let Some(m) = map {
                for id in ids.iter_mut() {
                    *id = eid::remap(*id, m).map_err(bad)?;
                }
            }
            out.push(ids.into_pyarray(py).into_any());
            any = true;
        } else if let Some(first) = implicit
            .cast::<PyDict>()
            .ok()
            .map(|d| d.get_item(k))
            .transpose()?
            .flatten()
        {
            out.push(remap(parse_hex(&first, "ids")?, map)?.into_pyobject(py)?.into_any());
            any = true;
        } else {
            out.push(PyString::new(py, "legacy").into_any());
        }
    }
    if !any {
        return Ok(PyString::new(py, "legacy").into_any());
    }
    Ok(PyTuple::new(py, out)?.into_any())
}

/// The process ids of a deferred source's elements, one array per kind in
/// plan order (`vertex_src` / `edge_src` from the plan, faces 1:1): what
/// the store built from that source would hold, legacy numbering included.
#[pyfunction]
pub fn plan_ids<'p>(
    py: Python<'p>,
    ids: &Obj<'p>,
    vertex_src: PyReadonlyArray1<'p, i64>,
    edge_src: PyReadonlyArray1<'p, i64>,
    n_points: usize,
    n_edges: usize,
    n_faces: usize,
) -> PyResult<Bound<'p, PyTuple>> {
    let owned = crate::store::ids_py::sources(Some(ids))?;
    let sources = crate::store::ids_py::borrow(&owned);
    let to_opt = |a: &[i64]| a.iter().map(|&i| (i >= 0).then_some(i as usize)).collect::<Vec<_>>();
    let vs = to_opt(vertex_src.as_slice()?);
    let es = to_opt(edge_src.as_slice()?);
    let fs: Vec<Option<usize>> = (0..n_faces).map(Some).collect();
    let cols = [
        crate::store::ids::build_column(0, sources[0], &vs, n_points),
        crate::store::ids::build_column(1, sources[1], &es, n_edges),
        crate::store::ids::build_column(2, sources[2], &fs, n_faces),
    ];
    let lens = [vs.len(), es.len(), n_faces];
    let arrays: Vec<Obj<'p>> = cols
        .iter()
        .zip(lens)
        .map(|(c, n)| (0..n as u32).map(|s| c.get(s)).collect::<Vec<u64>>().into_pyarray(py).into_any())
        .collect();
    PyTuple::new(py, arrays)
}

fn rows_of(src: &Obj<'_>, key: &str) -> PyResult<usize> {
    let v = src.call_method1("get", (key,))?;
    if v.is_none() {
        return Ok(0);
    }
    let shape: Vec<usize> = v.py().import("numpy")?.call_method1("shape", (v,))?.extract()?;
    Ok(match key {
        "pos" | "edges" if shape.len() == 1 => shape[0] / if key == "pos" { 3 } else { 2 },
        _ => shape.first().copied().unwrap_or(0),
    })
}

/// A deferred source's `ids`, minted once and kept in the source when it
/// has none (a block that came from an importer, not a file), so every
/// later build and save of it agrees.
#[pyfunction]
pub fn source_ids_of<'p>(src: &Obj<'p>) -> PyResult<Obj<'p>> {
    let have = src.call_method1("get", ("ids",))?;
    if !have.is_none() {
        return Ok(have);
    }
    let py = src.py();
    let n = [rows_of(src, "pos")?, rows_of(src, "edges")?, rows_of(src, "counts")?];
    let firsts: Vec<u64> = n.iter().enumerate().map(|(k, &n)| eid::mint_range(k, n as u64)).collect();
    let t = PyTuple::new(py, firsts)?.into_any();
    src.set_item("ids", &t)?;
    Ok(t)
}

/// Write side: `arrays` holds `vertex_id` / `edge_id` / `face_id` as
/// process ids in the block's element order. Replace them by the file's
/// encoding and fill `meta["ids"]`. Without a format-2 stream (`.igz`,
/// compatibility copies) the ids are dropped.
pub fn encode_block_ids<'p>(arrays: &Bound<'p, PyDict>, meta: &Bound<'p, PyDict>, sink: &Obj<'p>) -> PyResult<()> {
    let py = arrays.py();
    let mut cols = Vec::new();
    for k in KINDS {
        let name = format!("{k}_id");
        let col = arrays.get_item(&name)?;
        if col.is_some() {
            arrays.del_item(&name)?;
        }
        cols.push((k, name, col));
    }
    let Some(table) = sink.getattr_opt("id_origins")?.filter(|t| !t.is_none()) else {
        return crate::exact::file::encode(arrays, sink, None);
    };
    let table = table.cast_into::<PyList>()?;
    let mut index: Vec<u32> = table.extract()?;
    let implicit = PyDict::new(py);
    for (k, name, col) in cols {
        let Some(col) = col else { continue };
        let np = py.import("numpy")?;
        let a = np.call_method1("ascontiguousarray", (col, np.getattr("uint64")?))?;
        let a: PyReadonlyArray1<u64> = a.extract()?;
        let mut ids = a.as_slice()?.to_vec();
        if ids.is_empty() {
            continue;
        }
        for id in ids.iter_mut() {
            let o = eid::origin_index(*id);
            let f = match index.iter().position(|&p| p == o) {
                Some(f) => f,
                None => {
                    index.push(o);
                    index.len() - 1
                }
            };
            *id = eid::pack(f as u32, eid::counter(*id));
        }
        let sequential = ids.iter().enumerate().all(|(i, &id)| ids[0].wrapping_add(i as u64) == id)
            && eid::counter(ids[0]) + ids.len() as u64 <= eid::COUNTER_MASK + 1;
        if sequential {
            implicit.set_item(k, format!("{:x}", ids[0]))?;
        } else {
            arrays.set_item(name, delta(&ids).into_pyarray(py))?;
        }
    }
    crate::exact::file::encode(arrays, sink, Some(&index))?;
    sink.setattr("id_origins", PyList::new(py, index)?)?;
    if !implicit.is_empty() {
        meta.set_item("ids", implicit)?;
    }
    Ok(())
}

/// The document header of a format-2 save: `{"id", "origins"}` (hex).
pub fn document_header<'p>(py: Python<'p>, doc_id: Option<String>, sink: Option<&Obj<'p>>) -> PyResult<Bound<'p, PyDict>> {
    let out = PyDict::new(py);
    out.set_item("id", match doc_id { Some(d) => d, None => eid::fresh_doc_id(py)? })?;
    let index: Vec<u32> = match sink.map(|s| s.getattr_opt("id_origins")).transpose()?.flatten() {
        Some(t) if !t.is_none() => t.extract()?,
        _ => vec![0],
    };
    let origins: Vec<String> = index
        .iter()
        .map(|&i| format!("{:x}", eid::origin_of(i).unwrap_or(0)))
        .collect();
    out.set_item("origins", origins)?;
    Ok(out)
}

/// The document id of a scene, drawn (and kept on the scene) when it has
/// none yet: a new document, or a format-1 / `.igz` file before its first
/// format-2 save.
#[pyfunction]
pub(crate) fn ensure_doc_id(scene: &Obj<'_>) -> PyResult<String> {
    if let Some(v) = scene.getattr_opt("doc_id")? {
        if let Ok(s) = v.extract::<String>() {
            if s.len() == 32 {
                return Ok(s);
            }
        }
    }
    let id = eid::fresh_doc_id(scene.py())?;
    scene.setattr("doc_id", &id)?;
    Ok(id)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let ids = m.getattr("element_ids")?;
    let ids = ids.cast::<PyModule>()?;
    ids.add_function(wrap_pyfunction!(file_origin_map, ids)?)?;
    ids.add_function(wrap_pyfunction!(source_ids, ids)?)?;
    ids.add_function(wrap_pyfunction!(plan_ids, ids)?)?;
    ids.add_function(wrap_pyfunction!(source_ids_of, ids)?)?;
    ids.add_function(wrap_pyfunction!(ensure_doc_id, ids)?)?;
    ids.add("BUILT_FORMAT", FORMAT)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_round_trip_wraps() {
        let ids = [5u64, 3, u64::MAX, 0, 7];
        assert_eq!(undelta(&delta(&ids)), ids);
    }
}

// ------------------------------------------------------------ row uids

/// The uid a row object carries, or a new one set on it. `seen` keeps a
/// table's uids unique (a duplicated row copied its original's uid: the
/// copy takes a new one). An object that cannot hold one gets `fallback`.
pub(crate) fn row_uid(obj: &Obj<'_>, seen: &mut std::collections::HashSet<String>, fallback: &str) -> PyResult<String> {
    let have = obj
        .getattr_opt("uid")
        .ok()
        .flatten()
        .and_then(|v| v.extract::<String>().ok())
        .filter(|s| !s.is_empty() && !seen.contains(s));
    let uid = match have {
        Some(u) => u,
        None => {
            let u = eid::fresh_uid(obj.py())?;
            if obj.setattr("uid", &u).is_err() {
                fallback.to_string()
            } else {
                u
            }
        }
    };
    seen.insert(uid.clone());
    Ok(uid)
}

/// Archive side: give every row of `table` its object's uid (objects and
/// rows in the same order).
pub(crate) fn archive_row_uids(scene: &Obj<'_>, out: &Bound<'_, PyDict>, table: &str) -> PyResult<()> {
    let Some(rows) = out.get_item(table)? else { return Ok(()) };
    let Ok(rows) = rows.cast_into::<PyList>() else { return Ok(()) };
    let objs: Vec<Obj<'_>> = scene.getattr(table)?.try_iter()?.collect::<PyResult<_>>()?;
    if objs.len() != rows.len() {
        return Ok(());
    }
    let mut seen = std::collections::HashSet::new();
    for (i, (obj, row)) in objs.iter().zip(rows.iter()).enumerate() {
        let Ok(row) = row.cast_into::<PyDict>() else { continue };
        // A row without one is named after its table and place, as a file
        // without ids is on load: the same document saves the same bytes.
        let derived = eid::derived_uid(&format!("built:legacy:v1:{table}:{i}"));
        let empty = obj.getattr_opt("uid").ok().flatten().and_then(|v| v.extract::<String>().ok()).is_none_or(|s| s.is_empty());
        if empty && !seen.contains(&derived) {
            let _ = obj.setattr("uid", &derived);
        }
        row.set_item("uid", row_uid(obj, &mut seen, &derived)?)?;
    }
    Ok(())
}

/// Load side: each object of `table` gets its row's uid, or the migration
/// one (`built:legacy:v1:<table>:<index>`) when the row has none.
pub(crate) fn restore_row_uids(scene: &Obj<'_>, payload: &Bound<'_, PyDict>, table: &str) -> PyResult<()> {
    let objs: Vec<Obj<'_>> = match scene.getattr(table) {
        Ok(v) => v.try_iter()?.collect::<PyResult<_>>()?,
        Err(_) => return Ok(()),
    };
    let rows: Vec<Obj<'_>> = match payload.get_item(table)? {
        Some(r) if r.is_instance_of::<PyList>() => r.try_iter()?.collect::<PyResult<_>>()?,
        _ => Vec::new(),
    };
    let parallel = rows.len() == objs.len();
    for (i, obj) in objs.iter().enumerate() {
        let from_row = parallel
            .then(|| rows[i].call_method1("get", ("uid",)).ok())
            .flatten()
            .and_then(|v| v.extract::<String>().ok())
            .filter(|s| !s.is_empty());
        let uid = from_row.unwrap_or_else(|| eid::derived_uid(&format!("built:legacy:v1:{table}:{i}")));
        let _ = obj.setattr("uid", uid);
    }
    Ok(())
}

/// Sheet items with an empty uid get one (on the object and the row), and
/// a frame showing a scene by name also names it by uid (`view_uid`).
pub(crate) fn archive_sheet_uids(scene: &Obj<'_>, out: &Bound<'_, PyDict>) -> PyResult<()> {
    let Some(rows) = out.get_item("compositions")?.and_then(|r| r.cast_into::<PyList>().ok()) else {
        return Ok(());
    };
    let sheets: Vec<Obj<'_>> = scene.getattr("compositions")?.try_iter()?.collect::<PyResult<_>>()?;
    if sheets.len() != rows.len() {
        return Ok(());
    }
    let mut scene_uids = std::collections::HashMap::new();
    for v in scene.getattr("saved_views")?.try_iter()? {
        let v = v?;
        if let (Ok(name), Some(uid)) = (v.getattr("name").and_then(|n| n.extract::<String>()), v.getattr_opt("uid")?.and_then(|u| u.extract::<String>().ok())) {
            scene_uids.entry(name).or_insert(uid);
        }
    }
    for (sheet, row) in sheets.iter().zip(rows.iter()) {
        let Ok(row) = row.cast_into::<PyDict>() else { continue };
        let sheet_uid = row.get_item("uid")?.map(|u| u.str().map(|s| s.to_string())).transpose()?.unwrap_or_default();
        for (family, items) in row.iter() {
            let Ok(items) = items.cast_into::<PyList>() else { continue };
            let Ok(family) = family.extract::<String>() else { continue };
            let Ok(objs) = sheet.getattr(family.as_str()) else { continue };
            let objs: Vec<Obj<'_>> = match objs.try_iter() {
                Ok(it) => it.collect::<PyResult<_>>()?,
                Err(_) => continue,
            };
            if objs.len() != items.len() {
                continue;
            }
            let mut seen = std::collections::HashSet::new();
            for (i, (obj, item)) in objs.iter().zip(items.iter()).enumerate() {
                let Ok(item) = item.cast_into::<PyDict>() else { continue };
                if !item.contains("uid")? {
                    continue;
                }
                // An item without one is named after its sheet and place
                // (deterministic, so the same document saves the same bytes).
                let derived = eid::derived_uid(&format!("built:sheet:v1:{sheet_uid}:{family}:{i}"));
                let empty = obj.getattr_opt("uid")?.and_then(|v| v.extract::<String>().ok()).is_none_or(|s| s.is_empty());
                if empty && !seen.contains(&derived) {
                    let _ = obj.setattr("uid", &derived);
                }
                let uid = row_uid(obj, &mut seen, "")?;
                if !uid.is_empty() {
                    item.set_item("uid", uid)?;
                }
                if let Some(name) = item
                    .get_item("view_key")?
                    .and_then(|k| k.extract::<String>().ok())
                    .and_then(|k| k.strip_prefix("scene:").map(str::to_string))
                {
                    if let Some(u) = scene_uids.get(&name) {
                        item.set_item("view_uid", u)?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Load side of `view_uid`: a frame names a scene renamed elsewhere (a
/// file merged from another copy): follow the uid.
pub(crate) fn restore_sheet_views(scene: &Obj<'_>, payload: &Bound<'_, PyDict>) -> PyResult<()> {
    let Some(rows) = payload.get_item("compositions")?.and_then(|r| r.cast_into::<PyList>().ok()) else {
        return Ok(());
    };
    let mut names = std::collections::HashMap::new();
    for v in scene.getattr("saved_views")?.try_iter()? {
        let v = v?;
        if let (Some(uid), Ok(name)) = (v.getattr_opt("uid")?.and_then(|u| u.extract::<String>().ok()), v.getattr("name").and_then(|n| n.extract::<String>())) {
            names.insert(uid, name);
        }
    }
    let sheets: Vec<Obj<'_>> = scene.getattr("compositions")?.try_iter()?.collect::<PyResult<_>>()?;
    if sheets.len() != rows.len() {
        return Ok(());
    }
    for (sheet, row) in sheets.iter().zip(rows.iter()) {
        let Some(frames) = row.call_method1("get", ("frames",)).ok().filter(|f| f.is_instance_of::<PyList>()) else { continue };
        let Ok(objs) = sheet.getattr("frames") else { continue };
        let objs: Vec<Obj<'_>> = objs.try_iter()?.collect::<PyResult<_>>()?;
        for (obj, raw) in objs.iter().zip(frames.try_iter()?) {
            let raw = raw?;
            let Some(uid) = raw.call_method1("get", ("view_uid",))?.extract::<Option<String>>()? else { continue };
            if let Some(name) = names.get(&uid) {
                let key = format!("scene:{name}");
                if !obj.getattr("view_key")?.eq(&key)? {
                    obj.setattr("view_key", key)?;
                }
            }
        }
    }
    Ok(())
}
