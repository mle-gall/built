// SPDX-License-Identifier: GPL-3.0-or-later
//! Legacy JSON mesh records and format-3 blocks. Validated NumPy views keep
//! their owner alive while a deferred mesh waits for its first access.
use pyo3::{
    exceptions::{PyTypeError, PyValueError},
    prelude::*,
    types::{PyDict, PyInt, PyList, PySlice, PyString, PyTuple},
};
type Obj<'p> = Bound<'p, PyAny>;
fn api(py: Python<'_>) -> PyResult<Bound<'_, PyModule>> {
    py.import("ingetrazo_core.formats.igz")
}
fn get<'p>(d: &Obj<'p>, k: &str) -> PyResult<Obj<'p>> {
    d.call_method1("get", (k,))
}
fn rows<'p>(d: &Obj<'p>, k: &str) -> PyResult<Obj<'p>> {
    let v = get(d, k)?;
    Ok(if v.is_truthy()? {
        v
    } else {
        PyTuple::empty(d.py()).into_any()
    })
}
fn bad(k: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(format!("damaged geometry block ({k})"))
}
fn full(py: Python<'_>) -> Bound<'_, PySlice> {
    PySlice::full(py)
}
fn bump(module: &Bound<'_, PyModule>, curr: &Obj<'_>) -> PyResult<()> {
    let n = curr.py().import("builtins")?.call_method1("int", (curr,))?;
    if n.ge(module.getattr("_CURVE_COUNTER")?)? {
        module.setattr("_CURVE_COUNTER", n.add(1)?)?
    }
    Ok(())
}
fn repair(py: Python<'_>, skipped: usize) -> PyResult<()> {
    let count = api(py)?.getattr("_repair_count")?.call_method0("get")?;
    count.set_item(0, count.get_item(0)?.add(skipped)?)
}
#[pyfunction]
#[pyo3(signature=(meta,blob,checked=false,*,built_format=false))]
fn igz_geometry_arrays<'p>(
    meta: &Obj<'p>,
    blob: &Obj<'p>,
    checked: bool,
    built_format: bool,
) -> PyResult<Bound<'p, PyDict>> {
    let py = meta.py();
    let np = py.import("numpy")?;
    let int = py.import("builtins")?.getattr("int")?;
    let layout = meta
        .get_item("arrays")
        .and_then(|a| Ok(a.cast_into::<PyDict>()?))
        .map_err(|_| bad("no layout"))?;
    let out = PyDict::new(py);
    for (name, spec) in layout.iter() {
        let key = name.extract::<String>()?;
        let parsed = (|| -> PyResult<_> {
            if spec.len()? != 3 {
                return Err(bad(&key));
            }
            let dtype = spec.get_item(0)?.extract::<String>()?;
            if !matches!(dtype.as_str(), "<f4" | "<i4" | "<i8" | "|u1")
                && !(dtype == "<f8" && (checked || built_format && key == "pos"))
            {
                return Err(bad(&key));
            }
            let shape: Vec<usize> = spec
                .get_item(1)?
                .try_iter()?
                .map(|n| int.call1((n?,))?.extract())
                .collect::<PyResult<_>>()?;
            let count = shape
                .iter()
                .try_fold(1usize, |n, v| n.checked_mul(*v))
                .ok_or_else(|| bad(&key))?;
            let offset: usize = int.call1((spec.get_item(2)?,))?.extract()?;
            let width = match dtype.as_str() {
                "<f4" | "<i4" => 4,
                "<f8" | "<i8" => 8,
                _ => 1,
            };
            if count
                .checked_mul(width)
                .and_then(|n| n.checked_add(offset))
                .is_none_or(|end| end > blob.len().unwrap_or(0))
            {
                return Err(bad(&key));
            }
            np.call_method1("frombuffer", (blob, dtype, count, offset))?
                .call_method1("reshape", (PyTuple::new(py, shape)?,))
        })()
        .map_err(|_| bad(&key))?;
        out.set_item(name, parsed)?;
    }
    if checked {
        return Ok(out);
    }
    for k in [
        "pos",
        "edges",
        "rings",
        "ring_sizes",
        "ring_counts",
        "face_attr",
        "edge_flags",
    ] {
        if !out.contains(k)? {
            return Err(bad(format!("missing {k}")));
        }
    }
    let item = |k: &str| out.as_any().get_item(k);
    for (k, cols) in [("pos", 3), ("edges", 2)] {
        let a = item(k)?;
        let s: Vec<usize> = a.getattr("shape")?.extract()?;
        if s.len() != 2 || s[1] != cols {
            return Err(bad(k));
        }
    }
    let nv = item("pos")?.len()?;
    let ne = item("edges")?.len()?;
    let check_range = |k: &str, min: i64, max: usize| -> PyResult<()> {
        let a = item(k)?;
        if a.getattr("size")?.is_truthy()?
            && (a.call_method0("min")?.lt(min)? || a.call_method0("max")?.ge(max)?)
        {
            return Err(bad(k));
        }
        Ok(())
    };
    check_range("edges", 0, nv)?;
    check_range("rings", 0, nv)?;
    for (k, target) in [("ring_sizes", "rings"), ("ring_counts", "ring_sizes")] {
        let a = item(k)?;
        if a.getattr("size")?.is_truthy()? && a.call_method0("min")?.lt(1)?
            || int
                .call1((a.call_method0("sum")?,))?
                .ne(item(target)?.len()?)?
        {
            return Err(bad("rings"));
        }
    }
    if item("face_attr")?.len()? != item("ring_counts")?.len()? {
        return Err(bad("face_attr"));
    }
    check_range("face_attr", -1, rows(meta, "face_attrs")?.len()?)?;
    for k in ["edge_flags", "edge_curve", "edge_layer"] {
        if out.contains(k)? && item(k)?.len()? != ne {
            return Err(bad(k));
        }
    }
    if out.contains("edge_layer")? {
        check_range("edge_layer", -1, rows(meta, "edge_layers")?.len()?)?
    }
    Ok(out)
}
#[pyfunction]
fn igz_document_geometry_arrays<'p>(meta: &Obj<'p>, geom: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = meta.py();
    let member = if meta.is_instance_of::<PyDict>() {
        get(meta, "member")?
    } else {
        py.None().into_bound(py)
    };
    let blob = geom.call_method1("get", (&member,))?;
    if blob.is_none() {
        return Err(PyValueError::new_err(format!(
            "the document lost its geometry block {}",
            member.repr()?
        )));
    }
    let checked = geom
        .getattr("checked")
        .ok()
        .filter(|c| c.is_truthy().unwrap_or(false))
        .map(|d| d.call_method1("get", (&member,)).map(|m| m.is(meta)))
        .transpose()?
        .unwrap_or(false);
    if checked {
        return api(py)?.call_method1("_geometry_arrays", (meta, blob, true));
    }
    let kw = PyDict::new(py);
    kw.set_item(
        "built_format",
        geom.getattr("built_format")
            .is_ok_and(|b| b.is_truthy().unwrap_or(false)),
    )?;
    api(py)?.call_method("_geometry_arrays", (meta, blob), Some(&kw))
}
fn finite_point(point: &Obj<'_>) -> PyResult<bool> {
    let py = point.py();
    let float = py.import("builtins")?.getattr("float")?;

    let result = (|| -> PyResult<bool> {
        for c in point.try_iter()? {
            if !float.call1((c?,))?.extract::<f64>()?.is_finite() {
                return Ok(false);
            }
        }
        Ok(true)
    })();
    match result {
        Err(e) if e.is_instance_of::<PyTypeError>(py) || e.is_instance_of::<PyValueError>(py) => {
            Ok(false)
        }
        other => other,
    }
}

#[pyfunction(signature=(*points))]
fn igz_finite_points(points: &Bound<'_, PyTuple>) -> PyResult<bool> {
    for point in points.iter() {
        if !finite_point(&point)? {
            return Ok(false);
        }
    }
    Ok(true)
}
#[pyfunction]
fn igz_without_nonfinite<'p>(payload: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = payload.py();
    let codec = api(py)?;
    if !codec
        .getattr("_finite_flag")?
        .call_method0("get")?
        .get_item(0)?
        .is_truthy()?
    {
        return Ok(payload.clone());
    }
    let edges = PyList::empty(py);
    let faces = PyList::empty(py);
    let mut skipped = 0;
    for e in rows(payload, "edges")?.try_iter()? {
        let e = e?;
        if finite_point(&e.call_method1("get", ("a", PyTuple::empty(py)))?)?
            && finite_point(&e.call_method1("get", ("b", PyTuple::empty(py)))?)?
        {
            edges.append(e)?
        } else {
            skipped += 1
        }
    }
    for f in rows(payload, "faces")?.try_iter()? {
        let f = f?;
        let mut ok = true;
        let mut loops = vec![rows(&f, "vertices")?];
        loops.extend(
            rows(&f, "holes")?
                .try_iter()?
                .collect::<PyResult<Vec<_>>>()?,
        );
        'outer: for l in loops {
            for v in l.try_iter()? {
                if !finite_point(&v?)? {
                    ok = false;
                    break 'outer;
                }
            }
        }
        if ok {
            faces.append(f)?
        } else {
            skipped += 1
        }
    }
    if skipped == 0 {
        return Ok(payload.clone());
    }
    repair(py, skipped)?;
    let out = payload.cast::<PyDict>()?.copy()?;
    out.set_item("faces", faces)?;
    out.set_item("edges", edges)?;
    Ok(out.into_any())
}
#[pyfunction]
fn igz_load_mesh_json(mesh: &Obj<'_>, payload: &Obj<'_>) -> PyResult<()> {
    let py = mesh.py();
    let codec = api(py)?;
    let edges = rows(payload, "edges")?;
    let faces = rows(payload, "faces")?;
    if !codec
        .call_method1("_bulk_sized", (edges.len()?, faces.len()?))?
        .is_truthy()?
    {
        codec.call_method1("_load_mesh_small", (mesh, payload))?;
        return Ok(());
    }
    let flat = PyList::empty(py);
    let flags = PyList::empty(py);
    let mut max_curve = None::<Obj<'_>>;
    for raw in edges.try_iter()? {
        let raw = raw?;
        flat.append(raw.get_item("a")?)?;
        flat.append(raw.get_item("b")?)?;
        let cid = get(&raw, "curve")?;
        if !cid.is_none()
            && match &max_curve {
                Some(n) => cid.gt(n)?,
                None => true,
            }
        {
            max_curve = Some(cid.clone())
        }
        let soft = get(&raw, "soft")?.is_truthy()?;
        flags.append((
            soft,
            cid,
            get(&raw, "layer")?,
            get(&raw, "hidden")?.is_truthy()?,
            soft && get(&raw, "flat")?.is_truthy()?,
        ))?;
    }
    let edge_points = flat.len() as isize;
    let attrs = PyList::empty(py);
    let mut sizes = vec![];
    let mut counts = vec![];
    for raw in faces.try_iter()? {
        let raw = raw?;
        let loops = crate::file_mesh::rings(raw.cast::<PyDict>()?)?;
        counts.push(loops.len());
        for ring in loops {
            sizes.push(ring.len()?);
            flat.call_method1("extend", (ring,))?;
        }
        attrs.append(codec.call_method1("_face_attrs_from_json", (raw,))?)?;
    }
    if flat.is_empty() {
        return Ok(());
    }
    let array = py
        .import("numpy")?
        .call_method1("array", (flat, "float64"))?;
    let (vs, ids): (Obj<'_>, Obj<'_>) = mesh.call_method1("bulk_weld", (array,))?.extract()?;
    let emap = if edge_points > 0 {
        let em = mesh.call_method1(
            "add_edges_welded",
            (
                &vs,
                ids.get_item(PySlice::new(py, 0, edge_points, 2))?,
                ids.get_item(PySlice::new(py, 1, edge_points, 2))?,
                flags,
            ),
        )?;
        if let Some(cid) = max_curve {
            bump(&py.import("ingetrazo_core.mesh")?, &cid)?
        }
        em
    } else {
        py.None().into_bound(py)
    };
    if !counts.is_empty() {
        let kw = PyDict::new(py);
        kw.set_item("edge_map", emap)?;
        mesh.call_method(
            "add_faces_welded",
            (
                vs,
                ids.get_item(PySlice::new(py, edge_points, isize::MAX, 1))?,
                sizes,
                counts,
                attrs,
            ),
            Some(&kw),
        )?;
    }
    Ok(())
}
#[pyclass]
struct Builder {
    source: Py<PyAny>,
    precise: bool,
    json: bool,
}
#[pymethods]
impl Builder {
    fn __call__(&self, mesh: &Obj<'_>) -> PyResult<()> {
        let py = mesh.py();
        let codec = api(py)?;
        let s = self.source.bind(py);
        if self.json {
            codec.call_method1(
                "_load_mesh_json",
                (mesh, codec.call_method1("_payload_from_arrays", (s,))?),
            )?;
        } else {
            let empty = || -> PyResult<bool> {
                for k in ["vertices", "edges", "faces"] {
                    if mesh.getattr(k)?.is_truthy()? {
                        return Ok(false);
                    }
                }
                Ok(true)
            };
            if self.precise || empty()? {
                let (st, table) =
                    crate::kernel::store_views::block::store_from_source(s, self.precise)?;
                crate::kernel::store_views::lifecycle::become_store(
                    mesh,
                    &st,
                    table.as_any(),
                    None,
                )?;
            } else if get(s, "sequential")?.is_truthy()? {
                codec.call_method1(
                    "_load_mesh_small",
                    (mesh, codec.call_method1("_payload_from_arrays", (s,))?),
                )?;
            } else {
                codec.call_method(
                    "_build_geometry",
                    (
                        mesh,
                        s.get_item("meta")?,
                        s.get_item("pos")?,
                        s.get_item("edges")?,
                        s.get_item("flags")?,
                        s.get_item("curves")?,
                        s.get_item("layers")?,
                        s.get_item("rings")?,
                        s.get_item("sizes")?,
                        s.get_item("counts")?,
                        s.get_item("face_attr")?,
                    ),
                    None,
                )?;
            }
        }
        Ok(())
    }
}
#[pyfunction]
#[pyo3(signature=(mesh,payload,geom=None))]
fn igz_load_mesh(mesh: &Obj<'_>, payload: &Obj<'_>, geom: Option<&Obj<'_>>) -> PyResult<()> {
    let py = mesh.py();
    let codec = api(py)?;
    if payload.contains("geom")? {
        let g = geom.cloned().unwrap_or_else(|| PyDict::new(py).into_any());
        codec.call_method1("_load_geometry", (mesh, payload.get_item("geom")?, g))?;
        return Ok(());
    }
    let mut src = get(payload, "_arrays")?;
    let arrays = !src.is_none();
    let mut p = payload.clone();
    if arrays {
        codec.call_method1("_merge_native_attrs", (&src,))?;
    } else {
        p = codec.call_method1("_without_nonfinite", (payload,))?
    }
    let module = py.import("ingetrazo_core.mesh")?;
    if arrays
        || (py
            .import("core._native")?
            .call_method0("wanted")?
            .is_truthy()?
            && (mesh.get_type().is(&module.getattr("Mesh")?)
                || mesh
                    .get_type()
                    .is(&py.import("ingetrazo_core.store_mesh")?.getattr("StoreMesh")?)))
    {
        if !arrays {
            src = codec.call_method1("_json_arrays", (&p,))?
        }
        if !src.is_none() {
            update_curves(&module, &src.get_item("curves")?)?;
            let builder = Py::new(
                py,
                Builder {
                    source: src.clone().unbind(),
                    precise: false,
                    json: true,
                },
            )?;
            if module
                .getattr("DeferredMesh")?
                .call_method1(
                    "defer",
                    (mesh, builder, src.get_item("counts")?.len()?, &src),
                )?
                .is_truthy()?
            {
                return Ok(());
            }
            if arrays {
                p = codec.call_method1("_payload_from_arrays", (&src,))?
            }
        }
    }
    codec.call_method1("_load_mesh_json", (mesh, p))?;
    Ok(())
}
fn update_curves(module: &Bound<'_, PyModule>, curves: &Obj<'_>) -> PyResult<()> {
    if !curves.is_none() && curves.getattr("size")?.is_truthy()? {
        bump(module, &curves.call_method0("max")?)?
    }
    Ok(())
}
#[pyfunction]
fn igz_load_geometry<'p>(mesh: &Obj<'p>, meta: &Obj<'p>, geom: &Obj<'p>) -> PyResult<()> {
    let py = mesh.py();
    let codec = api(py)?;
    let np = py.import("numpy")?;
    let module = py.import("ingetrazo_core.mesh")?;
    let a = codec.call_method1("_document_geometry_arrays", (meta, geom))?;
    let s = PyDict::new(py);
    s.set_item("meta", meta)?;
    for (k, v) in [
        ("pos", "pos"),
        ("edges", "edges"),
        ("rings", "rings"),
        ("sizes", "ring_sizes"),
        ("counts", "ring_counts"),
        ("face_attr", "face_attr"),
        ("flags", "edge_flags"),
        ("curves", "edge_curve"),
        ("layers", "edge_layer"),
    ] {
        s.set_item(k, get(&a, v)?)?
    }
    let item = |k: &str| s.as_any().get_item(k);
    let pos = item("pos")?;
    let bad = np
        .call_method1("isfinite", (&pos,))?
        .call_method1("all", (1,))?
        .call_method0("__invert__")?;
    if bad.call_method0("any")?.is_truthy()? {
        let edges = item("edges")?;
        let sizes = item("sizes")?;
        let counts = item("counts")?;
        let rings = item("rings")?;
        let left = bad.get_item(edges.get_item((full(py), 0))?)?;
        let right = bad.get_item(edges.get_item((full(py), 1))?)?;
        let edge_bad = left.call_method1("__or__", (right,))?;
        let keep_e = edge_bad.call_method0("__invert__")?;
        let mask = |values: Obj<'p>, lengths: &Obj<'p>| -> PyResult<Obj<'p>> {
            if lengths.len()? == 0 {
                return np.call_method1("zeros", (0, "bool"));
            }
            let starts = np.call_method1("cumsum", (lengths,))?.sub(lengths)?;
            np.getattr("add")?
                .call_method1(
                    "reduceat",
                    (values.call_method1("astype", ("int64",))?, starts),
                )?
                .call_method1("__gt__", (0,))
        };
        let ring_bad = mask(bad.get_item(&rings)?, &sizes)?;
        let face_bad = mask(ring_bad, &counts)?;
        let n = py.import("builtins")?.getattr("int")?;
        repair(
            py,
            n.call1((edge_bad
                .call_method0("sum")?
                .add(face_bad.call_method0("sum")?)?,))?
                .extract()?,
        )?;
        for k in ["edges", "flags", "curves", "layers"] {
            let col = item(k)?;
            if !col.is_none() {
                s.set_item(k, col.get_item(&keep_e)?)?
            }
        }
        let which = np.call_method1(
            "repeat",
            (np.call_method1("arange", (counts.len()?,))?, &counts),
        )?;
        let keep_r = face_bad.get_item(which)?.call_method0("__invert__")?;
        let keep_f = face_bad.call_method0("__invert__")?;
        s.set_item(
            "rings",
            rings.get_item(np.call_method1("repeat", (&keep_r, &sizes))?)?,
        )?;
        s.set_item("sizes", sizes.get_item(keep_r)?)?;
        s.set_item("counts", counts.get_item(&keep_f)?)?;
        s.set_item("face_attr", item("face_attr")?.get_item(keep_f)?)?;
    }
    if pos.len()? == 0 || item("edges")?.len()? + item("counts")?.len()? == 0 {
        return Ok(());
    }
    update_curves(&module, &item("curves")?)?;
    if get(meta, "sequential")?.is(&true.into_pyobject(py)?) {
        s.set_item("sequential", true)?
    }
    let precise = pos
        .getattr("dtype")?
        .eq(np.call_method1("dtype", ("<f8",))?)?;
    let builder = Py::new(
        py,
        Builder {
            source: s.clone().into_any().unbind(),
            precise,
            json: false,
        },
    )?;
    if !precise
        && py
            .import("core._native")?
            .call_method0("wanted")?
            .is_truthy()?
        && module
            .getattr("DeferredMesh")?
            .call_method1("defer", (mesh, &builder, item("counts")?.len()?, s))?
            .is_truthy()?
    {
        return Ok(());
    }
    builder.call1(py, (mesh,))?;
    Ok(())
}
#[pyfunction]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature=(mesh,meta,pos,edges,flags,curves,layers,rings,sizes,counts,face_attr))]
fn igz_build_geometry(
    mesh: &Obj<'_>,
    meta: &Obj<'_>,
    pos: &Obj<'_>,
    edges: &Obj<'_>,
    flags: &Obj<'_>,
    curves: &Obj<'_>,
    layers: &Obj<'_>,
    rings: &Obj<'_>,
    sizes: &Obj<'_>,
    counts: &Obj<'_>,
    face_attr: &Obj<'_>,
) -> PyResult<()> {
    let py = mesh.py();
    let codec = api(py)?;
    let (vs, ids): (Obj<'_>, Obj<'_>) = mesh.call_method1("bulk_weld", (pos,))?.extract()?;
    let mut emap = py.None().into_bound(py);
    if edges.len()? > 0 {
        let ef =
            if flags.call_method0("any")?.is_truthy()? || !curves.is_none() || !layers.is_none() {
                let names = rows(meta, "edge_layers")?;
                let out = PyList::empty(py);
                let cv = if curves.is_none() {
                    None
                } else {
                    Some(curves.call_method0("tolist")?.extract::<Vec<i64>>()?)
                };
                let lv = if layers.is_none() {
                    None
                } else {
                    Some(layers.call_method0("tolist")?.extract::<Vec<i64>>()?)
                };
                let flags = flags.call_method0("tolist")?.extract::<Vec<u8>>()?;
                for (i, f) in flags.into_iter().enumerate() {
                    let c = cv
                        .as_ref()
                        .and_then(|v| v.get(i).copied())
                        .filter(|c| *c >= 0);
                    let l = lv
                        .as_ref()
                        .and_then(|v| v.get(i).copied())
                        .filter(|l| *l >= 0)
                        .map(|l| names.get_item(l))
                        .transpose()?;
                    out.append((f & 1 != 0, c, l, f & 2 != 0, f & 4 != 0))?;
                }
                out.into_any()
            } else {
                py.None().into_bound(py)
            };
        emap = mesh.call_method1(
            "add_edges_welded",
            (
                &vs,
                ids.get_item(edges.get_item((full(py), 0))?)?,
                ids.get_item(edges.get_item((full(py), 1))?)?,
                ef,
            ),
        )?;
    }
    if counts.len()? > 0 {
        let table = rows(meta, "face_attrs")?;
        let attrs = PyList::empty(py);
        for k in face_attr.call_method0("tolist")?.extract::<Vec<i64>>()? {
            attrs.append(if k >= 0 {
                codec.call_method1("_face_attrs_from_json", (table.get_item(k)?,))?
            } else {
                py.None().into_bound(py)
            })?
        }
        let kw = PyDict::new(py);
        kw.set_item("edge_map", emap)?;
        mesh.call_method(
            "add_faces_welded",
            (vs, ids.get_item(rings)?, sizes, counts, attrs),
            Some(&kw),
        )?;
    }
    Ok(())
}
#[pyfunction]
fn igz_normalize_text_curve_ids(payload: &Obj<'_>, geom: &Obj<'_>) -> PyResult<()> {
    let py = payload.py();
    let module = py.import("ingetrazo_core.mesh")?;
    let codec = api(py)?;
    let mut edges = vec![];
    let mut binary = vec![];
    let mut top = module.getattr("_CURVE_COUNTER")?.sub(1)?;
    for b in codec.call_method1("_mesh_blocks", (payload,))?.try_iter()? {
        let b = b?;
        if b.contains("geom")? {
            binary.push(b.get_item("geom")?);
            continue;
        }
        for e in rows(&b, "edges")?.try_iter()? {
            let e = e?;
            let c = get(&e, "curve")?;
            if c.is_instance_of::<PyString>() {
                edges.push(e)
            } else if c.is_instance_of::<PyInt>() && c.gt(&top)? {
                top = c;
            }
        }
    }
    if edges.is_empty() {
        return Ok(());
    }
    for meta in binary {
        let curves = get(
            &codec.call_method1("_document_geometry_arrays", (meta, geom))?,
            "edge_curve",
        )?;
        if !curves.is_none() && curves.getattr("size")?.is_truthy()? {
            let v = py
                .import("builtins")?
                .call_method1("int", (curves.call_method0("max")?,))?;
            if v.gt(&top)? {
                top = v
            }
        }
    }
    module.setattr("_CURVE_COUNTER", top.add(1)?)?;
    let names = PyDict::new(py);
    for e in edges {
        let name = e.get_item("curve")?;
        let id = if let Some(id) = names.get_item(&name)? {
            id
        } else {
            let id = module.getattr("Mesh")?.call_method0("next_curve_id")?;
            names.set_item(name, &id)?;
            id
        };
        e.set_item("curve", id)?;
    }
    Ok(())
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(igz_geometry_arrays, m)?)?;
    m.add_function(wrap_pyfunction!(igz_document_geometry_arrays, m)?)?;
    m.add_function(wrap_pyfunction!(igz_load_geometry, m)?)?;
    m.add_function(wrap_pyfunction!(igz_build_geometry, m)?)?;
    m.add_function(wrap_pyfunction!(igz_load_mesh, m)?)?;
    m.add_function(wrap_pyfunction!(igz_load_mesh_json, m)?)?;
    m.add_function(wrap_pyfunction!(igz_finite_points, m)?)?;
    m.add_function(wrap_pyfunction!(igz_without_nonfinite, m)?)?;
    m.add_function(wrap_pyfunction!(igz_normalize_text_curve_ids, m)?)
}
