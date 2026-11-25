// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use pyo3::{
    exceptions::PyValueError,
    types::{PyList, PySet, PyString},
};
fn dictionary(py: Python<'_>) -> Obj<'_> {
    PyDict::new(py).into_any()
}
fn values<'p>(d: &Obj<'p>, name: &str) -> PyResult<Obj<'p>> {
    d.call_method1("get", (name, dictionary(d.py())))
}
fn items<'p>(d: &Obj<'p>) -> PyResult<impl Iterator<Item = PyResult<Obj<'p>>> + 'p> {
    Ok(d.call_method0("items")?.try_iter()?)
}
fn copy_value<'p>(v: &Obj<'p>) -> PyResult<Obj<'p>> {
    v.py().import("copy")?.call_method1("deepcopy", (v,))
}
#[pyfunction]
fn _nodes<'p>(registry: &Obj<'p>) -> PyResult<Obj<'p>> {
    values(&values(registry, "structure")?, "nodes")
}
#[pyfunction]
fn _lineage<'p>(registry: &Obj<'p>, key: &Obj<'p>) -> PyResult<Vec<String>> {
    crate::structure::structure_lineage(&_nodes(registry)?, key)
}
#[pyfunction]
fn tree<'p>(registry: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = registry.py();
    let d = dictionary(py);
    let nodes = _nodes(registry)?;
    d.set_item(py.None(), PyList::empty(py))?;
    for row in items(&nodes)? {
        let row = row?;
        let key = row.get_item(0)?;
        let parent = row.get_item(1)?.get_item("parent")?;
        d.call_method1("setdefault", (parent, PyList::empty(py)))?
            .call_method1("append", (&key,))?;
        d.call_method1("setdefault", (key, PyList::empty(py)))?;
    }
    for children in d.call_method0("values")?.try_iter()? {
        let list = children?;
        let pairs = PyList::empty(py);
        for (i, key) in list.try_iter()?.enumerate() {
            let key = key?;
            pairs.append((nodes.get_item(&key)?.get_item("order")?, i, key))?;
        }
        pairs.call_method0("sort")?;
        for (i, row) in pairs.iter().enumerate() {
            list.set_item(i, row.get_item(2)?)?;
        }
    }
    Ok(d)
}
#[pyfunction]
fn path<'p>(registry: &Obj<'p>, key: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    let nodes = _nodes(registry)?;
    _lineage(registry, key)?
        .into_iter()
        .map(|k| nodes.get_item(k)?.get_item("name"))
        .collect()
}
#[pyfunction]
fn resolve<'p>(registry: &Obj<'p>, key: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = registry.py();
    let nodes = _nodes(registry)?;
    let recipe = dictionary(py);
    let codes = dictionary(py);
    let systems = dictionary(py);
    let psets = dictionary(py);
    let mut names = Vec::new();
    for k in _lineage(registry, key)? {
        let node = nodes.get_item(k)?;
        names.push(node.get_item("name")?);
        let local = values(
            &node,
            if node.contains("source")? {
                "source_recipe"
            } else {
                "recipe"
            },
        )?;
        for row in items(&local)? {
            let row = row?;
            let field = row.get_item(0)?;
            let v = row.get_item(1)?;
            match field.extract::<String>()?.as_str() {
                "systems" => {
                    for x in v.try_iter()? {
                        systems.set_item(x?, py.None())?;
                    }
                }
                "classifications" => {
                    for x in v.try_iter()? {
                        let x = x?;
                        codes.set_item(
                            (x.get_item("system")?, x.get_item("code")?),
                            copy_value(&x)?,
                        )?;
                    }
                }
                "psets" => {
                    for r in items(&v)? {
                        let r = r?;
                        psets
                            .call_method1("setdefault", (r.get_item(0)?, dictionary(py)))?
                            .call_method1("update", (copy_value(&r.get_item(1)?)?,))?;
                    }
                }
                _ => recipe.set_item(field, copy_value(&v)?)?,
            }
        }
    }
    let leaf = names
        .pop()
        .ok_or_else(|| PyValueError::new_err("Empty lineage"))?;
    recipe.call_method1("setdefault", ("tag", leaf))?;
    let folder = recipe.call_method1("get", ("folder", names))?;
    if folder.is_instance_of::<PyString>() {
        let list = PyList::empty(py);
        if folder.is_truthy()? {
            list.append(folder)?;
        }
        recipe.set_item("folder", list)?;
    } else {
        recipe.set_item("folder", folder)?;
    }
    let list = py.import("builtins")?.getattr("list")?;
    recipe.set_item("systems", list.call1((systems,))?)?;
    recipe.set_item(
        "classifications",
        list.call1((codes.call_method0("values")?,))?,
    )?;
    recipe.set_item("psets", psets)?;
    Ok(recipe)
}
#[pyfunction]
fn node_of<'p>(group: &Obj<'p>) -> PyResult<Obj<'p>> {
    let ext = group.getattr("ext")?;
    if ext.is_truthy()? {
        let s = ext.call_method1("get", ("structure",))?;
        if s.is_truthy()? {
            return s.call_method1("get", ("node",));
        }
    }
    Ok(group.py().None().into_bound(group.py()))
}
#[pyfunction]
fn _patch<'p>(group: &Obj<'p>, recipe: &Obj<'p>, registry: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = group.py();
    let mut old = group.getattr("ifc")?;
    if !old.is_truthy()? {
        old = dictionary(py);
    }
    let get = |v: &Obj<'p>, k: &str| v.call_method1("get", (k,));
    let mut cls = get(recipe, "class")?;
    let oldcls = get(&old, "class")?;
    let allowed = get(recipe, "classes_allowed")?;
    if !allowed.is_none() && !oldcls.is_none() {
        if !allowed.contains(&oldcls)? {
            return Err(PyValueError::new_err(format!(
                "{}: {} outside classes_allowed",
                group.getattr("uid")?,
                oldcls
            )));
        }
        if !cls.is_none() {
            cls = oldcls;
        }
    }
    let out = dictionary(py);
    if cls.is_none() {
        return Ok(out);
    }
    let pre = get(recipe, "predefined_type")?;
    if pre.is_truthy()? {
        let api = py.import("core.ifc_tables")?;
        let mut valid = false;
        for s in api
            .getattr("TABLES")?
            .get_item(&cls)?
            .get_item("schemas")?
            .try_iter()?
        {
            if api
                .call_method1("predefined_types", (&cls, s?))?
                .contains(&pre)?
            {
                valid = true;
                break;
            }
        }
        if !valid {
            return Err(PyValueError::new_err(format!(
                "{}: {} is invalid for {}",
                group.getattr("uid")?,
                pre,
                cls
            )));
        }
    }
    let codes = copy_value(&recipe.get_item("classifications")?)?;
    let namespaces = PySet::empty(py)?;
    for c in codes.try_iter()? {
        let c = c?;
        let system = c.get_item("system")?;
        namespaces.add(&system)?;
        let name = registry
            .get_item("classification_systems")?
            .get_item(system)?
            .get_item("codes")?
            .get_item(c.get_item("code")?)?
            .get_item("name")?;
        c.call_method1("setdefault", ("name", name))?;
    }
    let all = PyList::empty(py);
    for c in values(&old, "classifications")?.try_iter()? {
        let c = c?;
        if !namespaces.contains(get(&c, "system")?)? {
            all.append(c)?;
        }
    }
    all.call_method1("extend", (codes,))?;
    let psets = dictionary(py);
    let typed = py.import("core.record")?.getattr("typed")?;
    for r in items(&recipe.get_item("psets")?)? {
        let r = r?;
        let props = dictionary(py);
        for p in items(&r.get_item(1)?)? {
            let p = p?;
            props.set_item(p.get_item(0)?, typed.call1((p.get_item(1)?,))?)?;
        }
        psets.set_item(r.get_item(0)?, props)?;
    }
    for r in items(&values(&old, "psets")?)? {
        let r = r?;
        let props = psets.call_method1("setdefault", (r.get_item(0)?, dictionary(py)))?;
        for p in items(&r.get_item(1)?)? {
            let p = p?;
            let name = p.get_item(0)?;
            let v = p.get_item(1)?;
            let stored = if v.is_instance_of::<PyDict>() {
                get(&v, "value")?
            } else {
                v.clone()
            };
            if !stored.is_none() || !props.contains(&name)? {
                props.set_item(name, v)?;
            }
        }
    }
    for (k, v) in [
        ("class", cls),
        ("predefined_type", pre),
        ("type", get(recipe, "type")?),
        (
            "object_type",
            recipe.call_method1("get", ("object_type", ""))?,
        ),
        ("systems", recipe.get_item("systems")?),
        ("classifications", all.into_any()),
        ("psets", psets),
    ] {
        if !get(&old, k)?.eq(&v)? && (v.is_truthy()? || old.contains(k)?) {
            out.set_item(k, v)?;
        }
    }
    Ok(out)
}
fn registry<'p>(scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    scene
        .py()
        .import("core.bim_registry")?
        .call_method1("get", (scene,))
}
#[pyfunction(pass_module)]
fn file<'p>(m: &Module<'p>, scene: &Obj<'p>, uids: &Obj<'p>, key: &Obj<'p>) -> PyResult<Obj<'p>> {
    let groups = m.call_method1("_targets", (scene, uids))?;
    let r = registry(scene)?;
    let batch = dictionary(scene.py());
    batch.set_item(key, &groups)?;
    m.call_method1("_check_paths", (scene, &r, batch))?;
    m.call_method1(
        "CompoundCommand",
        (m.call_method1("_filing", (scene, groups, r, key, dictionary(scene.py())))?,),
    )
}
#[pyfunction(pass_module)]
fn file_instances<'p>(
    m: &Module<'p>,
    scene: &Obj<'p>,
    uid: &Obj<'p>,
    key: &Obj<'p>,
) -> PyResult<Obj<'p>> {
    let g = m
        .call_method1("_targets", (scene, vec![uid]))?
        .get_item(0)?;
    let mesh = g.getattr("mesh")?;
    let ids = PyList::empty(scene.py());
    for p in m.call_method1("_placements", (scene,))?.try_iter()? {
        let p = p?;
        let g = p.get_item(0)?;
        if !p.get_item(1)?.is_truthy()? && g.getattr("mesh")?.is(&mesh) {
            ids.append(g.getattr("uid")?)?;
        }
    }
    file(m, scene, ids.as_any(), key)
}
#[pyfunction(pass_module)]
fn unfile<'p>(m: &Module<'p>, scene: &Obj<'p>, uids: &Obj<'p>) -> PyResult<Obj<'p>> {
    m.call_method1(
        "SetStructureNodeCommand",
        (
            m.call_method1("_targets", (scene, uids))?,
            scene.py().None(),
        ),
    )
}
#[pyfunction]
fn _mount_index<'p>(registry: &Obj<'p>) -> PyResult<(Obj<'p>, Obj<'p>, Obj<'p>)> {
    let py = registry.py();
    let (m, s, t) = (dictionary(py), dictionary(py), dictionary(py));
    for row in items(&_nodes(registry)?)? {
        let row = row?;
        let key = row.get_item(0)?;
        let node = row.get_item(1)?;
        if node.contains("source")? {
            let src = node.get_item("source")?;
            if node.contains("source_node")? {
                s.set_item((src, node.get_item("source_node")?), key)?;
            } else if node.contains("source_tag")? {
                t.set_item((src, node.get_item("source_tag")?), key)?;
            } else if node.call_method1("get", ("mount",))?.is_truthy()? {
                m.set_item(src, key)?;
            }
        }
    }
    Ok((m, s, t))
}
#[pyfunction]
fn _membership<'p>(group: &Obj<'p>, source: &Obj<'p>, mount_index: &Obj<'p>) -> PyResult<Obj<'p>> {
    let key = node_of(group)?;
    if !source.is_truthy()? {
        return Ok(key);
    }
    if group
        .py()
        .import("built_core.links")?
        .call_method1("is_linked", (group,))?
        .is_truthy()?
    {
        return mount_index.get_item(0)?.call_method1("get", (source,));
    }
    mount_index.get_item(1)?.call_method1(
        "get",
        (
            (source, key),
            mount_index
                .get_item(2)?
                .call_method1("get", ((source, group.getattr("layer")?),))?,
        ),
    )
}
#[pyfunction(pass_module)]
fn _memberships<'p>(m: &Module<'p>, scene: &Obj<'p>, registry: &Obj<'p>) -> PyResult<Obj<'p>> {
    let index = _mount_index(registry)?.into_pyobject(m.py())?.into_any();
    let rows = PyList::empty(m.py());
    for p in m.call_method1("_placements", (scene,))?.try_iter()? {
        let p = p?;
        let g = p.get_item(0)?;
        let src = p.get_item(1)?;
        let k = _membership(&g, &src, &index)?;
        rows.append((g, src, p.get_item(2)?, k))?;
    }
    m.py().import("builtins")?.call_method1("iter", (rows,))
}
#[pyfunction(pass_module,signature=(scene,key,recursive=true))]
fn members<'p>(
    m: &Module<'p>,
    scene: &Obj<'p>,
    key: &Obj<'p>,
    recursive: bool,
) -> PyResult<Vec<Obj<'p>>> {
    let r = registry(scene)?;
    let children = tree(&r)?;
    if !_nodes(&r)?.contains(key)? {
        return Err(PyValueError::new_err(format!("Unknown node: {key}")));
    }
    let keys = PySet::empty(m.py())?;
    let mut stack = vec![key.clone()];
    while let Some(k) = stack.pop() {
        keys.add(&k)?;
        if recursive {
            for child in children.get_item(k)?.try_iter()? {
                stack.push(child?);
            }
        }
    }
    let mut out = Vec::new();
    for row in _memberships(m, scene, &r)?.try_iter()? {
        let row = row?;
        if keys.contains(row.get_item(3)?)? {
            out.push(row.get_item(0)?.getattr("uid")?);
        }
    }
    Ok(out)
}
#[pyfunction(pass_module,signature=(scene,keys=None))]
fn resync<'p>(m: &Module<'p>, scene: &Obj<'p>, keys: Option<&Obj<'p>>) -> PyResult<Obj<'p>> {
    m.call_method1("_resync", (scene, registry(scene)?, keys))
}
#[pyfunction]
fn _is_unsorted(
    group: &Obj<'_>,
    source: &Obj<'_>,
    root: &Obj<'_>,
    key: &Obj<'_>,
    recipes: &Obj<'_>,
) -> PyResult<bool> {
    if source.is_truthy()? {
        return Ok(false);
    }
    if !recipes.contains(key)? {
        return Ok(root.is_truthy()? || !key.is_none());
    }
    let recipe = recipes.get_item(key)?;
    let rec = group.getattr("ifc")?;
    Ok(rec.is_truthy()?
        && !recipe
            .call_method1(
                "get",
                (
                    "classes_allowed",
                    vec![recipe.call_method1("get", ("class",))?],
                ),
            )?
            .contains(rec.call_method1("get", ("class",))?)?)
}
#[pyfunction(pass_module)]
fn unsorted<'p>(m: &Module<'p>, scene: &Obj<'p>) -> PyResult<Vec<Obj<'p>>> {
    let r = registry(scene)?;
    let recipes = dictionary(m.py());
    for k in _nodes(&r)?.try_iter()? {
        let k = k?;
        recipes.set_item(&k, resolve(&r, &k)?)?;
    }
    let mut out = Vec::new();
    for row in _memberships(m, scene, &r)?.try_iter()? {
        let row = row?;
        let g = row.get_item(0)?;
        if _is_unsorted(
            &g,
            &row.get_item(1)?,
            &row.get_item(2)?,
            &row.get_item(3)?,
            &recipes,
        )? {
            out.push(g.getattr("uid")?);
        }
    }
    Ok(out)
}
#[pyfunction]
fn _has_status(record: &Obj<'_>) -> PyResult<bool> {
    for row in items(&values(record, "psets")?)? {
        let row = row?;
        if row.get_item(0)?.extract::<String>()?.ends_with("Common") {
            let mut v = row.get_item(1)?.call_method1("get", ("Status",))?;
            if v.is_instance_of::<PyDict>() {
                v = v.call_method1("get", ("value",))?;
            }
            if v.is_truthy()? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
#[pyfunction(pass_module)]
fn completeness<'p>(m: &Module<'p>, scene: &Obj<'p>, registry: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = m.py();
    let ids = py.import("core.structure_ids")?;
    let req = ids.call_method1("load_requirements", (scene, registry))?;
    let stats = dictionary(py);
    let lineage = dictionary(py);
    for k in _nodes(registry)?.try_iter()? {
        let k = k?;
        let row = dictionary(py);
        for n in ["members", "without_record", "missing_status"] {
            row.set_item(n, 0)?;
        }
        row.set_item("ids_hits", PyList::empty(py))?;
        stats.set_item(&k, row)?;
        lineage.set_item(&k, _lineage(registry, &k)?)?;
    }
    for row in _memberships(m, scene, registry)?.try_iter()? {
        let row = row?;
        let k = row.get_item(3)?;
        if !lineage.contains(&k)? {
            continue;
        }
        let group = row.get_item(0)?;
        let rec = group.getattr("ifc")?;
        let has = rec.is_truthy()?;
        let missing = has && !_has_status(&rec)?;
        let hits = ids.call_method1("requirement_hits", (group, registry, &req))?;
        for ancestor in lineage.get_item(k)?.try_iter()? {
            let row = stats.get_item(ancestor?)?;
            for (n, add) in [
                ("members", true),
                ("without_record", !has),
                ("missing_status", missing),
            ] {
                if add {
                    row.set_item(n, row.get_item(n)?.add(1)?)?;
                }
            }
            row.get_item("ids_hits")?.call_method1("extend", (&hits,))?;
        }
    }
    Ok(stats)
}
#[pyfunction]
fn move_init(this: &Obj<'_>, layer: &Obj<'_>, folder: &Obj<'_>) -> PyResult<()> {
    this.setattr("layer", layer)?;
    this.setattr("folder", folder)
}
#[pyfunction]
fn move_do(this: &Obj<'_>, scene: &Obj<'_>) -> PyResult<()> {
    this.setattr(
        "before",
        crate::definition_edits::set_placement_field(
            &this.getattr("layer")?,
            "folder",
            &this.getattr("folder")?,
        )?,
    )?;
    scene.setattr("version", scene.getattr("version")?.add(1)?)
}
#[pyfunction]
fn move_undo(this: &Obj<'_>, scene: &Obj<'_>) -> PyResult<()> {
    this.getattr("layer")?
        .setattr("folder", this.getattr("before")?)?;
    scene.setattr("version", scene.getattr("version")?.add(1)?)
}
#[pyfunction(pass_module, name = "__getattr__")]
fn dependency<'p>(m: &Module<'p>, name: &str) -> PyResult<Obj<'p>> {
    let py = m.py();
    let v = match name {
        "bim_registry" => py.import("core.bim_registry")?.into_any(),
        "links" => py.import("built_core.links")?.into_any(),
        "DEFAULT_LAYER" | "layer_of" => py.import("built_core.layers")?.getattr(name)?,
        "new_uid" => py.import("built_core.group")?.getattr(name)?,
        _ => return Err(pyo3::exceptions::PyAttributeError::new_err(name.to_owned())),
    };
    m.add(name, &v)?;
    Ok(v)
}
pub(super) fn install(m: &Module<'_>, core: &Module<'_>) -> PyResult<()> {
    let py = m.py();
    for (name, attr) in [
        ("_targets", "structure_targets"),
        ("_placements", "StructurePlacements"),
        ("_filing", "plan_filing"),
        ("_check_paths", "check_filing_paths"),
        ("_resync", "resync_filing"),
        ("_EnsureStructureLayers", "EnsureStructureLayersCommand"),
        ("bootstrap", "structure_proposal"),
        ("apply_bootstrap", "apply_structure_bootstrap"),
    ] {
        m.add(name, core.getattr(attr)?)?;
    }
    let hist = core.getattr("history")?;
    for n in [
        "Command",
        "CompoundCommand",
        "AssignLayerCommand",
        "SetRecordCommand",
        "SetRegistryCommand",
        "SetStructureNodeCommand",
        "EnsureLayersCommand",
    ] {
        m.add(n, hist.getattr(n)?)?;
    }
    m.add("deepcopy", py.import("copy")?.getattr("deepcopy")?)?;
    let ns = PyDict::new(py);
    ns.set_item("doc_undo", true)?;
    ns.set_item("__module__", "built_core.structure")?;
    for (n, f) in [
        ("__init__", wrap_pyfunction!(move_init, m)?),
        ("do", wrap_pyfunction!(move_do, m)?),
        ("undo", wrap_pyfunction!(move_undo, m)?),
    ] {
        method(&ns, n, f.as_any())?;
    }
    m.add(
        "_MoveTag",
        py.import("builtins")?
            .call_method1("type", ("_MoveTag", (hist.getattr("Command")?,), ns))?,
    )?;
    macro_rules! funcs{($($name:ident),*)=>{$(m.add_function(wrap_pyfunction!($name,m)?)?;)*};}
    funcs!(
        _nodes,
        _lineage,
        tree,
        path,
        resolve,
        node_of,
        _patch,
        file,
        file_instances,
        unfile,
        _mount_index,
        _membership,
        _memberships,
        members,
        resync,
        unsorted,
        _is_unsorted,
        _has_status,
        completeness,
        dependency
    );
    Ok(())
}
