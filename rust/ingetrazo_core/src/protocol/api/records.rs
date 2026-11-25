use super::*;
fn recipe<'p>(g: &Obj<'p>, reg: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = g.py();
    let s = py.import("ingetrazo_core.structure")?;
    let node = s.call_method1("node_of", (g,))?;
    if s.call_method1("_nodes", (reg,))?.contains(&node)? {
        s.call_method1("resolve", (reg, node))
    } else {
        Ok(dict(py))
    }
}
fn child_dict<'p>(obj: &Obj<'p>, k: &Obj<'p>) -> PyResult<Obj<'p>> {
    obj.call_method1("get", (k, dict(obj.py())))
}
fn field<'p>(
    fields: &Bound<'p, PyList>,
    path: Obj<'p>,
    value: Obj<'p>,
    owner: &str,
    editable: bool,
    kind: Obj<'p>,
) -> PyResult<()> {
    let d = dict(fields.py());
    d.set_item("path", path)?;
    d.set_item("value", value)?;
    d.set_item("owner", owner)?;
    d.set_item("editable", editable)?;
    d.set_item("kind", kind)?;
    fields.append(d)
}
#[pyfunction(signature=(group,registry,linked=false))]
fn occurrence<'p>(group: &Obj<'p>, registry: &Obj<'p>, linked: bool) -> PyResult<Obj<'p>> {
    let py = group.py();
    let rec = py.import("core.record")?;
    let own = rec.call_method1("normalize", (otherwise(group.getattr("ifc")?, dict(py))?,))?;
    let filing = rec.call_method1("normalize", (recipe(group, registry)?,))?;
    let effective = rec.call_method1("apply_type", (&own, registry))?;
    let fields = PyList::empty(py);
    let editable = !linked && !group.getattr("locked")?.is_truthy()?;
    let owner = |o| if linked { "Linked source" } else { o };
    let text = "text".into_pyobject(py)?.into_any();
    for k in [
        "class",
        "predefined_type",
        "type",
        "object_type",
        "name",
        "description",
        "tag",
    ] {
        let filed = get(&filing, "class")?.is_truthy()?
            && matches!(k, "class" | "predefined_type" | "type" | "object_type");
        let o = if effective.contains(k)? && !own.contains(k)? {
            "Type"
        } else if filed {
            "Filing node"
        } else {
            "This object"
        };
        field(
            &fields,
            PyList::new(py, [k])?.into_any(),
            otherwise(get(&effective, k)?, "".into_pyobject(py)?.into_any())?,
            owner(o),
            editable && !filed,
            text.clone(),
        )?;
    }
    if let Ok(ps) = get(&effective, "psets")?.cast_into::<PyDict>() {
        for (name, props) in ps.iter() {
            if let Ok(props) = props.cast_into::<PyDict>() {
                for (k, entry) in props.iter() {
                    let source = |obj: &Obj<'p>| {
                        child_dict(&obj.call_method1("get", ("psets", dict(py)))?, &name)
                    };
                    let default = source(&filing)?.call_method1("get", (&k,))?;
                    let o = if !source(&own)?.contains(&k)? {
                        "Type"
                    } else if entry.eq(default)? {
                        "Filing node"
                    } else {
                        "This object"
                    };
                    field(
                        &fields,
                        PyList::new(py, ["psets".into_pyobject(py)?.into_any(), name.clone(), k])?
                            .into_any(),
                        get(&entry, "value")?,
                        owner(o),
                        editable,
                        get(&entry, "kind")?,
                    )?;
                }
            }
        }
    }
    let codes = pyo3::types::PySet::empty(py)?;
    for c in filing
        .call_method1("get", ("classifications", list(py)))?
        .try_iter()?
    {
        codes.add(c?.get_item("system")?)?
    }
    for c in otherwise(get(&effective, "classifications")?, list(py))?.try_iter()? {
        let c = c?;
        if !c.is_instance_of::<PyDict>() {
            continue;
        }
        let system = c.call_method1("get", ("system", ""))?;
        let filed = codes.contains(&system)?;
        let o = if filed {
            "Filing node"
        } else if own.contains("classifications")? {
            "This object"
        } else {
            "Type"
        };
        let code = c.call_method1("get", ("code", ""))?;
        field(
            &fields,
            PyList::new(
                py,
                [
                    "classifications".into_pyobject(py)?.into_any(),
                    system,
                    code,
                ],
            )?
            .into_any(),
            c.call_method1("get", ("name", ""))?,
            owner(o),
            editable && !filed,
            text.clone(),
        )?;
    }
    let guid = get(&effective, "guid")?;
    if guid.is_truthy()? {
        field(
            &fields,
            PyList::new(py, ["guid"])?.into_any(),
            guid,
            owner("This object"),
            false,
            text,
        )?
    }
    let out = dict(py);
    out.set_item("uid", group.getattr("uid")?)?;
    out.set_item("fields", fields)?;
    out.set_item("ifc_class", get(&effective, "class")?)?;
    out.set_item("editable", editable)?;
    Ok(out)
}
#[pyfunction(signature=(history,placements,path,value,schema="IFC4",property_kind=None))]
fn patch_record_field<'p>(
    history: &Obj<'p>,
    placements: &Obj<'p>,
    path: &Obj<'p>,
    value: &Obj<'p>,
    schema: &str,
    property_kind: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let py = history.py();
    let scene = history.getattr("scene")?;
    let reg = py
        .import("core.bim_registry")?
        .call_method1("get", (&scene,))?;
    let rec = py.import("core.record")?;
    let hist = py.import("ingetrazo_core.history")?;
    let cmds = PyList::empty(py);
    for g in py
        .import("ingetrazo_core.structure")?
        .call_method1("_targets", (&scene, placements))?
        .try_iter()?
    {
        let g = g?;
        let mut kind = None;
        for f in occurrence(&g, &reg, false)?
            .get_item("fields")?
            .try_iter()?
        {
            let f = f?;
            if f.get_item("path")?.eq(path)? {
                if f.get_item("editable")?.is_truthy()? {
                    kind = Some(f.get_item("kind")?)
                }
                break;
            }
        }
        let keys: Vec<String> = path.extract()?;
        if kind.is_none()
            && keys.len() == 3
            && keys[0] == "psets"
            && keys.iter().all(|v| !v.trim().is_empty())
            && property_kind.is_some_and(|k| {
                rec.getattr("KINDS")
                    .and_then(|ks| ks.contains(k))
                    .unwrap_or(false)
            })
        {
            kind = property_kind.cloned();
        }
        if kind.is_none() || g.getattr("locked")?.is_truthy()? {
            return Err(PyValueError::new_err(format!(
                "Field is read-only or absent on {}",
                g.getattr("uid")?
            )));
        }
        let k = &keys[0];
        if k == "class" {
            let allowed = get(&recipe(&g, &reg)?, "classes_allowed")?;
            if !allowed.is_none() && !allowed.contains(value)? {
                return Err(PyValueError::new_err(
                    "Class is outside the filing rule allowlist",
                ));
            }
        }
        let old = otherwise(g.getattr("ifc")?, dict(py))?;
        let patch = dict(py);
        match k.as_str() {
            "psets" => {
                let (set, prop) = (&keys[1], &keys[2]);
                let ps = copied(&old.call_method1("get", ("psets", dict(py)))?)?;
                let props = ps.call_method1("get", (set, dict(py)))?;
                let entry =
                    rec.call_method1("typed", (props.call_method1("get", (prop, dict(py)))?,))?;
                if value.is_none() {
                    let def = recipe(&g, &reg)?
                        .call_method1("get", ("psets", dict(py)))?
                        .call_method1("get", (set, dict(py)))?
                        .call_method1("get", (prop,))?;
                    if def.is_none() {
                        props.call_method1("pop", (prop, py.None()))?;
                        if !props.is_truthy()? {
                            ps.call_method1("pop", (set, py.None()))?;
                        }
                    } else {
                        props.set_item(prop, rec.call_method1("typed", (def,))?)?;
                        ps.set_item(set, props)?;
                    }
                } else {
                    entry.set_item("kind", kind)?;
                    entry.set_item("value", value)?;
                    props.set_item(prop, entry)?;
                    ps.set_item(set, props)?;
                }
                patch.set_item(k, ps)?;
            }
            "classifications" => {
                let codes = copied(&rec.call_method1("apply_type", (&old, &reg))?.get_item(k)?)?;
                for c in codes.try_iter()? {
                    let c = c?;
                    if get(&c, "system")?.eq(&keys[1])? && get(&c, "code")?.eq(&keys[2])? {
                        c.set_item("name", value)?
                    }
                }
                patch.set_item(k, codes)?;
            }
            _ => patch.set_item(
                k,
                if matches!(k.as_str(), "type" | "predefined_type") && value.eq("")? {
                    none(py)
                } else {
                    value.clone()
                },
            )?,
        }
        let candidate = py.get_type::<PyDict>().call1((&old,))?;
        candidate.call_method1("update", (&patch,))?;
        let errors = rec.call_method1(
            "validate",
            (
                rec.call_method1("apply_type", (candidate, &reg))?,
                &reg,
                schema,
            ),
        )?;
        if errors.is_truthy()? {
            return Err(PyValueError::new_err(
                "; ".into_pyobject(py)?
                    .call_method1("join", (errors,))?
                    .extract::<String>()?,
            ));
        }
        cmds.append(hist.call_method1("SetRecordCommand", (PyList::new(py, [g])?, patch))?)?
    }
    let before = history.getattr("revision")?;
    history.call_method1(
        "execute",
        (if cmds.len() > 1 {
            hist.call_method1("CompoundCommand", (cmds,))?
        } else {
            cmds.get_item(0)?
        },),
    )?;
    Ok(super::super::answer(history, &before, true)?.into_any())
}
#[pyfunction(signature=(scene,key,recipe,*,commit_history=None,expected=None))]
fn filing_rule<'p>(
    scene: &Obj<'p>,
    key: &Obj<'p>,
    recipe: &Obj<'p>,
    commit_history: Option<&Obj<'p>>,
    expected: Option<&Obj<'p>>,
) -> PyResult<Obj<'p>> {
    let py = scene.py();
    let reg = py
        .import("core.bim_registry")?
        .call_method1("get", (scene,))?;
    let structure = py.import("ingetrazo_core.structure")?;
    let node = structure
        .call_method1("_nodes", (&reg,))?
        .call_method1("get", (key,))?;
    if node.is_none() || node.contains("source")? {
        return Err(PyValueError::new_err("Select an editable filing node"));
    }
    if !recipe.is_instance_of::<PyDict>() {
        return Err(PyValueError::new_err("The filing rule must be an object"));
    }
    if let Some(e) = expected {
        if !node.call_method1("get", ("recipe", dict(py)))?.eq(e)? {
            return Err(PyValueError::new_err(
                "The filing rule changed. Reopen the preview.",
            ));
        }
    }
    node.set_item("recipe", copied(recipe)?)?;
    let hist = py.import("ingetrazo_core.history")?;
    let update = hist.call_method1("SetRegistryCommand", (&reg,))?;
    let (sync, report): (Obj, Obj) = structure
        .call_method1("_resync", (scene, reg, PyList::new(py, [key])?))?
        .extract()?;
    if let Some(h) = commit_history {
        let cmds = PyList::new(py, [update])?;
        cmds.call_method1("extend", (sync.getattr("commands")?,))?;
        h.call_method1("execute", (hist.call_method1("CompoundCommand", (cmds,))?,))?;
    }
    let out = dict(py);
    out.set_item("changed", report.len()?)?;
    out.set_item("changes", report)?;
    Ok(out)
}
#[pyfunction]
fn set_filing_rule<'p>(
    history: &Obj<'p>,
    node: &Obj<'p>,
    recipe: &Obj<'p>,
    expected: &Obj<'p>,
) -> PyResult<Obj<'p>> {
    let before = history.getattr("revision")?;
    filing_rule(
        &history.getattr("scene")?,
        node,
        recipe,
        Some(history),
        if expected.is_none() {
            None
        } else {
            Some(expected)
        },
    )?;
    Ok(super::super::answer(history, &before, true)?.into_any())
}
pub(super) fn install(m: &Module<'_>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(occurrence, m)?)?;
    m.add_function(wrap_pyfunction!(patch_record_field, m)?)?;
    m.add_function(wrap_pyfunction!(filing_rule, m)?)?;
    m.add_function(wrap_pyfunction!(set_filing_rule, m)?)
}
