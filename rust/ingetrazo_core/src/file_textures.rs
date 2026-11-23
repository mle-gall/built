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

    fn walk(&self, node: &Bound<'_, PyAny>, depth: usize) -> PyResult<()> {
        if depth > 512 {
            return Err(PyRecursionError::new_err("texture reference nesting limit"));
        }
        if let Ok(dict) = node.cast::<PyDict>() {
            for (key, value) in dict {
                let text = key.cast::<PyString>().ok().and_then(|s| s.to_str().ok());
                if let Ok(image) = value.cast::<PyDict>() {
                    if text == Some("texture") {
                        self.slot(dict, &key, image)?;
                        continue;
                    }
                    if text == Some("pbr") {
                        let recipe = image.copy()?;
                        dict.set_item(key, &recipe)?;
                        for (slot, value) in &recipe {
                            let name = slot.cast::<PyString>()?.to_str()?;
                            if name.ends_with("_map") {
                                if let Ok(image) = value.cast::<PyDict>() {
                                    self.slot(&recipe, &slot, image)?;
                                }
                            }
                        }
                        continue;
                    }
                }
                if !matches!(
                    text,
                    Some("vertices" | "holes" | "a" | "b" | "edges" | "arrays")
                ) {
                    if value.is_instance_of::<PyDict>() || value.is_instance_of::<PyList>() {
                        self.walk(&value, depth + 1)?;
                    }
                }
            }
        } else if let Ok(list) = node.cast::<PyList>() {
            if !list.is_empty() {
                let first = list.get_item(0)?;
                if first.is_instance_of::<PyDict>() || first.is_instance_of::<PyList>() {
                    for item in list {
                        self.walk(&item, depth + 1)?;
                    }
                }
            }
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
    Textures { resolve, packing }.walk(node, 0)
}
