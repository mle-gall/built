//! Face paint summaries retained across independent edits and placement changes.
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Default)]
struct Paints {
    names: HashSet<String>,
    colours: BTreeMap<String, ([f64; 3], Option<f64>, usize)>,
}

impl Paints {
    fn add(&mut self, attrs: &Bound<'_, PyDict>, count: usize) -> PyResult<()> {
        if let Some(name) = attrs
            .get_item("mat")?
            .and_then(|v| v.extract::<String>().ok())
            .filter(|s| !s.is_empty())
        {
            self.names.insert(name);
        } else if !attrs
            .get_item("texture")?
            .is_some_and(|v| v.is_truthy().unwrap_or(false))
        {
            if let Some(rgb) = attrs
                .get_item("color")?
                .and_then(|v| v.extract::<[f64; 3]>().ok())
            {
                let [r, g, b] = rgb.map(|c| (255. * c.clamp(0., 1.)).round_ties_even() as u8);
                let opacity = attrs
                    .get_item("opacity")?
                    .and_then(|v| v.extract::<f64>().ok());
                let key = format!(
                    "#{r:02X}{g:02X}{b:02X}{}",
                    opacity.map(|a| format!("@{a:.2}")).unwrap_or_default()
                );
                self.colours.entry(key).or_insert((rgb, opacity, 0)).2 += count;
            }
        }
        Ok(())
    }

    fn scan(mesh: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut result = Self::default();
        let rows = mesh
            .py()
            .import("built_core.geometry_arrays")?
            .call_method1("face_attributes", (mesh,))?;
        for row in rows.try_iter()? {
            let row = row?;
            let Ok(attrs) = row.cast::<PyDict>() else {
                continue;
            };
            result.add(attrs, 1)?;
            if let Some(back) = attrs.get_item("back")? {
                if let Ok(back) = back.cast::<PyDict>() {
                    result.add(back, 1)?;
                }
            }
        }
        Ok(result)
    }
}

struct MeshPaint {
    owner: Py<PyAny>, // Keeps the pointer identity unique until the next survey.
    serial: Option<u64>,
    paints: Paints,
}

#[pyclass]
#[derive(Default)]
struct Survey {
    version: Option<i64>,
    meshes: HashMap<usize, MeshPaint>,
    result: Option<Py<PyTuple>>,
}

#[pymethods]
impl Survey {
    fn __traverse__(&self, visit: pyo3::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        for record in self.meshes.values() {
            visit.call(&record.owner)?;
        }
        if let Some(result) = &self.result {
            visit.call(result)?;
        }
        Ok(())
    }

    fn __clear__(&mut self) {
        *self = Self::default();
    }
}

#[pyfunction]
#[pyo3(signature=(scene, history=None))]
pub(crate) fn material_survey<'py>(
    scene: &Bound<'py, PyAny>,
    history: Option<&Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyTuple>> {
    let py = scene.py();
    let attrs = scene.getattr("__dict__")?;
    let cache = if let Some(cache) = attrs.cast::<PyDict>()?.get_item("_library_paint_survey")? {
        cache.cast_into::<Survey>()?
    } else {
        let cache = Bound::new(py, Survey::default())?;
        scene.setattr("_library_paint_survey", &cache)?;
        cache
    };
    let mut cache = cache.try_borrow_mut()?;
    let version = scene.getattr("content_version")?.extract::<i64>()?;
    let feed = history.map(|h| h.getattr("feed")).transpose()?;
    let current = match &feed {
        Some(f) => f
            .call_method1("current", ("library_paint", version))?
            .is_truthy()?,
        None => false,
    };
    // A transform may increment even a baked mesh's geometry serial. A
    // complete paint-preserving edit keeps the totals as well as each swatch.
    let same_paints = match &feed {
        Some(f) => f
            .call_method1("current", ("library_paint_totals", version))?
            .is_truthy()?,
        None => false,
    };
    if same_paints || (cache.version == Some(version) && (feed.is_none() || current)) {
        cache.version = Some(version);
        if let Some(result) = &cache.result {
            return Ok(result.bind(py).clone());
        }
    }
    if !current {
        cache.meshes.clear();
    } else if let Some(f) = &feed {
        let keys = py.import("built_core.command_protocol")?;
        for key in f
            .getattr("readers")?
            .call_method0("dirty_library_paint_meshes")?
            .try_iter()?
        {
            let mesh = keys.call_method1("known_mesh", (key?,))?;
            cache.meshes.remove(&(mesh.as_ptr() as usize));
        }
    }
    let mut named = BTreeMap::<String, usize>::new();
    let mut anonymous = BTreeMap::<String, ([f64; 3], Option<f64>, usize)>::new();
    let mut tally = |paints: &Paints| {
        for name in &paints.names {
            *named.entry(name.clone()).or_default() += 1;
        }
        for (key, (rgb, opacity, count)) in &paints.colours {
            anonymous
                .entry(key.clone())
                .or_insert((*rgb, *opacity, 0))
                .2 += count;
        }
    };
    let purge = py.import("built_core")?;
    let mut next = HashMap::new();
    for mesh in purge
        .call_method1("purge_iter_meshes", (scene,))?
        .try_iter()?
    {
        let mesh = mesh?;
        let id = mesh.as_ptr() as usize;
        let serial = mesh
            .getattr("__dict__")?
            .cast::<PyDict>()?
            .get_item("_mut_serial")?
            .and_then(|v| v.extract::<u64>().ok());
        let record = match cache.meshes.remove(&id) {
            Some(record) if record.serial == serial => record,
            _ => MeshPaint {
                paints: Paints::scan(&mesh)?,
                serial,
                owner: mesh.unbind(),
            },
        };
        tally(&record.paints);
        next.insert(id, record);
    }
    let registry = scene.getattr("materials")?;
    let mut stored = HashSet::new();
    let mut recipes = Vec::new();
    let groups = purge.call_method1("purge_document_groups", (scene,))?;
    for group in purge
        .call_method1("purge_iter_groups", (groups,))?
        .try_iter()?
    {
        let attrs = group?.getattr("material")?;
        if let Ok(paint) = attrs.cast::<PyDict>() {
            let mut paints = Paints::default();
            paints.add(paint, 1)?;
            tally(&paints);
            if let Some(name) = paints.names.into_iter().next() {
                if !registry.contains(&name)? && stored.insert(name.clone()) {
                    recipes.push((name, paint.clone()));
                }
            }
        }
    }
    let mut worn = Vec::with_capacity(recipes.len());
    for (name, attrs) in recipes {
        worn.push(crate::material_library::container_swatch(
            &attrs,
            &name,
            named[&name],
        )?);
    }
    let counts = py
        .import("collections")?
        .call_method1("Counter", (named,))?;
    let result = (counts, anonymous, worn).into_pyobject(py)?;
    if let Some(f) = feed {
        let readers = f.getattr("readers")?;
        readers.call_method1("mark", ("library_paint", version))?;
        readers.call_method1("mark", ("library_paint_totals", version))?;
    }
    cache.version = Some(version);
    cache.meshes = next;
    cache.result = Some(result.clone().unbind());
    Ok(result)
}

/// Display counts include back paint. The purge command and linked-file
/// classification use front paint, so retain that subset independently.
fn mesh_usage<'py>(mesh: &Bound<'py, PyAny>) -> PyResult<(Paints, HashSet<String>)> {
    let py = mesh.py();
    let mut paint = Paints::default();
    let mut front = HashSet::new();
    let mut add = |row: Bound<'py, PyAny>, count| -> PyResult<()> {
        if let Ok(attrs) = row.cast::<PyDict>() {
            paint.add(attrs, count)?;
            if let Some(name) = attrs
                .get_item("mat")?
                .and_then(|v| v.extract::<String>().ok())
                .filter(|s| !s.is_empty())
            {
                front.insert(name);
            }
            if let Some(back) = attrs.get_item("back")? {
                if let Ok(back) = back.cast::<PyDict>() {
                    paint.add(back, count)?;
                }
            }
        }
        Ok(())
    };
    if let Some(rows) = crate::store::tally::paint_rows(mesh)? {
        for (row, n) in rows {
            add(row, n)?;
        }
    } else if let Some(src) = mesh
        .getattr("__dict__")?
        .cast::<PyDict>()?
        .get_item("_deferred_source")?
        .filter(|s| !s.is_none())
    {
        let indices = src.get_item("face_attr")?;
        if !indices.is_none() {
            let column = py
                .import("numpy")?
                .call_method1("ascontiguousarray", (indices, "int64"))?;
            let keys = column
                .extract::<numpy::PyReadonlyArray1<i64>>()?
                .as_slice()?
                .to_vec();
            let histogram = py.detach(move || {
                let mut counts = BTreeMap::new();
                for key in keys.into_iter().filter(|k| *k >= 0) {
                    *counts.entry(key).or_default() += 1usize;
                }
                counts
            });
            let table = src.get_item("meta")?.call_method1("get", ("face_attrs",))?;
            let parse = py
                .import("built_core.formats.igz")?
                .getattr("_face_attrs_from_json")?;
            for (key, count) in histogram {
                add(parse.call1((table.get_item(key)?,))?, count)?;
            }
        }
    } else {
        let rows = py
            .import("built_core.geometry_arrays")?
            .call_method1("face_attributes", (mesh,))?;
        for row in rows.try_iter()? {
            add(row?, 1)?;
        }
    }
    Ok((paint, front))
}

/// Native registry edits from scripts can precede the scene-version bump.
#[pyfunction]
pub(crate) fn library_material_revision(scene: &Bound<'_, PyAny>) -> PyResult<(i64, u64)> {
    let version = scene.getattr("content_version")?.extract()?;
    let root = scene.getattr("_root")?;
    let epoch = if root.is_none() { 0 } else {
        let key = crate::doc::py::handle_key(&root)?;
        *crate::doc::lock().reader_epochs.entry(key).or_default()
    };
    Ok((version, epoch))
}

// A revision-local survey for the open Library. A worker owns its accumulators,
// and drops them if an edit overtakes it. Never borrow the history feed here.
#[pyfunction]
pub(crate) fn library_usage<'py>(
    scene: &Bound<'py, PyAny>,
    version: i64,
) -> PyResult<Option<Bound<'py, PyTuple>>> {
    let py = scene.py();
    let engine = py.import("built_core")?;
    let groups = engine.call_method1("purge_document_groups", (scene,))?;
    let placements = engine.call_method1("purge_iter_groups", (groups,))?;
    let mut meshes = HashMap::new();
    let loose = scene.getattr("loose_mesh")?;
    meshes.insert(loose.as_ptr() as usize, loose);
    let mut uses = BTreeMap::<String, usize>::new();
    let mut colours = BTreeMap::new();
    let mut used_front = HashSet::new();
    let registry = scene
        .getattr("materials")?
        .try_iter()?
        .map(|name| name?.extract::<String>())
        .collect::<PyResult<HashSet<_>>>()?;
    let mut recipes = BTreeMap::new();
    let mut merge = |paint: Paints| {
        for name in paint.names {
            *uses.entry(name).or_default() += 1;
        }
        for (key, (rgb, alpha, n)) in paint.colours {
            colours.entry(key).or_insert((rgb, alpha, 0usize)).2 += n;
        }
    };
    let mut checkpoint = std::time::Instant::now();
    let mut current = || -> PyResult<bool> {
        if checkpoint.elapsed().as_millis() >= 2 {
            py.detach(std::thread::yield_now);
            checkpoint = std::time::Instant::now();
        }
        scene.getattr("content_version")?.eq(version)
    };
    for group in placements.try_iter()? {
        if !current()? {
            return Ok(None);
        }
        let group = group?;
        let mesh = group.getattr("mesh")?;
        if !mesh.is_none() {
            meshes.insert(mesh.as_ptr() as usize, mesh);
        }
        if let Ok(attrs) = group.getattr("material")?.cast_into::<PyDict>() {
            let mut paint = Paints::default();
            paint.add(&attrs, 1)?;
            for name in &paint.names {
                used_front.insert(name.clone());
                if !registry.contains(name) {
                    recipes.entry(name.clone()).or_insert_with(|| attrs.clone());
                }
            }
            merge(paint);
        }
    }
    let mut front_by_mesh = HashMap::new();
    for (id, mesh) in meshes {
        if !current()? {
            return Ok(None);
        }
        let (paint, front) = mesh_usage(&mesh)?;
        used_front.extend(front.iter().cloned());
        front_by_mesh.insert(id, front);
        merge(paint);
    }
    let links = py.import("built_core.links")?;
    let files = PyDict::new(py);
    for path in links.call_method1("sources", (scene,))?.try_iter()? {
        let path = path?;
        let mut counts = BTreeMap::<String, usize>::new();
        for top in links
            .call_method1("linked_groups", (scene, &path))?
            .try_iter()?
        {
            for group in engine
                .call_method1("purge_iter_groups", ((top?,),))?
                .try_iter()?
            {
                if !current()? {
                    return Ok(None);
                }
                let mesh = group?.getattr("mesh")?;
                for name in front_by_mesh
                    .get(&(mesh.as_ptr() as usize))
                    .into_iter()
                    .flatten()
                {
                    *counts.entry(name.clone()).or_default() += 1;
                }
            }
        }
        if !counts.is_empty() {
            files.set_item(path, counts)?;
        }
    }
    let mut worn = Vec::new();
    for (name, attrs) in recipes {
        worn.push(crate::material_library::container_swatch(
            &attrs,
            &name,
            uses[&name],
        )?);
    }
    if !current()? {
        return Ok(None);
    }
    let unused = registry.difference(&used_front).count();
    Ok(Some(
        (uses, colours, worn, files, unused).into_pyobject(py)?,
    ))
}
