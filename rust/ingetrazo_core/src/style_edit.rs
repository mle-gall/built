//! History steps for the layer style catalogue and tag folders.
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList},
};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

type Change = (Py<PyAny>, &'static str, Py<PyAny>, Py<PyAny>);

#[pyclass(dict)]
pub struct StyleEdit {
    #[pyo3(get)]
    history_name: String,
    changes: Vec<Change>,
}

impl StyleEdit {
    fn new(name: &str) -> Self {
        Self {
            history_name: name.into(),
            changes: vec![],
        }
    }
    fn stage(
        &mut self,
        target: &Bound<'_, PyAny>,
        attr: &'static str,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let before = target.getattr(attr)?;
        if !before.eq(value)? {
            let copy = target.py().import("copy")?.getattr("deepcopy")?;
            self.changes.push((
                target.clone().unbind(),
                attr,
                copy.call1((before,))?.unbind(),
                copy.call1((value,))?.unbind(),
            ));
        }
        Ok(())
    }
    fn apply(&self, scene: &Bound<'_, PyAny>, forward: bool) -> PyResult<()> {
        let py = scene.py();
        let copy = py.import("copy")?.getattr("deepcopy")?;
        for (target, attr, old, new) in &self.changes {
            target
                .bind(py)
                .setattr(*attr, copy.call1((if forward { new } else { old },))?)?;
        }
        py.import("core.history")?
            .call_method1("_dirty_group_chunks", (scene,))?;
        scene.setattr("version", scene.getattr("version")?.extract::<u64>()? + 1)
    }
    fn submit(self, history: &Bound<'_, PyAny>) -> PyResult<()> {
        if !self.changes.is_empty() {
            history.call_method1("execute", (Py::new(history.py(), self)?,))?;
            for attr in ["last_rejected", "last_error"] {
                if let Some(err) = history.getattr(attr)?.extract::<Option<String>>()? {
                    return Err(PyValueError::new_err(err));
                }
            }
        }
        Ok(())
    }
}

#[pymethods]
impl StyleEdit {
    #[classattr]
    #[pyo3(name = "keeps_rows")]
    const KEEPS_ROWS: bool = true;
    fn r#do(&self, scene: &Bound<'_, PyAny>) -> PyResult<()> {
        self.apply(scene, true)
    }
    fn undo(&self, scene: &Bound<'_, PyAny>) -> PyResult<()> {
        self.apply(scene, false)
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct LineDraft {
    name: String,
    original: String,
    pieces: Vec<f64>,
    text: String,
    text_size: f64,
    color: Option<[f64; 3]>,
    tags: Vec<String>,
}

/// Change only the selected tags in each saved view. Preserve all other
/// visibility entries and restore their exact order on undo.
#[pyfunction]
pub fn tags_in_saved_views(history: &Bound<'_, PyAny>, keys: Vec<String>, hidden: bool) -> PyResult<()> {
    let scene = history.getattr("scene")?;
    if scene.getattr("worksession_read_only").is_ok_and(|v| v.is_truthy().unwrap_or(false)) { return Ok(()); }
    for key in &keys {
        if scene.call_method1("layer_by_key", (key,))?.is_none() { return Ok(()); }
    }
    let mut edit = StyleEdit::new("Change tag visibility in saved scenes");
    for view in scene.getattr("saved_views")?.try_iter()? {
        let view = view?;
        let mut after = view.getattr("hidden_layers")?.extract::<Option<Vec<String>>>()?.unwrap_or_default();
        if hidden {
            for key in &keys {
                if !after.contains(key) { after.push(key.clone()); }
            }
        } else { after.retain(|key| !keys.contains(key)); }
        edit.stage(&view, "hidden_layers", after.into_pyobject(history.py())?.as_any())?;
    }
    edit.submit(history)
}

#[pyfunction]
pub fn edit_line_type(history: &Bound<'_, PyAny>, verb: &str, draft: &str) -> PyResult<String> {
    let d: LineDraft =
        serde_json::from_str(draft).map_err(|e| PyValueError::new_err(e.to_string()))?;
    let py = history.py();
    let scene = history.getattr("scene")?;
    if verb == "ink" {
        if crate::model::linetypes::builtin(&d.name).is_none() {
            return Err(PyValueError::new_err("Unknown standard line type"));
        }
        let colors = scene
            .getattr("linetype_colors")?
            .cast_into::<PyDict>()?
            .copy()?;
        if let Some(rgb) = d.color {
            if !rgb.iter().all(|v| (0.0..=1.0).contains(v)) {
                return Err(PyValueError::new_err("Invalid line colour"));
            }
            colors.set_item(&d.name, (rgb[0], rgb[1], rgb[2]))?;
        } else if colors.contains(&d.name)? {
            colors.del_item(&d.name)?;
        }
        let mut edit = StyleEdit::new("Set line type colour");
        edit.stage(&scene, "linetype_colors", &colors)?;
        edit.submit(history)?;
        return Ok(d.name);
    }
    let own = scene.getattr("line_types")?.cast_into::<PyDict>()?.copy()?;
    let name = d.name.trim();
    let mut edit = StyleEdit::new("Edit line type");
    if !d.original.is_empty() && !own.contains(&d.original)? {
        return Err(PyValueError::new_err("The line type no longer exists"));
    }
    let key = match verb {
        "delete" => {
            if d.original.is_empty() {
                return Err(PyValueError::new_err("Choose a document line type"));
            }
            own.del_item(&d.original)?;
            "continuous".to_owned()
        }
        "save" => {
            if name.is_empty() || name.chars().count() > 64 {
                return Err(PyValueError::new_err("Enter a name of up to 64 characters"));
            }
            if !d.original.is_empty() && name != d.original && own.contains(name)? {
                return Err(PyValueError::new_err(
                    "A line type with that name already exists",
                ));
            }
            if d.pieces.len() > 32
                || (!d.text.is_empty()
                    && (!(0.5..=20.).contains(&d.text_size) || d.text.chars().count() > 24))
            {
                return Err(PyValueError::new_err(
                    "Invalid line type pattern or text size",
                ));
            }
            let Some(pieces) = crate::model::linetypes::clean_values(&d.pieces) else {
                return Err(PyValueError::new_err(
                    "Enter a dash and a negative gap in millimetres",
                ));
            };
            let (drawn, spec) = crate::model::linetypes::with_text(py, &pieces, &d.text, d.text_size)?;
            let Some(drawn) = crate::model::linetypes::clean_values(&drawn) else {
                return Err(PyValueError::new_err("The text gap is too large"));
            };
            let entry = PyDict::new(py);
            entry.set_item("label", name)?;
            entry.set_item("pieces", pieces)?;
            entry.set_item("pattern", drawn)?;
            if let Some(spec) = spec {
                entry.update(crate::model::linetypes::spec_dict(py, &spec)?.as_mapping())?;
            }
            if let Some(rgb) = d.color {
                if !rgb.iter().all(|v| (0.0..=1.0).contains(v)) {
                    return Err(PyValueError::new_err("Invalid line colour"));
                }
                entry.set_item("color", (rgb[0], rgb[1], rgb[2]))?;
            }
            if !d.original.is_empty() && d.original != name {
                own.del_item(&d.original)?;
            }
            own.set_item(name, entry)?;
            format!("lin:{name}")
        }
        _ => return Err(PyValueError::new_err("Unknown line type edit")),
    };
    edit.stage(&scene, "line_types", &own)?;
    let old = format!("lin:{}", d.original);
    let mut targets: HashSet<_> = d.tags.into_iter().collect();
    for ly in scene.getattr("layers")?.try_iter()? {
        let ly = ly?;
        if targets.remove(&ly.getattr("key")?.extract::<String>()?)
            || (!d.original.is_empty() && ly.getattr("linetype")?.eq(&old)?)
        {
            edit.stage(&ly, "linetype", key.as_str().into_pyobject(py)?.as_any())?;
        }
    }
    if !targets.is_empty() {
        return Err(PyValueError::new_err("A selected tag no longer exists"));
    }
    edit.submit(history)?;
    Ok(key)
}

#[pyfunction]
pub fn import_line_file(
    history: &Bound<'_, PyAny>,
    path: PathBuf,
) -> PyResult<(usize, Vec<String>)> {
    let py = history.py();
    let text = crate::hatch_file::read_pattern(path)?;
    let layers = py.import("core.layers")?;
    let (found, skipped) = crate::model::linetypes::py_parse_lin(py, &text)?;
    let scene = history.getattr("scene")?;
    let work = py.import("types")?.getattr("SimpleNamespace")?.call0()?;
    work.setattr(
        "line_types",
        scene.getattr("line_types")?.cast_into::<PyDict>()?.copy()?,
    )?;
    let count = layers
        .call_method1("load_line_types", (&work, found))?
        .extract()?;
    let mut edit = StyleEdit::new("Import line types");
    edit.stage(&scene, "line_types", &work.getattr("line_types")?)?;
    edit.submit(history)?;
    Ok((count, skipped))
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FolderDraft {
    folder: String,
    name: String,
    parent: String,
    folders: Vec<String>,
    owner: Option<usize>,
    tags: Vec<String>,
}

#[pyfunction]
pub fn edit_tag_folder(history: &Bound<'_, PyAny>, verb: &str, draft: &str) -> PyResult<String> {
    let d: FolderDraft =
        serde_json::from_str(draft).map_err(|e| PyValueError::new_err(e.to_string()))?;
    let py = history.py();
    let scene = history.getattr("scene")?;
    if d.owner
        .is_some_and(|owner| owner != scene.as_ptr() as usize)
    {
        return Err(PyValueError::new_err(
            "The active document changed during the drag",
        ));
    }
    let backend = py.import("core.layers")?;
    let namespace = py.import("types")?.getattr("SimpleNamespace")?;
    let work = namespace.call0()?;
    let attrs = [
        "layer_folders",
        "layer_folder_names",
        "folder_parents",
        "layer_folder_uids",
        "folder_visibility",
    ];
    for attr in attrs {
        let value = scene.getattr(attr)?;
        let container = if attr == "layer_folders" {
            "list"
        } else {
            "dict"
        };
        work.setattr(
            attr,
            py.import("builtins")?.call_method1(container, (value,))?,
        )?;
    }
    let tags = PyList::empty(py);
    let mut originals = Vec::new();
    for ly in scene.getattr("layers")?.try_iter()? {
        let ly = ly?;
        let tag = namespace.call0()?;
        tag.setattr("key", ly.getattr("key")?)?;
        tag.setattr("folder", ly.getattr("folder")?)?;
        tags.append(&tag)?;
        originals.push((ly, tag));
    }
    work.setattr("layers", &tags)?;
    let known: Vec<String> = backend.call_method1("folders_of", (&work,))?.extract()?;
    let parent = (!d.parent.is_empty()).then_some(d.parent.as_str());
    if parent.is_some_and(|v| !known.iter().any(|k| k == v)) {
        return Err(PyValueError::new_err("Destination folder no longer exists"));
    }
    if matches!(verb, "move" | "delete" | "rename") && !known.contains(&d.folder) {
        return Err(PyValueError::new_err("Folder no longer exists"));
    }
    let mut key = d.folder;
    match verb {
        "new" => {
            if d.name.trim().is_empty() {
                return Err(PyValueError::new_err("Enter a folder name"));
            }
            key = backend
                .call_method1("add_folder", (&work, "Folder", parent))?
                .extract()?;
            backend.call_method1("rename_folder", (&work, &key, d.name.trim()))?;
        }
        "rename" | "move" => {
            let accepted = if verb == "rename" {
                backend.call_method1("rename_folder", (&work, &key, d.name.trim()))?
            } else {
                backend.call_method1("move_folder", (&work, &key, parent))?
            };
            if !accepted.is_truthy()? {
                return Err(PyValueError::new_err("Invalid folder name or destination"));
            }
        }
        "delete" => {
            backend.call_method1("remove_folder", (&work, &key))?;
        }
        "file" | "batch" => {
            let folders: HashSet<_> = d.folders.into_iter().collect();
            let known: HashSet<_> = known.iter().collect();
            if folders.iter().any(|key| !known.contains(key)) {
                return Err(PyValueError::new_err("A selected folder no longer exists"));
            }
            // Work on the detached snapshot. Descendants of any selected
            // folder travel inside it, including separately selected tags.
            let parents = work.getattr("folder_parents")?.cast_into::<PyDict>()?;
            let mut children: HashMap<String, Vec<String>> = HashMap::new();
            for (child, up) in parents.iter() {
                let child: String = child.extract()?;
                if let Ok(up) = up.extract::<String>() {
                    if known.contains(&child) && known.contains(&up) {
                        children.entry(up).or_default().push(child);
                    }
                }
            }
            let mut covered = HashSet::new();
            let mut pending: Vec<_> = folders.iter().cloned().collect();
            while let Some(key) = pending.pop() {
                if covered.insert(key.clone()) {
                    if let Some(below) = children.get(&key) {
                        pending.extend(below.iter().cloned());
                    }
                }
            }
            if parent.is_some_and(|up| covered.contains(up)) {
                return Err(PyValueError::new_err(
                    "A folder cannot move inside itself or its descendants",
                ));
            }
            for folder in &folders {
                let old: Option<String> = parents
                    .get_item(folder)?
                    .map(|v| v.extract())
                    .transpose()?
                    .flatten();
                if old.as_ref().is_some_and(|up| covered.contains(up)) {
                    continue;
                }
                if let Some(up) = parent {
                    parents.set_item(folder, up)?;
                } else if old.is_some() {
                    parents.del_item(folder)?;
                }
            }
            let mut needed: HashSet<_> = d.tags.into_iter().collect();
            if needed.contains("Layer 0") {
                return Err(PyValueError::new_err("Untagged stays at the root"));
            }
            for (_, tag) in &originals {
                if needed.remove(&tag.getattr("key")?.extract::<String>()?) {
                    let old = tag.getattr("folder")?.extract::<Option<String>>()?;
                    if !old.is_some_and(|folder| covered.contains(&folder)) {
                        tag.setattr("folder", parent)?;
                    }
                }
            }
            if !needed.is_empty() {
                return Err(PyValueError::new_err("A selected tag no longer exists"));
            }
        }
        _ => return Err(PyValueError::new_err("Unknown folder edit")),
    }
    let mut edit = StyleEdit::new("Edit tag folders");
    for attr in attrs {
        edit.stage(&scene, attr, &work.getattr(attr)?)?;
    }
    for (ly, tag) in originals {
        edit.stage(&ly, "folder", &tag.getattr("folder")?)?;
    }
    edit.submit(history)?;
    Ok(key)
}

#[pyfunction]
pub fn document_piece(history: &Bound<'_, PyAny>, kind: &str, size: Vec<f64>) -> PyResult<()> {
    let scene = history.getattr("scene")?;
    let h = scene.py().import("core.hatches")?;
    if !h.getattr("PIECE_DEFAULT")?.contains(kind)? {
        return Err(PyValueError::new_err("Unknown piece kind"));
    }
    let bricks = scene.getattr("bricks")?.cast_into::<PyDict>()?.copy()?;
    if size.is_empty() {
        if bricks.contains(kind)? {
            bricks.del_item(kind)?;
        }
    } else {
        let value = h.call_method1("clean_brick", (size,))?;
        if value.is_none() {
            return Err(PyValueError::new_err(
                "Invalid piece dimensions or joint widths",
            ));
        }
        bricks.set_item(kind, value)?;
    }
    let mut edit = StyleEdit::new("Set document piece size");
    edit.stage(&scene, "bricks", &bricks)?;
    for ly in scene.getattr("layers")?.try_iter()? {
        let ly = ly?;
        if h.call_method1("piece_kind", (ly.getattr("hatch")?,))?
            .eq(kind)?
        {
            edit.stage(&ly, "brick", scene.py().None().bind(scene.py()))?;
        }
    }
    edit.submit(history)
}
