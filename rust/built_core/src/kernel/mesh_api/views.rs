// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
const LOCAL: u128 = (1 << 40) - 1;
#[pyfunction]
fn view_init(this: &O<'_>, m: &O<'_>, key: &O<'_>) -> PyResult<()> {
    this.setattr("_m", m)?;
    this.setattr("key", key)
}
#[pyfunction]
fn compare<'p>(equal: bool, this: &O<'p>, other: &O<'p>) -> PyResult<O<'p>> {
    let py = this.py();
    let m = api(py)?;
    let same = if this.is(other) {
        true
    } else {
        if !other.is_instance(
            (m.getattr("Vertex")?, m.getattr("Edge")?, m.getattr("Face")?)
                .into_pyobject(py)?
                .as_any(),
        )? {
            return Ok(py.NotImplemented().into_bound(py));
        }
        other.getattr("key")?.eq(this.getattr("key")?)?
    };
    Ok((same == equal).into_pyobject(py)?.to_owned().into_any())
}
#[pyfunction]
fn view_hash(this: &O<'_>) -> PyResult<isize> {
    this.getattr("key")?.hash()
}
#[pyfunction]
fn view_copy<'p>(this: &O<'p>, memo: &O<'p>) -> PyResult<O<'p>> {
    let m = this.getattr("_m")?;
    let k = this.getattr("key")?.extract::<u128>()?;
    if !m
        .getattr("_store")?
        .call_method1("knows", (k,))?
        .is_truthy()?
    {
        return Err(PyKeyError::new_err(format!("unknown mesh element {k}")));
    }
    let owner = module(this.py(), "copy")?.call_method1("deepcopy", (m, memo))?;
    _unpickle_view(this.get_type().as_any(), &owner, k & LOCAL)
}
#[pyfunction]
fn _unpickle_view<'p>(cls: &O<'p>, mesh: &O<'p>, local_key: u128) -> PyResult<O<'p>> {
    let base = mesh
        .getattr("_store")?
        .getattr("key_base")?
        .extract::<u128>()?;
    cls.call1((mesh, base | local_key))
}
#[pyfunction]
fn reduce<'p>(this: &O<'p>) -> PyResult<O<'p>> {
    let py = this.py();
    let k = this.getattr("key")?.extract::<u128>()? & LOCAL;
    Ok((
        store_api(py)?.getattr("_unpickle_view")?,
        (this.get_type(), this.getattr("_m")?, k),
    )
        .into_pyobject(py)?
        .into_any())
}
#[pyfunction]
fn view_query<'p>(name: &str, this: &O<'p>) -> PyResult<O<'p>> {
    kernel(this.py())?.call_method1(name, (this.getattr("_m")?, this.getattr("key")?))
}
#[pyfunction]
fn tuple_position<'p>(this: &O<'p>) -> PyResult<O<'p>> {
    builtin(this.py(), "tuple")?.call1((this
        .getattr("_m")?
        .getattr("_store")?
        .call_method1("vertex_position", (this.getattr("key")?,))?,))
}
#[pyfunction]
fn place(this: &O<'_>, value: &O<'_>) -> PyResult<()> {
    this.getattr("_m")?
        .call_method1("place_vertex", (this, value))?;
    Ok(())
}
#[pyfunction]
fn end<'p>(index: usize, this: &O<'p>) -> PyResult<O<'p>> {
    let m = this.getattr("_m")?;
    let k = m
        .getattr("_store")?
        .call_method1("edge_ends", (this.getattr("key")?,))?
        .get_item(index)?;
    kernel(this.py())?.call_method1("position", (m, k))
}
#[pyfunction]
fn other<'p>(this: &O<'p>, v: &O<'p>) -> PyResult<O<'p>> {
    let a = this.getattr("v0")?;
    if v.getattr("key")?.eq(a.getattr("key")?)? {
        this.getattr("v1")
    } else {
        Ok(a)
    }
}
#[pyfunction]
fn has(this: &O<'_>, v: &O<'_>) -> PyResult<bool> {
    let k = v.getattr("key")?;
    Ok(
        k.eq(this.getattr("v0")?.getattr("key")?)?
            || k.eq(this.getattr("v1")?.getattr("key")?)?,
    )
}
#[pyfunction]
fn length<'p>(this: &O<'p>) -> PyResult<O<'p>> {
    let st = this.getattr("_m")?.getattr("_store")?;
    let e = st.call_method1("edge_ends", (this.getattr("key")?,))?;
    module(this.py(), "math")?.call_method1(
        "dist",
        (
            st.call_method1("vertex_position", (e.get_item(0)?,))?,
            st.call_method1("vertex_position", (e.get_item(1)?,))?,
        ),
    )
}
#[pyfunction]
fn vertices<'p>(this: &O<'p>) -> PyResult<O<'p>> {
    list(&this.call_method0("_vectors")?.get_item(0)?)
}
#[pyfunction]
fn holes<'p>(this: &O<'p>) -> PyResult<O<'p>> {
    let out = PyList::empty(this.py());
    for r in this.call_method0("_vectors")?.try_iter()?.skip(1) {
        out.append(list(&r?)?)?;
    }
    Ok(out.into_any())
}
#[pyfunction]
fn reverse(this: &O<'_>) -> PyResult<()> {
    this.getattr("_m")?
        .getattr("_store")?
        .call_method1("reverse_face", (this.getattr("key")?,))?;
    Ok(())
}
#[pyfunction(signature=(this,normal=None))]
fn triangulate<'p>(this: &O<'p>, normal: Option<&O<'p>>) -> PyResult<O<'p>> {
    let py = this.py();
    let h = module(py, "core._native")?.call_method0("qt_hypot")?;
    let key = this.getattr("key")?;
    let store = this.getattr("_m")?.getattr("_store")?;
    if !h.is_truthy()? || !store.call_method1("is_live", (&key,))?.is_truthy()? {
        return elements::triangulate(this, normal);
    }
    let rows = store.call_method1(
        "face_triangles",
        (
            key,
            h,
            normal.map(|n| n.call_method0("toTuple")).transpose()?,
        ),
    )?;
    let vec = module(py, "built_core")?.getattr("Vec3")?;
    let out = PyList::empty(py);
    for t in rows.try_iter()? {
        let mut tri = Vec::new();
        for p in t?.try_iter()? {
            tri.push(vec.call(builtin(py, "tuple")?.call1((p?,))?.cast::<PyTuple>()?, None)?);
        }
        out.append(PyTuple::new(py, tri)?)?;
    }
    Ok(out.into_any())
}
#[pyfunction]
fn metric<'p>(name: &str, this: &O<'p>) -> PyResult<O<'p>> {
    let py = this.py();
    let r = this.call_method0("_positions")?;
    let first = r.get_item(0)?;
    match name {
        "area" => kernel(py)?.call_method1("face_area", (first,)),
        "net_area" => module(py, "built_core")?.call_method1(
            "polygon_net_area",
            (
                first,
                r.get_item(pyo3::types::PySlice::new(py, 1, isize::MAX, 1))?,
            ),
        ),
        n => module(py, "built_core")?.call_method1(n, (first,)),
    }
}
pub(super) fn install(s: &M<'_>, m: &M<'_>, core: &M<'_>, k: &O<'_>) -> PyResult<()> {
    let py = s.py();
    s.add_function(wrap_pyfunction!(_unpickle_view, s)?)?;
    let ns = PyDict::new(py);
    slots(&ns, "")?;
    method!(ns, s, "__init__", view_init);
    method!(ns, s, "__hash__", view_hash);
    method!(ns, s, "__deepcopy__", view_copy);
    method!(ns, s, "__reduce__", reduce);
    for (n, eq) in [("__eq__", true), ("__ne__", false)] {
        bind(
            &ns,
            n,
            &partial(wrap_pyfunction!(compare, s)?.as_any(), eq)?,
        )?;
    }
    let mix = class(s, "_ViewMixin", &[], &ns)?;
    let ns = PyDict::new(py);
    slots(&ns, "_m")?;
    method!(ns, s, "position_tuple", tuple_position);
    property(
        &ns,
        "position",
        &partial(wrap_pyfunction!(view_query, s)?.as_any(), "position")?,
        Some(wrap_pyfunction!(place, s)?.as_any()),
    )?;
    property(
        &ns,
        "edges",
        &k.getattr("vertex_edges")?,
        Some(&k.getattr("set_vertex_edges")?),
    )?;
    class(s, "VertexView", &[mix.clone(), m.getattr("Vertex")?], &ns)?;
    let ns = PyDict::new(py);
    slots(&ns, "_m")?;
    for (i, n) in ["v0", "v1"].iter().enumerate() {
        property(
            &ns,
            n,
            &partial(&k.getattr("endpoint")?, i)?,
            Some(&partial(&k.getattr("set_endpoint")?, i)?),
        )?;
    }
    for n in ["soft", "hidden", "flat", "curve", "layer"] {
        property(
            &ns,
            n,
            &partial(&k.getattr("edge_flag")?, n)?,
            Some(&partial(&k.getattr("set_edge_flag")?, n)?),
        )?;
    }
    for (i, n) in ["a", "b"].iter().enumerate() {
        property(
            &ns,
            n,
            &partial(wrap_pyfunction!(end, s)?.as_any(), i)?,
            None,
        )?;
    }
    property(
        &ns,
        "faces",
        &k.getattr("edge_faces")?,
        Some(&k.getattr("set_edge_faces")?),
    )?;
    method!(ns, s, "length", length);
    method!(ns, s, "other", other);
    method!(ns, s, "has", has);
    class(s, "EdgeView", &[mix.clone(), m.getattr("Edge")?], &ns)?;
    let ns = PyDict::new(py);
    slots(&ns, "_m")?;
    for (n, h) in [("loop", false), ("hole_loops", true)] {
        property(
            &ns,
            n,
            &partial(&k.getattr("face_loops")?, h)?,
            Some(&partial(&k.getattr("set_face_loops")?, h)?),
        )?;
    }
    for (n, get, set) in [
        ("interior", "interior", "set_interior"),
        ("attrs", "face_attrs", "set_face_attrs"),
    ] {
        property(&ns, n, &k.getattr(get)?, Some(&k.getattr(set)?))?;
    }
    for (n, f) in [
        ("_positions", "ring_positions"),
        ("_vectors", "ring_vectors"),
        ("normal", "cached_normal"),
    ] {
        bind(
            &ns,
            n,
            &partial(wrap_pyfunction!(view_query, s)?.as_any(), f)?,
        )?;
    }
    for (n, f) in [
        ("_newell", "polygon_newell"),
        ("area", "area"),
        ("net_area", "net_area"),
        ("centroid", "polygon_centroid"),
    ] {
        bind(&ns, n, &partial(wrap_pyfunction!(metric, s)?.as_any(), f)?)?;
    }
    getter!(ns, s, "vertices", vertices);
    getter!(ns, s, "holes", holes);
    method!(ns, s, "triangulate", triangulate);
    method!(ns, s, "reverse_rings", reverse);
    class(s, "FaceView", &[mix, m.getattr("Face")?], &ns)?;
    s.add("StoreFallback", core.getattr("StoreFallback")?)?;
    for n in ["Vertex", "Edge", "Face"] {
        s.add(n, m.getattr(n)?)?;
    }
    Ok(())
}
