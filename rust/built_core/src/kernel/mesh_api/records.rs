// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
#[pyfunction]
fn mutable_init(this: &O<'_>, owner: &O<'_>, path: &O<'_>, items: &O<'_>) -> PyResult<()> {
    this.py()
        .get_type::<PyList>()
        .call_method1("__init__", (this, items))?;
    this.setattr("_owner", owner)?;
    this.setattr("_path", path)
}
#[pyfunction(signature=(name,this,*args,**kwargs))]
fn mutate<'p>(
    name: &str,
    this: &O<'p>,
    args: &Bound<'p, PyTuple>,
    kwargs: Option<&D<'p>>,
) -> PyResult<O<'p>> {
    let kw = PyDict::new(this.py());
    let result = kernel(this.py())?.call_method1(
        "mutate_incidence",
        (
            this,
            if name == "__iadd__" { "extend" } else { name },
            args,
            kwargs.unwrap_or(&kw),
        ),
    )?;
    Ok(if matches!(name, "__iadd__" | "__imul__") {
        this.clone()
    } else {
        result
    })
}
#[pyfunction(signature=(_this,*_args))]
fn readonly(_this: &O<'_>, _args: &Bound<'_, PyTuple>) -> PyResult<()> {
    Err(PyValueError::new_err(
        "vertex.edges is determined by the endpoints of live edges",
    ))
}
#[pyfunction]
fn snapshot_init(
    this: &O<'_>,
    snap: &O<'_>,
    own: &O<'_>,
    table: &O<'_>,
    mesh: &O<'_>,
) -> PyResult<()> {
    this.setattr("snap", snap)?;
    this.setattr("owner", mesh.getattr("_store")?)?;
    this.setattr("own", own)?;
    this.setattr("table", table)?;
    this.setattr("mesh", mesh)?;
    this.setattr("_cache", PyDict::new(this.py()))
}
#[pyfunction]
fn positions_init(this: &O<'_>, snap: &O<'_>) -> PyResult<()> {
    this.setattr("snap", snap)
}
#[pyfunction]
fn map_init(this: &O<'_>, m: &O<'_>, vobjs: &O<'_>) -> PyResult<()> {
    this.setattr("m", m)?;
    this.setattr("vobjs", vobjs)?;
    this.setattr("_full", this.py().None())
}
#[pyfunction]
pub(super) fn _attribute_owner<'p>(mesh: &O<'p>) -> PyResult<O<'p>> {
    mesh.getattr("__dict__")?
        .call_method1("get", ("_instance_definition", mesh))?
        .getattr("__dict__")
}
#[pyfunction]
fn _store_of<'p>(m: &O<'p>) -> PyResult<O<'p>> {
    m.getattr("__dict__")?.call_method1("get", ("_store",))
}
#[pyfunction]
fn _weld_args(py: Python<'_>) -> PyResult<D<'_>> {
    let cfg = api(py)?;
    let out = PyDict::new(py);
    out.set_item("scale", cfg.getattr("_KEY_SCALE")?)?;
    let d = cfg.getattr("_KEY_DECIMALS")?.extract::<i32>()?;
    out.set_item("tol2", 10f64.powi(-d).powi(2))?;
    out.set_item("fused", false)?;
    Ok(out)
}
#[pyfunction]
pub(super) fn _xyz<'p>(p: &O<'p>) -> PyResult<O<'p>> {
    if let Some(f) = p.getattr_opt("toTuple")? {
        return f.call0();
    }
    let coords = builtin(p.py(), "tuple")?.call1((p,))?;
    if coords.len()? != 3 {
        return Err(PyValueError::new_err("expected three coordinates"));
    }
    let float = builtin(p.py(), "float")?;
    let values = (0..3)
        .map(|i| float.call1((coords.get_item(i)?,)))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(PyTuple::new(p.py(), values)?.into_any())
}
#[pyfunction]
pub(super) fn _hypot(py: Python<'_>) -> PyResult<O<'_>> {
    module(py, "core._native")?.call_method0("qt_hypot")
}
#[pyfunction]
fn _hole_filter<'p>(holes: &O<'p>) -> PyResult<O<'p>> {
    let py = holes.py();
    let rings = PyList::empty(py);
    for h in holes.try_iter()? {
        let r = PyList::empty(py);
        for p in h?.try_iter()? {
            r.append(_xyz(&p?)?)?;
        }
        rings.append(r)?;
    }
    let mut hypot = _hypot(py)?;
    if !hypot.is_truthy()? {
        hypot = pyo3::types::PyString::new(py, "libstdc++").into_any();
    }
    module(py, "built_core")?
        .getattr("MeshStore")?
        .call_method1("maximal_holes", (rings, hypot))
}
#[pyfunction]
fn enter<'p>(this: &O<'p>) -> PyResult<O<'p>> {
    strict_count(this.py(), 1)?;
    Ok(this.clone())
}
fn strict_count(py: Python<'_>, delta: i64) -> PyResult<()> {
    let c = store_api(py)?.getattr("_STRICT")?;
    c.set_item(0, c.get_item(0)?.add(delta)?)
}
#[pyfunction(signature=(this,*_exc))]
fn exit(this: &O<'_>, _exc: &Bound<'_, PyTuple>) -> PyResult<bool> {
    strict_count(this.py(), -1)?;
    Ok(false)
}
#[pyfunction(signature=(path,step,err,**extra))]
fn _log_refusal(path: &O<'_>, step: &str, err: &O<'_>, extra: Option<&D<'_>>) -> PyResult<()> {
    let py = path.py();
    let os = module(py, "os")?;
    let d = PyDict::new(py);
    let mut ops = Vec::new();
    let mut frame = module(py, "sys")?.call_method0("_getframe")?;
    while !frame.is_none() {
        let code = frame.getattr("f_code")?;
        let file = code.getattr("co_filename")?.extract::<String>()?;
        if let Some(n) = std::path::Path::new(&file)
            .file_stem()
            .and_then(|n| n.to_str())
        {
            if matches!(
                n,
                "push_pull" | "pushpull" | "cap_rebuild" | "history" | "topology" | "commands"
            ) && ops.len() < 6
            {
                ops.push(format!("{n}.{}", code.getattr("co_name")?));
            }
        }
        frame = frame.getattr("f_back")?;
    }
    d.set_item("ops", ops)?;
    d.set_item("step", step)?;
    let reason = if err.is_none() {
        String::new()
    } else {
        err.str()?.to_string().chars().take(200).collect()
    };
    d.set_item("reason", reason)?;
    let test = os
        .getattr("environ")?
        .call_method1("get", ("PYTEST_CURRENT_TEST", ""))?
        .extract::<String>()?;
    d.set_item("test", test.split(' ').next().unwrap_or(""))?;
    if let Some(v) = extra {
        d.update(v.as_mapping())?;
    }
    let line = module(py, "json")?.call_method1("dumps", (d,))?.add("\n")?;
    let file = builtin(py, "open")?.call1((path, "a"))?;
    let result = file.call_method1("write", (line,));
    file.call_method0("close")?;
    result.map(drop)
}
#[pyfunction]
fn _note_refusal(step: &str, err: &O<'_>) -> PyResult<()> {
    let py = err.py();
    let path = module(py, "os")?
        .getattr("environ")?
        .call_method1("get", ("BUILT_REFUSALS",))?;
    if path.is_truthy()? {
        _log_refusal(&path, step, err, None)?;
    }
    let s = store_api(py)?;
    if s.getattr("_STRICT")?.get_item(0)?.is_truthy()? {
        return Err(PyErr::from_value(s.call_method1(
            "StoreRefusal",
            (if err.is_none() {
                step.to_owned()
            } else {
                format!("{step}: {}", err.str()?)
            },),
        )?));
    }
    Ok(())
}
#[pyfunction(signature=(mesh,matrix,origin=[0.;3],for_edit=false))]
fn instance_view<'p>(
    mesh: &O<'p>,
    matrix: &O<'p>,
    origin: [f64; 3],
    for_edit: bool,
) -> PyResult<O<'p>> {
    kernel(mesh.py())?.call_method1("instance_view", (mesh, matrix, origin, for_edit))
}
#[pyfunction(signature=(version,archive,cls=None))]
fn _unpickle_mesh<'p>(version: &O<'p>, archive: &O<'p>, cls: Option<&O<'p>>) -> PyResult<O<'p>> {
    kernel(version.py())?.call_method1(
        "unpickle_mesh",
        (
            version,
            archive,
            match cls {
                Some(c) => c.clone(),
                None => store_api(version.py())?.getattr("StoreMesh")?,
            },
        ),
    )
}
#[pyfunction]
fn plan_cap_plane<'p>(mesh: &O<'p>, origin: &O<'p>, normal: &O<'p>) -> PyResult<O<'p>> {
    let py = mesh.py();
    let plan = mesh.getattr("_store")?.call_method1(
        "single_plane",
        (
            origin.call_method0("toTuple")?,
            normal.call_method0("toTuple")?,
            _hypot(py)?,
            py.version_info() >= (3, 12),
        ),
    )?;
    if plan.is_none() {
        return Ok(plan);
    }
    let vec = module(py, "built_core")?.getattr("Vec3")?;
    let out = PyList::empty(py);
    for row in plan.try_iter()? {
        let row = row?;
        let rings = PyList::empty(py);
        for ring in row.get_item(0)?.try_iter()? {
            let r = PyList::empty(py);
            for p in ring?.try_iter()? {
                r.append(vec.call(builtin(py, "tuple")?.call1((p?,))?.cast::<PyTuple>()?, None)?)?;
            }
            rings.append(r)?;
        }
        out.append((
            rings.get_item(0)?,
            rings.get_slice(1, rings.len()),
            row.get_item(1)?,
        ))?;
    }
    Ok(out.into_any())
}
pub(super) fn install(s: &M<'_>, _core: &M<'_>, k: &O<'_>) -> PyResult<()> {
    let py = s.py();
    let ns = PyDict::new(py);
    method!(ns, s, "__init__", mutable_init);
    for n in "append clear extend insert pop remove reverse sort __setitem__ __delitem__ __iadd__ __imul__".split_whitespace(){bind(&ns,n,&partial(wrap_pyfunction!(mutate,s)?.as_any(),n)?)?;}
    class(s, "_MutList", &[builtin(py, "list")?], &ns)?;
    let ns = PyDict::new(py);
    for n in "add discard remove pop clear update difference_update intersection_update symmetric_difference_update __ior__ __iand__ __ixor__ __isub__".split_whitespace(){bind(&ns,n,wrap_pyfunction!(readonly,s)?.as_any())?;}
    class(s, "_MutSet", &[builtin(py, "set")?], &ns)?;
    let mapping = py.import("collections.abc")?.getattr("Mapping")?;
    for (n, base, fields, init) in [
        (
            "StoreState",
            "MeshSnapshotRecord",
            "snap own table mesh _cache owner",
            wrap_pyfunction!(snapshot_init, s)?,
        ),
        (
            "_SnapPositions",
            "SnapshotPositions",
            "snap",
            wrap_pyfunction!(positions_init, s)?,
        ),
        (
            "_EdgeMap",
            "BulkEdgeMap",
            "m vobjs _full",
            wrap_pyfunction!(map_init, s)?,
        ),
    ] {
        let ns = PyDict::new(py);
        slots(&ns, fields)?;
        bind(&ns, "__init__", init.as_any())?;
        if n == "StoreState" {
            ns.set_item("_KEYS",PyTuple::new(py,"vertices edges faces registry vpos vedges efaces esoft eflat ehidden elayer ecurve floop fholes fattrs finterior".split_whitespace().collect::<Vec<_>>())?)?;
        }
        class(s, n, &[k.getattr(base)?, mapping.clone()], &ns)?;
    }
    s.add("LOCAL_MASK", (1u64 << 40) - 1)?;
    s.add("STORE_SIDE",PyTuple::new(py,"_store _attr_table _attr_own _bulk _pos_cache _face_cache _frame_parent _tool_frame_cache _planned_points _instance_definition".split_whitespace().collect::<Vec<_>>())?)?;
    s.add("_STRICT", vec![0])?;
    class(
        s,
        "StoreRefusal",
        &[builtin(py, "Exception")?],
        &PyDict::new(py),
    )?;
    let ns = PyDict::new(py);
    method!(ns, s, "__enter__", enter);
    method!(ns, s, "__exit__", exit);
    class(s, "strict", &[], &ns)?;
    s.add("faces_arrays", k.getattr("mesh_face_arrays")?)?;
    for f in [
        wrap_pyfunction!(_attribute_owner, s)?,
        wrap_pyfunction!(_store_of, s)?,
        wrap_pyfunction!(_weld_args, s)?,
        wrap_pyfunction!(_xyz, s)?,
        wrap_pyfunction!(_hypot, s)?,
        wrap_pyfunction!(_hole_filter, s)?,
        wrap_pyfunction!(_log_refusal, s)?,
        wrap_pyfunction!(_note_refusal, s)?,
        wrap_pyfunction!(instance_view, s)?,
        wrap_pyfunction!(_unpickle_mesh, s)?,
        wrap_pyfunction!(plan_cap_plane, s)?,
    ] {
        s.add_function(f)?;
    }
    Ok(())
}
