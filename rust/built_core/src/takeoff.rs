//! Quantities requested in a document's inspector. Geometry measurement and IFC
//! class rules come from the same backend that writes the quantities CSV.
use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};
use std::collections::HashMap;

#[pyclass(module = "built_core")]
pub struct InspectorTakeoff {
    memo: Option<Py<PyDict>>,
    totals: HashMap<usize, HashMap<String, f64>>,
    face_sets: HashMap<usize, Vec<(String, String)>>,
    version: Option<i64>,
    #[pyo3(get)]
    count: usize,
}

#[pymethods]
impl InspectorTakeoff {
    #[new]
    fn new() -> Self {
        Self {
            memo: None,
            totals: HashMap::new(),
            face_sets: HashMap::new(),
            version: None,
            count: 0,
        }
    }

    fn refresh(&mut self, history: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = history.py();
        let scene = history.getattr("scene")?;
        let cv = scene.getattr("content_version")?.extract()?;
        if self.version == Some(cv) {
            return Ok(());
        }
        let readers = history.getattr("feed")?.getattr("readers")?;
        if self.version.is_some()
            && self.count == 0
            && readers.call_method1("current", ("bim", cv))?.is_truthy()?
        {
            self.version = Some(cv);
            return Ok(());
        }
        let backend = py.import("core.bim")?;
        let memo = self.memo.get_or_insert_with(|| PyDict::new(py).unbind());
        let objects = backend.call_method1("collect_objects", (scene, memo.bind(py)))?;
        let mut totals: HashMap<usize, HashMap<String, f64>> = HashMap::new();
        let mut face_sets = HashMap::<usize, Vec<(String, String)>>::new();
        for obj in objects.try_iter()? {
            let obj = obj?;
            let group = obj.call_method1("get", ("group",))?;
            let owner = if group.is_none() {
                obj.call_method1("get", ("owner",))?
            } else {
                group
            };
            if owner.is_none() {
                continue;
            }
            if obj.contains("tag_id")? {
                let tag: String = obj.get_item("tag_id")?.repr()?.extract()?;
                let name: String = obj.get_item("name")?.extract()?;
                let label = if name.is_empty() {
                    obj.get_item("class")?.extract()?
                } else {
                    name
                };
                face_sets
                    .entry(owner.as_ptr() as usize)
                    .or_default()
                    .push((tag, label));
            }
            let mut quantity = obj.call_method1("get", ("_q",))?;
            if quantity.is_none() {
                quantity = backend
                    .call_method1("object_quantities", (&obj,))?
                    .get_item(2)?;
                obj.set_item("_q", &quantity)?;
            }
            let (value, unit): (Option<f64>, String) = quantity.extract()?;
            if let Some(value) = if unit == "und" { Some(1.) } else { value } {
                if !unit.is_empty() {
                    *totals
                        .entry(owner.as_ptr() as usize)
                        .or_default()
                        .entry(unit)
                        .or_default() += value;
                }
            }
        }
        readers.call_method1("mark", ("bim", cv))?;
        self.totals = totals;
        self.face_sets = face_sets;
        self.count = objects.len()?;
        self.version = Some(cv);
        Ok(())
    }

    fn quantities(&self, owner: &Bound<'_, PyAny>) -> HashMap<String, f64> {
        self.totals
            .get(&(owner.as_ptr() as usize))
            .cloned()
            .unwrap_or_default()
    }

    fn face_actions(&self, owner: &Bound<'_, PyAny>) -> Vec<(String, String)> {
        let address = owner.as_ptr() as usize;
        self.face_sets
            .get(&address)
            .into_iter()
            .flatten()
            .map(|(tag, label)| {
                (
                    format!("takeoff_faces:{address}/{tag}"),
                    format!("Select {label} faces"),
                )
            })
            .collect()
    }

    // Navigation can notify the inspector synchronously. Release its model borrow
    // before opening the edit path, then resolve the tags in the editable mesh.
    #[staticmethod]
    fn select_faces(vp: &Bound<'_, PyAny>, key: &str) -> PyResult<()> {
        let scene = vp.getattr("scene")?;
        let selected = scene.getattr("selection")?;
        if selected.len()? != 1 {
            return Ok(());
        }
        let Some(group) = selected.try_iter()?.next().transpose()? else {
            return Ok(());
        };
        let address = group.as_ptr() as usize;
        let Some(tag) = key.strip_prefix(&format!("{address}/")) else {
            return Ok(());
        };
        let Some(model) = scene.getattr_opt("_inspector_takeoff")? else {
            return Ok(());
        };
        {
            let model = model.cast::<InspectorTakeoff>()?.borrow();
            if model.version != Some(scene.getattr("content_version")?.extract()?)
                || !model
                    .face_sets
                    .get(&address)
                    .is_some_and(|tags| tags.iter().any(|(id, _)| *id == tag))
            {
                return Ok(());
            }
        }
        let path = vp
            .py()
            .import("built_core.outliner")?
            .call_method1("path_to", (scene.getattr("groups")?, &group))?;
        if path.is_none() {
            return Ok(());
        }
        path.call_method1("append", (&group,))?;
        if !vp
            .py()
            .import("built_render.tree_navigation")?
            .call_method1("open_to", (vp, path))?
            .is_truthy()?
        {
            vp.call_method1(
                "flash_status",
                ("Show and unlock the parent groups to select these faces.",),
            )?;
            return Ok(());
        }
        let faces = PyList::empty(vp.py());
        for face in scene.getattr("mesh")?.getattr("faces")?.try_iter()? {
            let face = face?;
            let ifc = face.getattr("attrs")?.call_method1("get", ("ifc",))?;
            if ifc.is_truthy()? && ifc.call_method1("get", ("id",))?.repr()?.to_str()? == tag {
                faces.append(face)?;
            }
        }
        scene.call_method1("select", (faces,))?;
        vp.call_method0("notify_scene_changed")?;
        Ok(())
    }

    fn __traverse__(&self, visit: pyo3::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        visit.call(&self.memo)
    }

    fn __clear__(&mut self) {
        self.memo = None;
    }
}
