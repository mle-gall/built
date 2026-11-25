// SPDX-License-Identifier: GPL-3.0-or-later
use super::records::{_attribute_owner, _hypot, _xyz};
use super::*;
use crate::store::py::StoreFallback;
fn st<'p>(m: &O<'p>) -> PyResult<O<'p>> {
    m.getattr("_store")
}
fn forget_bulk(m: &O<'_>) -> PyResult<()> {
    m.getattr("__dict__")?
        .call_method1("pop", ("_bulk", m.py().None()))?;
    Ok(())
}
fn view<'p>(m: &O<'p>, n: &str, k: &O<'p>) -> PyResult<O<'p>> {
    store_api(m.py())?.call_method1(n, (m, k))
}
fn maybe_view<'p>(m: &O<'p>, n: &str, k: O<'p>) -> PyResult<O<'p>> {
    if k.is_none() {
        Ok(k)
    } else {
        view(m, n, &k)
    }
}
fn refuse(m: &O<'_>, op: &str) -> PyResult<PyErr> {
    Ok(PyErr::from_value(store_api(m.py())?.call_method1(
        "StoreRefusal",
        (format!(
            "{op} requires live, representable incidence in this mesh"
        ),),
    )?))
}
#[pyfunction]
fn init(this: &O<'_>) -> PyResult<()> {
    this.setattr("_mut_serial", 0)?;
    this.setattr("_chunk_dirty", true)?;
    kernel(this.py())?.call_method1("initialise_store", (this,))?;
    Ok(())
}
const KINDS: [(&str, &str, &str); 3] = [
    ("vertex", "VertexView", "Vertex"),
    ("edge", "EdgeView", "Edge"),
    ("face", "FaceView", "Face"),
];
#[pyfunction]
fn sequence<'p>(i: usize, this: &O<'p>) -> PyResult<O<'p>> {
    let (n, v, b) = KINDS[i];
    module(this.py(), "built_core")?.call_method1(
        "MeshElements",
        (
            this,
            n,
            store_api(this.py())?.getattr(v)?,
            api(this.py())?.getattr(b)?,
        ),
    )
}
#[pyfunction]
fn replace(i: usize, this: &O<'_>, value: &O<'_>) -> PyResult<()> {
    sequence(i, this)?.call_method1("replace", (value,))?;
    Ok(())
}
#[pyfunction]
fn registry<'p>(this: &O<'p>) -> PyResult<O<'p>> {
    let py = this.py();
    let reg = st(this)?.call_method0("registry")?;
    let cells = reg.get_item(0)?.call_method0("tolist")?;
    let indices = reg.get_item(1)?.call_method0("tolist")?;
    let out = PyDict::new(py);
    let vs = this.getattr("vertices")?;
    for (c, i) in cells.try_iter()?.zip(indices.try_iter()?) {
        out.set_item(builtin(py, "tuple")?.call1((c?,))?, vs.get_item(i?)?)?;
    }
    Ok(out.into_any())
}
#[pyfunction]
fn displaced<'p>(this: &O<'p>) -> PyResult<O<'p>> {
    let d = PyDict::new(this.py());
    for (c, ks) in st(this)?
        .call_method0("displaced_keys")?
        .cast::<PyDict>()?
        .iter()
    {
        d.set_item(c, views_list(this, "VertexView", &ks)?)?;
    }
    Ok(d.into_any())
}
#[pyfunction]
fn curve_edges<'p>(this: &O<'p>, edge: &O<'p>) -> PyResult<O<'p>> {
    let cid = edge.getattr("curve")?;
    if cid.is_none() {
        Ok(PyList::new(this.py(), [edge])?.into_any())
    } else {
        views_list(
            this,
            "EdgeView",
            &st(this)?.call_method1("curve_keys", (cid,))?,
        )
    }
}
#[pyfunction]
fn grow_pick<'p>(this: &O<'p>, entity: &O<'p>, grow: &O<'p>) -> PyResult<O<'p>> {
    let key = entity.getattr("key")?;
    let picked = st(this)?.call_method1("tool_selection", (&key, grow))?;
    if picked.is_none() {
        return Ok(picked);
    }
    let retain = !grow.eq("connected")?
        && (entity.is_instance(&api(this.py())?.getattr("Face")?)?
            || entity.getattr("curve")?.is_none());
    let out = PyList::empty(this.py());
    for (i, n) in ["EdgeView", "FaceView"].iter().enumerate() {
        for k in picked.get_item(i)?.try_iter()? {
            let k = k?;
            out.append(if retain && key.eq(&k)? {
                entity.clone()
            } else {
                view(this, n, &k)?
            })?;
        }
    }
    Ok(out.into_any())
}
#[pyfunction]
fn surface_of<'p>(this: &O<'p>, face: &O<'p>) -> PyResult<O<'p>> {
    let v = this.call_method1("grow_pick", (face, "natural"))?;
    Ok(if v.is_none() {
        PyList::new(this.py(), [face])?.into_any()
    } else {
        v
    })
}
fn mark(this: &O<'_>) -> PyResult<()> {
    this.setattr("_chunk_dirty", true)?;
    this.setattr("_mut_serial", this.getattr("_mut_serial")?.add(1)?)
}
#[pyfunction]
fn bump(this: &O<'_>) -> PyResult<()> {
    mark(this)?;
    forget_bulk(this)
}
#[pyfunction]
fn input_point<'p>(this: &O<'p>, position: &O<'p>) -> PyResult<O<'p>> {
    let p = _xyz(position)?;
    let d = this
        .getattr("__dict__")?
        .call_method1("get", ("_planned_points",))?;
    if d.is_truthy()? {
        d.call_method1("get", (&p, &p))
    } else {
        Ok(p)
    }
}
#[pyfunction]
fn vertex_at<'p>(this: &O<'p>, position: &O<'p>) -> PyResult<O<'p>> {
    maybe_view(
        this,
        "VertexView",
        st(this)?.call_method1("vertex_at", (input_point(this, position)?,))?,
    )
}
#[pyfunction]
fn registers(this: &O<'_>, v: &O<'_>) -> PyResult<bool> {
    let py = this.py();
    match st(this)?.call_method1("registers", (v.getattr("key")?,)) {
        Ok(x) => x.is_truthy(),
        Err(e)
            if e.is_instance_of::<PyOverflowError>(py) || e.is_instance_of::<PyTypeError>(py) =>
        {
            Ok(false)
        }
        Err(e) => Err(e),
    }
}
#[pyfunction]
fn find_edge<'p>(this: &O<'p>, v0: &O<'p>, v1: &O<'p>) -> PyResult<O<'p>> {
    let py = this.py();
    if v0.is_none() || v1.is_none() {
        return Ok(none(py));
    }
    match st(this)?.call_method1("find_edge", (v0.getattr("key")?, v1.getattr("key")?)) {
        Ok(k) => maybe_view(this, "EdgeView", k),
        Err(e)
            if e.is_instance_of::<PyKeyError>(py)
                || e.is_instance_of::<PyOverflowError>(py)
                || e.is_instance_of::<PyTypeError>(py) =>
        {
            Ok(none(py))
        }
        Err(e) => Err(e),
    }
}
#[pyfunction]
fn edges_at<'p>(_this: &O<'p>, v: &O<'p>) -> PyResult<O<'p>> {
    builtin(v.py(), "set")?.call1((v.getattr("edges")?,))
}
#[pyfunction]
fn faces_at<'p>(_this: &O<'p>, v: &O<'p>) -> PyResult<O<'p>> {
    v.call_method0("faces")
}
#[pyfunction]
fn move_vertex(this: &O<'_>, v: &O<'_>, delta: &O<'_>) -> PyResult<()> {
    bump(this)?;
    st(this)?.call_method1("move_vertex", (v.getattr("key")?, _xyz(delta)?))?;
    Ok(())
}
#[pyfunction]
fn place_vertex(this: &O<'_>, v: &O<'_>, position: &O<'_>) -> PyResult<()> {
    bump(this)?;
    st(this)?.call_method1("place_vertex", (v.getattr("key")?, _xyz(position)?))?;
    Ok(())
}
#[pyfunction]
fn vertices_in_cells<'p>(this: &O<'p>, cells: &O<'p>, keyfn: &O<'p>) -> PyResult<O<'p>> {
    kernel(this.py())?.call_method1(
        "vertices_in_cells",
        (
            this,
            st(this)?,
            cells,
            keyfn,
            store_api(this.py())?.getattr("VertexView")?,
        ),
    )
}
#[pyfunction]
fn cells_of<'p>(this: &O<'p>, vertex_keys: &O<'p>, keyfn: &O<'p>) -> PyResult<O<'p>> {
    kernel(this.py())?.call_method1("cells_of", (st(this)?, vertex_keys, keyfn))
}
#[pyfunction]
fn textured_faces<'p>(this: &O<'p>) -> PyResult<O<'p>> {
    let cols = kernel(this.py())?.call_method1("attr_columns", (this,))?;
    let mut rows = std::collections::HashSet::new();
    for (i, a) in cols.get_item(0)?.try_iter()?.enumerate() {
        let a = a?;
        if a.is_truthy()? && a.call_method1("get", ("texture",))?.is_truthy()? {
            rows.insert(i as i64);
        }
    }
    let out = PyList::empty(this.py());
    if !rows.is_empty() {
        let keys = st(this)?.call_method1("keys", ("face",))?;
        for (i, r) in cols
            .get_item(1)?
            .call_method0("tolist")?
            .try_iter()?
            .enumerate()
        {
            if rows.contains(&r?.extract::<i64>()?) {
                out.append(view(this, "FaceView", &keys.get_item(i)?)?)?;
            }
        }
    }
    Ok(out.into_any())
}
#[pyfunction]
fn nonplanar_faces<'p>(this: &O<'p>, tolerance: &O<'p>) -> PyResult<O<'p>> {
    views_list(
        this,
        "FaceView",
        &st(this)?.call_method1("nonplanar_faces", (tolerance,))?,
    )
}
#[pyfunction]
fn inherit<'p>(this: &O<'p>, dirty: &O<'p>, replacements: &O<'p>) -> PyResult<O<'p>> {
    if dirty.is_truthy()? {
        this.call_method1("_dirty", (dirty,))?;
    }
    let sources = PyList::empty(this.py());
    for row in replacements.try_iter()? {
        let r = row?;
        let source = view(this, "FaceView", &r.get_item(0)?)?;
        sources.append(&source)?;
        for k in r.get_item(1)?.try_iter()? {
            view(this, "FaceView", &k?)?.setattr(
                "attrs",
                builtin(this.py(), "dict")?.call1((source.getattr("attrs")?,))?,
            )?;
        }
    }
    Ok(sources.into_any())
}
#[pyfunction]
fn live<'p>(this: &O<'p>, x: &O<'p>, kind: usize) -> PyResult<O<'p>> {
    kernel(this.py())?.call_method1("owned", (this, x, kind, true))
}
#[pyfunction]
fn incidence_key<'p>(this: &O<'p>, element: &O<'p>, kind: usize) -> PyResult<O<'p>> {
    let (n, v, _) = KINDS[kind];
    if !element.is_instance(&store_api(this.py())?.getattr(v)?)?
        || live(this, element, kind)?.is_none()
    {
        return Err(PyValueError::new_err(format!(
            "incidence requires a live {n} from this mesh"
        )));
    }
    element.getattr("key")
}
#[pyfunction]
fn vertex<'p>(this: &O<'p>, position: &O<'p>) -> PyResult<O<'p>> {
    forget_bulk(this)?;
    view(
        this,
        "VertexView",
        &st(this)?
            .call_method1("vertex", (input_point(this, position)?,))?
            .get_item(0)?,
    )
}
#[pyfunction]
fn link_edge<'p>(this: &O<'p>, v0: &O<'p>, v1: &O<'p>) -> PyResult<O<'p>> {
    let a = live(this, v0, 0)?;
    let b = live(this, v1, 0)?;
    if a.is_none() || b.is_none() {
        return Err(refuse(this, "_link_edge")?);
    }
    this.call_method1("_dirty", (1,))?;
    view(
        this,
        "EdgeView",
        &st(this)?.call_method1("link_edge", (a, b))?.get_item(0)?,
    )
}
#[pyfunction]
fn add_edge<'p>(this: &O<'p>, a: &O<'p>, b: &O<'p>) -> PyResult<O<'p>> {
    forget_bulk(this)?;
    let k = st(this)?
        .call_method1("add_edge", (input_point(this, a)?, input_point(this, b)?))?
        .get_item(0)?;
    this.call_method1("_dirty", (1,))?;
    view(this, "EdgeView", &k)
}
#[pyfunction(signature=(this,loop_positions,hole_loops=None))]
fn add_face<'p>(
    this: &O<'p>,
    loop_positions: &O<'p>,
    hole_loops: Option<&O<'p>>,
) -> PyResult<O<'p>> {
    kernel(this.py())?.call_method1("add_face", (this, loop_positions, hole_loops))
}
#[pyfunction]
fn edit_element<'p>(name: &str, kind: u8, this: &O<'p>, element: &O<'p>) -> PyResult<O<'p>> {
    kernel(this.py())?.call_method1(name, (this, element, kind))
}
#[pyfunction]
fn relink_hole(this: &O<'_>, face: &O<'_>, ring: &O<'_>) -> PyResult<()> {
    let ks = ring
        .try_iter()?
        .map(|v| v?.getattr("key"))
        .collect::<PyResult<Vec<_>>>()?;
    st(this)?.call_method1("relink_hole", (face.getattr("key")?, ks))?;
    forget_bulk(this)
}
#[pyfunction]
fn purge_debris<'p>(this: &O<'p>) -> PyResult<O<'p>> {
    let r = st(this)?.call_method0("purge_debris")?;
    let n = r.get_item(0)?;
    let g = r.get_item(1)?;
    if n.is_truthy()? || g.is_truthy()? {
        this.call_method1("_dirty", (1,))?;
    }
    Ok((n, g).into_pyobject(this.py())?.into_any())
}
#[pyfunction]
fn prune<'p>(this: &O<'p>) -> PyResult<O<'p>> {
    purge_debris(this)?.get_item(0)
}
#[pyfunction]
fn edit_vertices<'p>(name: &str, this: &O<'p>, vertices: &O<'p>) -> PyResult<O<'p>> {
    let keys = PyList::empty(this.py());
    let drop = name == "drop_orphan_vertices";
    for v in vertices.try_iter()? {
        let k = this.call_method1("_own", (v?, 0))?;
        if !k.is_none() {
            keys.append(k)?;
        } else if !drop {
            return Err(refuse(this, name)?);
        }
    }
    let r = match st(this)?.call_method1(name, (keys,)) {
        Err(e) if drop && e.is_instance_of::<StoreFallback>(this.py()) => {
            return Err(refuse(this, name)?)
        }
        r => r?,
    };
    if drop {
        let n = r.get_item(0)?;
        if n.is_truthy()? {
            this.call_method1("_dirty", (1,))?;
        }
        Ok(n)
    } else {
        this.call_method1("_dirty", (1,))?;
        Ok(none(this.py()))
    }
}
#[pyfunction]
fn undo_state<'p>(this: &O<'p>) -> PyResult<O<'p>> {
    let d = _attribute_owner(this)?;
    store_api(this.py())?.call_method1(
        "StoreState",
        (
            st(this)?.call_method0("snapshot")?,
            d.get_item("_attr_own")?,
            d.get_item("_attr_table")?,
            this,
        ),
    )
}
#[pyfunction(signature=(this,keys,radius,segments=8,profile=0.5,measure="radius",corners="round",shape=None))]
fn fillet_edges<'p>(
    this: &O<'p>,
    keys: &O<'p>,
    radius: &O<'p>,
    segments: usize,
    profile: f64,
    measure: &str,
    corners: &str,
    shape: Option<&O<'p>>,
) -> PyResult<O<'p>> {
    let plan = module(this.py(), "built_core")?.call_method1(
        "plan_fillet",
        (
            this,
            views_list(this, "EdgeView", keys)?,
            radius,
            segments,
            profile,
            measure,
            corners,
            shape,
        ),
    )?;
    this.call_method1("_apply_round_plan", (plan,))
}
#[pyfunction(signature=(this,keys,size,segments=8,profile=0.5))]
fn bevel_vertices<'p>(
    this: &O<'p>,
    keys: &O<'p>,
    size: &O<'p>,
    segments: usize,
    profile: f64,
) -> PyResult<O<'p>> {
    let plan = module(this.py(), "built_core")?.call_method1(
        "plan_vertex_bevel",
        (
            this,
            views_list(this, "VertexView", keys)?,
            size,
            segments,
            profile,
        ),
    )?;
    this.call_method1("_apply_round_plan", (plan,))
}
#[pyfunction]
fn apply_plan<'p>(this: &O<'p>, plan: &O<'p>) -> PyResult<O<'p>> {
    if plan.is_instance_of::<pyo3::types::PyString>() {
        return Err(PyValueError::new_err(plan.extract::<String>()?));
    }
    module(this.py(), "built_core")?.call_method1("apply_fillet", (this, plan))
}
#[pyfunction]
fn interior_vertex_on<'p>(this: &O<'p>, edge: &O<'p>) -> PyResult<O<'p>> {
    let k = this.call_method1("_own", (edge, 1))?;
    if k.is_none() {
        return Ok(k);
    }
    maybe_view(
        this,
        "VertexView",
        st(this)?.call_method1("interior_vertex_on", (k, _hypot(this.py())?))?,
    )
}
#[pyfunction]
fn collapsible_vertex(this: &O<'_>, v: &O<'_>) -> PyResult<bool> {
    let k = this.call_method1("_own", (v, 0))?;
    Ok(!k.is_none()
        && st(this)?
            .call_method1("collapsible_vertex", (k, _hypot(this.py())?))?
            .is_truthy()?)
}
#[pyfunction]
fn collapse_collinear<'p>(this: &O<'p>) -> PyResult<O<'p>> {
    let r = st(this)?
        .call_method1("collapse_collinear", (_hypot(this.py())?,))?
        .get_item(0)?;
    this.call_method1("_dirty", (r.get_item(1)?,))?;
    r.get_item(0)
}
#[pyfunction]
fn weld_into(this: &O<'_>, keep: &O<'_>, dup: &O<'_>) -> PyResult<()> {
    let a = live(this, keep, 0)?;
    let b = live(this, dup, 0)?;
    if a.is_none() || b.is_none() {
        return Err(refuse(this, "_weld_into")?);
    }
    let r = match st(this)?.call_method1("weld_into", (a, b)) {
        Err(e) if e.is_instance_of::<StoreFallback>(this.py()) => {
            return Err(refuse(this, "_weld_into")?)
        }
        r => r?,
    };
    this.call_method1("_dirty", (r.get_item(0)?,))?;
    Ok(())
}
#[pyfunction]
fn weld_coincident<'p>(this: &O<'p>) -> PyResult<O<'p>> {
    let r = match st(this)?.call_method0("weld_coincident") {
        Err(e) if e.is_instance_of::<StoreFallback>(this.py()) => {
            return Err(PyErr::from_value(store_api(this.py())?.call_method1(
                "StoreRefusal",
                ("coincident weld cannot preserve incidence",),
            )?))
        }
        r => r?,
    }
    .get_item(0)?;
    this.call_method1("_dirty", (r.get_item(1)?,))?;
    r.get_item(0)
}
#[pyfunction]
fn finish_push<'p>(
    this: &O<'p>,
    before: &O<'p>,
    face: &O<'p>,
    shift: &O<'p>,
    facet_cos: &O<'p>,
    painted: &O<'p>,
) -> PyResult<O<'p>> {
    let m = api(this.py())?;
    let r = st(this)?.call_method1(
        "finish_push",
        (
            before,
            face,
            shift,
            facet_cos,
            m.getattr("_CURVE_COUNTER")?,
            painted,
            _hypot(this.py())?,
        ),
    )?;
    m.setattr("_CURVE_COUNTER", r.get_item(0)?)?;
    views_list(this, "FaceView", &r.get_item(1)?)
}
#[pyfunction]
fn dissolve<'p>(tolerant: bool, this: &O<'p>, edge: &O<'p>) -> PyResult<O<'p>> {
    kernel(this.py())?.call_method1("dissolve_edge", (this, edge, tolerant))
}
#[pyfunction]
fn revert_stitch(this: &O<'_>, rec: &O<'_>) -> PyResult<()> {
    let s = rec.call_method1("get", ("_undo",))?;
    if !s.is_instance(&store_api(this.py())?.getattr("StoreState")?)?
        || !s.getattr("mesh")?.is(this)
    {
        return Err(PyValueError::new_err(
            "stitch record has no snapshot of this mesh",
        ));
    }
    this.call_method1("restore_state", (s,))?;
    Ok(())
}
#[pyfunction(signature=(this,loop_points,closed=true))]
fn tag_curve<'p>(this: &O<'p>, loop_points: &O<'p>, closed: bool) -> PyResult<O<'p>> {
    mark(this)?;
    let points = list(loop_points)?;
    let st = st(this)?;
    let m = api(this.py())?;
    let cid = m.getattr("_CURVE_COUNTER")?;
    let matched = if st.getattr("_model_frame")?.is_truthy()? {
        let xyz = points
            .try_iter()?
            .map(|p| p?.call_method0("toTuple"))
            .collect::<PyResult<Vec<_>>>()?;
        st.call_method1("tag_curve_path", (xyz, closed, cid))?
    } else {
        m.getattr("object_drawing")?
            .call_method1("tag_curve", (this.getattr("edges")?, points, closed, cid))?
    };
    if matched.is_truthy()? {
        this.call_method0("next_curve_id")
    } else {
        Ok(none(this.py()))
    }
}
#[pyfunction]
fn resplit_curves(this: &O<'_>) -> PyResult<()> {
    mark(this)?;
    let m = api(this.py())?;
    let st = st(this)?;
    let c = m.getattr("_CURVE_COUNTER")?;
    let n = if st.getattr("_model_frame")?.is_truthy()? {
        st.call_method1("resplit_curves", (c,))?
    } else {
        m.getattr("object_drawing")?
            .call_method1("resplit_curves", (this.getattr("edges")?, c))?
    };
    m.setattr("_CURVE_COUNTER", n)
}
#[pyfunction]
fn clear(this: &O<'_>) -> PyResult<()> {
    st(this)?.call_method0("clear")?;
    let d = _attribute_owner(this)?;
    d.get_item("_attr_own")?.call_method0("clear")?;
    d.set_item("_attr_table", PyList::empty(this.py()))?;
    this.call_method0("_dirty")?;
    Ok(())
}
#[pyfunction]
fn reduce<'p>(this: &O<'p>, _protocol: &O<'p>) -> PyResult<O<'p>> {
    kernel(this.py())?.call_method1("reduce_mesh", (this,))
}
#[pyfunction]
fn first_get<'p>(name: &str, this: &O<'p>) -> PyResult<O<'p>> {
    this.call_method0("_materialize")?;
    this.getattr(name)
}
#[pyfunction]
fn first_put(name: &str, this: &O<'_>, value: &O<'_>) -> PyResult<()> {
    this.call_method0("_materialize")?;
    this.setattr(name, value)
}
#[pyfunction(signature=(name,this,*args,**kw))]
fn first_call<'p>(
    name: &str,
    this: &O<'p>,
    args: &Bound<'p, PyTuple>,
    kw: Option<&D<'p>>,
) -> PyResult<O<'p>> {
    if this.is_instance(&store_api(this.py())?.getattr("DeferredMesh")?)? {
        this.call_method0("_materialize")?;
    }
    this.call_method(name, args, kw)
}
pub(super) fn install(s: &M<'_>, m: &M<'_>, _core: &M<'_>, k: &O<'_>) -> PyResult<()> {
    let py = s.py();
    let ns = PyDict::new(py);
    for (i, name) in ["vertices", "edges", "faces"].iter().enumerate() {
        property(
            &ns,
            name,
            &partial(wrap_pyfunction!(sequence, s)?.as_any(), i)?,
            Some(&partial(wrap_pyfunction!(replace, s)?.as_any(), i)?),
        )?;
    }
    getter!(ns, s, "_registry", registry);
    getter!(ns, s, "_displaced", displaced);
    for (name, func) in [
        ("__init__", wrap_pyfunction!(init, s)?),
        ("_bump", wrap_pyfunction!(bump, s)?),
        ("_input_point", wrap_pyfunction!(input_point, s)?),
        ("_inherit_face_attrs", wrap_pyfunction!(inherit, s)?),
        ("_incidence_key", wrap_pyfunction!(incidence_key, s)?),
        ("_live", wrap_pyfunction!(live, s)?),
        ("_link_edge", wrap_pyfunction!(link_edge, s)?),
        ("_undo_state", wrap_pyfunction!(undo_state, s)?),
        ("_apply_round_plan", wrap_pyfunction!(apply_plan, s)?),
        ("_weld_into", wrap_pyfunction!(weld_into, s)?),
        ("prune_orphan_vertices", wrap_pyfunction!(prune, s)?),
        ("__reduce_ex__", wrap_pyfunction!(reduce, s)?),
    ] {
        bind(&ns, name, func.as_any())?;
    }
    method!(ns, s, "curve_edges", curve_edges);
    method!(ns, s, "grow_pick", grow_pick);
    method!(ns, s, "surface_of", surface_of);
    method!(ns, s, "vertex_at", vertex_at);
    method!(ns, s, "registers", registers);
    method!(ns, s, "find_edge", find_edge);
    method!(ns, s, "edges_at", edges_at);
    method!(ns, s, "faces_at", faces_at);
    method!(ns, s, "move_vertex", move_vertex);
    method!(ns, s, "place_vertex", place_vertex);
    method!(ns, s, "vertices_in_cells", vertices_in_cells);
    method!(ns, s, "cells_of", cells_of);
    method!(ns, s, "textured_faces", textured_faces);
    method!(ns, s, "nonplanar_faces", nonplanar_faces);
    method!(ns, s, "vertex", vertex);
    method!(ns, s, "add_edge", add_edge);
    method!(ns, s, "add_face", add_face);
    method!(ns, s, "relink_hole", relink_hole);
    method!(ns, s, "purge_debris", purge_debris);
    method!(ns, s, "fillet_edges", fillet_edges);
    method!(ns, s, "bevel_vertices", bevel_vertices);
    method!(ns, s, "interior_vertex_on", interior_vertex_on);
    method!(ns, s, "collapsible_vertex", collapsible_vertex);
    method!(ns, s, "collapse_collinear", collapse_collinear);
    method!(ns, s, "weld_coincident", weld_coincident);
    method!(ns, s, "finish_push", finish_push);
    method!(ns, s, "revert_stitch", revert_stitch);
    method!(ns, s, "tag_curve", tag_curve);
    method!(ns, s, "resplit_curves", resplit_curves);
    method!(ns, s, "clear", clear);
    for n in "add_hole remove_hole split_edge split_edge_at collapse_vertex rebuild_cap_plane push_extrude push_translate stitch_connect dedupe_faces dissolve_coplanar_pair dissolve_coplanar_region stitch_arrays capture_state restore_state bulk_weld add_edges_welded add_faces_welded".split_whitespace(){bind(&ns,n,&k.getattr(n)?)?;}
    for (name, f) in [
        ("_own", "owned"),
        ("_dirty", "dirty"),
        ("_journal_state", "capture_state"),
        ("__deepcopy__", "deepcopy_mesh"),
        ("__setstate__", "set_mesh_state"),
    ] {
        bind(&ns, name, &k.getattr(f)?)?;
    }
    for (name, f, i) in [
        ("remove_edge", "remove_element", 1),
        ("remove_face", "remove_element", 2),
        ("relink_edge", "relink_element", 1),
        ("relink_face", "relink_element", 2),
    ] {
        let func = py
            .import("functools")?
            .call_method1("partial", (wrap_pyfunction!(edit_element, s)?, f, i))?;
        bind(&ns, name, &func)?;
    }
    for n in ["drop_orphan_vertices", "relink_vertices"] {
        bind(
            &ns,
            n,
            &partial(wrap_pyfunction!(edit_vertices, s)?.as_any(), n)?,
        )?;
    }
    for (n, t) in [("dissolve_edge", true), ("dissolve_edge_recorded", false)] {
        bind(
            &ns,
            n,
            &partial(wrap_pyfunction!(dissolve, s)?.as_any(), t)?,
        )?;
    }
    let state = m.getattr("_MeshState")?;
    let store = class(s, "StoreMesh", &[state.clone()], &ns)?;
    let deferred = PyDict::new(py);
    for n in [
        "_store",
        "vertices",
        "edges",
        "faces",
        "_registry",
        "_displaced",
    ] {
        property(
            &deferred,
            n,
            &partial(wrap_pyfunction!(first_get, s)?.as_any(), n)?,
            Some(&partial(wrap_pyfunction!(first_put, s)?.as_any(), n)?),
        )?;
    }
    deferred.set_item(
        "defer",
        builtin(py, "staticmethod")?.call1((k.getattr("defer")?,))?,
    )?;
    bind(&deferred, "_materialize", &k.getattr("materialize")?)?;
    for base in [&state, &store] {
        for k in base.getattr("__dict__")?.try_iter()? {
            let n = k?.extract::<String>()?;
            if matches!(n.as_str(), "__deepcopy__" | "__reduce_ex__")
                || (!n.starts_with("__")
                    && !matches!(
                        n.as_str(),
                        "bump_revision"
                            | "mark_modified"
                            | "invalidate_placement_chunks"
                            | "next_curve_id"
                    )
                    && base.getattr(n.as_str())?.is_callable())
            {
                bind(
                    &deferred,
                    &n,
                    &partial(wrap_pyfunction!(first_call, s)?.as_any(), n.as_str())?,
                )?;
            }
        }
    }
    class(s, "DeferredMesh", &[store], &deferred)?;
    Ok(())
}
