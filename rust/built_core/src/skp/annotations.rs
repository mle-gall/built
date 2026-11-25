// SPDX-License-Identifier: GPL-3.0-or-later
use super::{field, fields, point, Point};
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};

enum Anchor<'a> {
    Free(Point),
    Vertex(&'a [u8], Option<&'a [u8]>),
}

fn anchor(body: &[u8]) -> Option<Anchor<'_>> {
    let sub = field(body, 0x5208)?;
    if field(sub, 0x5209) == Some(&1u32.to_le_bytes()) {
        return point(field(sub, 0x520a)?).map(Anchor::Free);
    }
    let reference = field(field(sub, 0x520b)?, 0x53fc)?;
    let vertex = field(reference, 0x53fd).filter(|v| !v.is_empty())?;
    let instance = field(reference, 0x53fe).and_then(|v| {
        let (&size, tail) = v.split_first()?;
        tail.get(..usize::from(size)).filter(|id| !id.is_empty())
    });
    Some(Anchor::Vertex(vertex, instance))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(2 * bytes.len());
    for b in bytes {
        write!(s, "{b:02x}").unwrap();
    }
    s
}

impl Anchor<'_> {
    fn resolve(
        &self,
        vertices: &Bound<'_, PyDict>,
        transforms: &Bound<'_, PyDict>,
    ) -> PyResult<Option<Point>> {
        let Self::Vertex(key, instance) = self else {
            return Ok(match self {
                Self::Free(p) => Some(*p),
                _ => unreachable!(),
            });
        };
        let Some(p) = vertices.get_item(hex(key))? else {
            return Ok(None);
        };
        let [x, y, z] = p.extract::<[f64; 3]>()?;
        if let Some(id) = instance {
            if let Some(m) = transforms.get_item(hex(id))? {
                if !m.is_none() {
                    let m = m.extract::<Vec<f64>>()?;
                    if m.len() >= 12 {
                        let s = m.get(12).copied().unwrap_or(1.);
                        let axis = |i: usize| {
                            m[i] * s * x + m[i + 1] * s * y + m[i + 2] * s * z + m[9 + i / 3] * s
                        };
                        return Ok(Some((axis(0), axis(3), axis(6))));
                    }
                }
            }
        }
        Ok(Some((x, y, z)))
    }
}

enum Record<'a> {
    Dimension(Anchor<'a>, Anchor<'a>, Option<u32>),
    Text(String, Option<Anchor<'a>>, Option<Point>, bool),
}

fn scan(data: &[u8], dimensions: bool) -> Vec<(usize, Record<'_>)> {
    let marker = if dimensions {
        [0xcc, 0x5b]
    } else {
        [0xf0, 0x55]
    };
    data.windows(2)
        .enumerate()
        .filter_map(|(offset, tag)| {
            if tag != marker {
                return None;
            }
            let mut stream = super::Records(&data[offset..]);
            let (_, body) = stream.next()?;
            if body.len() < 40 || fields(body).is_none() {
                return None;
            }
            let record = if dimensions {
                Record::Dimension(
                    anchor(field(body, 0x5bcd)?)?,
                    anchor(field(body, 0x5bce)?)?,
                    field(body, 0x5bd1)
                        .and_then(|v| v.try_into().ok())
                        .map(u32::from_le_bytes),
                )
            } else {
                let text = String::from_utf8_lossy(field(body, 0x55f1)?)
                    .trim()
                    .to_owned();
                if text.is_empty() {
                    return None;
                }
                let point = anchor(field(body, 0x55f4)?);
                let label = field(body, 0x55f5).and_then(super::point);
                let hidden = field(body, 0x07d0)
                    .and_then(|sub| field(sub, 0x07d3))
                    .and_then(|v| v.first())
                    .is_some_and(|f| f & 1 != 0);
                Record::Text(text, point, label, hidden)
            };
            Some((offset, record))
        })
        .collect()
}

#[pyfunction]
pub fn skp_annotations<'py>(
    py: Python<'py>,
    dimensions: bool,
    data: &[u8],
    vertices: &Bound<'py, PyDict>,
    transforms: &Bound<'py, PyDict>,
) -> PyResult<Bound<'py, PyList>> {
    let records = py.detach(|| scan(data, dimensions));
    let result = PyList::empty(py);
    for (offset, record) in records {
        let row = PyDict::new(py);
        row.set_item("offset", offset)?;
        match record {
            Record::Dimension(a, b, axis) => {
                let (Some(a), Some(b)) = (
                    a.resolve(vertices, transforms)?,
                    b.resolve(vertices, transforms)?,
                ) else {
                    continue;
                };
                row.set_item("a", a)?;
                row.set_item("b", b)?;
                row.set_item("axis", axis)?;
            }
            Record::Text(text, at, label, hidden) => {
                row.set_item(
                    "point",
                    at.map(|p| p.resolve(vertices, transforms))
                        .transpose()?
                        .flatten(),
                )?;
                row.set_item("label_raw", label)?;
                row.set_item("text", text)?;
                row.set_item("hidden", hidden)?;
            }
        }
        result.append(row)?;
    }
    Ok(result)
}

#[pyfunction]
pub fn skp_definition_spans(data: &[u8]) -> Vec<(usize, usize)> {
    let first = super::Records(data).next();
    let model = match first {
        Some((0x01f4, body)) if body.len() + 6 == data.len() => body,
        _ => data,
    };
    let mut sub = model;
    for tag in [0x01f9, 0x1770, 0x1771] {
        let Some((_, body)) = super::Records(sub).find(|r| r.0 == tag) else {
            return Vec::new();
        };
        sub = body;
    }
    super::Records(sub)
        .filter(|r| r.0 == 0x157c)
        .map(|(_, body)| {
            let offset = body.as_ptr() as usize - data.as_ptr() as usize;
            (offset - 6, offset + body.len())
        })
        .collect()
}

fn members<'py>(obj: &Bound<'py, PyAny>, field: &str) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let Some(items) = obj.getattr_opt(field)?.filter(|v| !v.is_none()) else {
        return Ok(vec![]);
    };
    items.try_iter()?.collect()
}

type Bounds = ([f64; 3], [f64; 3]);

fn bounds(owner: &Bound<'_, PyAny>) -> PyResult<Option<Bounds>> {
    let Some(vertices) = owner.getattr_opt("vertices")?.filter(|v| !v.is_none()) else {
        return Ok(None);
    };
    let mut bounds: Option<Bounds> = None;
    for vertex in vertices.call_method0("values")?.try_iter()? {
        let v = vertex?;
        let p = [
            v.getattr("x")?.extract()?,
            v.getattr("y")?.extract()?,
            v.getattr("z")?.extract()?,
        ];
        let (lo, hi) = bounds.get_or_insert((p, p));
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    Ok(bounds)
}

fn label_position(
    anchor: Option<Point>,
    label: Option<Point>,
    bounds: Option<Bounds>,
) -> Option<Point> {
    let (Some(p), Some(l)) = (anchor, label) else {
        return label;
    };
    let delta = (p.0 + l.0, p.1 + l.1, p.2 + l.2);
    if let Some((lo, hi)) = bounds {
        let inside = |point: Point| {
            [point.0, point.1, point.2]
                .into_iter()
                .enumerate()
                .all(|(i, v)| v >= lo[i] - 1. && v <= hi[i] + 1.)
        };
        if inside(l) && !inside(delta) {
            return Some(l);
        }
    }
    Some(delta)
}

#[pyfunction]
pub fn skp_resolve_annotations(
    model: &Bound<'_, PyAny>,
    data: &[u8],
    vertices: &Bound<'_, PyDict>,
    transforms: &Bound<'_, PyDict>,
) -> PyResult<usize> {
    let py = model.py();
    let defs = match model.getattr_opt("definitions")?.filter(|v| !v.is_none()) {
        Some(map) => map
            .call_method0("values")?
            .try_iter()?
            .collect::<PyResult<Vec<_>>>()?,
        None => vec![],
    };
    let spans = py.detach(|| skp_definition_spans(data));
    let owner = |offset| {
        if spans.len() != defs.len() {
            return None;
        }
        spans
            .iter()
            .enumerate()
            .filter(|(_, (start, end))| *start <= offset && offset < *end)
            .min_by_key(|(_, (start, end))| end - start)
            .map(|(i, _)| &defs[i])
    };
    let root = model.getattr_opt("root")?.filter(|r| !r.is_none());
    let mut touched = 0;
    let dims = members(model, "dimensions")?;
    let mut has_dims = false;
    let mut has_texts = false;
    for d in &defs {
        has_dims |= d
            .getattr_opt("dimensions")?
            .is_some_and(|v| v.is_truthy().unwrap_or(false));
        has_texts |= d
            .getattr_opt("texts")?
            .is_some_and(|v| v.is_truthy().unwrap_or(false));
    }
    if !dims.is_empty() && !has_dims {
        let records = skp_annotations(py, true, data, vertices, transforms)?;
        if records.len() == dims.len() {
            let keep = PyList::empty(py);
            for (dim, rec) in dims.into_iter().zip(records.iter()) {
                dim.setattr("vff_axis", rec.get_item("axis")?)?;
                if let Some(def) = owner(rec.get_item("offset")?.extract()?) {
                    dim.setattr("vff_owned", true)?;
                    append_member(def, "dimensions", &dim)?;
                } else {
                    keep.append(dim)?;
                }
                touched += 1;
            }
            if keep.len() != records.len() {
                model.setattr("dimensions", keep)?;
            }
        }
    }
    if let Some(root) = root {
        if !has_texts && members(&root, "texts")?.is_empty() {
            let mut boxes = std::collections::HashMap::new();
            let class = py.import("types")?.getattr("SimpleNamespace")?;
            for rec in skp_annotations(py, false, data, vertices, transforms)? {
                let def = owner(rec.get_item("offset")?.extract()?).unwrap_or(&root);
                let bbox = match boxes.entry(def.as_ptr()) {
                    std::collections::hash_map::Entry::Occupied(slot) => *slot.get(),
                    std::collections::hash_map::Entry::Vacant(slot) => *slot.insert(bounds(def)?),
                };
                let kwargs = PyDict::new(py);
                for key in ["point", "text", "hidden"] {
                    kwargs.set_item(key, rec.get_item(key)?)?;
                }
                kwargs.set_item(
                    "label_point",
                    label_position(
                        rec.get_item("point")?.extract()?,
                        rec.get_item("label_raw")?.extract()?,
                        bbox,
                    ),
                )?;
                let tx = class.call((), Some(&kwargs))?;
                tx.setattr("vff_recovered", true)?;
                append_member(def, "texts", &tx)?;
                touched += 1;
            }
        }
    }
    Ok(touched)
}
fn append_member(owner: &Bound<'_, PyAny>, field: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
    if let Some(list) = owner
        .getattr_opt(field)?
        .and_then(|v| v.cast_into::<PyList>().ok())
    {
        return list.append(value);
    }
    let items = PyList::new(owner.py(), members(owner, field)?)?;
    items.append(value)?;
    owner.setattr(field, items)
}

fn optional<'py, T: for<'a> pyo3::FromPyObject<'a, 'py>>(
    obj: &Bound<'py, PyAny>,
    field: &str,
) -> PyResult<Option<T>> {
    obj.getattr_opt(field)?
        .filter(|v| !v.is_none())
        .map(|v| v.extract().map_err(Into::into))
        .transpose()
}

fn dimension_entry<'py>(
    entity: &Bound<'py, PyAny>,
    transform: Option<glam::DMat4>,
) -> PyResult<Bound<'py, PyDict>> {
    use glam::DVec3;
    let mapped = |pt: DVec3, vector: bool| {
        transform
            .map_or(pt, |m| {
                if vector {
                    m.transform_vector3(pt)
                } else {
                    m.transform_point3(pt)
                }
            })
            .to_array()
            .to_vec()
    };
    let entry = PyDict::new(entity.py());
    for field in ["a", "b"] {
        entry.set_item(
            field,
            mapped(
                DVec3::from_array(entity.getattr(field)?.extract()?) * 0.0254,
                false,
            ),
        )?;
    }
    entry.set_item(
        "text",
        optional::<String>(entity, "text")?.unwrap_or_default(),
    )?;
    let offset = optional::<f64>(entity, "offset")?.unwrap_or_default() * 0.0254;
    let x = optional::<[f64; 3]>(entity, "normal")?.map(DVec3::from_array);
    let n = optional::<[f64; 3]>(entity, "plane_x")?.map(DVec3::from_array);
    let direction = match (optional::<i32>(entity, "vff_axis")?, n, x) {
        (Some(1), Some(n), Some(x)) => Some(n.cross(x)),
        (Some(2), Some(_), Some(x)) => Some(-x),
        _ => None,
    };
    if let Some(v) = direction {
        entry.set_item("offset_vector", mapped(v * offset, true))?;
    } else {
        entry.set_item("offset", offset)?;
        if let Some(v) = x {
            entry.set_item("normal", mapped(v, true))?;
        }
    }
    Ok(entry)
}

fn text_entry<'py>(
    entity: &Bound<'py, PyAny>,
    transform: Option<glam::DMat4>,
) -> PyResult<Option<Bound<'py, PyDict>>> {
    use glam::DVec3;
    let point = optional::<[f64; 3]>(entity, "point")?;
    let label = optional::<[f64; 3]>(entity, "label_point")?;
    let Some(anchor) = point.or(label) else {
        return Ok(None);
    };
    let mapped = |p| {
        let pt = DVec3::from_array(p) * 0.0254;
        transform
            .map_or(pt, |m| m.transform_point3(pt))
            .to_array()
            .to_vec()
    };
    let entry = PyDict::new(entity.py());
    entry.set_item("anchor", mapped(anchor))?;
    entry.set_item(
        "text",
        optional::<String>(entity, "text")?
            .unwrap_or_default()
            .trim(),
    )?;
    entry.set_item(
        "hidden",
        entity
            .getattr_opt("hidden")?
            .map(|v| v.is_truthy())
            .transpose()?
            .unwrap_or(false),
    )?;
    if let (Some(_), Some(p)) = (point, label) {
        entry.set_item("label", mapped(p))?;
    }
    Ok(Some(entry))
}

pub(super) fn placed_matrix(m: &[f64]) -> PyResult<glam::DMat4> {
    if m.len() < 12 {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "incomplete instance transform",
        ));
    }
    let s = m.get(12).copied().filter(|v| *v != 0.).unwrap_or(1.);
    Ok(glam::DMat4::from_cols_array(&[
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
        m[9] * s * 0.0254,
        m[10] * s * 0.0254,
        m[11] * s * 0.0254,
        1.,
    ]))
}

#[pyfunction]
pub fn skp_instance_matrix(m: Vec<f64>) -> PyResult<crate::math::Matrix4> {
    Ok(placed_matrix(&m)?.into())
}

fn present(entity: &Bound<'_, PyAny>, name: &str) -> PyResult<bool> {
    Ok(entity.getattr_opt(name)?.is_some_and(|v| !v.is_none()))
}
fn marked(entity: &Bound<'_, PyAny>, name: &str) -> PyResult<bool> {
    entity
        .getattr_opt(name)?
        .map(|v| v.is_truthy())
        .transpose()
        .map(Option::unwrap_or_default)
}

#[pyfunction]
pub fn skp_model_annotations(
    model: &Bound<'_, PyAny>,
    payload: &Bound<'_, PyDict>,
) -> PyResult<()> {
    let py = model.py();
    let root = model.getattr_opt("root")?.filter(|v| !v.is_none());
    let dims = PyList::empty(py);
    let texts = PyList::empty(py);
    let endpoints =
        |v: &Bound<'_, PyAny>| -> PyResult<bool> { Ok(present(v, "a")? && present(v, "b")?) };
    let add_text = |v: &Bound<'_, PyAny>, matrix| -> PyResult<()> {
        if optional::<String>(v, "text")?.is_some_and(|t| !t.trim().is_empty()) {
            if let Some(row) = text_entry(v, matrix)? {
                texts.append(row)?;
            }
        }
        Ok(())
    };
    for dim in members(model, "dimensions")? {
        if endpoints(&dim)? {
            dims.append(dimension_entry(&dim, None)?)?;
        }
    }
    if let Some(root) = &root {
        for text in members(root, "texts")? {
            if present(&text, "point")? || marked(&text, "vff_recovered")? {
                add_text(&text, None)?;
            }
        }
    }
    let mut records = Vec::new();
    let indices = PyDict::new(py);
    let mut needs_placements = false;
    if let Some(defs) = model.getattr_opt("definitions")?.filter(|d| !d.is_none()) {
        for (key, def) in defs.cast::<PyDict>()? {
            let mut owned_dims = Vec::new();
            let mut owned_texts = Vec::new();
            for dim in members(&def, "dimensions")? {
                if marked(&dim, "vff_owned")? && endpoints(&dim)? {
                    owned_dims.push(dim);
                }
            }
            for tx in members(&def, "texts")? {
                if marked(&tx, "vff_recovered")? {
                    owned_texts.push(tx);
                }
            }
            needs_placements |= !owned_dims.is_empty() || !owned_texts.is_empty();
            indices.set_item(key, records.len())?;
            records.push((def, owned_dims, owned_texts));
        }
    }
    if let Some(root) = root.filter(|_| needs_placements) {
        let children = |def: &Bound<'_, PyAny>| -> PyResult<Vec<(usize, glam::DMat4)>> {
            let mut out = Vec::new();
            for inst in members(def, "instances")? {
                if let Some(id) = inst.getattr_opt("ref_idx")? {
                    if let (Some(index), Some(m)) = (
                        indices.get_item(id)?,
                        optional::<Vec<f64>>(&inst, "matrix")?,
                    ) {
                        if !m.is_empty() {
                            out.push((index.extract()?, placed_matrix(&m)?));
                        }
                    }
                }
            }
            Ok(out)
        };
        let root_places = children(&root)?;
        let graph = records
            .iter()
            .map(|(def, _, _)| children(def))
            .collect::<PyResult<Vec<_>>>()?;
        let mut worlds = vec![Vec::new(); records.len()];
        py.detach(|| {
            fn visit(
                id: usize,
                world: glam::DMat4,
                path: &mut Vec<usize>,
                graph: &[Vec<(usize, glam::DMat4)>],
                worlds: &mut [Vec<glam::DMat4>],
            ) {
                if path.len() > 32 || path.contains(&id) {
                    return;
                }
                worlds[id].push(world);
                path.push(id);
                for &(child, local) in &graph[id] {
                    visit(child, world * local, path, graph, worlds);
                }
                path.pop();
            }
            let mut path = vec![];
            for (id, w) in root_places {
                visit(id, w, &mut path, &graph, &mut worlds);
            }
        });
        for ((_, ds, ts), places) in records.into_iter().zip(worlds) {
            for w in places {
                for d in &ds {
                    dims.append(dimension_entry(d, Some(w))?)?;
                }
                for t in &ts {
                    add_text(t, Some(w))?;
                }
            }
        }
    }
    if !dims.is_empty() {
        payload.set_item("dimensions", dims)?;
    }
    if !texts.is_empty() {
        payload.set_item("texts", texts)?;
    }
    Ok(())
}
