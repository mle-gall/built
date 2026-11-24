// SPDX-License-Identifier: GPL-3.0-or-later
//! Archive columns and deferred mesh packing. Vertex, material and layer
//! indices follow first use, including loose edges before face rings.
use numpy::{ndarray::Array2, IntoPyArray, PyReadonlyArray1};
use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict, PyList},
};
use std::collections::{HashMap, HashSet};
type Obj<'p> = Bound<'p, PyAny>;
fn api(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("formats.igz")
}
fn column<'p>(value: &Obj<'p>) -> PyResult<PyReadonlyArray1<'p, i64>> {
    Ok(value
        .py()
        .import("numpy")?
        .call_method1("ascontiguousarray", (value, "int64"))?
        .call_method1("reshape", (-1,))?
        .extract()?)
}
/// Map original face table indices to distinct, serializable face paint in
/// first-use order. A touched store face can own a row equal to another row.
fn face_table<'p>(
    rows: &Obj<'p>,
    attrs: &Obj<'p>,
    from_file: bool,
) -> PyResult<(Bound<'p, PyList>, Vec<i32>)> {
    let py = rows.py();
    let codec = api(py)?;
    let table = PyList::empty(py);
    let keys = PyDict::new(py);
    let mut seen = HashMap::<i64, i32>::new();
    let wrapper = py.import("types")?.getattr("SimpleNamespace")?;
    let rows = column(rows)?;
    let rows = rows.as_slice()?;
    let mut output = Vec::with_capacity(rows.len());
    for &id in rows {
        let index = match seen.entry(id) {
            std::collections::hash_map::Entry::Occupied(e) => *e.get(),
            std::collections::hash_map::Entry::Vacant(e) => {
                let json = if id < 0 {
                    None
                } else {
                    let raw = attrs.get_item(id)?;
                    let paint = if from_file {
                        codec.call_method1("_face_attrs_from_json", (raw,))?
                    } else {
                        raw
                    };
                    if paint.is_truthy()? {
                        let simple = wrapper.call0()?;
                        simple.setattr("attrs", paint)?;
                        Some(codec.call_method1("_face_attrs_json", (simple,))?)
                    } else {
                        None
                    }
                };
                let i = if let Some(j) = json.filter(|v| v.is_truthy().unwrap_or(false)) {
                    let key = codec.call_method1("_attrs_key", (&j,))?;
                    if let Some(i) = keys.get_item(&key)? {
                        i.extract()?
                    } else {
                        let i = table.len() as i32;
                        table.append(j)?;
                        keys.set_item(key, i)?;
                        i
                    }
                } else {
                    -1
                };
                *e.insert(i)
            }
        };
        output.push(index);
    }
    Ok((table, output))
}
#[pyfunction]
#[pyo3(signature=(mesh,precise=false))]
fn igz_store_columns<'p>(mesh: &Obj<'p>, precise: bool) -> PyResult<Option<Obj<'p>>> {
    let py = mesh.py();
    let store = py.import("core.store_mesh")?;
    if !mesh.get_type().is(&store.getattr("StoreMesh")?)
        || api(py)?.call_method0("native_io")?.is_none()
    {
        return Ok(None);
    }
    let d = mesh.getattr("__dict__")?;
    if !d.call_method1("get", ("_instance_definition",))?.is_none()
        || !precise && d.call_method1("get", ("_pos_cache",))?.is_truthy()?
    {
        return Ok(None);
    }
    let p = mesh.getattr("_store")?.call_method0("igz_columns")?;
    if p.is_none() {
        return Ok(None);
    }
    let arrays = p.get_item(0)?;
    let layers = p.get_item(1)?;
    let kw = PyDict::new(py);
    kw.set_item("face_attr", arrays.get_item("face_attr")?)?;
    let (attrs, rows): (Obj<'p>, Obj<'p>) = store
        .call_method("attr_columns", (mesh,), Some(&kw))?
        .extract()?;
    let (table, indices) = face_table(&rows, &attrs, false)?;
    arrays.set_item("face_attr", indices.into_pyarray(py))?;
    Ok(Some((arrays, table, layers).into_pyobject(py)?.into_any()))
}
#[pyfunction]
fn igz_write_geometry<'p>(
    arrays: &Bound<'p, PyDict>,
    table: &Obj<'p>,
    layer_table: &Obj<'p>,
    sink: &Obj<'p>,
) -> PyResult<Bound<'p, PyDict>> {
    let py = arrays.py();
    let np = py.import("numpy")?;
    let codec = api(py)?;
    let builtin = py.import("builtins")?;
    arrays.set_item(
        "pos",
        py.import("formats.built")?
            .call_method1("point_array", (arrays.as_any().get_item("pos")?, sink))?,
    )?;
    let layout = PyDict::new(py);
    let chunks = PyList::empty(py);
    let mut offset = 0usize;
    for (k, a) in arrays.iter() {
        let a = np.call_method1("ascontiguousarray", (a,))?;
        let bytes = builtin
            .call_method1("memoryview", (a.call_method1("reshape", (-1,))?,))?
            .call_method1("cast", ("B",))?;
        let spec = PyList::empty(py);
        spec.append(a.getattr("dtype")?.getattr("str")?)?;
        spec.append(builtin.call_method1("list", (a.getattr("shape")?,))?)?;
        spec.append(offset)?;
        layout.set_item(k, spec)?;
        let size = bytes.len()?;
        let pad = (8 - size % 8) % 8;
        chunks.append(bytes)?;
        chunks.append(PyBytes::new(py, &[0; 8][..pad]))?;
        offset = offset
            .checked_add(size)
            .and_then(|n| n.checked_add(pad))
            .ok_or_else(|| {
                pyo3::exceptions::PyOverflowError::new_err("geometry exceeds address space")
            })?;
    }
    let member = format!(
        "{}{}.bin",
        codec.getattr("_GEOM_PREFIX")?.extract::<String>()?,
        sink.len()?
    );
    let data = if sink.is_instance(&codec.getattr("SaveStream")?)? {
        sink.call_method1("store", (chunks,))?
    } else {
        PyBytes::new(py, b"").call_method1("join", (chunks,))?
    };
    sink.set_item(&member, data)?;
    let meta = PyDict::new(py);
    meta.set_item("member", member)?;
    meta.set_item("arrays", layout)?;
    if table.is_truthy()? {
        meta.set_item("face_attrs", table)?
    }
    if layer_table.is_truthy()? {
        meta.set_item("edge_layers", layer_table)?
    }
    Ok(meta)
}
fn kept_loops<'p>(
    plan: &Obj<'p>,
    positions: &Obj<'p>,
    corners: &[i64],
    sizes: &[i64],
    counts: &[i64],
) -> PyResult<Vec<bool>> {
    let py = plan.py();
    let extra = plan.getattr("extra")?;
    let saved = extra.call_method1("get", ("keep_ring",))?;
    if !saved.is_none() {
        return saved.call_method0("tolist")?.extract();
    }
    let mut keep = vec![true; sizes.len()];
    let mut ring = 0;
    let mut corner = 0;
    for &nr in counts {
        if nr > 2 {
            let holes = PyList::empty(py);
            let mut offset = corner + sizes[ring] as usize;
            for j in 1..nr as usize {
                let n = sizes[ring + j] as usize;
                let points = PyList::empty(py);
                for &vi in &corners[offset..offset + n] {
                    let pt = positions
                        .get_item(vi)?
                        .call_method0("tolist")?
                        .extract::<[f64; 3]>()?;
                    points.append(crate::math::Vec3::from(glam::DVec3::from_array(pt)))?;
                }
                holes.append(points)?;
                offset += n
            }
            let chosen = py
                .import("core.topology")?
                .call_method1("_maximal_holes", (&holes,))?;
            let ids = chosen
                .try_iter()?
                .map(|h| Ok(h?.as_ptr() as usize))
                .collect::<PyResult<HashSet<_>>>()?;
            for (j, h) in holes.iter().enumerate() {
                keep[ring + 1 + j] = ids.contains(&(h.as_ptr() as usize))
            }
        }
        for &n in &sizes[ring..ring + nr as usize] {
            corner += n as usize
        }
        ring += nr as usize;
    }
    extra.set_item("keep_ring", keep.clone().into_pyarray(py))?;
    Ok(keep)
}
#[pyfunction]
fn igz_pack_plan<'p>(mesh: &Obj<'p>, plan: &Obj<'p>, sink: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = mesh.py();
    let np = py.import("numpy")?;
    let codec = api(py)?;
    let src = mesh.getattr("__dict__")?.get_item("_deferred_source")?;
    let points = plan.getattr("pos")?;
    let edges = column(&plan.getattr("edges")?)?;
    let corners = column(&plan.getattr("corners")?)?;
    let sizes = column(&plan.getattr("ring_sizes")?)?;
    let counts = column(&plan.getattr("ring_counts")?)?;
    let (edges, corners, sizes, counts) = (
        edges.as_slice()?,
        corners.as_slice()?,
        sizes.as_slice()?,
        counts.as_slice()?,
    );
    // These plans are also public script inputs. Check boundaries before
    // slicing them to reconstruct their loops.
    let total = |a: &[i64]| {
        a.iter().try_fold(0usize, |sum, &v| {
            usize::try_from(v).ok().and_then(|v| sum.checked_add(v))
        })
    };
    if total(sizes) != Some(corners.len())
        || total(counts) != Some(sizes.len())
        || edges.len() % 2 != 0
    {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "invalid deferred ring layout",
        ));
    }
    let keep = if counts.iter().any(|n| *n > 2) {
        Some(kept_loops(plan, &points, corners, sizes, counts)?)
    } else {
        None
    };
    if keep.as_ref().is_some_and(|k| k.len() != sizes.len()) {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "invalid deferred ring mask",
        ));
    }
    let mut order = vec![];
    let mut lookup = HashMap::new();
    let mut index = |id: i64| -> PyResult<i32> {
        if let Some(i) = lookup.get(&id) {
            return Ok(*i);
        }
        let i = i32::try_from(order.len())
            .map_err(|_| pyo3::exceptions::PyValueError::new_err("too many deferred vertices"))?;
        lookup.insert(id, i);
        order.push(id);
        Ok(i)
    };
    let edge_idx = edges
        .iter()
        .map(|&id| index(id))
        .collect::<PyResult<Vec<_>>>()?;
    let (mut rings, mut ring_sizes, mut ring_counts) = (vec![], vec![], vec![]);
    let (mut r, mut c) = (0, 0);
    for &n in counts {
        let mut retained = 0;
        for _ in 0..n {
            let size = sizes[r] as usize;
            if keep.as_ref().is_none_or(|k| k[r]) {
                ring_sizes.push(size as i32);
                retained += 1;
                for &v in &corners[c..c + size] {
                    rings.push(index(v)?)
                }
            }
            c += size;
            r += 1;
        }
        ring_counts.push(retained)
    }
    let pos = points
        .get_item(order.into_pyarray(py))?
        .call_method1("reshape", (-1, 3))?;
    if !np
        .call_method1("isfinite", (&pos,))?
        .call_method0("all")?
        .is_truthy()?
    {
        return Err(PyErr::from_value(
            codec.call_method1("NonFiniteDocumentError", ("geometry",))?,
        ));
    }
    let raw = src
        .get_item("meta")?
        .call_method1("get", ("face_attrs", PyList::empty(py)))?;
    let (table, fa) = face_table(&src.get_item("face_attr")?, &raw, true)?;
    let arrays = PyDict::new(py);
    arrays.set_item("pos", pos)?;
    arrays.set_item(
        "edges",
        Array2::from_shape_vec((edge_idx.len() / 2, 2), edge_idx)
            .unwrap()
            .into_pyarray(py),
    )?;
    arrays.set_item("rings", rings.into_pyarray(py))?;
    arrays.set_item("ring_sizes", ring_sizes.into_pyarray(py))?;
    arrays.set_item("ring_counts", ring_counts.into_pyarray(py))?;
    arrays.set_item("face_attr", fa.into_pyarray(py))?;
    let soft = plan.getattr("soft")?;
    let flat = plan.getattr("flat")?;
    let mut flags = soft.call_method1("astype", ("uint8",))?.call_method1(
        "__or__",
        (plan
            .getattr("hidden")?
            .call_method1("astype", ("uint8",))?
            .call_method1("__lshift__", (1,))?,),
    )?;
    if !flat.is_none() {
        flags = flags.call_method1(
            "__or__",
            (flat
                .call_method1("__and__", (&soft,))?
                .call_method1("astype", ("uint8",))?
                .call_method1("__lshift__", (2,))?,),
        )?
    }
    arrays.set_item("edge_flags", flags)?;
    let curves = plan.getattr("curve")?;
    let tags = plan.getattr("layer")?;
    let names = PyDict::new(py);
    if curves
        .call_method1("count", (py.None(),))?
        .extract::<usize>()?
        != curves.len()?
    {
        let v = curves
            .try_iter()?
            .map(|c| Ok(c?.extract::<Option<i64>>()?.unwrap_or(-1)))
            .collect::<PyResult<Vec<_>>>()?;
        arrays.set_item("edge_curve", v.into_pyarray(py))?;
    }
    if tags
        .call_method1("count", (py.None(),))?
        .extract::<usize>()?
        != tags.len()?
    {
        let mut v = Vec::with_capacity(tags.len()?);
        for tag in tags.try_iter()? {
            let tag = tag?;
            v.push(if tag.is_none() {
                -1i32
            } else {
                names
                    .call_method1("setdefault", (tag, names.len()))?
                    .extract()?
            })
        }
        arrays.set_item("edge_layer", v.into_pyarray(py))?;
    }
    codec.call_method1("_write_geometry", (arrays, table, names.keys(), sink))
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(igz_store_columns, m)?)?;
    m.add_function(wrap_pyfunction!(igz_write_geometry, m)?)?;
    m.add_function(wrap_pyfunction!(igz_pack_plan, m)?)
}
