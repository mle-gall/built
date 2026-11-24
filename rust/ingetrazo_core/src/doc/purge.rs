// SPDX-License-Identifier: GPL-3.0-or-later
//! The purge survey (moved from `core/purge.py`, docs/commands-port.md):
//! which tags and materials the document uses, and what carries a tag.
//!
//! A tag is a label an entity carries, never its owner, so emptying one
//! cannot delete it: Purge Unused is the explicit sweep. The walk descends
//! into nested placements and kept component definitions, because a name
//! used only inside one is in use. The Layers panel counts through the same
//! walk, step by step ([`LayerCountSteps`]).
use super::py::doc_walk;
use numpy::{PyArrayMethods, PyReadonlyArray1, PyReadonlyArray2};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PySet, PyTuple};
use std::collections::{BTreeMap, HashMap, HashSet};

type Obj<'py> = Bound<'py, PyAny>;

const DEFAULT_LAYER: &str = "Layer 0";

fn list_of<'py>(items: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    if items.is_none() {
        return Ok(Vec::new());
    }
    items.try_iter()?.collect()
}

fn attr_or_none<'py>(o: &Obj<'py>, name: &str) -> PyResult<Obj<'py>> {
    Ok(o.getattr_opt(name)?.unwrap_or_else(|| o.py().None().into_bound(o.py())))
}

/// `core.layers.layer_of`: the tag an entity carries (the default when unlabelled).
fn layer_of<'py>(e: &Obj<'py>) -> PyResult<Obj<'py>> {
    let py = e.py();
    let value = match e.getattr_opt("attrs")?.filter(|a| !a.is_none()) {
        Some(attrs) => attrs.call_method1("get", ("layer",))?,
        None => attr_or_none(e, "layer")?,
    };
    Ok(if value.is_truthy()? { value } else { pyo3::types::PyString::new(py, DEFAULT_LAYER).into_any() })
}

/// Distinct placements under `groups`, in document order, hidden ones
/// included: one native walk for document placements, else a stack walk.
fn walk<'py>(py: Python<'py>, groups: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    // Listed first: a generator must not be spent by a native walk that fails.
    let items = list_of(groups)?;
    if !items.is_empty() {
        if let Ok(found) = doc_walk(py, PyList::new(py, &items)?.as_any()) {
            return found.iter().map(Ok).collect();
        }
    }
    let mut stack = items;
    stack.reverse();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    while let Some(group) = stack.pop() {
        if !seen.insert(group.as_ptr() as usize) {
            continue;
        }
        let mut kids = list_of(&attr_or_none(&group, "children")?)?;
        kids.reverse();
        out.push(group);
        stack.extend(kids);
    }
    Ok(out)
}

#[pyfunction]
fn purge_iter_groups<'py>(py: Python<'py>, groups: &Obj<'py>) -> PyResult<Obj<'py>> {
    PyList::new(py, walk(py, groups)?)?.call_method0("__iter__")
}

/// The model's groups and the component definitions kept with no copy.
fn document_groups<'py>(py: Python<'py>, scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    let mut all = list_of(&attr_or_none(scene, "groups")?)?;
    all.extend(list_of(&attr_or_none(scene, "kept_components")?)?);
    Ok(PyList::new(py, all)?.into_any())
}

#[pyfunction]
fn purge_document_groups<'py>(py: Python<'py>, scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    document_groups(py, scene)
}

/// The loose mesh (not the open one: a sweep inside a group edit still
/// sees what was left outside) and every group's mesh, each once.
fn meshes<'py>(py: Python<'py>, scene: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    let mut found = vec![attr_or_none(scene, "loose_mesh")?];
    for g in walk(py, &document_groups(py, scene)?)? {
        found.push(g.getattr("mesh")?);
    }
    let mut seen = HashSet::new();
    Ok(found.into_iter().filter(|m| !m.is_none() && seen.insert(m.as_ptr() as usize)).collect())
}

#[pyfunction]
fn purge_iter_meshes<'py>(py: Python<'py>, scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    PyList::new(py, meshes(py, scene)?)?.call_method0("__iter__")
}

fn annotations<'py>(scene: &Obj<'py>) -> PyResult<Vec<Obj<'py>>> {
    let mut out = Vec::new();
    for coll in ["dimensions", "text_labels", "image_planes", "section_planes", "guides"] {
        out.extend(list_of(&attr_or_none(scene, coll)?)?);
    }
    Ok(out)
}

#[pyfunction]
fn purge_annotations<'py>(py: Python<'py>, scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    PyList::new(py, annotations(scene)?)?.call_method0("__iter__")
}

/// Everything that can carry a tag: faces and edges of every mesh, every
/// group (nested and kept), the annotations and the survey.
#[pyfunction]
fn purge_layer_carriers<'py>(py: Python<'py>, scene: &Obj<'py>) -> PyResult<Obj<'py>> {
    let mut out = Vec::new();
    for mesh in meshes(py, scene)? {
        out.extend(list_of(&mesh.getattr("faces")?)?);
        out.extend(list_of(&mesh.getattr("edges")?)?);
    }
    out.extend(walk(py, &document_groups(py, scene)?)?);
    out.extend(annotations(scene)?);
    let photo = attr_or_none(scene, "photo_mesh")?;
    if !photo.is_none() {
        out.push(photo);
    }
    PyList::new(py, out)?.call_method0("__iter__")
}

fn bump(tally: &Obj<'_>, name: &Obj<'_>, n: i64) -> PyResult<()> {
    let current: i64 = tally.get_item(name)?.extract()?;
    tally.set_item(name, current + n)
}

/// A lazy mesh's edges by tag, without building it: the same welded keys
/// `core.lazy_mesh.plan` makes, ring-only edges included; the last tag
/// assigned to a repeated edge wins.
fn count_edges(py: Python<'_>, src: &Obj<'_>, tally: &Obj<'_>) -> PyResult<()> {
    let welded = py.import("core.lazy_mesh")?.getattr("weld")?.call1((src.get_item("pos")?,))?;
    let n_vertices = welded.get_item(0)?.len()? as i64;
    let inverse_obj = welded.get_item(1)?;
    let inverse: PyReadonlyArray1<'_, i64> = inverse_obj.extract()?;
    let inverse = inverse.as_slice()?;
    let key = |a: i64, b: i64| a.min(b) * n_vertices + a.max(b);
    let np = py.import("numpy")?;
    let as_i64 = |o: Obj<'_>| -> PyResult<Vec<i64>> {
        np.call_method1("ascontiguousarray", (o, "int64"))?.call_method0("ravel")?.extract::<PyReadonlyArray1<'_, i64>>()?.as_slice().map(|s| s.to_vec()).map_err(Into::into)
    };
    let edges = as_i64(src.get_item("edges")?)?;
    let mut keep = Vec::new();
    let mut explicit = Vec::new();
    for (i, pair) in edges.chunks(2).enumerate() {
        let (a, b) = (inverse[pair[0] as usize], inverse[pair[1] as usize]);
        if a != b {
            keep.push(i);
            explicit.push(key(a, b));
        }
    }
    let mut all: HashSet<i64> = explicit.iter().copied().collect();
    let rings = as_i64(src.get_item("rings")?)?;
    if !rings.is_empty() {
        let sizes = as_i64(src.get_item("sizes")?)?;
        let corners: Vec<i64> = rings.iter().map(|&c| inverse[c as usize]).collect();
        let mut start = 0usize;
        for size in sizes {
            let size = size as usize;
            for j in 0..size {
                let next = if j + 1 == size { start } else { start + j + 1 };
                all.insert(key(corners[start + j], corners[next]));
            }
            start += size;
        }
    }
    let mut total = all.len() as i64;
    let layers = src.get_item("layers")?;
    if !layers.is_none() && !src.get_item("flags")?.is_none() {
        let labels = as_i64(layers)?;
        let names = src.get_item("meta")?.call_method1("get", ("edge_layers",))?;
        let names = if names.is_none() { Vec::new() } else { list_of(&names)? };
        // The last nonnegative label of each repeated edge wins.
        let mut last: HashMap<i64, i64> = HashMap::new();
        for (&i, &k) in keep.iter().zip(&explicit) {
            let label = labels[i];
            if label >= 0 {
                last.insert(k, label);
            }
        }
        let mut per_label: BTreeMap<i64, i64> = BTreeMap::new();
        for label in last.values() {
            *per_label.entry(*label).or_insert(0) += 1;
        }
        for (label, n) in per_label {
            bump(tally, &names[label as usize], n)?;
        }
        total -= last.len() as i64;
    }
    if total != 0 {
        bump(tally, &py.None().into_bound(py), total)?;
    }
    Ok(())
}

#[pyfunction]
fn purge_count_edges(py: Python<'_>, src: &Obj<'_>, tally: &Obj<'_>) -> PyResult<()> {
    count_edges(py, src, tally)
}

/// Count one mesh's faces and edges by tag into `tally`, once per mesh.
fn count_mesh(py: Python<'_>, mesh: &Obj<'_>, tally: &Obj<'_>, seen: &mut HashSet<usize>) -> PyResult<()> {
    if mesh.is_none() || !seen.insert(mesh.as_ptr() as usize) {
        return Ok(());
    }
    let src = mesh.getattr("__dict__")?.call_method1("get", ("_deferred_source",))?;
    if src.is_none() {
        if crate::store::tally::store_layer_tally(mesh, tally)? {
            return Ok(());
        }
        for f in list_of(&mesh.getattr("faces")?)? {
            bump(tally, &layer_of(&f)?, 1)?;
        }
        for e in list_of(&mesh.getattr("edges")?)? {
            bump(tally, &layer_of(&e)?, 1)?;
        }
        return Ok(());
    }
    count_edges(py, &src, tally)?;
    let face_attr = src.call_method1("get", ("face_attr",))?;
    let none = py.None().into_bound(py);
    if face_attr.is_none() {
        return bump(tally, &none, src.get_item("counts")?.len()? as i64);
    }
    let table = src.get_item("meta")?.call_method1("get", ("face_attrs",))?;
    let table = if table.is_none() { Vec::new() } else { list_of(&table)? };
    let np = py.import("numpy")?;
    let rows: Vec<i64> = np
        .call_method1("ascontiguousarray", (face_attr, "int64"))?
        .call_method0("ravel")?
        .extract::<PyReadonlyArray1<'_, i64>>()?
        .as_slice()?
        .to_vec();
    let mut per_row = vec![0i64; table.len()];
    let mut unnamed = 0i64;
    for r in rows {
        match usize::try_from(r).ok().filter(|&r| r < per_row.len()) {
            Some(r) => per_row[r] += 1,
            None => unnamed += 1,
        }
    }
    bump(tally, &none, unnamed)?;
    for (raw, n) in table.iter().zip(per_row) {
        if n != 0 {
            let name = if raw.is_truthy()? { raw.call_method1("get", ("layer",))? } else { none.clone() };
            bump(tally, &name, n)?;
        }
    }
    Ok(())
}

enum Stage {
    Start,
    Groups(usize),
    Done,
}

/// `core.purge.layer_count_steps`: None between meshes and group batches,
/// then one complete `{tag: count}`. With a History, the counts outside
/// the edited mesh are kept while History accounts for every edit since.
#[pyclass(module = "ingetrazo_core", name = "LayerCountSteps")]
pub struct LayerCountSteps {
    scene: Py<PyAny>,
    history: Option<Py<PyAny>>,
    photo: bool,
    tally: Py<PyAny>,
    seen: HashSet<usize>,
    groups: Vec<Py<PyAny>>,
    active: Option<Py<PyAny>>,
    active_present: bool,
    key: Option<Py<PyAny>>,
    stage: Stage,
}

impl LayerCountSteps {
    fn finish<'py>(&mut self, py: Python<'py>) -> PyResult<Obj<'py>> {
        let scene = self.scene.bind(py);
        let tally = self.tally.bind(py);
        let counts = PyDict::new(py);
        let add = |name: &Obj<'_>, n: i64| -> PyResult<()> {
            let name = if name.is_truthy()? { name.clone() } else { pyo3::types::PyString::new(py, DEFAULT_LAYER).into_any() };
            let current: i64 = match counts.get_item(&name)? {
                Some(v) => v.extract()?,
                None => 0,
            };
            counts.set_item(name, current + n)
        };
        for item in tally.call_method0("items")?.try_iter()? {
            let item = item?;
            add(&item.get_item(0)?, item.get_item(1)?.extract()?)?;
        }
        for ent in annotations(scene)? {
            add(&layer_of(&ent)?, 1)?;
        }
        if self.photo {
            let survey = attr_or_none(scene, "photo_mesh")?;
            if !survey.is_none() {
                add(&layer_of(&survey)?, 1)?;
            }
        }
        self.stage = Stage::Done;
        Ok(counts.into_any())
    }
}

#[pymethods]
impl LayerCountSteps {
    #[new]
    #[pyo3(signature = (scene, photo = true, history = None))]
    fn new(py: Python<'_>, scene: Obj<'_>, photo: bool, history: Option<Obj<'_>>) -> PyResult<Self> {
        let tally = py.import("collections")?.getattr("Counter")?.call0()?;
        let history = history.filter(|h| !h.is_none());
        let key = match &history {
            Some(_) => Some(PyTuple::new(py, [
                (scene.as_ptr() as usize).into_pyobject(py)?.into_any(),
                scene.getattr("content_version")?,
                (scene.getattr("mesh")?.as_ptr() as usize).into_pyobject(py)?.into_any(),
            ])?.into_any().unbind()),
            None => None,
        };
        Ok(Self {
            scene: scene.unbind(),
            history: history.map(|h| h.unbind()),
            photo,
            tally: tally.unbind(),
            seen: HashSet::new(),
            groups: Vec::new(),
            active: None,
            active_present: false,
            key,
            stage: Stage::Start,
        })
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__<'py>(&mut self, py: Python<'py>) -> PyResult<Option<Obj<'py>>> {
        let scene = self.scene.clone_ref(py).into_bound(py);
        let tally = self.tally.clone_ref(py).into_bound(py);
        let none = py.None().into_bound(py);
        loop {
            match self.stage {
                Stage::Done => return Ok(None),
                Stage::Start => {
                    let cached = match &self.history {
                        Some(h) => h.bind(py).getattr_opt("layer_count_rest")?.filter(|c| !c.is_none()),
                        None => None,
                    };
                    if let (Some(cached), Some(key)) = (&cached, &self.key) {
                        if cached.get_item(0)?.eq(key.bind(py))? {
                            tally.call_method1("update", (cached.get_item(1)?,))?;
                            count_mesh(py, &scene.getattr("mesh")?, &tally, &mut self.seen)?;
                            return self.finish(py).map(Some);
                        }
                    }
                    let active = if self.history.is_some() { Some(scene.getattr("mesh")?) } else { None };
                    // A detached edit mesh counts only if the walk meets it.
                    let active_ptr = active.as_ref().map_or(py.None().as_ptr() as usize, |a| a.as_ptr() as usize);
                    self.seen.insert(active_ptr);
                    let loose = attr_or_none(&scene, "loose_mesh")?;
                    self.active_present = active.as_ref().is_some_and(|a| a.is(&loose));
                    self.active = active.map(|a| a.unbind());
                    count_mesh(py, &loose, &tally, &mut self.seen)?;
                    self.groups = walk(py, &document_groups(py, &scene)?)?.into_iter().map(|g| g.unbind()).collect();
                    self.stage = Stage::Groups(0);
                    return Ok(Some(none));
                }
                Stage::Groups(i) if i < self.groups.len() => {
                    let group = self.groups[i].clone_ref(py).into_bound(py);
                    self.stage = Stage::Groups(i + 1);
                    bump(&tally, &layer_of(&group)?, 1)?;
                    let mesh = group.getattr("mesh")?;
                    if let Some(active) = &self.active {
                        self.active_present |= mesh.is(active.bind(py));
                    }
                    if !self.seen.contains(&(mesh.as_ptr() as usize)) {
                        count_mesh(py, &mesh, &tally, &mut self.seen)?;
                        return Ok(Some(none));
                    }
                    if i % 256 == 0 {
                        return Ok(Some(none));
                    }
                }
                Stage::Groups(_) => {
                    if let Some(active) = self.active.as_ref().map(|a| a.clone_ref(py).into_bound(py)) {
                        if self.active_present {
                            if let (Some(history), Some(key)) = (&self.history, &self.key) {
                                if scene.getattr("content_version")?.eq(key.bind(py).get_item(1)?)? {
                                    let rest = PyTuple::new(py, [key.bind(py).clone(), tally.call_method0("copy")?])?;
                                    history.bind(py).setattr("layer_count_rest", rest)?;
                                }
                            }
                            self.seen.remove(&(active.as_ptr() as usize));
                            count_mesh(py, &active, &tally, &mut self.seen)?;
                        }
                    }
                    return self.finish(py).map(Some);
                }
            }
        }
    }
}

/// Every tag something in the document carries, counted as the Layers
/// panel counts (a lazy mesh is not built to answer); the survey's left out.
fn used_layers<'py>(py: Python<'py>, scene: &Obj<'py>) -> PyResult<HashSet<String>> {
    let mut steps = LayerCountSteps::new(py, scene.clone(), false, None)?;
    let mut used: HashSet<String> = [DEFAULT_LAYER.to_owned()].into();
    while let Some(step) = steps.__next__(py)? {
        if !step.is_none() {
            for name in step.try_iter()? {
                used.insert(name?.extract()?);
            }
        }
    }
    Ok(used)
}

#[pyfunction]
fn purge_used_layers<'py>(py: Python<'py>, scene: &Obj<'py>) -> PyResult<Bound<'py, PySet>> {
    PySet::new(py, used_layers(py, scene)?)
}

/// Tags a saved view switches off: part of that view's meaning, kept.
fn held_by_scenes(scene: &Obj<'_>) -> PyResult<HashSet<String>> {
    let mut held = HashSet::new();
    for view in list_of(&attr_or_none(scene, "saved_views")?)? {
        for name in list_of(&attr_or_none(&view, "hidden_layers")?)? {
            held.insert(name.extract()?);
        }
    }
    Ok(held)
}

#[pyfunction]
fn purge_layers_held_by_scenes<'py>(py: Python<'py>, scene: &Obj<'py>) -> PyResult<Bound<'py, PySet>> {
    PySet::new(py, held_by_scenes(scene)?)
}

/// The tags a purge would remove.
#[pyfunction]
fn purge_unused_layers<'py>(py: Python<'py>, scene: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let mut keep = used_layers(py, scene)?;
    keep.extend(held_by_scenes(scene)?);
    let mut out = Vec::new();
    for tag in list_of(&attr_or_none(scene, "layers")?)? {
        let key: String = tag.getattr("key")?.extract()?;
        if key != DEFAULT_LAYER && !keep.contains(&key) {
            out.push(tag);
        }
    }
    PyList::new(py, out)
}

/// Every registry material name a face or a placement's own paint wears.
fn used_materials<'py>(py: Python<'py>, scene: &Obj<'py>) -> PyResult<Bound<'py, PySet>> {
    let used = PySet::empty(py)?;
    for mesh in meshes(py, scene)? {
        if let Some(names) = crate::store::tally::store_material_names(&mesh)? {
            for n in names.iter() {
                used.add(n)?;
            }
            continue;
        }
        for f in list_of(&mesh.getattr("faces")?)? {
            let name = f.getattr("attrs")?.call_method1("get", ("mat",))?;
            if name.is_truthy()? {
                used.add(name)?;
            }
        }
    }
    // A group's own paint lives on the group, not on its faces.
    let placements = py.import("core.group")?.getattr("iter_placements")?;
    for top in list_of(&document_groups(py, scene)?)? {
        for row in placements.call1((top,))?.try_iter()? {
            let paint = attr_or_none(&row?.get_item(0)?, "material")?;
            if paint.is_truthy()? {
                let name = paint.call_method1("get", ("mat",))?;
                if name.is_truthy()? {
                    used.add(name)?;
                }
            }
        }
    }
    Ok(used)
}

#[pyfunction]
fn purge_used_materials<'py>(py: Python<'py>, scene: &Obj<'py>) -> PyResult<Bound<'py, PySet>> {
    used_materials(py, scene)
}

/// Registry material names nothing wears, in registry order.
#[pyfunction]
fn purge_unused_materials<'py>(py: Python<'py>, scene: &Obj<'py>) -> PyResult<Bound<'py, PyList>> {
    let used = used_materials(py, scene)?;
    let mut out = Vec::new();
    for name in list_of(&attr_or_none(scene, "materials")?)? {
        if !used.contains(&name)? {
            out.push(name);
        }
    }
    PyList::new(py, out)
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<LayerCountSteps>()?;
    m.add_function(wrap_pyfunction!(purge_iter_groups, m)?)?;
    m.add_function(wrap_pyfunction!(purge_count_edges, m)?)?;
    m.add_function(wrap_pyfunction!(purge_document_groups, m)?)?;
    m.add_function(wrap_pyfunction!(purge_iter_meshes, m)?)?;
    m.add_function(wrap_pyfunction!(purge_annotations, m)?)?;
    m.add_function(wrap_pyfunction!(purge_layer_carriers, m)?)?;
    m.add_function(wrap_pyfunction!(purge_used_layers, m)?)?;
    m.add_function(wrap_pyfunction!(purge_layers_held_by_scenes, m)?)?;
    m.add_function(wrap_pyfunction!(purge_unused_layers, m)?)?;
    m.add_function(wrap_pyfunction!(purge_used_materials, m)?)?;
    m.add_function(wrap_pyfunction!(purge_unused_materials, m)?)?;
    Ok(())
}
