// SPDX-License-Identifier: GPL-3.0-or-later
//! A Library choice paints the occurrences for which the sheet opened.
//! Selection can move while browsing. Validate every target again at Apply.
use super::Occurrences;
use crate::ui::dialog::Field;
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyDict, PyList},
};
use std::collections::HashSet;

#[pyclass]
pub struct TreePaint {
    document: u64,
    groups: Py<PyList>,
    #[pyo3(get)]
    current: Option<String>,
    color: String,
    name: String,
}

fn document(scene: &Bound<'_, PyAny>) -> PyResult<u64> {
    scene.getattr("_root")?.getattr("_doc_key")?.extract()
}

impl TreePaint {
    fn fields(&self) -> Vec<Field> {
        [
            ("tree_colour", "color", "Colour", &self.color),
            ("tree_name", "edit", "Name (optional)", &self.name),
        ]
        .map(|(id, kind, label, value)| {
            (
                id.into(),
                kind.into(),
                label.into(),
                value.clone(),
                0.,
                0.,
                0.,
                vec![],
                0,
                String::new(),
            )
        })
        .into()
    }
}

#[pymethods]
impl TreePaint {
    #[new]
    fn new(scene: &Bound<'_, PyAny>, clicked: &Bound<'_, PyAny>) -> PyResult<Self> {
        let tree = Occurrences::read(scene, clicked)?;
        let groups = PyList::new(
            scene.py(),
            tree.targets.iter().map(|&i| &tree.rows[i].group),
        )?;
        let current = groups
            .iter()
            .map(|g| {
                let paint = g.getattr("material")?;
                if paint.is_none() {
                    Ok(None)
                } else {
                    paint
                        .call_method1("get", ("mat",))?
                        .extract::<Option<String>>()
                }
            })
            .collect::<PyResult<Vec<_>>>()?;
        let common = current
            .first()
            .filter(|first| current.iter().all(|n| n == *first))
            .cloned()
            .flatten();
        Ok(Self {
            document: document(scene)?,
            groups: groups.unbind(),
            current: common,
            color: "#ffffff".into(),
            name: String::new(),
        })
    }

    /// Limit tile menus to the question the sheet asks. Recent loose paints
    /// are additional choices even when no open document registers them.
    fn prepare(&self, items: &Bound<'_, PyList>, browse: &Bound<'_, PyDict>) -> PyResult<()> {
        let py = items.py();
        let recent = browse
            .get_item("recent")?
            .ok_or_else(|| PyValueError::new_err("Missing recent Library choices"))?
            .cast_into::<PyList>()?;
        let mut seen = HashSet::new();
        for item in items.iter() {
            item.set_item("verbs", ["apply"])?;
            if item.get_item("source")?.extract::<String>()? == "model" {
                seen.insert(item.get_item("name")?.extract::<String>()?);
            }
        }
        let brush = py.import("tools.paint")?.getattr("PaintTool")?;
        let swatches: Vec<(String, String, String, String)> = py
            .import("ingetrazo_core")?
            .call_method1("recent_swatches", (brush,))?
            .extract()?;
        for (key, name, color, path) in swatches {
            if !seen.insert(name.clone()) {
                continue;
            }
            let item = PyDict::new(py);
            item.set_item("key", &key)?;
            item.set_item("name", name)?;
            item.set_item("source", "recent")?;
            item.set_item("base", if path.is_empty() { "colour" } else { "image" })?;
            item.set_item("thumb", path)?;
            item.set_item("hex", color)?;
            item.set_item("verbs", ["apply"])?;
            items.append(item)?;
            recent.append(key)?;
        }
        Ok(())
    }

    fn wire(&self) -> ((), (), (), (), Vec<Field>) {
        ((), (), (), (), self.fields())
    }
    fn field(&self, id: &str) -> Option<Field> {
        self.fields().into_iter().find(|f| f.0 == id)
    }
    fn receive(&mut self, id: &str, text: String) {
        match id {
            "tree_colour" => self.color = text,
            "tree_name" => self.name = text,
            _ => (),
        }
    }

    /// Convert the chosen Library recipe into the same container stamp as a
    /// Paint stroke. Register a new material in the same undo transaction.
    #[pyo3(signature=(vp, key, recipe=None))]
    fn apply(
        &self,
        vp: &Bound<'_, PyAny>,
        key: &str,
        recipe: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let py = vp.py();
        let scene = vp.getattr("scene")?;
        let groups = self.groups.bind(py);
        let stale =
            || PyValueError::new_err("The chosen rows changed. Reopen the material picker.");
        if document(&scene)? != self.document || groups.is_empty() {
            return Err(stale());
        }
        let tree = Occurrences::read(&scene, groups.as_any())?;
        if tree.targets.len() != groups.len() {
            return Err(stale());
        }
        let reason = tree.refusal(&scene, &tree.targets, false)?;
        if !reason.is_empty() {
            return Err(PyValueError::new_err(reason));
        }
        let history = py.import("core.history")?;
        let cmds = PyList::empty(py);
        let materials = scene.getattr("materials")?;
        let brush = if key == "activepaint" {
            Some(py.import("tools.paint")?.getattr("PaintTool")?)
        } else {
            None
        };
        let clear = match &brush {
            Some(b) => b.getattr("current_is_default")?.is_truthy()?,
            None => key == "clearpaint",
        };
        let stamp = if clear {
            py.None().into_bound(py)
        } else {
            let data = if let Some(brush) = brush {
                let material = brush.getattr("current_material")?;
                if material.is_none() {
                    brush
                        .call_method0("_current_as_material")?
                        .cast_into::<PyDict>()?
                } else {
                    material.call_method0("to_dict")?.cast_into::<PyDict>()?
                }
            } else if key == "custompaint" {
                let rgb = self
                    .color
                    .strip_prefix('#')
                    .filter(|s| s.len() == 6)
                    .and_then(|s| u32::from_str_radix(s, 16).ok())
                    .ok_or_else(|| PyValueError::new_err("Choose an RGB colour"))?;
                let value = PyDict::new(py);
                value.set_item(
                    "color",
                    [16, 8, 0].map(|i| f64::from((rgb >> i) & 255) / 255.),
                )?;
                if !self.name.trim().is_empty() {
                    if materials.contains(self.name.trim())? {
                        return Err(PyValueError::new_err("A material already has that name."));
                    }
                    value.set_item("name", self.name.trim())?;
                }
                value
            } else if let Some(raw) = key.strip_prefix("stamp:") {
                let value = py
                    .import("json")?
                    .call_method1("loads", (raw,))?
                    .cast_into::<PyDict>()?;
                if let Some(name) = value.get_item("mat")? {
                    value.set_item("name", name)?;
                }
                value
            } else if let Some(recipe) = recipe {
                recipe.copy()?
            } else {
                let mat =
                    materials.call_method1("get", (key.strip_prefix("m:").unwrap_or(key),))?;
                if mat.is_none() {
                    return Err(PyValueError::new_err(
                        "The material is no longer in the model.",
                    ));
                }
                mat.call_method0("to_dict")?.cast_into::<PyDict>()?
            };
            let name = data
                .get_item("name")?
                .and_then(|n| n.extract::<String>().ok())
                .unwrap_or_default();
            if name.is_empty() {
                let stamp = PyDict::new(py);
                for field in ["color", "texture", "opacity", "pbr"] {
                    if let Some(value) = data.get_item(field)? {
                        if !value.is_none() {
                            stamp.set_item(field, value)?;
                        }
                    }
                }
                stamp.into_any()
            } else {
                let mut mat = materials.call_method1("get", (&name,))?;
                if mat.is_none() {
                    mat = py
                        .import("core.materials")?
                        .getattr("Material")?
                        .call_method1("from_dict", (&data,))?;
                    cmds.append(history.call_method1(
                        "SetFaceMaterialTagCommand",
                        (PyList::empty(py), &name, &mat),
                    )?)?;
                }
                let mut color = mat.getattr("color")?;
                if color.is_none() {
                    color = (1., 1., 1.).into_pyobject(py)?.into_any();
                }
                py.import("core.paint_ops")?.call_method1(
                    "paint_stamp",
                    (color, mat.getattr("texture")?, mat.getattr("opacity")?, mat),
                )?
            }
        };
        for group in groups.iter() {
            if !group.getattr("material")?.eq(&stamp)? {
                cmds.append(history.call_method1("SetGroupMaterialCommand", (group, &stamp))?)?;
            }
        }
        let command = history.call_method1("CompoundCommand", (cmds,))?;
        py.import("views.tree_navigation")?
            .call_method1("execute", (vp, command))?;
        Ok(())
    }
}
