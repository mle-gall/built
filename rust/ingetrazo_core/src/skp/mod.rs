// SPDX-License-Identifier: GPL-3.0-or-later
//! Metadata carried in .skp VFF model.dat records. Tags are little-endian.
mod payload;
mod dispatch;
pub use dispatch::register as register_dispatch;
mod lifetime;
pub use lifetime::{image_stage_geometry, FlatGeometry, PrototypeGeometry};
mod finish;
pub use finish::{
    skp_cached_colorize, skp_empty_model, skp_file_layers, skp_inverted_png, skp_merge_protos,
    skp_needs_tint, skp_texture_dir,
};
mod collect;
pub use collect::{
    skp_collect, skp_curve_segments, skp_defn_geom, skp_instance_identity, skp_loose_segments,
    skp_soft_segments, skp_world_points,
};
mod tree;
pub use tree::{skp_adapt_inner, skp_parse};
mod vff;
pub use vff::skp_vff_model;
mod images;
pub use images::{skp_image_attached, skp_image_bounds, skp_image_faces, skp_mark_drapes};
mod surface;
pub use surface::{skp_face_entry, skp_positioned_uvs, skp_ring};
mod apply;
pub use apply::{skp_apply, skp_apply_folders, skp_apply_geometry, skp_apply_metadata};
mod annotations;
pub use annotations::{
    skp_annotations, skp_definition_spans, skp_instance_matrix, skp_model_annotations,
    skp_resolve_annotations,
};
mod insertion;
pub use insertion::SkpInsertion;
pub(crate) use insertion::SkpOpenUndo;
mod materials;
pub use materials::{skp_materials, skp_read_pbr};
mod mfc;
pub use mfc::{skp_mfc_object, skp_mfc_readers, SkpArchiveCursor};
mod definition;
pub use definition::skp_definition;
mod geometry;
pub use geometry::skp_geometry_nodes;
use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict, PyInt, PyList},
};

#[derive(Clone, Copy)]
struct Records<'a>(&'a [u8]);

impl<'a> Iterator for Records<'a> {
    type Item = (u16, &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        let header = self.0.get(..6)?;
        let tag = u16::from_le_bytes(header[..2].try_into().ok()?);
        let size = u32::from_le_bytes(header[2..].try_into().ok()?) as usize;
        let (body, tail) = self.0[6..].split_at_checked(size)?;
        self.0 = tail;
        Some((tag, body))
    }
}

// Page fields must form one complete record sequence. The folder manager
// instead retains complete siblings preceding a truncated record.
fn fields(data: &[u8]) -> Option<Records<'_>> {
    let mut records = Records(data);
    if records.any(|(tag, _)| tag == 0) || !records.0.is_empty() {
        return None;
    }
    Some(Records(data))
}

fn field(data: &[u8], tag: u16) -> Option<&[u8]> {
    fields(data)?.find(|r| r.0 == tag).map(|r| r.1)
}

fn id_list(mut data: &[u8]) -> Vec<&[u8]> {
    let mut ids = Vec::new();
    while let Some((&size, tail)) = data.split_first() {
        let Some((id, rest)) = tail.split_at_checked(usize::from(size)) else {
            break;
        };
        if id.is_empty() {
            break;
        }
        ids.push(id);
        data = rest;
    }
    ids
}

fn integer<'py>(py: Python<'py>, bytes: &[u8]) -> PyResult<Bound<'py, PyAny>> {
    if bytes.len() <= 8 {
        let mut raw = [0; 8];
        raw[..bytes.len()].copy_from_slice(bytes);
        return Ok(u64::from_le_bytes(raw).into_pyobject(py)?.into_any());
    }
    py.get_type::<PyInt>()
        .call_method1("from_bytes", (PyBytes::new(py, bytes), "little"))
}

fn ids<'py>(py: Python<'py>, bytes: &[&[u8]]) -> PyResult<Bound<'py, PyList>> {
    let result = PyList::empty(py);
    for id in bytes {
        result.append(integer(py, id)?)?;
    }
    Ok(result)
}

struct Folder<'a> {
    key: Option<&'a [u8]>,
    name: String,
    parent: Option<usize>,
    hidden: bool,
    tags: Vec<&'a [u8]>,
}

fn folders(data: &[u8]) -> Vec<Folder<'_>> {
    let mut out = Vec::new();
    // One iterator per level, including for very deeply nested files.
    let mut stack = vec![(Records(data), None, true)];
    while let Some((children, parent, root)) = stack.last_mut() {
        let Some((_, body)) = children.find(|r| r.0 == 0x3e80) else {
            stack.pop();
            continue;
        };
        let mut row = Folder {
            key: None,
            name: String::new(),
            parent: *parent,
            hidden: false,
            tags: Vec::new(),
        };
        let mut subs = &[][..];
        for (tag, value) in Records(body) {
            match tag {
                0x05dc => {
                    row.key = if value.starts_with(&[0xde, 0x05]) {
                        Records(value).next().map(|r| r.1)
                    } else {
                        Some(value)
                    }
                }
                0x3e81 => row.name = String::from_utf8_lossy(value).into_owned(),
                0x3e82 => row.hidden = value.first() == Some(&1),
                0x3e83 => subs = value,
                0x3e84 => row.tags = id_list(value),
                _ => (),
            }
        }
        let parent = if *root {
            None
        } else {
            out.push(row);
            Some(out.len() - 1)
        };
        if !subs.is_empty() {
            stack.push((Records(subs), parent, false));
        }
    }
    out
}

#[pyfunction]
pub fn skp_tag_folders<'py>(py: Python<'py>, payload: &[u8]) -> PyResult<Bound<'py, PyList>> {
    let rows = py.detach(|| folders(payload));
    let result = PyList::empty(py);
    let mut keys: Vec<Bound<'py, PyAny>> = Vec::new();
    for row in rows {
        let record = PyDict::new(py);
        let key = match row.key {
            Some(bytes) => integer(py, bytes)?,
            None => ("anon", keys.len()).into_pyobject(py)?.into_any(),
        };
        record.set_item("key", &key)?;
        record.set_item("parent", row.parent.map(|i| &keys[i]))?;
        record.set_item("name", row.name)?;
        record.set_item("tags", ids(py, &row.tags)?)?;
        record.set_item("hidden", row.hidden)?;
        result.append(record)?;
        keys.push(key);
    }
    Ok(result)
}

type Point = (f64, f64, f64);
fn point(raw: &[u8]) -> Option<Point> {
    let raw: &[u8; 24] = raw.try_into().ok()?;
    Some((
        number(&raw[..8])?,
        number(&raw[8..16])?,
        number(&raw[16..])?,
    ))
}
fn number(raw: &[u8]) -> Option<f64> {
    Some(f64::from_le_bytes(raw.try_into().ok()?))
}

#[derive(IntoPyObject)]
struct CameraPage {
    name: String,
    eye: Option<Point>,
    target: Option<Point>,
    up: Option<Point>,
    fov: f64,
    parallel: bool,
    ortho_height: f64,
}

fn pages(data: &[u8]) -> Vec<(CameraPage, Vec<&[u8]>)> {
    let mut out = Vec::new();
    let children = |body, tag| {
        fields(body)
            .into_iter()
            .flatten()
            .filter_map(move |(t, v)| (tag == t).then_some(v))
    };
    for page in children(data, 0x6d60)
        .flat_map(|v| children(v, 0x6d61))
        .flat_map(|v| children(v, 0x7148))
    {
        let Some(items) = fields(page) else { continue };
        let get = |tag| items.clone().find(|r| r.0 == tag).map(|r| r.1);
        let name = get(0x6f54)
            .and_then(|v| field(v, 0x6f55))
            .unwrap_or_default();
        let camera = get(0x714a).and_then(|v| field(v, 0x34bc)).and_then(fields);
        let cam = |tag| camera?.find(|r| r.0 == tag).map(|r| r.1);
        out.push((
            CameraPage {
                name: String::from_utf8_lossy(name).into_owned(),
                eye: cam(0x34bd).and_then(point),
                target: cam(0x34be).and_then(point),
                up: cam(0x34bf).and_then(point),
                fov: cam(0x34c4).and_then(number).unwrap_or(35.),
                parallel: cam(0x34c2).and_then(|v| v.first()) == Some(&0),
                ortho_height: cam(0x34c3).and_then(number).unwrap_or(0.),
            },
            id_list(get(0x7150).unwrap_or_default()),
        ));
    }
    out
}

#[pyfunction]
pub fn skp_pages<'py>(py: Python<'py>, payload: &[u8]) -> PyResult<Bound<'py, PyList>> {
    let rows = py.detach(|| pages(payload));
    let result = PyList::empty(py);
    for (page, hidden) in rows {
        let record = page.into_pyobject(py)?;
        record.set_item("hidden_layer_ids", ids(py, &hidden)?)?;
        result.append(record)?;
    }
    Ok(result)
}
