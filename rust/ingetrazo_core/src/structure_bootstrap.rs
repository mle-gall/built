// SPDX-License-Identifier: GPL-3.0-or-later
//! Propose the project tree from placed records, preserving first-seen
//! tie breaks, linked-source mounts and the order of conflict diagnostics.
use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PySet},
};
use std::collections::{HashMap, HashSet};
type Obj<'py> = Bound<'py, PyAny>;
macro_rules! record { ($py:expr; $($key:literal=>$value:expr),* $(,)?)=>{{
    let d=PyDict::new($py);$(d.set_item($key,$value)?;)*d
}} }
fn rows<'py>(v: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    v.try_iter()?.collect()
}
fn get<'py>(d: &Obj<'py>, k: &str) -> PyResult<Obj<'py>> {
    d.call_method1("get", (k,))
}
fn conflict<'py>(
    log: &Bound<'py, PyList>,
    node: &str,
    reason: &str,
) -> PyResult<Bound<'py, PyDict>> {
    let r = record!(log.py();"node"=>node,"reason"=>reason);
    log.append(&r)?;
    Ok(r)
}
fn infer<'py>(
    py: Python<'py>,
    groups: &[Obj<'py>],
    key: &str,
    log: &Bound<'py, PyList>,
) -> PyResult<Bound<'py, PyDict>> {
    let mut records = Vec::new();
    let counts = PyDict::new(py);
    let mut tagged = 0;
    for g in groups {
        let r = g.getattr("ifc")?;
        let r = if r.is_truthy()? {
            r
        } else {
            PyDict::new(py).into_any()
        };
        let class = get(&r, "class")?;
        if class.is_truthy()? {
            let n = counts
                .get_item(&class)?
                .map(|v| v.extract::<usize>())
                .transpose()?
                .unwrap_or(0);
            counts.set_item(class, n + 1)?;
            tagged += 1;
        }
        records.push(r);
    }
    let mut best = py.None().into_bound(py);
    let mut high = 0;
    for (k, n) in &counts {
        let n = n.extract::<usize>()?;
        if n > high {
            high = n;
            best = k;
        }
    }
    let recipe = record!(py;"class"=>best);
    if tagged == 0 {
        return Ok(recipe);
    }
    if 2 * tagged < groups.len() {
        let mut placements = Vec::new();
        for (g, r) in groups.iter().zip(&records) {
            let c = get(r, "class")?;
            if c.is_truthy()? {
                placements.push(record!(py;"uid"=>g.getattr("uid")?,"class"=>c));
            }
        }
        conflict(log, key, "recorded under null class")?.set_item("placements", placements)?;
        return Ok(record!(py;"class"=>py.None()));
    }
    if counts.len() > 1 {
        recipe.set_item("classes_allowed", counts.keys())?;
        conflict(log, key, "mixed classes")?.set_item("counts", counts)?;
    }
    if tagged < groups.len() {
        conflict(log, key, "would newly classify")?.set_item("count", groups.len() - tagged)?;
    }
    for field in ["predefined_type", "object_type", "type"] {
        let v = get(&records[0], field)?;
        let mut agree = true;
        for r in &records[1..] {
            if !get(r, field)?.eq(&v)? {
                agree = false;
                break;
            }
        }
        if agree {
            if !v.is_none() {
                recipe.set_item(field, v)?;
            }
        } else {
            conflict(log, key, &format!("varying {field}"))?;
        }
    }
    let deepcopy = py.import("copy")?.getattr("deepcopy")?;
    for field in ["systems", "classifications"] {
        let identity = |v: Obj<'py>| -> PyResult<Obj<'py>> {
            if field == "systems" {
                Ok(v)
            } else {
                Ok((v.get_item("system")?, v.get_item("code")?)
                    .into_pyobject(py)?
                    .into_any())
            }
        };
        let mut sets = Vec::new();
        let mut first = Vec::new();
        for (i, r) in records.iter().enumerate() {
            let values = r.call_method1("get", (field, PyList::empty(py)))?;
            let ids = PySet::empty(py)?;
            for v in values.try_iter()? {
                let v = v?;
                let id = identity(v.clone())?;
                ids.add(&id)?;
                if i == 0 {
                    first.push((v, id));
                }
            }
            sets.push(ids);
        }
        let common = sets[0].call_method0("copy")?;
        for ids in &sets[1..] {
            common.call_method1("intersection_update", (ids,))?;
        }
        let mut values = Vec::new();
        for (v, id) in first {
            if common.contains(id)? {
                values.push(deepcopy.call1((v,))?);
            }
        }
        if !values.is_empty() {
            recipe.set_item(field, values)?;
        }
        for ids in sets {
            if !ids.as_any().eq(&common)? {
                conflict(log, key, &format!("varying {field}"))?;
                break;
            }
        }
    }
    if get(recipe.as_any(), "predefined_type")?.eq("USERDEFINED")?
        && !get(recipe.as_any(), "object_type")?.is_truthy()?
    {
        recipe.del_item("predefined_type")?;
    }
    Ok(recipe)
}
struct Tree<'py> {
    nodes: Bound<'py, PyDict>,
    next: HashMap<Option<String>, usize>,
}
impl<'py> Tree<'py> {
    fn add(
        &mut self,
        key: &str,
        name: &str,
        parent: Option<&String>,
        extra: &Bound<'py, PyDict>,
    ) -> PyResult<()> {
        let order = self.next.entry(parent.cloned()).or_default();
        let node = record!(self.nodes.py();"name"=>name,"parent"=>parent,"order"=>*order);
        node.update(extra.as_mapping())?;
        self.nodes.set_item(key, node)?;
        *order += 1;
        Ok(())
    }
}
#[pyfunction]
pub(crate) fn structure_proposal<'py>(
    scene: &Obj<'py>,
    registry: &Obj<'py>,
) -> PyResult<Bound<'py, PyDict>> {
    let py = scene.py();
    let api = py.import("core.structure")?;
    let sep = crate::model::links::SEP.to_string();
    let default = api.getattr("DEFAULT_LAYER")?;
    let mut tree = Tree {
        nodes: PyDict::new(py),
        next: HashMap::new(),
    };
    let conflicts = PyList::empty(py);
    let filing = PyDict::new(py);
    let tags = PyDict::new(py);
    let sources = PyDict::new(py);
    for item in api.call_method1("_placements", (scene,))?.try_iter()? {
        let item = item?;
        let g = item.get_item(0)?;
        let src = item.get_item(1)?;
        let tag = g.getattr("layer")?;
        if src.is_truthy()? {
            sources.set_item(crate::model::links::stem(&src)?, &src)?;
        }
        if tag.is_truthy()? && !tag.eq(&default)? {
            let bucket = tags.call_method1("setdefault", (tag, PyList::empty(py)))?;
            bucket.call_method1("append", ((g, src),))?;
        } else if item.get_item(2)?.is_truthy()? && !src.is_truthy()? {
            conflicts.append(record!(py;"uid"=>g.getattr("uid")?,"name"=>g.getattr("name")?,"reason"=>"untagged placement"))?;
        }
    }
    let mut folders = scene.getattr("layer_folders")?.extract::<Vec<String>>()?;
    let layers = rows(&scene.getattr("layers")?)?;
    for layer in &layers {
        let f = layer.getattr("folder")?;
        if f.is_truthy()? {
            folders.push(f.extract()?);
        }
    }
    let mut seen = HashSet::new();
    folders.retain(|f| seen.insert(f.clone()));
    let mut keys = HashMap::new();
    for f in &folders {
        let k = if sources.contains(f)? {
            format!("link:{f}")
        } else {
            let uid: String = scene
                .getattr("layer_folder_uids")?
                .call_method1("get", (f, f))?
                .extract()?;
            format!("folder:{uid}")
        };
        keys.insert(f.clone(), k);
    }
    let source_trees = PyDict::new(py);
    let registered = crate::model::links::registry_view(scene)?;
    for pair in registered.call_method0("items")?.try_iter()? {
        let pair = pair?;
        let structure = get(&pair.get_item(1)?, "structure")?;
        if structure.is_truthy()? {
            source_trees.set_item(pair.get_item(0)?, structure)?;
        }
    }
    let mut folder_sources: HashMap<String, String> = HashMap::new();
    for f in &folders {
        let mount = sources.contains(f)?;
        let src = sources
            .get_item(f)?
            .or(sources.get_item(f.split(&sep).next().unwrap_or(f))?)
            .map(|s| s.extract::<String>())
            .transpose()?;
        let extra = PyDict::new(py);
        if let Some(src) = &src {
            folder_sources.insert(f.clone(), src.clone());
            if !mount && source_trees.contains(src)? {
                continue;
            }
            extra.set_item("source", src)?;
            extra.set_item("mount", mount)?;
        }
        let parent = scene
            .getattr("folder_parents")?
            .call_method1("get", (f,))?
            .extract::<Option<String>>()?;
        let label: String = crate::model::module(py, "ingetrazo_core")?.getattr("tag_edits")?
            .call_method1("folder_label", (scene, f))?
            .extract()?;
        let label = if src.is_some() {
            label.split_once(&sep).map_or(label.as_str(), |(_, s)| s)
        } else {
            &label
        };
        tree.add(
            &keys[f],
            label,
            parent.as_ref().and_then(|p| keys.get(p)),
            &extra,
        )?;
    }
    let mut top = HashSet::<String>::new();
    for r in py
        .import("core.link_structure")?
        .call_method1("walk", (scene,))?
        .try_iter()?
    {
        let r = r?;
        let chain = r.get_item(1)?;
        let inner = r.get_item(2)?;
        if !inner.eq(&chain)? && !chain.is_truthy()? {
            top.insert(
                crate::model::links::stem(&inner.get_item(-1)?.get_item(1)?)?,
            );
        }
    }
    for (stem, src) in &sources {
        let stem: String = stem.extract()?;
        let key = format!("link:{stem}");
        if !tree.nodes.contains(&key)? {
            if !top.contains(&stem) {
                continue;
            }
            tree.add(&key, &stem, None, &record!(py;"source"=>&src,"mount"=>true))?;
        }
        if let Some(nodes) = source_trees.get_item(&src)? {
            for node in nodes.call_method0("items")?.try_iter()? {
                let node = node?;
                let sk: String = node.get_item(0)?.extract()?;
                let sn = node.get_item(1)?;
                let parent = sn
                    .get_item("parent")?
                    .extract::<Option<String>>()?
                    .map_or_else(|| key.clone(), |p| format!("{key}/{p}"));
                tree.nodes.set_item(format!("{key}/{sk}"),record!(py;"name"=>sn.get_item("name")?,"parent"=>parent,
               "order"=>sn.get_item("order")?,"source"=>&src,"source_node"=>sk,"source_recipe"=>api.call_method1("deepcopy",(sn.get_item("recipe")?,))?))?;
            }
        }
    }
    for layer in layers {
        let tag = layer.getattr("key")?;
        if tag.eq(&default)? {
            continue;
        }
        let folder = layer.getattr("folder")?.extract::<Option<String>>()?;
        let src = folder.as_ref().and_then(|f| folder_sources.get(f));
        if source_trees.contains(src)? {
            continue;
        }
        let key = format!("layer:{}", layer.getattr("uid")?.extract::<String>()?);
        let pairs = tags.call_method1("pop", (&tag, PyList::empty(py)))?;
        let mut groups = Vec::new();
        for r in pairs.try_iter()? {
            let r = r?;
            let g = r.get_item(0)?;
            if r.get_item(1)?.eq(src)? && !crate::model::links::is_linked(&g)? {
                groups.push(g);
            }
        }
        let recipe = infer(py, &groups, &key, &conflicts)?;
        recipe.set_item("tag", &tag)?;
        let name: String = layer.getattr("name")?.extract()?;
        let name = if src.is_some() {
            name.split_once(&sep).map_or(name.as_str(), |(_, s)| s)
        } else {
            &name
        };
        if name
            .into_pyobject(py)?
            .call_method0("casefold")?
            .call_method1("startswith", ("_to review",))?
            .is_truthy()?
        {
            let c = conflict(&conflicts, &key, "review layer")?;
            c.set_item("count", groups.len())?;
            let placements = groups
                .iter()
                .map(|g| -> PyResult<_> {
                    Ok(record!(py;"uid"=>g.getattr("uid")?,"name"=>g.getattr("name")?))
                })
                .collect::<PyResult<Vec<_>>>()?;
            c.set_item("placements", placements)?;
        }
        let extra = if let Some(src) = src {
            record!(py;"source"=>src,"source_tag"=>tag,"source_recipe"=>recipe)
        } else {
            record!(py;"recipe"=>recipe)
        };
        tree.add(
            &key,
            name,
            folder.as_ref().and_then(|f| keys.get(f)),
            &extra,
        )?;
        if src.is_none() && !groups.is_empty() {
            filing.set_item(
                key,
                groups
                    .iter()
                    .map(|g| g.getattr("uid"))
                    .collect::<PyResult<Vec<_>>>()?,
            )?;
        }
    }
    for (tag, pairs) in tags {
        for r in pairs.try_iter()? {
            let r = r?;
            if !r.get_item(1)?.is_truthy()? {
                conflicts.append(record!(py;"uid"=>r.get_item(0)?.getattr("uid")?,"reason"=>"missing layer","tag"=>&tag))?;
            }
        }
    }
    let proposal = record!(py;"nodes"=>tree.nodes);
    let checked = py.get_type::<PyDict>().call1((registry,))?;
    checked.set_item("structure", &proposal)?;
    for msg in api
        .getattr("bim_registry")?
        .call_method1("_structure_errors", (checked,))?
        .try_iter()?
    {
        conflicts.append(record!(py;"reason"=>"registry validation","message"=>msg?))?;
    }
    Ok(record!(py;"structure"=>proposal,"filing"=>filing,"conflicts"=>conflicts))
}
