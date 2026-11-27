// SPDX-License-Identifier: GPL-3.0-or-later
//! Per-entity records and column mapping for legacy IGZ and Built meshes.
use numpy::{ndarray::Array2, IntoPyArray};
use pyo3::{
    exceptions::{PyKeyError, PyValueError},
    prelude::*,
    types::{PyBool, PyDict, PyFloat, PyList},
};
use std::collections::HashMap;

type Obj<'py> = Bound<'py, PyAny>;
type Dict<'py> = Bound<'py, PyDict>;
fn empty(py: Python<'_>) -> Obj<'_> {
    PyList::empty(py).into_any()
}
fn list<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    v.py().get_type::<PyList>().call1((v,))
}
fn dict<'py>(v: &Obj<'py>) -> PyResult<Dict<'py>> {
    Ok(v.py().get_type::<PyDict>().call1((v,))?.cast_into()?)
}
fn field<'py>(row: &Dict<'py>, k: &str) -> PyResult<Obj<'py>> {
    row.get_item(k)?
        .ok_or_else(|| PyKeyError::new_err(k.to_owned()))
}
fn records<'py>(row: &Dict<'py>, k: &str) -> PyResult<Obj<'py>> {
    Ok(row.get_item(k)?.unwrap_or_else(|| empty(row.py())))
}

fn attrs<'py>(raw: &Obj<'py>, writing: bool) -> PyResult<Dict<'py>> {
    let py = raw.py();
    let out = PyDict::new(py);
    if raw.is_none() || !raw.is_truthy()? {
        return Ok(out);
    }
    // Key order is part of plain JSON output and its byte-parity tests.
    let keys = if writing {
        [
            "color", "texture", "pbr", "layer", "ifc", "opacity", "hidden", "mat", "back",
        ]
    } else {
        [
            "color", "pbr", "texture", "layer", "ifc", "opacity", "hidden", "mat", "back",
        ]
    };
    for k in keys {
        let v = if let Ok(d) = raw.cast::<PyDict>() {
            let Some(v) = d.get_item(k)? else { continue };
            v
        } else {
            raw.call_method1("get", (k,))?
        };
        if v.is_none() {
            continue;
        }
        match k {
            "color" => out.set_item(k, list(&v)?)?,
            "pbr" | "texture" => out.set_item(k, dict(&v)?)?,
            "ifc" if writing || v.is_truthy()? => out.set_item(k, dict(&v)?)?,
            "layer" if writing || v.is_truthy()? => out.set_item(k, v)?,
            "opacity" => out.set_item(k, py.get_type::<PyFloat>().call1((v,))?)?,
            "hidden" if v.is_truthy()? => out.set_item(k, true)?,
            "mat" if v.is_truthy()? => out.set_item(k, v)?,
            "back" if v.is_instance_of::<PyDict>() => {
                let copy = dict(&v)?;
                if writing {
                    for (key, value) in v.cast::<PyDict>()? {
                        if value.is_instance_of::<PyDict>() {
                            copy.set_item(key, dict(&value)?)?;
                        }
                    }
                }
                out.set_item(k, copy)?;
            }
            "back" if v.is(&PyBool::new(py, true)) => out.set_item(k, true)?,
            _ => {}
        }
    }
    Ok(out)
}
#[pyfunction]
fn igz_face_attrs_json<'py>(f: &Obj<'py>) -> PyResult<Dict<'py>> {
    attrs(
        &f.getattr("attrs")
            .unwrap_or_else(|_| f.py().None().into_bound(f.py())),
        true,
    )
}
#[pyfunction]
fn igz_face_attrs_from_json<'py>(raw: &Obj<'py>) -> PyResult<Option<Dict<'py>>> {
    let out = attrs(raw, false)?;
    Ok((!out.is_empty()).then_some(out))
}
fn position<'py>(v: &Obj<'py>) -> PyResult<Obj<'py>> {
    list(&v.getattr("position")?.call_method0("toTuple")?)
}
fn positions<'py>(vertices: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let out = PyList::empty(vertices.py());
    for v in vertices.try_iter()? {
        out.append(position(&v?)?)?;
    }
    Ok(out)
}
#[pyfunction]
fn igz_face_json<'py>(f: &Obj<'py>) -> PyResult<Dict<'py>> {
    let py = f.py();
    let out = PyDict::new(py);
    out.set_item("vertices", positions(&f.getattr("loop")?)?)?;
    if let Ok(loops) = f.getattr("hole_loops") {
        if loops.is_truthy()? {
            let holes = PyList::empty(py);
            for ring in loops.try_iter()? {
                holes.append(positions(&ring?)?)?;
            }
            out.set_item("holes", holes)?;
        }
    }
    out.call_method1("update", (igz_face_attrs_json(f)?,))?;
    Ok(out)
}
#[pyfunction]
fn igz_edge_json<'py>(e: &Obj<'py>) -> PyResult<Dict<'py>> {
    let out = PyDict::new(e.py());
    out.set_item("a", position(&e.getattr("v0")?)?)?;
    out.set_item("b", position(&e.getattr("v1")?)?)?;
    let flag = |k| e.getattr(k).map_or(Ok(false), |v| v.is_truthy());
    if flag("soft")? {
        out.set_item("soft", true)?;
        if flag("flat")? {
            out.set_item("flat", true)?;
        }
    }
    if flag("hidden")? {
        out.set_item("hidden", true)?;
    }
    for k in ["curve", "layer"] {
        if let Ok(v) = e.getattr(k) {
            if !v.is_none() {
                out.set_item(k, v)?;
            }
        }
    }
    Ok(out)
}

fn vectors<'py>(raw: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let out = PyList::empty(raw.py());
    for xyz in raw.try_iter()? {
        let v = crate::math::xyz(&xyz?)?;
        out.append(Py::new(raw.py(), crate::math::Vec3::from(v))?)?;
    }
    Ok(out)
}
fn bump_curve(module: &Bound<'_, PyModule>, cid: &Obj<'_>) -> PyResult<()> {
    if cid.ge(module.getattr("_CURVE_COUNTER")?)? {
        module.setattr("_CURVE_COUNTER", cid.call_method1("__add__", (1,))?)?;
    }
    Ok(())
}
#[pyfunction]
fn igz_load_mesh_small(mesh: &Obj<'_>, payload: &Dict<'_>) -> PyResult<()> {
    let py = mesh.py();
    let module = py.import("built_core.mesh")?;
    for raw in records(payload, "edges")?.try_iter()? {
        let raw = raw?.cast_into::<PyDict>()?;
        let point = |k| {
            Py::new(
                py,
                crate::math::Vec3::from(crate::math::xyz(&field(&raw, k)?)?),
            )
        };
        let edge = match mesh.call_method1("add_edge", (point("a")?, point("b")?)) {
            Ok(e) => e,
            Err(e) if e.is_instance_of::<PyValueError>(py) => continue,
            Err(e) => return Err(e),
        };
        for k in ["soft", "flat", "hidden", "layer"] {
            if k == "flat" && !raw.get_item("soft")?.map_or(Ok(false), |v| v.is_truthy())? {
                continue;
            }
            if let Some(v) = raw.get_item(k)? {
                if v.is_truthy()? {
                    if k == "layer" {
                        edge.setattr(k, v)?;
                    } else {
                        edge.setattr(k, true)?;
                    }
                }
            }
        }
        if let Some(cid) = raw.get_item("curve")?.filter(|v| !v.is_none()) {
            edge.setattr("curve", &cid)?;
            bump_curve(&module, &cid)?;
        }
    }
    for raw in records(payload, "faces")?.try_iter()? {
        let raw = raw?.cast_into::<PyDict>()?;
        let holes = PyList::empty(py);
        for ring in records(&raw, "holes")?.try_iter()? {
            holes.append(vectors(&ring?)?)?;
        }
        let face = mesh.call_method1("add_face", (vectors(&field(&raw, "vertices")?)?, holes))?;
        if let Some(style) = igz_face_attrs_from_json(&raw)? {
            face.getattr("attrs")?.call_method1("update", (style,))?;
        }
    }
    Ok(())
}

pub(crate) fn rings<'py>(row: &Dict<'py>) -> PyResult<Vec<Obj<'py>>> {
    let mut out = vec![field(row, "vertices")?];
    let holes = records(row, "holes")?;
    if holes.len()? <= 1 {
        for h in holes.try_iter()? {
            out.push(h?);
        }
    } else {
        let py = row.py();
        let loops = PyList::empty(py);
        for h in holes.try_iter()? {
            loops.append(vectors(&h?)?)?;
        }
        let kept = py
            .import("built_core.topology")?
            .call_method1("_maximal_holes", (loops,))?;
        for ring in kept.try_iter()? {
            let coords = PyList::empty(py);
            for v in ring?.try_iter()? {
                coords.append(v?.call_method0("toTuple")?)?;
            }
            out.push(coords.into_any());
        }
    }
    Ok(out)
}
#[pyfunction]
fn igz_json_arrays<'py>(payload: &Dict<'py>) -> PyResult<Option<Dict<'py>>> {
    let py = payload.py();
    let edges = records(payload, "edges")?;
    let faces = records(payload, "faces")?;
    let ne = edges.len()?;
    let nf = faces.len()?;
    let small = py
        .import("built_core.formats.igz")?
        .call_method1("_bulk_sized", (ne, nf))?
        .is_truthy()?
        == false;
    let mut points = Vec::new();
    let mut flags = Vec::new();
    let mut curves = Vec::new();
    let mut layers = Vec::new();
    let names = PyDict::new(py);
    for edge in edges.try_iter()? {
        let row = edge?.cast_into::<PyDict>()?;
        for k in ["a", "b"] {
            points.extend(crate::math::xyz(&field(&row, k)?)?.to_array());
        }
        let flag = |k| row.get_item(k)?.map_or(Ok(false), |v| v.is_truthy());
        let soft = flag("soft")?;
        flags.push(
            soft as u8 | ((flag("hidden")? as u8) << 1) | (((soft && flag("flat")?) as u8) << 2),
        );
        curves.push(match row.get_item("curve")?.filter(|v| !v.is_none()) {
            Some(v) => py
                .import("builtins")?
                .call_method1("int", (v,))?
                .extract::<i64>()?,
            None => -1,
        });
        let layer = match row.get_item("layer")? {
            Some(v) if !v.is_none() && (!small || v.is_truthy()?) => names
                .call_method1("setdefault", (v, names.len()))?
                .extract::<i32>()?,
            _ => -1,
        };
        layers.push(layer);
    }
    let mut sizes = Vec::new();
    let mut counts = Vec::new();
    let mut indices = Vec::new();
    let table = PyList::empty(py);
    let mut keys = HashMap::new();
    let options = PyDict::new(py);
    options.set_item("sort_keys", true)?;
    options.set_item("default", py.import("builtins")?.getattr("str")?)?;
    let dumps = py.import("json")?.getattr("dumps")?;
    for face in faces.try_iter()? {
        let row = face?.cast_into::<PyDict>()?;
        let loops = rings(&row)?;
        counts.push(loops.len() as i64);
        for ring in loops {
            sizes.push(ring.len()? as i64);
            for xyz in ring.try_iter()? {
                points.extend(crate::math::xyz(&xyz?)?.to_array());
            }
        }
        let mut index = -1;
        if igz_face_attrs_from_json(&row)?.is_some() {
            let raw = row.copy()?;
            raw.del_item("vertices")?;
            if raw.contains("holes")? {
                raw.del_item("holes")?;
            }
            let key = dumps.call((&raw,), Some(&options))?.extract::<String>()?;
            index = *keys.entry(key).or_insert(table.len() as i64);
            if index == table.len() as i64 {
                table.append(raw)?;
            }
        }
        indices.push(index);
    }
    if points.is_empty() {
        return Ok(None);
    }
    let n = points.len() / 3;
    let out = PyDict::new(py);
    let meta = PyDict::new(py);
    meta.set_item("face_attrs", table)?;
    if !names.is_empty() {
        meta.set_item("edge_layers", names.keys())?;
    }
    out.set_item("meta", meta)?;
    out.set_item(
        "pos",
        Array2::from_shape_vec((n, 3), points)
            .unwrap()
            .into_pyarray(py),
    )?;
    out.set_item(
        "edges",
        Array2::from_shape_fn((ne, 2), |(i, j)| (2 * i + j) as i64).into_pyarray(py),
    )?;
    out.set_item("flags", flags.into_pyarray(py))?;
    out.set_item(
        "curves",
        curves
            .iter()
            .any(|i| *i >= 0)
            .then(|| curves.into_pyarray(py)),
    )?;
    out.set_item(
        "layers",
        (!names.is_empty()).then(|| layers.into_pyarray(py)),
    )?;
    out.set_item(
        "rings",
        ((2 * ne) as i64..n as i64)
            .collect::<Vec<_>>()
            .into_pyarray(py),
    )?;
    out.set_item("sizes", sizes.into_pyarray(py))?;
    out.set_item("counts", counts.into_pyarray(py))?;
    out.set_item("face_attr", indices.into_pyarray(py))?;
    out.set_item("sequential", small)?;
    Ok(Some(out))
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(igz_pack_geometry, m)?)?;
    m.add_function(wrap_pyfunction!(igz_plan_json, m)?)?;
    m.add_function(wrap_pyfunction!(igz_payload_from_arrays, m)?)?;
    m.add_function(wrap_pyfunction!(igz_mesh_json, m)?)?;
    m.add_function(wrap_pyfunction!(igz_face_json, m)?)?;
    m.add_function(wrap_pyfunction!(igz_edge_json, m)?)?;
    m.add_function(wrap_pyfunction!(igz_face_attrs_json, m)?)?;
    m.add_function(wrap_pyfunction!(igz_face_attrs_from_json, m)?)?;
    m.add_function(wrap_pyfunction!(igz_load_mesh_small, m)?)?;
    m.add_function(wrap_pyfunction!(igz_json_arrays, m)?)?;
    Ok(())
}

fn array_list<'py>(d: &Obj<'py>, k: &str) -> PyResult<Obj<'py>> {
    d.get_item(k)?.call_method0("tolist")
}
fn optional_list<'py>(d: &Obj<'py>, k: &str) -> PyResult<Option<Obj<'py>>> {
    let v = d.get_item(k)?;
    if v.is_none() {
        Ok(None)
    } else {
        Ok(Some(v.call_method0("tolist")?))
    }
}
struct MeshRecords<'py> {
    pos: Obj<'py>,
    edges: Obj<'py>,
    flags: Vec<u8>,
    curves: Option<Obj<'py>>,
    layers: Option<Obj<'py>>,
    names: Option<Obj<'py>>,
    corners: Vec<usize>,
    sizes: Vec<usize>,
    counts: Vec<usize>,
    indices: Vec<i64>,
    table: Obj<'py>,
}
impl<'py> MeshRecords<'py> {
    fn payload(&self, plan: bool) -> PyResult<Dict<'py>> {
        let py = self.pos.py();
        let edges = PyList::empty(py);
        let faces = PyList::empty(py);
        let point = |i: usize| -> PyResult<Obj<'py>> {
            let v = self.pos.get_item(i)?;
            if plan {
                list(&v)
            } else {
                Ok(v)
            }
        };
        for (i, pair) in self.edges.try_iter()?.enumerate() {
            let [a, b] = pair?.extract::<[usize; 2]>()?;
            let row = PyDict::new(py);
            row.set_item("a", point(a)?)?;
            row.set_item("b", point(b)?)?;
            let f = *self
                .flags
                .get(i)
                .ok_or_else(|| PyValueError::new_err("missing edge flags"))?;
            if f & 1 != 0 {
                row.set_item("soft", true)?;
                if f & 4 != 0 {
                    row.set_item("flat", true)?;
                }
            }
            if f & 2 != 0 {
                row.set_item("hidden", true)?;
            }
            if let Some(curves) = &self.curves {
                let c = curves.get_item(i)?;
                if !c.is_none() && (plan || c.ge(0)?) {
                    row.set_item("curve", c)?;
                }
            }
            if let Some(layers) = &self.layers {
                let l = layers.get_item(i)?;
                if plan {
                    if !l.is_none() {
                        row.set_item("layer", l)?;
                    }
                } else if l.ge(0)? {
                    row.set_item(
                        "layer",
                        self.names
                            .as_ref()
                            .ok_or_else(|| PyValueError::new_err("missing edge layers"))?
                            .get_item(l)?,
                    )?;
                }
            }
            edges.append(row)?;
        }
        let mut ring = 0;
        let mut corner = 0usize;
        for (&count, &idx) in self.counts.iter().zip(&self.indices) {
            let row = if !plan && idx >= 0 {
                dict(&self.table.get_item(idx)?)?
            } else {
                PyDict::new(py)
            };
            let holes = PyList::empty(py);
            for j in 0..count {
                let size = *self
                    .sizes
                    .get(ring)
                    .ok_or_else(|| PyValueError::new_err("missing ring size"))?;
                ring += 1;
                let end = corner
                    .checked_add(size)
                    .ok_or_else(|| PyValueError::new_err("invalid ring size"))?;
                let slice = self
                    .corners
                    .get(corner..end)
                    .ok_or_else(|| PyValueError::new_err("missing ring corners"))?;
                let points = PyList::empty(py);
                for &i in slice {
                    points.append(point(i)?)?;
                }
                if j == 0 {
                    row.set_item("vertices", points)?;
                } else {
                    holes.append(points)?;
                }
                corner = end;
            }
            if !holes.is_empty() {
                row.set_item("holes", holes)?;
            }
            if plan && idx >= 0 {
                if let Some(read) = igz_face_attrs_from_json(&self.table.get_item(idx)?)? {
                    row.call_method1("update", (attrs(&read, true)?,))?;
                }
            }
            faces.append(row)?;
        }
        let out = PyDict::new(py);
        out.set_item("edges", edges)?;
        out.set_item("faces", faces)?;
        Ok(out)
    }
}
#[pyfunction]
fn igz_payload_from_arrays<'py>(src: &Obj<'py>) -> PyResult<Dict<'py>> {
    let meta = src.get_item("meta")?.cast_into::<PyDict>()?;
    MeshRecords {
        pos: array_list(src, "pos")?,
        edges: array_list(src, "edges")?,
        flags: array_list(src, "flags")?.extract()?,
        curves: optional_list(src, "curves")?,
        layers: optional_list(src, "layers")?,
        names: meta.get_item("edge_layers")?,
        corners: array_list(src, "rings")?.extract()?,
        sizes: array_list(src, "sizes")?.extract()?,
        counts: array_list(src, "counts")?.extract()?,
        indices: array_list(src, "face_attr")?.extract()?,
        table: records(&meta, "face_attrs")?,
    }
    .payload(false)
}
#[pyfunction]
fn igz_plan_json<'py>(mesh: &Obj<'py>, plan: &Obj<'py>) -> PyResult<Dict<'py>> {
    let src = mesh.getattr("__dict__")?.get_item("_deferred_source")?;
    let meta = src.get_item("meta")?.cast_into::<PyDict>()?;
    let values = |k| plan.getattr(k)?.call_method0("tolist");
    let soft = values("soft")?.extract::<Vec<bool>>()?;
    let hidden = values("hidden")?.extract::<Vec<bool>>()?;
    let flat = plan.getattr("flat")?;
    let flat = if flat.is_none() {
        vec![false; soft.len()]
    } else {
        flat.call_method0("tolist")?.extract()?
    };
    let flags = soft
        .into_iter()
        .zip(hidden)
        .zip(flat)
        .map(|((s, h), f)| s as u8 | (h as u8) << 1 | ((s && f) as u8) << 2)
        .collect();
    MeshRecords {
        pos: values("pos")?,
        edges: values("edges")?,
        flags,
        curves: Some(plan.getattr("curve")?),
        layers: Some(plan.getattr("layer")?),
        names: None,
        corners: values("corners")?.extract()?,
        sizes: values("ring_sizes")?.extract()?,
        counts: values("ring_counts")?.extract()?,
        indices: array_list(&src, "face_attr")?.extract()?,
        table: records(&meta, "face_attrs")?,
    }
    .payload(true)
}
// Read live vertices, ring indices and edge flags in store order. Overlay
// mutable position vectors and painted face dictionaries held by scripts.
fn plain_store_records<'py>(mesh: &Obj<'py>) -> PyResult<Option<Dict<'py>>> {
    use crate::store::{
        keys::{Kind, LOCAL_BITS},
        py::PyMeshStore,
    };
    let py = mesh.py();
    let module = py.import("built_core.store_mesh")?;
    if !mesh.get_type().is(&module.getattr("StoreMesh")?) {
        return Ok(None);
    }
    let md = mesh.getattr("__dict__")?.cast_into::<PyDict>()?;
    if md
        .get_item("_instance_definition")?
        .is_some_and(|v| !v.is_none())
    {
        return Ok(None);
    }
    let store = field(&md, "_store")?;
    if !store.getattr("_model_frame")?.is_truthy()? {
        return Ok(None);
    }
    let native = store.cast::<PyMeshStore>()?.try_borrow()?;
    let st = &native.inner;
    let mut d = st.dense();
    let base = (st.state.ns as u128) << LOCAL_BITS;
    if let Some(c) = md.get_item("_pos_cache")?.filter(|v| !v.is_none()) {
        if c.get_item(0)?.eq(st.revision)? && c.get_item(2)?.is(&store) {
            let cache = c.get_item(1)?.cast_into::<PyDict>()?;
            if !cache.is_empty() {
                for (p, k) in d.pos.iter_mut().zip(st.local_keys(Kind::Vertex)) {
                    if let Some(v) = cache.get_item(base | u128::from(k))? {
                        *p = crate::math::xyz(&v)?.to_array();
                    }
                }
            }
        }
    }
    let edges = PyList::empty(py);
    for (i, pair) in d.edges.iter().enumerate() {
        let e = PyDict::new(py);
        e.set_item("a", d.pos[pair[0] as usize])?;
        e.set_item("b", d.pos[pair[1] as usize])?;
        let flags = d.flags[i];
        if flags & crate::store::SOFT != 0 {
            e.set_item("soft", true)?;
            if flags & crate::store::FLAT != 0 {
                e.set_item("flat", true)?
            }
        }
        if flags & crate::store::HIDDEN != 0 {
            e.set_item("hidden", true)?
        }
        if d.curve[i] >= 0 {
            e.set_item("curve", d.curve[i])?
        }
        if d.layer[i] >= 0 {
            e.set_item("layer", &st.state.layers[d.layer[i] as usize])?
        }
        edges.append(e)?;
    }
    let own = field(&md, "_attr_own")?.cast_into::<PyDict>()?;
    let table = field(&md, "_attr_table")?;
    let keys = (!own.is_empty()).then(|| st.local_keys(Kind::Face));
    let faces = PyList::empty(py);
    let mut rings = d.ring_sizes.iter();
    let mut corner = 0;
    for (i, &count) in d.ring_counts.iter().enumerate() {
        let f = PyDict::new(py);
        let holes = PyList::empty(py);
        for r in 0..count {
            let n = *rings.next().unwrap() as usize;
            let pts = PyList::empty(py);
            for &p in &d.corners[corner..corner + n] {
                pts.append(d.pos[p as usize])?
            }
            corner += n;
            if r == 0 {
                f.set_item("vertices", pts)?
            } else {
                holes.append(pts)?
            }
        }
        if !holes.is_empty() {
            f.set_item("holes", holes)?
        }
        let style = if let Some(keys) = &keys {
            own.get_item(base | u128::from(keys[i]))?
        } else {
            None
        };
        let style = match style {
            None if d.face_attr[i] >= 0 => Some(table.get_item(d.face_attr[i])?),
            other => other,
        };
        if let Some(v) = style {
            f.update(attrs(&v, true)?.as_mapping())?
        }
        faces.append(f)?;
    }
    let result = PyDict::new(py);
    result.set_item("edges", edges)?;
    result.set_item("faces", faces)?;
    Ok(Some(result))
}
#[pyfunction]
#[pyo3(signature=(mesh,geom_sink=None))]
fn igz_mesh_json<'py>(mesh: &Obj<'py>, geom_sink: Option<&Obj<'py>>) -> PyResult<Dict<'py>> {
    let py = mesh.py();
    let plan = py
        .import("built_core")?
        .call_method1("plan_of", (mesh,))?;
    let codec = py.import("built_core.formats.igz")?;
    let out = PyDict::new(py);
    if !plan.is_none() && geom_sink.is_none() {
        return igz_plan_json(mesh, &plan);
    }
    if geom_sink.is_none() {
        if let Some(records) = plain_store_records(mesh)? {
            return Ok(records);
        }
    }
    let edges = if plan.is_none() {
        mesh.getattr("edges")?
    } else {
        plan.getattr("edges")?
    };
    let ne = edges.len()?;
    let nf = if plan.is_none() {
        mesh.getattr("faces")?.len()?
    } else {
        plan.getattr("nfaces")?.extract()?
    };
    if let Some(sink) = geom_sink.filter(|_| !plan.is_none() || ne > 0 || nf > 0) {
        let meta = if plan.is_none() {
            codec.call_method1("_pack_geometry", (mesh, sink))?
        } else {
            codec.call_method1("_pack_plan", (mesh, plan, sink))?
        };
        if !codec.call_method1("_bulk_sized", (ne, nf))?.is_truthy()? {
            meta.set_item("sequential", true)?;
        }
        out.set_item("geom", meta)?;
    } else {
        let es = PyList::empty(py);
        let fs = PyList::empty(py);
        for e in edges.try_iter()? {
            es.append(igz_edge_json(&e?)?)?;
        }
        for f in mesh.getattr("faces")?.try_iter()? {
            fs.append(igz_face_json(&f?)?)?;
        }
        out.set_item("edges", es)?;
        out.set_item("faces", fs)?;
    }
    Ok(out)
}

pub(crate) struct Vertices {
    precise: bool,
    indices: HashMap<u128, i32>,
    pos: Vec<f64>,
    /// Keys in index order (their persistent ids go with the block).
    pub(crate) keys: Vec<u128>,
}
impl Vertices {
    pub(crate) fn new(precise: bool) -> Self {
        Self {
            precise,
            indices: HashMap::new(),
            pos: Vec::new(),
            keys: Vec::new(),
        }
    }
    pub(crate) fn positions(self) -> Vec<f64> {
        self.pos
    }
    pub(crate) fn index(&mut self, v: &Obj<'_>) -> PyResult<i32> {
        let key = v.getattr("key")?.extract()?;
        if let Some(i) = self.indices.get(&key) {
            return Ok(*i);
        }
        let index = i32::try_from(self.indices.len())
            .map_err(|_| PyValueError::new_err("too many mesh vertices"))?;
        let point = if self.precise && v.hasattr("position_tuple")? {
            v.call_method0("position_tuple")?
        } else {
            v.getattr("position")?
        };
        self.pos.extend(crate::math::xyz(&point)?.to_array());
        self.indices.insert(key, index);
        self.keys.push(key);
        Ok(index)
    }
}
/// The persistent ids of elements written element by element, by key,
/// when the mesh has a store (`vertex_id` / `edge_id` / `face_id`).
pub(crate) fn object_ids<'py>(
    mesh: &Obj<'py>,
    keys: [&[u128]; 3],
    arrays: &Bound<'py, PyDict>,
) -> PyResult<()> {
    let Some(store) = mesh.getattr_opt("_store")?.filter(|s| !s.is_none()) else {
        return Ok(());
    };
    if !store.hasattr("ids_of")? {
        return Ok(());
    }
    for (k, keys) in ["vertex", "edge", "face"].iter().zip(keys) {
        let ids: Vec<u64> = match store.call_method1("ids_of", (*k, keys.to_vec())) {
            Ok(v) => v.extract()?,
            Err(_) => return Ok(()),
        };
        arrays.set_item(format!("{k}_id"), numpy::IntoPyArray::into_pyarray(ids, mesh.py()))?;
    }
    Ok(())
}
pub(crate) fn maximal_vertex_holes<'py>(holes: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    let py = holes.py();
    let originals: Vec<_> = holes.try_iter()?.collect::<PyResult<_>>()?;
    if originals.len() < 2 {
        return Ok(originals);
    }
    let positions = PyList::empty(py);
    for ring in &originals {
        let vs = PyList::empty(py);
        for v in ring.try_iter()? {
            vs.append(v?.getattr("position")?)?;
        }
        positions.append(vs)?;
    }
    let kept = py
        .import("built_core.topology")?
        .call_method1("_maximal_holes", (&positions,))?;
    let ids = kept
        .try_iter()?
        .map(|r| Ok(r?.as_ptr() as usize))
        .collect::<PyResult<std::collections::HashSet<_>>>()?;
    Ok(originals
        .into_iter()
        .zip(positions.iter())
        .filter(|(_, r)| ids.contains(&(r.as_ptr() as usize)))
        .map(|(r, _)| r)
        .collect())
}
#[pyfunction]
fn igz_pack_geometry<'py>(mesh: &Obj<'py>, sink: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = mesh.py();
    let codec = py.import("built_core.formats.igz")?;
    let precise = ["built", "compatibility"]
        .iter()
        .try_fold(false, |flag, k| -> PyResult<_> {
            Ok(flag || sink.getattr(*k).map_or(Ok(false), |v| v.is_truthy())?)
        })?;
    let packed = codec.call_method1("_store_columns", (mesh, precise))?;
    if !packed.is_none() {
        return codec.call_method1(
            "_write_geometry",
            (
                packed.get_item(0)?,
                packed.get_item(1)?,
                packed.get_item(2)?,
                sink,
            ),
        );
    }
    let mut vertices = Vertices::new(precise);
    let mut edges = Vec::new();
    let mut flags = Vec::new();
    let mut curves = Vec::new();
    let mut layers = Vec::new();
    let layer_table = PyDict::new(py);
    let mut has_curves = false;
    let mut edge_keys: Vec<u128> = Vec::new();
    let mut face_keys: Vec<u128> = Vec::new();
    for edge in mesh.getattr("edges")?.try_iter()? {
        let edge = edge?;
        edge_keys.push(edge.getattr("key")?.extract()?);
        edges.push(vertices.index(&edge.getattr("v0")?)?);
        edges.push(vertices.index(&edge.getattr("v1")?)?);
        let s = edge.getattr("soft")?.is_truthy()?;
        let f = s && edge.getattr("flat")?.is_truthy()?;
        flags
            .push(s as u8 | ((edge.getattr("hidden")?.is_truthy()? as u8) << 1) | ((f as u8) << 2));
        let cid = edge.getattr("curve")?;
        curves.push(if cid.is_none() {
            -1i64
        } else {
            has_curves = true;
            cid.extract()?
        });
        let layer = edge.getattr("layer")?;
        layers.push(if layer.is_none() {
            -1i32
        } else {
            layer_table
                .call_method1("setdefault", (layer, layer_table.len()))?
                .extract()?
        });
    }
    let mut rings = Vec::new();
    let mut sizes = Vec::new();
    let mut counts = Vec::new();
    let mut fa = Vec::new();
    let table = PyList::empty(py);
    let mut indices = HashMap::new();
    let key_of = codec.getattr("_attrs_key")?;
    for face in mesh.getattr("faces")?.try_iter()? {
        let face = face?;
        face_keys.push(face.getattr("key")?.extract()?);
        let mut loops = vec![face.getattr("loop")?];
        let holes = face.getattr("hole_loops")?;
        if holes.is_truthy()? {
            loops.extend(maximal_vertex_holes(&holes)?);
        }
        counts.push(loops.len() as i32);
        for r in loops {
            sizes.push(r.len()? as i32);
            for v in r.try_iter()? {
                rings.push(vertices.index(&v?)?);
            }
        }
        let attrs = igz_face_attrs_json(&face)?;
        if attrs.is_empty() {
            fa.push(-1i32);
        } else {
            let key = key_of.call1((&attrs,))?.extract::<String>()?;
            let i = *indices.entry(key).or_insert(table.len() as i32);
            if i == table.len() as i32 {
                table.append(attrs)?;
            }
            fa.push(i);
        }
    }
    let n = vertices.pos.len() / 3;
    let ne = edges.len() / 2;
    let arrays = PyDict::new(py);
    object_ids(mesh, [&vertices.keys, &edge_keys, &face_keys], &arrays)?;
    let pos = Array2::from_shape_vec((n, 3), vertices.pos).unwrap();
    if precise {
        arrays.set_item("pos", pos.into_pyarray(py))?;
    } else {
        arrays.set_item("pos", pos.mapv(|v| v as f32).into_pyarray(py))?;
    }
    arrays.set_item(
        "edges",
        Array2::from_shape_vec((ne, 2), edges)
            .unwrap()
            .into_pyarray(py),
    )?;
    arrays.set_item("rings", rings.into_pyarray(py))?;
    arrays.set_item("ring_sizes", sizes.into_pyarray(py))?;
    arrays.set_item("ring_counts", counts.into_pyarray(py))?;
    arrays.set_item("face_attr", fa.into_pyarray(py))?;
    arrays.set_item("edge_flags", flags.into_pyarray(py))?;
    if has_curves {
        arrays.set_item("edge_curve", curves.into_pyarray(py))?;
    }
    if !layer_table.is_empty() {
        arrays.set_item("edge_layer", layers.into_pyarray(py))?;
    }
    codec.call_method1("_write_geometry", (arrays, table, layer_table.keys(), sink))
}
