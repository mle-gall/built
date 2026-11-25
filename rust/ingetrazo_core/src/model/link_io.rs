// SPDX-License-Identifier: GPL-3.0-or-later
//! Linked files read from disk (`core/links.py`): a source loaded into a
//! definition renamed into the host, a link made from it, a file read
//! again into every placement of it, the links of a document just opened
//! resolved, the nested-link policy switched, the host's overrides
//! cleared, the relative paths written for a new host location.
//!
//! Each step is the Python's, call for call, over the same Python objects
//! (the readers, `Scene`, `Group`, `LinkMeshes`), so the links goldens
//! (`tests/test_links_golden.py`) hold. A source being read is on a
//! per-thread stack, the cycle guard. `core.links.LinkError` is what a
//! source that cannot be linked raises.

use std::cell::RefCell;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PySet, PyString, PyTuple};

use super::links::{self, LINK_KEY};
use super::module;

type Obj<'py> = Bound<'py, PyAny>;

thread_local! {
    /// The sources being read on this thread, normalised (cycle guard).
    static LOADING: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn none(py: Python<'_>) -> Obj<'_> {
    py.None().into_bound(py)
}

fn get<'py>(d: &Obj<'py>, key: &str) -> PyResult<Obj<'py>> {
    d.call_method1(pyo3::intern!(d.py(), "get"), (key,))
}

fn path_of<'py>(py: Python<'py>, p: &Obj<'py>) -> PyResult<Obj<'py>> {
    module(py, "pathlib")?.call_method1("Path", (p,))
}

fn name_of(py: Python<'_>, p: &Obj<'_>) -> PyResult<String> {
    path_of(py, p)?.getattr("name")?.extract()
}

fn link_error(py: Python<'_>, msg: String) -> PyResult<PyErr> {
    Ok(PyErr::from_value(module(py, "core.links")?.getattr("LinkError")?.call1((msg,))?))
}

fn is_link_error(py: Python<'_>, e: &PyErr) -> PyResult<bool> {
    e.value(py).is_instance(&module(py, "core.links")?.getattr("LinkError")?)
}

/// Where the source is: next to the host as last saved (`rel`), then the
/// absolute path. None when neither exists.
#[pyfunction]
#[pyo3(name = "link_locate", signature = (link, host_path=None))]
pub fn locate<'py>(link: &Obj<'py>, host_path: Option<&Obj<'py>>) -> PyResult<Option<Obj<'py>>> {
    let py = link.py();
    let rel = get(link, "rel")?;
    if let Some(host) = host_path.filter(|h| !h.is_none()) {
        if rel.is_truthy()? {
            let cand = path_of(py, host)?.getattr("parent")?.call_method1("__truediv__", (&rel,))?;
            if cand.call_method0("is_file")?.is_truthy()? {
                return Ok(Some(cand.call_method0("resolve")?));
            }
        }
    }
    let p = path_of(py, &link.get_item("path")?)?;
    Ok(p.call_method0("is_file")?.is_truthy()?.then_some(p))
}

/// `{"path": absolute, "rel": relative to the host's folder}` (no `rel`
/// across drives).
#[pyfunction]
#[pyo3(name = "link_record", signature = (path, host_path=None))]
pub fn link_record<'py>(path: &Obj<'py>, host_path: Option<&Obj<'py>>) -> PyResult<Bound<'py, PyDict>> {
    let py = path.py();
    let rec = PyDict::new(py);
    let resolved = path_of(py, path)?.call_method0("resolve")?;
    rec.set_item("path", resolved.str()?)?;
    if let Some(host) = host_path.filter(|h| !h.is_none()) {
        let parent = path_of(py, host)?.call_method0("resolve")?.getattr("parent")?;
        match module(py, "os")?.getattr("path")?.call_method1("relpath", (&resolved, parent)) {
            Ok(rel) => rec.set_item("rel", rel)?,
            Err(e) if e.is_instance_of::<pyo3::exceptions::PyValueError>(py) => {} // another drive
            Err(e) => return Err(e),
        }
    }
    Ok(rec)
}

/// The groups of a subset of `src`: the placements `select` names (uids
/// or occurrence paths, or a Navigator filter), each detached with its
/// subtree and placed where it was in the source.
fn subset<'py>(py: Python<'py>, src: &Obj<'py>, path: &Obj<'py>, select: &Obj<'py>, groups: Vec<Obj<'py>>)
               -> PyResult<(Vec<Obj<'py>>, Obj<'py>)> {
    let m4 = module(py, "ingetrazo_core")?.getattr("Matrix4")?;
    let refs = get(select, "uids")?;
    let wanted = PySet::empty(py)?;
    let targets = PySet::empty(py)?;
    if refs.is_truthy()? {
        for r in refs.try_iter()? {
            let r = r?;
            if r.is_instance_of::<PyString>() {
                wanted.add(&r)?;
            } else if r.is_instance_of::<PyList>() {
                targets.add(src.call_method1("resolve_occurrence", (&r,))?)?;
            }
        }
    }
    let requested = targets.call_method0("copy")?;
    if refs.is_none() {
        let facets = module(py, "ingetrazo_core.facets")?;
        let index = facets.call_method1("build", (src,))?;
        let found = index.getattr("groups")?;
        for k in facets.call_method1("evaluate", (&index, select.get_item("filter")?))?.try_iter()? {
            targets.add(found.get_item(k?)?)?;
        }
    }
    let mut selected: Vec<(Obj<'py>, Obj<'py>)> = Vec::new();
    fn visit<'py>(items: Vec<Obj<'py>>, parent: &Obj<'py>, m4: &Obj<'py>, wanted: &Bound<'py, PySet>,
                  targets: &Bound<'py, PySet>, out: &mut Vec<(Obj<'py>, Obj<'py>)>) -> PyResult<()> {
        for group in items {
            let xf = group.getattr("xform")?;
            let own = if xf.is_truthy()? { xf } else { m4.call0()? };
            let world = parent.mul(own)?;
            if wanted.contains(group.getattr("uid")?)? || targets.contains(&group)? {
                out.push((group, world));
            } else {
                let kids = group.getattr("children")?;
                let kids: Vec<Obj<'py>> = if kids.is_truthy()? { kids.try_iter()?.collect::<PyResult<_>>()? } else { Vec::new() };
                visit(kids, &world, m4, wanted, targets, out)?;
            }
        }
        Ok(())
    }
    visit(groups, &m4.call0()?, &m4, &wanted, &targets, &mut selected)?;
    fn detached<'py>(group: &Obj<'py>) -> PyResult<Obj<'py>> {
        let copy = links::clone_shell_of(group)?;
        let mut kids = Vec::new();
        for c in group.getattr("children")?.try_iter()? {
            kids.push(detached(&c?)?);
        }
        copy.setattr("children", PyList::new(group.py(), kids)?)?;
        Ok(copy)
    }
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (g, _) in &selected {
        *counts.entry(g.getattr("uid")?.extract()?).or_default() += 1;
    }
    let repeated = if requested.is_truthy()? { src.call_method0("repeated_uids")? } else { PySet::empty(py)?.into_any() };
    let row_id = module(py, "core.link_structure")?.getattr("row_id")?;
    let mut out = Vec::new();
    for (group, world) in selected {
        let part = detached(&group)?;
        let uid: String = group.getattr("uid")?.extract()?;
        if counts[&uid] > 1 || (requested.contains(&group)? && repeated.contains(&uid)?) {
            // Two equal leaf uids can become siblings in a flat subset.
            part.setattr("uid", row_id.call1((src.call_method1("occurrence_path", (&group,))?,))?)?;
        }
        part.setattr("xform", world)?;
        out.push(part);
    }
    let mesh = module(py, "ingetrazo_core.mesh")?.getattr("Mesh")?.call0()?;
    let stamp = PyTuple::new(py, [PyTuple::new(py, [path.str()?.into_any(), PyString::new(py, "subset-root").into_any()])?.into_any(), mesh.getattr("revision")?])?;
    mesh.getattr("__dict__")?.set_item("_link_geometry", stamp)?;
    Ok((out, mesh))
}

/// `(definition, source tags, source materials)` for the .igz at `path`:
/// the whole document as one component definition, its own links resolved
/// first. Raises `LinkError`.
#[pyfunction]
#[pyo3(name = "link_load_source", signature = (path, select=None, *, include_nested=true, link_meshes=None, progress=None))]
pub fn load_source<'py>(path: &Obj<'py>, select: Option<&Obj<'py>>, include_nested: bool, link_meshes: Option<&Obj<'py>>,
                        progress: Option<&Obj<'py>>) -> PyResult<Bound<'py, PyTuple>> {
    let py = path.py();
    let key = links::norm(path)?;
    let chain: Option<Vec<String>> = LOADING.with(|s| {
        let s = s.borrow();
        s.contains(&key).then(|| s.clone())
    });
    if let Some(chain) = chain {
        let names = chain.iter().map(|p| name_of(py, &PyString::new(py, p))).collect::<PyResult<Vec<_>>>()?;
        return Err(link_error(py, format!("{} links itself (through {})", name_of(py, path)?, names.join(" → ")))?);
    }
    if !path_of(py, path)?.call_method0("is_file")?.is_truthy()? {
        return Err(link_error(py, format!("{} was not found", path.str()?))?);
    }
    LOADING.with(|s| s.borrow_mut().push(key.clone()));
    let read = (|| -> PyResult<(Obj<'py>, Obj<'py>, Obj<'py>)> {
        let src = module(py, "core.scene")?.getattr("Scene")?.call0()?;
        let kw = PyDict::new(py);
        kw.set_item("progress", progress.map_or_else(|| none(py), |p| p.clone()))?;
        kw.set_item("include_links", include_nested)?;
        kw.set_item("link_meshes", link_meshes.map_or_else(|| none(py), |p| p.clone()))?;
        module(py, "formats.igz")?.getattr("load_into")?.call((&src, path_of(py, path)?), Some(&kw))?;
        // A cycle further down is this link's failure too; a source of the
        // source that is merely missing is not (its copies show empty).
        if let Some(errors) = src.getattr_opt("link_errors")?.filter(|e| e.is_truthy().unwrap_or(false)) {
            for err in errors.call_method0("values")?.try_iter()? {
                let err = err?;
                if err.contains("links itself")? {
                    return Err(link_error(py, err.str()?.to_string())?);
                }
            }
        }
        // The source's tag table outlives the component (which consumes
        // the scene's meshes, not its tags).
        let dict_of = |name: &str| -> PyResult<Bound<'py, PyDict>> {
            let out = PyDict::new(py);
            if let Some(v) = src.getattr_opt(name)?.filter(|v| v.is_truthy().unwrap_or(false)) {
                out.call_method1("update", (v,))?;
            }
            Ok(out)
        };
        let tags = PyDict::new(py);
        let layers = PyList::empty(py);
        for ly in src.getattr("layers")?.try_iter()? {
            layers.append(ly?.call_method0("to_dict")?)?;
        }
        tags.set_item("layers", layers)?;
        let folders = PyList::empty(py);
        if let Some(f) = src.getattr_opt("layer_folders")?.filter(|f| f.is_truthy().unwrap_or(false)) {
            for x in f.try_iter()? {
                folders.append(x?)?;
            }
        }
        tags.set_item("folders", folders)?;
        tags.set_item("folder_visibility", dict_of("folder_visibility")?)?;
        tags.set_item("folder_names", dict_of("layer_folder_names")?)?;
        tags.set_item("parents", dict_of("folder_parents")?)?;
        let vocabulary = module(py, "core.bim_registry")?.call_method1("get", (&src,))?;
        let nodes = vocabulary.call_method1("get", ("structure", PyDict::new(py)))?.call_method1("get", ("nodes", PyDict::new(py)))?;
        if nodes.is_truthy()? {
            let resolve = module(py, "ingetrazo_core.structure")?.getattr("resolve")?;
            let structure = PyDict::new(py);
            for item in nodes.call_method0("items")?.try_iter()? {
                let (k, node): (Obj<'py>, Obj<'py>) = item?.extract()?;
                let entry = PyDict::new(py);
                entry.call_method1("update", (&node,))?;
                entry.set_item("recipe", resolve.call1((&vocabulary, &k))?)?;
                structure.set_item(k, entry)?;
            }
            tags.set_item("structure", structure)?;
        }
        let materials = dict_of("materials")?;
        let select = select.filter(|s| !s.is_none());
        links::validate_select(&select.map_or_else(|| none(py), |s| s.clone()))?;
        let figure = module(py, "ingetrazo_core.document_insert")?.getattr("is_scale_figure")?;
        let mut groups = Vec::new();
        for g in src.getattr("groups")?.try_iter()? {
            let g = g?;
            if !figure.call1((&g,))?.is_truthy()? {
                groups.push(g);
            }
        }
        let (groups, mesh) = match select {
            Some(sel) => subset(py, &src, path, sel, groups)?,
            None => (groups, src.getattr("mesh")?),
        };
        let kw = PyDict::new(py);
        kw.set_item("name", path_of(py, path)?.getattr("stem")?)?;
        let comp = module(py, "core.group")?.getattr("Group")?.call((mesh,), Some(&kw))?;
        comp.call_method1("adopt", (PyList::new(py, groups)?,))?;
        Ok((comp, tags.into_any(), materials.into_any()))
    })();
    LOADING.with(|s| {
        let mut s = s.borrow_mut();
        if let Some(i) = s.iter().rposition(|k| *k == key) {
            s.remove(i);
        }
    });
    let (comp, tags, materials) = match read {
        Ok(r) => r,
        Err(e) if is_link_error(py, &e)? => return Err(e),
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyException>(py) => {
            // Any reader failure.
            let wrapped = link_error(py, format!("{}: {}", name_of(py, path)?, e.value(py).str()?))?;
            wrapped.set_cause(py, Some(e));
            return Err(wrapped);
        }
        Err(e) => return Err(e),
    };
    if comp.getattr("xform")?.is_none() {
        comp.setattr("xform", module(py, "ingetrazo_core")?.getattr("Matrix4")?.call0()?)?;
    }
    comp.setattr("component", true)?;
    PyTuple::new(py, [comp, tags, materials])
}

/// The source at `path` read into a definition renamed into the host: its
/// materials registered in `registry` (renamed where a name is the host's
/// already), its tags and paint on the host's names. `(definition, tags)`.
#[pyfunction]
#[pyo3(name = "link_read_definition", signature = (registry, path, select, include_nested, previous=None, *, progress=None))]
pub fn read_definition<'py>(registry: &Obj<'py>, path: &Obj<'py>, select: &Obj<'py>, include_nested: &Obj<'py>,
                            previous: Option<&Obj<'py>>, progress: Option<&Obj<'py>>) -> PyResult<Bound<'py, PyTuple>> {
    let py = path.py();
    let filtered = !select.is_none() && select.contains("filter")?;
    let kw = PyDict::new(py);
    kw.set_item("filtered", filtered)?;
    let reader = module(py, "ingetrazo_core.link_meshes")?.getattr("LinkMeshes")?
        .call((path, registry, previous.map_or_else(|| none(py), |p| p.clone())), Some(&kw))?;
    let read = load_source(path, Some(select), include_nested.is_truthy()?, Some(&reader), progress)?;
    let (comp, tags, materials) = (read.get_item(0)?, read.get_item(1)?, read.get_item(2)?);
    let register = module(py, "ingetrazo_core.materials")?.getattr("register")?;
    let renamed = PyDict::new(py);
    let items: Vec<(Obj<'py>, Obj<'py>)> = materials.call_method0("items")?.try_iter()?.map(|kv| kv?.extract()).collect::<PyResult<_>>()?;
    for (old, mat) in items {
        let fin = register.call1((registry, mat))?;
        if !fin.eq(&old)? {
            renamed.set_item(old, fin)?;
        }
    }
    links::install_definition(&comp, path, &reader.getattr("reused")?, &renamed)?;
    PyTuple::new(py, [comp, tags])
}

/// Keep `comp` (a definition as read) as the layout the placements of
/// this configuration are projected from: the scene root holds one per
/// (path, subset, nested-link policy), and no placement owns it.
#[pyfunction]
#[pyo3(name = "link_hold")]
pub fn hold<'py>(scene: &Obj<'py>, path: &Obj<'py>, select: &Obj<'py>, include_nested: &Obj<'py>,
                 comp: &Obj<'py>, tags: &Obj<'py>) -> PyResult<()> {
    let py = scene.py();
    if let Some(root) = scene.getattr_opt("_root")?.filter(|r| !r.is_none()) {
        let key = links::definition_key(path, select, include_nested)?;
        root.call_method1("_set_link_definition", (key, PyTuple::new(py, [comp, tags])?))?;
    }
    Ok(())
}

fn mtime<'py>(path: &Obj<'py>) -> PyResult<Obj<'py>> {
    path.call_method0("stat")?.getattr("st_mtime_ns")
}

/// A copy of the .igz at `path` ready to place in `scene` (by the
/// placement tool, like an imported component), its tags brought in.
/// Raises `LinkError`.
#[pyfunction]
#[pyo3(name = "link_make", signature = (scene, path, host_path=None, select=None, *, include_nested=true))]
pub fn make_link<'py>(scene: &Obj<'py>, path: &Obj<'py>, host_path: Option<&Obj<'py>>, select: Option<&Obj<'py>>,
                      include_nested: bool) -> PyResult<Obj<'py>> {
    let py = scene.py();
    let path = path_of(py, path)?.call_method0("resolve")?;
    let host = host_path.filter(|h| !h.is_none());
    if let Some(h) = host {
        if links::norm(&path)? == links::norm(h)? {
            return Err(link_error(py, "a document cannot link itself".into())?);
        }
    }
    let select = select.map_or_else(|| none(py), |s| s.clone());
    let nested = pyo3::types::PyBool::new(py, include_nested).to_owned().into_any();
    let read = read_definition(&scene.getattr("materials")?, &path, &select, &nested, None, None)?;
    let (comp, tags) = (read.get_item(0)?, read.get_item(1)?);
    links::sync_tags(scene, &path, &tags)?;
    // The definition stays as read, held by the document's root; the
    // placement is projected from it.
    let kw = PyDict::new(py);
    kw.set_item("name", path.getattr("stem")?)?;
    let link = module(py, "core.group")?.getattr("Group")?.call((comp.getattr("mesh")?,), Some(&kw))?;
    link.setattr("xform", module(py, "ingetrazo_core")?.getattr("Matrix4")?.call0()?)?;
    link.setattr("component", true)?;
    let ext = PyDict::new(py);
    ext.set_item(LINK_KEY, link_record(&path, host)?)?;
    link.setattr("ext", ext)?;
    let rec = link.getattr("ext")?.get_item(LINK_KEY)?;
    if !include_nested {
        rec.set_item("include_nested", false)?;
    }
    if !select.is_none() {
        rec.set_item("select", module(py, "copy")?.call_method1("deepcopy", (&select,))?)?;
    }
    links::populate(&link, &comp, false, None)?;
    hold(scene, &path, &select, &nested, &comp, &tags)?;
    links::entry(scene, &path)?.set_item("loaded_mtime_ns", mtime(&path)?)?;
    Ok(link)
}

/// Refresh one placement from disk with the chosen nested-link policy.
#[pyfunction]
#[pyo3(name = "link_set_include_nested")]
pub fn set_include_nested<'py>(scene: &Obj<'py>, group: &Obj<'py>, enabled: &Obj<'py>) -> PyResult<()> {
    let py = scene.py();
    let rec = links::link_of(group)?.ok_or_else(|| pyo3::exceptions::PyTypeError::new_err("not a link"))?;
    let path = rec.get_item("path")?;
    let select = get(&rec, "select")?;
    let read = read_definition(&scene.getattr("materials")?, &path, &select, enabled, Some(group), None)?;
    let (comp, tags) = (read.get_item(0)?, read.get_item(1)?);
    links::sync_tags(scene, &path, &tags)?;
    links::populate(group, &comp, true, Some(&scene.getattr("selection")?))?;
    hold(scene, &path, &select, enabled, &comp, &tags)?;
    rec.set_item("include_nested", enabled)?;
    bump(scene)?;
    let _ = py;
    Ok(())
}

fn bump(scene: &Obj<'_>) -> PyResult<()> {
    let v = scene.getattr("version")?;
    scene.setattr("version", v.add(1)?)
}

/// Read `path` again and show it in every copy. Raises `LinkError` (the
/// copies keep what they showed).
#[pyfunction]
#[pyo3(name = "link_reload", signature = (scene, path, host_path=None, *, progress=None))]
pub fn reload<'py>(scene: &Obj<'py>, path: &Obj<'py>, host_path: Option<&Obj<'py>>, progress: Option<&Obj<'py>>) -> PyResult<()> {
    let py = scene.py();
    let _ = host_path;
    // Prepare every subset before changing any placement or host material.
    let path = path_of(py, path)?.call_method0("resolve")?;
    let materials = PyDict::new(py);
    materials.call_method1("update", (scene.getattr("materials")?,))?;
    let mut roots: Vec<(Obj<'py>, Obj<'py>)> = Vec::new();
    for found in links::linked_groups(scene, Some(&path))? {
        let mut group = found.clone();
        // A nested copy wears every enclosing source's tag namespace and
        // uid mapping. Its outer definition also owns the packed policy.
        while let Some(parent) = links::linked_ancestor(&group.getattr("owner")?)? {
            group = parent;
        }
        let source = path_of(py, &links::link_of(&group)?.expect("a link").get_item("path")?)?.call_method0("resolve")?;
        if (group.is(&found) || !links::is_packed(scene, &source)?) && !roots.iter().any(|(g, _)| g.is(&group)) {
            roots.push((group, source));
        }
    }
    let mut definitions: Vec<(String, Obj<'py>, Obj<'py>)> = Vec::new();
    let mut prepared: Vec<(Obj<'py>, Obj<'py>)> = Vec::new();
    let mut held: Vec<(Obj<'py>, Obj<'py>, Obj<'py>, Obj<'py>, Obj<'py>)> = Vec::new();
    let mut tags_by_path: Vec<(Obj<'py>, Obj<'py>)> = Vec::new();
    for (group, source) in &roots {
        let rec = links::link_of(group)?.expect("a link");
        let nested = rec.call_method1("get", ("include_nested", true))?;
        let select = get(&rec, "select")?;
        let config = format!("{}\0{}\0{}", source.str()?, select.repr()?, nested.repr()?);
        let (comp, tags) = match definitions.iter().find(|(c, _, _)| *c == config) {
            Some((_, comp, tags)) => (comp.clone(), tags.clone()),
            None => {
                let read = read_definition(&materials, source, &select, &nested, Some(group), progress)?;
                let (comp, tags) = (read.get_item(0)?, read.get_item(1)?);
                definitions.push((config, comp.clone(), tags.clone()));
                (comp, tags)
            }
        };
        prepared.push((group.clone(), comp.clone()));
        match tags_by_path.iter_mut().find(|(p, _)| p.eq(source).unwrap_or(false)) {
            Some(slot) => slot.1 = tags.clone(),
            None => tags_by_path.push((source.clone(), tags.clone())),
        }
        held.push((source.clone(), select, nested, comp, tags));
    }
    if prepared.is_empty() {
        return Ok(());
    }
    let before = scene.getattr("content_version")?;
    let mut swaps: Option<Vec<(Obj<'py>, usize)>> = Some(Vec::new());
    scene.getattr("materials")?.call_method1("update", (&materials,))?;
    let selection = scene.getattr("selection")?;
    for (group, comp) in &prepared {
        match links::populate(group, comp, true, Some(&selection))? {
            None => swaps = None,
            Some(changed) => {
                if let Some(s) = swaps.as_mut() {
                    s.extend(changed);
                }
            }
        }
    }
    // The definitions swap: each configuration read is the one held now.
    for (source, select, nested, comp, tags) in &held {
        hold(scene, source, select, nested, comp, tags)?;
    }
    for (source, tags) in &tags_by_path {
        if links::sync_tags(scene, source, tags)? {
            swaps = None;
        }
        links::entry(scene, source)?.set_item("loaded_mtime_ns", mtime(source)?)?;
    }
    bump(scene)?;
    // A view that has exactly the previous revision can retain every
    // matrix, paint and pick id. A missed revision or a tree or tag change
    // uses its diff.
    let swaps = match swaps {
        None => none(py),
        Some(s) => {
            let l = PyList::empty(py);
            for (g, id) in s {
                l.append((g, id))?;
            }
            l.into_any()
        }
    };
    scene.setattr("_link_mesh_changes", (before, scene.getattr("content_version")?, swaps))?;
    Ok(())
}

/// A progress callback for one source of several.
#[pyclass(module = "ingetrazo_core")]
struct SourceProgress {
    progress: Py<PyAny>,
    index: usize,
    count: usize,
    name: String,
}

#[pymethods]
impl SourceProgress {
    fn __call__(&self, py: Python<'_>, fraction: f64, phase: &Bound<'_, PyAny>) -> PyResult<()> {
        let at = (self.index as f64 + fraction) / self.count as f64;
        self.progress.call1(py, (at, format!("{} ({}/{}) · {}", self.name, self.index + 1, self.count, phase.str()?)))?;
        Ok(())
    }

    fn __traverse__(&self, visit: pyo3::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        visit.call(&self.progress)
    }
}

/// Fill the linked copies of a document just opened from their sources
/// (packed ones keep what the file holds). `{path: error}` for the
/// sources that could not be read: their copies stay empty placements,
/// kept, so a later save does not lose the link.
#[pyfunction]
#[pyo3(name = "link_resolve", signature = (scene, host_path=None, *, progress=None))]
pub fn resolve<'py>(scene: &Obj<'py>, host_path: Option<&Obj<'py>>, progress: Option<&Obj<'py>>) -> PyResult<Bound<'py, PyDict>> {
    let py = scene.py();
    let host = host_path.filter(|h| !h.is_none());
    if let Some(h) = host {
        let key = links::norm(h)?;
        if !LOADING.with(|s| s.borrow().contains(&key)) {
            // The document being opened is part of the chain: a source
            // that links back to it is a cycle.
            LOADING.with(|s| s.borrow_mut().push(key.clone()));
            let out = resolve(scene, host_path, progress);
            LOADING.with(|s| {
                let mut s = s.borrow_mut();
                if let Some(i) = s.iter().rposition(|k| *k == key) {
                    s.remove(i);
                }
            });
            return out;
        }
    }
    let errors = PyDict::new(py);
    for g in links::linked_groups(scene, None)? {
        // A packed copy keeps what the file holds, and is not read again:
        // its parts read-only all the same.
        links::adopt_parts(&g)?;
        let link = links::link_of(&g)?.expect("a link");
        if let Some(found) = locate(&link, host)? {
            if links::norm(&found)? != links::norm(&link.get_item("path")?)? {
                link.set_item("path", found.str()?)?; // the project moved: follow it
            }
        }
    }
    let mut paths = Vec::new();
    for p in links::sources(scene)? {
        let p = PyString::new(py, &p).into_any();
        if !links::is_packed(scene, &p)? {
            paths.push(p);
        }
    }
    let count = paths.len();
    let progress = progress.filter(|p| p.is_truthy().unwrap_or(false));
    for (index, path) in paths.iter().enumerate() {
        let report = match progress {
            Some(p) => {
                let r = Py::new(py, SourceProgress { progress: p.clone().unbind(), index, count, name: name_of(py, path)? })?;
                r.call1(py, (0.0, "Reading linked document…"))?;
                Some(r.into_any().into_bound(py))
            }
            None => None,
        };
        match reload(scene, path, host, report.as_ref()) {
            Ok(()) => {}
            Err(e) if is_link_error(py, &e)? => errors.set_item(path, e.value(py).str()?)?,
            Err(e) => return Err(e),
        }
    }
    scene.setattr("link_errors", &errors)?;
    Ok(errors)
}

/// Show `path`'s tags as the source has them again.
#[pyfunction]
#[pyo3(name = "link_clear_overrides", signature = (scene, path, host_path=None))]
pub fn clear_overrides<'py>(scene: &Obj<'py>, path: &Obj<'py>, host_path: Option<&Obj<'py>>) -> PyResult<()> {
    let py = scene.py();
    let entry = links::entry(scene, path)?;
    entry.set_item("overrides", PyDict::new(py))?;
    // Back to what the source said, so the reload does not read the old
    // override as a new one.
    let last = get(&entry, "source_visible")?;
    if last.is_truthy()? {
        for item in last.call_method0("items")?.try_iter()? {
            let (name, vis): (Obj<'py>, Obj<'py>) = item?.extract()?;
            let ly = scene.call_method1("layer", (name,))?;
            if !ly.is_none() {
                ly.setattr("visible", vis.is_truthy()?)?;
            }
        }
    }
    reload(scene, path, host_path, None)
}

/// Rewrite each link's `rel` for the host's new location (Save As).
#[pyfunction]
#[pyo3(name = "link_refresh_paths")]
pub fn refresh_paths<'py>(scene: &Obj<'py>, host_path: &Obj<'py>) -> PyResult<()> {
    for g in links::linked_groups(scene, None)? {
        let rec = links::link_of(&g)?.expect("a link");
        let fresh = link_record(&rec.get_item("path")?, Some(host_path))?;
        g.getattr("ext")?.get_item(LINK_KEY)?.call_method1("update", (fresh,))?;
    }
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(locate, m)?)?;
    m.add_function(wrap_pyfunction!(link_record, m)?)?;
    m.add_function(wrap_pyfunction!(load_source, m)?)?;
    m.add_function(wrap_pyfunction!(read_definition, m)?)?;
    m.add_function(wrap_pyfunction!(hold, m)?)?;
    m.add_function(wrap_pyfunction!(make_link, m)?)?;
    m.add_function(wrap_pyfunction!(set_include_nested, m)?)?;
    m.add_function(wrap_pyfunction!(reload, m)?)?;
    m.add_function(wrap_pyfunction!(resolve, m)?)?;
    m.add_function(wrap_pyfunction!(clear_overrides, m)?)?;
    m.add_function(wrap_pyfunction!(refresh_paths, m)?)?;
    m.add_class::<SourceProgress>()?;
    Ok(())
}
