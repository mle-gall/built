// SPDX-License-Identifier: GPL-3.0-or-later
//! VFF import. Entity tables retain definition-local indices and the file's
//! declaration order. No TLV tree or duplicate encoded model is retained.
mod archive;
mod paint;
use super::{field, integer, number, point, Records};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet},
};
use std::{io::Read, path::PathBuf};

fn namespace(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    py.import("types")?.getattr("SimpleNamespace")?.call0()
}
fn raw(body: &[u8], tag: u16) -> Option<&[u8]> {
    Records(body).find(|(t, _)| *t == tag).map(|r| r.1)
}
fn path<'a>(mut body: &'a [u8], tags: &[u16]) -> Option<&'a [u8]> {
    for &t in tags {
        body = raw(body, t)?;
    }
    Some(body)
}
fn text_value(b: Option<&[u8]>) -> String {
    b.map(String::from_utf8_lossy)
        .unwrap_or_default()
        .into_owned()
}
fn numeric<'py>(py: Python<'py>, b: Option<&[u8]>) -> PyResult<Option<Bound<'py, PyAny>>> {
    b.map(|v| integer(py, v)).transpose()
}
fn bytes_id(b: &[u8]) -> &[u8] {
    raw(b, 0x05de).unwrap_or(b)
}
fn entity(b: &[u8]) -> Option<&[u8]> {
    raw(b, 0x05de)
        .or_else(|| raw(b, 0x05dc).map(bytes_id))
        .or_else(|| path(b, &[0x07d0, 0x05dc]).map(bytes_id))
}
fn hex(b: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(b.len() * 2);
    for byte in b {
        write!(s, "{byte:02x}").unwrap();
    }
    s
}
fn doubles(b: &[u8], count: usize) -> Option<Vec<f64>> {
    b.get(..count * 8).map(|b| {
        b.chunks_exact(8)
            .map(|p| f64::from_le_bytes(p.try_into().unwrap()))
            .collect()
    })
}
fn containers(tag: u16) -> bool {
    matches!(tag,0x01f4|0x01f6..=0x01f9|0x1770|0x1771|0x3a98|0x3a99|0x30d4|0x30d5|0x1388..=0x1397|0x0dae|0x1194|0x1195|0x4a38|0x1d4c|0x1f40|0x010f)
}
/// Collect only the requested record headers. Never recurse through an
/// opaque entity: texture bytes and attribute strings can contain tag bytes.
fn records<'a>(b: &'a [u8], target: &[u16], depth: u8, out: &mut Vec<(u16, &'a [u8])>) {
    if depth >= 64 {
        return;
    }
    for (t, body) in Records(b) {
        if target.contains(&t) {
            out.push((t, body));
        } else if containers(t) {
            records(body, target, depth + 1, out);
        }
    }
}
fn select<'a>(py: Python<'_>, b: &'a [u8], tags: &[u16]) -> Vec<(u16, &'a [u8])> {
    py.detach(|| {
        let mut out = Vec::new();
        records(b, tags, 0, &mut out);
        out
    })
}

/// Connection points may refer to an instance embedded inside another
/// instance record. Compose those physical placements in source inches.
fn annotation_transforms<'a>(
    body: &'a [u8],
    parent: Option<glam::DMat4>,
    depth: usize,
    positions: &mut Vec<(&'a [u8], glam::DMat4)>,
) {
    if depth > 64 {
        return;
    }
    for (tag, rec) in Records(body) {
        if tag == 0x1964 {
            let local = raw(rec, 0x1966).and_then(|v| doubles(v, 13)).map(|m| {
                let s = if m[12] == 0. { 1. } else { m[12] };
                glam::DMat4::from_cols_array(&[
                    m[0] * s,
                    m[3] * s,
                    m[6] * s,
                    0.,
                    m[1] * s,
                    m[4] * s,
                    m[7] * s,
                    0.,
                    m[2] * s,
                    m[5] * s,
                    m[8] * s,
                    0.,
                    m[9] * s,
                    m[10] * s,
                    m[11] * s,
                    1.,
                ])
            });
            let placed = match (parent, local) {
                (Some(a), Some(b)) => Some(a * b),
                (a, b) => a.or(b),
            };
            if let (Some(key), Some(xf)) = (entity(rec), placed) {
                positions.push((key, xf));
            }
            annotation_transforms(rec, placed, depth + 1, positions);
        } else if tag == 0x157c || containers(tag) {
            annotation_transforms(rec, parent, depth + 1, positions);
        }
    }
}

struct Model<'py> {
    py: Python<'py>,
    layer_names: Bound<'py, PyDict>,
    vertex_ids: Bound<'py, PyDict>,
    transforms: Bound<'py, PyDict>,
}
impl<'py> Model<'py> {
    fn layer(&self, id: Option<&[u8]>) -> PyResult<Option<Bound<'py, PyAny>>> {
        let Some(key) = numeric(self.py, id)? else {
            return Ok(None);
        };
        self.layer_names.get_item(key)
    }
    fn definition(&self, body: &[u8], root: bool) -> PyResult<Bound<'py, PyAny>> {
        let py = self.py;
        let def = namespace(py)?;
        let kind = raw(body, 0x1583)
            .and_then(|b| b.first())
            .copied()
            .unwrap_or(0);
        def.setattr(
            "id",
            if root {
                Some(0u8.into_pyobject(py)?.into_any())
            } else {
                numeric(
                    py,
                    path(body, &[0x1388, 0x07d0, 0x05dc])
                        .map(bytes_id)
                        .or_else(|| entity(body)),
                )?
            },
        )?;
        def.setattr(
            "name",
            if root {
                "ROOT_MODEL".into()
            } else {
                text_value(raw(body, 0x157e))
            },
        )?;
        def.setattr(
            "guid",
            if root {
                "ROOT".into()
            } else {
                raw(body, 0x157d)
                    .map(hex)
                    .unwrap_or_default()
                    .to_ascii_uppercase()
            },
        )?;
        def.setattr("is_group", kind == 1)?;
        def.setattr("is_image", kind == 2)?;
        for (t, k) in [
            (0x1b5d, "always_faces_camera"),
            (0x1b5e, "shadows_face_sun"),
        ] {
            def.setattr(
                k,
                path(body, &[0x1b58, t]).is_some_and(|s| s.first() == Some(&1)),
            )?;
        }
        let vertices = PyDict::new(py);
        let edges = PyDict::new(py);
        let faces = PyDict::new(py);
        let placements = PyList::empty(py);
        let construction_lines = PyList::empty(py);
        let construction_points = PyList::empty(py);
        for (i, (tag, rec)) in select(py, body, &[0x09c4, 0x0bb8, 0x0dac, 0x1964, 0x4269, 0x426c])
            .into_iter()
            .enumerate()
        {
            if i % 256 == 0 {
                crate::file_open::skp_checkpoint(py)?;
            }
            let id = entity(rec);
            let key = numeric(py, id)?;
            let draw = raw(rec, 0x07d0).unwrap_or_default();
            let flags = raw(draw, 0x07d3)
                .and_then(|v| v.first())
                .copied()
                .unwrap_or(0);
            match tag {
                0x09c4 => {
                    if let (Some(id), Some(p)) = (
                        id,
                        raw(rec, 0x09c5).and_then(|v| v.get(..24)).and_then(point),
                    ) {
                        let v = namespace(py)?;
                        v.setattr("id", &key)?;
                        v.setattr("x", p.0)?;
                        v.setattr("y", p.1)?;
                        v.setattr("z", p.2)?;
                        vertices.set_item(key, v)?;
                        self.vertex_ids.set_item(hex(id), p)?;
                    }
                }
                0x0bb8 if key.is_some() => {
                    let e = namespace(py)?;
                    e.setattr("id", &key)?;
                    for (tag, name) in [(0x0bb9, "v1_id"), (0x0bba, "v2_id"), (0x0bbb, "curve_id")]
                    {
                        e.setattr(
                            name,
                            numeric(
                                py,
                                raw(rec, tag)
                                    .filter(|b| tag != 0x0bbb || b.iter().any(|&x| x != 0)),
                            )?,
                        )?;
                    }
                    e.setattr("layer", numeric(py, raw(draw, 0x07d2))?)?;
                    for (bit, k) in [(1, "hidden"), (8, "soft"), (16, "smooth")] {
                        e.setattr(k, flags & bit != 0)?;
                    }
                    edges.set_item(key, e)?;
                }
                0x0dac if key.is_some() => {
                    let face = namespace(py)?;
                    face.setattr("id", &key)?;
                    face.setattr(
                        "normal",
                        raw(rec, 0x0dad)
                            .and_then(|b| b.get(..24))
                            .and_then(point)
                            .unwrap_or((0., 0., 1.)),
                    )?;
                    face.setattr("material_id", numeric(py, raw(draw, 0x07d1))?)?;
                    face.setattr("back_material_id", numeric(py, raw(rec, 0x0daf))?)?;
                    if root {
                        face.setattr("layer", numeric(py, raw(draw, 0x07d2))?)?;
                    } else {
                        let name = self
                            .layer(raw(draw, 0x07d2))?
                            .map(|s| s.extract::<String>())
                            .transpose()?
                            .unwrap_or_default();
                        face.setattr(
                            "layer",
                            if matches!(name.as_str(), "Layer0" | "Layer 0" | "Untagged") {
                                ""
                            } else {
                                &name
                            },
                        )?;
                    }
                    face.setattr("hidden", flags & 1 != 0)?;
                    let uv =
                        path(draw, &[0x05dc, 0x05dd, 0x36b1, 0x36b2, 0x2710]).unwrap_or_default();
                    for (tag, k) in [(0x2711, "uv_transform"), (0x2712, "uv_transform_back")] {
                        face.setattr(
                            k,
                            path(uv, &[tag, 0x2713, 0x2715])
                                .filter(|b| b.len() == 72)
                                .and_then(|b| doubles(b, 9)),
                        )?;
                    }
                    let loops = PyList::empty(py);
                    for (_, l) in select(py, raw(rec, 0x0dae).unwrap_or_default(), &[0x1194]) {
                        let uses = PyList::empty(py);
                        for (_, u) in select(py, l, &[0x0fa0]) {
                            if let (Some(edge), Some(sense)) = (raw(u, 0x0fa1), raw(u, 0x0fa2)) {
                                uses.append((
                                    integer(py, edge)?,
                                    if sense.iter().any(|n| *n != 0) { -1 } else { 1 },
                                ))?;
                            }
                        }
                        if !uses.is_empty() {
                            loops.append(uses)?;
                        }
                    }
                    face.setattr("loops", loops)?;
                    faces.set_item(key, face)?;
                }
                0x1964 => {
                    let ins = namespace(py)?;
                    ins.setattr("persistent_id", &key)?;
                    ins.setattr("ref_idx", numeric(py, raw(rec, 0x1967))?)?;
                    ins.setattr("name", text_value(raw(rec, 0x1965)))?;
                    let matrix = raw(rec, 0x1966)
                        .and_then(|b| doubles(b, 13))
                        .unwrap_or_default();
                    ins.setattr("matrix", matrix)?;
                    ins.setattr("hidden", flags & 1 != 0)?;
                    ins.setattr("layer", self.layer(raw(draw, 0x07d2))?)?;
                    ins.setattr("material_id", numeric(py, raw(draw, 0x07d1))?)?;
                    placements.append(ins)?;
                }
                0x4269 => {
                    if raw(rec, 0x426a).is_some_and(|b| b.len() >= 64) {
                        construction_lines.append(true)?;
                    }
                }
                0x426c => {
                    if raw(rec, 0x426d).is_some_and(|b| b.len() >= 24) {
                        construction_points.append(true)?;
                    }
                }
                _ => (),
            }
        }
        for (k, v) in [("vertices", vertices), ("edges", edges), ("faces", faces)] {
            def.setattr(k, v)?;
        }
        def.setattr("instances", placements)?;
        def.setattr("construction_lines", construction_lines)?;
        def.setattr("construction_points", construction_points)?;
        def.setattr("dimensions", PyList::empty(py))?;
        def.setattr("texts", PyList::empty(py))?;
        Ok(def)
    }
}

#[pyfunction]
pub fn skp_vff_model<'py>(
    py: Python<'py>,
    source: PathBuf,
    tables: &Bound<'py, PyAny>,
) -> PyResult<Option<Bound<'py, PyAny>>> {
    let classic = py.detach(|| -> std::io::Result<bool> {
        let mut head = Vec::new();
        std::fs::File::open(&source)?
            .take(512)
            .read_to_end(&mut head)?;
        Ok(head.windows(11).any(|w| w == b"CVersionMap"))
    })?;
    if classic {
        return Ok(None);
    }
    let mut zip = py.detach(|| archive::Archive::open(&source))?;
    let data = py.detach(|| zip.read("model.dat"))?;
    if super::fields(&data).is_none() || data.is_empty() {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "Incomplete .skp model.dat",
        ));
    }
    let model = namespace(py)?;
    model.setattr("version", &zip.version)?;
    let layer_names = PyDict::new(py);
    let mat_names = PyDict::new(py);
    let hidden = PyDict::new(py);
    for (t, b) in select(py, &data, &[0x3c8c, 0x32c8]) {
        if let Some(id) = raw(b, 0x05dc).map(bytes_id) {
            let key = integer(py, id)?;
            if t == 0x3c8c {
                let name = text_value(raw(b, 0x3c8d));
                layer_names.set_item(key, &name)?;
                hidden.set_item(name, raw(b, 0x3c8e).is_some_and(|v| v.first() == Some(&1)))?;
            } else {
                mat_names.set_item(key, text_value(raw(b, 0x32cc)))?;
            }
        }
    }
    if !layer_names.contains(1)? {
        layer_names.set_item(1, "Layer0")?;
    }
    let colors = PyDict::new(py);
    paint::read(py, &mut zip, &mat_names, &colors, &model)?;
    if !colors.contains("Layer0")? {
        colors.set_item("Layer0", (136, 136, 136))?;
    }
    let layers = PyList::empty(py);
    for (name, _) in colors {
        let l = namespace(py)?;
        l.setattr("name", &name)?;
        l.setattr(
            "hidden",
            hidden
                .get_item(name)?
                .is_some_and(|v| v.is_truthy().unwrap_or(false)),
        )?;
        layers.append(l)?;
    }
    model.setattr("layers", layers)?;
    let meta = Model {
        py,
        layer_names,
        vertex_ids: PyDict::new(py),
        transforms: PyDict::new(py),
    };
    let defs = PyDict::new(py);
    let groups = PySet::empty(py)?;
    let mut root = None;
    for (t, b) in select(py, &data, &[0x157c, 0x01f6]) {
        let d = meta.definition(b, t == 0x01f6)?;
        if t == 0x01f6 {
            root = Some(d)
        } else {
            let key = d.getattr("id")?;
            if d.getattr("is_group")?.is_truthy()? {
                groups.add(&key)?;
            }
            defs.set_item(key, d)?;
        }
    }
    model.setattr(
        "root",
        match root {
            Some(v) => v,
            None => meta.definition(&[], true)?,
        },
    )?;
    model.setattr("definitions", defs)?;
    let mut camera_list = PyList::empty(py);
    let mut folders = PyList::empty(py);
    for (t, b) in select(py, &data, &[0x0207, 0x3a9b]) {
        if t == 0x3a9b {
            folders = super::skp_tag_folders(py, b)?;
        } else {
            let rows = super::skp_pages(py, b)?;
            camera_list = PyList::empty(py);
            for row in rows {
                let page = namespace(py)?;
                page.getattr("__dict__")?.call_method1("update", (&row,))?;
                let tags = PyList::empty(py);
                for id in row.get_item("hidden_layer_ids")?.try_iter()? {
                    if let Some(name) = meta.layer_names.get_item(id?)? {
                        tags.append(name)?;
                    }
                }
                page.setattr("hidden_layers", tags)?;
                camera_list.append(page)?;
            }
        }
    }
    model.setattr("pages", camera_list)?;
    let positions = py.detach(|| {
        let mut positions = Vec::new();
        annotation_transforms(&data, None, 0, &mut positions);
        positions
    });
    for (id, xf) in positions {
        let m = xf.to_cols_array();
        meta.transforms.set_item(
            hex(id),
            [
                m[0], m[4], m[8], m[1], m[5], m[9], m[2], m[6], m[10], m[12], m[13], m[14], 1.,
            ],
        )?;
    }
    let dims = PyList::empty(py);
    for row in super::skp_annotations(py, true, &data, &meta.vertex_ids, &meta.transforms)? {
        let offset = row.get_item("offset")?.extract::<usize>()?;
        let body = Records(&data[offset..]).next().unwrap().1;
        let d = namespace(py)?;
        d.setattr("a", row.get_item("a")?)?;
        d.setattr("b", row.get_item("b")?)?;
        d.setattr("plane_x", field(body, 0x5bcf).and_then(point))?;
        d.setattr("normal", field(body, 0x5bd0).and_then(point))?;
        d.setattr("offset", field(body, 0x5bd2).and_then(number).unwrap_or(0.))?;
        dims.append(d)?;
    }
    model.setattr("dimensions", dims)?;
    super::skp_resolve_annotations(&model, &data, &meta.vertex_ids, &meta.transforms)?;
    tables.setattr("layers", meta.layer_names)?;
    tables.setattr("folders", folders)?;
    tables.setattr("groups", groups)?;
    tables.setattr("kinds", true)?;
    tables.setattr("pids", PyDict::new(py))?;
    tables.setattr("pbr", PyDict::new(py))?;
    Ok(Some(model))
}

pub(super) fn thumbnail(path: &std::path::Path) -> std::io::Result<Vec<u8>> {
    archive::Archive::open(path)?.read("meta/model_thumbnail.png")
}
