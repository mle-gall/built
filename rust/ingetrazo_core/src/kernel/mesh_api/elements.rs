// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
fn core(py: Python<'_>) -> PyResult<O<'_>> {
    module(py, "ingetrazo_core")
}
fn set_key(o: &O<'_>, key: Option<&O<'_>>) -> PyResult<()> {
    o.setattr(
        "key",
        match key {
            Some(k) => k.clone(),
            None => api(o.py())?
                .getattr("_STANDALONE_KEYS")?
                .call_method0("__next__")?,
        },
    )
}
#[pyfunction(signature=(this,position,*,key=None))]
fn vertex_init(this: &O<'_>, position: &O<'_>, key: Option<&O<'_>>) -> PyResult<()> {
    set_key(this, key)?;
    this.setattr(
        "position",
        core(this.py())?.call_method1("Vec3", (position,))?,
    )?;
    this.setattr("edges", PySet::empty(this.py())?)
}
#[pyfunction]
fn position_tuple<'p>(this: &O<'p>) -> PyResult<O<'p>> {
    this.getattr("position")?.call_method0("toTuple")
}
#[pyfunction]
fn faces<'p>(this: &O<'p>) -> PyResult<O<'p>> {
    core(this.py())?
        .getattr("object_replay")?
        .call_method1("vertex_faces", (this.getattr("edges")?,))
}
#[pyfunction]
fn vertex_repr(this: &O<'_>) -> PyResult<String> {
    let p = this.getattr("position")?;
    let (x, y, z) = p.call_method0("toTuple")?.extract::<(f64, f64, f64)>()?;
    Ok(format!("Vertex({x:.3}, {y:.3}, {z:.3})"))
}
#[pyfunction(signature=(this,v0,v1,*,key=None))]
fn edge_init(this: &O<'_>, v0: &O<'_>, v1: &O<'_>, key: Option<&O<'_>>) -> PyResult<()> {
    set_key(this, key)?;
    this.setattr("v0", v0)?;
    this.setattr("v1", v1)?;
    this.setattr("faces", PyList::empty(this.py()))?;
    for k in ["soft", "flat", "hidden"] {
        this.setattr(k, false)?;
    }
    for k in ["layer", "curve"] {
        this.setattr(k, this.py().None())?;
    }
    Ok(())
}
#[pyfunction]
fn endpoint<'p>(name: &str, this: &O<'p>) -> PyResult<O<'p>> {
    this.getattr(name)?.getattr("position")
}
#[pyfunction]
fn other<'p>(this: &O<'p>, v: &O<'p>) -> PyResult<O<'p>> {
    this.getattr(if v.eq(this.getattr("v0")?)? {
        "v1"
    } else {
        "v0"
    })
}
#[pyfunction]
fn has(this: &O<'_>, v: &O<'_>) -> PyResult<bool> {
    Ok(v.eq(this.getattr("v0")?)? || v.eq(this.getattr("v1")?)?)
}
#[pyfunction]
fn length<'p>(this: &O<'p>) -> PyResult<O<'p>> {
    endpoint("v1", this)?
        .sub(endpoint("v0", this)?)?
        .call_method0("length")
}
#[pyfunction]
fn edge_repr(this: &O<'_>) -> PyResult<String> {
    Ok(format!(
        "Edge({} -> {})",
        this.getattr("v0")?.repr()?,
        this.getattr("v1")?.repr()?
    ))
}
#[pyfunction(signature=(this,r#loop,hole_loops=None,*,key=None))]
fn face_init(
    this: &O<'_>,
    r#loop: &O<'_>,
    hole_loops: Option<&O<'_>>,
    key: Option<&O<'_>>,
) -> PyResult<()> {
    set_key(this, key)?;
    this.setattr("loop", list(r#loop)?)?;
    let holes = PyList::empty(this.py());
    if let Some(h) = hole_loops {
        for ring in h.try_iter()? {
            holes.append(list(&ring?)?)?;
        }
    }
    this.setattr("hole_loops", holes)?;
    this.setattr("attrs", PyDict::new(this.py()))?;
    this.setattr("interior", false)
}
fn points<'p>(ring: &O<'p>) -> PyResult<O<'p>> {
    let out = PyList::empty(ring.py());
    for v in ring.try_iter()? {
        out.append(v?.getattr("position")?)?;
    }
    Ok(out.into_any())
}
#[pyfunction]
fn vertices<'p>(this: &O<'p>) -> PyResult<O<'p>> {
    points(&this.getattr("loop")?)
}
#[pyfunction]
fn holes<'p>(this: &O<'p>) -> PyResult<O<'p>> {
    let out = PyList::empty(this.py());
    for ring in this.getattr("hole_loops")?.try_iter()? {
        out.append(points(&ring?)?)?;
    }
    Ok(out.into_any())
}
#[pyfunction]
fn ring_metric<'p>(name: &str, this: &O<'p>) -> PyResult<O<'p>> {
    core(this.py())?
        .getattr("object_replay")?
        .call_method1(name, (this.getattr("loop")?,))
}
#[pyfunction]
fn polygon_metric<'p>(name: &str, this: &O<'p>) -> PyResult<O<'p>> {
    core(this.py())?.call_method1(name, (this.getattr("vertices")?,))
}
#[pyfunction]
fn net_area<'p>(this: &O<'p>) -> PyResult<O<'p>> {
    core(this.py())?.call_method1(
        "polygon_net_area",
        (this.getattr("vertices")?, this.getattr("holes")?),
    )
}
#[pyfunction(signature=(this,normal=None))]
pub(super) fn triangulate<'p>(this: &O<'p>, normal: Option<&O<'p>>) -> PyResult<O<'p>> {
    if this.getattr("loop")?.len()? < 3 {
        return Ok(PyList::empty(this.py()).into_any());
    }
    module(this.py(), "core.triangulate")?.call_method1(
        "triangulate",
        (
            this.getattr("vertices")?,
            this.getattr("holes")?,
            match normal {
                Some(n) => n.clone(),
                None => this.call_method0("normal")?,
            },
        ),
    )
}
#[pyfunction]
fn face_repr(this: &O<'_>) -> PyResult<String> {
    Ok(format!(
        "Face({} verts, {} holes)",
        this.getattr("loop")?.len()?,
        this.getattr("holes")?.len()?
    ))
}
#[pyfunction]
fn dirty_get<'p>(attrs: bool, this: &O<'p>) -> PyResult<O<'p>> {
    this.getattr("__dict__")?.call_method1(
        "get",
        (
            if attrs {
                "_attrs_dirty_value"
            } else {
                "_chunk_dirty_value"
            },
            !attrs,
        ),
    )
}
#[pyfunction]
fn dirty_set(attrs: bool, this: &O<'_>, value: &O<'_>) -> PyResult<()> {
    this.getattr("__dict__")?.set_item(
        if attrs {
            "_attrs_dirty_value"
        } else {
            "_chunk_dirty_value"
        },
        value,
    )?;
    if value.is_truthy()? {
        this.call_method0("bump_revision")?;
    }
    Ok(())
}
#[pyfunction]
fn invalidate(this: &O<'_>) -> PyResult<()> {
    let d = this.getattr("__dict__")?;
    d.set_item("_chunk_dirty_value", true)?;
    d.set_item("_attrs_dirty_value", true)
}
#[pyfunction]
fn next_curve_id(py: Python<'_>) -> PyResult<O<'_>> {
    let m = api(py)?;
    let cid = m.getattr("_CURVE_COUNTER")?;
    m.setattr("_CURVE_COUNTER", cid.add(1)?)?;
    Ok(cid)
}
#[pyfunction(signature=(this,pos,ring_sizes,face_ring_counts,attrs=None))]
fn add_faces_bulk<'p>(
    this: &O<'p>,
    pos: &O<'p>,
    ring_sizes: &O<'p>,
    face_ring_counts: &O<'p>,
    attrs: Option<&O<'p>>,
) -> PyResult<O<'p>> {
    let a = this.call_method1("bulk_weld", (pos,))?;
    this.call_method1(
        "add_faces_welded",
        (
            a.get_item(0)?,
            a.get_item(1)?,
            ring_sizes,
            face_ring_counts,
            attrs,
        ),
    )
}
#[pyfunction]
fn element_identity<'p>(item: &O<'p>) -> PyResult<O<'p>> {
    let m = api(item.py())?;
    for n in ["Vertex", "Edge", "Face"] {
        if item.is_instance(&m.getattr(n)?)? {
            return item.getattr("key");
        }
    }
    builtin(item.py(), "id")?.call1((item,))
}
/// Delay reading face attributes until the caller first advances the iterator.
#[pyclass]
struct FaceAttributes {
    mesh: Option<Py<PyAny>>,
    rows: Option<Py<PyAny>>,
}
#[pymethods]
impl FaceAttributes {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__<'p>(&mut self, py: Python<'p>) -> PyResult<Option<O<'p>>> {
        if let Some(mesh) = self.mesh.take() {
            let rows = kernel(py)?
                .call_method1("face_attribute_rows", (mesh,))?
                .try_iter()?;
            self.rows = Some(rows.into_any().unbind());
        }
        match self.rows.as_ref() {
            Some(rows) => rows
                .bind(py)
                .cast::<pyo3::types::PyIterator>()?
                .clone()
                .next()
                .transpose(),
            None => Ok(None),
        }
    }
}
#[pyfunction]
fn face_attrs(mesh: Py<PyAny>) -> FaceAttributes {
    FaceAttributes {
        mesh: Some(mesh),
        rows: None,
    }
}
pub(super) fn install(m: &M<'_>, core: &M<'_>, k: &O<'_>) -> PyResult<()> {
    let py = m.py();
    let count = py.import("itertools")?.getattr("count")?;
    m.add("_KEY_DECIMALS", 4)?;
    m.add("_CURVE_COUNTER", 1)?;
    m.add("_KEY_NAMESPACES", count.call1((1,))?)?;
    m.add("_STANDALONE_KEYS", count.call1((-1, -1))?)?;
    m.add("_STITCH_TOL", 1e-4)?;
    m.add("_KEY_SCALE", 1e4)?;
    m.add("_WELD_TOL2", 1e-8)?;
    m.add("PAINT_KEYS", ("color", "mat", "texture", "opacity", "pbr"))?;
    m.add("QVector3D", core.getattr("Vec3")?)?;
    for n in ["object_drawing", "object_replay", "store_views"] {
        m.add(n, core.getattr(n)?)?;
    }
    for n in [
        "EDGE_FLAG_NAMES",
        "EDGE_FLAG_DEFAULTS",
        "smooths",
        "edge_flags",
        "edge_is_plain",
        "stamp_edge_flags",
        "face_count",
        "turn_face_over",
        "swap_orientation_paint",
    ] {
        m.add(n, k.getattr(n)?)?;
    }
    m.add_function(wrap_pyfunction!(face_attrs, m)?)?;
    m.add_function(wrap_pyfunction!(element_identity, m)?)?;
    let kw = PyDict::new(py);
    kw.set_item("default", py.None())?;
    m.add(
        "on_mesh_materialized",
        py.import("contextvars")?
            .getattr("ContextVar")?
            .call(("on_mesh_materialized",), Some(&kw))?,
    )?;
    let ns = PyDict::new(py);
    slots(&ns, "position edges key")?;
    method!(ns, m, "__init__", vertex_init);
    method!(ns, m, "faces", faces);
    method!(ns, m, "position_tuple", position_tuple);
    method!(ns, m, "__repr__", vertex_repr);
    class(m, "Vertex", &[], &ns)?;
    let ns = PyDict::new(py);
    slots(&ns, "key v0 v1 faces soft curve layer hidden flat")?;
    method!(ns, m, "__init__", edge_init);
    method!(ns, m, "other", other);
    method!(ns, m, "has", has);
    method!(ns, m, "length", length);
    method!(ns, m, "__repr__", edge_repr);
    for (name, field) in [("a", "v0"), ("b", "v1")] {
        property(
            &ns,
            name,
            &partial(wrap_pyfunction!(endpoint, m)?.as_any(), field)?,
            None,
        )?;
    }
    class(m, "Edge", &[], &ns)?;
    let ns = PyDict::new(py);
    slots(&ns, "key loop hole_loops interior attrs")?;
    method!(ns, m, "__init__", face_init);
    getter!(ns, m, "vertices", vertices);
    getter!(ns, m, "holes", holes);
    method!(ns, m, "net_area", net_area);
    method!(ns, m, "triangulate", triangulate);
    method!(ns, m, "__repr__", face_repr);
    bind(
        &ns,
        "reverse_rings",
        &core.getattr("object_replay")?.getattr("reverse_rings")?,
    )?;
    for (name, f) in [("normal", "ring_normal"), ("area", "loop_area")] {
        bind(
            &ns,
            name,
            &partial(wrap_pyfunction!(ring_metric, m)?.as_any(), f)?,
        )?;
    }
    for (name, f) in [
        ("_newell", "polygon_newell"),
        ("centroid", "polygon_centroid"),
    ] {
        bind(
            &ns,
            name,
            &partial(wrap_pyfunction!(polygon_metric, m)?.as_any(), f)?,
        )?;
    }
    class(m, "Face", &[], &ns)?;
    let ns = PyDict::new(py);
    property(&ns, "revision", &k.getattr("mesh_revision")?, None)?;
    for (field, attrs) in [("_chunk_dirty", false), ("_attrs_dirty", true)] {
        property(
            &ns,
            field,
            &partial(wrap_pyfunction!(dirty_get, m)?.as_any(), attrs)?,
            Some(&partial(wrap_pyfunction!(dirty_set, m)?.as_any(), attrs)?),
        )?;
    }
    for n in ["mark_modified", "bump_revision", "note_rewound"] {
        bind(&ns, n, &k.getattr(n)?)?;
    }
    ns.set_item(
        "next_curve_id",
        builtin(py, "staticmethod")?.call1((wrap_pyfunction!(next_curve_id, m)?,))?,
    )?;
    method!(ns, m, "invalidate_placement_chunks", invalidate);
    method!(ns, m, "add_faces_bulk", add_faces_bulk);
    class(m, "_MeshState", &[], &ns)?;
    Ok(())
}
