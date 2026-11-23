// SPDX-License-Identifier: GPL-3.0-or-later
//! Structure membership and filing are planned against the live placement tree.
//! Registry lookup and IFC patch construction remain with the records adapter.
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList, PySet, PyString, PyTuple},
};
use std::collections::HashSet;
type Obj<'py> = Bound<'py, PyAny>;
fn empty<'py>(py: Python<'py>) -> Obj<'py> {
    PyDict::new(py).into_any()
}
fn get<'py>(map: &Obj<'py>, key: &str) -> PyResult<Obj<'py>> {
    map.call_method1("get", (key,))
}
fn none<'py>(py: Python<'py>) -> Obj<'py> {
    py.None().into_bound(py)
}
fn items<'py>(map: &Obj<'py>) -> PyResult<Vec<(Obj<'py>, Obj<'py>)>> {
    map.call_method0("items")?
        .try_iter()?
        .map(|p| {
            let p = p?;
            Ok((p.get_item(0)?, p.get_item(1)?))
        })
        .collect()
}
fn list<'py>(value: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    value.try_iter()?.collect()
}
fn text(value: &Obj<'_>) -> PyResult<String> {
    Ok(value.str()?.to_string())
}
fn node(group: &Obj<'_>) -> PyResult<Option<String>> {
    let ext = group.getattr("ext")?;
    if !ext.is_truthy()? {
        return Ok(None);
    }
    let member = get(&ext, "structure")?;
    if member.is_truthy()? {
        get(&member, "node")?.extract()
    } else {
        Ok(None)
    }
}

#[pyfunction]
pub(crate) fn structure_lineage(nodes: &Obj<'_>, key: &Obj<'_>) -> PyResult<Vec<String>> {
    if !key.is_instance_of::<PyString>() || !nodes.contains(key)? {
        return Err(PyValueError::new_err(format!(
            "Unknown structure node: {}",
            text(key)?
        )));
    }
    let mut path = Vec::new();
    let mut seen = HashSet::new();
    let mut current = Some(key.extract::<String>()?);
    while let Some(k) = current {
        if !seen.insert(k.clone()) || !nodes.contains(&k)? {
            return Err(PyValueError::new_err(format!(
                "Unknown or cyclic structure node: {k}"
            )));
        }
        current = nodes.get_item(&k)?.get_item("parent")?.extract()?;
        path.push(k);
    }
    path.reverse();
    Ok(path)
}

type Entry = (Py<PyAny>, Option<String>, bool);
#[pyclass(module = "ingetrazo_core")]
pub(crate) struct StructurePlacements {
    stack: Vec<Entry>,
    pending: Option<(Py<PyAny>, Option<String>)>,
    seen: HashSet<usize>,
    link: Py<PyAny>,
}
#[pymethods]
impl StructurePlacements {
    #[new]
    fn new(scene: &Obj<'_>) -> PyResult<Self> {
        let groups = list(&scene.getattr("groups")?)?;
        Ok(Self {
            stack: groups
                .into_iter()
                .rev()
                .map(|g| (g.unbind(), None, true))
                .collect(),
            pending: None,
            seen: HashSet::new(),
            link: scene
                .py()
                .import("core.links")?
                .getattr("link_of")?
                .unbind(),
        })
    }
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<Entry>> {
        if let Some((parent, source)) = self.pending.take() {
            let children = parent.bind(py).getattr("children")?;
            if children.is_truthy()? {
                self.stack.extend(
                    list(&children)?
                        .into_iter()
                        .rev()
                        .map(|g| (g.unbind(), source.clone(), false)),
                );
            }
        }
        while let Some((group, mut source, root)) = self.stack.pop() {
            if !self.seen.insert(group.as_ptr() as usize) {
                continue;
            }
            let link = self.link.bind(py).call1((group.bind(py),))?;
            if link.is_truthy()? {
                source = link.get_item("path")?.extract()?;
            }
            self.pending = Some((group.clone_ref(py), source.clone()));
            return Ok(Some((group, source, root)));
        }
        Ok(None)
    }
}

#[pyfunction]
pub(crate) fn structure_targets<'py>(scene: &Obj<'py>, uids: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    let py = scene.py();
    let sequence = |v: &Obj<'_>| v.is_instance_of::<PyTuple>() || v.is_instance_of::<PyList>();
    let invalid = || PyValueError::new_err("uids must be a nonempty list of strings or UID paths");
    if !sequence(uids) || uids.is_empty()? {
        return Err(invalid());
    }
    let paths = PyDict::new(py);
    let mut addresses = Vec::new();
    for uid in uids.try_iter()? {
        let mut uid = uid?;
        if sequence(&uid) {
            let keys = list(&uid)?;
            if keys.is_empty() || !keys.iter().all(|k| k.is_instance_of::<PyString>()) {
                return Err(invalid());
            }
            uid = PyTuple::new(py, keys)?.into_any();
            let target = scene.call_method1("resolve_occurrence", (&uid,))?;
            if target.is_none() {
                return Err(PyValueError::new_err(format!(
                    "Unknown occurrence path {}",
                    uid.repr()?
                )));
            }
            paths
                .call_method1("setdefault", (target, PyList::empty(py)))?
                .call_method1("append", (&uid,))?;
        } else if !uid.is_instance_of::<PyString>() {
            return Err(invalid());
        }
        addresses.push(uid);
    }
    let wanted = PySet::new(py, &addresses)?;
    let targets = PyDict::new(py);
    let mut walk = StructurePlacements::new(scene)?;
    while let Some((group, source, _)) = walk.__next__(py)? {
        let group = group.into_bound(py);
        let uid = group.getattr("uid")?;
        let keys = paths.get_item(&group)?;
        if wanted.contains(&uid)? || keys.is_some() {
            if let Some(src) = source.filter(|s| !s.is_empty()) {
                return Err(PyValueError::new_err(format!(
                    "Linked placement is read-only: {} ({src})",
                    text(&uid)?
                )));
            }
            if wanted.contains(&uid)? {
                targets.set_item(&uid, &group)?;
            }
            if let Some(keys) = keys {
                for key in keys.try_iter()? {
                    targets.set_item(key?, &group)?;
                }
            }
        }
    }
    let mut missing = Vec::new();
    for uid in wanted.iter() {
        if !targets.contains(&uid)? {
            missing.push(text(&uid)?);
        }
    }
    if !missing.is_empty() {
        missing.sort();
        return Err(PyValueError::new_err(format!(
            "Unknown uids: {}",
            missing.join(", ")
        )));
    }
    let seen = PySet::empty(py)?;
    let mut out = Vec::new();
    for uid in addresses {
        let group = targets.get_item(uid)?.unwrap();
        if !seen.contains(&group)? {
            seen.add(&group)?;
            out.push(group);
        }
    }
    Ok(out)
}

type Prior<'py> = (bool, bool, Obj<'py>);
#[pyfunction]
pub(crate) fn set_structure_nodes<'py>(
    targets: &Obj<'py>,
    key: &Obj<'py>,
) -> PyResult<Vec<Prior<'py>>> {
    let py = targets.py();
    let copy = py.import("copy")?.getattr("deepcopy")?;
    let mut prior = Vec::new();
    for group in targets.try_iter()? {
        let group = group?;
        let old = group.getattr("ext")?;
        let ext = if old.is_truthy()? {
            py.get_type::<PyDict>().call1((&old,))?
        } else {
            empty(py)
        };
        prior.push((
            old.is_none(),
            ext.contains("structure")?,
            copy.call1((get(&ext, "structure")?,))?,
        ));
        if key.is_none() {
            ext.call_method1("pop", ("structure", py.None()))?;
        } else {
            let member = PyDict::new(py);
            member.set_item("node", key)?;
            ext.set_item("structure", member)?;
        }
        group.setattr("ext", if ext.is_truthy()? { ext } else { none(py) })?;
    }
    Ok(prior)
}
#[pyfunction]
pub(crate) fn restore_structure_nodes(targets: &Obj<'_>, prior: Vec<Prior<'_>>) -> PyResult<()> {
    let py = targets.py();
    let copy = py.import("copy")?.getattr("deepcopy")?;
    for (group, (was_none, present, value)) in targets.try_iter()?.zip(prior) {
        let group = group?;
        let old = group.getattr("ext")?;
        let ext = if old.is_truthy()? {
            py.get_type::<PyDict>().call1((old,))?
        } else {
            empty(py)
        };
        if present {
            ext.set_item("structure", copy.call1((value,))?)?;
        } else {
            ext.call_method1("pop", ("structure", py.None()))?;
        }
        group.setattr(
            "ext",
            if was_none && ext.is_empty()? {
                none(py)
            } else {
                ext
            },
        )?;
    }
    Ok(())
}

#[pyfunction]
pub(crate) fn plan_filing<'py>(
    scene: &Obj<'py>,
    groups: Vec<Obj<'py>>,
    registry: &Obj<'py>,
    key: &Obj<'py>,
    changes: &Obj<'py>,
) -> PyResult<Vec<Obj<'py>>> {
    let py = scene.py();
    let api = py.import("core.structure")?;
    let recipe = api.call_method1("resolve", (registry, key))?;
    let nodes = api.call_method1("_nodes", (registry,))?;
    for k in structure_lineage(&nodes, key)? {
        if nodes.get_item(k)?.contains("source")? {
            return Err(PyValueError::new_err("Linked structure is read-only"));
        }
    }
    let tag = recipe.get_item("tag")?;
    let folders = recipe.get_item("folder")?;
    let existing = scene.getattr("layer_folders")?;
    let old_parents = scene.getattr("folder_parents")?;
    let parents = PyDict::new(py);
    let mut folder = none(py);
    let mut changed = false;
    for name in folders.try_iter()? {
        let name = name?;
        if parents.contains(&name)? {
            return Err(PyValueError::new_err(format!(
                "Folder names must identify a single path: {}",
                text(&PyString::new(py, "/").call_method1("join", (&folders,))?)?
            )));
        }
        parents.set_item(&name, &folder)?;
        changed |= !existing.contains(&name)?
            || !old_parents.call_method1("get", (&name,))?.eq(&folder)?;
        folder = name;
    }
    if tag.eq(api.getattr("DEFAULT_LAYER")?)? && folder.is_truthy()? {
        return Err(PyValueError::new_err(
            "The default layer cannot be moved into a folder",
        ));
    }
    let layer = scene.call_method1("layer", (&tag,))?;
    let move_layer = layer.is_truthy()? && !layer.getattr("folder")?.eq(&folder)?;
    changed |= !layer.is_truthy()? || move_layer;
    let mut commands = Vec::new();
    let mut moved = Vec::new();
    let mut filed = Vec::new();
    if !groups.is_empty() && changed {
        let new_uid = api.getattr("new_uid")?;
        let entry = PyDict::new(py);
        entry.set_item("name", &tag)?;
        entry.set_item("folder", &folder)?;
        entry.set_item("uid", new_uid.call0()?)?;
        let uids = PyDict::new(py);
        for f in folders.try_iter()? {
            let f = f?;
            if !existing.contains(&f)? {
                uids.set_item(f, new_uid.call0()?)?;
            }
        }
        let template = PyDict::new(py);
        template.set_item("layers", vec![entry])?;
        template.set_item("folders", folders)?;
        template.set_item("folder_parents", parents)?;
        template.set_item("folder_uids", uids)?;
        commands.push(api.call_method1("_EnsureStructureLayers", (template,))?);
        if move_layer {
            commands.push(api.call_method1("_MoveTag", (layer, folder))?);
        }
    }
    for group in groups {
        let fields = PyList::empty(py);
        if !api.call_method1("node_of", (&group,))?.eq(key)? {
            filed.push(group.clone());
            fields.append("node")?;
        }
        if !api.call_method1("layer_of", (&group,))?.eq(&tag)? {
            moved.push(group.clone());
            fields.append("tag")?;
        }
        if changed {
            fields.append("folder")?;
        }
        let patch = api.call_method1("_patch", (&group, &recipe, registry))?;
        if patch.is_truthy()? {
            let kwargs = PyDict::new(py);
            kwargs.set_item("normalize_values", false)?;
            commands.push(
                api.getattr("SetRecordCommand")?
                    .call((vec![group.clone()], &patch), Some(&kwargs))?,
            );
            fields.call_method1("extend", (patch,))?;
        }
        if !fields.is_empty() {
            changes.set_item(group.getattr("uid")?, fields)?;
        }
    }
    if !moved.is_empty() {
        commands.push(api.call_method1("AssignLayerCommand", (moved, tag))?);
    }
    if !filed.is_empty() {
        commands.push(api.call_method1("SetStructureNodeCommand", (filed, key))?);
    }
    Ok(commands)
}

#[pyfunction]
pub(crate) fn check_filing_paths(
    scene: &Obj<'_>,
    registry: &Obj<'_>,
    batches: &Obj<'_>,
) -> PyResult<()> {
    let py = scene.py();
    let api = py.import("core.structure")?;
    let nodes = api.call_method1("_nodes", (registry,))?;
    // Error order follows the interpreter's occupied-node set.
    let pending = PySet::empty(py)?;
    let occupied = PySet::empty(py)?;
    for (key, groups) in items(batches)? {
        if groups.is_truthy()? {
            occupied.add(key)?;
        }
        for g in groups.try_iter()? {
            pending.add(g?.getattr("uid")?)?;
        }
    }
    let mut walk = StructurePlacements::new(scene)?;
    while let Some((g, src, _)) = walk.__next__(py)? {
        if src.is_some_and(|s| !s.is_empty()) || pending.contains(g.bind(py).getattr("uid")?)? {
            continue;
        }
        if let Some(k) = node(g.bind(py))? {
            if nodes.contains(&k)? {
                occupied.add(k)?;
            }
        }
    }
    let tags = PyDict::new(py);
    let parents = PyDict::new(py);
    for key in occupied.iter() {
        let recipe = api.call_method1("resolve", (registry, key))?;
        let folders = recipe.get_item("folder")?;
        let tag = recipe.get_item("tag")?;
        if !tags
            .call_method1("setdefault", (&tag, &folders))?
            .eq(&folders)?
        {
            return Err(PyValueError::new_err(format!(
                "Conflicting folder paths for tag: {}",
                text(&tag)?
            )));
        }
        let mut parent = none(py);
        for name in folders.try_iter()? {
            let name = name?;
            if !parents
                .call_method1("setdefault", (&name, &parent))?
                .eq(&parent)?
            {
                return Err(PyValueError::new_err(format!(
                    "Conflicting parents for folder: {}",
                    text(&name)?
                )));
            }
            parent = name;
        }
    }
    Ok(())
}

#[pyfunction]
pub(crate) fn resync_filing<'py>(
    scene: &Obj<'py>,
    registry: &Obj<'py>,
    keys: &Obj<'py>,
) -> PyResult<(Obj<'py>, Bound<'py, PyDict>)> {
    let py = scene.py();
    let api = py.import("core.structure")?;
    let nodes = api.call_method1("_nodes", (registry,))?;
    let chosen = PySet::new(py, list(if keys.is_none() { &nodes } else { keys })?)?;
    if chosen
        .iter()
        .any(|k| matches!(nodes.contains(k), Ok(false)))
    {
        return Err(PyValueError::new_err("Unknown resync node"));
    }
    let batches = PyDict::new(py);
    for row in api
        .call_method1("_memberships", (scene, registry))?
        .try_iter()?
    {
        let row = row?;
        let src = row.get_item(1)?;
        let key = row.get_item(3)?;
        if !src.is_truthy()? && nodes.contains(&key)? {
            batches
                .call_method1("setdefault", (key, PyList::empty(py)))?
                .call_method1("append", (row.get_item(0)?,))?;
        }
    }
    check_filing_paths(scene, registry, batches.as_any())?;
    let report = PyDict::new(py);
    let mut commands = Vec::new();
    for (key, groups) in batches {
        if structure_lineage(&nodes, &key)?
            .iter()
            .any(|k| chosen.contains(k).unwrap_or(false))
        {
            commands.extend(plan_filing(
                scene,
                list(&groups)?,
                registry,
                &key,
                report.as_any(),
            )?);
        }
    }
    Ok((api.call_method1("CompoundCommand", (commands,))?, report))
}

#[pyfunction]
pub(crate) fn apply_structure_bootstrap<'py>(
    scene: &Obj<'py>,
    proposal: &Obj<'py>,
) -> PyResult<Obj<'py>> {
    let py = scene.py();
    let api = py.import("core.structure")?;
    let registry = api.getattr("bim_registry")?.call_method1("get", (scene,))?;
    registry.set_item(
        "structure",
        api.call_method1("deepcopy", (proposal.get_item("structure")?,))?,
    )?;
    let filing = proposal.get_item("filing")?;
    let mut uids = Vec::new();
    for (_, targets) in items(&filing)? {
        uids.extend(list(&targets)?);
    }
    let wanted = PySet::new(py, &uids)?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("refiled_uids", &uids)?;
    let mut commands = vec![api
        .getattr("SetRegistryCommand")?
        .call((&registry,), Some(&kwargs))?];
    if uids.len() != wanted.len() {
        return Err(PyValueError::new_err("A placement can only be filed once"));
    }
    let groups = PyDict::new(py);
    if !uids.is_empty() {
        for g in structure_targets(scene, PyList::new(py, uids)?.as_any())? {
            groups.set_item(g.getattr("uid")?, g)?;
        }
    }
    let nodes = api.call_method1("_nodes", (&registry,))?;
    let mut walk = StructurePlacements::new(scene)?;
    while let Some((g, src, _)) = walk.__next__(py)? {
        let g = g.bind(py);
        if src.is_some_and(|s| !s.is_empty()) || groups.contains(g.getattr("uid")?)? {
            continue;
        }
        if let Some(k) = node(g)? {
            if !k.is_empty() && !nodes.contains(&k)? {
                return Err(PyValueError::new_err(format!(
                    "Cannot delete occupied node: {k}"
                )));
            }
        }
    }
    let batches = PyDict::new(py);
    for (k, targets) in items(&filing)? {
        let batch = list(&targets)?
            .into_iter()
            .map(|uid| groups.as_any().get_item(uid))
            .collect::<PyResult<Vec<_>>>()?;
        batches.set_item(k, batch)?;
    }
    check_filing_paths(scene, &registry, batches.as_any())?;
    for (k, targets) in batches {
        commands.extend(plan_filing(
            scene,
            list(&targets)?,
            &registry,
            &k,
            &empty(py),
        )?);
    }
    api.call_method1("CompoundCommand", (commands,))
}
