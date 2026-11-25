// SPDX-License-Identifier: GPL-3.0-or-later
//! Ordered occurrence metadata for the navigator, classification and saved views.
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList, PySet},
};
mod filters;
type Obj<'p> = Bound<'p, PyAny>;
fn api(py: Python) -> PyResult<Bound<PyModule>> {
    py.import("built_core.facets")
}
fn get<'p>(d: &Obj<'p>, key: &str) -> PyResult<Obj<'p>> {
    d.call_method1("get", (key,))
}
fn empty<'p>(v: Obj<'p>) -> PyResult<Obj<'p>> {
    if v.is_truthy()? {
        Ok(v)
    } else {
        Ok(PyDict::new(v.py()).into_any())
    }
}
fn items<'p>(d: &Obj<'p>, key: &str) -> PyResult<Obj<'p>> {
    d.call_method1("get", (key, PyList::empty(d.py())))
}
fn named(v: Obj<'_>) -> PyResult<Obj<'_>> {
    if v.is_instance_of::<PyDict>() {
        get(&v, "name")
    } else {
        Ok(v)
    }
}

struct Walk<'p> {
    index: Obj<'p>,
    groups: Obj<'p>,
    qualified: Obj<'p>,
    repeated: Obj<'p>,
    layers: Bound<'p, PyDict>,
    defs: Obj<'p>,
    link: Obj<'p>,
    role: Obj<'p>,
    row_id: Obj<'p>,
}
impl<'p> Walk<'p> {
    fn visit(
        &self,
        groups: &Obj<'p>,
        names: &mut Vec<String>,
        uids: &mut Vec<String>,
        ancestors: &mut Vec<String>,
        spatial: &Obj<'p>,
        source: &Obj<'p>,
    ) -> PyResult<()> {
        let py = groups.py();
        for g in groups.try_iter()? {
            let g = g?;
            let uid = g.getattr("uid")?.extract::<String>()?;
            let name = g.getattr("name")?.extract::<String>()?;
            let link = self.link.call1((&g,))?;
            let src = if link.is_truthy()? {
                link.get_item("path")?
            } else {
                source.clone()
            };
            let mesh = g.getattr("mesh")?;
            if !self.defs.contains(&mesh)? {
                self.defs.set_item(
                    &mesh,
                    format!("{}:{}", self.defs.len()? + 1, mesh.as_ptr() as usize),
                )?;
            }
            let rec = empty(g.getattr("ifc")?)?;
            let key = if self.repeated.contains(&uid)? || uid.starts_with("@[") {
                let mut path = uids.clone();
                path.push(uid.clone());
                let key = self.row_id.call1((path,))?.extract::<String>()?;
                self.qualified.set_item(&g, &key)?;
                key
            } else {
                uid.clone()
            };
            let row = PyDict::new(py);
            row.set_item("uid", &uid)?;
            row.set_item("occurrence", &key)?;
            row.set_item("path_keys", &*ancestors)?;
            row.set_item("problems", py.None())?;
            row.set_item("name", &name)?;
            row.set_item("path", names.join("/"))?;
            row.set_item("path_uids", &*uids)?;
            row.set_item("depth", uids.len())?;
            row.set_item(
                "definition",
                format!("{name} [{}]", self.defs.get_item(&mesh)?.str()?),
            )?;
            row.set_item("instance", !g.getattr("xform")?.is_none())?;
            let tag = g.getattr("layer")?;
            let layer = self.layers.get_item(&tag)?;
            row.set_item("tag", tag)?;
            row.set_item(
                "tag_uid",
                layer.as_ref().map(|l| l.getattr("uid")).transpose()?,
            )?;
            row.set_item(
                "tag_folder",
                match layer {
                    Some(l) => l.getattr_opt("folder")?,
                    None => None,
                },
            )?;
            row.set_item("hidden", g.getattr("hidden")?.is_truthy()?)?;
            row.set_item("linked", !src.is_none())?;
            row.set_item("link_source", &src)?;
            for k in ["predefined_type", "type", "class"] {
                row.set_item(k, get(&rec, k)?)?;
            }
            row.set_item("system", items(&rec, "systems")?)?;
            let s = get(&rec, "storey")?;
            row.set_item(
                "storey",
                named(if s.is_truthy()? { s } else { spatial.clone() })?,
            )?;
            row.set_item("status", py.None())?;
            row.set_item("material", named(get(&rec, "material")?)?)?;
            for item in items(&rec, "classifications")?.try_iter()? {
                let item = item?;
                row.call_method1(
                    "setdefault",
                    (
                        format!("kind:{}", item.get_item("system")?.str()?),
                        PyList::empty(py),
                    ),
                )?
                .call_method1("append", (item.get_item("code")?,))?;
            }
            if let Some(psets) = rec.cast::<PyDict>()?.get_item("psets")? {
                for (n, p) in psets.cast::<PyDict>()?.iter() {
                    if n.extract::<String>()?.ends_with("Common") && p.contains("Status")? {
                        let st = p.get_item("Status")?;
                        row.set_item(
                            "status",
                            if st.is_instance_of::<PyDict>() {
                                get(&st, "value")?
                            } else {
                                st
                            },
                        )?;
                    }
                }
            }
            self.index.set_item(&key, row)?;
            self.groups.set_item(&key, &g)?;
            let children = g.getattr("children")?;
            if children.is_truthy()? {
                let next = if self.role.call1((&rec,))?.eq("spatial")? {
                    g.getattr("name")?
                } else {
                    spatial.clone()
                };
                names.push(name);
                uids.push(uid);
                ancestors.push(key);
                self.visit(&children, names, uids, ancestors, &next, &src)?;
                names.pop();
                uids.pop();
                ancestors.pop();
            }
        }
        Ok(())
    }
}
#[pyfunction]
fn build<'p>(scene: &Obj<'p>) -> PyResult<Obj<'p>> {
    let py = scene.py();
    let groups = scene.getattr("groups")?;
    let key = (
        scene.as_ptr() as usize,
        scene.getattr("version")?,
        groups.len()?,
    )
        .into_pyobject(py)?;
    if let Some(cache) = scene.getattr_opt("_facet_index")? {
        if cache.is_truthy()? && cache.get_item(0)?.eq(&key)? {
            return cache.get_item(1);
        }
    }
    let module = api(py)?;
    let idx = module.call_method1("Index", (scene,))?;
    let layers = PyDict::new(py);
    for layer in scene.getattr("layers")?.try_iter()? {
        let l = layer?;
        layers.set_item(l.getattr("key")?, l)?;
    }
    let walk = Walk {
        index: idx.clone(),
        groups: idx.getattr("groups")?,
        qualified: idx.getattr("qualified")?,
        repeated: scene.call_method0("repeated_uids")?,
        layers,
        defs: module.getattr("_definitions")?,
        link: py.import("built_core.links")?.getattr("link_of")?,
        role: py.import("core.record")?.getattr("role")?,
        row_id: py.import("built_core.link_structure")?.getattr("row_id")?,
    };
    walk.visit(
        &groups,
        &mut vec![],
        &mut vec![],
        &mut vec![],
        &py.None().into_bound(py),
        &py.None().into_bound(py),
    )?;
    scene.setattr("_facet_index", (key, &idx))?;
    Ok(idx)
}
#[pyfunction]
fn prepare(idx: &Obj, facet: &str) -> PyResult<()> {
    let py = idx.py();
    if !matches!(facet, "structure" | "material" | "problems")
        || idx.getattr("lazy")?.contains(facet)?
    {
        return Ok(());
    }
    let scene = idx.getattr("scene")?;
    let registry = py
        .import("core.bim_registry")?
        .call_method1("get", (&scene,))?;
    if facet == "structure" {
        let st = py.import("built_core.structure")?;
        let roots = PyDict::new(py);
        for k in st.call_method1("_nodes", (&registry,))?.try_iter()? {
            let k = k?;
            roots.set_item(
                &k,
                st.call_method1("_lineage", (&registry, &k))?.get_item(0)?,
            )?;
        }
        for entry in st
            .call_method1("_memberships", (&scene, &registry))?
            .try_iter()?
        {
            let e = entry?;
            if let Some(r) = roots.get_item(e.get_item(3)?)? {
                let g = e.get_item(0)?;
                let key = idx
                    .getattr("qualified")?
                    .call_method1("get", (&g, g.getattr("uid")?))?;
                idx.get_item(key)?.set_item(facet, r)?;
            }
        }
    } else {
        let cache = PyDict::new(py);
        let groups = idx.getattr("groups")?;
        for (uid, row) in idx.cast::<PyDict>()?.iter() {
            let group = groups.get_item(uid)?;
            if facet == "problems" {
                let rec = group.getattr("ifc")?;
                row.set_item(
                    facet,
                    if rec.is_truthy()? {
                        py.import("core.record")?
                            .call_method1("validate", (rec, &registry))?
                            .len()?
                    } else {
                        0
                    },
                )?;
            } else if !row.get_item("material")?.is_truthy()? {
                let mesh = group.getattr("mesh")?;
                if !cache.contains(&mesh)? {
                    let areas = PyDict::new(py);
                    for f in mesh.getattr("faces")?.try_iter()? {
                        let f = f?;
                        let name = get(&f.getattr("attrs")?, "mat")?;
                        if name.is_truthy()? {
                            let total = areas
                                .get_item(&name)?
                                .map(|v| v.extract::<f64>())
                                .transpose()?
                                .unwrap_or_default()
                                + f.call_method0("area")?.extract::<f64>()?;
                            areas.set_item(name, total)?;
                        }
                    }
                    let mut best = None;
                    let mut max = f64::NEG_INFINITY;
                    for (k, v) in areas.iter() {
                        let v = v.extract::<f64>()?;
                        if v > max {
                            max = v;
                            best = Some(k);
                        }
                    }
                    cache.set_item(&mesh, best)?;
                }
                row.set_item(facet, cache.get_item(mesh)?)?;
            }
        }
    }
    idx.getattr("lazy")?.call_method1("add", (facet,))?;
    Ok(())
}
#[pyfunction]
fn apply_rule<'p>(
    scene: &Obj<'p>,
    history: &Obj<'p>,
    registry: &Obj<'p>,
    key: &str,
) -> PyResult<Obj<'p>> {
    let py = scene.py();
    let rule = registry.get_item("rules")?.get_item(key)?;
    let idx = build(scene)?;
    let uids = filters::evaluate(&idx, Some(&rule.get_item("filter")?))?;
    let h = py.import("built_core.history")?;
    let copy = py.import("copy")?.getattr("deepcopy")?;
    let validate = py.import("core.record")?.getattr("validate")?;
    let steps = PyList::empty(py);
    for uid in uids.iter() {
        if idx.get_item(&uid)?.get_item("linked")?.is_truthy()? {
            return Err(PyValueError::new_err(
                "Linked objects must be classified in their source file",
            ));
        }
        let g = idx.getattr("groups")?.get_item(uid)?;
        let apply = rule.get_item("apply")?;
        let patch = copy.call1((apply.call_method1("get", ("patch", PyDict::new(py)))?,))?;
        let rec = empty(g.getattr("ifc")?)?;
        let mut entries = list(&copy.call1((
            patch.call_method1("get", ("classifications", items(&rec, "classifications")?))?,
        ))?)?;
        for item in items(&apply, "classifications")?.try_iter()? {
            let item = item?;
            let keep = PyList::empty(py);
            for c in entries.iter() {
                if !c.get_item("system")?.eq(item.get_item("system")?)?
                    || !c.get_item("code")?.eq(item.get_item("code")?)?
                {
                    keep.append(c)?;
                }
            }
            keep.append(copy.call1((item,))?)?;
            entries = keep;
        }
        if !entries.is_empty() {
            patch.set_item("classifications", entries)?;
        }
        let merged = rec.cast::<PyDict>()?.copy()?;
        merged.call_method1("update", (&patch,))?;
        let errors = validate
            .call1((merged, registry))?
            .extract::<Vec<String>>()?;
        if !errors.is_empty() {
            return Err(PyValueError::new_err(errors.join("; ")));
        }
        steps.append(h.call_method1("SetRecordCommand", (PyList::new(py, [g])?, patch))?)?;
    }
    if !steps.is_empty() {
        history.call_method1("execute", (h.call_method1("CompoundCommand", (steps,))?,))?;
    }
    Ok(uids.into_any())
}
fn list<'p>(v: &Obj<'p>) -> PyResult<Bound<'p, PyList>> {
    PyList::new(v.py(), v.try_iter()?.collect::<PyResult<Vec<_>>>()?)
}
#[pyfunction]
fn init_index(index: &Obj, scene: &Obj) -> PyResult<()> {
    index.setattr("scene", scene)?;
    index.setattr("groups", PyDict::new(index.py()))?;
    index.setattr("qualified", PyDict::new(index.py()))?;
    index.setattr("lazy", PySet::empty(index.py())?)
}

pub fn register(m: &Bound<PyModule>) -> PyResult<()> {
    let py = m.py();
    let name = "built_core.facets";
    let sub = PyModule::new(py, name)?;
    let ns = PyDict::new(py);
    ns.set_item("__module__", name)?;
    ns.set_item(
        "__init__",
        py.import("functools")?
            .call_method1("partialmethod", (wrap_pyfunction!(init_index, py)?,))?,
    )?;
    ns.set_item(
        "prepare",
        py.import("functools")?
            .call_method1("partialmethod", (wrap_pyfunction!(prepare, py)?,))?,
    )?;
    let bi = py.import("builtins")?;
    sub.add(
        "Index",
        bi.call_method1("type", ("Index", (bi.getattr("dict")?,), ns))?,
    )?;
    sub.add(
        "_definitions",
        py.import("weakref")?.call_method0("WeakKeyDictionary")?,
    )?;
    for f in [
        wrap_pyfunction!(build, &sub)?,
        wrap_pyfunction!(apply_rule, &sub)?,
    ] {
        sub.add_function(f)?;
    }
    filters::register(&sub)?;
    py.import("sys")?.getattr("modules")?.set_item(name, &sub)?;
    m.add("facets", sub)
}
