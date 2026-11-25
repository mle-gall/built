// SPDX-License-Identifier: GPL-3.0-or-later
//! The drawing of a lazy mesh, baked from its arrays (`views.array_bake`):
//! the chunk entries the picks and the hidden lines read, the faces of a
//! still-lazy mesh read from its plan (`LightFace`), and the lazy
//! sequences a pick index is made of (render-glue lane; from
//! `views/array_bake.py`).
//!
//! The bake itself is `ingetrazo_core.bake_deferred_chunk`
//! (`resident/chunk.rs`), here as the module's `_bake`.
use super::*;
use numpy::PyReadonlyArray2;
use pyo3::exceptions::{PyException, PyIndexError};
use pyo3::types::{PyCFunction, PySlice};

fn attr<'py>(o: &Obj<'py>, name: &str) -> Reply<'py> {
    optional(o, name)
}
fn id(o: &Obj<'_>) -> usize {
    o.as_ptr() as usize
}
fn bake<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyModule>> {
    py.import("views.array_bake")
}
fn np<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyModule>> {
    py.import("numpy")
}
fn int(value: Obj<'_>) -> PyResult<isize> {
    value.call_method0("__int__")?.extract()
}
fn span<'py>(py: Python<'py>, from: isize, to: isize) -> Bound<'py, PySlice> {
    PySlice::new(py, from, to, 1)
}
fn deferred<'py>(py: Python<'py>) -> Reply<'py> {
    bake(py)?.getattr("DeferredMesh")
}
fn is_deferred(mesh: &Obj<'_>) -> PyResult<bool> {
    Ok(mesh.get_type().is(&deferred(mesh.py())?))
}
fn plan_of<'py>(mesh: &Obj<'py>) -> Reply<'py> {
    bake(mesh.py())?.getattr("lazy_mesh")?.call_method1("plan_of", (mesh,))
}
/// A worker for a thread pool: it owns what it captures until the pool ends.
fn worker<'py, F>(py: Python<'py>, run: F) -> PyResult<Bound<'py, PyCFunction>>
where
    F: Fn(&Bound<'_, PyAny>) -> PyResult<()> + Send + Sync + 'static,
{
    PyCFunction::new_closure(py, None, None, move |args: &Bound<'_, PyTuple>, _: Option<&Bound<'_, PyDict>>| -> PyResult<()> {
        // The bake will retry what fails here.
        match run(&args.get_item(0)?) {
            Err(error) if error.is_instance_of::<PyException>(args.py()) => Ok(()),
            other => other,
        }
    })
}
/// Run `work` over `items` side by side, at most eight at a time.
fn side_by_side<'py>(py: Python<'py>, work: Bound<'py, PyCFunction>, items: impl IntoPyObject<'py>) -> PyResult<()> {
    let cpus = py.import("os")?.call_method0("cpu_count")?;
    let workers = if cpus.is_truthy()? { cpus.extract::<usize>()?.min(8) } else { 2 };
    let pool = py.import("concurrent.futures")?.getattr("ThreadPoolExecutor")?.call((), Some(&fields!(py; "max_workers" => workers)))?;
    let ran = pool.call_method1("map", (work, items)).and_then(|results| py.get_type::<PyList>().call1((results,)));
    pool.call_method0("shutdown")?;
    ran.map(drop)
}

/// Whether `mesh` is deferred and this engine can bake it as is.
fn can_bake<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let native = bake(py)?.getattr("_native")?;
    (is_deferred(&a.get_item(0)?)? && !native.getattr("native")?.is_none() && !native.call_method0("qt_hypot")?.is_none()).into_bound_py_any(py)
}

// ---- reading_only -----------------------------------------------------------

fn reading_enter<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = bake(py)?;
    a.get_item(0)?.setattr("_was", module.getattr("_reading")?)?;
    module.setattr("_reading", true)?;
    Ok(none(py))
}
#[pyfunction(signature = (this, *exc))]
fn reading_exit(this: &Obj<'_>, exc: &Bound<'_, PyTuple>) -> PyResult<()> {
    let _ = exc;
    bake(this.py())?.setattr("_reading", this.getattr("_was")?)
}
fn reading_now<'py>(a: &Args<'py>) -> Reply<'py> {
    bake(a.py())?.getattr("_reading")
}

// ---- LightFace --------------------------------------------------------------

fn light_real<'py>(a: &Args<'py>) -> Reply<'py> {
    let this = a.get_item(0)?;
    this.getattr("mesh")?.getattr("faces")?.get_item(this.getattr("index")?)
}

/// Where each ring starts among the corners and each face among the rings,
/// memoised on the plan.
fn ring_starts<'py>(plan: &Obj<'py>) -> PyResult<(Obj<'py>, Obj<'py>)> {
    let py = plan.py();
    let extra = plan.getattr("extra")?;
    let mut rings = extra.call_method1("get", ("ring_start",))?;
    if rings.is_none() {
        let numpy = np(py)?;
        let starts = |sizes: Obj<'py>| numpy.call_method1("concatenate", ((PyList::new(py, [0])?, numpy.call_method1("cumsum", (sizes,))?),));
        rings = starts(plan.getattr("ring_sizes")?)?;
        extra.set_item("ring_start", &rings)?;
        extra.set_item("face_ring", starts(plan.getattr("ring_counts")?)?)?;
    }
    Ok((rings, extra.get_item("face_ring")?))
}

/// `(start, edge)`: the edges incident to each face, CSR by face — the
/// inverse of the plan's edge → faces table. Memoised on the plan.
fn face_edges<'py>(plan: &Obj<'py>) -> PyResult<(Obj<'py>, Obj<'py>)> {
    let py = plan.py();
    let extra = plan.getattr("extra")?;
    let mut hit = extra.call_method1("get", ("face_edges",))?;
    if hit.is_none() {
        let numpy = np(py)?;
        let faces = plan.getattr("ef_face")?;
        let per_edge = numpy.call_method1("diff", (plan.getattr("ef_start")?,))?;
        let edge_of = numpy.call_method1("repeat", (numpy.call_method1("arange", (plan.getattr("edges")?.len()?,))?, per_edge))?;
        let order = numpy.call_method("argsort", (&faces,), Some(&fields!(py; "kind" => "stable")))?;
        let counts = numpy.call_method("bincount", (&faces,), Some(&fields!(py; "minlength" => plan.getattr("nfaces")?)))?;
        let starts = numpy.call_method1("concatenate", ((PyList::new(py, [0])?, numpy.call_method1("cumsum", (counts,))?),))?;
        hit = (starts, edge_of.get_item(order)?).into_bound_py_any(py)?;
        extra.set_item("face_edges", &hit)?;
    }
    Ok((hit.get_item(0)?, hit.get_item(1)?))
}

/// Face `f` of lazy `mesh` as a `LightFace`, the same object while the mesh
/// is lazy: the picks of a hover tell faces apart by identity.
fn light_face<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (mesh, face) = (a.get_item(0)?, a.get_item(1)?);
    let module = bake(py)?;
    let plan = plan_of(&mesh)?;
    let memo = plan.getattr("extra")?.call_method1("setdefault", ("light_faces", PyDict::new(py)))?;
    let hit = memo.call_method1("get", (&face,))?;
    if !hit.is_none() {
        return Ok(hit);
    }
    let at = int(face.clone())?;
    let (ring_start, face_ring) = ring_starts(&plan)?;
    let (vertex, vector) = (module.getattr("Vertex")?, module.getattr("QVector3D")?);
    let (corners, positions) = (plan.getattr("corners")?, plan.getattr("pos")?);
    let vertices = PyDict::new(py);
    let rings = PyList::empty(py);
    for ring in int(face_ring.get_item(at)?)?..int(face_ring.get_item(at + 1)?)? {
        let members = PyList::empty(py);
        let indices = corners.get_item(span(py, int(ring_start.get_item(ring)?)?, int(ring_start.get_item(ring + 1)?)?))?.call_method0("tolist")?;
        for index in indices.try_iter()? {
            let index = index?;
            let known = match vertices.get_item(&index)? {
                Some(known) => known,
                None => {
                    let made = vertex.call1((vector.call1(positional(&positions.get_item(&index)?)?)?,))?;
                    vertices.set_item(&index, &made)?;
                    made
                }
            };
            members.append(known)?;
        }
        rings.append(members)?;
    }
    let holes = rings.get_slice(1, rings.len());
    let out = module.getattr("LightFace")?.call1((rings.get_item(0)?, if holes.is_empty() { none(py) } else { holes.into_any() }))?;
    out.setattr("mesh", &mesh)?;
    out.setattr("index", &face)?;
    let (starts, edge_of) = face_edges(&plan)?;
    let edge = py.import("ingetrazo_core.mesh")?.getattr("Edge")?;
    let flag = |name: &str, index: &Obj<'py>| -> PyResult<bool> { plan.getattr(name)?.get_item(index)?.is_truthy() };
    let flat = plan.getattr("flat")?;
    for index in edge_of.get_item(span(py, int(starts.get_item(at)?)?, int(starts.get_item(at + 1)?)?))?.call_method0("tolist")?.try_iter()? {
        let index = index?;
        let [from, to] = unpack::<2>(&plan.getattr("edges")?.get_item(&index)?.call_method0("tolist")?)?;
        let (Some(from), Some(to)) = (vertices.get_item(from)?, vertices.get_item(to)?) else {
            continue;
        };
        let made = edge.call1((&from, &to))?;
        made.setattr("soft", flag("soft", &index)?)?;
        made.setattr("flat", !flat.is_none() && flat.get_item(&index)?.is_truthy()?)?;
        made.setattr("hidden", flag("hidden", &index)?)?;
        made.setattr("curve", plan.getattr("curve")?.get_item(&index)?)?;
        made.setattr("layer", plan.getattr("layer")?.get_item(&index)?)?;
        from.getattr("edges")?.call_method1("add", (&made,))?;
        to.getattr("edges")?.call_method1("add", (&made,))?;
    }
    let source = mesh.getattr("__dict__")?.get_item("_deferred_source")?;
    let row = int(source.get_item("face_attr")?.get_item(at)?)?;
    if row >= 0 {
        let raw = source.get_item("meta")?.call_method1("get", ("face_attrs",))?;
        let own = py.import("formats.igz")?.call_method1("_face_attrs_from_json", (raw.get_item(row)?,))?;
        if own.is_truthy()? {
            out.getattr("attrs")?.call_method1("update", (own,))?;
        }
    }
    if memo.len()? > 4096 {
        // A hover's worth is what matters.
        memo.call_method0("clear")?;
    }
    memo.set_item(face, &out)?;
    Ok(out)
}

// ---- DeferredFaces ----------------------------------------------------------

fn faces_init<'py>(a: &Args<'py>) -> Reply<'py> {
    let this = a.get_item(0)?;
    this.setattr("mesh", a.get_item(1)?)?;
    this.setattr("index", a.get_item(2)?)?; // drawn face → face of the mesh
    Ok(none(a.py()))
}
fn faces_len<'py>(a: &Args<'py>) -> Reply<'py> {
    a.get_item(0)?.getattr("index")?.len()?.into_bound_py_any(a.py())
}
fn faces_getitem<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (this, at) = (a.get_item(0)?, a.get_item(1)?);
    let (mesh, index) = (this.getattr("mesh")?, this.getattr("index")?);
    if at.is_instance_of::<PySlice>() {
        let faces = mesh.getattr("faces")?;
        let out = PyList::empty(py);
        for face in index.get_item(at)?.call_method0("tolist")?.try_iter()? {
            out.append(faces.get_item(face?)?)?;
        }
        return Ok(out.into_any());
    }
    let face = index.get_item(at)?.call_method0("__int__")?;
    let module = bake(py)?;
    if module.getattr("_reading")?.is_truthy()? && is_deferred(&mesh)? {
        return module.call_method1("light_face", (mesh, face));
    }
    mesh.getattr("faces")?.get_item(face)
}
fn faces_iter<'py>(a: &Args<'py>) -> Reply<'py> {
    let this = a.get_item(0)?;
    let faces = this.getattr("mesh")?.getattr("faces")?;
    a.py().import("builtins")?.call_method1("map", (faces.getattr("__getitem__")?, this.getattr("index")?.call_method0("tolist")?))
}
/// The layer the `i`-th drawn face carries (`None`: none of its own), read
/// from the document while the mesh is still unbuilt: the section cut asks
/// it of every face it crosses, and asking for the face built the whole
/// mesh — 140 s of a first cut on a big model.
fn faces_layer_at<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let this = a.get_item(0)?;
    let mesh = this.getattr("mesh")?;
    let face = int(this.getattr("index")?.get_item(a.get_item(1)?)?)?;
    let own = |attrs: Obj<'py>| -> Reply<'py> {
        if attrs.is_truthy()? { attrs.call_method1("get", ("layer",)) } else { Ok(none(py)) }
    };
    if !mesh.get_type().is(&py.import("ingetrazo_core.mesh")?.getattr("DeferredMesh")?) {
        return own(mesh.getattr("faces")?.get_item(face)?.getattr("attrs")?);
    }
    let source = mesh.getattr("__dict__")?.get_item("_deferred_source")?;
    let row = int(source.get_item("face_attr")?.get_item(face)?)?;
    if row < 0 {
        return Ok(none(py));
    }
    let layer = source.get_item("meta")?.call_method1("get", ("face_attrs",))?.get_item(row)?.call_method1("get", ("layer",))?;
    Ok(if layer.is_truthy()? { layer } else { none(py) })
}

#[rustfmt::skip]
const DEFERRED_FACES: &[Method] = &[
    ("__init__", Mode::Method, faces_init, &["self", "mesh", "index"], "", ""),
    ("__len__", Mode::Method, faces_len, &["self"], "", ""),
    ("__getitem__", Mode::Method, faces_getitem, &["self", "i"], "", ""),
    ("__iter__", Mode::Method, faces_iter, &["self"], "", ""),
    ("layer_at", Mode::Method, faces_layer_at, &["self", "i"], "", ""),
];

// ---- geometry, side by side -------------------------------------------------

fn geometry<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (mesh, plan) = (a.get_item(0)?, a.get_item(1)?);
    let own = mesh.getattr("__dict__")?;
    let hit = own.call_method1("get", ("_deferred_geom",))?;
    if !hit.is_none() {
        return Ok(hit);
    }
    let numpy = np(py)?;
    let native = bake(py)?.getattr("_native")?;
    let recent = py.import("sys")?.getattr("version_info")?.ge((3, 12))?;
    let made = native.getattr("native")?.call_method1(
        "face_geometry",
        (
            numpy.call_method1("asarray", (plan.getattr("pos")?, numpy.getattr("float64")?))?,
            plan.getattr("corners")?,
            plan.getattr("ring_sizes")?,
            plan.getattr("ring_counts")?,
            native.call_method0("qt_hypot")?,
            recent,
        ),
    )?;
    own.set_item("_deferred_geom", &made)?;
    Ok(made)
}

/// The meshes of `meshes` this engine bakes that have no geometry yet, each
/// once.
fn without_geometry<'py>(py: Python<'py>, meshes: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let module = bake(py)?;
    let seen = PySet::empty(py)?;
    let todo = PyList::empty(py);
    for mesh in meshes.try_iter()? {
        let mesh = mesh?;
        if seen.contains(id(&mesh))? {
            continue;
        }
        seen.add(id(&mesh))?;
        if module.call_method1("can_bake", (&mesh,))?.is_truthy()? && !mesh.getattr("__dict__")?.contains("_deferred_geom")? {
            todo.append(mesh)?;
        }
    }
    Ok(todo)
}

/// Plan and face geometry of every lazy mesh in `meshes`, side by side: the
/// weld and the kernel let go of the GIL, so the meshes of a big model are
/// worked out in parallel before the bake reads them.
fn prefetch<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let todo = without_geometry(py, &a.get_item(0)?)?;
    if todo.len() >= 2 {
        let work = worker(py, |mesh| bake(mesh.py())?.call_method1("_geometry", (mesh, plan_of(mesh)?)).map(drop))?;
        side_by_side(py, work, todo)?;
    }
    Ok(none(py))
}

/// The bakes `jobs` (`(mesh, paint, msig)`) ask for, side by side. They
/// wait in `vp._prebaked` for the bake that asks for them, which numbers
/// them and takes them away.
fn prebake<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (vp, jobs) = (a.get_item(0)?, a.get_item(1)?);
    let module = bake(py)?;
    let least = module.getattr("_PREBAKE_MIN_FACES")?;
    let todo = PyDict::new(py);
    for job in jobs.try_iter()? {
        let [mesh, paint, msig] = unpack::<3>(&job?)?;
        if module.call_method1("can_bake", (&mesh,))?.is_truthy()? && plan_of(&mesh)?.getattr("nfaces")?.ge(&least)? {
            todo.set_item((id(&mesh), &msig), (&mesh, paint, &msig))?;
        }
    }
    let done = PyDict::new(py);
    if todo.len() >= 2 {
        let (held_vp, held_done) = (vp.clone().unbind(), done.clone().unbind());
        let work = worker(py, move |item| {
            let py = item.py();
            let [key, job] = unpack::<2>(item)?;
            let [mesh, paint, msig] = unpack::<3>(&job)?;
            held_done.bind(py).set_item(key, bake(py)?.call_method1("_bake", (held_vp.bind(py), mesh, paint, msig))?)
        })?;
        side_by_side(py, work, todo.items())?;
    }
    vp.setattr("_prebaked", done)?;
    Ok(none(py))
}

/// Each face's own attrs dict (a fresh one per face, as Face keeps) is one
/// of a few: the document's table, parsed once.
fn attrs_table<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let parse = py.import("formats.igz")?.getattr("_face_attrs_from_json")?;
    let out = PyList::empty(py);
    let table = a.get_item(0)?.get_item("meta")?.call_method1("get", ("face_attrs",))?;
    if table.is_truthy()? {
        for entry in table.try_iter()? {
            let parsed = parse.call1((entry?,))?;
            out.append(if parsed.is_truthy()? { parsed } else { PyDict::new(py).into_any() })?;
        }
    }
    Ok(out.into_any())
}

// ---- fingerprints -----------------------------------------------------------

/// `Viewport._mesh_fingerprint` of the built mesh.
fn fingerprint<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (plan, table, face_attr) = (a.get_item(1)?, a.get_item(2)?, a.get_item(3)?);
    let numpy = np(py)?;
    let builtins = py.import("builtins")?;
    let positions = plan.getattr("pos")?;
    let pos: PyReadonlyArray2<'_, f64> = numpy.call_method1("ascontiguousarray", (&positions, numpy.getattr("float64")?))?.extract()?;
    // float(np.cumsum(x + y * 1.000003 + z * 1.000007)[-1]): added in order.
    let sum = pos.as_array().outer_iter().fold(0f64, |sum, p| sum + ((p[0] + p[1] * 1.000003) + p[2] * 1.000007));
    let mut paint = 0.into_bound_py_any(py)?;
    let tuple = |value: Obj<'py>| py.get_type::<PyTuple>().call1((value,));
    for (face, row) in face_attr.call_method0("tolist")?.try_iter()?.enumerate() {
        let row = row?;
        if row.lt(0)? {
            continue;
        }
        let attrs = table.get_item(row)?;
        if !attrs.is_truthy()? {
            continue;
        }
        let get = |name: &str| attrs.call_method1("get", (name,));
        let colour = get("color")?;
        let texture = get("texture")?;
        let back = get("back")?;
        let mapped = if texture.is_truthy()? {
            let uvw = texture.call_method1("get", ("uvw",))?;
            (
                texture.call_method1("get", ("path",))?,
                texture.call_method1("get", ("sw",))?,
                texture.call_method1("get", ("sh",))?,
                texture.call_method1("get", ("rot", 0))?,
                if uvw.is_truthy()? { tuple(uvw)? } else { PyTuple::empty(py).into_any() },
            )
                .into_bound_py_any(py)?
        } else {
            none(py)
        };
        let described = (
            face,
            if colour.is_none() { colour } else { tuple(colour)? },
            mapped,
            get("layer")?,
            get("opacity")?,
            if back.is_truthy()? { back.repr()?.into_any() } else { none(py) },
            get("hidden")?.is_truthy()?,
        );
        paint = paint.bitxor(builtins.call_method1("hash", (described,))?)?;
    }
    let mixed = |flags: Obj<'py>| -> PyResult<u64> {
        let mut mix = 0u64;
        for index in numpy.call_method1("flatnonzero", (flags,))?.call_method0("tolist")?.try_iter()? {
            mix ^= (index?.extract::<u64>()? + 1).wrapping_mul(2654435761) & 0xFFFF_FFFF;
        }
        Ok(mix)
    };
    let (soft_edges, flat_edges) = (plan.getattr("soft")?, plan.getattr("flat")?);
    let count = soft_edges.call_method0("sum")?.call_method0("__int__")?;
    let soft = if !flat_edges.is_none() && flat_edges.call_method0("any")?.is_truthy()? {
        // Soft only («Smooth normals» off) bakes otherwise; the same term
        // as Viewport._mesh_fingerprint's, so the two keys agree.
        (count, mixed(flat_edges.bitand(&soft_edges)?)?).into_bound_py_any(py)?
    } else {
        count
    };
    let hidden = mixed(plan.getattr("hidden")?)?;
    (positions.len()?, plan.getattr("edges")?.len()?, plan.getattr("nfaces")?, builtins.call_method1("round", (sum, 4))?, paint, soft, hidden).into_bound_py_any(py)
}

fn lazy_init<'py>(a: &Args<'py>) -> Reply<'py> {
    let this = a.get_item(0)?;
    this.setattr("_args", a.get_item(1)?)?;
    this.setattr("_suffix", a.get_item(2)?)?;
    this.setattr("_value", a.py().None())?;
    Ok(none(a.py()))
}
fn lazy_value_of<'py>(this: &Obj<'py>) -> Reply<'py> {
    let py = this.py();
    let known = this.getattr("_value")?;
    if !known.is_none() {
        return Ok(known);
    }
    let args = this.getattr("_args")?;
    let args = if args.is_callable() { args.call0()? } else { args };
    let value = bake(py)?.getattr("fingerprint")?.call1(positional(&args)?)?.add(this.getattr("_suffix")?)?;
    this.setattr("_value", &value)?;
    this.setattr("_args", py.None())?;
    Ok(value)
}
fn lazy_value<'py>(a: &Args<'py>) -> Reply<'py> {
    lazy_value_of(&a.get_item(0)?)
}
fn lazy_add<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let this = a.get_item(0)?;
    let other = py.get_type::<PyTuple>().call1((a.get_item(1)?,))?;
    let known = this.getattr("_value")?;
    if !known.is_none() {
        return known.add(other);
    }
    bake(py)?.getattr("LazyFingerprint")?.call1((this.getattr("_args")?, this.getattr("_suffix")?.add(other)?))
}
fn lazy_same<'py>(a: &Args<'py>) -> PyResult<bool> {
    let other = a.get_item(1)?;
    let other = if other.is_instance(&bake(a.py())?.getattr("LazyFingerprint")?)? { lazy_value_of(&other)? } else { other };
    lazy_value_of(&a.get_item(0)?)?.eq(other)
}
fn lazy_eq<'py>(a: &Args<'py>) -> Reply<'py> {
    lazy_same(a)?.into_bound_py_any(a.py())
}
fn lazy_ne<'py>(a: &Args<'py>) -> Reply<'py> {
    (!lazy_same(a)?).into_bound_py_any(a.py())
}
fn lazy_hash<'py>(a: &Args<'py>) -> Reply<'py> {
    lazy_value_of(&a.get_item(0)?)?.hash()?.into_bound_py_any(a.py())
}
fn lazy_getitem<'py>(a: &Args<'py>) -> Reply<'py> {
    lazy_value_of(&a.get_item(0)?)?.get_item(a.get_item(1)?)
}
fn lazy_len<'py>(a: &Args<'py>) -> Reply<'py> {
    lazy_value_of(&a.get_item(0)?)?.len()?.into_bound_py_any(a.py())
}
fn lazy_iter<'py>(a: &Args<'py>) -> Reply<'py> {
    Ok(lazy_value_of(&a.get_item(0)?)?.try_iter()?.into_any())
}
fn lazy_repr<'py>(a: &Args<'py>) -> Reply<'py> {
    format!("LazyFingerprint({})", lazy_value_of(&a.get_item(0)?)?.repr()?).into_bound_py_any(a.py())
}

#[rustfmt::skip]
const LAZY_FINGERPRINT: &[Method] = &[
    ("__init__", Mode::Method, lazy_init, &["self", "args", "suffix=()"], "", ""),
    ("value", Mode::Method, lazy_value, &["self"], "", ""),
    ("__add__", Mode::Method, lazy_add, &["self", "other"], "", ""),
    ("__eq__", Mode::Method, lazy_eq, &["self", "other"], "", ""),
    ("__ne__", Mode::Method, lazy_ne, &["self", "other"], "", ""),
    ("__hash__", Mode::Method, lazy_hash, &["self"], "", ""),
    ("__getitem__", Mode::Method, lazy_getitem, &["self", "i"], "", ""),
    ("__len__", Mode::Method, lazy_len, &["self"], "", ""),
    ("__iter__", Mode::Method, lazy_iter, &["self"], "", ""),
    ("__repr__", Mode::Method, lazy_repr, &["self"], "", ""),
];

// ---- the chunk entry --------------------------------------------------------

/// The chunk entry of deferred `mesh` inside a container of paint `paint`.
///
/// The first one asked of a document works out the plans and face geometry
/// of all its lazy meshes side by side: the picks ask for every chunk at
/// the first click, one after the other.
fn bake_entry<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (vp, mesh) = (a.get_item(0)?, a.get_item(1)?);
    let module = bake(py)?;
    if !mesh.getattr("__dict__")?.contains("_deferred_geom")? {
        let placements = attr(&vp, "_placements")?;
        if placements.is_callable() {
            let meshes = PyList::empty(py);
            for group in placements.call0()?.try_iter()? {
                meshes.append(group?.getattr("mesh")?)?;
            }
            module.call_method1("prefetch", (meshes,))?;
        }
    }
    direct(py, bake_uncached, (vp, mesh, a.get_item(2)?, a.get_item(3)?, a.get_item(4)?, a.get_item(5)?))
}

/// `Viewport._group_chunk`'s rebuild, from arrays.
pub(super) fn bake_uncached<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (vp, mesh, paint, msig) = (a.get_item(0)?, a.get_item(1)?, a.get_item(2)?, a.get_item(3)?);
    let module = bake(py)?;
    let key = (id(&mesh), &msig).into_bound_py_any(py)?;
    let ready = attr(&vp, "_prebaked")?;
    let mut fresh = if ready.is_truthy()? { ready.call_method1("pop", (&key, py.None()))? } else { none(py) };
    let live = module.getattr("_live_bakes")?;
    if fresh.is_none() {
        // Another view of this document holds this very bake: its arrays do
        // (four views of a big model were four copies of them, past 16 GB).
        let other = live.call_method1("get", (&key,))?;
        fresh = if !other.is_none() && other.get_item("mesh")?.is(&mesh) {
            other.getattr("as_baked")?
        } else {
            module.call_method1("_bake", (&vp, &mesh, paint, &msig))?
        };
    }
    let entry = module.getattr("BakedEntry")?.call1((&fresh,))?;
    entry.setattr("as_baked", fresh)?;
    entry.set_item("vkey", a.get_item(4)?)?;
    entry.set_item("uid", a.get_item(5)?)?;
    live.set_item(key, &entry)?;
    Ok(entry)
}

// ---- LazyPairs --------------------------------------------------------------

fn pairs_init<'py>(a: &Args<'py>) -> Reply<'py> {
    let this = a.get_item(0)?;
    this.setattr("faces", a.get_item(1)?)?;
    this.setattr("owner", a.get_item(2)?)?;
    Ok(none(a.py()))
}
fn pairs_len<'py>(a: &Args<'py>) -> Reply<'py> {
    a.get_item(0)?.getattr("faces")?.len()?.into_bound_py_any(a.py())
}
fn pairs_getitem<'py>(a: &Args<'py>) -> Reply<'py> {
    let this = a.get_item(0)?;
    (this.getattr("faces")?.get_item(a.get_item(1)?)?, this.getattr("owner")?).into_bound_py_any(a.py())
}
fn pairs_owner_at<'py>(a: &Args<'py>) -> Reply<'py> {
    a.get_item(0)?.getattr("owner")
}
fn pairs_iter<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let this = a.get_item(0)?;
    let owners = py.import("itertools")?.call_method1("repeat", (this.getattr("owner")?,))?;
    py.import("builtins")?.call_method1("zip", (this.getattr("faces")?, owners))
}
/// `(the face's own layer or None, owner)`.
fn layer_and_owner<'py>(faces: &Obj<'py>, at: Obj<'py>, owner: Obj<'py>) -> Reply<'py> {
    let py = faces.py();
    if faces.hasattr("layer_at")? {
        return (faces.call_method1("layer_at", (at,))?, owner).into_bound_py_any(py);
    }
    let attrs = attr(&faces.get_item(at)?, "attrs")?;
    let layer = if attrs.is_truthy()? { attrs.call_method1("get", ("layer",))? } else { none(py) };
    (layer, owner).into_bound_py_any(py)
}
fn pairs_layer_at<'py>(a: &Args<'py>) -> Reply<'py> {
    let this = a.get_item(0)?;
    layer_and_owner(&this.getattr("faces")?, a.get_item(1)?, this.getattr("owner")?)
}

#[rustfmt::skip]
const LAZY_PAIRS: &[Method] = &[
    ("__init__", Mode::Method, pairs_init, &["self", "faces", "owner"], "", ""),
    ("__len__", Mode::Method, pairs_len, &["self"], "", ""),
    ("__getitem__", Mode::Method, pairs_getitem, &["self", "i"], "", ""),
    ("owner_at", Mode::Method, pairs_owner_at, &["self", "_i"], "", ""),
    ("__iter__", Mode::Method, pairs_iter, &["self"], "", ""),
    ("layer_at", Mode::Method, pairs_layer_at, &["self", "i"], "", ""),
];

// ---- PickEntities -----------------------------------------------------------

fn entities_init<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let this = a.get_item(0)?;
    this.setattr("_parts", PyList::empty(py))?;
    this.setattr("_starts", PyList::empty(py))?;
    this.setattr("_len", 0)?;
    this.call_method1("extend", (a.get_item(1)?,))?;
    Ok(none(py))
}
fn entities_extend<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (this, sequence) = (a.get_item(0)?, a.get_item(1)?);
    if sequence.is_instance(&bake(py)?.getattr("PickEntities")?)? {
        for part in sequence.getattr("_parts")?.try_iter()? {
            this.call_method1("extend", (part?,))?;
        }
        return Ok(none(py));
    }
    let count = sequence.len()?;
    if count != 0 {
        let total: usize = this.getattr("_len")?.extract()?;
        this.getattr("_parts")?.call_method1("append", (sequence,))?;
        this.getattr("_starts")?.call_method1("append", (total,))?;
        this.setattr("_len", total + count)?;
    }
    Ok(none(py))
}
fn entities_len<'py>(a: &Args<'py>) -> Reply<'py> {
    a.get_item(0)?.getattr("_len")
}
fn entities_bool<'py>(a: &Args<'py>) -> Reply<'py> {
    a.get_item(0)?.getattr("_len")?.gt(0)?.into_bound_py_any(a.py())
}
/// The part entity `at` is in and its place there. A negative `at` counts
/// from the end when `wrap`.
fn entity_place<'py>(this: &Obj<'py>, at: Obj<'py>, wrap: bool, checked: bool) -> PyResult<(Obj<'py>, isize)> {
    let py = this.py();
    let mut at = int(at)?;
    let total: isize = this.getattr("_len")?.extract()?;
    if wrap && at < 0 {
        at += total;
    }
    if checked && !(0 <= at && at < total) {
        return Err(PyIndexError::new_err(at));
    }
    let starts = this.getattr("_starts")?;
    let part: isize = py.import("bisect")?.call_method1("bisect_right", (&starts, at))?.extract::<isize>()? - 1;
    Ok((this.getattr("_parts")?.get_item(part)?, at - starts.get_item(part)?.extract::<isize>()?))
}
fn entities_getitem<'py>(a: &Args<'py>) -> Reply<'py> {
    let (part, at) = entity_place(&a.get_item(0)?, a.get_item(1)?, true, true)?;
    part.get_item(at)
}
/// The owner half of entity `i`, with no face made for it.
fn entities_owner_at<'py>(a: &Args<'py>) -> Reply<'py> {
    let (part, at) = entity_place(&a.get_item(0)?, a.get_item(1)?, true, false)?;
    if part.is_instance(&bake(a.py())?.getattr("LazyPairs")?)? {
        part.call_method1("owner_at", (at,))
    } else {
        part.get_item(at)?.get_item(1)
    }
}
fn entities_iter<'py>(a: &Args<'py>) -> Reply<'py> {
    a.py().import("itertools")?.getattr("chain")?.call_method1("from_iterable", (a.get_item(0)?.getattr("_parts")?,))
}
/// `(the face's own layer or None, owner)` of entity `i`, without building
/// a lazy mesh for it.
fn entities_layer_at<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let (part, at) = entity_place(&a.get_item(0)?, a.get_item(1)?, false, false)?;
    if part.hasattr("layer_at")? {
        return part.call_method1("layer_at", (at,));
    }
    let [face, owner] = unpack::<2>(&part.get_item(at)?)?;
    let attrs = attr(&face, "attrs")?;
    let layer = if attrs.is_truthy()? { attrs.call_method1("get", ("layer",))? } else { none(py) };
    (layer, owner).into_bound_py_any(py)
}

#[rustfmt::skip]
const PICK_ENTITIES: &[Method] = &[
    ("__init__", Mode::Method, entities_init, &["self", "first=()"], "", ""),
    ("extend", Mode::Method, entities_extend, &["self", "seq"], "", ""),
    ("__len__", Mode::Method, entities_len, &["self"], "", ""),
    ("__bool__", Mode::Method, entities_bool, &["self"], "", ""),
    ("__getitem__", Mode::Method, entities_getitem, &["self", "i"], "", ""),
    ("owner_at", Mode::Method, entities_owner_at, &["self", "i"], "", ""),
    ("__iter__", Mode::Method, entities_iter, &["self"], "", ""),
    ("layer_at", Mode::Method, entities_layer_at, &["self", "i"], "", ""),
];

// ---- the module -------------------------------------------------------------

#[rustfmt::skip]
const FUNCTIONS: &[Method] = &[
    ("can_bake", Mode::Function, can_bake, &["mesh"], "", ""),
    ("reading_now", Mode::Function, reading_now, &[], "", ""),
    ("light_face", Mode::Function, light_face, &["mesh", "f"], "", ""),
    ("_geometry", Mode::Function, geometry, &["mesh", "plan"], "", ""),
    ("prefetch", Mode::Function, prefetch, &["meshes"], "", ""),
    ("prebake", Mode::Function, prebake, &["vp", "jobs"], "", ""),
    ("_attrs_table", Mode::Function, attrs_table, &["src"], "", ""),
    ("fingerprint", Mode::Function, fingerprint, &["mesh", "plan", "table", "face_attr"], "", ""),
    ("bake", Mode::Function, bake_entry, &["vp", "mesh", "paint", "msig", "vkey", "uid"], "", ""),
];

fn slotted<'py>(module: &Bound<'py, PyModule>, name: &str, bases: &[Obj<'py>], slots: &[&str], doc: &str, methods: &[Method]) -> Reply<'py> {
    let py = module.py();
    let attrs = fields!(py; "__slots__" => PyTuple::new(py, slots)?, "__doc__" => doc);
    let cls = modules::class(module, name, bases, &attrs)?;
    install(&cls, &module.dict(), methods)?;
    Ok(cls)
}

pub(super) fn install_module(module: &Bound<PyModule>) -> PyResult<()> {
    let py = module.py();
    module.add(
        "__doc__",
        "The drawing of a lazy mesh, baked from its arrays: the very chunk the object\nbake makes from the built mesh, field for field, with no vertex, edge or face\nobject made, so a model that is only looked at stays deferred\n(core.mesh.DeferredMesh).\n\nThe face geometry (Newell normal, area, centroid, triangles) comes from the\nkernel; the topology from core.lazy_mesh.plan. tests/test_array_bake.py holds\nthe result to the object bake.",
    )?;
    module.add("np", py.import("numpy")?)?;
    modules::import_names(module, "ingetrazo_core", &[("Vec3", "QVector3D"), ("bake_deferred_chunk", "_bake")])?;
    module.add("_native", py.import("core._native")?)?;
    module.add("lazy_mesh", py.import("core.lazy_mesh")?)?;
    modules::import_names(module, "ingetrazo_core.mesh", &[("DeferredMesh", "DeferredMesh"), ("Face", "Face"), ("Vertex", "Vertex")])?;
    // While True, a face asked of a still-lazy mesh comes as a LightFace and
    // the mesh stays unbuilt: only the hover's read-only passes turn it on.
    module.add("_reading", false)?;
    // Below this a mesh's bake is small: hundreds of them in threads only
    // queue for the interpreter.
    module.add("_PREBAKE_MIN_FACES", 1000)?;
    // (mesh id, paint signature) → a live entry of that bake, in any view.
    module.add("_live_bakes", py.import("weakref")?.getattr("WeakValueDictionary")?.call0()?)?;
    install(module.dict().as_any(), &module.dict(), FUNCTIONS)?;

    let reading = modules::class(
        module,
        "reading_only",
        &[],
        &fields!(py; "__doc__" => "``with reading_only():`` the code inside only READS the faces it picks\n— their positions, their boundary's flags — and keeps none. A big lazy\nmesh is not built for that: 9 s and 2 GB on the hover that crossed into\nthe largest group of a 5 M-face model, in the middle of a Line (#1092)."),
    )?;
    install(&reading, &module.dict(), &[("__enter__", Mode::Method, reading_enter, &["self"], "", "")])?;
    modules::method(&reading, "__exit__", wrap_pyfunction!(reading_exit, module)?.as_any())?;

    slotted(
        module,
        "LightFace",
        &[module.getattr("Face")?],
        &["mesh", "index"],
        "A face of a still-lazy mesh, read from its plan: the positions, holes,\nattributes and boundary edges (with their flags) the built face would\nhave, on vertices and edges of its own. For reading only; ``real()`` is\nthe mesh's own face, built with the mesh.",
        &[("real", Mode::Method, light_real, &["self"], "", "")],
    )?;
    slotted(
        module,
        "DeferredFaces",
        &[],
        &["mesh", "index"],
        "The ``faces`` of a chunk baked from arrays: the drawn faces, built\n(with their whole mesh) only when one is asked for — a click on it.",
        DEFERRED_FACES,
    )?;
    slotted(
        module,
        "LazyFingerprint",
        &[],
        &["_args", "_suffix", "_value"],
        "A chunk's ``fp`` that is only worked out when compared — a per-face\nwalk, and a lazy mesh's chunk is only checked against its fingerprint\nonce the mesh is built. Compares, slices and extends as the tuple.",
        LAZY_FINGERPRINT,
    )?;
    modules::class(
        module,
        "BakedEntry",
        &[py.get_type::<PyDict>().into_any()],
        &fields!(py; "__doc__" => "A chunk entry of a lazy mesh: one view's own dict, which that view goes\non to write its own keys in, and ``as_baked``, the fields as they left the\nbake, what another view's entry is made from."),
    )?;
    slotted(
        module,
        "LazyPairs",
        &[],
        &["faces", "owner"],
        "``[(face, owner) for face in faces]`` of a :class:`DeferredFaces`,\nmade one by one when asked for.",
        LAZY_PAIRS,
    )?;
    slotted(
        module,
        "PickEntities",
        &[],
        &["_parts", "_starts", "_len"],
        "The pick index's ``entities`` — one list of ``(face, owner)`` — kept\nas the pieces it is made of, so a lazy mesh's faces are only built\nwhen a pick lands on one. Only used once a lazy piece is in.",
        PICK_ENTITIES,
    )?;
    Ok(())
}
