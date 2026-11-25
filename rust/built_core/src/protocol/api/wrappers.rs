use super::*;
#[pyfunction(signature=(scene,*,photo=true,history=None))]
fn layer_count_steps<'p>(
    scene: &Obj<'p>,
    photo: bool,
    history: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    scene
        .py()
        .import("built_core")?
        .call_method1("LayerCountSteps", (scene, photo, history))
}
#[pyfunction(signature=(scene,*,photo=true))]
fn layer_counts<'p>(scene: &Obj<'p>, photo: bool) -> PyResult<Obj<'p>> {
    for v in layer_count_steps(scene, photo, None)?.try_iter()? {
        let v = v?;
        if !v.is_none() {
            return Ok(v);
        }
    }
    Err(pyo3::exceptions::PyStopIteration::new_err(()))
}
#[pyfunction(signature=(this,loops,closed,face=None))]
fn profile(
    this: &Obj<'_>,
    loops: &Obj<'_>,
    closed: &Obj<'_>,
    face: Option<&Obj<'_>>,
) -> PyResult<()> {
    let py = this.py();
    let out = PyList::empty(py);
    for ring in loops.try_iter()? {
        out.append(py.get_type::<PyList>().call1((ring?,))?)?
    }
    this.setattr("loops", out)?;
    this.setattr("closed", closed)?;
    this.setattr("face", face)
}
#[pyfunction]
fn points<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    let out = list(this.py());
    for r in this.getattr("loops")?.try_iter()? {
        out.call_method1("extend", (r?,))?;
    }
    Ok(out)
}
#[pyfunction]
fn smooth<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    let soft = this.getattr("soft")?;
    let flat = this.getattr("flat")?;
    if flat.is_none() {
        Ok(soft)
    } else {
        soft.call_method1("__and__", (flat.call_method0("__invert__")?,))
    }
}
#[pyfunction]
fn nfaces(this: &Obj<'_>) -> PyResult<usize> {
    this.getattr("ring_counts")?.len()
}
#[pyfunction]
fn face_net_areas<'p>(plan: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = plan.py();
    let np = py.import("numpy")?;
    let ints = |k| np.call_method1("asarray", (plan.getattr(k)?, np.getattr("int64")?));
    py.import("built_core")?.call_method1(
        "planned_face_areas",
        (
            plan.getattr("pos")?,
            plan.getattr("corners")?,
            ints("ring_sizes")?,
            ints("ring_counts")?,
        ),
    )
}
#[pyfunction]
fn row_id<'p>(address: &Obj<'p>) -> PyResult<Obj<'p>> {
    if address.is_instance_of::<pyo3::types::PyString>() {
        return Ok(address.clone());
    }
    let py = address.py();
    let kw = PyDict::new(py);
    kw.set_item("separators", (",", ":"))?;
    Ok(format!(
        "@{}",
        py.import("json")?
            .call_method("dumps", (address,), Some(&kw))?
    )
    .into_pyobject(py)?
    .into_any())
}
#[pyfunction]
fn mount_key(stems: &Obj<'_>) -> PyResult<String> {
    Ok(format!(
        "link:{}",
        "/link:"
            .into_pyobject(stems.py())?
            .call_method1("join", (stems,))?
    ))
}
#[pyfunction]
fn is_linked_key(key: &Obj<'_>) -> bool {
    key.extract::<String>()
        .is_ok_and(|s| s == "linked" || s.starts_with("link:"))
}
#[pyfunction]
fn tree_init(this: &Obj<'_>, scene: &Obj<'_>, registry: &Obj<'_>) -> PyResult<()> {
    this.py()
        .import("built_core")?
        .call_method1("build_project_tree", (this, scene, registry))?;
    Ok(())
}
#[pyfunction]
fn address<'p>(this: &Obj<'p>, group: &Obj<'p>) -> PyResult<Obj<'p>> {
    let uid = group.getattr("uid")?;
    if this.getattr("_qualified")?.contains(&uid)?
        || uid.call_method1("startswith", ("@[",))?.is_truthy()?
    {
        this.getattr("scene")?
            .call_method1("occurrence_path", (group,))
    } else {
        Ok(uid)
    }
}
#[pyfunction]
fn tree_row<'p>(this: &Obj<'p>, group: &Obj<'p>) -> PyResult<Obj<'p>> {
    row_id(&address(this, group)?)
}
#[pyfunction]
fn resolve<'p>(this: &Obj<'p>, mut key: Obj<'p>) -> PyResult<Obj<'p>> {
    let a = this.getattr("alias")?;
    for _ in 0..64 {
        if !a.contains(&key)? {
            break;
        }
        key = a.get_item(key)?;
    }
    Ok(key)
}
#[pyfunction]
fn order<'p>(this: &Obj<'p>) -> PyResult<Obj<'p>> {
    this.py()
        .import("built_core")?
        .call_method1("tree_order", (this.getattr("children")?,))
}
#[pyfunction]
fn subtree<'p>(this: &Obj<'p>, key: &Obj<'p>) -> PyResult<Obj<'p>> {
    this.py()
        .import("built_core")?
        .call_method1("tree_subtree", (this.getattr("children")?, key))
}
#[pyfunction]
fn members<'p>(this: &Obj<'p>, key: &Obj<'p>) -> PyResult<Obj<'p>> {
    let out = list(this.py());
    let direct = this.getattr("direct")?;
    for k in subtree(this, key)?.try_iter()? {
        out.call_method1(
            "extend",
            (direct.call_method1("get", (k?, PyTuple::empty(this.py())))?,),
        )?;
    }
    Ok(out)
}
#[pyfunction]
fn file_tags<'p>(this: &Obj<'p>, key: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = this.py();
    let out = PyList::empty(py);
    let n = this.getattr("nodes")?.call_method1("get", (key,))?;
    if !n.is_none() && n.get_item("kind")?.eq("link")? {
        let sep: String = py.import("core.links")?.getattr("SEP")?.extract()?;
        let bases: Vec<String> = n.get_item("folders")?.extract()?;
        let pre = bases
            .iter()
            .map(|b| format!("{b}{sep}"))
            .collect::<Vec<_>>();
        for l in this.getattr("scene")?.getattr("layers")?.try_iter()? {
            let tag = l?.getattr("key")?;
            let s: String = tag.extract()?;
            if pre.iter().any(|p| s.starts_with(p)) {
                out.append(tag)?
            }
        }
    }
    Ok(out.into_any())
}
#[pyfunction(signature=(scene,path,nested=false))]
fn file_state<'p>(scene: &Obj<'p>, path: &Obj<'p>, nested: bool) -> PyResult<Obj<'p>> {
    let _ = nested;
    scene
        .py()
        .import("core.links")?
        .call_method1("link_state", (scene, path))
}
fn share_init(
    this: &Obj<'_>,
    group: &Obj<'_>,
    proto: &Obj<'_>,
    xform: &Obj<'_>,
    edited: &Obj<'_>,
    inner: Option<&Obj<'_>>,
) -> PyResult<()> {
    let py = this.py();
    for (k, v) in [
        ("group", group),
        ("proto", proto),
        ("xform", xform),
        ("edited", edited),
    ] {
        this.setattr(k, v)?
    }
    this.setattr(
        "inner",
        match inner {
            Some(s) => py.get_type::<PyList>().call1((s,))?,
            None => list(py),
        },
    )?;
    this.setattr("_before", py.None())?;
    this.setattr("_after", py.None())
}
#[pyfunction(signature=(this,group,proto,xform,edited,inner=None))]
fn instance(
    this: &Obj<'_>,
    group: &Obj<'_>,
    proto: &Obj<'_>,
    xform: &Obj<'_>,
    edited: &Obj<'_>,
    inner: Option<&Obj<'_>>,
) -> PyResult<()> {
    share_init(this, group, proto, xform, edited, inner)
}
#[pyfunction(signature=(this,group,proto,xform,edited,state0,parts,inner=None))]
fn assembly(
    this: &Obj<'_>,
    group: &Obj<'_>,
    proto: &Obj<'_>,
    xform: &Obj<'_>,
    edited: &Obj<'_>,
    state0: &Obj<'_>,
    parts: &Obj<'_>,
    inner: Option<&Obj<'_>>,
) -> PyResult<()> {
    let py = this.py();
    share_init(
        this,
        group,
        proto,
        &py.import("built_core")?
            .call_method1("Matrix4", (xform,))?,
        edited,
        inner,
    )?;
    this.setattr("parts", parts)?;
    this.setattr("state0", state0)?;
    for k in ["_framed", "_edited", "_left", "_entered"] {
        this.setattr(k, py.None())?
    }
    this.setattr("shared", false)
}
#[pyfunction]
fn session(
    this: &Obj<'_>,
    share: &Obj<'_>,
    parts: &Obj<'_>,
    inner: &Obj<'_>,
    shared: &Obj<'_>,
    entry: &Obj<'_>,
) -> PyResult<()> {
    for (i, k) in ["group", "proto", "xform"].iter().enumerate() {
        this.setattr(*k, share.get_item(i)?)?
    }
    this.setattr("edited", this.getattr("group")?.getattr("mesh")?)?;
    for (k, v) in [
        ("parts", parts),
        ("inner", inner),
        ("shared", shared),
        ("entry", entry),
    ] {
        this.setattr(k, v)?
    }
    this.setattr("reference_assembly", !parts.is_none())?;
    this.setattr("reference_instance", parts.is_none())?;
    this.setattr(
        "history_name",
        if parts.is_none() {
            "ReshareInstanceCommand"
        } else {
            "ReshareAssemblyCommand"
        },
    )?;
    this.setattr("replay", this.py().None())
}
pub(super) fn install(m: &Module<'_>, name: &str, core: &Module<'_>) -> PyResult<()> {
    let py = m.py();
    match name {
        "core.bridge" => {
            exports(
                m,
                core.as_any(),
                "align_chain align_loop bridge follow_neighbours profiles_from_selection strips",
            )?;
            m.add("plan", core.getattr("bridge_plan")?)?;
            class(
                m,
                "BridgeError",
                vec![py.get_type::<PyValueError>().into_any()],
                &PyDict::new(py),
            )?;
            let ns = PyDict::new(py);
            meth!(ns, m, "__init__", profile);
            prop(&ns, "points", wrap_pyfunction!(points, m)?.into_any())?;
            class(m, "Profile", vec![], &ns)?;
        }
        "core.lazy_mesh" => {
            exports(m, core.as_any(), "PlannedVertices entity_count plan_of")?;
            m.add("plan", core.getattr("deferred_plan")?)?;
            m.add("weld", core.getattr("deferred_weld")?)?;
            let dc = py.import("dataclasses")?;
            let fields = PyList::empty(py);
            for n in
                "pos edges soft hidden curve layer corners ring_sizes ring_counts ef_start ef_face"
                    .split_whitespace()
            {
                fields.append((
                    n,
                    if matches!(n, "curve" | "layer") {
                        "list"
                    } else {
                        "np.ndarray"
                    },
                ))?
            }
            let kw = PyDict::new(py);
            kw.set_item("default_factory", py.get_type::<PyDict>())?;
            fields.append(("extra", "dict", dc.call_method("field", (), Some(&kw))?))?;
            let kw = PyDict::new(py);
            kw.set_item("default", py.None())?;
            fields.append((
                "flat",
                "np.ndarray | None",
                dc.call_method("field", (), Some(&kw))?,
            ))?;
            let c = dc.call_method1("make_dataclass", ("Plan", fields))?;
            c.setattr("__module__", name)?;
            let property = py.import("builtins")?.getattr("property")?;
            c.setattr("smooth", property.call1((wrap_pyfunction!(smooth, m)?,))?)?;
            c.setattr("nfaces", property.call1((wrap_pyfunction!(nfaces, m)?,))?)?;
            m.add("np", py.import("numpy")?)?;
            m.add("Plan", c)?;
            m.add_function(wrap_pyfunction!(face_net_areas, m)?)?;
        }
        "core.purge" => {
            let mut names = Vec::new();
            for n in "iter_groups iter_meshes layer_carriers layers_held_by_scenes unused_layers unused_materials used_layers used_materials _annotations _count_edges _document_groups".split_whitespace(){m.add(n,core.getattr(format!("purge_{}",n.trim_start_matches('_')))?)?;names.push(n)}
            m.add_function(wrap_pyfunction!(layer_count_steps, m)?)?;
            m.add_function(wrap_pyfunction!(layer_counts, m)?)?;
            names.extend(["layer_count_steps", "layer_counts"]);
            m.add("__all__", names)?;
        }
        "core.reference_edit" => {
            let p = core.getattr("command_protocol")?;
            let h = py.import("built_core.history")?;
            for (n, b, c, flag, init) in [
                (
                    "ReshareInstanceCommand",
                    "ReshareStep",
                    "Command",
                    "reference_instance",
                    wrap_pyfunction!(instance, m)?,
                ),
                (
                    "ReshareAssemblyCommand",
                    "ReshareStep",
                    "Command",
                    "reference_assembly",
                    wrap_pyfunction!(assembly, m)?,
                ),
                (
                    "_StoreSession",
                    "StoreSessionStep",
                    "MeshCommand",
                    "",
                    wrap_pyfunction!(session, m)?,
                ),
            ] {
                let ns = PyDict::new(py);
                method(&ns, "__init__", init.into_any())?;
                if !flag.is_empty() {
                    ns.set_item(flag, true)?
                }
                class(m, n, vec![p.getattr(b)?, h.getattr(c)?], &ns)?;
            }
            m.add("record_leave", p.getattr("record_leave")?)?;
        }
        "core.link_structure" => {
            m.add("LINKED", "linked")?;
            m.add(
                "LINKED_NAME",
                py.import("core.links")?.getattr("LINKED_FOLDER")?,
            )?;
            m.add("STATES", ("resolved", "modified", "missing", "packed"))?;
            let ns = PyDict::new(py);
            meth!(ns, m, "__init__", tree_init);
            meth!(ns, m, "address", address);
            meth!(ns, m, "row_id", tree_row);
            meth!(ns, m, "resolve", resolve);
            meth!(ns, m, "order", order);
            meth!(ns, m, "subtree", subtree);
            meth!(ns, m, "members", members);
            meth!(ns, m, "file_tags", file_tags);
            class(m, "Tree", vec![], &ns)?;
            m.add_function(wrap_pyfunction!(mount_key, m)?)?;
            m.add_function(wrap_pyfunction!(is_linked_key, m)?)?;
            m.add_function(wrap_pyfunction!(row_id, m)?)?;
            m.add_function(wrap_pyfunction!(file_state, m)?)?;
        }
        _ => return Err(PyAttributeError::new_err(name.to_owned())),
    }
    Ok(())
}
