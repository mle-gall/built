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
    fn add(&mut self, attrs: &Bound<'_, PyDict>) -> PyResult<()> {
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
                self.colours.entry(key).or_insert((rgb, opacity, 0)).2 += 1;
            }
        }
        Ok(())
    }

    fn scan(mesh: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut result = Self::default();
        let rows = mesh
            .py()
            .import("core.geometry_arrays")?
            .call_method1("face_attributes", (mesh,))?;
        for row in rows.try_iter()? {
            let row = row?;
            let Ok(attrs) = row.cast::<PyDict>() else {
                continue;
            };
            result.add(attrs)?;
            if let Some(back) = attrs.get_item("back")? {
                if let Ok(back) = back.cast::<PyDict>() {
                    result.add(back)?;
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
        Some(f) => f.call_method1("current", ("library_paint_totals", version))?.is_truthy()?,
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
        let keys = py.import("core.protocol.keys")?;
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
    let purge = py.import("core.purge")?;
    let mut next = HashMap::new();
    for mesh in purge.call_method1("iter_meshes", (scene,))?.try_iter()? {
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
    let groups = purge.call_method1("_document_groups", (scene,))?;
    for group in purge.call_method1("iter_groups", (groups,))?.try_iter()? {
        let attrs = group?.getattr("material")?;
        if let Ok(paint) = attrs.cast::<PyDict>() {
            let mut paints = Paints::default();
            paints.add(paint)?;
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
