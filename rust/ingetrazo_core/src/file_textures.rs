// SPDX-License-Identifier: GPL-3.0-or-later
//! Map texture and PBR references between cache paths and archive members.
use pyo3::exceptions::PyRecursionError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyString};

struct Textures<'py> {
    resolve: Bound<'py, PyAny>,
    packing: bool,
}

impl Textures<'_> {
    fn slot(
        &self,
        parent: &Bound<'_, PyDict>,
        key: &Bound<'_, PyAny>,
        image: &Bound<'_, PyDict>,
    ) -> PyResult<()> {
        let image = if self.packing {
            let copy = image.copy()?;
            parent.set_item(key, &copy)?;
            copy
        } else {
            image.clone()
        };
        let pairs = if self.packing {
            [("path", "embed"), ("base", "base_embed")]
        } else {
            [("base_embed", "base"), ("embed", "path")]
        };
        for (from, to) in pairs {
            let Some(value) = image.get_item(from)? else {
                continue;
            };
            if !self.packing {
                image.del_item(from)?;
                if let Some(existing) = image.get_item(to)? {
                    if existing.is_truthy()? {
                        continue;
                    }
                }
            }
            if value.is_truthy()? {
                let mapped = self.resolve.call1((value,))?;
                if mapped.is_truthy()? {
                    if self.packing {
                        image.del_item(from)?;
                    }
                    image.set_item(to, mapped)?;
                }
            }
        }
        Ok(())
    }

    fn walk(&self, root: &Bound<'_, PyAny>) -> PyResult<()> {
        enum Task<'py> {
            Enter(Bound<'py, PyAny>),
            Leave(usize),
            Slot(Bound<'py, PyDict>, Bound<'py, PyAny>, Bound<'py, PyDict>),
        }
        let mut pending = vec![Task::Enter(root.clone())];
        let mut ancestors = std::collections::HashSet::new();
        while let Some(task) = pending.pop() {
            let node = match task {
                Task::Leave(id) => {
                    ancestors.remove(&id);
                    continue;
                }
                Task::Slot(parent, key, image) => {
                    self.slot(&parent, &key, &image)?;
                    continue;
                }
                Task::Enter(node) => node,
            };
            let id = node.as_ptr() as usize;
            if !ancestors.insert(id) {
                return Err(PyRecursionError::new_err("cyclic texture references"));
            }
            pending.push(Task::Leave(id));
            let start = pending.len();
            if let Ok(dict) = node.cast::<PyDict>() {
                for (key, value) in dict {
                    let text = key.cast::<PyString>().ok().and_then(|s| s.to_str().ok());
                    if let Ok(image) = value.cast::<PyDict>() {
                        if text == Some("texture") {
                            pending.push(Task::Slot(dict.clone(), key, image.clone()));
                            continue;
                        }
                        if text == Some("pbr") {
                            let recipe = image.copy()?;
                            dict.set_item(key, &recipe)?;
                            for (slot, value) in &recipe {
                                if slot.cast::<PyString>()?.to_str()?.ends_with("_map") {
                                    if let Ok(image) = value.cast::<PyDict>() {
                                        pending.push(Task::Slot(
                                            recipe.clone(),
                                            slot,
                                            image.clone(),
                                        ));
                                    }
                                }
                            }
                            continue;
                        }
                    }
                    if !matches!(
                        text,
                        Some("vertices" | "holes" | "a" | "b" | "edges" | "arrays")
                    ) && (value.is_instance_of::<PyDict>() || value.is_instance_of::<PyList>())
                    {
                        pending.push(Task::Enter(value));
                    }
                }
            } else if let Ok(list) = node.cast::<PyList>() {
                if !list.is_empty() {
                    let first = list.get_item(0)?;
                    if first.is_instance_of::<PyDict>() || first.is_instance_of::<PyList>() {
                        pending.extend(list.iter().map(Task::Enter));
                    }
                }
            }
            // Preserve the insertion order of content-addressed archive members.
            pending[start..].reverse();
        }
        Ok(())
    }
}

#[pyfunction]
pub fn igz_texture_references(
    node: &Bound<'_, PyAny>,
    resolve: Bound<'_, PyAny>,
    packing: bool,
) -> PyResult<()> {
    Textures { resolve, packing }.walk(node)
}
